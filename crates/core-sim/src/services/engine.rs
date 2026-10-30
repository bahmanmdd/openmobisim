//! The fleet engine: one discrete-event simulation of every service's day, with one clock.
//!
//! **Inputs** ([`FleetDay`]): the services' legs (who asks for which service, when, from where
//! to where, with what weight), the window and a seed; the services themselves
//! ([`ServiceSetup`]). Both are shared by `Arc`, so engines on the same day (vectorised
//! episodes, a lookahead's copy) share them.
//!
//! **Events**, in one queue ordered by (second, kind, key): at one second, **returns** first (a
//! bike returned is a bike found), then **arrivals** of vehicles (a taxi's drop-off, end of a
//! charge, arrival at a charger, arrival at a pickup, in that order), then **requests**, then
//! **expiries** (a taxi request that waited its longest), then the **policy step** (it sees
//! everything that happened up to its second), then the **window's end**; within a kind by the
//! leg's trip id (or the service and vehicle index), then in the order scheduled: a total
//! order, so a day replays exactly.
//!
//! **State** ([`FleetState`]): per dock its bikes; per docked service the bikes riding and in
//! vans; per vehicle its state, node and charge. The handlers' own running state (a taxi
//! fleet's waiting requests, idle vehicles by node, plug queues) is the engine's too, so a
//! clone copies a day exactly. **Conservation** is checked after every event:
//! bikes docked, riding and in vans add up to the fleet; vehicles in each state add up to the
//! fleet (asserted in debug builds, counted in release).
//!
//! **Handlers.** A docked service's requests and returns are taken by its handlers
//! ([`super::docks`]): pickups, redirections, returns and overflow, with the bikes counted and
//! the docks' counts kept bin by bin. A taxi fleet's requests, pickups, drop-offs and charges
//! are taken by its handlers ([`super::taxis`]): the built-in matching, the default charging
//! rule, waits and failures, every vehicle tracked. What the handlers need to know of the city
//! they ask a [`ServiceLegs`] (a ride between two docks, a walk from a dock to a leg's
//! destination) and a [`ServiceRoads`] (a vehicle's drive, the nodes within a few minutes).
//!
//! **A policy** ([`super::policy::Policy`]), if the engine is given one, is called at each
//! step (every [`super::policy::Policy::step_s`] seconds from the window's start) with a
//! [`View`] of the state and returns [`Decision`]s, which are checked, refused with a reason
//! or carried out ([`super::decisions`]), and recorded. Without a policy there are no steps,
//! and the day is exactly as before. A policy's failure stops the day: [`FleetEngine::advance_to`]
//! and [`FleetEngine::run_day`] return its message.
//!
//! **Cost:** per leg one event per pickup, redirection and return, and about 120 bytes; per
//! event `O(log n)` in the queue; per policy step one view (borrowed arrays, and one pass over
//! a taxi fleet's waiting requests and chargers) and the policy's own time.

use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::sync::Arc;

use openmobisim_core_types::ids::NodeId;
use openmobisim_core_types::rng::{DrawAddress, RngKey, Stream, StreamRng};

use super::decisions::{Decision, DecisionRecord};
use super::docks::{DockRun, Fleet, LegCtx};
use super::outcome::{FleetOutcome, LegOutcome, LegStatus};
use super::policy::{DayInfo, Policy, PolicyDescription, PolicySlot};
use super::queries::Queries;
use super::taxis::{Move, Next, TaxiRun, Taxis};
use super::view::{DockView, ServiceView, TaxiView, View};
use super::world::ServiceWorld;
use super::{ServiceParts, ServiceSetup};

/// One service leg: the part of a trip that needs a service's vehicle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Leg {
    /// The service, by its index in the run's services.
    pub service: u16,
    /// The trip it belongs to (the key that orders legs at one second).
    pub trip: u32,
    /// The people it stands for: the bikes or seats it takes.
    pub weight: u32,
    /// The second it asks for a vehicle: a taxi at its departure, a bike at its dock.
    pub request_s: f64,
    /// Where it starts: the road node of a taxi's pickup, the walk node of a bike's origin.
    pub origin: NodeId,
    /// Where it ends.
    pub destination: NodeId,
    /// A bike leg's planned docks, by the service's dock index (`u32::MAX` for a taxi).
    pub pickup_dock: u32,
    /// See [`Self::pickup_dock`].
    pub return_dock: u32,
    /// A bike leg's planned ride from its pickup dock to its return dock, in seconds, the
    /// fleet's speed included (`NaN` for a taxi).
    pub ride_s: f64,
    /// A bike leg's walk from its return dock to its destination, in seconds (`NaN` for a taxi).
    pub egress_s: f64,
}

/// What the services' handlers ask of the city: rides and walks a leg did not plan, for
/// redirections.
pub trait ServiceLegs: Send + Sync + std::fmt::Debug {
    /// Service `service`'s ride from dock `from` to dock `to`, in seconds (INFINITY if none).
    fn ride_s(&self, service: u16, from: u32, to: u32) -> f64;
    /// The walk from dock `dock` to leg number `leg`'s destination, in seconds.
    fn walk_to_destination_s(&self, leg: usize, dock: u32) -> f64;
}

/// No city: no ride between docks (INFINITY), no walk to a destination (0 s). For tests and a
/// day without legs ([`super::costs::DockCosts::initial`]).
#[derive(Clone, Copy, Debug, Default)]
pub struct NoLegs;

