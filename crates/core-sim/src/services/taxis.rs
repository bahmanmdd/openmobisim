//! Electric taxis on the fleet engine: vehicles with a battery, chargers with plugs, the
//! built-in matching and the default charging rule, with every vehicle tracked one by one.
//!
//! **Requests.** A taxi leg asks for a vehicle at its departure (the engine's `Request`), at
//! its origin's road node. A leg of weight `w` is `w` **unit requests** at once (one seat each,
//! no pooling; a weight of 0 counts as 1), matched one after the other.
//!
//! **Matching** (built in). At a request, the idle vehicles that can reach the pickup within
//! [`TaxiDefaults::max_wait_s`] are found by one bounded backward search on free-flow times
//! from the pickup; the nearest by that estimate that has charge enough for the drive to the
//! pickup and the trip, above [`TaxiDefaults::reserve_soc`], is sent (ties by vehicle index);
//! its drives are timed on the day's roads ([`ServiceRoads::drive`]). With none, the unit
//! request **waits**, and is offered, oldest first, to every vehicle freed; past
//! [`TaxiDefaults::max_wait_s`] it **fails** (an `Expire` event, taken after the requests of
//! its second and before the policy step).
//!
//! **Energy.** Each drive uses [`TaxiDefaults::kwh_per_km`] per kilometre, taken when the
//! drive starts. **The default charging rule:** a vehicle that drops a passenger below
//! [`TaxiDefaults::charge_below`] (or starts the day below it) drives to the charger nearest
//! it by free-flow time, within [`TaxiDefaults::charger_reach_s`], and charges to
//! [`TaxiDefaults::charge_to`] at the charger's power, first come first served at its plugs. A
//! charge never goes below zero.
//!
//! **A policy's decisions**: an idle vehicle sent elsewhere ([`super::decisions::Decision::Relocate`])
//! drives there empty and waits, idle, offered to the waiting requests on arriving; one sent
//! to charge ([`super::decisions::Decision::Charge`]) drives to the charger, queues and charges
//! to the share asked, as under the default rule.
//!
//! **Events.** A vehicle reaching a drop-off, finishing a charge, reaching a charger, reaching
//! a pickup and ending a relocation are the engine's `Arrival`s, taken in that order at one
//! second (a vehicle freed can take the next request at once), then by service and vehicle
//! index.
//!
//! **No traffic of its own**: the vehicles drive on the given road times and add none.
//! **Conservation**: the vehicles in each state add up to the fleet, checked by the engine
//! after every event.
//!
//! **Cost:** per vehicle about 100 bytes, per charger about 40; per unit request about 70
//! bytes, one bounded backward search, and one or two timed drives; per vehicle freed while
//! requests wait, one bounded forward search; per charger, once, one backward search for the
//! nearest charger of every node.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Arc;

use openmobisim_core_types::ids::{EntityId, NodeId};

use super::engine::{Leg, ServiceRoads, VehicleState};
use super::outcome::{LegOutcome, LegStatus};
use super::view::WaitingRequest;
use super::{Fleet, TaxiDefaults};

/// The length of a [`FleetBins`] bin, in seconds.
pub const FLEET_BIN_S: u32 = 900;

/// What became of one unit request.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TaxiRequest {
    /// The leg it belongs to, by its index in the day.
    pub leg: u32,
    /// The vehicle sent, or `None`: not (yet) sent.
    pub vehicle: Option<u32>,
    /// The second it asked (`NaN` if never).
    pub requested_s: f64,
    /// The second a vehicle was sent (`NaN` if none was).
    pub assigned_s: f64,
    /// The second the vehicle reached the pickup (`NaN` if it did not).
    pub picked_s: f64,
    /// The second the passenger reached the destination (`NaN` if they did not).
    pub arrival_s: f64,
    /// The kilometres driven empty to the pickup (0 if none was sent).
    pub empty_km: f64,
    /// The kilometres driven with the passenger (0 if not picked up).
    pub trip_km: f64,
    /// Whether it waited past [`TaxiDefaults::max_wait_s`] and failed.
    pub failed: bool,
}

/// One vehicle's day.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VehicleDay {
    /// Unit requests carried to their destination.
    pub served: u32,
    /// Kilometres with a passenger.
    pub km_occupied: f64,
    /// Kilometres empty: to pickups and to chargers.
    pub km_empty: f64,
    /// Energy used, in kWh.
    pub kwh_used: f64,
    /// Energy charged, in kWh.
    pub kwh_charged: f64,
    /// Seconds at a charger: charging, and waiting for a plug.
    pub charging_s: f64,
    /// Its charge at the clock's second, as a share of the battery.
    pub soc_end: f64,
}

/// How a taxi fleet stood over the day: the vehicles in each state at the start of each bin.
#[derive(Clone, Debug, PartialEq)]
pub struct FleetBins {
    /// The bin length, in seconds ([`FLEET_BIN_S`]).
    pub bin_s: u32,
    /// The window's start, in seconds from midnight: bin 0 starts here.
    pub start_s: f64,
    /// Per bin, the vehicles in each [`VehicleState`] (in [`VehicleState::ALL`]'s order) at its
    /// start; a bin after the clock holds the counts at the clock.
    pub states: Vec<[u32; 7]>,
}

