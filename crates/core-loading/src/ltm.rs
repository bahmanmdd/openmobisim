//! The loading at levels 2–4: the link transmission model with vehicles on
//! the curves (S84, S85), in S147's hybrid form (S151).
//!
//! # What it computes
//!
//! Every vehicle carries its route (S85) and its own clock. Every link has a
//! triangular fundamental diagram (S113) that limits three things:
//!
//! 1. **Free-flow travel** — a vehicle can leave a link no earlier than its
//!    entry time plus the link's free-flow time, which already includes the
//!    signal control delay (S90).
//! 2. **Discharge capacity** — vehicles leave a link no closer together than
//!    `PCU / (capacity × g/C)` seconds, the saturation headway of the approach
//!    (S90's green-time fraction; 1 at an unsignalised node). This is the
//!    queue, and the delay behind a bottleneck.
//! 3. **Receiving capacity** — a vehicle enters a link only while the link has
//!    room: its storage, plus what had left it one backward-wave travel time
//!    earlier, minus what has entered ([`crate::curves::room_at`], the LTM
//!    receiving condition in continuous time); and no closer behind the
//!    previous entrant than `PCU / capacity`. This is spillback. A link with
//!    *any* room admits the next vehicle, so a vehicle longer than a very
//!    short link still enters it (the overhang rule — without it, weighted
//!    travellers deadlock on short links, S148).
//!
//! The curves change **only when a whole vehicle moves** (S150's D1–D4 came
//! from fractional flow and vehicles being counted separately), so the PCU a
//! link's curves say it holds is always exactly the PCU of the vehicles on it.
//!
//! # How: in time order
//!
//! Within a loading step, vehicle movements are processed **in time order**
//! from one event queue. An event says "the vehicle at the front of link `i`
//! is due at time `t`"; processing it either releases the vehicle into its
//! next link (or out of the network), reschedules it to the moment the next
//! link will have room or inflow capacity, or parks the approach until the
//! next link discharges. Because every constraint is checked at the moment it
//! applies, with every earlier movement already committed, the result is
//! exact in continuous time: no sub-step sweeps, no smear, no step-resolved
//! spillback, and first-in-first-out on every link.
//!
//! Events are ordered by `(time, service tag, link id)` — a total order, so
//! the result never depends on input order or on how anything was stored
//! (S77's requirement, met by ordering rather than by Jacobi sweeps; S88's
//! event-queue convention). The **service tag** is each approach's virtual
//! clock, advanced by one saturation headway per vehicle it releases: approaches
//! that are never held back are served first come, first served, and saturated
//! approaches competing for the same room are served in proportion to their
//! discharge capacities (S48). A vehicle that cannot enter its next link blocks
//! everything behind it on its own link (S77's full blocking, FIFO on the whole
//! approach).
//!
//! The loading step is an output and bookkeeping boundary, not a numerical
//! one (S84): nothing about a vehicle's timing depends on it. At each step's
//! end every link forgets exits older than one backward-wave travel time
//! ([`crate::curves::LinkCurves::forget_before`]), bounding memory. The work is
//! proportional to vehicle movements, and a link nobody uses costs nothing.
//!
//! # Recorded biases
//!
//! - **The overhang rule** lets a link exceed its storage by less than one
//!   vehicle, so it spills back one vehicle later than the continuous model.
//! - **Service tags restart at each step's start**, so an approach backlogged
//!   for a long time cannot hold priority over a newcomer for more than one
//!   step; within a step, sharing is capacity-proportional.
//! - **Departures do not use inflow capacity**: they appear on their first link
//!   from outside the network, occupying its storage.

use std::cmp::{Ordering, Reverse};
use std::collections::{BinaryHeap, VecDeque};

use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_types::ids::{EntityId, LinkId};
use openmobisim_core_types::time::Second;
use openmobisim_core_types::units::{Duration, Pcu};

use crate::curves::{LinkCurves, ROOM_EPSILON, room_at};
use crate::level0::{LinkTraversal, Trajectory};
use crate::vehicle::Vehicle;

