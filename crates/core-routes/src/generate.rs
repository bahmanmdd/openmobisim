//! Route-set generation behind one interface (design §13.6).
//!
//! **A method is a [`RouteSetGenerator`]**: given a search and an origin and a
//! destination node, it returns that pair's routes. Everything else — the
//! store, the parallel driver, the fingerprint, the Python surface — is
//! written against the trait, so a new method (exact label-constrained
//! k-shortest, plateau/via-node, a sampled or learned generator, a multimodal
//! one) is one new type and one line in a [`Registry`]; nothing else changes.
//!
//! Built in:
//!
//! | name | what it does |
//! |---|---|
//! | `penalty` (the default, [`DEFAULT_METHOD`]) | The shortest route, then repeatedly the shortest under penalties on the links already used, keeping routes that stay within a detour bound and an overlap limit |
//! | `shortest` | The single shortest route |
//!
//! A method's **options** are numbers by name ([`Options`]), validated when the
//! method is made, and its **descriptor** — its name and every option with
//! defaults filled in — is part of a store's identity, so a set made with
//! other settings is never mistaken for this one.

use std::collections::BTreeMap;
use std::fmt;

use openmobisim_core_types::ids::NodeId;

use crate::search::{MAX_ROUTES_PER_SET, Route, Search};

/// The method used unless another is asked for.
pub const DEFAULT_METHOD: &str = "penalty";

/// A method's options: numbers by name. Unknown names are an error, not ignored.
pub type Options = BTreeMap<String, f64>;

/// Why a method could not be made.
#[derive(Clone, PartialEq, Debug)]
pub enum RouteError {
    /// No method has this name.
    UnknownMethod {
        /// The name asked for.
        name: String,
        /// The names that exist.
        known: Vec<String>,
    },
    /// The method has no such option.
    UnknownOption {
        /// The method.
        method: String,
        /// The option asked for.
        option: String,
        /// The options the method has.
        known: Vec<&'static str>,
    },
    /// An option's value is not allowed.
    BadOption {
        /// The method.
        method: String,
        /// The option.
        option: String,
        /// What is wrong with it.
        reason: String,
    },
}

impl fmt::Display for RouteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownMethod { name, known } => {
                write!(f, "unknown route-set method {name:?}; choose from {known:?}")
            }
            Self::UnknownOption { method, option, known } => {
                write!(f, "method {method:?} has no option {option:?}; its options are {known:?}")
            }
            Self::BadOption { method, option, reason } => {
                write!(f, "method {method:?}, option {option:?}: {reason}")
            }
        }
    }
}

impl std::error::Error for RouteError {}

/// A route-set generation method.
///
/// Implementations must be **deterministic**: the same search, origin and
/// destination give the same routes in the same order, whatever thread runs
/// them and whatever else has been generated (the search leaves no state
/// behind).
pub trait RouteSetGenerator: Send + Sync {
    /// The method's name, as it is selected.
    fn name(&self) -> &str;

    /// The method's name and every option with defaults filled in, as a
    /// canonical string. It is part of a store's identity.
    fn descriptor(&self) -> String;

    /// The routes from `origin` to `destination`, best first, or none if there
    /// is no route. `origin` and `destination` differ.
    fn generate(&self, search: &mut Search<'_>, origin: NodeId, destination: NodeId) -> Vec<Route>;
}

/// Sort routes by cost, then by their links, so the order is a function of the
/// routes alone.
fn sort_best_first(routes: &mut [Route]) {
    routes
        .sort_by(|a, b| a.cost.total_cmp(&b.cost).then_with(|| a.links.iter().cmp(b.links.iter())));
}

// --- shortest ---------------------------------------------------------------------

/// The single shortest route.
#[derive(Clone, Debug, Default)]
pub struct Shortest;

impl Shortest {
    const OPTIONS: [&'static str; 0] = [];

    /// Make the method from its options (it has none).
    ///
    /// # Errors
    ///
    /// [`RouteError::UnknownOption`] for any option.
    pub fn from_options(options: &Options) -> Result<Self, RouteError> {
        check_known("shortest", options, &Self::OPTIONS)?;
        Ok(Self)
    }
}

impl RouteSetGenerator for Shortest {
    fn name(&self) -> &str {
        "shortest"
    }

