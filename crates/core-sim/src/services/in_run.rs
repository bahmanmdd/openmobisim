//! A run's services after it (S303): the trips of the run's **last loading** that chose a shared
//! bike or a taxi, played on the fleet engine **once, after the run has stopped**, in one day
//! (both services, when the run has both, on one clock; they share nothing, so each plays as it
//! would alone). The loadings never play the services, and what this day gives is never fed
//! back: the travellers chose on fixed assumptions ([`super::ServiceAssumptions`]), and their
//! trips keep the arrivals they planned.
//!
//! **Shared bikes.** The road loading never sees one. Each leg is the trip's chosen pickup and
//! return docks, a request when the traveller reaches the pickup dock, the ride and the walk
//! on; the engine plays them with no operator: an empty dock sends the traveller on to another,
//! a full one likewise, and a traveller who finds no bike at any dock tried walks on (the leg is
//! cancelled).
//!
//! **Taxis.** Their drives are not vehicles of the road loading: they are timed on the last
//! loading's link times ([`RunRoads::times`], what its cars drove at, by the second they enter each
//! link, as [`RoadTimes`]: the same road times a frozen day of the run holds, so a replay of the
//! frozen day drives the same drives), so a taxi meets the day's congestion but adds none. A trip
//! that chose a taxi requests one at its departure, at its origin's road node; the engine's
//! matching sends the nearest idle taxi (see [`super::taxis`]); **a request no taxi can pick up
//! within the fleet's cancel limit is cancelled, and the trip ends there**, with no walk on.
//!
//! **What the day actually gave.** [`ServiceOutcome`] says what became of each trip
//! (completed, cancelled, unfinished); [`TaxiSummary`] and [`DockSummary`] sum the day up per
//! service in a dispatcher's words (requests, rides, cancellations, waiting times, kilometres,
//! utilisation; for docks the extra walks), each over people (the travellers' weights).
//!
//! **An operator** ([`super::policy::Policy`], [`crate::run::Run::with_service_policy`]) may
//! steer the day as it steers a frozen day: the day is then played on the run's roads, with the
//! zones a frozen day has (the default grid), so a frozen day of the run replayed with the same
//! policy takes the same decisions. Without one, nothing is built for it.
//!
//! **Cost:** one engine day per run (about 120 bytes and a few events per leg); per
//! redirection one walk or ride search on a static layer, rides kept per dock pair.

//! redirection one walk or ride search on a static layer, rides kept per dock pair.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_routes::{Reach, SearchContext};
use openmobisim_core_types::ids::{EntityId, NodeId, TripId};

use super::costs::{DockCosts, TaxiAssumption};
use super::decisions::DecisionRecord;
use super::engine::{FleetDay, FleetEngine, Leg, NoRoads, ServiceLegs};
use super::outcome::{FleetOutcome, LegOutcome, LegStatus, TaxiDay};
use super::policy::Policy;
use super::queries::{ZONE_GRID_M, Zones};
use super::roads::{FrozenRoads, RoadTimes};
use super::world::ServiceWorld;
use super::{ServiceKind, ServiceParts, ServiceSetup};
use crate::layers::{LayerSetup, StaticLayers};

/// What the service day actually gave a trip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceOutcome {
    /// Completed: carried to the end of its service leg (a shared bike docked, a taxi's
    /// drop-off).
    Completed,
    /// Cancelled: a taxi request no taxi could pick up within the fleet's cancel limit (the
    /// trip ends there), or no bike at any dock tried (the traveller walked on).
    Cancelled,
    /// Unfinished: still waiting or riding when the simulation window ended, or asked after it.
    Unfinished,
}

impl ServiceOutcome {
    /// The stable snake_case name, as `trip_results` writes it (`service_outcome`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            ServiceOutcome::Completed => "completed",
            ServiceOutcome::Cancelled => "cancelled",
            ServiceOutcome::Unfinished => "unfinished",
        }
    }

    /// What a leg's engine outcome `o` says, and the second the traveller reached the end of
    /// the leg: by the service, or on foot after a shared-bike leg found no bike (`NaN`: not by
    /// the window's end `end_s`, and always for a cancelled taxi request, which ends the trip).
    #[must_use]
    pub fn of(o: &LegOutcome, end_s: f64) -> (Self, f64) {
        let at =
            if o.arrived_s.is_finite() && o.arrived_s <= end_s { o.arrived_s } else { f64::NAN };
        match o.status {
            LegStatus::Completed => (ServiceOutcome::Completed, at),
            LegStatus::Cancelled => (ServiceOutcome::Cancelled, at),
            _ => (ServiceOutcome::Unfinished, f64::NAN),
        }
    }
}