impl ServiceLegs for NoLegs {
    fn ride_s(&self, _: u16, _: u32, _: u32) -> f64 {
        f64::INFINITY
    }
    fn walk_to_destination_s(&self, _: usize, _: u32) -> f64 {
        0.0
    }
}

/// What the services' handlers ask of the roads: a vehicle's timed drive, and the bounded
/// free-flow searches a matching uses.
pub trait ServiceRoads: Send + Sync + std::fmt::Debug {
    /// The drive from `from` to `to` leaving at second `t`: seconds and metres (INFINITY if none).
    fn drive(&self, from: NodeId, to: NodeId, t: f64) -> (f64, f64);
    /// Nodes reaching `to` within `bound` s at free flow, with those seconds, nearest first (ties
    /// by node id).
    fn reach_to(&self, to: NodeId, bound: f64) -> Vec<(NodeId, f64)>;
    /// Nodes reached from `from` within `bound` s at free flow, nearest first (ties by node id).
    fn reach_from(&self, from: NodeId, bound: f64) -> Vec<(NodeId, f64)>;
    /// Seconds from each of `from` to each of `to` leaving at second `t`, row by row
    /// (`out[i * to.len() + j]`), infinite past `max_s`: by default one [`Self::drive`] per pair.
    fn travel_times(&self, from: &[NodeId], to: &[NodeId], t: f64, max_s: f64) -> Vec<f64> {
        from.iter()
            .flat_map(|&a| to.iter().map(move |&b| (a, b)))
            .map(|(a, b)| self.drive(a, b, t).0)
            .map(|s| if s <= max_s { s } else { f64::INFINITY })
            .collect()
    }
}

/// No roads: no drive (INFINITY), nothing within reach. For a day of docked services only.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoRoads;

impl ServiceRoads for NoRoads {
    fn drive(&self, _: NodeId, _: NodeId, _: f64) -> (f64, f64) {
        (f64::INFINITY, f64::INFINITY)
    }
    fn reach_to(&self, _: NodeId, _: f64) -> Vec<(NodeId, f64)> {
        Vec::new()
    }
    fn reach_from(&self, _: NodeId, _: f64) -> Vec<(NodeId, f64)> {
        Vec::new()
    }
}

/// What one fleet day is run on: its legs, its window and its seed. (The steps are the
/// policy's: [`super::policy::Policy::step_s`].)
#[derive(Clone, Debug, PartialEq)]
pub struct FleetDay {
    /// The window's start and end, in seconds from midnight.
    pub window: (f64, f64),
    /// The legs, in any order.
    pub legs: Vec<Leg>,
    /// The day's seed (the engine's own draws, from the stream `fleet_day`; a policy's, from
    /// `fleet_policy`, through [`View::rng_seed`]).
    pub seed: u64,
}

/// An `Arrival`'s rank for an operator's van (after the taxis' moves at one second).
const VAN_RANK: u64 = 5;

/// An operator's van on the way.
#[derive(Clone, Copy, Debug)]
pub(super) struct Van {
    pub(super) service: u16,
    pub(super) to: u32,
    pub(super) n: u32,
    /// Its decision's record.
    pub(super) record: u32,
}

/// A vehicle's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum VehicleState {
    /// Free, at a node or a stand.
    Idle = 0,
    /// Driving to a pickup.
    ToPickup = 1,
    /// Carrying a passenger.
    Occupied = 2,
    /// Driving, empty, where a policy sent it.
    Relocating = 3,
    /// Driving to a charger.
    ToCharger = 4,
    /// Charging.
    Charging = 5,
    /// Waiting for a plug.
    QueuedAtCharger = 6,
}

impl VehicleState {
    /// Every state, in discriminant order.
    pub const ALL: [VehicleState; 7] = [
        VehicleState::Idle,
        VehicleState::ToPickup,
        VehicleState::Occupied,
        VehicleState::Relocating,
        VehicleState::ToCharger,
        VehicleState::Charging,
        VehicleState::QueuedAtCharger,
    ];

    /// The stable snake_case name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            VehicleState::Idle => "idle",
            VehicleState::ToPickup => "to_pickup",
            VehicleState::Occupied => "occupied",
            VehicleState::Relocating => "relocating",
            VehicleState::ToCharger => "to_charger",
            VehicleState::Charging => "charging",
            VehicleState::QueuedAtCharger => "queued_at_charger",
        }
    }
}

/// One service's part of the state.
#[derive(Clone, Debug, PartialEq)]
pub enum ServiceState {
    /// A docked service: bikes per dock, riding, in vans, and the fleet they add up to.
    BikeDocked {
        /// Per dock, the bikes docked now.
        bikes: Vec<u32>,
        /// Bikes being ridden.
        riding: u64,
        /// Bikes in an operator's vans.
        moving: u64,
        /// The fleet: the bikes docked at the start of the day.
        fleet: u64,
    },
    /// A taxi fleet: each vehicle's state, node and charge, and how many are in each state.
    Taxi {
        /// Per vehicle.
        state: Vec<VehicleState>,
        /// See [`Self::Taxi::state`].
        node: Vec<NodeId>,
        /// See [`Self::Taxi::state`].
        soc: Vec<f64>,
        /// Vehicles in each [`VehicleState`], in [`VehicleState::ALL`]'s order.
        counts: [u32; 7],
    },
}

