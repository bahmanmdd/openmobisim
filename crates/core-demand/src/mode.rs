//! A trip's mode: the optional `mode` column of `trips.parquet`.
//!
//! A trip with a mode keeps it; a trip without one is a car trip until mode
//! choice exists, and then chooses. The values name the mode sequences
//! by their main mode; the ones this build cannot simulate yet
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
    /// By a shared bike of a docked service: walk to a dock, ride, return it at a dock, walk.
    /// No bike of one's own.
    BikeShare = 6,
}

impl Mode {
    /// Every mode, in discriminant order.
    pub const ALL: [Mode; 7] = [
        Mode::Car,
        Mode::Bike,
        Mode::Walk,
        Mode::Transit,
        Mode::CarTransit,
        Mode::BikeTransit,
        Mode::BikeShare,
    ];

    /// How many modes there are.
    pub const COUNT: usize = Mode::ALL.len();

    /// The modes that need no service of the run, in discriminant order: what the outputs of a
    /// run without services list, so they stay what they were before services existed.
    pub const BASE: [Mode; 6] =
        [Mode::Car, Mode::Bike, Mode::Walk, Mode::Transit, Mode::CarTransit, Mode::BikeTransit];

    /// The modes a run's outputs list (its reasons per mode, its gaps per mode): the
    /// [`Self::BASE`] modes, and the service modes too when the run has a service.
    #[must_use]
    pub fn listed(with_services: bool) -> &'static [Mode] {
        if with_services { &Mode::ALL } else { &Mode::BASE }
    }

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
            Mode::BikeShare => "bike_share",
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
            Mode::Walk | Mode::Transit | Mode::BikeShare => None,
        }
    }

    /// Whether the mode needs a service of the run (a shared bike): offered, and written in the
    /// outputs, only in a run that has one.
    #[must_use]
    pub const fn is_service(self) -> bool {
        matches!(self, Mode::BikeShare)
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

    #[test]
    fn the_base_modes_are_those_without_a_service_in_order() {
        assert!(Mode::BASE.iter().all(|m| !m.is_service()));
        assert_eq!(&Mode::ALL[..Mode::BASE.len()], &Mode::BASE);
        assert!(Mode::ALL[Mode::BASE.len()..].iter().all(|m| m.is_service()));
        assert_eq!(Mode::listed(false), &Mode::BASE);
        assert_eq!(Mode::listed(true), &Mode::ALL);
    }
}
