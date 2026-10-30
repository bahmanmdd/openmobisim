//! A run's services inside it: the trips that chose a shared bike or a taxi in a loading,
//! played on the fleet engine after it, in one day (both services, when the run has both, on
//! one clock; they share nothing, so each plays as it would alone).
//!
//! **Coupling** (level 1): the road loading never sees a shared bike. After each loading, the
//! trips whose choice was a shared bike give the day's legs (their chosen pickup and return
//! docks; a request at the departure plus the walk to the pickup dock); the engine plays them
//! with no operator; a trip whose pickup fails walks on and arrives on foot, as the engine's
//! day says. The docks' realised costs (what the legs met per dock and bin,
//! [`DockCosts::with_legs`]; the counts' estimate where no leg went) are averaged into the
//! expectation the next choice reads (the method of successive averages, [`DockCosts::absorb`]),
//! so the expectation's fixed point is what the travellers met.
//!
//! **The mismatch** of a loading ([`ShareDay::mismatch_s`]): per leg that arrived, the absolute
//! difference between the extra seconds it was expected to spend at the docks (its choice's
//! `dock_min`, in seconds) and the extra seconds it spent (its arrival less its arrival had every
//! dock had a bike and a free slot), averaged over the legs weighted by the people they stand
//! for: how far the travellers' expectation was from what they met. Its signed mean
//! ([`ShareDay::bias_s`]: realised less expected) says whether the expectation was too low or
//! too high on the whole; the absolute one does not vanish even when the expectation is right on
//! average, since travellers of one bin meet different outcomes (a share `p` failing at cost `F`
//! and the rest at none, expected `pF`, leaves `2p(1 − p)F` per leg).
//!
//! **Taxis** (S302), likewise at level 1: **their drives are not vehicles of the road
//! loading**; they are timed on the loading's own link times ([`LinkTimes`], what the loading's
//! cars met, by the second they enter each link), so a taxi meets the congestion of the day but
//! adds none. A trip that chose a taxi asks for one at its departure, at its origin's road
//! node; the engine's matching sends the nearest idle vehicle (see [`super::taxis`]); a request
//! no vehicle is sent to within the fleet's longest wait fails, and **the traveller walks from
//! the origin to the destination** on the walk layer, setting off when the wait ends (the
//! shared bikes' rule for a traveller who finds no bike). The fleet's expected wait and failure
//! share per zone and bin ([`TaxiCosts`]) are averaged as the docks' costs are. Its mismatch per
//! leg is the door-to-door seconds met less those expected ([`TaxiLeg::expected_s`]).
//!
//! **Cost:** per loading one engine day (about 120 bytes and a few events per leg); per
//! redirection one walk or ride search on a static layer, rides kept per dock pair.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_routes::{Reach, Search, SearchContext};
use openmobisim_core_types::ids::{EntityId, LinkId, NodeId, TripId};

use super::costs::{DockCosts, TaxiCosts};
use super::engine::{FleetDay, FleetEngine, Leg, NoRoads, ServiceLegs, ServiceRoads};
use super::outcome::{FleetOutcome, LegOutcome, LegStatus, TaxiDay};
use super::roads::DRIVE_BOUND_S;
use super::world::ServiceWorld;
use super::{ServiceKind, ServiceParts, ServiceSetup};
use crate::layers::{LayerSetup, StaticLayers};
use crate::link_times::LinkTimes;

/// One trip's shared-bike leg in a loading, with what its choice expected.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShareLeg {
    /// The trip.
    pub trip: TripId,
    /// Its leg, as the engine plays it.
    pub leg: Leg,
    /// The extra seconds at the docks its choice expected (`dock_min` × 60).
    pub expected_dock_s: f64,
}

impl ShareLeg {
    /// The second it would arrive had its pickup dock a bike and its return dock a free slot.
    #[must_use]
    pub fn planned_arrival_s(&self, setup: &ServiceSetup) -> f64 {
        let (pickup_s, return_s) = match setup.parts() {
            ServiceParts::BikeDocked(_, d) => (d.pickup_s, d.return_s),
            ServiceParts::Taxi(..) => (0.0, 0.0),
        };
        self.leg.request_s + pickup_s + self.leg.ride_s + return_s + self.leg.egress_s
    }
}

