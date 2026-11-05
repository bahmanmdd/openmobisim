//! Metric units as zero-cost newtypes over `f64`.
//!
//! Inside the simulator every quantity is in metres, seconds, metres per second and vehicles
//! per second. Kilometres per hour and vehicles per hour exist only at the boundary: the
//! constructors [`Speed::from_km_per_hour`] and [`Flow::from_veh_per_hour`] convert once, as a
//! value is read, and nothing downstream divides by 3 600 again.
//!
//! There is no addition between different quantities and no implicit conversion. The physically
//! meaningful cross-unit operations are written out at the end of this module, and the list is
//! short on purpose. A bike, a walker and a car share these units; there is no passenger-car
//! unit, so a flow counts vehicles.

use core::fmt;
use core::iter::Sum;
use core::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

/// Seconds in an hour, for the two boundary conversions.
pub const SECONDS_PER_HOUR: f64 = 3600.0;
/// Metres in a kilometre.
pub const METRES_PER_KM: f64 = 1000.0;

macro_rules! define_unit {
    ($(#[$doc:meta])* $name:ident, $unit:literal) => {
        $(#[$doc])*
        ///
        #[doc = concat!("Unit: **", $unit, "**.")]
        #[derive(Clone, Copy, PartialEq, PartialOrd, Default)]
        #[repr(transparent)]
        pub struct $name(pub f64);

        impl $name {
            /// Zero.
            pub const ZERO: Self = Self(0.0);
            /// Positive infinity: the value of "no limit".
            pub const INFINITE: Self = Self(f64::INFINITY);

            /// The value, in the unit named above.
            #[inline]
            #[must_use]
            pub const fn get(self) -> f64 {
                self.0
            }

            /// Whether the value is finite (neither infinite nor NaN).
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

        /// Like divided by like is a dimensionless ratio.
        impl Div for $name {
            type Output = f64;
            #[inline]
            fn div(self, rhs: Self) -> f64 { self.0 / rhs.0 }
        }

        impl Sum for $name {
            /// Sequential, left to right. For a large parallel sum use [`crate::reduce`].
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
define_unit!(/// A speed. For a bike or a walker, the free-flow speed.
    Speed, "m/s");
define_unit!(/// An elapsed time, fractional.
    ///
    /// The clock is whole seconds ([`crate::time::Second`]); this is the type of durations
    /// computed from lengths, speeds and readings, fractional until they are put on the clock.
    Duration, "s");
define_unit!(/// A flow rate, in vehicles per second.
    ///
    /// A capacity is given in vehicles per hour; [`Flow::from_veh_per_hour`] converts it once.
    Flow, "veh/s");
define_unit!(/// The cost of an alternative: the systematic utility of the run's choice model, each
    /// attribute (time, money and the others) weighted by its coefficient.
    ///
    /// In a time-only run (time coefficient 1, the others 0) it equals the travel time in
    /// seconds. The choice module applies the coefficients; this type only carries the value.
    Cost, "cost");

impl Speed {
    /// From kilometres per hour: a boundary constructor (36 km/h is 10 m/s).
    #[inline]
    #[must_use]
    pub fn from_km_per_hour(km_per_hour: f64) -> Self {
        Self(km_per_hour * METRES_PER_KM / SECONDS_PER_HOUR)
    }
}

impl Flow {
    /// From vehicles per hour: the boundary constructor (1 800 veh/h is 0.5 veh/s).
    #[inline]
    #[must_use]
    pub fn from_veh_per_hour(veh_per_hour: f64) -> Self {
        Self(veh_per_hour / SECONDS_PER_HOUR)
    }

    /// In vehicles per hour, for outputs.
    #[inline]
    #[must_use]
    pub fn as_veh_per_hour(self) -> f64 {
        self.0 * SECONDS_PER_HOUR
    }
}

// The cross-unit operations, in full. Adding to this list is a design decision.

/// `length / speed = duration`: the free-flow time of a link.
impl Div<Speed> for Metres {
    type Output = Duration;
    #[inline]
    fn div(self, rhs: Speed) -> Duration {
        Duration(self.0 / rhs.0)
    }
}

/// `length / duration = speed`: the mean speed over a traversal.
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

/// `flow × duration = vehicles`: how many vehicles a flow passes in a duration.
impl Mul<Duration> for Flow {
    type Output = f64;
    #[inline]
    fn mul(self, rhs: Duration) -> f64 {
        self.0 * rhs.0
    }
}

/// `duration × flow = vehicles`.
impl Mul<Flow> for Duration {
    type Output = f64;
    #[inline]
    fn mul(self, rhs: Flow) -> f64 {
        self.0 * rhs.0
    }
}
