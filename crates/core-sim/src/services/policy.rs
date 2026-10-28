//! Operators: the code that steers the services during a day.
//!
//! A **policy** (an operator, in code) is called by the fleet engine every [`Policy::step_s`]
//! seconds, the first at the window's start, with a read-only [`View`] of the state; it returns
//! [`Decision`]s, which the engine checks, refuses with a reason when they cannot be carried
//! out (the day goes on), and carries out otherwise. A policy never changes the simulation
//! itself. One policy may manage every service at once (a **hub manager**): the view holds every
//! service's section.
//!
//! A policy's own failure (`Err` from [`Policy::step`]) stops the day: the engine returns the
//! message.
//!
//! **Determinism**: a policy's own draws come from [`View::rng_seed`], one number per step from
//! the stream `fleet_policy` keyed by the day's seed and the step; the same day, policy and seed
//! give the same decisions. A copied engine copies its policy ([`PolicyClone`]), so the copy
//! continues as the original would.

use std::fmt;

use super::ServiceSetup;
use super::decisions::Decision;
use super::view::View;

/// What a policy is, for the run's fingerprint and manifest: its name and parameters.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PolicyDescription {
    /// Its name (`"noop"` for [`NoOp`]).
    pub name: String,
    /// Its parameters, by name, in the order it gives them.
    pub parameters: Vec<(String, f64)>,
}

/// What a policy is told when a day begins.
#[derive(Clone, Copy, Debug)]
pub struct DayInfo<'a> {
    /// The window's start and end, in seconds from midnight.
    pub window: (f64, f64),
    /// The day's seed.
    pub seed: u64,
    /// The services, in the run's order (a decision names a service by its index here).
    pub services: &'a [ServiceSetup],
}

/// An operator: see the [module docs](self).
pub trait Policy: Send + PolicyClone {
    /// Its name and parameters.
    fn describe(&self) -> PolicyDescription;

    /// The seconds between its steps, the first at the window's start (900 by convention). A
    /// value that is not a positive number stops the day before its first step.
    fn step_s(&self) -> f64;

    /// Its decisions at this step, appended to `out`.
    ///
    /// # Errors
    ///
    /// Its own failure, which stops the day with the message.
    fn step(&mut self, view: &View<'_>, out: &mut Vec<Decision>) -> Result<(), String>;

    /// A new day begins (an operator with state resets here). Called once, when the engine is
    /// built.
    fn day_start(&mut self, day: &DayInfo<'_>) {
        let _ = day;
    }
}

/// A copy of a policy, its state included, for a copied engine. Every `Clone` policy has it.
pub trait PolicyClone {
    /// A boxed copy.
    fn clone_box(&self) -> Box<dyn Policy>;
}

impl<T: Policy + Clone + 'static> PolicyClone for T {
    fn clone_box(&self) -> Box<dyn Policy> {
        Box::new(self.clone())
    }
}

/// The policy that decides nothing: the day is the day without an operator, with steps.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NoOp {
    /// The seconds between steps.
    pub step_s: f64,
}

impl NoOp {
    /// Steps every 900 s.
    pub const DEFAULT: Self = Self { step_s: 900.0 };

    /// Steps every `step_s` seconds.
    #[must_use]
    pub const fn every(step_s: f64) -> Self {
        Self { step_s }
    }
}

impl Default for NoOp {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl Policy for NoOp {
    fn describe(&self) -> PolicyDescription {
        PolicyDescription {
            name: "noop".to_string(),
            parameters: vec![("step_s".into(), self.step_s)],
        }
    }

    fn step_s(&self) -> f64 {
        self.step_s
    }

    fn step(&mut self, _: &View<'_>, _: &mut Vec<Decision>) -> Result<(), String> {
        Ok(())
    }
}

/// An engine's policy: a box that copies and prints.
pub(crate) struct PolicySlot(pub(crate) Box<dyn Policy>);

impl Clone for PolicySlot {
    fn clone(&self) -> Self {
        Self(self.0.clone_box())
    }
}

impl fmt::Debug for PolicySlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Policy({:?})", self.0.describe())
    }
}