/// One loading's engine day.
#[derive(Clone, Debug, PartialEq)]
pub struct ShareDay {
    /// The legs, in trip order.
    pub legs: Vec<ShareLeg>,
    /// What became of them, and the docks' counts.
    pub outcome: FleetOutcome,
    /// The docks' costs the day realised.
    pub realised: DockCosts,
    /// The mean absolute mismatch per leg, in seconds (see the [module docs](self)); `NaN`
    /// with no leg that arrived.
    pub mismatch_s: f64,
    /// The signed mean per leg, realised less expected, in seconds; `NaN` likewise.
    pub bias_s: f64,
    /// The mean extra seconds the legs that arrived met at the docks (their arrival less their
    /// arrival had the planned docks had a bike and a free slot), weighted by people; `NaN`
    /// likewise.
    pub extra_mean_s: f64,
    /// The engine's wall-clock seconds.
    pub engine_s: f64,
}

/// What a run's services did: the numbers `kpis.parquet` carries per service, and the last
/// loading's day `service_legs.parquet` and `hub_bins.parquet` are made of.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ServicesRun {
    /// The service's name (the run's docked service).
    pub service: String,
    /// Its bikes at the start of the day.
    pub fleet: f64,
    /// Per loading, its [`ShareDay::mismatch_s`].
    pub mismatch_s: Vec<f64>,
    /// Per loading, its [`ShareDay::bias_s`].
    pub bias_s: Vec<f64>,
    /// Per loading, its legs (people).
    pub legs_people: Vec<f64>,
    /// The engine's wall-clock seconds over the run.
    pub engine_s: f64,
    /// The last loading's day: `None` if no trip took a shared bike in it.
    pub last: Option<ShareDay>,
    /// The docks' expected costs as the run ended.
    pub expected: Option<DockCosts>,
}

impl ServicesRun {
    /// Nothing done yet by `setup`, the run's service.
    #[must_use]
    pub fn of(setup: &ServiceSetup) -> Self {
        #[allow(clippy::cast_precision_loss, reason = "a count of bikes")]
        let fleet = match setup.parts() {
            ServiceParts::BikeDocked(docks, _) => docks.fleet() as f64,
            ServiceParts::Taxi(fleet, _) => fleet.vehicle_ids.len() as f64,
        };
        Self { service: setup.name().to_string(), fleet, ..Self::default() }
    }
}

/// Rides between docks and walks from a dock to a leg's destination, on the run's layers.
#[derive(Debug)]
struct RunLegs {
    setups: Arc<Vec<ServiceSetup>>,
    layers: Arc<StaticLayers>,
    /// Per leg of the day, its service and its destination's walk node (a taxi leg's: unused).
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

impl ServiceLegs for RunLegs {
    fn ride_s(&self, service: u16, from: u32, to: u32) -> f64 {
        let (ServiceParts::BikeDocked(docks, d), Some(bike)) =
            (self.setups[usize::from(service)].parts(), self.layers.bike.as_ref())
        else {
            return f64::INFINITY;
        };
        let key = (service, from, to);
        if let Some(&s) = self.rides.lock().expect("not poisoned").get(&key) {
            return s;
        }
        let s = shortest(bike, docks.bike_node[from as usize], docks.bike_node[to as usize])
            * d.ride_factor;
        self.rides.lock().expect("not poisoned").insert(key, s);
        s
    }

    fn walk_to_destination_s(&self, leg: usize, dock: u32) -> f64 {
        let (service, to) = self.destinations[leg];
        let (ServiceParts::BikeDocked(docks, _), Some(walk)) =
            (self.setups[usize::from(service)].parts(), self.layers.walk.as_ref())
        else {
            return f64::INFINITY;
        };
        docks.walk_node.get(dock as usize).map_or(f64::INFINITY, |&w| shortest(walk, w, to))
    }
}

/// A loading's roads for its taxis: drives by the time-dependent fastest-route search on the
/// loading's link times (turn rules kept), on the links motor traffic may use; the bounded
/// free-flow searches the matching uses.
struct LinkRoads<'a> {
    road: &'a RoadNetwork,
    times: &'a LinkTimes,
    free: Vec<f64>,
    every: Vec<bool>,
    search: Mutex<Search<'a>>,
}

impl std::fmt::Debug for LinkRoads<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LinkRoads").finish_non_exhaustive()
    }
}

