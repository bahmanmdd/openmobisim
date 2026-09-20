//! The choice layer (Phase 2 item 7; design §4).
//!
//! | Module | What it holds |
//! |---|---|
//! | [`batch`] | [`ChoiceBatch`] (situations × alternatives, with named attributes and stable identities) and [`Choices`] (the answer) |
//! | [`model`](mod@model) | The [`ChoiceModel`] interface, the helpers a new model builds on ([`gumbel`], [`sample_random_utility`], …), the by-name [`Registry`] |
//! | [`builtin`] | [`Deterministic`] (the default) and [`Logit`] (with the path-size term, the standard route-choice model) |
//!
//! # Extending it
//!
//! A new choice model is a type that implements [`ChoiceModel`]. Add a factory
//! to a [`Registry`] to select it by name like the built-in ones; the run, the
//! fingerprint, the manifest and (through `py-bindings`) the Python surface take
//! no other change. A random utility model is a utility function and one call:
//! [`sample_random_utility`]. A model written in Python plugs in the same way,
//! called once per batch.
//!
//! **Nothing here knows what is being chosen.** Routes today; modes, hubs and
//! departure times later, behind the same interface (design §11.5).
//!
//! **Draws** are keyed on `(traveller, trip, iteration, alternative identity)`
//! from the choice stream, so results do not depend on thread count, batch
//! size or order, and adding or removing an alternative leaves everyone else's
//! draws alone.

pub mod batch;
pub mod builtin;
pub mod model;

pub use batch::{ChoiceBatch, Choices};
pub use builtin::{Deterministic, Logit};
pub use model::{
    ChoiceError, ChoiceModel, DEFAULT_MODEL, Factory, Options, Registry, default_model,
    first_argmax, gumbel, gumbel_noise, logit_probabilities, model, sample_random_utility,
};
