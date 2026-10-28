//! The fleet engine: one discrete-event simulation of every service's day, with one clock.
//!
//! **Inputs** ([`FleetDay`]): the services' legs (who asks for which service, when, from where
//! to where, with what weight), the window and a seed; the services themselves
//! ([`ServiceSetup`]). Both are shared by `Arc`, so engines on the same day (vectorised
//! episodes, a lookahead's copy) share them.
//!
//! **Events**, in one queue ordered by (second, kind, key): at one second, **returns** first (a
//! bike returned is a bike found), then **arrivals** of vehicles, then **requests**, then the
//! **policy step** (it sees everything that happened up to its second), then the **window's
//! end**; within a kind by the leg's trip id (or the vehicle's index), then in the order
//! scheduled: a total order, so a day replays exactly.
//!
//! **State** ([`FleetState`]): per dock its bikes; per docked service the bikes riding and in
//! vans; per vehicle its state, node and charge. **Conservation** is checked after every event:
//! bikes docked, riding and in vans add up to the fleet; vehicles in each state add up to the
//! fleet (asserted in debug builds, counted in release).
//!
//! **This module is the skeleton**: the clock, the queue, the state, the window's end and the
//! policy steps' times. A request is recorded and waits; what serves it (docks, then taxis) is
//! added by the services' own handlers.
//!
//! **Cost:** per leg one event and 40 bytes; per event `O(log n)` in the queue.

use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::sync::Arc;

use openmobisim_core_types::ids::NodeId;

use super::outcome::{FleetOutcome, LegOutcome, LegStatus};
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
}

/// What one fleet day is run on: its legs, its window and its seed.
#[derive(Clone, Debug, PartialEq)]
pub struct FleetDay {
    /// The window's start and end, in seconds from midnight.
    pub window: (f64, f64),
    /// The legs, in any order.
    pub legs: Vec<Leg>,
    /// The seconds between policy steps, the first at the window's start; `None`: no steps.
    pub step_s: Option<f64>,
    /// The day's seed (the engine's own draws, from the stream `fleet_day`).
    pub seed: u64,
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
    /// A policy step.
    Step = 3,
    /// The window's end.
    End = 4,
}

#[derive(Clone, Copy, Debug)]
struct Event {
    t: f64,
    kind: EventKind,
    /// The leg's trip id, or the vehicle's index: orders events of one kind at one second.
    key: u64,
    /// The order scheduled: the last tie-breaker.
    seq: u64,
    /// The leg or vehicle the event is about.
    subject: u32,
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

/// The fleet engine: see the [module docs](self).
#[derive(Clone, Debug)]
pub struct FleetEngine {
    setups: Arc<Vec<ServiceSetup>>,
    day: Arc<FleetDay>,
    state: FleetState,
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
    /// An engine at the window's start of `day`, for `setups`: every leg's request, every
    /// policy step and the window's end scheduled.
    ///
    /// # Panics
    ///
    /// If a leg names a service `setups` does not have, or there are more than `u32::MAX` legs.
    #[must_use]
    pub fn new(setups: Arc<Vec<ServiceSetup>>, day: Arc<FleetDay>) -> Self {
        let (start, end) = day.window;
        let services = setups.iter().map(ServiceState::start).collect();
        let legs_n = day.legs.len();
        let legs = vec![
            LegOutcome {
                status: LegStatus::NotRequested,
                requested_s: f64::NAN,
                arrived_s: f64::NAN,
            };
            day.legs.len()
        ];
        let mut engine = Self {
            setups,
            day,
            state: FleetState { services, legs },
            heap: BinaryHeap::with_capacity(legs_n + 2),
            seq: 0,
            now: start,
            done: false,
            events: 0,
            steps: 0,
            violations: 0,
            trace: None,
        };
        let day = Arc::clone(&engine.day);
        for (i, leg) in day.legs.iter().enumerate() {
            assert!(
                usize::from(leg.service) < engine.setups.len(),
                "leg {i} names service {}, of {}",
                leg.service,
                engine.setups.len()
            );
            if leg.request_s <= end {
                // A leg asking before the window opens asks at its start.
                let i = u32::try_from(i).expect("legs fit u32");
                engine.schedule(
                    leg.request_s.max(start),
                    EventKind::Request,
                    u64::from(leg.trip),
                    i,
                );
            }
        }
        if let Some(step) = day.step_s.filter(|s| *s > 0.0) {
            let mut t = start;
            let mut k = 0u32;
            while t < end {
                engine.schedule(t, EventKind::Step, u64::from(k), k);
                k += 1;
                t = start + f64::from(k) * step;
            }
        }
        engine.schedule(end, EventKind::End, 0, 0);
        engine
    }

