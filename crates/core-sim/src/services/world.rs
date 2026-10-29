//! A fleet day's city, owned: the network, its turns, the walk and bike layers and the road
//! times, shared by `Arc`, so an engine that uses it borrows nothing and can be kept, copied
//! and stepped between calls (a Python environment's).
//!
//! **Why owned, and how the searches stay cheap.** A road search borrows its context, which
//! borrows the network; a search made for every drive would cost one pass over the links each
//! time. [`ServiceWorld::with_roads`] makes the context and the search **once per call** and
//! lends them for the call: an engine's [`super::engine::FleetEngine::advance_to`] takes many
//! events, so the pass is paid once per step of a policy, not per drive.
//!
//! **Cost:** per call one pass over the links (the search's context and scratch: about 40
//! bytes per link); per ride or walk one static search, rides kept per dock pair; the day's
//! legs kept (about 60 bytes a leg) and the zones (4 bytes a node).

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_routes::{Reach, SearchContext};
use openmobisim_core_types::ids::{EntityId, NodeId};

use super::engine::{Leg, ServiceLegs, ServiceRoads};
use super::queries::{ZONE_GRID_M, Zones};
use super::roads::{FrozenRoads, RoadTimes};
use super::{ServiceParts, ServiceSetup};
use crate::layers::LayerSetup;

/// What an engine needs of its city, owned (see the [module docs](self)).
pub trait ServiceWorld: Send + Sync + std::fmt::Debug {
    /// The rides and walks a day's redirections ask for.
    fn legs(&self) -> &dyn ServiceLegs;
    /// Lend `f` the roads for one call: drives on the road times, free-flow reaches.
    fn with_roads(&self, f: &mut dyn FnMut(&dyn ServiceRoads));
    /// The day's legs before any variation a seed draws (a policy's expectation,
    /// [`super::queries::Queries::expected_legs`]); `None`: the engine's own day's legs.
    fn base_legs(&self) -> Option<&[Leg]> {
        None
    }
    /// The zones over the road network's nodes ([`super::queries::Zones`]); `None`: none.
    fn zones(&self) -> Option<&Zones> {
        None
    }
}

/// A city for a frozen day: its network and turns, its walk and bike layers, the road times,
/// the services, the day's legs (their destinations, and the legs a policy may expect: before
/// any variation a seed draws) and the zones over the network's nodes, every one owned.
#[derive(Debug)]
pub struct FrozenWorld {
    road: Arc<RoadNetwork>,
    turns: Arc<TurnTable>,
    times: RoadTimes,
    legs: OwnedLegs,
    base: Vec<Leg>,
    zones: Option<Zones>,
}

impl FrozenWorld {
    /// The city of `road` (its turns), the layers `walk` and `bike`, `times`, for `setups` and
    /// the day's `legs` (before any variation a seed draws); zones of [`ZONE_GRID_M`] metres.
    ///
    /// # Panics
    ///
    /// Never: [`ZONE_GRID_M`] is a positive number of metres.
    #[must_use]
    pub fn new(
        road: Arc<RoadNetwork>,
        turns: Arc<TurnTable>,
        walk: Arc<LayerSetup>,
        bike: Arc<LayerSetup>,
        times: RoadTimes,
        setups: Arc<Vec<ServiceSetup>>,
        legs: &[Leg],
    ) -> Self {
        let zones = Zones::grid(&road, ZONE_GRID_M).expect("a positive grid");
        Self {
            road,
            turns,
            times,
            base: legs.to_vec(),
            zones,
            legs: OwnedLegs {
                setups,
                walk,
                bike,
                destinations: legs.iter().map(|l| (l.service, l.destination)).collect(),
                rides: Mutex::new(HashMap::new()),
            },
        }
    }

    /// The road times it drives on.
    #[must_use]
    pub fn times(&self) -> &RoadTimes {
        &self.times
    }

    /// The same city with zones of `grid_m` metres (the fleet day's option `zone_grid_m`,
    /// [`ZONE_GRID_M`] unless given).
    ///
    /// # Errors
    ///
    /// A message for a side that is not a positive number of metres.
    pub fn with_zone_grid_m(mut self, grid_m: f64) -> Result<Self, String> {
        self.zones = Zones::grid(&self.road, grid_m)?;
        Ok(self)
    }
}

impl ServiceWorld for FrozenWorld {
    fn legs(&self) -> &dyn ServiceLegs {
        &self.legs
    }

    fn with_roads(&self, f: &mut dyn FnMut(&dyn ServiceRoads)) {
        let ctx = SearchContext::new(&self.road, &self.turns);
        let roads = FrozenRoads::with_times(&self.road, &ctx, Cow::Borrowed(&self.times));
        f(&roads);
    }

    fn base_legs(&self) -> Option<&[Leg]> {
        Some(&self.base)
    }

    fn zones(&self) -> Option<&Zones> {
        self.zones.as_ref()
    }
}

/// The rides and walks of a frozen day, on layers it owns.
#[derive(Debug)]
struct OwnedLegs {
    setups: Arc<Vec<ServiceSetup>>,
    walk: Arc<LayerSetup>,
    bike: Arc<LayerSetup>,
    /// Per leg of the day, its service and its destination's walk node.
    destinations: Vec<(u16, NodeId)>,
    rides: Mutex<HashMap<(u16, u32, u32), f64>>,
}

/// The shortest time from `from` to `to` on `layer` (infinite with no path).
fn shortest(layer: &LayerSetup, from: NodeId, to: NodeId) -> f64 {
    if from == to {
        return 0.0;
    }
    let graph = layer.network().network();
    let mut wanted = vec![false; graph.node_count() as usize];
    wanted[to.index()] = true;
    Reach::new(graph, layer.seconds())
        .forward(from, f64::INFINITY, &wanted)
        .first()
        .map_or(f64::INFINITY, |&(_, s)| s)
}

impl ServiceLegs for OwnedLegs {
    fn ride_s(&self, service: u16, from: u32, to: u32) -> f64 {
        let ServiceParts::BikeDocked(docks, d) = self.setups[usize::from(service)].parts() else {
            return f64::INFINITY;
        };
        let key = (service, from, to);
        if let Some(&s) = self.rides.lock().expect("not poisoned").get(&key) {
            return s;
        }
        let s = shortest(&self.bike, docks.bike_node[from as usize], docks.bike_node[to as usize])
            * d.ride_factor;
        self.rides.lock().expect("not poisoned").insert(key, s);
        s
    }

    fn walk_to_destination_s(&self, leg: usize, dock: u32) -> f64 {
        let (service, to) = self.destinations[leg];
        match self.setups[usize::from(service)].parts() {
            ServiceParts::BikeDocked(docks, _) => docks
                .walk_node
                .get(dock as usize)
                .map_or(f64::INFINITY, |&w| shortest(&self.walk, w, to)),
            ServiceParts::Taxi(..) => f64::INFINITY,
        }
    }
}
