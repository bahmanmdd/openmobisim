//! What a policy sees at a step: the clock, the step, a seed for its own draws, and per
//! service its state, read-only.
//!
//! The arrays the state already holds are borrowed, not copied: a docked service's bikes per
//! dock, a taxi fleet's vehicles' states, nodes and charges. The counts since the last step
//! (pickups, returns, failed pickups, redirections, per dock) are kept by the engine and
//! cleared after each step. A taxi fleet's waiting requests and chargers' plugs in use and
//! queues are gathered at the step (one pass over the waiting requests and the chargers).

use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_types::ids::NodeId;

use super::engine::VehicleState;

/// What a policy sees at a step: see the [module docs](self).
#[derive(Clone, Debug)]
pub struct View<'a> {
    /// The step's second, from midnight.
    pub t: f64,
    /// The step's index: 0 at the window's start.
    pub step: u32,
    /// A number for the policy's own draws at this step: from the stream `fleet_policy`, keyed
    /// by the day's seed and [`Self::step`]; the same day and step give the same number.
    pub rng_seed: u64,
    /// The window's start and end, in seconds from midnight.
    pub window: (f64, f64),
    /// Per service, in the run's order.
    pub services: Vec<ServiceView<'a>>,
}

impl<'a> View<'a> {
    /// Service `s`'s section, if it is a docked service.
    #[must_use]
    pub fn docked(&self, s: u16) -> Option<&DockView<'a>> {
        match self.services.get(usize::from(s)) {
            Some(ServiceView::BikeDocked(v)) => Some(v),
            _ => None,
        }
    }

    /// Service `s`'s section, if it is a taxi fleet.
    #[must_use]
    pub fn taxi(&self, s: u16) -> Option<&TaxiView<'a>> {
        match self.services.get(usize::from(s)) {
            Some(ServiceView::Taxi(v)) => Some(v),
            _ => None,
        }
    }
}

/// One service's section of a [`View`].
#[derive(Clone, Debug)]
pub enum ServiceView<'a> {
    /// A docked service.
    BikeDocked(DockView<'a>),
    /// A taxi fleet.
    Taxi(TaxiView<'a>),
}

/// A docked service at a step. Per-dock arrays are indexed by the service's dock index.
#[derive(Clone, Debug)]
pub struct DockView<'a> {
    /// The service's name.
    pub name: &'a str,
    /// Per dock, the bikes docked now.
    pub bikes: &'a [u32],
    /// Per dock, its slots.
    pub slots: &'a [u32],
    /// Per dock, where it is.
    pub position: &'a [LonLat],
    /// Per dock, its road node (empty: set up without roads, no van can move bikes).
    pub road_node: &'a [NodeId],
    /// Per dock, bikes taken since the last step (people).
    pub pickups: &'a [f64],
    /// Per dock, bikes returned since the last step (people).
    pub returns: &'a [f64],
    /// Per dock, pickups that failed there (the last dock tried) since the last step (people).
    pub failed: &'a [f64],
    /// Per dock, travellers sent on to it from an empty or full dock since the last step
    /// (people; counted when they were sent).
    pub redirected_in: &'a [f64],
    /// Per dock, travellers who found it empty or full and went on since the last step (people).
    pub redirected_out: &'a [f64],
    /// Bikes in operators' vans now.
    pub moving: u64,
    /// Bikes being ridden now.
    pub riding: u64,
}

/// A request waiting for a taxi.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WaitingRequest {
    /// The unit request's index in the fleet's day ([`super::outcome::TaxiDay::requests`]).
    pub id: u32,
    /// The leg it belongs to, by its index in the day.
    pub leg: u32,
    /// Its pickup's road node.
    pub origin: NodeId,
    /// The second it asked.
    pub since_s: f64,
}

/// A taxi fleet at a step. Per-vehicle arrays are indexed by the fleet's vehicle index,
/// per-charger arrays by its charger index.
#[derive(Clone, Debug)]
pub struct TaxiView<'a> {
    /// The service's name.
    pub name: &'a str,
    /// Per vehicle, its state.
    pub state: &'a [VehicleState],
    /// Per vehicle, its road node: where it is when idle, charging or queued; where it is going
    /// when driving.
    pub node: &'a [NodeId],
    /// Per vehicle, its charge as a share of the battery.
    pub soc: &'a [f64],
    /// The requests waiting for a vehicle, oldest first.
    pub waiting: Vec<WaitingRequest>,
    /// Per charger, its road node.
    pub charger_node: &'a [NodeId],
    /// Per charger, its plugs.
    pub charger_plugs: &'a [u32],
    /// Per charger, the plugs in use now.
    pub charger_in_use: Vec<u32>,
    /// Per charger, the vehicles queued for a plug now.
    pub charger_queued: Vec<u32>,
}