/// Two times closer than this are the same instant (float noise in sums of
/// headways).
const TIME_EPSILON: f64 = 1e-9;

/// No link: the end of an intrusive list.
const NO_LINK: u32 = u32::MAX;

/// Which term of the triangular diagram this run keeps — design §10.1's
/// nesting: levels 2–4 are **the same code**, called with a limiting
/// parameter overridden (S76). Levels 0 and 1 are separate mechanisms
/// (free-flow traversal, `crate::level0`; volume-delay, not yet built) and
/// are not represented here.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum FidelityLevel {
    /// Level 2, *point queue*: infinite storage. Queues form behind capacity,
    /// but take no space, so they never block an upstream link.
    PointQueue,
    /// Level 3, *spatial queue*: infinite backward wave speed. Storage is real
    /// and spillback happens, but room freed at a link's downstream end is
    /// available at its upstream end at once.
    SpatialQueue,
    /// Level 4, the full triangular diagram, as measured or configured.
    #[default]
    Full,
}

/// One vehicle on a link.
///
/// Stored once per vehicle in flight, so its size is asserted.
#[derive(Clone, Copy, Debug)]
struct OnLink {
    /// Index into [`LtmNetwork::vehicles`].
    slot: u32,
    /// Position of this link in the vehicle's route.
    leg: u32,
    /// When the vehicle entered the link, in seconds.
    enter: f64,
    /// Entry time plus the link's free-flow time.
    earliest_exit: f64,
}

/// "The front vehicle of `link` is due at `time`."
///
/// Superseded events are skipped rather than removed: each schedule bumps the
/// link's generation.
#[derive(Clone, Copy, Debug)]
struct Event {
    time: f64,
    tag: f64,
    link: u32,
    generation: u32,
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
    fn cmp(&self, other: &Self) -> Ordering {
        self.time
            .total_cmp(&other.time)
            .then(self.tag.total_cmp(&other.tag))
            .then(self.link.cmp(&other.link))
            .then(self.generation.cmp(&other.generation))
    }
}

/// The running state of a loading: every link's counts and queue, and every
/// vehicle's trajectory so far.
pub struct LtmNetwork<'a> {
    network: &'a RoadNetwork,
    level: FidelityLevel,
    now: f64,

    // Per link, fixed for the run (structure of arrays).
    free_flow: Vec<f64>,
    discharge_rate: Vec<f64>,
    inflow_rate: Vec<f64>,
    storage: Vec<f64>,
    wave_lag: Vec<f64>,

    // Per link, running.
    curves: Vec<LinkCurves>,
    queues: Vec<VecDeque<OnLink>>,
    /// Each approach's service tag: the virtual time its last released
    /// vehicle was due (see the module docs).
    service_tag: Vec<f64>,
    generation: Vec<u32>,
    /// Head of the list of approaches parked until this link discharges.
    first_blocked: Vec<u32>,
    /// Next approach in the list this link is parked on.
    next_blocked: Vec<u32>,

    // Per vehicle.
    vehicles: Vec<&'a Vehicle>,
    traversals: Vec<Vec<LinkTraversal>>,
    /// Slots not yet departed, latest departure first (so `pop` is next).
    pending: Vec<u32>,
    pending_sorted: bool,

    events: BinaryHeap<Reverse<Event>>,
}

