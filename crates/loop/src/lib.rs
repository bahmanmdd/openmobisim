//! The loop: search, choose, load, measure, assess, iterated to the stopping rule.
//!
//! [`run::run`] takes a network, a demand, a timetable or none, a window and the parameters,
//! and returns a [`run::Run`]: the trips table, the links-by-bin table of the last loading and
//! the iterations table. Every iteration is the five stages of `README.md`, nothing else.

pub mod alternative;
pub mod assess;
pub mod choose;
pub mod context;
pub mod load;
pub mod parameters;
pub mod run;
pub mod search;

/// The whole second at or after `t`, a count of seconds at or above zero: the second of the
/// timetable a traveller is at a stop on. Route legs are entered at their exact instant.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "t is at or above zero and a second of the window, far below u32::MAX"
)]
#[must_use]
pub(crate) fn whole(t: f64) -> u32 {
    t.ceil() as u32
}

/// `a / b` as a share, `NaN` when `b` is zero.
#[allow(clippy::cast_precision_loss, reason = "counts of trips, far below 2^53")]
#[must_use]
pub(crate) fn share(a: u64, b: u64) -> f64 {
    if b == 0 { f64::NAN } else { a as f64 / b as f64 }
}