/// The mean of `x` over `n` people, `NaN` with nobody.
fn mean(x: f64, n: f64) -> f64 {
    if n > 0.0 { x / n } else { f64::NAN }
}

/// The people-weighted `q` quantile of `values` (each `(value, people)`): the smallest value
/// such that the people at or below it are at least the share `q` of all; `NaN` with nobody.
fn weighted_quantile(values: &mut [(f64, f64)], q: f64) -> f64 {
    values.sort_by(|a, b| a.0.total_cmp(&b.0));
    let all: f64 = values.iter().map(|v| v.1).sum();
    if all <= 0.0 {
        return f64::NAN;
    }
    let mut seen = 0.0;
    for &(v, w) in values.iter() {
        seen += w;
        if seen >= q * all - 1e-9 * all {
            return v;
        }
    }
    values.last().map_or(f64::NAN, |v| v.0)
}

/// A taxi fleet's service day summed up in a dispatcher's words, over people (the travellers'
/// weights; one request per person, no pooling). A mean over nobody is `NaN`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TaxiSummary {
    /// The people of the trips that chose a taxi in the last loading: one request each.
    pub requests: f64,
    /// Of them, picked up and dropped off at the destination.
    pub rides: f64,
    /// Cancelled: no taxi could pick them up within the cancel limit.
    pub cancelled: f64,
    /// Unfinished: still waiting or riding when the simulation window ended, or asking after it.
    pub unfinished: f64,
    /// The mean waiting time of the rides, from the request to the pickup (the dispatch delay
    /// included), in seconds.
    pub waiting_mean_s: f64,
    /// The median of the same waiting times.
    pub waiting_median_s: f64,
    /// Their 90th percentile.
    pub waiting_p90_s: f64,
    /// The mean time in the vehicle of the rides, from the pickup to the drop-off (boarding
    /// included), in seconds.
    pub in_vehicle_mean_s: f64,
    /// The mean kilometres of the rides with the passenger on board.
    pub ride_km_mean: f64,
    /// The fleet's kilometres driven without a passenger (to pickups, to chargers, relocating)
    /// per ride.
    pub empty_km_per_ride: f64,
    /// The share of the fleet's time in the window with a passenger on board (boarding
    /// included), from 0 to 1.
    pub utilisation: f64,
    /// Rides per taxi of the fleet.
    pub rides_per_taxi: f64,
    /// The energy the fleet charged, in kWh.
    pub kwh_charged: f64,
}

impl TaxiSummary {
    /// The summary of a fleet of `fleet` taxis that no trip chose: no request, the fleet idle.
    #[must_use]
    pub fn idle() -> Self {
        Self {
            requests: 0.0,
            rides: 0.0,
            cancelled: 0.0,
            unfinished: 0.0,
            waiting_mean_s: f64::NAN,
            waiting_median_s: f64::NAN,
            waiting_p90_s: f64::NAN,
            in_vehicle_mean_s: f64::NAN,
            ride_km_mean: f64::NAN,
            empty_km_per_ride: f64::NAN,
            utilisation: 0.0,
            rides_per_taxi: 0.0,
            kwh_charged: 0.0,
        }
    }

    /// The rows `kpis.parquet` writes, by metric name, in order; `assumed_wait_s` the waiting
    /// time the travellers assumed.
    #[must_use]
    pub fn rows(&self, assumed_wait_s: f64) -> Vec<(&'static str, f64)> {
        vec![
            ("requests", self.requests),
            ("rides", self.rides),
            ("cancelled", self.cancelled),
            ("unfinished", self.unfinished),
            ("waiting_mean_s", self.waiting_mean_s),
            ("waiting_median_s", self.waiting_median_s),
            ("waiting_p90_s", self.waiting_p90_s),
            ("assumed_wait_s", assumed_wait_s),
            ("in_vehicle_mean_s", self.in_vehicle_mean_s),
            ("ride_km_mean", self.ride_km_mean),
            ("empty_km_per_ride", self.empty_km_per_ride),
            ("utilisation", self.utilisation),
            ("rides_per_taxi", self.rides_per_taxi),
            ("kwh_charged", self.kwh_charged),
        ]
    }
}