/// One taxi fleet's day.
#[derive(Clone, Debug, PartialEq)]
pub struct TaxiDay {
    /// Per unit request, the legs' in the day's order, each leg's `weight` of them in a row.
    pub requests: Vec<TaxiRequest>,
    /// Per vehicle, in fleet order.
    pub vehicles: Vec<VehicleDay>,
    /// The fleet bin by bin.
    pub bins: FleetBins,
    /// Each vehicle bin by bin.
    pub vehicle_bins: VehicleBins,
    /// The day's events in time order (one second's in the order handled); `None` with the
    /// fleet's `log_events` off.
    pub log: Option<Vec<TaxiEvent>>,
}

/// No vehicle, unit request or charger, in a [`TaxiEvent`].
pub const NO: u32 = u32::MAX;

/// What happened, in a taxi fleet's log ([`TaxiEvent`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaxiEventKind {
    /// A traveller asked for a taxi (one unit request: one seat).
    AskedForTaxi,
    /// No idle vehicle could take the request at once: it waits, offered to every vehicle freed.
    WaitingForTaxi,
    /// A vehicle was sent to the pickup (its drive there, `km`, starts).
    TaxiAssigned,
    /// The vehicle reached the pickup; boarding starts.
    TaxiArrivedAtPickup,
    /// Boarded, the vehicle leaves with the passenger (its drive, `km`, starts).
    PickedUp,
    /// The passenger reached the destination.
    DroppedOff,
    /// No vehicle was sent within the longest wait: the request failed.
    NoTaxiWithinWaitLimit,
    /// The window ended with the request still waiting (for a vehicle, or for the one sent).
    StillWaitingAtWindowEnd,
    /// The window ended with the passenger on board.
    StillRidingAtWindowEnd,
    /// The vehicle, low on charge, drives to a charger (`km`).
    VehicleWentToCharge,
    /// It reached the charger and every plug was taken: it queues.
    QueuedForPlug,
    /// It started charging.
    StartedCharging,
    /// It finished charging, and is idle.
    Charged,
    /// An operator's policy sent it, empty, elsewhere (`km`).
    VehicleRelocating,
    /// It arrived where the policy sent it, and is idle.
    VehicleRelocated,
}

impl TaxiEventKind {
    /// The stable snake_case name, as `taxi_events.parquet` writes it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AskedForTaxi => "asked_for_taxi",
            Self::WaitingForTaxi => "waiting_for_taxi",
            Self::TaxiAssigned => "taxi_assigned",
            Self::TaxiArrivedAtPickup => "taxi_arrived_at_pickup",
            Self::PickedUp => "picked_up",
            Self::DroppedOff => "dropped_off",
            Self::NoTaxiWithinWaitLimit => "no_taxi_within_wait_limit",
            Self::StillWaitingAtWindowEnd => "still_waiting_at_window_end",
            Self::StillRidingAtWindowEnd => "still_riding_at_window_end",
            Self::VehicleWentToCharge => "vehicle_went_to_charge",
            Self::QueuedForPlug => "queued_for_plug",
            Self::StartedCharging => "started_charging",
            Self::Charged => "charged",
            Self::VehicleRelocating => "vehicle_relocating",
            Self::VehicleRelocated => "vehicle_relocated",
        }
    }
}

/// One event of a taxi fleet's day: what happened, to which unit request and vehicle, and the
/// vehicle's state and charge just after.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TaxiEvent {
    /// When, in seconds from midnight.
    pub t: f64,
    /// What happened.
    pub kind: TaxiEventKind,
    /// The unit request ([`TaxiDay::requests`]' index), or [`NO`].
    pub unit: u32,
    /// The vehicle, by fleet index, or [`NO`].
    pub vehicle: u32,
    /// The charger, by index, or [`NO`].
    pub charger: u32,
    /// The vehicle's state just after (`None` without a vehicle).
    pub state_after: Option<VehicleState>,
    /// The vehicle's charge just after, as a share of its battery (`NaN` without a vehicle);
    /// a drive's energy is taken as it starts.
    pub soc_after: f64,
    /// The kilometres of the drive that starts with this event (`NaN` for none).
    pub km: f64,
}

/// Each vehicle of a taxi fleet bin by bin ([`FLEET_BIN_S`] seconds from the window's start):
/// cell `vehicle * bins + bin`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VehicleBins {
    /// The bin length, in seconds.
    pub bin_s: u32,
    /// The window's start, in seconds from midnight.
    pub start_s: f64,
    /// The window's end: the last bin ends there.
    pub end_s: f64,
    /// Bins per vehicle.
    pub bins: usize,
    /// Seconds in each [`VehicleState`] (in [`VehicleState::ALL`]'s order).
    pub state_s: Vec<[f64; 7]>,
    /// Kilometres driven with a passenger, a drive's spread over its seconds.
    pub km_occupied: Vec<f64>,
    /// Kilometres driven empty (to pickups, to chargers, relocations), likewise.
    pub km_empty: Vec<f64>,
    /// Energy charged, in kWh, a charge's spread over its seconds.
    pub kwh_charged: Vec<f64>,
    /// The charge at the bin's start, as a share of the battery.
    pub soc_start: Vec<f64>,
    /// The charge at the bin's end (the next bin's start; the last: the clock's).
    pub soc_end: Vec<f64>,
    /// Passengers dropped off.
    pub dropped_off: Vec<f64>,
}

