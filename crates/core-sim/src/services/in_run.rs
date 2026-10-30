//! A run's services after it (S303): the trips of the run's **last loading** that chose a shared
//! bike or a taxi, played on the fleet engine **once, after the run has stopped**, in one day
//! (both services, when the run has both, on one clock; they share nothing, so each plays as it
//! would alone). The loadings never play the services, and what this day gives is never fed
//! back: the travellers planned on fixed expectations ([`super::ServiceExpectations`]), and
//! their trips keep the arrivals they planned.
//!
//! **Shared bikes.** The road loading never sees one. Each leg is the trip's chosen pickup and
//! return docks, a request when the traveller reaches the pickup dock, the ride and the walk
//! on; the engine plays them with no operator: an empty dock sends the traveller on to another,
//! a full one likewise, and a traveller who finds no bike at any dock tried walks on.
//!
//! **Taxis.** Their drives are not vehicles of the road loading: they are timed on the last
//! loading's link times ([`RunRoads::times`], what its cars met, by the second they enter each
//! link, as [`RoadTimes`]: the same road times a frozen day of the run holds, so a replay of the
//! frozen day drives the same drives), so a taxi meets the day's congestion but adds none. A trip
//! that chose a taxi asks for one at its departure, at its origin's road node; the engine's
//! matching sends the nearest idle vehicle (see [`super::taxis`]); **a request no vehicle is sent
//! to within the fleet's longest wait fails, and the trip ends there**, with no walk on.
//!
//! **Met against expected.** Each leg carries what its choice expected: the second its service
//! leg was to end ([`ShareLeg::expected_end_s`], [`TaxiLeg::expected_end_s`]: the destination,
//! or for a shared bike to transit the hub's dock). [`Met`] says what the day gave it, and
//! [`ServiceSummary`] sums the day up per service: the counts, the expected and met door-to-door
//! seconds (from the trip's departure to its service leg's end) and the mismatch (met less
//! expected, positive when worse than expected), each over people.
//!
//! **Cost:** one engine day per run (about 120 bytes and a few events per leg); per
//! redirection one walk or ride search on a static layer, rides kept per dock pair.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_routes::{Reach, SearchContext};
use openmobisim_core_types::ids::{EntityId, NodeId, TripId};

use super::costs::{DockCosts, TaxiExpectation};
use super::engine::{FleetDay, FleetEngine, Leg, NoRoads, ServiceLegs};
use super::outcome::{FleetOutcome, LegOutcome, LegStatus, TaxiDay};
use super::roads::{FrozenRoads, RoadTimes};
use super::world::ServiceWorld;
use super::{ServiceKind, ServiceParts, ServiceSetup};
use crate::layers::{LayerSetup, StaticLayers};

/// What the service day gave a trip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Met {
    /// Carried to the end of its service leg (a shared bike docked, a taxi's drop-off).
    Served,
    /// No vehicle: a taxi sent to no request within the longest wait (the trip ends there), or
    /// no bike at any dock tried (the traveller walked on).
    Failed,
    /// Still waiting or under way when the window ended, or asked after it.
    Truncated,
}

impl Met {
    /// The stable snake_case name, as `trip_results` writes it (`service_outcome`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Met::Served => "served",
            Met::Failed => "failed",
            Met::Truncated => "truncated",
        }
    }

    /// What a leg's engine outcome `o` says, and the second its service leg ended (`NaN`: it
    /// did not, by the window's end `end_s`). A failed taxi leg never ends.
    #[must_use]
    pub fn of(o: &LegOutcome, taxi: bool, end_s: f64) -> (Self, f64) {
        let ended = |t: f64| if t.is_finite() && t <= end_s { t } else { f64::NAN };
        match o.status {
            LegStatus::Served => (Met::Served, ended(o.arrived_s)),
            LegStatus::Failed if taxi => (Met::Failed, f64::NAN),
            LegStatus::Failed => (Met::Failed, ended(o.arrived_s)),
            _ => (Met::Truncated, f64::NAN),
        }
    }
}