/// A docked bike service's service day summed up, over people (the travellers' weights). A
/// mean over nobody is `NaN`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DockSummary {
    /// The people of the trips that planned a shared bike in the last loading.
    pub requests: f64,
    /// Of them, got a bike and returned it.
    pub rides: f64,
    /// Found no bike at any dock tried (the traveller walked on).
    pub no_bike: f64,
    /// Still looking for a bike or riding when the simulation window ended, or asking after it.
    pub unfinished: f64,
    /// The mean extra walk at the pickup over the rides, in seconds: a ride's arrival less its
    /// arrival had the planned docks had a bike and a free slot, less its return's part (the
    /// walks between docks looking for a bike, the ride's change from another pickup dock).
    pub extra_walk_at_pickup_mean_s: f64,
    /// The mean extra at the return over the rides, in seconds ([`LegOutcome::extra_walk_at_return_s`]:
    /// the rides between return docks after a full one, the walk from the dock used less the
    /// planned walk).
    pub extra_walk_at_return_mean_s: f64,
    /// The share of the rides sent on to another dock at either end, from 0 to 1.
    pub redirected_share: f64,
}

impl DockSummary {
    /// The summary of a docked service that no trip chose.
    #[must_use]
    pub fn idle() -> Self {
        Self {
            requests: 0.0,
            rides: 0.0,
            no_bike: 0.0,
            unfinished: 0.0,
            extra_walk_at_pickup_mean_s: f64::NAN,
            extra_walk_at_return_mean_s: f64::NAN,
            redirected_share: f64::NAN,
        }
    }

    /// The rows `kpis.parquet` writes, by metric name, in order; `assumed` the extra walks the
    /// travellers assumed at the pickup and at the return (`NaN`: from the docks' start-of-day
    /// bikes).
    #[must_use]
    pub fn rows(&self, assumed: (f64, f64)) -> Vec<(&'static str, f64)> {
        vec![
            ("requests", self.requests),
            ("rides", self.rides),
            ("no_bike", self.no_bike),
            ("unfinished", self.unfinished),
            ("extra_walk_at_pickup_mean_s", self.extra_walk_at_pickup_mean_s),
            ("extra_walk_at_return_mean_s", self.extra_walk_at_return_mean_s),
            ("assumed_extra_walk_at_pickup_s", assumed.0),
            ("assumed_extra_walk_at_return_s", assumed.1),
            ("redirected_share", self.redirected_share),
        ]
    }
}

/// One trip's shared-bike leg in the last loading, with what its choice assumed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShareLeg {
    /// The trip.
    pub trip: TripId,
    /// Its leg, as the engine plays it.
    pub leg: Leg,
    /// The extra walk at the docks its choice assumed, in seconds (`dock_min` × 60).
    pub assumed_extra_walk_s: f64,
    /// The trip's departure, in seconds from midnight.
    pub departure_s: f64,
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

    /// The second its choice planned the leg to end: [`Self::planned_arrival_s`] and the
    /// assumed extra walk at the docks (the trip's planned arrival; for a shared bike to
    /// transit, its planned arrival at the hub's dock).
    #[must_use]
    pub fn planned_end_s(&self, setup: &ServiceSetup) -> f64 {
        self.planned_arrival_s(setup) + self.assumed_extra_walk_s
    }

    /// The extra seconds the service day gave this leg at the pickup and at the return, if its
    /// leg ended (completed, or cancelled and walked on): its arrival less
    /// [`Self::planned_arrival_s`], split into the return's part
    /// ([`LegOutcome::extra_walk_at_return_s`]) and the rest, the pickup's (the walks between docks
    /// looking for a bike, the ride's change from another pickup dock, or the walk on of a
    /// traveller who found no bike).
    #[must_use]
    pub fn dock_extra_s(&self, setup: &ServiceSetup, o: &LegOutcome) -> Option<(f64, f64)> {
        let ended = matches!(o.status, LegStatus::Completed | LegStatus::Cancelled);
        (ended && o.arrived_s.is_finite()).then(|| {
            let extra = o.arrived_s - self.planned_arrival_s(setup);
            (extra - o.extra_walk_at_return_s, o.extra_walk_at_return_s)
        })
    }
}