impl VehicleBins {
    fn new(vehicles: usize, bins: usize, window: (f64, f64)) -> Self {
        let n = vehicles * bins;
        Self {
            bin_s: FLEET_BIN_S,
            start_s: window.0,
            end_s: window.1,
            bins,
            state_s: vec![[0.0; 7]; n],
            km_occupied: vec![0.0; n],
            km_empty: vec![0.0; n],
            kwh_charged: vec![0.0; n],
            soc_start: vec![f64::NAN; n],
            soc_end: vec![f64::NAN; n],
            dropped_off: vec![0.0; n],
        }
    }

    fn bin_of(&self, t: f64) -> usize {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a bin")]
        let b = ((t - self.start_s).max(0.0) / f64::from(self.bin_s)) as usize;
        b.min(self.bins - 1)
    }

    /// Spread `amount` over the seconds `from..to` of vehicle `v`'s bins, each bin's share by
    /// its seconds (all of it in `from`'s bin for an instant); `put` adds a share to a cell.
    fn spread(&mut self, v: u32, from: f64, to: f64, amount: f64, put: fn(&mut Self, usize, f64)) {
        let base = v as usize * self.bins;
        let (a, z) = (from.max(self.start_s), to.min(self.end_s.max(self.start_s)));
        if z <= a {
            let c = base + self.bin_of(from.min(self.end_s));
            put(self, c, amount);
            return;
        }
        let span = to - from;
        let len = f64::from(self.bin_s);
        for b in self.bin_of(a)..=self.bin_of(z) {
            #[allow(clippy::cast_precision_loss, reason = "a bin index")]
            let lo = self.start_s + len * b as f64;
            let hi = if b + 1 == self.bins { f64::INFINITY } else { lo + len };
            let part = (z.min(hi) - a.max(lo)).max(0.0);
            if part > 0.0 {
                put(self, base + b, amount * part / span);
            }
        }
    }
}

/// What a vehicle's `Arrival` is, in the order they are taken at one second.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum Move {
    Dropoff = 0,
    Charged = 1,
    AtCharger = 2,
    AtPickup = 3,
    /// At the end of a policy's relocation.
    Relocated = 4,
}

impl Move {
    pub(crate) fn from_rank(rank: u64) -> Self {
        match rank {
            0 => Move::Dropoff,
            1 => Move::Charged,
            2 => Move::AtCharger,
            3 => Move::AtPickup,
            _ => Move::Relocated,
        }
    }
}

/// What a handler asks the engine to schedule.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Next {
    /// Vehicle `.2`'s move `.1` at second `.0`.
    Vehicle(f64, Move, u32),
    /// Unit request `.1` expires at second `.0`.
    Expire(f64, u32),
}

/// One taxi fleet's running state on the engine, beside the vehicles' states, nodes and
/// charges the engine's [`super::engine::ServiceState::Taxi`] holds.
#[derive(Clone, Debug)]
pub(crate) struct TaxiRun {
    /// Per vehicle: the unit request it serves, or the charger it heads to or is at.
    task: Vec<u32>,
    /// Per vehicle: the second it reached its charger.
    since: Vec<f64>,
    /// Per vehicle: the charge it charges to at its charger.
    charge_target: Vec<f64>,
    /// Per vehicle: the second its drive or charge under way ends (`NaN`: idle or queued).
    pub(crate) until: Vec<f64>,
    /// Per vehicle: the policy's decision it is carrying out (its record's index), or
    /// `u32::MAX`.
    pub(crate) decision: Vec<u32>,
    /// Idle vehicles by node.
    idle_at: BTreeMap<u32, Vec<u32>>,
    /// Unit requests waiting, oldest first.
    waiting: VecDeque<u32>,
    /// Per charger, its free plugs and the vehicles queued for one.
    charger_free: Vec<u32>,
    charger_queue: Vec<VecDeque<u32>>,
    /// Per node with a charger within reach: the nearest.
    pub(crate) nearest_charger: Arc<HashMap<u32, u32>>,
    /// Per unit request: its leg's first unit and one past its last.
    range: Vec<(u32, u32)>,
    pub(crate) requests: Vec<TaxiRequest>,
    pub(crate) days: Vec<VehicleDay>,
    pub(crate) bins: FleetBins,
    bins_n: usize,
    /// Per vehicle: the second it entered its state.
    state_since: Vec<f64>,
    vehicle_bins: VehicleBins,
    log: Option<Vec<TaxiEvent>>,
}