    fn descriptor(&self) -> String {
        "shortest".to_string()
    }

    fn generate(&self, search: &mut Search<'_>, origin: NodeId, destination: NodeId) -> Vec<Route> {
        search.shortest(origin, destination).into_iter().collect()
    }
}

// --- penalty ----------------------------------------------------------------------

/// The penalty method: find the shortest route, multiply the cost of the links
/// of the route just found and search again, and keep a new route if it is
/// distinct, within `max_detour` of the best cost, and shares at most
/// `max_overlap` of its cost with every route kept before.
///
/// **Cost:** at most `max_attempts` searches per pair, each bounded to routes
/// within `max_detour` of the best, so none explores more than about
/// `max_detour²` times the first. **Guarantee:** none of
/// optimality or completeness — it finds plausible, distinct alternatives, not
/// the k best. The first route is always a shortest one.
#[derive(Clone, Debug, PartialEq)]
pub struct Penalty {
    /// How many routes to keep at most (1 to 32).
    pub max_paths: usize,
    /// No route costs more than this multiple of the best route's cost (at least 1).
    pub max_detour: f64,
    /// A route shares at most this share of its cost with any earlier route (above 0, at most 1).
    pub max_overlap: f64,
    /// The factor applied to the cost of the links of each route found (above 1).
    pub penalty: f64,
    /// How many searches to try at most for one pair.
    pub max_attempts: usize,
}

impl Default for Penalty {
    fn default() -> Self {
        Self { max_paths: 5, max_detour: 1.3, max_overlap: 0.75, penalty: 1.5, max_attempts: 15 }
    }
}

impl Penalty {
    const OPTIONS: [&'static str; 5] =
        ["max_attempts", "max_detour", "max_overlap", "max_paths", "penalty"];

    /// Make the method from its options, defaults for those not given.
    ///
    /// # Errors
    ///
    /// [`RouteError::UnknownOption`] for a name it does not have,
    /// [`RouteError::BadOption`] for a value out of range.
    pub fn from_options(options: &Options) -> Result<Self, RouteError> {
        check_known("penalty", options, &Self::OPTIONS)?;
        let mut m = Self::default();
        let bad = |option: &str, reason: &str| RouteError::BadOption {
            method: "penalty".to_string(),
            option: option.to_string(),
            reason: reason.to_string(),
        };
        let whole = |option: &str, v: f64, lo: usize, hi: usize| -> Result<usize, RouteError> {
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "checked to be a non-negative whole number within a small range"
            )]
            let n = v as usize;
            if v.is_finite() && v >= 0.0 && v.fract() == 0.0 && n >= lo && n <= hi {
                Ok(n)
            } else {
                Err(bad(option, &format!("must be a whole number from {lo} to {hi}, got {v}")))
            }
        };
        if let Some(&v) = options.get("max_paths") {
            m.max_paths = whole("max_paths", v, 1, MAX_ROUTES_PER_SET)?;
            m.max_attempts = (3 * m.max_paths).max(m.max_paths);
        }
        if let Some(&v) = options.get("max_attempts") {
            m.max_attempts = whole("max_attempts", v, 1, 1000)?;
        }
        if let Some(&v) = options.get("max_detour") {
            if !(v.is_finite() && v >= 1.0) {
                return Err(bad("max_detour", &format!("must be at least 1, got {v}")));
            }
            m.max_detour = v;
        }
        if let Some(&v) = options.get("max_overlap") {
            if !(v > 0.0 && v <= 1.0) {
                return Err(bad("max_overlap", &format!("must be above 0 and at most 1, got {v}")));
            }
            m.max_overlap = v;
        }
        if let Some(&v) = options.get("penalty") {
            if !(v.is_finite() && v > 1.0) {
                return Err(bad("penalty", &format!("must be above 1, got {v}")));
            }
            m.penalty = v;
        }
        Ok(m)
    }
}

impl RouteSetGenerator for Penalty {
    fn name(&self) -> &str {
        "penalty"
    }