impl ServiceState {
    fn start(setup: &ServiceSetup) -> Self {
        match setup.parts() {
            ServiceParts::BikeDocked(docks, _) => ServiceState::BikeDocked {
                bikes: docks.start.clone(),
                riding: 0,
                moving: 0,
                fleet: docks.start.iter().map(|&b| u64::from(b)).sum(),
            },
            ServiceParts::Taxi(fleet, _) => {
                let n = fleet.vehicle_ids.len();
                let mut counts = [0; 7];
                counts[VehicleState::Idle as usize] = u32::try_from(n).expect("a fleet fits u32");
                ServiceState::Taxi {
                    state: vec![VehicleState::Idle; n],
                    node: fleet.vehicle_node.clone(),
                    soc: fleet.vehicle_soc.clone(),
                    counts,
                }
            }
        }
    }

    /// Whether the service's vehicles add up: bikes docked, riding and in vans to the fleet;
    /// the vehicles in each state to the fleet, each counted in its own state.
    #[must_use]
    pub fn conserved(&self) -> bool {
        match self {
            ServiceState::BikeDocked { bikes, riding, moving, fleet } => {
                bikes.iter().map(|&b| u64::from(b)).sum::<u64>() + riding + moving == *fleet
            }
            ServiceState::Taxi { state, soc, counts, .. } => {
                let mut seen = [0u32; 7];
                for s in state {
                    seen[*s as usize] += 1;
                }
                seen == *counts && soc.iter().all(|&c| c >= 0.0)
            }
        }
    }
}

/// The state of every service at the engine's clock.
#[derive(Clone, Debug, PartialEq)]
pub struct FleetState {
    /// Per service, in the run's order.
    pub services: Vec<ServiceState>,
    /// Per leg, in the day's order.
    pub legs: Vec<LegOutcome>,
}

/// What happens at one moment, in the order kinds are taken at one second.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum EventKind {
    /// A bike docked.
    Return = 0,
    /// A vehicle or a van reached where it was going.
    Arrival = 1,
    /// A leg asks for a vehicle.
    Request = 2,
    /// A taxi request's longest wait ends.
    Expire = 3,
    /// A policy step.
    Step = 4,
    /// The window's end.
    End = 5,
}

#[derive(Clone, Copy, Debug)]
struct Event {
    t: f64,
    kind: EventKind,
    /// The leg's trip id, or the vehicle's index: orders events of one kind at one second.
    key: u64,
    /// The order scheduled: the last tie-breaker.
    seq: u64,
    /// The leg, vehicle or unit request the event is about.
    subject: u32,
    /// The service of a vehicle or unit request.
    service: u16,
}

impl PartialEq for Event {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Event {}

impl PartialOrd for Event {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Event {
    // Reversed for `BinaryHeap`: the earliest first, then by kind, key and scheduling.
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .t
            .total_cmp(&self.t)
            .then_with(|| other.kind.cmp(&self.kind))
            .then_with(|| other.key.cmp(&self.key))
            .then_with(|| other.seq.cmp(&self.seq))
    }
}

/// What one [`FleetEngine::advance_to`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StepReport {
    /// Events handled.
    pub events: u64,
    /// Requests among them.
    pub requests: u64,
    /// Policy steps among them.
    pub steps: u32,
}

/// A city from owned parts: rides and walks, and roads that need no lending (the tests' and
/// [`NoLegs`]' and [`NoRoads`]'s).
#[derive(Debug)]
struct PartsWorld {
    legs: Arc<dyn ServiceLegs>,
    roads: Arc<dyn ServiceRoads>,
}

impl ServiceWorld for PartsWorld {
    fn legs(&self) -> &dyn ServiceLegs {
        &*self.legs
    }

    fn with_roads(&self, f: &mut dyn FnMut(&dyn ServiceRoads)) {
        f(&*self.roads);
    }
}

/// The fleet engine: see the [module docs](self).
#[derive(Clone, Debug)]
pub struct FleetEngine {
    pub(super) setups: Arc<Vec<ServiceSetup>>,
    day: Arc<FleetDay>,
    /// The city: rides and walks, and the roads lent one call at a time.
    world: Arc<dyn ServiceWorld>,
    pub(super) state: FleetState,
    /// Per service: a docked service's running counts.
    pub(super) docks: Vec<Option<DockRun>>,
    /// Per service: a taxi fleet's running state.
    pub(super) taxis: Vec<Option<TaxiRun>>,
    /// The policy, if any.
    policy: Option<PolicySlot>,
    /// The draws handed to the policy, one number per step.
    policy_rng: StreamRng,
    /// A policy's failure: the day stopped with this message.
    failure: Option<String>,
    /// Every decision taken, in order.
    pub(super) records: Vec<DecisionRecord>,
    /// Every van sent, in order.
    pub(super) vans: Vec<Van>,
    /// Per leg of a taxi fleet: its first unit request.
    leg_unit: Vec<u32>,
    /// Per leg: the dock its next pickup or return is at.
    leg_dock: Vec<u32>,
    /// Per leg: the docks tried since its last pickup.
    tried: Vec<Vec<u32>>,
    heap: BinaryHeap<Event>,
    seq: u64,
    now: f64,
    done: bool,
    events: u64,
    steps: u32,
    violations: u32,
    /// The events handled, `(second, kind, key)`, when [`Self::with_trace`] asked for them.
    trace: Option<Vec<(f64, EventKind, u64)>>,
}