/// One service's day summed up, over people (the travellers' weights): see the
/// [module docs](self). A mean with nobody to average is `NaN`.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct ServiceSummary {
    /// The people of the trips that chose the service in the last loading.
    pub trips: f64,
    /// Of them, served.
    pub served: f64,
    /// Failed.
    pub failed: f64,
    /// Truncated.
    pub truncated: f64,
    /// The mean expected seconds from the departure to the service leg's end, over every trip.
    pub expected_door_to_door_s: f64,
    /// The mean met seconds, over the trips whose service leg ended.
    pub met_door_to_door_s: f64,
    /// The mean of met less expected, over the same trips.
    pub mismatch_mean_s: f64,
    /// The mean of its absolute value.
    pub mismatch_abs_mean_s: f64,
    /// A taxi fleet's mean wait of the trips served, in seconds (`NaN` for a docked service).
    pub wait_mean_s: f64,
}

impl ServiceSummary {
    /// The summary of legs given as `(people, departure, expected end, met, met end, wait)`
    /// (the wait `NaN` where none is kept).
    fn of(legs: impl Iterator<Item = (f64, f64, f64, Met, f64, f64)>) -> Self {
        let mut s = Self::default();
        let (mut expected, mut met, mut signed, mut abs, mut ended) = (0.0, 0.0, 0.0, 0.0, 0.0);
        let (mut waited, mut waits) = (0.0, 0.0);
        for (w, departure, expected_end, m, met_end, wait) in legs {
            s.trips += w;
            match m {
                Met::Served => s.served += w,
                Met::Failed => s.failed += w,
                Met::Truncated => s.truncated += w,
            }
            expected += w * (expected_end - departure);
            if met_end.is_finite() {
                met += w * (met_end - departure);
                signed += w * (met_end - expected_end);
                abs += w * (met_end - expected_end).abs();
                ended += w;
            }
            if m == Met::Served && wait.is_finite() {
                waited += w * wait;
                waits += w;
            }
        }
        let mean = |x: f64, n: f64| if n > 0.0 { x / n } else { f64::NAN };
        s.expected_door_to_door_s = mean(expected, s.trips);
        s.met_door_to_door_s = mean(met, ended);
        s.mismatch_mean_s = mean(signed, ended);
        s.mismatch_abs_mean_s = mean(abs, ended);
        s.wait_mean_s = mean(waited, waits);
        s
    }

    /// The rows `kpis.parquet` writes, by metric name, in order (a `NaN` mean has none).
    #[must_use]
    pub fn rows(&self) -> Vec<(&'static str, f64)> {
        [
            ("trips", self.trips),
            ("served", self.served),
            ("failed", self.failed),
            ("truncated", self.truncated),
            ("expected_door_to_door_s", self.expected_door_to_door_s),
            ("met_door_to_door_s", self.met_door_to_door_s),
            ("mismatch_mean_s", self.mismatch_mean_s),
            ("mismatch_abs_mean_s", self.mismatch_abs_mean_s),
            ("wait_mean_s", self.wait_mean_s),
        ]
        .into_iter()
        .filter(|(_, v)| v.is_finite())
        .collect()
    }
}

/// One trip's shared-bike leg in the last loading, with what its choice expected.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShareLeg {
    /// The trip.
    pub trip: TripId,
    /// Its leg, as the engine plays it.
    pub leg: Leg,
    /// The extra seconds at the docks its choice expected (`dock_min` × 60).
    pub expected_dock_s: f64,
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

    /// The second its choice expected the leg to end: [`Self::planned_arrival_s`] and the
    /// expected extra seconds at the docks (the trip's planned arrival; for a shared bike to
    /// transit, its planned arrival at the hub's dock).
    #[must_use]
    pub fn expected_end_s(&self, setup: &ServiceSetup) -> f64 {
        self.planned_arrival_s(setup) + self.expected_dock_s
    }
}

