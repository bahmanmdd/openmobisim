//! Internal units: SI, per-second, and enforced by the type system
//! (Foundations §9).
//!
//! # The rule
//!
//! **Internal units are SI and per-second, and conversion happens exactly once,
//! at the scenario build boundary.** Capacities are written in scenario files
//! as veh/h because that is how engineers write them; they become veh/s the
//! moment they are read, and nothing downstream ever divides by 3600 again.
//! The same goes for km/h → m/s and km → m.
//!
//! This is a performance decision as much as a correctness one. A conversion
//! inside a per-link, per-step loop is a multiply that buys nothing, and the
//! alternative to enforcing it — remembering — does not survive contact with a
//! second contributor.
//!
//! # Zero cost
//!
//! Every type here is `#[repr(transparent)]` over an `f64` with `#[inline]`
//! operators, so `Vec<Metres>` has exactly the layout of `Vec<f64>` and the
//! generated code is identical to raw floats. `tests/layout.rs` asserts the
//! sizes so that a future field cannot quietly change this.
//!
//! # What is deliberately missing
//!
//! There is no `Add` between different quantities and no implicit conversion.
//! Where a cross-unit operation is physically meaningful — `Metres / Speed`
//! gives a [`Duration`], `Flow * Duration` gives [`Pcu`] — it is implemented
//! explicitly, and the list of such operations is short on purpose.

use core::fmt;
use core::iter::Sum;
use core::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

/// Seconds per hour, for the one conversion boundary that needs it.
pub const SECONDS_PER_HOUR: f64 = 3600.0;
/// Metres per kilometre.
pub const METRES_PER_KM: f64 = 1000.0;

macro_rules! define_unit {
    ($(#[$doc:meta])* $name:ident, $unit:literal) => {
        $(#[$doc])*
        ///
        #[doc = concat!("Internal unit: **", $unit, "**.")]
        #[derive(Clone, Copy, PartialEq, PartialOrd, Default)]
        #[repr(transparent)]
        pub struct $name(pub f64);

        impl $name {
            /// Zero.
            pub const ZERO: Self = Self(0.0);
            /// Positive infinity — the "no limit" value for a cap or a cutoff.
            pub const INFINITE: Self = Self(f64::INFINITY);

            /// The underlying value, in the unit named above.
            #[inline]
            #[must_use]
            pub const fn get(self) -> f64 {
                self.0
            }

            /// Whether the value is finite and not NaN.
            ///
            /// Scenario validation uses this; the hot path does not.
            #[inline]
            #[must_use]
            pub fn is_finite(self) -> bool {
                self.0.is_finite()
            }

            /// The larger of two values.
            #[inline]
            #[must_use]
            pub fn max(self, other: Self) -> Self {
                Self(self.0.max(other.0))
            }

            /// The smaller of two values.
            #[inline]
            #[must_use]
            pub fn min(self, other: Self) -> Self {
                Self(self.0.min(other.0))
            }

            /// Clamped to `[lo, hi]`.
            #[inline]
            #[must_use]
            pub fn clamp(self, lo: Self, hi: Self) -> Self {
                Self(self.0.clamp(lo.0, hi.0))
            }

            /// Clamped below at zero.
            ///
            /// Cumulative-curve arithmetic produces small negative values from
            /// rounding; this is the sanctioned way to absorb them.
            #[inline]
            #[must_use]
            pub fn non_negative(self) -> Self {
                Self(self.0.max(0.0))
            }
        }

        impl Add for $name {
            type Output = Self;
            #[inline]
            fn add(self, rhs: Self) -> Self { Self(self.0 + rhs.0) }
        }

        impl AddAssign for $name {
            #[inline]
            fn add_assign(&mut self, rhs: Self) { self.0 += rhs.0; }
        }

        impl Sub for $name {
            type Output = Self;
            #[inline]
            fn sub(self, rhs: Self) -> Self { Self(self.0 - rhs.0) }
        }

        impl SubAssign for $name {
            #[inline]
            fn sub_assign(&mut self, rhs: Self) { self.0 -= rhs.0; }
        }

        impl Neg for $name {
            type Output = Self;
            #[inline]
            fn neg(self) -> Self { Self(-self.0) }
        }

        impl Mul<f64> for $name {
            type Output = Self;
            #[inline]
            fn mul(self, rhs: f64) -> Self { Self(self.0 * rhs) }
        }

        impl Mul<$name> for f64 {
            type Output = $name;
            #[inline]
            fn mul(self, rhs: $name) -> $name { $name(self * rhs.0) }
        }

        impl Div<f64> for $name {
            type Output = Self;
            #[inline]
            fn div(self, rhs: f64) -> Self { Self(self.0 / rhs) }
        }

        /// Dividing like by like gives a dimensionless ratio.
        impl Div for $name {
            type Output = f64;
            #[inline]
            fn div(self, rhs: Self) -> f64 { self.0 / rhs.0 }
        }

        impl Sum for $name {
            /// Sequential, left-to-right — deterministic by construction.
            ///
            /// For a parallel sum use [`crate::reduce`]; never
            /// `par_iter().sum()`.
            #[inline]
            fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
                iter.fold(Self::ZERO, Add::add)
            }
        }

        impl<'a> Sum<&'a $name> for $name {
            #[inline]
            fn sum<I: Iterator<Item = &'a Self>>(iter: I) -> Self {
                iter.copied().fold(Self::ZERO, Add::add)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!(stringify!($name), "({} ", $unit, ")"), self.0)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!("{} ", $unit), self.0)
            }
        }
    };
}