impl TaxiRun {
    /// The run of `fleet` over `window`, with `legs`' unit requests (`leg_unit` gets each of
    /// this service's legs' first unit); each node's nearest charger found on `roads`.
    pub(crate) fn new(
        service: u16,
        fleet: &Fleet,
        d: &TaxiDefaults,
        legs: &[Leg],
        leg_unit: &mut [u32],
        roads: &dyn ServiceRoads,
        window: (f64, f64),
    ) -> Self {
        let mut nearest: HashMap<u32, (u32, f64)> = HashMap::new();
        for (c, &node) in (0_u32..).zip(&fleet.charger_node) {
            for (from, s) in roads.reach_to(node, d.charger_reach_s) {
                let e = nearest.entry(from.raw()).or_insert((c, s));
                if s < e.1 || (s.total_cmp(&e.1) == Ordering::Equal && c < e.0) {
                    *e = (c, s);
                }
            }
        }
        let mut requests = Vec::new();
        let mut range = Vec::new();
        for (i, leg) in legs.iter().enumerate() {
            if leg.service != service {
                continue;
            }
            let first = u32::try_from(requests.len()).expect("unit requests fit u32");
            let end = first + leg.weight.max(1);
            leg_unit[i] = first;
            for _ in first..end {
                requests.push(TaxiRequest {
                    leg: u32::try_from(i).expect("legs fit u32"),
                    vehicle: None,
                    requested_s: f64::NAN,
                    assigned_s: f64::NAN,
                    picked_s: f64::NAN,
                    arrival_s: f64::NAN,
                    empty_km: 0.0,
                    trip_km: 0.0,
                    failed: false,
                });
                range.push((first, end));
            }
        }
        let n = fleet.vehicle_ids.len();
        let window_s = (window.1 - window.0).max(0.0);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a bin count")]
        let bins_n = ((window_s / f64::from(FLEET_BIN_S)).ceil() as usize).max(1);
        Self {
            task: vec![u32::MAX; n],
            since: vec![0.0; n],
            charge_target: vec![d.charge_to; n],
            until: vec![f64::NAN; n],
            decision: vec![u32::MAX; n],
            idle_at: BTreeMap::new(),
            waiting: VecDeque::new(),
            charger_free: fleet.charger_plugs.clone(),
            charger_queue: vec![VecDeque::new(); fleet.charger_ids.len()],
            nearest_charger: Arc::new(nearest.into_iter().map(|(k, (c, _))| (k, c)).collect()),
            range,
            requests,
            days: vec![VehicleDay::default(); n],
            bins: FleetBins {
                bin_s: FLEET_BIN_S,
                start_s: window.0,
                states: Vec::with_capacity(bins_n),
            },
            bins_n,
            state_since: vec![window.0; n],
            vehicle_bins: VehicleBins::new(n, bins_n, window),
            log: d.log_events.then(Vec::new),
        }
    }

    /// The requests waiting, oldest first, for a policy's view.
    pub(crate) fn waiting_view(&self, legs: &[Leg]) -> Vec<WaitingRequest> {
        self.waiting
            .iter()
            .map(|&u| {
                let r = &self.requests[u as usize];
                WaitingRequest {
                    id: u,
                    leg: r.leg,
                    origin: legs[r.leg as usize].origin,
                    since_s: r.requested_s,
                }
            })
            .collect()
    }

    /// Per charger, the plugs in use and the vehicles queued, for a policy's view.
    pub(crate) fn chargers_view(&self, fleet: &Fleet) -> (Vec<u32>, Vec<u32>) {
        let in_use =
            fleet.charger_plugs.iter().zip(&self.charger_free).map(|(p, f)| p - f).collect();
        let queued = self
            .charger_queue
            .iter()
            .map(|q| u32::try_from(q.len()).expect("a queue fits u32"))
            .collect();
        (in_use, queued)
    }

    /// Note the counts, and each vehicle's charge `soc`, at the start of every bin starting at
    /// or before second `t`.
    pub(crate) fn record(&mut self, t: f64, counts: &[u32; 7], soc: &[f64]) {
        while self.bins.states.len() < self.bins_n {
            let b = self.bins.states.len();
            #[allow(clippy::cast_precision_loss, reason = "a count of bins")]
            let start = self.bins.start_s + f64::from(FLEET_BIN_S) * b as f64;
            if start > t {
                break;
            }
            self.bins.states.push(*counts);
            for (v, &c) in soc.iter().enumerate() {
                self.vehicle_bins.soc_start[v * self.bins_n + b] = c;
            }
        }
    }

    /// The day so far: the bins after the clock hold `counts`, each vehicle's charge is `soc`
    /// and its state `state` since its last change.
    pub(crate) fn finish(
        &self,
        now: f64,
        counts: &[u32; 7],
        soc: &[f64],
        state: &[VehicleState],
    ) -> TaxiDay {
        let mut run = self.clone();
        run.record(now, counts, soc);
        while run.bins.states.len() < run.bins_n {
            let b = run.bins.states.len();
            run.bins.states.push(*counts);
            for (v, &c) in soc.iter().enumerate() {
                run.vehicle_bins.soc_start[v * run.bins_n + b] = c;
            }
        }
        for (v, s) in run.days.iter_mut().zip(soc) {
            v.soc_end = *s;
        }
        for (v, &st) in (0_u32..).zip(state) {
            let since = run.state_since[v as usize];
            run.add_state_s(v, st, since, now);
        }
        let vb = &mut run.vehicle_bins;
        for (v, &now_soc) in soc.iter().enumerate() {
            for b in 0..vb.bins {
                let c = v * vb.bins + b;
                vb.soc_end[c] = if b + 1 < vb.bins { vb.soc_start[c + 1] } else { now_soc };
            }
        }
        let mut log = run.log.take();
        if let Some(log) = log.as_mut() {
            log.sort_by(|a, b| a.t.total_cmp(&b.t));
        }
        TaxiDay {
            requests: run.requests,
            vehicles: run.days,
            bins: run.bins,
            vehicle_bins: run.vehicle_bins,
            log,
        }
    }