impl FleetEngine {
    /// An engine at the window's start of `day`, for `setups`, asking `legs` for rides and
    /// walks a leg did not plan and `roads` for vehicles' drives: every leg's request, every
    /// policy step and the window's end scheduled; a taxi fleet's vehicles below the charging
    /// threshold already sent to charge.
    ///
    /// # Panics
    ///
    /// If a leg names a service `setups` does not have, a docked service's leg names a dock it
    /// does not have, or there are more than `u32::MAX` legs.
    ///
    /// With `policy`, its steps are scheduled every [`Policy::step_s`] from the window's start
    /// (a step at the window's end is not), and it is told the day begins
    /// ([`Policy::day_start`]); a `step_s` that is not a positive number stops the day at
    /// once (the engine's first [`Self::advance_to`] returns why).
    #[must_use]
    pub fn new(
        setups: Arc<Vec<ServiceSetup>>,
        day: Arc<FleetDay>,
        legs: Arc<dyn ServiceLegs>,
        roads: Arc<dyn ServiceRoads>,
        policy: Option<Box<dyn Policy>>,
    ) -> Self {
        Self::in_world(setups, day, Arc::new(PartsWorld { legs, roads }), policy)
    }

    /// As [`Self::new`], in `world`: an owned city whose roads are lent one call at a time
    /// ([`ServiceWorld`]), so the engine borrows nothing and can be kept between calls.
    ///
    /// # Panics
    ///
    /// As [`Self::new`].
    #[must_use]
    pub fn in_world(
        setups: Arc<Vec<ServiceSetup>>,
        day: Arc<FleetDay>,
        world: Arc<dyn ServiceWorld>,
        policy: Option<Box<dyn Policy>>,
    ) -> Self {
        let lent = Arc::clone(&world);
        let mut parts = Some((setups, day, world));
        let mut built = None;
        lent.with_roads(&mut |roads| {
            if let Some((setups, day, world)) = parts.take() {
                built = Some(Self::build(setups, day, world, roads));
            }
        });
        let mut engine = built.expect("a world lends its roads");
        engine.schedule_all(policy);
        lent.with_roads(&mut |roads| {
            for s in 0..engine.setups.len() {
                let start = engine.day.window.0;
                engine.with_taxis(s, roads, |taxis| taxis.start(start));
            }
        });
        engine
    }

    fn build(
        setups: Arc<Vec<ServiceSetup>>,
        day: Arc<FleetDay>,
        world: Arc<dyn ServiceWorld>,
        roads: &dyn ServiceRoads,
    ) -> Self {
        let start = day.window.0;
        let seed = day.seed;
        let services = setups.iter().map(ServiceState::start).collect();
        let docks = setups
            .iter()
            .map(|s| match s.parts() {
                ServiceParts::BikeDocked(docks, d) => Some(DockRun::new(docks, d, day.window)),
                ServiceParts::Taxi(..) => None,
            })
            .collect();
        let legs_n = day.legs.len();
        let leg_dock = day.legs.iter().map(|l| l.pickup_dock).collect();
        let mut leg_unit = vec![u32::MAX; legs_n];
        let taxis = (0_u16..)
            .zip(setups.iter())
            .map(|(s, setup)| match setup.parts() {
                ServiceParts::Taxi(fleet, d) => {
                    Some(TaxiRun::new(s, fleet, d, &day.legs, &mut leg_unit, roads, day.window))
                }
                ServiceParts::BikeDocked(..) => None,
            })
            .collect();
        let engine = Self {
            setups,
            day,
            world,
            docks,
            taxis,
            leg_unit,
            leg_dock,
            tried: vec![Vec::new(); legs_n],
            state: FleetState { services, legs: vec![LegOutcome::NOT_REQUESTED; legs_n] },
            heap: BinaryHeap::with_capacity(legs_n + 2),
            seq: 0,
            now: start,
            done: false,
            events: 0,
            steps: 0,
            violations: 0,
            trace: None,
            policy: None,
            policy_rng: StreamRng::new(RngKey::from_seed(seed), Stream::FleetPolicy),
            failure: None,
            records: Vec::new(),
            vans: Vec::new(),
        };
        let _ = &engine;
        engine
    }

    /// Schedule every leg's request, the policy's steps and the window's end.
    fn schedule_all(&mut self, policy: Option<Box<dyn Policy>>) {
        let engine = self;
        let (start, end) = engine.day.window;
        let day = Arc::clone(&engine.day);
        for (i, leg) in day.legs.iter().enumerate() {
            assert!(
                usize::from(leg.service) < engine.setups.len(),
                "leg {i} names service {}, of {}",
                leg.service,
                engine.setups.len()
            );
            if let ServiceParts::BikeDocked(docks, _) =
                engine.setups[usize::from(leg.service)].parts()
            {
                let n = docks.len();
                assert!(
                    (leg.pickup_dock as usize) < n && (leg.return_dock as usize) < n,
                    "leg {i} names docks {} and {}, of {n}",
                    leg.pickup_dock,
                    leg.return_dock
                );
            }
            if leg.request_s <= end {
                // A leg asking before the window opens asks at its start.
                let i = u32::try_from(i).expect("legs fit u32");
                engine.schedule(
                    leg.request_s.max(start),
                    EventKind::Request,
                    u64::from(leg.trip),
                    i,
                    leg.service,
                );
            }
        }
        if let Some(mut policy) = policy {
            let step = policy.step_s();
            if step.is_finite() && step > 0.0 {
                let mut t = start;
                let mut k = 0u32;
                while t < end {
                    engine.schedule(t, EventKind::Step, u64::from(k), k, 0);
                    k += 1;
                    t = start + f64::from(k) * step;
                }
            } else {
                engine.failure = Some(format!(
                    "a policy's step_s must be a positive number of seconds, not {step}"
                ));
            }
            policy.day_start(&DayInfo {
                window: day.window,
                seed: day.seed,
                services: &engine.setups,
            });
            engine.policy = Some(PolicySlot(policy));
        }
        engine.schedule(end, EventKind::End, 0, 0, 0);
    }