impl<'a> LtmNetwork<'a> {
    /// An empty network at level 4 (the full diagram).
    ///
    /// Reads each approach's green-time fraction from `turns` once: an
    /// approach discharges at `capacity × g/C`, the largest fraction among
    /// its turns (all turns of a signalised approach share it, S90).
    #[must_use]
    pub fn new(network: &'a RoadNetwork, turns: &TurnTable) -> Self {
        let n = network.link_count() as usize;
        let mut discharge_rate = Vec::with_capacity(n);
        let mut inflow_rate = Vec::with_capacity(n);
        let mut free_flow = Vec::with_capacity(n);
        for idx in 0..n {
            let link = LinkId::from_index(idx);
            let capacity = network.link_parameters(link).capacity.get().max(f64::MIN_POSITIVE);
            let green = turns
                .turns_from(link)
                .iter()
                .map(|&t| f64::from(turns.capacity_fraction(t)))
                .reduce(f64::max)
                .unwrap_or(1.0)
                .max(f64::MIN_POSITIVE);
            discharge_rate.push(capacity * green);
            inflow_rate.push(capacity);
            free_flow.push(network.free_flow_time(link).get());
        }
        let mut sim = Self {
            network,
            level: FidelityLevel::Full,
            now: 0.0,
            free_flow,
            discharge_rate,
            inflow_rate,
            storage: Vec::new(),
            wave_lag: Vec::new(),
            curves: (0..n).map(|_| LinkCurves::new()).collect(),
            queues: (0..n).map(|_| VecDeque::new()).collect(),
            service_tag: vec![f64::NEG_INFINITY; n],
            generation: vec![0; n],
            first_blocked: vec![NO_LINK; n],
            next_blocked: vec![NO_LINK; n],
            vehicles: Vec::new(),
            traversals: Vec::new(),
            pending: Vec::new(),
            pending_sorted: true,
            events: BinaryHeap::new(),
        };
        sim.apply_level();
        sim
    }

    /// The same network, run at `level` (S76's nesting: nothing else changes).
    #[must_use]
    pub fn with_level(mut self, level: FidelityLevel) -> Self {
        self.level = level;
        self.apply_level();
        self
    }

    fn apply_level(&mut self) {
        let n = self.network.link_count() as usize;
        self.storage.clear();
        self.wave_lag.clear();
        for idx in 0..n {
            let link = LinkId::from_index(idx);
            let storage = match self.level {
                FidelityLevel::PointQueue => f64::INFINITY,
                FidelityLevel::SpatialQueue | FidelityLevel::Full => {
                    self.network.storage(link).get()
                }
            };
            let lag = match self.level {
                FidelityLevel::SpatialQueue => 0.0,
                FidelityLevel::PointQueue | FidelityLevel::Full => {
                    let w = self.network.link_parameters(link).wave_speed.get();
                    if w.is_finite() && w > 0.0 {
                        self.network.link_length(link).get() / w
                    } else {
                        0.0
                    }
                }
            };
            self.storage.push(storage);
            self.wave_lag.push(lag);
        }
    }

    /// The loading clock: the end of the last step.
    #[must_use]
    pub fn now(&self) -> Duration {
        Duration(self.now)
    }

    /// One link's counts and recent exits, for tests and diagnostics.
    #[must_use]
    pub fn curve(&self, link: LinkId) -> &LinkCurves {
        &self.curves[link.index()]
    }

    /// `N_up` of a link: every PCU that has entered it.
    #[must_use]
    pub fn cumulative_in(&self, link: LinkId) -> Pcu {
        self.curves[link.index()].cumulative_in()
    }

    /// `N_dn` of a link: every PCU that has left it.
    #[must_use]
    pub fn cumulative_out(&self, link: LinkId) -> Pcu {
        self.curves[link.index()].cumulative_out()
    }

    /// How many vehicles are on a link.
    #[must_use]
    pub fn queue_len(&self, link: LinkId) -> usize {
        self.queues[link.index()].len()
    }

    /// The PCU of the vehicles on a link. Between steps, always equal to
    /// `cumulative_in − cumulative_out`.
    #[must_use]
    pub fn queued_pcu(&self, link: LinkId) -> Pcu {
        self.queues[link.index()].iter().map(|q| self.vehicles[q.slot as usize].pcu).sum()
    }

    /// How many exits a link retains for its receiving-condition reads.
    #[must_use]
    pub fn retained_history(&self, link: LinkId) -> usize {
        self.curves[link.index()].retained()
    }

