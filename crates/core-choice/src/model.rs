//! The choice-model interface, the helpers a new model builds on, and the
//! by-name registry (design §4).
//!
//! **A model is a [`ChoiceModel`]**: given a [`ChoiceBatch`] and the run's
//! choice stream, it says which alternative each situation takes. Everything
//! else — building the batch, the run, the fingerprint, the manifest, the Python
//! surface — is written against the trait, so a new model is one type and one
//! line in a [`Registry`].
//!
//! # Writing a random utility model in a few lines
//!
//! Most models compute a utility for every alternative and let the traveller
//! take the best one after adding a random error. Implement only the utility
//! and call [`sample_random_utility`]:
//!
//! ```
//! use openmobisim_core_choice::{
//!     ChoiceBatch, ChoiceError, ChoiceModel, Choices, sample_random_utility,
//! };
//! use openmobisim_core_types::rng::StreamRng;
//!
//! /// Everyone dislikes each minute equally, and each kilometre twice as much.
//! struct Simple;
//!
//! impl ChoiceModel for Simple {
//!     fn name(&self) -> &str { "simple" }
//!     fn descriptor(&self) -> String { "simple".to_string() }
//!     fn is_sampled(&self) -> bool { true }
//!     fn required_attributes(&self) -> Option<Vec<String>> {
//!         Some(vec!["time_min".into(), "length_km".into()])
//!     }
//!     fn choose(&self, batch: &ChoiceBatch, rng: &StreamRng) -> Result<Choices, ChoiceError> {
//!         let time = batch.attribute("time_min").ok_or(ChoiceError::missing("time_min"))?;
//!         let length = batch.attribute("length_km").ok_or(ChoiceError::missing("length_km"))?;
//!         let utility: Vec<f64> = time.iter().zip(length).map(|(t, l)| -0.1 * t - 0.2 * l).collect();
//!         Ok(sample_random_utility(batch, &utility, rng))
//!     }
//! }
//! ```
//!
//! # Common random numbers
//!
//! Every random draw is keyed on `(traveller, trip, iteration, alternative
//! identity)` (Foundations §4), never on position, so a design that adds or
//! removes an alternative leaves the noise of every other alternative — and, in
//! a Gumbel-max model, the choice of everyone who did not prefer the removed
//! alternative — exactly as it was.

use std::collections::BTreeMap;
use std::fmt;

use openmobisim_core_types::rng::{DrawAddress, StreamRng};

use crate::batch::{ChoiceBatch, Choices};

/// The model used unless another is asked for.
pub const DEFAULT_MODEL: &str = "deterministic";

/// A model's options: numbers by name. Unknown names are an error, not ignored.
pub type Options = BTreeMap<String, f64>;

/// Why a choice could not be made.
#[derive(Clone, PartialEq, Debug)]
pub enum ChoiceError {
    /// No model has this name.
    UnknownModel {
        /// The name asked for.
        name: String,
        /// The names that exist.
        known: Vec<String>,
    },
    /// The model has no such option.
    UnknownOption {
        /// The model.
        model: String,
        /// The option asked for.
        option: String,
        /// What the model accepts, described.
        known: String,
    },
    /// An option's value is not allowed.
    BadOption {
        /// The model.
        model: String,
        /// The option.
        option: String,
        /// What is wrong with it.
        reason: String,
    },
    /// The model reads an attribute the alternatives do not carry.
    MissingAttribute {
        /// The attribute.
        name: String,
        /// The attributes on offer.
        offered: Vec<String>,
    },
    /// The batch is malformed.
    BadBatch(String),
    /// The model's answer does not fit the batch.
    BadAnswer(String),
    /// The model itself failed (a Python model raised, say).
    Failed(String),
}

impl ChoiceError {
    /// [`ChoiceError::MissingAttribute`] with no list of what is on offer, for a
    /// model that only knows the one name.
    #[must_use]
    pub fn missing(name: &str) -> Self {
        Self::MissingAttribute { name: name.to_string(), offered: Vec::new() }
    }
}

