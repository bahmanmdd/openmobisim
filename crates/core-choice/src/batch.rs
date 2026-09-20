//! What a choice model is asked, and what it answers (design §4).
//!
//! A [`ChoiceBatch`] is **situations × alternatives**: each situation (one
//! traveller's one trip, at one iteration) offers a handful of alternatives,
//! each with the same named numeric **attributes** (time, length, overlap, …)
//! and an **identity** that does not depend on which other alternatives are
//! offered. Layout is flat and columnar, so a model reads one attribute for all
//! alternatives as one slice, and the whole batch crosses to Python as arrays.
//!
//! The batch says nothing about *what* is being chosen: routes today, modes and
//! hubs later. That is what lets a new model, or a new kind of choice, reuse
//! everything else.

use std::ops::Range;

use crate::ChoiceError;

/// Situations × alternatives, with attributes. See the [module docs](self).
#[derive(Clone, Debug, PartialEq)]
pub struct ChoiceBatch {
    iteration: u32,
    names: Vec<String>,
    traveller: Vec<u32>,
    trip: Vec<u32>,
    /// `situations() + 1` entries: situation `s` owns alternatives
    /// `offsets[s]..offsets[s + 1]`.
    offsets: Vec<u32>,
    identity: Vec<u32>,
    /// One column per attribute, each `alternatives()` long.
    columns: Vec<Vec<f64>>,
}

impl ChoiceBatch {
    /// An empty batch for `iteration`, whose alternatives carry the attributes
    /// called `attribute_names`, in that order.
    #[must_use]
    pub fn new(iteration: u32, attribute_names: &[&str]) -> Self {
        Self {
            iteration,
            names: attribute_names.iter().map(|n| (*n).to_string()).collect(),
            traveller: Vec::new(),
            trip: Vec::new(),
            offsets: vec![0],
            identity: Vec::new(),
            columns: vec![Vec::new(); attribute_names.len()],
        }
    }

    /// Empty the batch for reuse, keeping its allocations and its attribute names.
    pub fn clear(&mut self) {
        self.traveller.clear();
        self.trip.clear();
        self.offsets.truncate(1);
        self.identity.clear();
        self.columns.iter_mut().for_each(Vec::clear);
    }

    /// Start a new situation: `traveller`'s `trip`. Alternatives pushed next belong to it.
    pub fn begin_situation(&mut self, traveller: u32, trip: u32) {
        self.traveller.push(traveller);
        self.trip.push(trip);
        // The offset that closes this situation is written as its alternatives arrive.
        self.offsets.push(*self.offsets.last().unwrap_or(&0));
    }

    /// Add an alternative with `identity` and its `attributes` to the current situation.
    ///
    /// # Panics
    ///
    /// Panics if no situation has begun, or if `attributes` is not one value per
    /// attribute name.
    pub fn push_alternative(&mut self, identity: u32, attributes: &[f64]) {
        assert!(!self.traveller.is_empty(), "begin a situation before adding alternatives");
        assert_eq!(attributes.len(), self.columns.len(), "one value per attribute");
        self.identity.push(identity);
        for (column, value) in self.columns.iter_mut().zip(attributes) {
            column.push(*value);
        }
        *self.offsets.last_mut().expect("offsets is never empty") += 1;
    }

    /// The iteration this batch is for. Part of the address of every random draw.
    #[must_use]
    pub fn iteration(&self) -> u32 {
        self.iteration
    }

    /// How many situations.
    #[must_use]
    pub fn situations(&self) -> usize {
        self.traveller.len()
    }

    /// How many alternatives across all situations.
    #[must_use]
    pub fn alternatives(&self) -> usize {
        self.identity.len()
    }

    /// Situation `s`'s alternatives, as a range into every per-alternative slice.
    ///
    /// # Panics
    ///
    /// Panics if `s` is out of range.
    #[must_use]
    pub fn range(&self, s: usize) -> Range<usize> {
        self.offsets[s] as usize..self.offsets[s + 1] as usize
    }

    /// The situation offsets: `situations() + 1` entries.
    #[must_use]
    pub fn offsets(&self) -> &[u32] {
        &self.offsets
    }

    /// Each situation's traveller id.
    #[must_use]
    pub fn travellers(&self) -> &[u32] {
        &self.traveller
    }

    /// Each situation's trip id.
    #[must_use]
    pub fn trips(&self) -> &[u32] {
        &self.trip
    }

    /// Each alternative's identity: stable when other alternatives come and go.
    #[must_use]
    pub fn identities(&self) -> &[u32] {
        &self.identity
    }

    /// The attribute names, in column order.
    #[must_use]
    pub fn attribute_names(&self) -> &[String] {
        &self.names
    }

    /// The attribute called `name`, one value per alternative.
    #[must_use]
    pub fn attribute(&self, name: &str) -> Option<&[f64]> {
        self.names.iter().position(|n| n == name).map(|i| self.columns[i].as_slice())
    }

    /// The attribute in column `i`.
    ///
    /// # Panics
    ///
    /// Panics if `i` is out of range.
    #[must_use]
    pub fn column(&self, i: usize) -> &[f64] {
        &self.columns[i]
    }

    /// Check the batch is one a model can be given: every situation has an
    /// alternative, identities are distinct within a situation, and every
    /// attribute is a finite number.
    ///
    /// # Errors
    ///
    /// [`ChoiceError::BadBatch`] saying what is wrong and where.
    pub fn validate(&self) -> Result<(), ChoiceError> {
        for s in 0..self.situations() {
            let range = self.range(s);
            if range.is_empty() {
                return Err(ChoiceError::BadBatch(format!(
                    "situation {s} (trip {}) has no alternatives",
                    self.trip[s]
                )));
            }
            let ids = &self.identity[range];
            for (i, id) in ids.iter().enumerate() {
                if ids[..i].contains(id) {
                    return Err(ChoiceError::BadBatch(format!(
                        "situation {s} (trip {}) has two alternatives with identity {id}",
                        self.trip[s]
                    )));
                }
            }
        }
        for (name, column) in self.names.iter().zip(&self.columns) {
            if let Some(a) = column.iter().position(|v| !v.is_finite()) {
                return Err(ChoiceError::BadBatch(format!(
                    "attribute {name} is not finite at alternative {a}"
                )));
            }
        }
        Ok(())
    }
}

/// What a model answers: for each situation, which alternative, and how likely.
#[derive(Clone, Debug, PartialEq)]
pub struct Choices {
    /// Each situation's chosen alternative, as its index **within the situation**
    /// (0 is the first alternative offered).
    pub chosen: Vec<u32>,
    /// The probability the model gave the alternative it chose, or `NaN` when
    /// the model has no probabilities to give (a learned or a Python model may not).
    pub probability: Vec<f64>,
}

impl Choices {
    /// Check the answer fits `batch`: one choice per situation, each in range.
    ///
    /// # Errors
    ///
    /// [`ChoiceError::BadAnswer`] saying what is wrong.
    pub fn validate(&self, batch: &ChoiceBatch) -> Result<(), ChoiceError> {
        if self.chosen.len() != batch.situations() || self.probability.len() != batch.situations() {
            return Err(ChoiceError::BadAnswer(format!(
                "expected {} choices, got {} (and {} probabilities)",
                batch.situations(),
                self.chosen.len(),
                self.probability.len()
            )));
        }
        for (s, &c) in self.chosen.iter().enumerate() {
            let offered = batch.range(s).len();
            if c as usize >= offered {
                return Err(ChoiceError::BadAnswer(format!(
                    "situation {s} chose alternative {c}, but only {offered} were offered"
                )));
            }
        }
        Ok(())
    }
}