    /// Schedule a vehicle. It joins the first link of its route at its own
    /// departure second, during whichever step contains it; a departure
    /// earlier than [`Self::now`] joins at the start of the next step.
    ///
    /// # Panics
    ///
    /// Panics if the vehicle's route is empty (a [`Vehicle`] never has one),
    /// or if more than `u32::MAX` vehicles are scheduled.
    pub fn depart(&mut self, vehicle: &'a Vehicle) {
        assert!(!vehicle.route.is_empty(), "a vehicle always has a route");
        let slot = u32::try_from(self.vehicles.len()).expect("vehicle count fits u32");
        self.vehicles.push(vehicle);
        self.traversals.push(Vec::with_capacity(vehicle.route.len()));
        self.pending.push(slot);
        self.pending_sorted = false;
    }

    /// Advance the loading by `dt`, returning every trip that finished in it,
    /// in the order they finished.
    ///
    /// # Panics
    ///
    /// Panics if `dt` is not positive and finite.
    pub fn step(&mut self, dt: Duration) -> Vec<Trajectory> {
        assert!(dt.get() > 0.0 && dt.is_finite(), "a loading step must be positive, got {dt:?}");
        let (t0, t1) = (self.now, self.now + dt.get());
        let mut completed = Vec::new();
        self.sort_pending();

        loop {
            let next_event = self.events.peek().map(|Reverse(e)| e.time);
            let next_departure = self.next_departure_time(t0);
            match (next_event, next_departure) {
                (_, Some(d)) if d < t1 && next_event.is_none_or(|e| d <= e) => {
                    self.depart_next(t0);
                }
                (Some(e), _) if e < t1 => {
                    let Reverse(event) = self.events.pop().expect("just peeked");
                    self.process(event, t0, &mut completed);
                }
                _ => break,
            }
        }

        for idx in 0..self.curves.len() {
            if self.curves[idx].retained() > 0 {
                self.curves[idx].forget_before(Duration(t1 - self.wave_lag[idx]));
            }
        }
        self.now = t1;
        completed
    }

    fn sort_pending(&mut self) {
        if !self.pending_sorted {
            let vehicles = &self.vehicles;
            self.pending.sort_unstable_by(|&a, &b| {
                let (va, vb) = (vehicles[a as usize], vehicles[b as usize]);
                (vb.departure, vb.id.raw(), b).cmp(&(va.departure, va.id.raw(), a))
            });
            self.pending_sorted = true;
        }
    }

    fn next_departure_time(&self, t0: f64) -> Option<f64> {
        self.pending
            .last()
            .map(|&slot| f64::from(self.vehicles[slot as usize].departure.get()).max(t0))
    }

    /// The next pending vehicle joins its first link.
    fn depart_next(&mut self, t0: f64) {
        let slot = self.pending.pop().expect("a departure is pending");
        let vehicle = self.vehicles[slot as usize];
        let first = vehicle.route[0].index();
        let enter = f64::from(vehicle.departure.get()).max(t0);
        self.curves[first].record_departure(vehicle.pcu);
        self.join(
            first,
            OnLink { slot, leg: 0, enter, earliest_exit: enter + self.free_flow[first] },
            t0,
        );
    }

    /// Put a vehicle at the back of link `i`'s queue; if it is the front,
    /// schedule it.
    fn join(&mut self, i: usize, on_link: OnLink, t0: f64) {
        self.queues[i].push_back(on_link);
        if self.queues[i].len() == 1 {
            self.schedule_front(i, t0, f64::NEG_INFINITY);
        }
    }

    /// The front vehicle of link `i`: `(when it is ready to leave, its tag)`.
    fn front_due(&self, i: usize, t0: f64) -> Option<(f64, f64)> {
        let front = self.queues[i].front()?;
        let headway = self.vehicles[front.slot as usize].pcu.get() / self.discharge_rate[i];
        let tag = front.earliest_exit.max(self.service_tag[i] + headway).max(t0);
        let ready = tag.max(self.curves[i].last_exit().get() + headway);
        Some((ready, tag))
    }