impl<'a> LinkRoads<'a> {
    fn new(road: &'a RoadNetwork, ctx: &'a SearchContext<'a>, times: &'a LinkTimes) -> Self {
        let free = (0..road.link_count())
            .map(|i| {
                let link = LinkId::new(i);
                if road.link_class(link).carries_motor_traffic() {
                    road.free_flow_time(link).get()
                } else {
                    f64::INFINITY
                }
            })
            .collect();
        Self {
            road,
            times,
            free,
            every: vec![true; road.node_count() as usize],
            search: Mutex::new(Search::new(ctx)),
        }
    }

    fn reach(&self, node: NodeId, bound: f64, back: bool) -> Vec<(NodeId, f64)> {
        let mut reach = Reach::new(self.road, &self.free);
        let mut out = if back {
            reach.backward(node, bound, &self.every)
        } else {
            reach.forward(node, bound, &self.every)
        };
        out.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.raw().cmp(&b.0.raw())));
        out
    }
}

impl ServiceRoads for LinkRoads<'_> {
    fn drive(&self, from: NodeId, to: NodeId, t: f64) -> (f64, f64) {
        if from == to {
            return (0.0, 0.0);
        }
        let seconds = |l: u32, at: f64| {
            if self.free[l as usize].is_finite() {
                self.times.link_seconds(l, at)
            } else {
                f64::INFINITY
            }
        };
        self.search
            .lock()
            .expect("a search not poisoned")
            .fastest_route(from, to, t, DRIVE_BOUND_S, &|_, _| 0.0, &seconds)
            .map_or((f64::INFINITY, f64::INFINITY), |(s, links)| {
                (s, links.iter().map(|&l| self.road.link_length(l).get()).sum())
            })
    }
    fn reach_to(&self, to: NodeId, bound: f64) -> Vec<(NodeId, f64)> {
        self.reach(to, bound, true)
    }
    fn reach_from(&self, from: NodeId, bound: f64) -> Vec<(NodeId, f64)> {
        self.reach(from, bound, false)
    }
}

/// What a run's taxis drive on in a loading: the road network, its turns and the loading's
/// link times.
#[derive(Clone, Debug)]
pub struct RunRoads {
    /// The road network.
    pub road: Arc<RoadNetwork>,
    /// Its turn rules.
    pub turns: Arc<TurnTable>,
    /// The loading's link times.
    pub times: Arc<LinkTimes>,
}

/// A loading's city for the engine: the rides and walks of its shared bikes, and its roads for
/// its taxis (none without).
#[derive(Debug)]
struct RunWorld {
    legs: RunLegs,
    roads: Option<RunRoads>,
}

impl ServiceWorld for RunWorld {
    fn legs(&self) -> &dyn ServiceLegs {
        &self.legs
    }

    fn with_roads(&self, f: &mut dyn FnMut(&dyn ServiceRoads)) {
        match &self.roads {
            Some(r) => {
                let ctx = SearchContext::new(&r.road, &r.turns);
                let roads = LinkRoads::new(&r.road, &ctx, &r.times);
                f(&roads);
            }
            None => f(&NoRoads),
        }
    }
}

/// One trip's taxi leg in a loading, with what its choice expected.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TaxiLeg {
    /// The trip.
    pub trip: TripId,
    /// Its leg, as the engine plays it: asked at the departure, from the origin's road node to
    /// the destination's.
    pub leg: Leg,
    /// The wait its choice expected, should a vehicle come, in seconds.
    pub expected_wait_s: f64,
    /// The share of such requests its choice expected to fail.
    pub expected_fail: f64,
    /// The door-to-door seconds its choice expected: the expected wait, boarding and drive
    /// should a vehicle come, and the longest wait then the walk should none, weighed by the
    /// expected failure share.
    pub expected_s: f64,
    /// The walk from its origin to its destination, in seconds (the way on should no taxi
    /// come; infinite with no walk path).
    pub walk_s: f64,
}

