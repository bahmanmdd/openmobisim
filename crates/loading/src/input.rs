//! What a loading is given: the vehicles, each with its trip, mode, route and departure; and the
//! checks that refuse a vehicle the loading cannot move.
//!
//! Nothing is repaired. A departure outside the window, an empty route, a route whose links do
//! not follow one another, a link that forbids the vehicle's mode: each is an error naming the
//! trip, and the loading does not run.

use core::fmt;

use openmobisim_graph::network::Network;
use openmobisim_types::ids::{EntityId, LinkId, TripId};
use openmobisim_types::mode::Mode;
use openmobisim_types::time::{Second, StepGrid};

/// One traveller's movement through the network: a car, a bike or a walker (a walker counts as
/// its own vehicle here).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Vehicle {
    /// The trip, which is also its traveller. Unique among the vehicles of one loading.
    pub trip: TripId,
    /// Car, bike or walk. Transit runs on its timetable and is refused here.
    pub mode: Mode,
    /// The links in the order travelled, at least one: the downstream node of each is the
    /// upstream node of the next, and every link allows the mode.
    pub route: Vec<LinkId>,
    /// When the vehicle enters the upstream end of its first link: a second inside the window.
    pub departure: Second,
}

/// Why a loading refused its vehicles. Each names the trip it refuses.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum LoadError {
    /// Two vehicles carry the same trip.
    DuplicateTrip(TripId),
    /// A vehicle's mode is transit, which runs on its timetable, not on links.
    Transit(TripId),
    /// A vehicle's route has no link.
    EmptyRoute(TripId),
    /// A vehicle departs before the window's start or at or after its end.
    DepartureOutsideWindow {
        /// The trip.
        trip: TripId,
        /// Its departure.
        departure: Second,
        /// The window's first second.
        start: Second,
        /// The first second after the window.
        end: Second,
    },
    /// A route names a link the network does not have.
    UnknownLink {
        /// The trip.
        trip: TripId,
        /// The link it names.
        link: LinkId,
    },
    /// A link of the route does not allow the vehicle's mode.
    ModeNotAllowed {
        /// The trip.
        trip: TripId,
        /// The link.
        link: LinkId,
        /// The vehicle's mode.
        mode: Mode,
    },
    /// Two links of a route do not follow one another: the downstream node of `from` is not the
    /// upstream node of `to`.
    NotConsecutive {
        /// The trip.
        trip: TripId,
        /// The earlier link.
        from: LinkId,
        /// The link after it in the route.
        to: LinkId,
    },
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateTrip(trip) => write!(f, "trip {trip} is given twice"),
            Self::Transit(trip) => {
                write!(f, "trip {trip} is by transit, which runs on its timetable, not on links")
            }
            Self::EmptyRoute(trip) => write!(f, "trip {trip} has an empty route"),
            Self::DepartureOutsideWindow { trip, departure, start, end } => write!(
                f,
                "trip {trip} departs at {departure}, outside the window from {start} to {end}"
            ),
            Self::UnknownLink { trip, link } => {
                write!(f, "trip {trip} names link {link}, which the network does not have")
            }
            Self::ModeNotAllowed { trip, link, mode } => {
                write!(f, "trip {trip} goes by {mode} on link {link}, which does not allow {mode}")
            }
            Self::NotConsecutive { trip, from, to } => write!(
                f,
                "trip {trip} goes from link {from} to link {to}, which does not start where \
                 {from} ends"
            ),
        }
    }
}

impl core::error::Error for LoadError {}

/// The vehicles' places in `vehicles`, sorted by trip, every vehicle checked.
///
/// # Errors
///
/// The first fault in trip order: a trip given twice first (the lowest such trip), then a
/// vehicle that cannot be moved. The same vehicles in any order give the same error.
pub(crate) fn check(
    network: &Network,
    vehicles: &[Vehicle],
    window: StepGrid,
) -> Result<Vec<u32>, LoadError> {
    let count = u32::try_from(vehicles.len()).expect("at most 2^32 vehicles in one loading");
    let mut order: Vec<u32> = (0..count).collect();
    order.sort_unstable_by_key(|&i| vehicles[i as usize].trip);
    if let Some(w) =
        order.windows(2).find(|w| vehicles[w[0] as usize].trip == vehicles[w[1] as usize].trip)
    {
        return Err(LoadError::DuplicateTrip(vehicles[w[0] as usize].trip));
    }
    for &i in &order {
        check_one(network, &vehicles[i as usize], window)?;
    }
    Ok(order)
}

/// Check one vehicle: its mode, its departure, then its route link by link.
fn check_one(network: &Network, v: &Vehicle, window: StepGrid) -> Result<(), LoadError> {
    let trip = v.trip;
    if v.mode == Mode::Transit {
        return Err(LoadError::Transit(trip));
    }
    if v.route.is_empty() {
        return Err(LoadError::EmptyRoute(trip));
    }
    if !window.contains(v.departure) {
        return Err(LoadError::DepartureOutsideWindow {
            trip,
            departure: v.departure,
            start: window.origin(),
            end: window.end(),
        });
    }
    for (k, &link) in v.route.iter().enumerate() {
        if link.raw() >= network.link_count() {
            return Err(LoadError::UnknownLink { trip, link });
        }
        if !network.modes(link).contains(v.mode) {
            return Err(LoadError::ModeNotAllowed { trip, link, mode: v.mode });
        }
        if k > 0 {
            let from = v.route[k - 1];
            if network.downstream(from) != network.upstream(link) {
                return Err(LoadError::NotConsecutive { trip, from, to: link });
            }
        }
    }
    Ok(())
}
