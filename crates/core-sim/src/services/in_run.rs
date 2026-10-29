//! A docked bike service inside a run: the trips that chose a shared bike in a loading, played
//! on the fleet engine after it.
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
//! **Cost:** per loading one engine day (about 120 bytes and a few events per leg); per
//! redirection one walk or ride search on a static layer, rides kept per dock pair.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use openmobisim_core_routes::Reach;
use openmobisim_core_types::ids::{EntityId, NodeId, TripId};

use super::costs::DockCosts;
use super::engine::{FleetDay, FleetEngine, Leg, NoRoads, ServiceLegs};
use super::outcome::{FleetOutcome, LegStatus};
use super::{ServiceParts, ServiceSetup};
use crate::layers::{LayerSetup, StaticLayers};

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
    /// The service's name (the run's one docked service).
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
    /// Per leg, its destination's walk node.
    destinations: Vec<NodeId>,
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
        let (ServiceParts::BikeDocked(docks, _), Some(walk)) =
            (self.setups[0].parts(), self.layers.walk.as_ref())
        else {
            return f64::INFINITY;
        };
        docks
            .walk_node
            .get(dock as usize)
            .map_or(f64::INFINITY, |&w| shortest(walk, w, self.destinations[leg]))
    }
}

/// Play one loading's shared-bike `legs` (in trip order) of the run's docked services `setups`
/// on the run's `layers`, over `window` (seconds from midnight), with no operator.
///
/// # Panics
///
/// If `setups` is empty or its first service is not docked bikes (the run's docked service is
/// its only one).
#[must_use]
pub fn play(
    setups: &Arc<Vec<ServiceSetup>>,
    layers: &Arc<StaticLayers>,
    legs: Vec<ShareLeg>,
    window: (f64, f64),
    seed: u64,
) -> ShareDay {
    let started = Instant::now();
    let city = RunLegs {
        setups: Arc::clone(setups),
        layers: Arc::clone(layers),
        destinations: legs.iter().map(|l| l.leg.destination).collect(),
        rides: Mutex::new(HashMap::new()),
    };
    let day = FleetDay { window, legs: legs.iter().map(|l| l.leg).collect(), seed };
    let outcome = FleetEngine::new(
        Arc::clone(setups),
        Arc::new(day),
        Arc::new(city),
        Arc::new(NoRoads),
        None,
    )
    .run_day()
    .expect("no policy, no policy failure");
    let setup = &setups[0];
    let bins = outcome.docks[0].as_ref().expect("the run's service is docked bikes");
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
    ShareDay {
        legs,
        outcome,
        realised,
        mismatch_s,
        bias_s,
        extra_mean_s,
        engine_s: started.elapsed().as_secs_f64(),
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