    /// Record every event handled, for tests and diagnostics.
    #[must_use]
    pub fn with_trace(mut self) -> Self {
        self.trace = Some(Vec::new());
        self
    }

    fn schedule(&mut self, t: f64, kind: EventKind, key: u64, subject: u32, service: u16) {
        self.heap.push(Event { t, kind, key, seq: self.seq, subject, service });
        self.seq += 1;
    }

    /// Run `f` on service `s`'s taxi handlers, if it is a taxi fleet, and schedule what they
    /// ask for inside the window; what `f` returns (`None`: not a taxi fleet).
    pub(super) fn with_taxis<R>(
        &mut self,
        s: usize,
        roads: &dyn ServiceRoads,
        f: impl FnOnce(&mut Taxis<'_>) -> R,
    ) -> Option<R> {
        let setups = Arc::clone(&self.setups);
        let day = Arc::clone(&self.day);
        let (ServiceParts::Taxi(fleet, d), Some(run)) = (setups[s].parts(), self.taxis[s].as_mut())
        else {
            return None;
        };
        let ServiceState::Taxi { state, node, soc, counts } = &mut self.state.services[s] else {
            return None;
        };
        let mut next = Vec::new();
        let mut taxis = Taxis {
            run,
            state,
            node,
            soc,
            counts,
            fleet,
            d,
            roads,
            legs: &day.legs,
            out: &mut self.state.legs,
            next: &mut next,
            now: self.now,
        };
        let r = f(&mut taxis);
        let service = u16::try_from(s).expect("services fit u16");
        for n in next {
            match n {
                Next::Vehicle(t, mv, v) => {
                    if t <= day.window.1 {
                        let key = ((mv as u64) << 48) | (u64::from(service) << 32) | u64::from(v);
                        self.schedule(t, EventKind::Arrival, key, v, service);
                    }
                }
                Next::Expire(t, u) => {
                    if t <= day.window.1 {
                        let leg = run_leg(self.taxis[s].as_ref(), u);
                        let trip = day.legs[leg].trip;
                        self.schedule(t, EventKind::Expire, u64::from(trip), u, service);
                    }
                }
            }
        }
        Some(r)
    }

    /// Schedule van `van`'s arrival at second `t`, if inside the window.
    pub(super) fn schedule_van(&mut self, t: f64, van: u32, service: u16) {
        if t <= self.day.window.1 {
            let key = (VAN_RANK << 48) | (u64::from(service) << 32) | u64::from(van);
            self.schedule(t, EventKind::Arrival, key, van, service);
        }
    }

    /// The engine's clock, in seconds from midnight.
    #[must_use]
    pub fn now(&self) -> f64 {
        self.now
    }

    /// Whether the window has ended.
    #[must_use]
    pub fn is_done(&self) -> bool {
        self.done
    }

    /// The state at the clock.
    #[must_use]
    pub fn state(&self) -> &FleetState {
        &self.state
    }

    /// The day it runs.
    #[must_use]
    pub fn day(&self) -> &FleetDay {
        &self.day
    }

    /// The policy's name and parameters, if there is a policy.
    #[must_use]
    pub fn policy_description(&self) -> Option<PolicyDescription> {
        self.policy.as_ref().map(|p| p.0.describe())
    }

    /// The decisions taken so far, in order.
    #[must_use]
    pub fn decisions(&self) -> &[DecisionRecord] {
        &self.records
    }

    /// What a policy would see now: the step index is the number of steps taken so far.
    #[must_use]
    pub fn view(&self) -> View<'_> {
        self.view_at(self.steps)
    }