/// One loading's taxi day.
#[derive(Clone, Debug, PartialEq)]
pub struct TaxiRunDay {
    /// The legs, in trip order.
    pub legs: Vec<TaxiLeg>,
    /// Where the legs start in the engine's day (its unit requests' `leg` less this is the
    /// index in [`Self::legs`]): after the shared bikes' legs.
    pub offset: usize,
    /// What became of each, by the engine.
    pub outcome: Vec<LegOutcome>,
    /// Per leg, the second the traveller reached the destination: the drop-off, or, for a
    /// request that failed, the end of the longest wait and the walk (`NaN`: not by the
    /// window's end).
    pub arrived_s: Vec<f64>,
    /// The fleet's day: its unit requests, vehicles, bins and log.
    pub day: TaxiDay,
    /// The fleet's costs the day realised.
    pub realised: TaxiCosts,
    /// The mean absolute mismatch per leg that arrived, door-to-door seconds met less those
    /// expected, weighted by people; `NaN` with none.
    pub mismatch_s: f64,
    /// Its signed mean (positive: worse than expected); `NaN` likewise.
    pub bias_s: f64,
    /// The mean wait of the legs carried, in seconds, weighted by people; `NaN` with none.
    pub wait_mean_s: f64,
    /// The share of the legs that asked whose request failed, by people; `NaN` with none.
    pub no_taxi_share: f64,
    /// The engine's wall-clock seconds for the whole day (the shared bikes' included when both
    /// services run).
    pub engine_s: f64,
}

/// What a run's taxi fleet did: per loading its mismatch, and the last loading's day.
#[derive(Clone, Debug, PartialEq)]
pub struct TaxisRun {
    /// The fleet's name.
    pub service: String,
    /// Its vehicles.
    pub fleet: f64,
    /// Per loading, its [`TaxiRunDay::mismatch_s`].
    pub mismatch_s: Vec<f64>,
    /// Per loading, its [`TaxiRunDay::bias_s`].
    pub bias_s: Vec<f64>,
    /// Per loading, its legs (people).
    pub legs_people: Vec<f64>,
    /// Per loading, its [`TaxiRunDay::wait_mean_s`].
    pub wait_mean_s: Vec<f64>,
    /// Per loading, its [`TaxiRunDay::no_taxi_share`].
    pub no_taxi_share: Vec<f64>,
    /// The engine's wall-clock seconds over the run (shared with the docked service's).
    pub engine_s: f64,
    /// The last loading's day: `None` if no trip took a taxi in it.
    pub last: Option<TaxiRunDay>,
    /// The expected costs as the run ended.
    pub expected: Option<TaxiCosts>,
}

impl TaxisRun {
    /// Nothing done yet by `setup`, the run's taxi fleet.
    #[must_use]
    pub fn of(setup: &ServiceSetup) -> Self {
        #[allow(clippy::cast_precision_loss, reason = "a count of vehicles")]
        let fleet = match setup.parts() {
            ServiceParts::Taxi(fleet, _) => fleet.vehicle_ids.len() as f64,
            ServiceParts::BikeDocked(..) => 0.0,
        };
        Self {
            service: setup.name().to_string(),
            fleet,
            mismatch_s: Vec::new(),
            bias_s: Vec::new(),
            legs_people: Vec::new(),
            wait_mean_s: Vec::new(),
            no_taxi_share: Vec::new(),
            engine_s: 0.0,
            last: None,
            expected: None,
        }
    }
}

/// The index of `setups`' service of `kind`, if it has one.
#[must_use]
pub fn index_of(setups: &[ServiceSetup], kind: ServiceKind) -> Option<usize> {
    setups.iter().position(|s| s.kind() == kind)
}

/// What one loading gives the engine: its shared-bike legs and its taxi legs (each in trip
/// order), the roads its taxis drive on and the taxi fleet's expected costs the travellers chose
/// on (kept where none of its requests went).
#[derive(Debug, Default)]
pub struct DayLegs<'a> {
    /// The shared-bike legs.
    pub share: Vec<ShareLeg>,
    /// The taxi legs.
    pub taxi: Vec<TaxiLeg>,
    /// The roads (none without taxi legs).
    pub roads: Option<RunRoads>,
    /// The taxi fleet's expected costs, as the loading's choice read them.
    pub taxi_estimate: Option<&'a TaxiCosts>,
}

