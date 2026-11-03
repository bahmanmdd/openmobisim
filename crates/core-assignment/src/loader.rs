//! Loaders: the dynamic network loading behind one contract.
//!
//! A [`Loader`] takes the vehicles of an assignment iteration (each with its route, its
//! weight and its departure second) and the network, moves them, and returns what happened:
//! each vehicle's arrival second at the downstream end of its last link (or that it had not
//! arrived when the loading ended), and the link travel times by time of day that the next
//! iteration's searches and the measure stage read.
//!
//! **Level 0, free flow** ([`FreeFlow`]): no capacity and no interaction between vehicles.
//! Each vehicle arrives at its departure second plus the sum of its links' free-flow times
//! (control delay included), exactly, with no rounding of the clock; the link travel times are
//! the free-flow times. A vehicle whose arrival would fall after the loading's end
//! ([`FreeFlow::end_seconds`], unbounded by default) is reported as unfinished.

use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_loading::{LinkTimes, Vehicle};
use openmobisim_core_types::ids::EntityId;

/// What a loading returns.
#[derive(Clone, Debug)]
pub struct Loaded {
    /// Per vehicle, in the order the vehicles were given: its arrival, in seconds after
    /// midnight, at the downstream end of its last link; `None` if it had not arrived when the
    /// loading ended (unfinished).
    pub arrival: Vec<Option<f64>>,
    /// The link travel times by time of day that the loading measured.
    pub link_times: LinkTimes,
}

/// A dynamic network loading. See the [module docs](self).
pub trait Loader {
    /// Move `vehicles` on `network` and say what happened.
    fn load(&self, vehicles: &[Vehicle], network: &RoadNetwork) -> Loaded;
}

/// Level 0: every vehicle at free flow, independently. See the [module docs](self).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FreeFlow {
    /// When the loading ends, in seconds after midnight: a vehicle arriving later is unfinished.
    pub end_seconds: f64,
}

impl Default for FreeFlow {
    /// A loading that never ends: every vehicle arrives.
    fn default() -> Self {
        Self { end_seconds: f64::INFINITY }
    }
}

impl Loader for FreeFlow {
    fn load(&self, vehicles: &[Vehicle], network: &RoadNetwork) -> Loaded {
        let link_times = LinkTimes::free_flow(network);
        let arrival = vehicles
            .iter()
            .map(|v| {
                let links: Vec<u32> = v.route.iter().map(|l| l.raw()).collect();
                let departure = f64::from(v.departure.get());
                let at = departure + link_times.route_seconds(&links, departure);
                (at <= self.end_seconds).then_some(at)
            })
            .collect();
        Loaded { arrival, link_times }
    }
}