    /// The view at step `step`, at the clock.
    fn view_at(&self, step: u32) -> View<'_> {
        let rng_seed = self.policy_rng.block(DrawAddress::from_one(step)).word64(0);
        let services = self
            .setups
            .iter()
            .zip(&self.state.services)
            .enumerate()
            .map(|(s, (setup, state))| match (setup.parts(), state) {
                (
                    ServiceParts::BikeDocked(docks, _),
                    ServiceState::BikeDocked { bikes, riding, moving, .. },
                ) => {
                    let run = self.docks[s].as_ref().expect("a docked service's run");
                    let since = &run.since;
                    ServiceView::BikeDocked(DockView {
                        name: setup.name(),
                        bikes,
                        slots: &docks.capacity,
                        position: &docks.position,
                        road_node: &docks.road_node,
                        pickups: &since.pickups,
                        returns: &since.returns,
                        failed: &since.failed,
                        redirected_in: &since.redirected_in,
                        redirected_out: &since.redirected_out,
                        incoming: &run.incoming,
                        moving: *moving,
                        riding: *riding,
                    })
                }
                (ServiceParts::Taxi(fleet, _), ServiceState::Taxi { state, node, soc, .. }) => {
                    let run = self.taxis[s].as_ref().expect("a taxi fleet's run");
                    let (charger_in_use, charger_queued) = run.chargers_view(fleet);
                    ServiceView::Taxi(TaxiView {
                        name: setup.name(),
                        state,
                        node,
                        until_s: &run.until,
                        soc,
                        waiting: run.waiting_view(&self.day.legs),
                        charger_node: &fleet.charger_node,
                        charger_plugs: &fleet.charger_plugs,
                        charger_in_use,
                        charger_queued,
                        nearest: &run.nearest_charger,
                    })
                }
                _ => unreachable!("a service's state is of its kind"),
            })
            .collect();
        let vehicles = self
            .state
            .services
            .iter()
            .map(|s| match s {
                ServiceState::Taxi { node, .. } => node.clone(),
                ServiceState::BikeDocked { .. } => Vec::new(),
            })
            .collect();
        let queries = Queries::new(
            Arc::clone(&self.world),
            Arc::clone(&self.setups),
            Arc::clone(&self.day),
            self.now,
            vehicles,
        );
        View { t: self.now, step, rng_seed, window: self.day.window, services, queries }
    }

    /// A step taken by the caller (an environment's): check and carry out `decisions` at the
    /// clock, as at step [`Self::view`]'s, each recorded; then the counts since the last step
    /// cleared and the step counted, as a policy's step does. Per decision given, why it was
    /// refused (`None`: carried out).
    pub fn apply(&mut self, decisions: &[Decision]) -> Vec<Option<String>> {
        let world = Arc::clone(&self.world);
        let mut out = Vec::new();
        world.with_roads(&mut |roads| out = self.apply_at(self.steps, decisions, roads));
        for run in self.docks.iter_mut().flatten() {
            run.since.clear();
        }
        self.steps += 1;
        out
    }

    /// The policy's step `step`: its view, its decisions carried out, the counts since the
    /// last step cleared.
    fn policy_step(&mut self, step: u32, roads: &dyn ServiceRoads) -> Result<(), String> {
        let Some(mut slot) = self.policy.take() else { return Ok(()) };
        let mut out = Vec::new();
        let result = slot.0.step(&self.view_at(step), &mut out);
        self.policy = Some(slot);
        for run in self.docks.iter_mut().flatten() {
            run.since.clear();
        }
        result?;
        self.apply_at(step, &out, roads);
        Ok(())
    }

    /// The events handled so far, if [`Self::with_trace`] asked for them.
    #[must_use]
    pub fn trace(&self) -> Option<&[(f64, EventKind, u64)]> {
        self.trace.as_deref()
    }

    /// Handle every event up to and including second `t` (never past the window's end), in
    /// order; the clock ends at `t`, or at the window's end.
    ///
    /// # Errors
    ///
    /// The policy's failure (its message), here or at an earlier call: the day stopped there.
    pub fn advance_to(&mut self, t: f64) -> Result<StepReport, String> {
        if let Some(why) = &self.failure {
            return Err(why.clone());
        }
        let world = Arc::clone(&self.world);
        let mut out = Ok(StepReport::default());
        world.with_roads(&mut |roads| out = self.advance_with(t, roads));
        out
    }

    fn advance_with(&mut self, t: f64, roads: &dyn ServiceRoads) -> Result<StepReport, String> {
        let mut report = StepReport::default();
        while let Some(&event) = self.heap.peek() {
            if event.t > t || self.done {
                break;
            }
            self.heap.pop();
            self.now = event.t;
            self.record_bins(event.t);
            if let Err(why) = self.handle(event, &mut report, roads) {
                self.failure = Some(why.clone());
                return Err(why);
            }
            report.events += 1;
            self.events += 1;
            if let Some(trace) = &mut self.trace {
                trace.push((event.t, event.kind, event.key));
            }
            if !self.state.services.iter().all(ServiceState::conserved) {
                self.violations += 1;
                debug_assert!(false, "conservation failed at {} after {:?}", event.t, event.kind);
            }
        }
        if !self.done {
            self.now = self.now.max(t.min(self.day.window.1));
        }
        Ok(report)
    }

    /// Note every taxi fleet's counts at the start of each bin up to second `t`.
    fn record_bins(&mut self, t: f64) {
        for (run, state) in self.taxis.iter_mut().zip(&self.state.services) {
            if let (Some(run), ServiceState::Taxi { counts, soc, .. }) = (run, state) {
                run.record(t, counts, soc);
            }
        }
    }

    fn handle(
        &mut self,
        event: Event,
        report: &mut StepReport,
        roads: &dyn ServiceRoads,
    ) -> Result<(), String> {
        match event.kind {
            EventKind::Request => {
                let i = event.subject as usize;
                let leg = &mut self.state.legs[i];
                // A further request, at the next dock tried, keeps the first second.
                if leg.status == LegStatus::NotRequested {
                    leg.status = LegStatus::Waiting;
                    leg.requested_s = event.t;
                    report.requests += 1;
                }
                let s = usize::from(self.day.legs[i].service);
                if self.taxis[s].is_some() {
                    let first = self.leg_unit[i];
                    self.with_taxis(s, roads, |taxis| taxis.request(i, first, event.t));
                } else {
                    self.handle_dock(event);
                }
            }
            EventKind::Return => self.handle_dock(event),
            EventKind::Step => {
                self.steps += 1;
                report.steps += 1;
                self.policy_step(event.subject, roads)?;
            }
            EventKind::Arrival if event.key >> 48 == VAN_RANK => {
                self.van_arrives(event.subject, event.t)
            }
            EventKind::Arrival => {
                let mv = Move::from_rank(event.key >> 48);
                let s = usize::from(event.service);
                let v = event.subject as usize;
                if matches!(mv, Move::Charged | Move::Relocated) {
                    // The end of a policy's charge or relocation.
                    if let Some(run) = self.taxis[s].as_mut() {
                        let d = std::mem::replace(&mut run.decision[v], u32::MAX);
                        if let Some(r) = self.records.get_mut(d as usize) {
                            r.finished_s = event.t;
                        }
                    }
                }
                self.with_taxis(s, roads, |taxis| {
                    taxis.arrive(mv, event.subject, event.t);
                });
            }
            EventKind::Expire => {
                self.with_taxis(usize::from(event.service), roads, |taxis| {
                    taxis.expire(event.subject, event.t);
                });
            }
            EventKind::End => {
                self.done = true;
                for s in 0..self.setups.len() {
                    self.with_taxis(s, roads, |taxis| taxis.end(event.t));
                }
                for leg in &mut self.state.legs {
                    if matches!(leg.status, LegStatus::Waiting | LegStatus::Riding) {
                        leg.status = LegStatus::Truncated;
                    }
                }
            }
        }
        Ok(())
    }

    /// A docked leg's pickup or return, by its service's handlers; the next moment they ask
    /// for scheduled, if inside the window.
    fn handle_dock(&mut self, event: Event) {
        let world = Arc::clone(&self.world);
        let i = event.subject as usize;
        let leg = &self.day.legs[i];
        let s = usize::from(leg.service);
        let (ServiceParts::BikeDocked(docks, defaults), Some(run)) =
            (self.setups[s].parts(), self.docks[s].as_mut())
        else {
            return;
        };
        let ServiceState::BikeDocked { bikes, riding, .. } = &mut self.state.services[s] else {
            return;
        };
        let mut fleet = Fleet { bikes, riding };
        let mut ctx = LegCtx {
            docks,
            defaults,
            legs: world.legs(),
            leg,
            index: i,
            tried: &mut self.tried[i],
            out: &mut self.state.legs[i],
        };
        let dock = self.leg_dock[i];
        let next = match event.kind {
            EventKind::Request => run.pickup(&mut fleet, &mut ctx, dock, event.t),
            EventKind::Return => run.ret(&mut fleet, &mut ctx, dock, event.t),
            _ => None,
        };
        if let Some((t, kind, dock)) = next {
            self.leg_dock[i] = dock;
            if t <= self.day.window.1 {
                self.schedule(t, kind, event.key, event.subject, event.service);
            }
        }
    }

    /// Run to the window's end and return the outcome.
    ///
    /// # Errors
    ///
    /// The policy's failure: its message.
    pub fn run_day(mut self) -> Result<FleetOutcome, String> {
        self.advance_to(self.day.window.1)?;
        Ok(self.into_outcome())
    }

    /// The outcome so far.
    #[must_use]
    pub fn into_outcome(mut self) -> FleetOutcome {
        let mut docks = Vec::with_capacity(self.setups.len());
        let mut overflow_max = Vec::with_capacity(self.setups.len());
        let mut dock_log = Vec::with_capacity(self.setups.len());
        for (s, run) in self.docks.iter_mut().enumerate() {
            match (run, self.setups[s].parts(), &self.state.services[s]) {
                (
                    Some(run),
                    ServiceParts::BikeDocked(setup, _),
                    ServiceState::BikeDocked { bikes, .. },
                ) => {
                    run.finish(setup, bikes, self.now);
                    docks.push(Some(run.bins.clone()));
                    overflow_max.push(run.overflow_max);
                    // In time order; one second's events in the order handled.
                    let mut log = run.log.take();
                    if let Some(log) = log.as_mut() {
                        log.sort_by(|a, b| a.t.total_cmp(&b.t));
                    }
                    dock_log.push(log);
                }
                _ => {
                    docks.push(None);
                    overflow_max.push(0.0);
                    dock_log.push(None);
                }
            }
        }
        let taxis = self
            .taxis
            .iter()
            .zip(&self.state.services)
            .map(|(run, state)| match (run, state) {
                (Some(run), ServiceState::Taxi { counts, soc, state, .. }) => {
                    Some(run.finish(self.now, counts, soc, state))
                }
                _ => None,
            })
            .collect();
        FleetOutcome {
            legs: self.state.legs,
            events: self.events,
            steps: self.steps,
            end_s: self.now,
            invariant_violations: self.violations,
            docks,
            overflow_max,
            dock_log,
            taxis,
            decisions: self.records,
        }
    }
}