/// The shared bikes' part of the service day.
#[derive(Clone, Debug, PartialEq)]
pub struct ShareDay {
    /// The legs, in trip order.
    pub legs: Vec<ShareLeg>,
    /// What became of them, and the docks' counts.
    pub outcome: FleetOutcome,
    /// Per leg, what the day gave it and the second its leg ended, on a bike or on foot after
    /// finding none (`NaN`: not by the window's end).
    pub outcomes: Vec<(ServiceOutcome, f64)>,
    /// The day summed up.
    pub summary: DockSummary,
    /// The engine's wall-clock seconds.
    pub engine_s: f64,
}

/// What a run's docked service did: its service day, what `kpis.parquet`,
/// `bike_share_*.parquet` and the trip rows are made of.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ServicesRun {
    /// The service's name (the run's docked service).
    pub service: String,
    /// Its bikes at the start of the day.
    pub fleet: f64,
    /// The engine's wall-clock seconds (the whole service day, the taxis' included when both
    /// services run).
    pub engine_s: f64,
    /// The service day: `None` if no trip took a shared bike in the last loading.
    pub last: Option<ShareDay>,
    /// The docks' costs the travellers assumed, the whole run.
    pub assumed: Option<DockCosts>,
    /// The extra walks the travellers assumed at the pickup and at the return, in seconds
    /// (`NaN`: from the docks' start-of-day bikes).
    pub assumed_extra_walk_s: (f64, f64),
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
        Self {
            service: setup.name().to_string(),
            fleet,
            assumed_extra_walk_s: (f64::NAN, f64::NAN),
            ..Self::default()
        }
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

/// What a run's taxis drive on in its service day: the road network, its turns and the last
/// loading's link times as road times.
#[derive(Clone, Debug)]
pub struct RunRoads {
    /// The road network.
    pub road: Arc<RoadNetwork>,
    /// Its turn rules.
    pub turns: Arc<TurnTable>,
    /// The last loading's link times, by the bin a vehicle enters a link.
    pub times: Arc<RoadTimes>,
}

/// The service day's city for the engine: the rides and walks of its shared bikes, and its
/// roads for its taxis and an operator's vans (none without either), with the zones an
/// operator's view groups places by (none without an operator).
#[derive(Debug)]
struct RunWorld {
    legs: RunLegs,
    roads: Option<RunRoads>,
    zones: Option<Zones>,
}

impl ServiceWorld for RunWorld {
    fn legs(&self) -> &dyn ServiceLegs {
        &self.legs
    }

    fn with_roads(&self, f: &mut dyn FnMut(&dyn super::engine::ServiceRoads)) {
        match &self.roads {
            Some(r) => {
                let ctx = SearchContext::new(&r.road, &r.turns);
                let roads = FrozenRoads::with_times(&r.road, &ctx, Cow::Borrowed(&*r.times));
                f(&roads);
            }
            None => f(&NoRoads),
        }
    }

    fn zones(&self) -> Option<&Zones> {
        self.zones.as_ref()
    }
}

/// One trip's taxi leg in the last loading, with what its choice assumed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TaxiLeg {
    /// The trip.
    pub trip: TripId,
    /// Its leg, as the engine plays it: requested at the departure, from the origin's road
    /// node to the destination's.
    pub leg: Leg,
    /// The waiting time its choice assumed before a taxi picks the traveller up, in seconds.
    pub assumed_wait_s: f64,
    /// The trip's planned time, in seconds: the assumed wait, boarding and the drive.
    pub planned_s: f64,
}

impl TaxiLeg {
    /// The second its choice planned it to arrive: its departure and [`Self::planned_s`].
    #[must_use]
    pub fn planned_end_s(&self) -> f64 {
        self.leg.request_s + self.planned_s
    }
}