    /// Schedule link `i`'s front vehicle, no earlier than `not_before`.
    fn schedule_front(&mut self, i: usize, t0: f64, not_before: f64) {
        if let Some((ready, tag)) = self.front_due(i, t0) {
            self.generation[i] = self.generation[i].wrapping_add(1);
            self.events.push(Reverse(Event {
                time: ready.max(not_before),
                tag,
                link: link_u32(i),
                generation: self.generation[i],
            }));
        }
    }

    fn process(&mut self, event: Event, t0: f64, completed: &mut Vec<Trajectory>) {
        let i = event.link as usize;
        if event.generation != self.generation[i] {
            return;
        }
        let Some((ready, tag)) = self.front_due(i, t0) else {
            return;
        };
        let t = ready.max(event.time);
        let front = *self.queues[i].front().expect("front_due found one");
        let vehicle = self.vehicles[front.slot as usize];
        let pcu = vehicle.pcu.get();
        let Some(j) = vehicle.route.get(front.leg as usize + 1).map(|l| l.index()) else {
            self.release(i, t, tag, None, t0, completed);
            return;
        };

        // Inflow capacity of the next link.
        let inflow_ok = self.curves[j].last_entry().get() + pcu / self.inflow_rate[j];
        if inflow_ok > t + TIME_EPSILON {
            self.schedule_front(i, t0, inflow_ok);
            return;
        }
        // Room on the next link.
        let (storage, lag) = (Pcu(self.storage[j]), Duration(self.wave_lag[j]));
        if room_at(&self.curves[j], storage, lag, Duration(t)).get() > ROOM_EPSILON {
            self.release(i, t, tag, Some(j), t0, completed);
            return;
        }
        let needed = self.curves[j].cumulative_in() - storage + Pcu(ROOM_EPSILON);
        match self.curves[j].first_exit_exceeding(needed) {
            Some(freed) if freed.get() + lag.get() > t + TIME_EPSILON => {
                self.schedule_front(i, t0, freed.get() + lag.get());
            }
            Some(_) => {
                // The room was freed at `t`, within float noise of the read above.
                self.release(i, t, tag, Some(j), t0, completed);
            }
            None => {
                // Parked until `j` discharges (S77: nothing behind it moves).
                self.next_blocked[i] = self.first_blocked[j];
                self.first_blocked[j] = link_u32(i);
            }
        }
    }

    fn release(
        &mut self,
        i: usize,
        at: f64,
        tag: f64,
        next: Option<usize>,
        t0: f64,
        completed: &mut Vec<Trajectory>,
    ) {
        let on_link = self.queues[i].pop_front().expect("releasing the front vehicle");
        let slot = on_link.slot as usize;
        let vehicle = self.vehicles[slot];
        self.traversals[slot].push(LinkTraversal {
            link: LinkId::from_index(i),
            enter: floor_to_second(on_link.enter),
            exit: floor_to_second(at),
        });
        self.curves[i].record_exit(Duration(at), vehicle.pcu);
        self.curves[i].set_last_exit(Duration(at));
        self.service_tag[i] = tag;

        // Room has been freed on `i`: wake every approach parked on it, at the
        // moment the news reaches `i`'s upstream end.
        let woken_at = at + self.wave_lag[i];
        let mut parked = std::mem::replace(&mut self.first_blocked[i], NO_LINK);
        while parked != NO_LINK {
            let u = parked as usize;
            parked = std::mem::replace(&mut self.next_blocked[u], NO_LINK);
            self.schedule_front(u, t0, woken_at);
        }

        match next {
            Some(j) => {
                self.curves[j].record_entry(Duration(at), vehicle.pcu);
                let on_next = OnLink {
                    slot: on_link.slot,
                    leg: on_link.leg + 1,
                    enter: at,
                    earliest_exit: at + self.free_flow[j],
                };
                self.join(j, on_next, t0);
            }
            None => completed.push(Trajectory {
                vehicle: vehicle.id,
                links: std::mem::take(&mut self.traversals[slot]),
            }),
        }
        self.schedule_front(i, t0, at);
    }
}