    /// Record every event handled, for tests and diagnostics.
    #[must_use]
    pub fn with_trace(mut self) -> Self {
        self.trace = Some(Vec::new());
        self
    }

    fn schedule(&mut self, t: f64, kind: EventKind, key: u64, subject: u32) {
        self.heap.push(Event { t, kind, key, seq: self.seq, subject });
        self.seq += 1;
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

    /// The events handled so far, if [`Self::with_trace`] asked for them.
    #[must_use]
    pub fn trace(&self) -> Option<&[(f64, EventKind, u64)]> {
        self.trace.as_deref()
    }

    /// Handle every event up to and including second `t` (never past the window's end), in
    /// order; the clock ends at `t`, or at the window's end.
    pub fn advance_to(&mut self, t: f64) -> StepReport {
        let mut report = StepReport::default();
        while let Some(&event) = self.heap.peek() {
            if event.t > t || self.done {
                break;
            }
            self.heap.pop();
            self.now = event.t;
            self.handle(event, &mut report);
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
        report
    }

    fn handle(&mut self, event: Event, report: &mut StepReport) {
        match event.kind {
            EventKind::Request => {
                let leg = &mut self.state.legs[event.subject as usize];
                leg.status = LegStatus::Waiting;
                leg.requested_s = event.t;
                report.requests += 1;
            }
            EventKind::Step => {
                self.steps += 1;
                report.steps += 1;
            }
            EventKind::Return | EventKind::Arrival => {}
            EventKind::End => {
                self.done = true;
                for leg in &mut self.state.legs {
                    if matches!(leg.status, LegStatus::Waiting | LegStatus::Riding) {
                        leg.status = LegStatus::Truncated;
                    }
                }
            }
        }
    }

    /// Run to the window's end and return the outcome.
    #[must_use]
    pub fn run_day(mut self) -> FleetOutcome {
        self.advance_to(self.day.window.1);
        self.into_outcome()
    }

    /// The outcome so far.
    #[must_use]
    pub fn into_outcome(self) -> FleetOutcome {
        FleetOutcome {
            legs: self.state.legs,
            events: self.events,
            steps: self.steps,
            end_s: self.now,
            invariant_violations: self.violations,
        }
    }
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
            pickup_dock: u32::MAX,
            return_dock: u32::MAX,
        }
    }

    /// One docked service of two docks, 2 and 3 bikes.
    fn one_service() -> Arc<Vec<ServiceSetup>> {
        let docks = crate::services::Docks {
            external: vec!["a".into(), "b".into()],
            start: vec![2, 3],
            capacity: vec![4, 4],
            ..crate::services::Docks::default()
        };
        Arc::new(vec![ServiceSetup::from_parts(
            "bike_share",
            ServiceParts::BikeDocked(docks, crate::services::DockDefaults::SHIPPED),
        )])
    }

    fn day(legs: Vec<Leg>, step_s: Option<f64>) -> Arc<FleetDay> {
        Arc::new(FleetDay { window: (27_000.0, 34_200.0), legs, step_s, seed: 1 })
    }

    #[test]
    fn an_empty_day_ends_at_the_window() {
        let engine = FleetEngine::new(Arc::new(Vec::new()), day(Vec::new(), None));
        assert!((engine.now() - 27_000.0).abs() < 1e-9, "the clock starts at the window's start");
        let out = engine.run_day();
        assert_eq!((out.events, out.steps, out.invariant_violations), (1, 0, 0), "the end alone");
        assert!((out.end_s - 34_200.0).abs() < 1e-9);
    }

    #[test]
    fn steps_fall_every_step_s_from_the_window_s_start() {
        let out = FleetEngine::new(Arc::new(Vec::new()), day(Vec::new(), Some(900.0))).run_day();
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
        let mut engine = FleetEngine::new(one_service(), day(legs, Some(900.0))).with_trace();
        let report = engine.advance_to(27_900.0);
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
        let out = engine.run_day();
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
        let mut engine = FleetEngine::new(one_service(), day(legs, Some(600.0)));
        engine.advance_to(30_000.0);
        let copy = engine.clone();
        let a = engine.run_day();
        // Printed, so the times not yet known (`NaN`) compare equal.
        assert_eq!(format!("{a:?}"), format!("{:?}", copy.clone().run_day()), "the same day twice");
        assert!(copy.now() < 30_000.0 + 1e-9, "the copy was not advanced by the first");
        assert_eq!(
            copy.state().services[0],
            ServiceState::BikeDocked { bikes: vec![2, 3], riding: 0, moving: 0, fleet: 5 },
            "the fleet is the bikes docked at the start"
        );
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