impl fmt::Display for ChoiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownModel { name, known } => {
                write!(f, "no choice model called {name:?}; the models are: {}", known.join(", "))
            }
            Self::UnknownOption { model, option, known } => {
                write!(f, "choice model {model:?} has no option {option:?}; it accepts {known}")
            }
            Self::BadOption { model, option, reason } => {
                write!(f, "choice model {model:?}, option {option:?}: {reason}")
            }
            Self::MissingAttribute { name, offered } if offered.is_empty() => {
                write!(
                    f,
                    "the choice model needs the attribute {name:?}, which the alternatives do not carry"
                )
            }
            Self::MissingAttribute { name, offered } => write!(
                f,
                "the choice model needs the attribute {name:?}; the alternatives carry: {}",
                offered.join(", ")
            ),
            Self::BadBatch(m) => write!(f, "the choice batch is malformed: {m}"),
            Self::BadAnswer(m) => write!(f, "the choice model's answer does not fit: {m}"),
            Self::Failed(m) => write!(f, "the choice model failed: {m}"),
        }
    }
}

impl std::error::Error for ChoiceError {}

/// A way of choosing. See the [module docs](self) for how to write one.
pub trait ChoiceModel: Send + Sync {
    /// The model's name, as it is selected.
    fn name(&self) -> &str;

    /// The model's name and every option with defaults filled in, as a canonical
    /// string. It is part of the run's fingerprint, so two settings of a model
    /// are never mistaken for one.
    fn descriptor(&self) -> String;

    /// Whether the model draws from the choice stream. A run records the stream
    /// as live in its manifest exactly when it does.
    fn is_sampled(&self) -> bool;

    /// The attributes the model reads, or `None` if it may read any (a Python
    /// model): the run computes only what is asked for, or everything.
    fn required_attributes(&self) -> Option<Vec<String>>;

    /// Choose for every situation in `batch`. `rng` is the run's choice stream;
    /// a model that draws must key its draws on the situation's traveller and
    /// trip, the batch's iteration and the alternative's identity (see
    /// [`gumbel`]).
    ///
    /// # Errors
    ///
    /// [`ChoiceError`] if an attribute is missing or the model fails.
    fn choose(&self, batch: &ChoiceBatch, rng: &StreamRng) -> Result<Choices, ChoiceError>;
}

// --- helpers a new model builds on ------------------------------------------------

/// The standard Gumbel(0, 1) error for one alternative of one situation:
/// `−ln(−ln U)` with `U` the choice-stream draw at address `(traveller, trip,
/// iteration, alternative identity)`.
///
/// Adding a Gumbel to each utility and taking the largest is an exact draw from
/// the logit (the Gumbel-max construction).
#[must_use]
pub fn gumbel(rng: &StreamRng, traveller: u32, trip: u32, iteration: u32, alternative: u32) -> f64 {
    let u = rng.block(DrawAddress::from_quad(traveller, trip, iteration, alternative)).open_unit(0);
    -(-u.ln()).ln()
}

/// The Gumbel error of every alternative in `batch`, keyed on identity.
#[must_use]
pub fn gumbel_noise(batch: &ChoiceBatch, rng: &StreamRng) -> Vec<f64> {
    let mut noise = Vec::with_capacity(batch.alternatives());
    for s in 0..batch.situations() {
        let (traveller, trip) = (batch.travellers()[s], batch.trips()[s]);
        for &identity in &batch.identities()[batch.range(s)] {
            noise.push(gumbel(rng, traveller, trip, batch.iteration(), identity));
        }
    }
    noise
}

/// The index of the largest value, the first of any that tie.
///
/// # Panics
///
/// Panics if `values` is empty.
#[must_use]
pub fn first_argmax(values: &[f64]) -> usize {
    let mut best = 0;
    for (i, v) in values.iter().enumerate().skip(1) {
        if *v > values[best] {
            best = i;
        }
    }
    best
}

