//! The plugin registry: names resolved once, at build time (Foundations §7).
//!
//! # The rule
//!
//! **Every plugin name is resolved exactly once, when the scenario is built,
//! into a concrete callable.** There is never a name lookup, a hash probe or a
//! string comparison inside a loop. What the simulation holds is the thing
//! itself, not a way of finding it.
//!
//! This is also the mechanism behind "off means absent" (brief §3e): a plugin
//! that was not selected is not merely skipped at run time, it is not present
//! at all, so it cannot cost a branch.
//!
//! # Deterministic by construction
//!
//! Entries live in a [`BTreeMap`], so [`Registry::names`] is sorted and
//! independent of registration order. A registry is part of the build, and
//! anything that varies between builds eventually shows up in a result.
//!
//! # Examples
//!
//! ```
//! use openmobisim_core_types::registry::{Registry, RegistryError};
//!
//! // A toy plugin point: something that scores a link given a multiplier.
//! type Scorer = Box<dyn Fn(f64) -> f64 + Send + Sync>;
//!
//! let mut reg: Registry<f64, Scorer> = Registry::new("link_scorer");
//! reg.register("linear", |&k| Ok(Box::new(move |x| k * x) as Scorer)).unwrap();
//! reg.register("square", |&k| Ok(Box::new(move |x| k * x * x) as Scorer)).unwrap();
//!
//! assert_eq!(reg.names().collect::<Vec<_>>(), ["linear", "square"]);
//!
//! // Resolved once, at build time; the loop below holds the closure itself.
//! let scorer = reg.resolve("square", &2.0).unwrap();
//! assert_eq!(scorer(3.0), 18.0);
//!
//! // An unknown name fails at build time, with the alternatives listed.
//! assert!(matches!(
//!     reg.resolve("cubic", &1.0),
//!     Err(RegistryError::UnknownName { .. })
//! ));
//! ```

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::fmt;
use std::sync::Arc;

/// A constructor registered under a name.
type Constructor<P, T> = Arc<dyn Fn(&P) -> Result<T, RegistryError> + Send + Sync>;

/// Why a registry operation failed.
///
/// Every variant is a **build-time** failure: an invalid scenario is rejected
/// before any run starts, which is why none of these are conditions the
/// simulation has to carry on through.
#[derive(Clone, Debug)]
pub enum RegistryError {
    /// The scenario asked for a name nothing had registered.
    UnknownName {
        /// Which plugin point was being resolved.
        point: &'static str,
        /// The name that was asked for.
        requested: String,
        /// The names that are available, sorted.
        available: Vec<String>,
    },
    /// Two constructors were registered under one name.
    DuplicateName {
        /// Which plugin point.
        point: &'static str,
        /// The name registered twice.
        name: String,
    },
    /// The constructor ran and refused its parameters.
    Construction {
        /// Which plugin point.
        point: &'static str,
        /// The name being constructed.
        name: String,
        /// What the constructor objected to.
        reason: String,
    },
}

impl RegistryError {
    /// Build a [`RegistryError::Construction`] from inside a constructor.
    #[must_use]
    pub fn construction(
        point: &'static str,
        name: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        RegistryError::Construction { point, name: name.into(), reason: reason.into() }
    }
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegistryError::UnknownName { point, requested, available } => {
                write!(f, "unknown {point} plugin {requested:?}; available: ")?;
                for (i, name) in available.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{name:?}")?;
                }
                if available.is_empty() {
                    f.write_str("(none registered)")?;
                }
                Ok(())
            }
            RegistryError::DuplicateName { point, name } => {
                write!(f, "{point} plugin {name:?} is registered twice")
            }
            RegistryError::Construction { point, name, reason } => {
                write!(f, "{point} plugin {name:?} rejected its parameters: {reason}")
            }
        }
    }
}

impl std::error::Error for RegistryError {}

/// Maps plugin names to constructors for one plugin point.
///
/// `P` is the parameter type the plugin point hands its constructors — usually
/// a small struct deserialised from the scenario file. `T` is what gets built:
/// a boxed trait object, or a concrete type where the point has only one
/// shape.
pub struct Registry<P, T> {
    point: &'static str,
    entries: BTreeMap<String, Constructor<P, T>>,
}

impl<P, T> Registry<P, T> {
    /// An empty registry for the named plugin point.
    ///
    /// `point` appears in every error message, so name it as a user would
    /// recognise it from the scenario file: `"choice_model"`, `"fleet_policy"`.
    #[must_use]
    pub fn new(point: &'static str) -> Self {
        Self { point, entries: BTreeMap::new() }
    }

    /// The plugin point this registry serves.
    #[must_use]
    pub const fn point(&self) -> &'static str {
        self.point
    }

    /// Register a constructor under `name`.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryError::DuplicateName`] if the name is taken. Silently
    /// overwriting would make the result depend on registration order, which
    /// is exactly the kind of thing that turns into an unreproducible run six
    /// months later.
    pub fn register<F>(&mut self, name: impl Into<String>, ctor: F) -> Result<(), RegistryError>
    where
        F: Fn(&P) -> Result<T, RegistryError> + Send + Sync + 'static,
    {
        let name = name.into();
        match self.entries.entry(name) {
            Entry::Occupied(e) => {
                Err(RegistryError::DuplicateName { point: self.point, name: e.key().clone() })
            }
            Entry::Vacant(e) => {
                e.insert(Arc::new(ctor));
                Ok(())
            }
        }
    }

    /// Resolve `name` into a built plugin.
    ///
    /// Call this once, at scenario build time. Holding the result is the whole
    /// point; calling it again per iteration is the thing this design exists
    /// to avoid.
    ///
    /// # Errors
    ///
    /// [`RegistryError::UnknownName`] if nothing is registered under `name`,
    /// or whatever the constructor returns.
    pub fn resolve(&self, name: &str, params: &P) -> Result<T, RegistryError> {
        let ctor = self.entries.get(name).ok_or_else(|| RegistryError::UnknownName {
            point: self.point,
            requested: name.to_owned(),
            available: self.entries.keys().cloned().collect(),
        })?;
        ctor(params)
    }

    /// Whether a name is registered.
    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.entries.contains_key(name)
    }

    /// Every registered name, sorted.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    /// How many names are registered.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether nothing is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl<P, T> fmt::Debug for Registry<P, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Registry")
            .field("point", &self.point)
            .field("names", &self.entries.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl<P, T> Clone for Registry<P, T> {
    fn clone(&self) -> Self {
        Self { point: self.point, entries: self.entries.clone() }
    }
}