/// The leg of unit request `u` of a taxi run.
fn run_leg(run: Option<&TaxiRun>, u: u32) -> usize {
    run.map_or(0, |r| r.requests[u as usize].leg as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use openmobisim_core_types::ids::EntityId;

    fn leg(trip: u32, request_s: f64) -> Leg {
        Leg {
            service: 0,
            trip,
            weight: 1,
            request_s,
            origin: NodeId::new(0),
            destination: NodeId::new(1),
            pickup_dock: 0,
            return_dock: 1,
            ride_s: f64::INFINITY,
            egress_s: 0.0,
        }
    }

    /// An engine without roads; with a policy that decides nothing every `step_s`, if given.
    fn engine(
        setups: Arc<Vec<ServiceSetup>>,
        day: Arc<FleetDay>,
        step_s: Option<f64>,
    ) -> FleetEngine {
        let policy =
            step_s.map(|s| Box::new(crate::services::policy::NoOp::every(s)) as Box<dyn Policy>);
        FleetEngine::new(setups, day, Arc::new(NoLegs), Arc::new(NoRoads), policy)
    }

    /// One docked service of two docks, 5 and 3 bikes.
    fn one_service() -> Arc<Vec<ServiceSetup>> {
        let docks = crate::services::Docks {
            external: vec!["a".into(), "b".into()],
            start: vec![5, 3],
            capacity: vec![8, 8],
            position: vec![openmobisim_core_graph::geometry::LonLat::new(0.0, 0.0); 2],
            ..crate::services::Docks::default()
        };
        Arc::new(vec![ServiceSetup::from_parts(
            "bike_share",
            ServiceParts::BikeDocked(docks, crate::services::DockDefaults::SHIPPED),
        )])
    }

    fn day(legs: Vec<Leg>) -> Arc<FleetDay> {
        Arc::new(FleetDay { window: (27_000.0, 34_200.0), legs, seed: 1 })
    }

    #[test]
    fn an_empty_day_ends_at_the_window() {
        let engine = engine(Arc::new(Vec::new()), day(Vec::new()), None);
        assert!((engine.now() - 27_000.0).abs() < 1e-9, "the clock starts at the window's start");
        let out = engine.run_day().unwrap();
        assert_eq!((out.events, out.steps, out.invariant_violations), (1, 0, 0), "the end alone");
        assert!((out.end_s - 34_200.0).abs() < 1e-9);
    }

    #[test]
    fn steps_fall_every_step_s_from_the_window_s_start() {
        let out = engine(Arc::new(Vec::new()), day(Vec::new()), Some(900.0)).run_day().unwrap();
        assert_eq!(out.steps, 8, "07:30, 07:45, ..., 09:15: two hours of 15 minutes");
    }

    #[test]
    fn events_at_one_second_go_requests_by_trip_then_the_step_then_the_end() {
        // Two legs at the step's second, given in reverse trip order; one at the window's end;
        // one before the window opens; one after it ends.
        let legs = vec![
            leg(9, 27_900.0),
            leg(4, 27_900.0),
            leg(7, 34_200.0),
            leg(2, 26_000.0),
            leg(5, 40_000.0),
        ];
        let mut engine = engine(one_service(), day(legs), Some(900.0)).with_trace();
        let report = engine.advance_to(27_900.0).unwrap();
        assert_eq!((report.requests, report.steps), (3, 2));
        let at: Vec<(EventKind, u64)> = engine
            .trace()
            .unwrap()
            .iter()
            .filter(|e| (e.0 - 27_900.0).abs() < 1e-9)
            .map(|e| (e.1, e.2))
            .collect();
        assert_eq!(
            at,
            [(EventKind::Request, 4), (EventKind::Request, 9), (EventKind::Step, 1)],
            "requests before the step, by trip id"
        );
        assert!((engine.now() - 27_900.0).abs() < 1e-9);
        let out = engine.run_day().unwrap();
        let last = out.legs[2];
        assert_eq!(
            last.status,
            LegStatus::Truncated,
            "a request at the end is taken, then truncated"
        );
        assert_eq!(out.legs[3].status, LegStatus::Truncated);
        assert!(
            (out.legs[3].requested_s - 27_000.0).abs() < 1e-9,
            "an early leg asks at the start"
        );
        assert_eq!(out.legs[4].status, LegStatus::NotRequested, "after the window: never asked");
        assert_eq!(out.invariant_violations, 0);
    }

    #[test]
    fn a_copy_runs_the_same_day_and_leaves_the_original_alone() {
        let legs = (0..50).map(|k| leg(k, 27_000.0 + f64::from(k * 97 % 7_000))).collect();
        let mut engine = engine(one_service(), day(legs), Some(600.0));
        engine.advance_to(30_000.0).unwrap();
        let copy = engine.clone();
        let a = engine.run_day().unwrap();
        // Printed, so the times not yet known (`NaN`) compare equal.
        assert_eq!(
            format!("{a:?}"),
            format!("{:?}", copy.clone().run_day().unwrap()),
            "the same day twice"
        );
        assert!(copy.now() < 30_000.0 + 1e-9, "the copy was not advanced by the first");
        let ServiceState::BikeDocked { bikes, riding, fleet, .. } = &copy.state().services[0]
        else {
            panic!("a docked service")
        };
        assert_eq!(*fleet, 8, "the fleet is the bikes docked at the start");
        assert_eq!(bikes.iter().map(|&b| u64::from(b)).sum::<u64>() + riding, 8);
    }

    #[test]
    fn conservation_counts_bikes_and_vehicles() {
        let docked =
            ServiceState::BikeDocked { bikes: vec![3, 0, 5], riding: 2, moving: 0, fleet: 10 };
        assert!(docked.conserved());
        let lost =
            ServiceState::BikeDocked { bikes: vec![3, 0, 5], riding: 1, moving: 0, fleet: 10 };
        assert!(!lost.conserved());
        let mut counts = [0; 7];
        counts[0] = 1;
        counts[2] = 1;
        let taxis = ServiceState::Taxi {
            state: vec![VehicleState::Idle, VehicleState::Occupied],
            node: vec![NodeId::new(0), NodeId::new(1)],
            soc: vec![0.5, 0.0],
            counts,
        };
        assert!(taxis.conserved());
    }
}
