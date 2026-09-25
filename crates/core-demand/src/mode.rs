//! A trip's mode (S195): the optional `mode` column of `trips.parquet`.
//!
//! A trip with a mode keeps it; a trip without one is a car trip until mode
//! choice exists, and then chooses. The values name the mode sequences of
//! design §22.2 by their main mode; the ones this build cannot simulate yet
//! (transit and the combinations with it) are read, kept and reported as not
//! available, never silently turned into car trips.

use crate::DemandError;
use crate::vehicles::VehicleKind;

/// How a trip is made.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
#[repr(u8)]
pub enum Mode {
    /// By the traveller's own car.
    #[default]
    Car = 0,
    /// By the traveller's own bike.
    Bike = 1,
    /// On foot.
    Walk = 2,
    /// By scheduled public transport, walking to and from it.
    Transit = 3,
    /// By car to a hub, then public transport (park-and-ride).
    CarTransit = 4,
    /// By bike to a hub, then public transport (bike-and-ride).
    BikeTransit = 5,
}

impl Mode {
    /// Every mode, in discriminant order.
    pub const ALL: [Mode; 6] =
        [Mode::Car, Mode::Bike, Mode::Walk, Mode::Transit, Mode::CarTransit, Mode::BikeTransit];

    /// How many modes there are.
    pub const COUNT: usize = Mode::ALL.len();

    /// The stable snake_case name, as the `mode` column writes it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Mode::Car => "car",
            Mode::Bike => "bike",
            Mode::Walk => "walk",
            Mode::Transit => "transit",
            Mode::CarTransit => "car_transit",
            Mode::BikeTransit => "bike_transit",
        }
    }

    /// The mode a `mode` value names.
    ///
    /// # Errors
    ///
    /// [`DemandError::Schema`] naming the accepted values, for any other: a
    /// mode is a closed vocabulary, and guessing would turn a typo into a car
    /// trip.
    pub fn from_name(name: &str) -> Result<Self, DemandError> {
        Mode::ALL.into_iter().find(|m| m.as_str() == name).ok_or_else(|| {
            let accepted: Vec<&str> = Mode::ALL.iter().map(|m| m.as_str()).collect();
            DemandError::Schema(format!(
                "unknown mode `{name}`; the accepted values are {}",
                accepted.join(", ")
            ))
        })
    }

    /// The position of this mode in [`Mode::ALL`], for per-mode tables.
    #[inline]
    #[must_use]
    pub const fn index(self) -> usize {
        self as usize
    }

    /// The traveller's own vehicle the trip needs at its origin, if any.
    #[must_use]
    pub const fn vehicle(self) -> Option<VehicleKind> {
        match self {
            Mode::Car | Mode::CarTransit => Some(VehicleKind::Car),
            Mode::Bike | Mode::BikeTransit => Some(VehicleKind::Bike),
            Mode::Walk | Mode::Transit => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_and_anything_else_is_refused_with_the_list() {
        for mode in Mode::ALL {
            assert_eq!(Mode::from_name(mode.as_str()).unwrap(), mode);
            assert_eq!(Mode::ALL[mode.index()], mode);
        }
        let err = Mode::from_name("Bike").unwrap_err().to_string();
        assert!(err.contains("`Bike`") && err.contains("car, bike, walk"), "{err}");
        assert_eq!(Mode::default(), Mode::Car);
    }
}