/// The taxis' part of the service day.
#[derive(Clone, Debug, PartialEq)]
pub struct TaxiRunDay {
    /// The legs, in trip order.
    pub legs: Vec<TaxiLeg>,
    /// Where the legs start in the engine's day (its unit requests' `leg` less this is the
    /// index in [`Self::legs`]): after the shared bikes' legs.
    pub offset: usize,
    /// What became of each, by the engine.
    pub outcome: Vec<LegOutcome>,
    /// Per leg, the second the traveller was dropped off at the destination (`NaN`: a request
    /// cancelled, or not by the window's end).
    pub arrived_s: Vec<f64>,
    /// Per leg, what the day gave it and the second it ended (`NaN`: it did not).
    pub outcomes: Vec<(ServiceOutcome, f64)>,
    /// The fleet's day: its unit requests, vehicles, bins and log.
    pub day: TaxiDay,
    /// The day summed up.
    pub summary: TaxiSummary,
    /// The engine's wall-clock seconds for the whole day (the shared bikes' included when both
    /// services run).
    pub engine_s: f64,
}

/// What a run's taxi fleet did: its service day, what `kpis.parquet`, `taxi_*.parquet` and the
/// trip rows are made of.
#[derive(Clone, Debug, PartialEq)]
pub struct TaxisRun {
    /// The fleet's name.
    pub service: String,
    /// Its vehicles.
    pub fleet: f64,
    /// The engine's wall-clock seconds (the whole service day, shared with the docked
    /// service's).
    pub engine_s: f64,
    /// The service day: `None` if no trip took a taxi in the last loading.
    pub last: Option<TaxiRunDay>,
    /// What the travellers assumed about the fleet, the whole run.
    pub assumed: TaxiAssumption,
}

impl TaxisRun {
    /// Nothing done yet by `setup`, the run's taxi fleet, assumed as `assumed`.
    #[must_use]
    pub fn of(setup: &ServiceSetup, assumed: TaxiAssumption) -> Self {
        #[allow(clippy::cast_precision_loss, reason = "a count of vehicles")]
        let fleet = match setup.parts() {
            ServiceParts::Taxi(fleet, _) => fleet.vehicle_ids.len() as f64,
            ServiceParts::BikeDocked(..) => 0.0,
        };
        Self { service: setup.name().to_string(), fleet, engine_s: 0.0, last: None, assumed }
    }
}

/// The index of `setups`' service of `kind`, if it has one.
#[must_use]
pub fn index_of(setups: &[ServiceSetup], kind: ServiceKind) -> Option<usize> {
    setups.iter().position(|s| s.kind() == kind)
}

/// What the service day is given: the last loading's shared-bike legs and taxi legs (each in
/// trip order; each leg carries what its choice assumed) and the roads its taxis drive on.
#[derive(Debug, Default)]
pub struct DayLegs {
    /// The shared-bike legs.
    pub share: Vec<ShareLeg>,
    /// The taxi legs.
    pub taxi: Vec<TaxiLeg>,
    /// The roads (none without taxi legs or an operator).
    pub roads: Option<RunRoads>,
}

impl DayLegs {
    /// The engine's legs, in the day's order: the shared bikes' first, then the taxis'.
    #[must_use]
    pub fn engine_legs(&self) -> Vec<Leg> {
        self.share.iter().map(|l| l.leg).chain(self.taxi.iter().map(|l| l.leg)).collect()
    }
}

/// The service day's operator (S304): its policy, steering the day as it would a frozen day
/// of the run.
pub struct Steer {
    /// The policy, called every `step_s` seconds from the window's start.
    pub policy: Box<dyn Policy>,
}

/// What a service day gave: the shared bikes' day and the taxis' day, each `None` without legs
/// of its kind, and an operator's decisions (empty without one).
pub struct Played {
    /// The shared bikes' day.
    pub share: Option<ShareDay>,
    /// The taxis' day.
    pub taxi: Option<TaxiRunDay>,
    /// The operator's decisions in the order taken, each with what became of it, and the
    /// moves of the target solver.
    pub decisions: Vec<DecisionRecord>,
}