fn link_u32(index: usize) -> u32 {
    u32::try_from(index).expect("link ids are u32 (Foundations §1)")
}

/// Floor a time in seconds onto the whole-second clock (S88).
fn floor_to_second(seconds: f64) -> Second {
    debug_assert!(seconds.is_finite(), "a traversal time must be finite, got {seconds}");
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "loading times are finite, non-negative and within the u32 clock; truncation is \
                  S88's deliberate floor to a whole second"
    )]
    let whole = seconds.max(0.0) as u32;
    Second(whole)
}

/// Run every vehicle to completion or to the end of `window`, whichever
/// comes first: the whole-network entry point.
///
/// Vehicles depart at their own departure second. Trips still in flight when
/// `window` ends are **not** returned; `core-sim` counts them as truncated
/// (S57). `level` picks which term of the triangular diagram is in force
/// (S76).
///
/// # Panics
///
/// Panics if `step` is not positive.
#[must_use]
pub fn run_ltm(
    network: &RoadNetwork,
    turns: &TurnTable,
    vehicles: &[Vehicle],
    window: Duration,
    step: Duration,
    level: FidelityLevel,
) -> Vec<Trajectory> {
    assert!(step.get() > 0.0, "the loading step must be positive, got {step:?}");
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "window and step are finite and non-negative scenario parameters"
    )]
    let n_steps = (window.get() / step.get()).ceil().max(0.0) as usize;

    let mut sim = LtmNetwork::new(network, turns).with_level(level);
    for vehicle in vehicles {
        if Duration::from_clock(vehicle.departure) < window {
            sim.depart(vehicle);
        }
    }
    let mut completed = Vec::new();
    for _ in 0..n_steps {
        completed.extend(sim.step(step));
    }
    completed.retain(|t| Duration::from_clock(t.arrival()) <= window);
    completed
}

#[cfg(test)]
mod tests {
    use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
    use openmobisim_core_graph::geometry::LonLat;
    use openmobisim_core_graph::network::{LinkSpec, RoadNetworkBuilder};
    use openmobisim_core_types::diagnostics::Diagnostics;
    use openmobisim_core_types::ids::VehicleId;

    use super::*;

    fn link(network: &RoadNetwork, external: &str) -> LinkId {
        network.link_external_ids().typed_id_of::<LinkId>(external).expect("known link")
    }

    /// Size check: one of these exists per vehicle in flight.
    #[test]
    fn a_vehicle_on_a_link_is_24_bytes() {
        assert_eq!(std::mem::size_of::<OnLink>(), 24);
    }

    /// Size check: the event queue holds about one of these per busy link.
    #[test]
    fn an_event_is_24_bytes() {
        assert_eq!(std::mem::size_of::<Event>(), 24);
    }

    #[test]
    fn a_vehicle_can_cross_several_links_within_one_step() {
        // Three ~100 m residential links: a lone vehicle crosses all three in
        // one 300 s step — the risk S85 named.
        let mut b = RoadNetworkBuilder::new();
        b.add_node("a", LonLat::new(4.8000, 45.700));
        b.add_node("b", LonLat::new(4.8012, 45.700));
        b.add_node("c", LonLat::new(4.8024, 45.700));
        b.add_node("d", LonLat::new(4.8036, 45.700));
        b.add_link("ab", "a", "b", LinkSpec::new(RoadClass::Residential));
        b.add_link("bc", "b", "c", LinkSpec::new(RoadClass::Residential));
        b.add_link("cd", "c", "d", LinkSpec::new(RoadClass::Residential));
        let mut diagnostics = Diagnostics::new();
        let network = b
            .build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut diagnostics)
            .expect("buildable");
        let turns = TurnTable::build(&network, SignalDefaults::SHIPPED);
        let route = vec![link(&network, "ab"), link(&network, "bc"), link(&network, "cd")];
        let vehicle = Vehicle::new(VehicleId::new(0), route, Pcu(1.0), Second(0));