/// Play one loading's `legs` of the run's services `setups` on the run's `layers`, over `window`
/// (seconds from midnight), with no operator: the shared bikes' day and the taxis' day, each
/// `None` without legs of its kind.
///
/// # Panics
///
/// If `setups` has no docked service while `legs` has shared-bike legs, or no taxi fleet (or
/// no estimate of its costs) while it has taxi legs.
#[must_use]
pub fn play(
    setups: &Arc<Vec<ServiceSetup>>,
    layers: &Arc<StaticLayers>,
    legs: DayLegs<'_>,
    window: (f64, f64),
    seed: u64,
) -> (Option<ShareDay>, Option<TaxiRunDay>) {
    let started = Instant::now();
    let DayLegs { share: share_legs, taxi: taxi_legs, roads, taxi_estimate } = legs;
    let n_share = share_legs.len();
    let mut destinations: Vec<(u16, NodeId)> =
        share_legs.iter().map(|l| (l.leg.service, l.leg.destination)).collect();
    destinations.extend(taxi_legs.iter().map(|l| (l.leg.service, l.leg.destination)));
    let world = RunWorld {
        legs: RunLegs {
            setups: Arc::clone(setups),
            layers: Arc::clone(layers),
            destinations,
            rides: Mutex::new(HashMap::new()),
        },
        roads: roads.filter(|_| !taxi_legs.is_empty()),
    };
    // The shared bikes' legs first, then the taxis': each kind's outcomes are a slice.
    let mut all: Vec<Leg> = share_legs.iter().map(|l| l.leg).collect();
    all.extend(taxi_legs.iter().map(|l| l.leg));
    let day = FleetDay { window, legs: all, seed };
    let mut outcome =
        FleetEngine::in_world(Arc::clone(setups), Arc::new(day), Arc::new(world), None)
            .run_day()
            .expect("no policy, no policy failure");
    let engine_s = started.elapsed().as_secs_f64();
    let taxi_out = outcome.legs.split_off(n_share);
    let taxi_index = index_of(setups, ServiceKind::Taxi);
    let taxi_day = taxi_index.and_then(|t| outcome.taxis[t].take());
    let share = (!share_legs.is_empty()).then(|| {
        let s = index_of(setups, ServiceKind::BikeDocked).expect("shared-bike legs need docks");
        share_day(&setups[s], s, share_legs, outcome, engine_s)
    });
    let taxi = (!taxi_legs.is_empty()).then(|| {
        let estimate = taxi_estimate.expect("taxi legs need the fleet's estimate");
        let t = taxi_index.expect("taxi legs need a taxi fleet");
        let day = taxi_day.expect("a taxi fleet's day");
        taxi_run_day(&setups[t], n_share, taxi_legs, taxi_out, day, estimate, window, engine_s)
    });
    (share, taxi)
}

/// The shared bikes' part of a day: what their legs met, the docks' realised costs, the
/// mismatch.
fn share_day(
    setup: &ServiceSetup,
    s: usize,
    legs: Vec<ShareLeg>,
    outcome: FleetOutcome,
    engine_s: f64,
) -> ShareDay {
    let bins = outcome.docks[s].as_ref().expect("the run's docked service has docks");
    let (pickup_s, return_s) = match setup.parts() {
        ServiceParts::BikeDocked(_, d) => (d.pickup_s, d.return_s),
        ServiceParts::Taxi(..) => (0.0, 0.0),
    };
    let (mut met_pickup, mut met_return) = (Vec::new(), Vec::new());
    for (l, o) in legs.iter().zip(&outcome.legs) {
        if !(matches!(o.status, LegStatus::Served | LegStatus::Failed) && o.arrived_s.is_finite()) {
            continue;
        }
        let w = f64::from(l.leg.weight);
        let total = o.arrived_s - l.planned_arrival_s(setup);
        // The pickup's part: the seconds until a bike was in hand past the planned ones; a
        // pickup that failed met the whole of it.
        let at_pickup = if o.picked.is_some() && o.picked_s.is_finite() {
            (o.picked_s - l.leg.request_s - pickup_s).max(0.0)
        } else {
            total
        };
        met_pickup.push((l.leg.pickup_dock, l.leg.request_s, at_pickup, w));
        if o.picked.is_some() {
            let planned_return = l.leg.request_s + pickup_s + l.leg.ride_s + return_s;
            met_return.push((l.leg.return_dock, planned_return, (total - at_pickup).max(0.0), w));
        }
    }
    let realised = DockCosts::from_bins(setup, bins).with_legs(&met_pickup, &met_return);
    let (mut sum, mut signed, mut people, mut met) = (0.0, 0.0, 0.0, 0.0);
    for (l, o) in legs.iter().zip(&outcome.legs) {
        if matches!(o.status, LegStatus::Served | LegStatus::Failed) && o.arrived_s.is_finite() {
            let w = f64::from(l.leg.weight);
            let extra = o.arrived_s - l.planned_arrival_s(setup);
            sum += w * (extra - l.expected_dock_s).abs();
            signed += w * (extra - l.expected_dock_s);
            met += w * extra;
            people += w;
        }
    }
    let (mismatch_s, bias_s, extra_mean_s) = if people > 0.0 {
        (sum / people, signed / people, met / people)
    } else {
        (f64::NAN, f64::NAN, f64::NAN)
    };
    ShareDay { legs, outcome, realised, mismatch_s, bias_s, extra_mean_s, engine_s }
}