    fn descriptor(&self) -> String {
        format!(
            "penalty;max_attempts={};max_detour={};max_overlap={};max_paths={};penalty={}",
            self.max_attempts, self.max_detour, self.max_overlap, self.max_paths, self.penalty
        )
    }

    fn generate(&self, search: &mut Search<'_>, origin: NodeId, destination: NodeId) -> Vec<Route> {
        search.clear_route_state();
        let Some(first) = search.shortest(origin, destination) else {
            return Vec::new();
        };
        let best = first.cost;
        search.mark(&first.links, 0);
        let mut last = first.links.clone();
        let mut kept = vec![first];
        for _ in 0..self.max_attempts {
            if kept.len() >= self.max_paths {
                break;
            }
            for &l in &last {
                search.factors.multiply(l, self.penalty);
            }
            // Bounded by the detour limit: a route past it would be rejected anyway, and
            // an unbounded penalised search wanders far beyond it (S165).
            let Some(mut candidate) =
                search.shortest_within(origin, destination, self.max_detour * best)
            else {
                break;
            };
            last.clone_from(&candidate.links);
            if candidate.cost > self.max_detour * best
                || kept.iter().any(|k| k.links == candidate.links)
            {
                continue;
            }
            let overlap = search.max_overlap(&candidate.links);
            if overlap > self.max_overlap {
                continue;
            }
            candidate.overlap = overlap;
            search.mark(&candidate.links, kept.len());
            kept.push(candidate);
        }
        search.clear_route_state();
        sort_best_first(&mut kept);
        kept
    }
}

fn check_known(method: &str, options: &Options, known: &[&'static str]) -> Result<(), RouteError> {
    for name in options.keys() {
        if !known.contains(&name.as_str()) {
            return Err(RouteError::UnknownOption {
                method: method.to_string(),
                option: name.clone(),
                known: known.to_vec(),
            });
        }
    }
    Ok(())
}

// --- the registry -----------------------------------------------------------------

/// Makes a method from its options.
pub type Factory = fn(&Options) -> Result<Box<dyn RouteSetGenerator>, RouteError>;

/// Methods by name. [`Registry::builtin`] has the built-in ones; a researcher's
/// method is added with [`Registry::register`] and then selected by name like
/// any other.
#[derive(Clone, Debug)]
pub struct Registry {
    methods: Vec<(String, Factory)>,
}

impl Registry {
    /// The built-in methods: `penalty` and `shortest`.
    #[must_use]
    pub fn builtin() -> Self {
        let mut r = Self { methods: Vec::new() };
        r.register("penalty", |o| Ok(Box::new(Penalty::from_options(o)?)));
        r.register("shortest", |o| Ok(Box::new(Shortest::from_options(o)?)));
        r
    }

    /// Add a method under `name`, or replace one of that name.
    pub fn register(&mut self, name: &str, factory: Factory) {
        match self.methods.iter_mut().find(|(n, _)| n == name) {
            Some(entry) => entry.1 = factory,
            None => self.methods.push((name.to_string(), factory)),
        }
    }

    /// The names that can be selected, in registration order.
    #[must_use]
    pub fn names(&self) -> Vec<&str> {
        self.methods.iter().map(|(n, _)| n.as_str()).collect()
    }

    /// Make the method called `name` from `options`.
    ///
    /// # Errors
    ///
    /// [`RouteError::UnknownMethod`] if there is none, or whatever the
    /// method's own validation says about its options.
    pub fn create(
        &self,
        name: &str,
        options: &Options,
    ) -> Result<Box<dyn RouteSetGenerator>, RouteError> {
        match self.methods.iter().find(|(n, _)| n == name) {
            Some((_, factory)) => factory(options),
            None => Err(RouteError::UnknownMethod {
                name: name.to_string(),
                known: self.names().into_iter().map(str::to_string).collect(),
            }),
        }
    }
}

/// Make a built-in method by name; see [`Registry::builtin`].
///
/// # Errors
///
/// As [`Registry::create`].
pub fn generator(name: &str, options: &Options) -> Result<Box<dyn RouteSetGenerator>, RouteError> {
    Registry::builtin().create(name, options)
}

/// The default method with its default options.
#[must_use]
pub fn default_generator() -> Box<dyn RouteSetGenerator> {
    Box::new(Penalty::default())
}