/// The shared bikes' part of the service day.
#[derive(Clone, Debug, PartialEq)]
pub struct ShareDay {
    /// The legs, in trip order.
    pub legs: Vec<ShareLeg>,
    /// What became of them, and the docks' counts.
    pub outcome: FleetOutcome,
    /// Per leg, what the day gave it and the second its leg ended (`NaN`: it did not).
    pub met: Vec<(Met, f64)>,
    /// The day summed up.
    pub summary: ServiceSummary,
    /// The mean extra seconds the legs that arrived met at the docks (their arrival less their
    /// arrival had the planned docks had a bike and a free slot), weighted by people; `NaN`
    /// with none.
    pub extra_mean_s: f64,
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
    /// The docks' expected costs the travellers planned on, the whole run.
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
/// roads for its taxis (none without).
#[derive(Debug)]
struct RunWorld {
    legs: RunLegs,
    roads: Option<RunRoads>,
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
}

/// One trip's taxi leg in the last loading, with what its choice expected.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TaxiLeg {
    /// The trip.
    pub trip: TripId,
    /// Its leg, as the engine plays it: asked at the departure, from the origin's road node to
    /// the destination's.
    pub leg: Leg,
    /// The wait its choice expected, should a vehicle come, in seconds.
    pub expected_wait_s: f64,
    /// The share of such requests its choice expected to get no vehicle.
    pub expected_fail: f64,
    /// The door-to-door seconds its choice expected (the alternative's total).
    pub expected_s: f64,
}

impl TaxiLeg {
    /// The second its choice expected it to arrive: its departure and [`Self::expected_s`].
    #[must_use]
    pub fn expected_end_s(&self) -> f64 {
        self.leg.request_s + self.expected_s
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
    /// Per leg, the second the traveller was dropped at the destination (`NaN`: a request that
    /// failed, or not by the window's end).
    pub arrived_s: Vec<f64>,
    /// Per leg, what the day gave it and the second it ended (`NaN`: it did not).
    pub met: Vec<(Met, f64)>,
    /// The fleet's day: its unit requests, vehicles, bins and log.
    pub day: TaxiDay,
    /// The day summed up.
    pub summary: ServiceSummary,
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
    /// What the travellers expected of the fleet, the whole run.
    pub expected: TaxiExpectation,
}

impl TaxisRun {
    /// Nothing done yet by `setup`, the run's taxi fleet, expected as `expected`.
    #[must_use]
    pub fn of(setup: &ServiceSetup, expected: TaxiExpectation) -> Self {
        #[allow(clippy::cast_precision_loss, reason = "a count of vehicles")]
        let fleet = match setup.parts() {
            ServiceParts::Taxi(fleet, _) => fleet.vehicle_ids.len() as f64,
            ServiceParts::BikeDocked(..) => 0.0,
        };
        Self { service: setup.name().to_string(), fleet, engine_s: 0.0, last: None, expected }
    }
}

/// The index of `setups`' service of `kind`, if it has one.
#[must_use]
pub fn index_of(setups: &[ServiceSetup], kind: ServiceKind) -> Option<usize> {
    setups.iter().position(|s| s.kind() == kind)
}

/// What the service day is given: the last loading's shared-bike legs and taxi legs (each in
/// trip order; each leg carries what its choice expected) and the roads its taxis drive on.
#[derive(Debug, Default)]
pub struct DayLegs {
    /// The shared-bike legs.
    pub share: Vec<ShareLeg>,
    /// The taxi legs.
    pub taxi: Vec<TaxiLeg>,
    /// The roads (none without taxi legs).
    pub roads: Option<RunRoads>,
}

impl DayLegs {
    /// The engine's legs, in the day's order: the shared bikes' first, then the taxis'.
    #[must_use]
    pub fn engine_legs(&self) -> Vec<Leg> {
        self.share.iter().map(|l| l.leg).chain(self.taxi.iter().map(|l| l.leg)).collect()
    }
}

/// Play the service day of `legs` of the run's services `setups` on the run's `layers`, over
/// `window` (seconds from midnight), with the built-in rules and no operator (the nearest idle
/// taxi, the charging rule, no van moves): the shared bikes' day and the taxis' day, each `None`
/// without legs of its kind. What each leg expected rides on the leg ([`ShareLeg`],
/// [`TaxiLeg`]), so the mismatch is measured against it.
///
/// # Panics
///
/// If `setups` has no docked service while `legs` has shared-bike legs, or no taxi fleet while
/// it has taxi legs.
#[must_use]
pub fn play(
    setups: &Arc<Vec<ServiceSetup>>,
    layers: &Arc<StaticLayers>,
    legs: DayLegs,
    window: (f64, f64),
    seed: u64,
) -> (Option<ShareDay>, Option<TaxiRunDay>) {
    let started = Instant::now();
    let all = legs.engine_legs();
    let DayLegs { share: share_legs, taxi: taxi_legs, roads } = legs;
    let n_share = share_legs.len();
    let destinations: Vec<(u16, NodeId)> = all.iter().map(|l| (l.service, l.destination)).collect();
    let world = RunWorld {
        legs: RunLegs {
            setups: Arc::clone(setups),
            layers: Arc::clone(layers),
            destinations,
            rides: Mutex::new(HashMap::new()),
        },
        roads: roads.filter(|_| !taxi_legs.is_empty()),
    };
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
        share_day(&setups[s], share_legs, outcome, window, engine_s)
    });
    let taxi = (!taxi_legs.is_empty()).then(|| {
        taxi_index.expect("taxi legs need a taxi fleet");
        let day = taxi_day.expect("a taxi fleet's day");
        taxi_run_day(n_share, taxi_legs, taxi_out, day, window, engine_s)
    });
    (share, taxi)
}