define_unit!(/// A length.
    Metres, "m");
define_unit!(/// A speed.
    Speed, "m/s");
define_unit!(/// An elapsed time, fractional.
    ///
    /// The master clock is integer seconds ([`crate::time::Second`]); this is
    /// the type for *durations* computed from curves and costs, which are
    /// fractional until they are floored onto the clock.
    Duration, "s");
define_unit!(/// A flow rate, in passenger-car units per second.
    ///
    /// Scenario files give capacities in veh/h; use
    /// [`Flow::from_veh_per_hour`] at the build boundary and never divide by
    /// 3600 again.
    Flow, "PCU/s");
define_unit!(/// A density, in passenger-car units per metre.
    Density, "PCU/m");
define_unit!(/// A quantity of traffic, in passenger-car units.
    ///
    /// The unit of the cumulative curves. One car is 1.0 PCU by definition; a
    /// bus is more.
    Pcu, "PCU");
define_unit!(/// A generalised cost, expressed in seconds-equivalent.
    ///
    /// Time is the numeraire: money terms are divided by a value of time at
    /// the build boundary, so that everything the choice model compares is one
    /// commensurable quantity.
    Cost, "s-equiv");

impl Metres {
    /// From kilometres — a build-boundary constructor.
    #[inline]
    #[must_use]
    pub fn from_km(km: f64) -> Self {
        Self(km * METRES_PER_KM)
    }

    /// In kilometres, for reporting.
    #[inline]
    #[must_use]
    pub fn as_km(self) -> f64 {
        self.0 / METRES_PER_KM
    }
}

impl Speed {
    /// From km/h — a build-boundary constructor.
    #[inline]
    #[must_use]
    pub fn from_km_per_hour(kmh: f64) -> Self {
        Self(kmh * METRES_PER_KM / SECONDS_PER_HOUR)
    }

    /// In km/h, for reporting.
    #[inline]
    #[must_use]
    pub fn as_km_per_hour(self) -> f64 {
        self.0 * SECONDS_PER_HOUR / METRES_PER_KM
    }
}

impl Duration {
    /// From minutes.
    #[inline]
    #[must_use]
    pub fn from_minutes(minutes: f64) -> Self {
        Self(minutes * 60.0)
    }

    /// From hours.
    #[inline]
    #[must_use]
    pub fn from_hours(hours: f64) -> Self {
        Self(hours * SECONDS_PER_HOUR)
    }

    /// In minutes, for reporting.
    #[inline]
    #[must_use]
    pub fn as_minutes(self) -> f64 {
        self.0 / 60.0
    }

    /// In hours, for reporting.
    #[inline]
    #[must_use]
    pub fn as_hours(self) -> f64 {
        self.0 / SECONDS_PER_HOUR
    }

    /// From a whole number of clock seconds.
    #[inline]
    #[must_use]
    pub fn from_clock(seconds: crate::time::Second) -> Self {
        Self(f64::from(seconds.get()))
    }
}

impl Flow {
    /// From vehicles (PCU) per hour — **the** build-boundary constructor.
    ///
    /// Scenario keys carry their units (`capacity_veh_per_hour`); this is the
    /// only place that unit becomes an internal one.
    #[inline]
    #[must_use]
    pub fn from_veh_per_hour(veh_per_hour: f64) -> Self {
        Self(veh_per_hour / SECONDS_PER_HOUR)
    }