/// The taxis' part of a day: when each traveller arrived (a failed request walks on), the
/// fleet's realised costs, the mismatch.
#[allow(clippy::too_many_arguments, reason = "the parts of a day, named")]
fn taxi_run_day(
    setup: &ServiceSetup,
    offset: usize,
    legs: Vec<TaxiLeg>,
    outcome: Vec<LegOutcome>,
    day: TaxiDay,
    estimate: &TaxiCosts,
    window: (f64, f64),
    engine_s: f64,
) -> TaxiRunDay {
    let max_wait_s = match setup.parts() {
        ServiceParts::Taxi(_, d) => d.max_wait_s,
        ServiceParts::BikeDocked(..) => 0.0,
    };
    let arrived_s: Vec<f64> = legs
        .iter()
        .zip(&outcome)
        .map(|(l, o)| match o.status {
            LegStatus::Served => o.arrived_s,
            LegStatus::Failed => {
                let at = o.requested_s + max_wait_s + l.walk_s;
                if at.is_finite() && at <= window.1 { at } else { f64::NAN }
            }
            _ => f64::NAN,
        })
        .collect();
    // What the requests met, by the zone of their pickup and the bin they asked in: a unit
    // request that failed, or one served (its wait); those still waiting at the window's end
    // tell nothing yet.
    let mut met = Vec::new();
    for r in &day.requests {
        let Some(l) = (r.leg as usize).checked_sub(offset).and_then(|i| legs.get(i)) else {
            continue;
        };
        if r.failed {
            met.push((l.leg.origin, r.requested_s, None, 1.0));
        } else if r.picked_s.is_finite() {
            met.push((l.leg.origin, r.requested_s, Some(r.picked_s - r.requested_s), 1.0));
        }
    }
    let realised = estimate.clone().with_requests(&met);
    let (mut sum, mut signed, mut people) = (0.0, 0.0, 0.0);
    for (l, &at) in legs.iter().zip(&arrived_s) {
        if at.is_finite() {
            let w = f64::from(l.leg.weight);
            let d = (at - l.leg.request_s) - l.expected_s;
            sum += w * d.abs();
            signed += w * d;
            people += w;
        }
    }
    let (mismatch_s, bias_s) =
        if people > 0.0 { (sum / people, signed / people) } else { (f64::NAN, f64::NAN) };
    let (mut asked, mut failed, mut carried, mut waited) = (0.0, 0.0, 0.0, 0.0);
    for (l, o) in legs.iter().zip(&outcome) {
        let w = f64::from(l.leg.weight);
        match o.status {
            LegStatus::NotRequested => continue,
            LegStatus::Failed => failed += w,
            LegStatus::Served => {
                carried += w;
                waited += w * o.waited_s;
            }
            _ => {}
        }
        asked += w;
    }
    let wait_mean_s = if carried > 0.0 { waited / carried } else { f64::NAN };
    let no_taxi_share = if asked > 0.0 { failed / asked } else { f64::NAN };
    TaxiRunDay {
        legs,
        offset,
        outcome,
        arrived_s,
        day,
        realised,
        mismatch_s,
        bias_s,
        wait_mean_s,
        no_taxi_share,
        engine_s,
    }
}

/// The day of the run's docked services `setups` over `window` (seconds from midnight) with no
/// leg: the docks keep their start-of-day bikes (what `hub_bins.parquet` holds for a run whose
/// last loading had no shared-bike trip).
///
/// # Panics
///
/// Never in practice: a day without a policy cannot fail.
#[must_use]
pub fn idle(setups: &Arc<Vec<ServiceSetup>>, window: (f64, f64)) -> FleetOutcome {
    let day = FleetDay { window, legs: Vec::new(), seed: 0 };
    FleetEngine::new(
        Arc::clone(setups),
        Arc::new(day),
        Arc::new(super::engine::NoLegs),
        Arc::new(NoRoads),
        None,
    )
    .run_day()
    .expect("no policy, no policy failure")
}