/// The shared bikes' part of a day: what their legs met, summed up.
fn share_day(
    setup: &ServiceSetup,
    legs: Vec<ShareLeg>,
    outcome: FleetOutcome,
    window: (f64, f64),
    engine_s: f64,
) -> ShareDay {
    let met: Vec<(Met, f64)> = outcome.legs.iter().map(|o| Met::of(o, false, window.1)).collect();
    let summary = ServiceSummary::of(legs.iter().zip(&met).map(|(l, &(m, at))| {
        (f64::from(l.leg.weight), l.departure_s, l.expected_end_s(setup), m, at, f64::NAN)
    }));
    let (mut extra, mut people) = (0.0, 0.0);
    for (l, o) in legs.iter().zip(&outcome.legs) {
        if matches!(o.status, LegStatus::Served | LegStatus::Failed) && o.arrived_s.is_finite() {
            let w = f64::from(l.leg.weight);
            extra += w * (o.arrived_s - l.planned_arrival_s(setup));
            people += w;
        }
    }
    let extra_mean_s = if people > 0.0 { extra / people } else { f64::NAN };
    ShareDay { legs, outcome, met, summary, extra_mean_s, engine_s }
}

/// The taxis' part of a day: when each traveller arrived (a failed request does not), summed
/// up.
fn taxi_run_day(
    offset: usize,
    legs: Vec<TaxiLeg>,
    outcome: Vec<LegOutcome>,
    day: TaxiDay,
    window: (f64, f64),
    engine_s: f64,
) -> TaxiRunDay {
    let met: Vec<(Met, f64)> = outcome.iter().map(|o| Met::of(o, true, window.1)).collect();
    let arrived_s = met.iter().map(|&(_, at)| at).collect();
    let summary =
        ServiceSummary::of(legs.iter().zip(&met).zip(&outcome).map(|((l, &(m, at)), o)| {
            (f64::from(l.leg.weight), l.leg.request_s, l.expected_end_s(), m, at, o.waited_s)
        }));
    TaxiRunDay { legs, offset, outcome, arrived_s, met, day, summary, engine_s }
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