    /// In veh/h, for reporting and for round-tripping a scenario file.
    #[inline]
    #[must_use]
    pub fn as_veh_per_hour(self) -> f64 {
        self.0 * SECONDS_PER_HOUR
    }
}

impl Density {
    /// From vehicles (PCU) per kilometre — a build-boundary constructor.
    #[inline]
    #[must_use]
    pub fn from_veh_per_km(veh_per_km: f64) -> Self {
        Self(veh_per_km / METRES_PER_KM)
    }

    /// In veh/km, for reporting.
    #[inline]
    #[must_use]
    pub fn as_veh_per_km(self) -> f64 {
        self.0 * METRES_PER_KM
    }
}

// --- The cross-unit operations, in full ------------------------------------
//
// This list is short on purpose. Adding to it is a design decision, not a
// convenience: every implicit conversion is a place a unit error can hide.

/// `length / speed = duration` — free-flow travel time.
impl Div<Speed> for Metres {
    type Output = Duration;
    #[inline]
    fn div(self, rhs: Speed) -> Duration {
        Duration(self.0 / rhs.0)
    }
}

/// `length / duration = speed` — the mean speed over a traversal.
impl Div<Duration> for Metres {
    type Output = Speed;
    #[inline]
    fn div(self, rhs: Duration) -> Speed {
        Speed(self.0 / rhs.0)
    }
}

/// `speed × duration = length`.
impl Mul<Duration> for Speed {
    type Output = Metres;
    #[inline]
    fn mul(self, rhs: Duration) -> Metres {
        Metres(self.0 * rhs.0)
    }
}

/// `duration × speed = length`.
impl Mul<Speed> for Duration {
    type Output = Metres;
    #[inline]
    fn mul(self, rhs: Speed) -> Metres {
        Metres(self.0 * rhs.0)
    }
}

/// `flow × duration = quantity` — how much a link can pass in one step.
impl Mul<Duration> for Flow {
    type Output = Pcu;
    #[inline]
    fn mul(self, rhs: Duration) -> Pcu {
        Pcu(self.0 * rhs.0)
    }
}

/// `duration × flow = quantity`.
impl Mul<Flow> for Duration {
    type Output = Pcu;
    #[inline]
    fn mul(self, rhs: Flow) -> Pcu {
        Pcu(self.0 * rhs.0)
    }
}

/// `quantity / duration = flow` — the rate implied by a curve increment.
impl Div<Duration> for Pcu {
    type Output = Flow;
    #[inline]
    fn div(self, rhs: Duration) -> Flow {
        Flow(self.0 / rhs.0)
    }
}

/// `quantity / length = density` — vehicles per metre on a link.
impl Div<Metres> for Pcu {
    type Output = Density;
    #[inline]
    fn div(self, rhs: Metres) -> Density {
        Density(self.0 / rhs.0)
    }
}

/// `density × length = quantity` — the storage capacity of a link.
impl Mul<Metres> for Density {
    type Output = Pcu;
    #[inline]
    fn mul(self, rhs: Metres) -> Pcu {
        Pcu(self.0 * rhs.0)
    }
}

/// `length × density = quantity`.
impl Mul<Density> for Metres {
    type Output = Pcu;
    #[inline]
    fn mul(self, rhs: Density) -> Pcu {
        Pcu(self.0 * rhs.0)
    }
}

/// `density × speed = flow` — the fundamental relation q = k·v.
impl Mul<Speed> for Density {
    type Output = Flow;
    #[inline]
    fn mul(self, rhs: Speed) -> Flow {
        Flow(self.0 * rhs.0)
    }
}

/// `speed × density = flow`.
impl Mul<Density> for Speed {
    type Output = Flow;
    #[inline]
    fn mul(self, rhs: Density) -> Flow {
        Flow(self.0 * rhs.0)
    }
}

/// `flow / density = speed` — the space-mean speed at a point on the diagram.
impl Div<Density> for Flow {
    type Output = Speed;
    #[inline]
    fn div(self, rhs: Density) -> Speed {
        Speed(self.0 / rhs.0)
    }
}

/// `flow / speed = density`.
impl Div<Speed> for Flow {
    type Output = Density;
    #[inline]
    fn div(self, rhs: Speed) -> Density {
        Density(self.0 / rhs.0)
    }
}

/// A duration is a cost when time is the numeraire.
impl From<Duration> for Cost {
    #[inline]
    fn from(d: Duration) -> Cost {
        Cost(d.0)
    }
}