/// Play the service day of `legs` of the run's services `setups` on the run's `layers`, over
/// `window` (seconds from midnight), with the built-in rules (the nearest idle taxi, the
/// charging rule) and, with `steer`, its operator; without, no van moves. With an operator the
/// day needs `legs.roads` (its vans and taxis drive on them) and has the zones of the default
/// grid, as a frozen day of the run has.
///
/// # Errors
///
/// The operator's failure, which stops the day: its own error, or a step it could not take.
///
/// # Panics
///
/// If `setups` has no docked service while `legs` has shared-bike legs, or no taxi fleet while
/// it has taxi legs, or with an operator and no roads.
pub fn play(
    setups: &Arc<Vec<ServiceSetup>>,
    layers: &Arc<StaticLayers>,
    legs: DayLegs,
    window: (f64, f64),
    seed: u64,
    steer: Option<Steer>,
) -> Result<Played, String> {
    let started = Instant::now();
    let all = legs.engine_legs();
    let DayLegs { share: share_legs, taxi: taxi_legs, roads } = legs;
    let n_share = share_legs.len();
    let destinations: Vec<(u16, NodeId)> = all.iter().map(|l| (l.service, l.destination)).collect();
    let (roads, zones, policy) = match steer {
        Some(Steer { policy }) => {
            let roads = roads.expect("an operator's day drives on the run's roads");
            let zones = Zones::grid(&roads.road, ZONE_GRID_M)?;
            (Some(roads), zones, Some(policy))
        }
        None => (roads.filter(|_| !taxi_legs.is_empty()), None, None),
    };
    let world = RunWorld {
        legs: RunLegs {
            setups: Arc::clone(setups),
            layers: Arc::clone(layers),
            destinations,
            rides: Mutex::new(HashMap::new()),
        },
        roads,
        zones,
    };
    let day = FleetDay { window, legs: all, seed };
    let mut outcome =
        FleetEngine::in_world(Arc::clone(setups), Arc::new(day), Arc::new(world), policy)
            .run_day()?;
    let engine_s = started.elapsed().as_secs_f64();
    let decisions = std::mem::take(&mut outcome.decisions);
    let taxi_out = outcome.legs.split_off(n_share);
    let taxi_index = index_of(setups, ServiceKind::Taxi);
    let taxi_day = taxi_index.and_then(|t| outcome.taxis[t].take());
    let share = (!share_legs.is_empty()).then(|| {
        let s = index_of(setups, ServiceKind::BikeDocked).expect("shared-bike legs need docks");
        share_day(&setups[s], share_legs, outcome, window, engine_s)
    });
    let taxi = (!taxi_legs.is_empty()).then(|| {
        let t = taxi_index.expect("taxi legs need a taxi fleet");
        let day = taxi_day.expect("a taxi fleet's day");
        taxi_run_day(&setups[t], n_share, taxi_legs, taxi_out, day, window, engine_s)
    });
    Ok(Played { share, taxi, decisions })
}

/// The shared bikes' part of a day: what became of their legs, summed up.
fn share_day(
    setup: &ServiceSetup,
    legs: Vec<ShareLeg>,
    outcome: FleetOutcome,
    window: (f64, f64),
    engine_s: f64,
) -> ShareDay {
    let outcomes: Vec<(ServiceOutcome, f64)> =
        outcome.legs.iter().map(|o| ServiceOutcome::of(o, window.1)).collect();
    let mut s = DockSummary::idle();
    let (mut pickup, mut ret, mut redirected) = (0.0, 0.0, 0.0);
    for ((l, o), &(m, _)) in legs.iter().zip(&outcome.legs).zip(&outcomes) {
        let w = f64::from(l.leg.weight);
        s.requests += w;
        match m {
            ServiceOutcome::Completed => s.rides += w,
            ServiceOutcome::Cancelled => s.no_bike += w,
            ServiceOutcome::Unfinished => s.unfinished += w,
        }
        if m != ServiceOutcome::Completed {
            continue;
        }
        if let Some((p, r)) = l.dock_extra_s(setup, o) {
            pickup += w * p;
            ret += w * r;
        }
        if o.redirections > 0 {
            redirected += w;
        }
    }
    s.extra_walk_at_pickup_mean_s = mean(pickup, s.rides);
    s.extra_walk_at_return_mean_s = mean(ret, s.rides);
    s.redirected_share = mean(redirected, s.rides);
    ShareDay { legs, outcome, outcomes, summary: s, engine_s }
}