    fn add_state_s(&mut self, v: u32, s: VehicleState, from: f64, to: f64) {
        if to > from {
            let k = s as usize;
            let put: fn(&mut VehicleBins, usize, f64) = match k {
                0 => |b, c, x| b.state_s[c][0] += x,
                1 => |b, c, x| b.state_s[c][1] += x,
                2 => |b, c, x| b.state_s[c][2] += x,
                3 => |b, c, x| b.state_s[c][3] += x,
                4 => |b, c, x| b.state_s[c][4] += x,
                5 => |b, c, x| b.state_s[c][5] += x,
                _ => |b, c, x| b.state_s[c][6] += x,
            };
            self.vehicle_bins.spread(v, from, to, to - from, put);
        }
    }
}

/// A taxi fleet's handlers, given its run, its vehicles and what they need of the day.
pub(crate) struct Taxis<'a> {
    pub(crate) run: &'a mut TaxiRun,
    pub(crate) state: &'a mut [VehicleState],
    pub(crate) node: &'a mut [NodeId],
    pub(crate) soc: &'a mut [f64],
    pub(crate) counts: &'a mut [u32; 7],
    pub(crate) fleet: &'a Fleet,
    pub(crate) d: &'a TaxiDefaults,
    pub(crate) roads: &'a dyn ServiceRoads,
    pub(crate) legs: &'a [Leg],
    pub(crate) out: &'a mut [LegOutcome],
    pub(crate) next: &'a mut Vec<Next>,
    /// The engine's clock: the second of the event being handled.
    pub(crate) now: f64,
}