        let trajectories = run_ltm(
            &network,
            &turns,
            &[vehicle],
            Duration(300.0),
            Duration(300.0),
            FidelityLevel::Full,
        );
        assert_eq!(trajectories.len(), 1, "the vehicle must complete within the one step given");
        assert_eq!(trajectories[0].links.len(), 3);
    }

    /// S76's exact-assertion pair, through the whole engine: `bc` is short,
    /// low-capacity and signalised at its end, downstream of higher-capacity
    /// `ab`. Under the full diagram a burst fills `bc` and spillback throttles
    /// `ab`'s discharge (S77); with
    /// infinite storage it cannot, so `ab` discharges strictly more in the same
    /// time. With infinite wave speed, room freed on `bc` is available at once,
    /// so `ab` discharges at least as much as under the full diagram.
    #[test]
    fn the_nested_levels_order_discharge_as_the_ladder_predicts() {
        let mut b = RoadNetworkBuilder::new();
        b.add_node("a", LonLat::new(4.8000, 45.700));
        b.add_node("b", LonLat::new(4.8012, 45.700));
        b.add_node("c", LonLat::new(4.8014, 45.700)); // "bc" ~15 m: tiny storage
        b.add_node("d", LonLat::new(4.8026, 45.700));
        b.add_link("ab", "a", "b", LinkSpec::new(RoadClass::Secondary));
        b.add_link("bc", "b", "c", LinkSpec::new(RoadClass::Service));
        b.add_link("cd", "c", "d", LinkSpec::new(RoadClass::Service));
        // A signal at c: bc discharges at g/C of the rate it can receive, so it fills.
        b.mark_signalised("c");
        let mut diagnostics = Diagnostics::new();
        let network = b
            .build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut diagnostics)
            .expect("buildable");
        let turns = TurnTable::build(&network, SignalDefaults::SHIPPED);
        let (ab, bc, cd) = (link(&network, "ab"), link(&network, "bc"), link(&network, "cd"));
        assert!(network.storage(bc).get() < 5.0, "the fixture needs bc's storage to be small");

        let vehicles: Vec<Vehicle> = (0..200)
            .map(|i| Vehicle::new(VehicleId::new(i), vec![ab, bc, cd], Pcu(1.0), Second(0)))
            .collect();
        let ab_discharge_after = |level: FidelityLevel| -> f64 {
            let mut sim = LtmNetwork::new(&network, &turns).with_level(level);
            for v in &vehicles {
                sim.depart(v);
            }
            for _ in 0..2 {
                let _ = sim.step(Duration(60.0));
            }
            sim.cumulative_out(ab).get()
        };
        let full = ab_discharge_after(FidelityLevel::Full);
        let spatial = ab_discharge_after(FidelityLevel::SpatialQueue);
        let point = ab_discharge_after(FidelityLevel::PointQueue);
        assert!(
            point > full + 1e-6,
            "infinite storage must discharge strictly more: {full} vs {point}"
        );
        assert!(
            spatial >= full - 1e-6,
            "infinite wave speed cannot discharge less: {full} vs {spatial}"
        );
        assert!(
            point >= spatial - 1e-6,
            "infinite storage bounds infinite wave speed: {spatial} vs {point}"
        );
    }

    #[test]
    fn a_step_on_an_empty_network_only_moves_the_clock() {
        let network = RoadNetworkBuilder::new().build(
            GlobalMultipliers::default(),
            SignalDefaults::SHIPPED,
            &mut Diagnostics::new(),
        );
        if let Ok(network) = network {
            let turns = TurnTable::build(&network, SignalDefaults::SHIPPED);
            let mut sim = LtmNetwork::new(&network, &turns);
            assert!(sim.step(Duration(300.0)).is_empty());
            assert_eq!(sim.now(), Duration(300.0));
        }
    }
}