/// The taxis' part of a day of the fleet `setup`: when each traveller arrived (a cancelled
/// request does not), summed up.
fn taxi_run_day(
    setup: &ServiceSetup,
    offset: usize,
    legs: Vec<TaxiLeg>,
    outcome: Vec<LegOutcome>,
    day: TaxiDay,
    window: (f64, f64),
    engine_s: f64,
) -> TaxiRunDay {
    // A cancelled request ends the trip: no second it reached the destination, whatever an
    // earlier unit request of the leg did.
    let outcomes: Vec<(ServiceOutcome, f64)> = outcome
        .iter()
        .map(|o| match ServiceOutcome::of(o, window.1) {
            (ServiceOutcome::Cancelled, _) => (ServiceOutcome::Cancelled, f64::NAN),
            other => other,
        })
        .collect();
    let arrived_s = outcomes.iter().map(|&(_, at)| at).collect();
    let summary = taxi_summary(setup, offset, &legs, &outcome, &outcomes, &day);
    TaxiRunDay { legs, offset, outcome, arrived_s, outcomes, day, summary, engine_s }
}

/// A taxi fleet's day summed up ([`TaxiSummary`]).
fn taxi_summary(
    setup: &ServiceSetup,
    offset: usize,
    legs: &[TaxiLeg],
    outcome: &[LegOutcome],
    outcomes: &[(ServiceOutcome, f64)],
    day: &TaxiDay,
) -> TaxiSummary {
    // Each leg's last unit request: the one its outcome follows.
    let mut ride_km = vec![f64::NAN; legs.len()];
    for r in &day.requests {
        if let Some(slot) = (r.leg as usize).checked_sub(offset).and_then(|i| ride_km.get_mut(i)) {
            *slot = r.trip_km;
        }
    }
    let mut s = TaxiSummary::idle();
    let (mut waits, mut in_vehicle, mut km) = (Vec::new(), 0.0, 0.0);
    for (i, ((l, o), &(m, _))) in legs.iter().zip(outcome).zip(outcomes).enumerate() {
        let w = f64::from(l.leg.weight);
        s.requests += w;
        match m {
            ServiceOutcome::Completed => s.rides += w,
            ServiceOutcome::Cancelled => s.cancelled += w,
            ServiceOutcome::Unfinished => s.unfinished += w,
        }
        if m == ServiceOutcome::Completed {
            waits.push((o.waited_s, w));
            in_vehicle += w * (o.arrived_s - o.picked_s);
            km += w * ride_km[i];
        }
    }
    let waited: f64 = waits.iter().map(|&(x, w)| x * w).sum();
    s.waiting_mean_s = mean(waited, s.rides);
    s.waiting_median_s = weighted_quantile(&mut waits, 0.5);
    s.waiting_p90_s = weighted_quantile(&mut waits, 0.9);
    s.in_vehicle_mean_s = mean(in_vehicle, s.rides);
    s.ride_km_mean = mean(km, s.rides);
    let empty: f64 = day.vehicles.iter().map(|v| v.km_empty).sum();
    s.empty_km_per_ride = mean(empty, s.rides);
    let vb = &day.vehicle_bins;
    let all: f64 = vb.state_s.iter().map(|c| c.iter().sum::<f64>()).sum();
    let busy: f64 =
        vb.state_s.iter().map(|c| c[super::engine::VehicleState::Occupied as usize]).sum();
    s.utilisation = if all > 0.0 { busy / all } else { 0.0 };
    #[allow(clippy::cast_precision_loss, reason = "a count of vehicles")]
    let fleet = match setup.parts() {
        ServiceParts::Taxi(fleet, _) => fleet.vehicle_ids.len() as f64,
        ServiceParts::BikeDocked(..) => 0.0,
    };
    s.rides_per_taxi = if fleet > 0.0 { s.rides / fleet } else { 0.0 };
    s.kwh_charged = day.vehicles.iter().map(|v| v.kwh_charged).sum();
    s
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