impl Taxis<'_> {
    fn set_state(&mut self, v: u32, s: VehicleState) {
        let old = self.state[v as usize];
        let since = std::mem::replace(&mut self.run.state_since[v as usize], self.now);
        self.run.add_state_s(v, old, since, self.now);
        self.counts[old as usize] -= 1;
        self.counts[s as usize] += 1;
        self.state[v as usize] = s;
    }

    /// Log `kind` at second `t` about unit request `unit`, vehicle `v` and charger `charger`
    /// ([`NO`] for none), with the drive of `km` starting then (`NaN`: none).
    fn log(&mut self, t: f64, kind: TaxiEventKind, unit: u32, v: u32, charger: u32, km: f64) {
        let (state_after, soc_after) = if v == NO {
            (None, f64::NAN)
        } else {
            (Some(self.state[v as usize]), self.soc[v as usize])
        };
        if let Some(log) = self.run.log.as_mut() {
            log.push(TaxiEvent { t, kind, unit, vehicle: v, charger, state_after, soc_after, km });
        }
    }

    /// Kilometres `km` of vehicle `v`'s drive from `from` to `to`, spread over its bins.
    fn drive_km(&mut self, v: u32, occupied: bool, from: f64, to: f64, km: f64) {
        let put: fn(&mut VehicleBins, usize, f64) =
            if occupied { |b, c, x| b.km_occupied[c] += x } else { |b, c, x| b.km_empty[c] += x };
        self.run.vehicle_bins.spread(v, from, to, km, put);
    }

    /// Vehicle `v`'s drive or charge under way ends with `mv` at second `at`.
    fn schedule(&mut self, at: f64, mv: Move, v: u32) {
        self.run.until[v as usize] = at;
        self.next.push(Next::Vehicle(at, mv, v));
    }

    fn make_idle(&mut self, v: u32, node: NodeId) {
        self.set_state(v, VehicleState::Idle);
        self.node[v as usize] = node;
        self.run.idle_at.entry(node.raw()).or_default().push(v);
    }

    fn take_idle(&mut self, v: u32) {
        let node = self.node[v as usize].raw();
        if let Some(list) = self.run.idle_at.get_mut(&node) {
            list.retain(|&x| x != v);
            if list.is_empty() {
                self.run.idle_at.remove(&node);
            }
        }
    }

    fn use_energy(&mut self, v: u32, metres: f64) {
        let kwh = self.d.kwh_per_km * metres / 1000.0;
        let i = v as usize;
        self.soc[i] = (self.soc[i] - kwh / self.d.battery_kwh).max(0.0);
        self.run.days[i].kwh_used += kwh;
    }

    /// Whether vehicle `v` has charge for `metres` more and its reserve.
    fn can_drive(&self, v: u32, metres: f64) -> bool {
        let need = self.d.kwh_per_km * metres / 1000.0 / self.d.battery_kwh;
        self.soc[v as usize] - need >= self.d.reserve_soc
    }

    fn leg_of(&self, u: u32) -> &Leg {
        &self.legs[self.run.requests[u as usize].leg as usize]
    }

    fn is_last(&self, u: u32) -> bool {
        self.run.range[u as usize].1 == u + 1
    }

    /// Send vehicle `v` from its node to unit request `u` at second `t`, if it has the charge.
    fn try_send(&mut self, v: u32, u: u32, t: f64) -> bool {
        let leg = *self.leg_of(u);
        let from = self.node[v as usize];
        let (to_s, to_m) = self.roads.drive(from, leg.origin, t);
        if !to_s.is_finite() {
            return false;
        }
        let (_, trip_m) = self.roads.drive(leg.origin, leg.destination, t + to_s);
        if !trip_m.is_finite() || !self.can_drive(v, to_m + trip_m) {
            return false;
        }
        self.take_idle(v);
        self.set_state(v, VehicleState::ToPickup);
        self.run.task[v as usize] = u;
        let r = &mut self.run.requests[u as usize];
        r.vehicle = Some(v);
        r.assigned_s = t;
        r.empty_km = to_m / 1000.0;
        if self.is_last(u) {
            let o = &mut self.out[self.run.requests[u as usize].leg as usize];
            o.vehicle = Some(v);
            o.assigned_s = t;
        }
        self.run.days[v as usize].km_empty += to_m / 1000.0;
        self.use_energy(v, to_m);
        self.node[v as usize] = leg.origin;
        self.drive_km(v, false, t, t + to_s, to_m / 1000.0);
        self.log(t, TaxiEventKind::TaxiAssigned, u, v, NO, to_m / 1000.0);
        self.schedule(t + to_s, Move::AtPickup, v);
        true
    }

    /// Match unit request `u` at second `t` to the nearest idle vehicle that can serve it.
    fn match_unit(&mut self, u: u32, t: f64) -> bool {
        let origin = self.leg_of(u).origin;
        for (node, _) in self.roads.reach_to(origin, self.d.max_wait_s) {
            let mut here = self.run.idle_at.get(&node.raw()).cloned().unwrap_or_default();
            here.sort_unstable();
            for v in here {
                if self.try_send(v, u, t) {
                    return true;
                }
            }
        }
        false
    }

    /// Offer the waiting unit requests, oldest first, to vehicle `v`, just freed at second `t`.
    fn serve_waiting(&mut self, v: u32, t: f64) {
        if self.run.waiting.is_empty() {
            return;
        }
        let reach: HashMap<u32, f64> = self
            .roads
            .reach_from(self.node[v as usize], self.d.max_wait_s)
            .into_iter()
            .map(|(n, s)| (n.raw(), s))
            .collect();
        let mut kept = VecDeque::new();
        let mut sent = false;
        while let Some(u) = self.run.waiting.pop_front() {
            let asked = self.run.requests[u as usize].requested_s;
            let origin = self.leg_of(u).origin;
            let in_time =
                reach.get(&origin.raw()).is_some_and(|&s| t + s <= asked + self.d.max_wait_s);
            if !sent && in_time && self.try_send(v, u, t) {
                sent = true;
            } else {
                kept.push_back(u);
            }
        }
        self.run.waiting = kept;
    }

    /// Send vehicle `v`, at its node at second `t`, to its nearest charger; `false` if there
    /// is none it can reach.
    fn go_charge(&mut self, v: u32, t: f64) -> bool {
        let node = self.node[v as usize];
        let Some(&c) = self.run.nearest_charger.get(&node.raw()) else { return false };
        let at = self.fleet.charger_node[c as usize];
        let (s, m) = self.roads.drive(node, at, t);
        if !s.is_finite() {
            return false;
        }
        self.drive_to_charger(v, c, (s, m), self.d.charge_to, t);
        true
    }

    /// Vehicle `v`, taken from the idle ones, drives `(seconds, metres)` to charger `c` at
    /// second `t`, to charge to `to_soc`.
    fn drive_to_charger(&mut self, v: u32, c: u32, (s, m): (f64, f64), to_soc: f64, t: f64) {
        self.set_state(v, VehicleState::ToCharger);
        self.run.task[v as usize] = c;
        self.run.charge_target[v as usize] = to_soc;
        self.run.days[v as usize].km_empty += m / 1000.0;
        self.use_energy(v, m);
        self.node[v as usize] = self.fleet.charger_node[c as usize];
        self.drive_km(v, false, t, t + s, m / 1000.0);
        self.log(t, TaxiEventKind::VehicleWentToCharge, NO, v, c, m / 1000.0);
        self.schedule(t + s, Move::AtCharger, v);
    }

    /// Vehicle `v`'s id, for messages.
    fn vehicle_id(&self, v: u32) -> &str {
        &self.fleet.vehicle_ids[v as usize]
    }

    /// Why vehicle `v` cannot take a policy's decision, if it cannot: it does not exist or is
    /// not idle.
    fn not_idle(&self, v: u32) -> Option<String> {
        match self.state.get(v as usize) {
            None => Some(format!("no vehicle {v}")),
            Some(VehicleState::Idle) => None,
            Some(s) => Some(format!("vehicle {} is not idle ({})", self.vehicle_id(v), s.as_str())),
        }
    }

    /// A policy sends idle vehicle `v` empty to `node` at second `t`: the drive's seconds and
    /// metres, or why not.
    pub(crate) fn relocate(&mut self, v: u32, node: NodeId, t: f64) -> Result<(f64, f64), String> {
        if let Some(why) = self.not_idle(v) {
            return Err(why);
        }
        let from = self.node[v as usize];
        if from == node {
            return Err(format!("vehicle {} is already there", self.vehicle_id(v)));
        }
        let (s, m) = self.roads.drive(from, node, t);
        if !s.is_finite() {
            return Err(format!("no road for vehicle {} to that node", self.vehicle_id(v)));
        }
        if !self.can_drive(v, m) {
            return Err(format!(
                "vehicle {} has not the charge for the drive and the reserve",
                self.vehicle_id(v)
            ));
        }
        self.take_idle(v);
        self.set_state(v, VehicleState::Relocating);
        self.run.days[v as usize].km_empty += m / 1000.0;
        self.use_energy(v, m);
        self.node[v as usize] = node;
        self.drive_km(v, false, t, t + s, m / 1000.0);
        self.log(t, TaxiEventKind::VehicleRelocating, NO, v, NO, m / 1000.0);
        self.schedule(t + s, Move::Relocated, v);
        Ok((s, m))
    }

    /// A policy sends idle vehicle `v` to charger `c` at second `t`, to charge to `to_soc`:
    /// the drive's seconds and metres, or why not.
    pub(crate) fn charge(
        &mut self,
        v: u32,
        c: u32,
        to_soc: f64,
        t: f64,
    ) -> Result<(f64, f64), String> {
        if let Some(why) = self.not_idle(v) {
            return Err(why);
        }
        let Some(&at) = self.fleet.charger_node.get(c as usize) else {
            return Err(format!("no charger {c}"));
        };
        let soc = self.soc[v as usize];
        if !(to_soc > soc && to_soc <= 1.0) {
            return Err(format!(
                "to_soc {to_soc} is not above vehicle {}'s charge {soc} and at most 1",
                self.vehicle_id(v)
            ));
        }
        let (s, m) = self.roads.drive(self.node[v as usize], at, t);
        if !s.is_finite() {
            return Err(format!("no road to charger {}", self.fleet.charger_ids[c as usize]));
        }
        if self.d.kwh_per_km * m / 1000.0 / self.d.battery_kwh > soc {
            return Err(format!(
                "vehicle {} has not the charge to reach charger {}",
                self.vehicle_id(v),
                self.fleet.charger_ids[c as usize]
            ));
        }
        self.take_idle(v);
        self.drive_to_charger(v, c, (s, m), to_soc, t);
        Ok((s, m))
    }

    /// Start charging vehicle `v` at charger `c` at second `t`.
    fn start_charge(&mut self, v: u32, c: u32, t: f64) {
        self.set_state(v, VehicleState::Charging);
        self.run.charger_free[c as usize] -= 1;
        let to = self.run.charge_target[v as usize];
        let kwh = ((to - self.soc[v as usize]) * self.d.battery_kwh).max(0.0);
        let s = kwh / self.fleet.charger_kw[c as usize] * 3600.0;
        self.run.vehicle_bins.spread(v, t, t + s, kwh, |b, c, x| b.kwh_charged[c] += x);
        self.log(t, TaxiEventKind::StartedCharging, NO, v, c, f64::NAN);
        self.schedule(t + s, Move::Charged, v);
    }

    /// The day's start at second `t`: every vehicle idle, those below the threshold sent to
    /// charge.
    pub(crate) fn start(&mut self, t: f64) {
        for v in 0..u32::try_from(self.state.len()).expect("a fleet fits u32") {
            let node = self.node[v as usize];
            self.run.idle_at.entry(node.raw()).or_default().push(v);
            if self.soc[v as usize] < self.d.charge_below {
                self.take_idle(v);
                if !self.go_charge(v, t) {
                    self.run.idle_at.entry(node.raw()).or_default().push(v);
                }
            }
        }
    }

    /// Leg `leg` asks for its vehicles at second `t`; its unit requests start at `first`.
    pub(crate) fn request(&mut self, leg: usize, first: u32, t: f64) {
        let end = self.run.range[first as usize].1;
        debug_assert_eq!(self.run.requests[first as usize].leg as usize, leg);
        for u in first..end {
            self.run.requests[u as usize].requested_s = t;
            self.log(t, TaxiEventKind::AskedForTaxi, u, NO, NO, f64::NAN);
            if !self.match_unit(u, t) {
                self.log(t, TaxiEventKind::WaitingForTaxi, u, NO, NO, f64::NAN);
                self.run.waiting.push_back(u);
                self.next.push(Next::Expire(t + self.d.max_wait_s, u));
            }
        }
    }

    /// Unit request `u`'s wait ends at second `t`: it fails if no vehicle was sent.
    pub(crate) fn expire(&mut self, u: u32, t: f64) {
        let r = &mut self.run.requests[u as usize];
        if r.vehicle.is_some() || r.failed {
            return;
        }
        r.failed = true;
        let leg = r.leg as usize;
        let asked = r.requested_s;
        self.run.waiting.retain(|&x| x != u);
        self.log(t, TaxiEventKind::NoTaxiWithinWaitLimit, u, NO, NO, f64::NAN);
        let last = self.is_last(u);
        let o = &mut self.out[leg];
        o.status = LegStatus::Failed;
        if last {
            o.waited_s = t - asked;
        }
    }

    /// Vehicle `v`'s move `mv` ends at second `t`.
    pub(crate) fn arrive(&mut self, mv: Move, v: u32, t: f64) {
        let i = v as usize;
        self.run.until[i] = f64::NAN;
        match mv {
            Move::AtPickup => {
                let u = self.run.task[i];
                let leg = *self.leg_of(u);
                let li = self.run.requests[u as usize].leg as usize;
                self.run.requests[u as usize].picked_s = t;
                if self.is_last(u) {
                    self.out[li].picked_s = t;
                    self.out[li].waited_s = t - self.out[li].requested_s;
                }
                let (a, z) = self.run.range[u as usize];
                if self.out[li].status == LegStatus::Waiting
                    && (a..z).all(|k| !self.run.requests[k as usize].picked_s.is_nan())
                {
                    self.out[li].status = LegStatus::Riding;
                }
                self.set_state(v, VehicleState::Occupied);
                self.log(t, TaxiEventKind::TaxiArrivedAtPickup, u, v, NO, f64::NAN);
                let leave = t + self.d.board_s;
                let (s, m) = self.roads.drive(leg.origin, leg.destination, leave);
                self.run.requests[u as usize].trip_km = m / 1000.0;
                self.run.days[i].km_occupied += m / 1000.0;
                self.use_energy(v, m);
                self.node[i] = leg.destination;
                self.drive_km(v, true, leave, leave + s, m / 1000.0);
                self.log(leave, TaxiEventKind::PickedUp, u, v, NO, m / 1000.0);
                self.schedule(leave + s, Move::Dropoff, v);
            }
            Move::Dropoff => {
                let u = self.run.task[i];
                let li = self.run.requests[u as usize].leg as usize;
                self.run.requests[u as usize].arrival_s = t;
                self.run.days[i].served += 1;
                let (a, z) = self.run.range[u as usize];
                let reqs = &self.run.requests[a as usize..z as usize];
                if self.out[li].status != LegStatus::Failed
                    && reqs.iter().all(|r| !r.arrival_s.is_nan())
                {
                    self.out[li].status = LegStatus::Served;
                    self.out[li].arrived_s =
                        reqs.iter().map(|r| r.arrival_s).fold(f64::NEG_INFINITY, f64::max);
                }
                let node = self.node[i];
                self.make_idle(v, node);
                let c = v as usize * self.run.vehicle_bins.bins + self.run.vehicle_bins.bin_of(t);
                self.run.vehicle_bins.dropped_off[c] += 1.0;
                self.log(t, TaxiEventKind::DroppedOff, u, v, NO, f64::NAN);
                if self.soc[i] < self.d.charge_below {
                    self.take_idle(v);
                    if self.go_charge(v, t) {
                        return;
                    }
                    self.run.idle_at.entry(node.raw()).or_default().push(v);
                }
                self.serve_waiting(v, t);
            }
            Move::AtCharger => {
                let c = self.run.task[i];
                self.run.since[i] = t;
                if self.run.charger_free[c as usize] > 0 {
                    self.start_charge(v, c, t);
                } else {
                    self.set_state(v, VehicleState::QueuedAtCharger);
                    self.log(t, TaxiEventKind::QueuedForPlug, NO, v, c, f64::NAN);
                    self.run.charger_queue[c as usize].push_back(v);
                }
            }
            Move::Charged => {
                let c = self.run.task[i];
                let to = self.run.charge_target[i];
                let kwh = ((to - self.soc[i]) * self.d.battery_kwh).max(0.0);
                self.run.days[i].kwh_charged += kwh;
                self.soc[i] = self.soc[i].max(to);
                self.run.charge_target[i] = self.d.charge_to;
                self.run.days[i].charging_s += t - self.run.since[i];
                self.run.charger_free[c as usize] += 1;
                if let Some(queued) = self.run.charger_queue[c as usize].pop_front() {
                    self.start_charge(queued, c, t);
                }
                let node = self.node[i];
                self.make_idle(v, node);
                self.log(t, TaxiEventKind::Charged, NO, v, c, f64::NAN);
                self.serve_waiting(v, t);
            }
            Move::Relocated => {
                let node = self.node[i];
                self.make_idle(v, node);
                self.log(t, TaxiEventKind::VehicleRelocated, NO, v, NO, f64::NAN);
                self.serve_waiting(v, t);
            }
        }
    }

    /// The window ends at second `t`: a leg still waiting has waited until then.
    pub(crate) fn end(&mut self, t: f64) {
        for u in 0..self.run.requests.len() {
            let r = self.run.requests[u];
            let last = self.run.range[u].1 as usize == u + 1;
            if last && r.picked_s.is_nan() && !r.failed && !r.requested_s.is_nan() {
                self.out[r.leg as usize].waited_s = t - r.requested_s;
            }
            let unit = u32::try_from(u).expect("unit requests fit u32");
            let v = r.vehicle.unwrap_or(NO);
            if r.requested_s.is_nan() || r.failed || !r.arrival_s.is_nan() {
                continue;
            }
            if r.picked_s.is_nan() {
                self.log(t, TaxiEventKind::StillWaitingAtWindowEnd, unit, v, NO, f64::NAN);
            } else {
                self.log(t, TaxiEventKind::StillRidingAtWindowEnd, unit, v, NO, f64::NAN);
            }
        }
    }
}