/// The logit probabilities of one situation's `utilities`: `exp(vᵢ) / Σ exp(vⱼ)`,
/// computed with the largest utility subtracted so it cannot overflow. Exact —
/// never tabulated (S75).
#[must_use]
pub fn logit_probabilities(utilities: &[f64]) -> Vec<f64> {
    let top = utilities.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let weights: Vec<f64> = utilities.iter().map(|v| (v - top).exp()).collect();
    let total: f64 = weights.iter().sum();
    weights.iter().map(|w| w / total).collect()
}

/// The random-utility choice for a whole batch: each situation takes the
/// alternative with the largest `utility + Gumbel error`, which is an exact
/// draw from the logit of `utility`. `utility` has one value per alternative.
///
/// The probability reported for each choice is the logit probability of the
/// alternative chosen.
///
/// # Panics
///
/// Panics if `utility` does not have one value per alternative.
#[must_use]
pub fn sample_random_utility(batch: &ChoiceBatch, utility: &[f64], rng: &StreamRng) -> Choices {
    assert_eq!(utility.len(), batch.alternatives(), "one utility per alternative");
    let noise = gumbel_noise(batch, rng);
    let mut chosen = Vec::with_capacity(batch.situations());
    let mut probability = Vec::with_capacity(batch.situations());
    let mut scratch: Vec<f64> = Vec::new();
    for s in 0..batch.situations() {
        let range = batch.range(s);
        scratch.clear();
        scratch.extend(range.clone().map(|a| utility[a] + noise[a]));
        let pick = first_argmax(&scratch);
        let probs = logit_probabilities(&utility[range]);
        #[allow(
            clippy::cast_possible_truncation,
            reason = "an index within one situation's few alternatives"
        )]
        chosen.push(pick as u32);
        probability.push(probs[pick]);
    }
    Choices { chosen, probability }
}

// --- the registry -----------------------------------------------------------------

/// Makes a model from its options.
pub type Factory = fn(&Options) -> Result<Box<dyn ChoiceModel>, ChoiceError>;

/// Models by name. [`Registry::builtin`] has the built-in ones; a researcher's
/// model is added with [`Registry::register`] and then selected by name like
/// any other.
#[derive(Clone, Debug)]
pub struct Registry {
    models: Vec<(String, Factory)>,
}

impl Registry {
    /// The built-in models: `deterministic` and `logit`.
    #[must_use]
    pub fn builtin() -> Self {
        let mut r = Self { models: Vec::new() };
        r.register("deterministic", |o| Ok(Box::new(crate::Deterministic::from_options(o)?)));
        r.register("logit", |o| Ok(Box::new(crate::Logit::from_options(o)?)));
        r
    }

    /// Add a model under `name`, or replace one of that name.
    pub fn register(&mut self, name: &str, factory: Factory) {
        match self.models.iter_mut().find(|(n, _)| n == name) {
            Some(entry) => entry.1 = factory,
            None => self.models.push((name.to_string(), factory)),
        }
    }

    /// The names that can be selected, in registration order.
    #[must_use]
    pub fn names(&self) -> Vec<&str> {
        self.models.iter().map(|(n, _)| n.as_str()).collect()
    }

    /// Make the model called `name` from `options`.
    ///
    /// # Errors
    ///
    /// [`ChoiceError::UnknownModel`] if there is none, or whatever the model's
    /// own validation says about its options.
    pub fn create(
        &self,
        name: &str,
        options: &Options,
    ) -> Result<Box<dyn ChoiceModel>, ChoiceError> {
        match self.models.iter().find(|(n, _)| n == name) {
            Some((_, factory)) => factory(options),
            None => Err(ChoiceError::UnknownModel {
                name: name.to_string(),
                known: self.names().into_iter().map(str::to_string).collect(),
            }),
        }
    }
}

/// Make a built-in model by name; see [`Registry::builtin`].
///
/// # Errors
///
/// As [`Registry::create`].
pub fn model(name: &str, options: &Options) -> Result<Box<dyn ChoiceModel>, ChoiceError> {
    Registry::builtin().create(name, options)
}

/// The default model with its default options.
#[must_use]
pub fn default_model() -> Box<dyn ChoiceModel> {
    Box::new(crate::Deterministic)
}
