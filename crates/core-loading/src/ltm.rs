//! The loading at levels 2–4: the link transmission model with vehicles on
//! the curves (S84, S85), in S147's hybrid form (S151), with vehicles that
//! straddle links, stop lines and origin queues (S153).
//!
//! # What it computes
//!
//! Every vehicle carries its route (S85) and its own clock. Every link has a
//! triangular fundamental diagram (S113) that limits four things:
//!
//! 1. **Free-flow travel** — a vehicle's front reaches the downstream end of a
//!    link no earlier than it entered plus the link's free-flow travel time.
//! 2. **Discharge capacity** — vehicle fronts pass the downstream end no closer
//!    together than `PCU / (capacity × g/C)` seconds, the saturation headway of
//!    the approach (S90's green-time fraction; 1 at an unsignalised node).
//! 3. **Signal delay at the stop line** — on a signalised approach, a vehicle
//!    passes the stop line when the next link has room for it, leaves the
//!    approach, and waits S90's control delay at the stop line before
//!    entering the next link, checking its room again. The delay belongs to
//!    the approach in the vehicle's trajectory but holds no storage — neither
//!    the approach's nor the next link's — so a short signalised approach, and
//!    a short link after it, pass `capacity × g/C`. A full next link stops
//!    vehicles reaching the stop line, so the approach fills and spills back.
//! 4. **Receiving capacity** — a vehicle's front enters a link only if the link
//!    has room (below), and no closer behind the previous entrant than
//!    `PCU / capacity`. Room is the LTM receiving condition: storage, plus what
//!    had left one backward-wave travel time earlier, minus what has entered
//!    ([`crate::curves::room_at`]).
//!
//! # Vehicles straddle links: no length threshold
//!
//! A link's storage is jam density × length × lanes, in PCU; nothing compares a
//! vehicle's length with a link's. A vehicle's front enters a link when the
//! link has any room, and **takes only the room there is: the rest of the
//! vehicle stays counted on the link behind it** (or outside the network, at
//! an origin or a stop line), as it physically does. As room appears ahead,
//! the vehicle moves onto it and releases the same amount from its rear. So a
//! car crossing a 3-m junction piece is counted on three pieces at once, a bus
//! on a 12-m piece hangs back onto the approach, and a 300-m street holds
//! forty cars — one rule, whatever the length, and **no link ever counts more
//! than its storage**.
//!
//! While a vehicle straddles a link's upstream end, nothing else enters that
//! link, and the vehicle behind it on the link it is leaving cannot move: it is
//! physically in the way (S77's full blocking).
//!
//! # Roundabouts: entering traffic gives way
//!
//! Where a link is part of a roundabout's circulating carriageway (OSM
//! `junction=roundabout`), a vehicle entering it from outside the roundabout —
//! from an approach, an origin or a stop line — waits while the circulating
//! link before it holds any vehicle bound for the same link (S155): the
//! priority merge, where circulating traffic that wants the room gets it
//! first, so entries use only room circulating traffic does not need. Without
//! this, entries compete equally for room and fill the ring until it locks
//! (S153). It costs one flag per link and, per entry into a roundabout link, a
//! look at the few vehicles on the circulating link before it.
//!
//! # When traffic stops
//!
//! A vehicle waits for exactly two things: room on the link ahead, or the rear
//! of the vehicle in front clearing a link end — which in turn waits for room
//! ahead of that vehicle. Any room is used as soon as it is heard, by the one
//! vehicle entitled to it. So a set of vehicles waiting on one another forever
//! must be waiting on links that are all full (to within [`MIN_PART`]; an
//! entry giving way at a roundabout also waits on a circulating vehicle, which
//! itself waits for room): **traffic stops only at jam density**, where the fundamental diagram's flow is zero (S152, S153). No
//! vehicle is removed or forced. [`LtmNetwork::waiting_cycles`] lists such
//! stops.
//!
//! # Origins
//!
//! A departing vehicle waits outside the network, in its first link's origin
//! queue, until the link has room. It does not use the link's inflow capacity.
//! The wait is part of its travel time: a [`Trajectory`] keeps the scheduled
//! departure.
//!
//! # How: in time order
//!
//! Vehicle movements are processed **in time order** from one event queue. An
//! event says "the vehicle at the front of queue `q` is due at time `t`", where
//! `q` is a link, an origin queue or a stop line — or "room released on link
//! `l` reaches its upstream end now", which lets the vehicle straddling that
//! end move further onto it and wakes the queues waiting for the link.
//! Processing an event moves a vehicle on, reschedules it to the moment the
//! next link will have room or inflow capacity, or parks its queue until the
//! link it needs has room to hear of or an upstream end clears.
//! Because every constraint is checked at the moment it applies, with every
//! earlier movement committed, the result is exact in continuous time: FIFO on
//! every queue, spillback resolved to the second.
//!
//! Events are ordered by `(time, service tag, queue)` — a total order, so the
//! result never depends on input order (S77's requirement, met by ordering;
//! S88's event-queue convention). The **service tag** is weighted fair
//! queueing's virtual finish time: an approach's tag advances by one saturation
//! headway per vehicle it releases, and a queue that has just become
//! backlogged starts from the link's **virtual time** — the largest tag released
//! onto the link it is bound for. Approaches that are never held back are
//! served first come, first served (by `time`), and saturated approaches
//! competing for the same room are served in proportion to their discharge
//! capacities (S48), whatever the loading step. Tags are in their own units and
//! are never compared with real seconds: under saturation the virtual clock
//! runs slower than the real one, and mixing them let the queue that had waited
//! longest win merges regardless of capacity (S161, K31). A vehicle that cannot
//! move blocks everything behind it in its queue (S77's full blocking).
//!
//! The loading step is an output and bookkeeping boundary (S84): nothing about
//! a vehicle's timing depends on it. At each step's end every link forgets
//! exits older than one backward-wave travel time
//! ([`crate::curves::LinkCurves::forget_before`]).
//!
//! # Recorded biases
//!
//! - **A straddling vehicle can stretch.** It moves onto room it has heard of,
//!   so its part on a link it spans end to end can be shorter than the link
//!   while the rest of that link's room has not yet reached its upstream end;
//!   it closes up as the room arrives. Its PCU is always counted in full.
//! - **The smallest part a vehicle's front takes** is [`MIN_PART`] PCU, or the
//!   whole of a link's storage if that is less.
//! - **Give way is keyed to the OSM tag only** (S154, S155): a ring mapped
//!   without `junction=roundabout` does not give way, and can still lock at jam
//!   density — [`LtmNetwork::waiting_cycles`] reports it. Priority is by
//!   presence on the circulating link before the entry, not by a critical gap
//!   in time: a long circulating link holds entries back further ahead of an
//!   arriving vehicle than a driver would (roundabout pieces are short).
//! - **Signal delay is a cycle average** at the stop line (S90): queues within a
//!   cycle are not represented. Vehicles serving their delay are counted on no
//!   link, so while a signalised approach is held back by a full next link it
//!   stores, beyond its own storage, the vehicles that passed its stop line in
//!   the last control delay (at most `capacity × g/C × delay`).

use std::cmp::{Ordering, Reverse};
use std::collections::{BinaryHeap, VecDeque};

use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_types::ids::{EntityId, LinkId};
use openmobisim_core_types::time::Second;
use openmobisim_core_types::units::{Duration, Pcu};

use crate::curves::{LinkCurves, ROOM_EPSILON, room_at};
use crate::level0::{LinkTraversal, Trajectory};
use crate::link_bins::{LinkBinRecorder, LinkBins};
use crate::vehicle::Vehicle;

/// The smallest part of a vehicle its front moves onto a link (PCU): room
/// below this is left until more appears, unless the link's whole storage is
/// smaller. About 7 cm of a car; it keeps float-sized slivers from moving.
pub const MIN_PART: f64 = 0.01;

/// Two times closer than this are the same instant.
const TIME_EPSILON: f64 = 1e-9;

/// No queue, link or vehicle: the end of an intrusive list, or "outside the
/// network" in a vehicle's parts.
const NONE: u32 = u32::MAX;

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

impl FidelityLevel {
    /// The level's number in the fidelity ladder (design §10.1): 2, 3 or 4.
    #[must_use]
    pub const fn number(self) -> u32 {
        match self {
            Self::PointQueue => 2,
            Self::SpatialQueue => 3,
            Self::Full => 4,
        }
    }
}

/// One vehicle in a queue: its front on a link, or the vehicle at an origin
/// or at a stop line.
///
/// Stored once per vehicle in flight, so its size is asserted.
#[derive(Clone, Copy, Debug)]
struct Queued {
    /// Index into [`LtmNetwork::vehicles`].
    slot: u32,
    /// Position in the vehicle's route of the link this queue belongs to.
    leg: u32,
    /// When the vehicle's front entered the link (for origins: its departure).
    enter: f64,
    /// The earliest time it can be served at the front of this queue.
    ready: f64,
}

/// "The vehicle at the front of queue `queue` is due at `time`."
///
/// Superseded events are skipped rather than removed: each schedule bumps the
/// queue's generation.
#[derive(Clone, Copy, Debug)]
struct Event {
    time: f64,
    tag: f64,
    queue: u32,
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
            .then(self.queue.cmp(&other.queue))
            .then(self.generation.cmp(&other.generation))
    }
}

/// What a vehicle waits for when it cannot move.
#[derive(Clone, Copy, Debug)]
struct Blocked {
    /// The link whose release of room (or of its upstream end) it needs.
    link: usize,
    /// A committed exit whose room reaches the vehicle later, if one exists.
    retry_at: Option<f64>,
}

/// Where a queue or event index points.
#[derive(Clone, Copy, Debug)]
enum Place {
    Link(usize),
    Origin,
    StopLine(usize),
    /// Room released on this link reaches its upstream end.
    Heard(usize),
}

/// The running state of a loading.
pub struct LtmNetwork<'a> {
    network: &'a RoadNetwork,
    level: FidelityLevel,
    now: f64,
    /// The time of the movement being processed: nothing is scheduled before it.
    clock: f64,
    links: usize,

    // Per link, fixed for the run.
    travel: Vec<f64>,
    stop_delay: Vec<f64>,
    discharge_rate: Vec<f64>,
    inflow_rate: Vec<f64>,
    storage: Vec<f64>,
    wave_lag: Vec<f64>,

    // Per link, running.
    curves: Vec<LinkCurves>,
    /// Whole PCU of the vehicles whose fronts have passed each link's
    /// downstream end (its stop line, on a signalised approach).
    discharged: Vec<f64>,
    first_parked: Vec<u32>,
    /// The vehicle straddling each link's upstream end, or `NONE`.
    straddling_in: Vec<u32>,
    /// The vehicle straddling each link's downstream end, or `NONE`.
    straddling_out: Vec<u32>,
    /// When a room-heard event is pending for each link, or `+∞`.
    heard_due: Vec<f64>,
    /// The largest service tag released onto each link: the virtual time a
    /// queue that has just become backlogged for it starts from (S161, K31).
    virtual_time: Vec<f64>,

    // Per queue: links `0..n`, origins `n..2n`, stop lines `2n..3n`. Room-heard
    // events use `3n..4n`.
    queues: Vec<VecDeque<Queued>>,
    /// Each link queue's service tag: the tag of the vehicle it released last.
    service_tag: Vec<f64>,
    generation: Vec<u32>,
    next_parked: Vec<u32>,
    parked_on: Vec<u32>,

    // Per vehicle.
    vehicles: Vec<&'a Vehicle>,
    traversals: Vec<Vec<LinkTraversal>>,
    /// Where a vehicle's PCU is counted: `(link, PCU)` from its front to its
    /// rear, `NONE` for a part still outside the network. Empty when it is
    /// counted on no link (waiting at an origin or a stop line, or done).
    parts: Vec<VecDeque<(u32, f64)>>,
    /// Slots not yet departed, latest departure first (so `pop` is next).
    pending: Vec<u32>,
    pending_sorted: bool,

    events: BinaryHeap<Reverse<Event>>,

    /// Per-link, per-bin results, when asked for (S163).
    recorder: Option<LinkBinRecorder>,
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
        let mut travel = Vec::with_capacity(n);
        let mut stop_delay = Vec::with_capacity(n);
        let mut discharge_rate = Vec::with_capacity(n);
        let mut inflow_rate = Vec::with_capacity(n);
        for idx in 0..n {
            let link = LinkId::from_index(idx);
            let params = network.link_parameters(link);
            let capacity = params.capacity.get().max(f64::MIN_POSITIVE);
            let green = turns
                .turns_from(link)
                .iter()
                .map(|&t| f64::from(turns.capacity_fraction(t)))
                .reduce(f64::max)
                .unwrap_or(1.0)
                .max(f64::MIN_POSITIVE);
            let delay = params.control_delay.get().max(0.0);
            let free_flow = network.free_flow_time(link).get();
            travel.push((free_flow - delay).max(0.0));
            stop_delay.push(delay);
            discharge_rate.push(capacity * green);
            inflow_rate.push(capacity);
        }
        let queues = 3 * n;
        let mut sim = Self {
            network,
            level: FidelityLevel::Full,
            now: 0.0,
            clock: 0.0,
            links: n,
            travel,
            stop_delay,
            discharge_rate,
            inflow_rate,
            storage: Vec::new(),
            wave_lag: Vec::new(),
            curves: (0..n).map(|_| LinkCurves::new()).collect(),
            discharged: vec![0.0; n],
            first_parked: vec![NONE; n],
            straddling_in: vec![NONE; n],
            straddling_out: vec![NONE; n],
            heard_due: vec![f64::INFINITY; n],
            virtual_time: vec![0.0; n],
            queues: (0..queues).map(|_| VecDeque::new()).collect(),
            service_tag: vec![f64::NEG_INFINITY; queues],
            generation: vec![0; queues + n],
            next_parked: vec![NONE; queues],
            parked_on: vec![NONE; queues],
            vehicles: Vec::new(),
            traversals: Vec::new(),
            parts: Vec::new(),
            pending: Vec::new(),
            pending_sorted: true,
            events: BinaryHeap::new(),
            recorder: None,
        };
        sim.apply_level();
        sim
    }

    /// The same network, also recording per-link, per-time-bin results (S163):
    /// bins of `bin_seconds`, ignoring traversals that finish at or after
    /// `window` seconds. Collect them with [`Self::take_link_bins`].
    ///
    /// Costs 20 bytes per link of scratch, a 28-byte row per (link, bin) that
    /// saw traffic, and about three additions per link crossing.
    ///
    /// # Panics
    ///
    /// Panics if `bin_seconds` is zero.
    #[must_use]
    pub fn with_link_bins(mut self, bin_seconds: u32, window: f64) -> Self {
        self.recorder = Some(LinkBinRecorder::new(self.links, bin_seconds, window));
        self
    }

    /// The per-link, per-bin results recorded so far, if
    /// [`Self::with_link_bins`] asked for them. Recording stops.
    #[must_use]
    pub fn take_link_bins(&mut self) -> Option<LinkBins> {
        self.recorder.take().map(LinkBinRecorder::finish)
    }

    /// The same network, run at `level` (S76's nesting: nothing else changes).
    #[must_use]
    pub fn with_level(mut self, level: FidelityLevel) -> Self {
        self.level = level;
        self.apply_level();
        self
    }

    fn apply_level(&mut self) {
        self.storage.clear();
        self.wave_lag.clear();
        for idx in 0..self.links {
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

    /// The whole PCU of every vehicle whose front has passed a link's
    /// downstream end (its stop line, on a signalised approach).
    #[must_use]
    pub fn discharged_pcu(&self, link: LinkId) -> Pcu {
        Pcu(self.discharged[link.index()])
    }

    /// How many vehicles have their front on a link (not at its stop line).
    #[must_use]
    pub fn queue_len(&self, link: LinkId) -> usize {
        self.queues[link.index()].len()
    }

    /// The PCU of every vehicle part on a link, found by walking the vehicles
    /// themselves — for tests: it always equals `cumulative_in −
    /// cumulative_out`. Takes time proportional to the vehicles scheduled.
    #[must_use]
    pub fn queued_pcu(&self, link: LinkId) -> Pcu {
        let l = link_u32(link.index());
        Pcu(self.parts.iter().flatten().filter(|p| p.0 == l).map(|p| p.1).sum())
    }

    /// Everything counted against a link's storage now,
    /// `cumulative_in − cumulative_out`. Never more than its storage.
    #[must_use]
    pub fn counted_pcu(&self, link: LinkId) -> Pcu {
        self.cumulative_in(link) - self.cumulative_out(link)
    }

    /// How many vehicles wait outside the network to depart onto a link.
    #[must_use]
    pub fn waiting_at_origin(&self, link: LinkId) -> usize {
        self.queues[self.links + link.index()].len()
    }

    /// How many vehicles wait at a link's stop line, serving its signal delay
    /// or waiting for room on the next link.
    #[must_use]
    pub fn waiting_at_stop_line(&self, link: LinkId) -> usize {
        self.queues[2 * self.links + link.index()].len()
    }

    /// The link a link's front vehicle is waiting on, if it is parked.
    #[must_use]
    pub fn parked_on(&self, link: LinkId) -> Option<LinkId> {
        let p = self.parked_on[link.index()];
        (p != NONE).then(|| LinkId::from_index(p as usize))
    }

    /// How many exits a link retains for its receiving-condition reads.
    #[must_use]
    pub fn retained_history(&self, link: LinkId) -> usize {
        self.curves[link.index()].retained()
    }

    /// Closed loops of links whose front vehicles wait on one another now,
    /// each as its links in waiting order. A link's front waits on the link its
    /// queue is parked on or, while the vehicle ahead still straddles the
    /// link's end, on the link that vehicle's front is on. By the module docs'
    /// argument every link whose room is waited for in such a loop is full, to
    /// within [`MIN_PART`]: this is where traffic has reached jam density.
    #[must_use]
    pub fn waiting_cycles(&self) -> Vec<Vec<LinkId>> {
        const UNSEEN: u8 = 0;
        const ON_PATH: u8 = 1;
        const DONE: u8 = 2;
        let waits_on = |i: usize| -> Option<usize> {
            let ahead = self.straddling_out[i];
            if ahead != NONE {
                return self.parts[ahead as usize].front().map(|p| p.0 as usize);
            }
            (self.parked_on[i] != NONE).then(|| self.parked_on[i] as usize)
        };
        let mut state = vec![UNSEEN; self.links];
        let mut cycles = Vec::new();
        let mut path = Vec::new();
        for first in 0..self.links {
            path.clear();
            let mut at = first;
            let mut closed = false;
            while state[at] == UNSEEN {
                state[at] = ON_PATH;
                path.push(at);
                match waits_on(at) {
                    None => break,
                    Some(next) => {
                        closed = state[next] == ON_PATH;
                        at = next;
                    }
                }
            }
            if closed {
                if let Some(from) = path.iter().position(|&p| p == at) {
                    cycles.push(path[from..].iter().map(|&p| LinkId::from_index(p)).collect());
                }
            }
            for &p in &path {
                state[p] = DONE;
            }
        }
        cycles
    }

    /// Schedule a vehicle. It departs at its own departure second, waiting at
    /// its first link's origin until the link can take it; a departure earlier
    /// than [`Self::now`] departs at the start of the next step.
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
        self.parts.push(VecDeque::new());
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
        self.clock = self.clock.max(t0);

        loop {
            let next_event = self.events.peek().map(|Reverse(e)| e.time);
            let next_departure = self.next_departure_time(t0);
            match (next_event, next_departure) {
                (_, Some(d)) if d < t1 && next_event.is_none_or(|e| d <= e) => {
                    self.clock = self.clock.max(d);
                    self.depart_next(t0);
                }
                (Some(e), _) if e < t1 => {
                    let Reverse(event) = self.events.pop().expect("just peeked");
                    self.clock = self.clock.max(event.time);
                    self.process(event, t0, &mut completed);
                }
                _ => break,
            }
        }

        for idx in 0..self.links {
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

    /// The next pending vehicle joins its first link's origin queue.
    fn depart_next(&mut self, t0: f64) {
        let slot = self.pending.pop().expect("a departure is pending");
        let vehicle = self.vehicles[slot as usize];
        let at = f64::from(vehicle.departure.get()).max(t0);
        let origin = self.links + vehicle.route[0].index();
        self.join(origin, Queued { slot, leg: 0, enter: at, ready: at }, t0);
    }

    fn place(&self, queue: usize) -> Place {
        let n = self.links;
        if queue < n {
            Place::Link(queue)
        } else if queue < 2 * n {
            Place::Origin
        } else if queue < 3 * n {
            Place::StopLine(queue - 2 * n)
        } else {
            Place::Heard(queue - 3 * n)
        }
    }

    /// Put a vehicle at the back of a queue; if it is the front, schedule it.
    fn join(&mut self, queue: usize, queued: Queued, t0: f64) {
        self.queues[queue].push_back(queued);
        if self.queues[queue].len() == 1 {
            self.schedule_front(queue, t0, f64::NEG_INFINITY);
        }
    }

    /// The front vehicle of a queue: `(when it is ready to move, its tag)`.
    fn front_due(&self, queue: usize, t0: f64) -> Option<(f64, f64)> {
        let front = self.queues[queue].front()?;
        match self.place(queue) {
            Place::Link(i) => {
                let vehicle = self.vehicles[front.slot as usize];
                let headway = vehicle.pcu.get() / self.discharge_rate[i];
                // When it can move: it has reached the end, its approach's
                // headway has passed, and the step has begun. Real seconds.
                let ready = front.ready.max(self.curves[i].last_exit().get() + headway).max(t0);
                // Who goes first among approaches ready for the same room: a
                // virtual time, in its own units, never mixed with `ready`.
                let onto = vehicle.route.get(front.leg as usize + 1);
                let virtual_now = onto.map_or(0.0, |l| self.virtual_time[l.index()]);
                let tag = virtual_now.max(self.service_tag[queue] + headway);
                Some((ready, tag))
            }
            Place::Origin | Place::StopLine(_) | Place::Heard(_) => {
                // The tag is the real arrival time, *not* clamped to the step's
                // start: clamping gave every queue that had waited across a step
                // boundary the same tag, so ties fell to queue id instead of
                // arrival order and results depended on the step (S163).
                Some((front.ready.max(t0), front.ready))
            }
        }
    }

    /// Schedule a queue's front vehicle, no earlier than `not_before`.
    fn schedule_front(&mut self, queue: usize, t0: f64, not_before: f64) {
        if let Some((ready, tag)) = self.front_due(queue, t0) {
            self.generation[queue] = self.generation[queue].wrapping_add(1);
            self.events.push(Reverse(Event {
                time: ready.max(not_before).max(self.clock),
                tag,
                queue: queue_u32(queue),
                generation: self.generation[queue],
            }));
        }
    }

    /// Schedule room released on link `l` to be heard at its upstream end at
    /// `at`, unless an earlier hearing is pending: whoever it serves then finds
    /// any later release from the link's retained exits.
    fn schedule_heard(&mut self, l: usize, at: f64) {
        let at = at.max(self.clock);
        if at >= self.heard_due[l] {
            return;
        }
        self.heard_due[l] = at;
        let queue = 3 * self.links + l;
        self.generation[queue] = self.generation[queue].wrapping_add(1);
        self.events.push(Reverse(Event {
            time: at,
            tag: at,
            queue: queue_u32(queue),
            generation: self.generation[queue],
        }));
    }

    fn process(&mut self, event: Event, t0: f64, completed: &mut Vec<Trajectory>) {
        let queue = event.queue as usize;
        if event.generation != self.generation[queue] {
            return;
        }
        if let Place::Heard(l) = self.place(queue) {
            self.heard_due[l] = f64::INFINITY;
            // The straddling vehicle is entitled to the room first.
            self.close_straddle(l, event.time, t0);
            return self.wake(l, event.time, t0);
        }
        let Some((ready, tag)) = self.front_due(queue, t0) else {
            return;
        };
        let t = ready.max(event.time);
        if t > event.time + TIME_EPSILON {
            // Not due yet after all: keep time order.
            return self.schedule_front(queue, t0, t);
        }
        let front = *self.queues[queue].front().expect("front_due found one");
        let vehicle = self.vehicles[front.slot as usize];
        let pcu = vehicle.pcu.get();
        let next_leg = front.leg as usize + 1;
        let next = vehicle.route.get(next_leg).map(|l| l.index());

        match self.place(queue) {
            Place::Link(i) => {
                if self.straddling_out[i] != NONE {
                    // The vehicle ahead is still in the way; its rear clearing
                    // the link end reschedules this queue.
                    return;
                }
                let room = match next {
                    None => f64::INFINITY,
                    Some(j) => {
                        // On a signalised approach the inflow headway applies
                        // when the vehicle leaves the stop line.
                        if self.stop_delay[i] <= 0.0 && self.inflow_blocks(j, pcu, queue, t, t0) {
                            return;
                        }
                        match self.admissible(j, pcu, t, Some(i)) {
                            Ok(room) => room,
                            Err(blocked) => return self.block(queue, blocked, t, t0),
                        }
                    }
                };
                self.advance(i, tag, t, room, t0, completed);
            }
            Place::Origin => {
                let first = vehicle.route[0].index();
                match self.admissible(first, pcu, t, None) {
                    Ok(room) => {
                        self.pop_front(queue, t0);
                        self.enter_from_outside(front.slot, 0, t, room, false, t0);
                    }
                    Err(blocked) => self.block(queue, blocked, t, t0),
                }
            }
            Place::StopLine(i) => {
                let mut room = f64::INFINITY;
                if let Some(j) = next {
                    if self.inflow_blocks(j, pcu, queue, t, t0) {
                        return;
                    }
                    match self.admissible(j, pcu, t, Some(i)) {
                        Ok(r) => room = r,
                        Err(blocked) => return self.block(queue, blocked, t, t0),
                    }
                }
                self.pop_front(queue, t0);
                self.record_link_bins(i, front.slot, front.enter, t);
                self.traversals[front.slot as usize].push(traversal(i, front.enter, t));
                if next.is_some() {
                    self.enter_from_outside(front.slot, next_leg, t, room, true, t0);
                } else {
                    self.complete(front.slot, t, t0, completed);
                }
            }
            Place::Heard(_) => unreachable!("room-heard events are handled first"),
        }
    }

    /// Whether link `j` can take a vehicle's front at `t`, and the room it has:
    /// nothing straddles its upstream end, no circulating vehicle it must give
    /// way to is waiting for it, and its room is at least a part —
    /// [`MIN_PART`], the vehicle, or the link's whole storage, whichever is
    /// least. `from` is the link the vehicle comes from (`None` at an origin).
    /// If not, what to wait for.
    fn admissible(&self, j: usize, pcu: f64, t: f64, from: Option<usize>) -> Result<f64, Blocked> {
        if self.straddling_in[j] != NONE {
            return Err(Blocked { link: j, retry_at: None });
        }
        if let Some(r) = self.must_give_way(j, from) {
            return Err(Blocked { link: r, retry_at: None });
        }
        let storage = self.storage[j];
        if storage.is_infinite() {
            return Ok(f64::INFINITY);
        }
        let (room, retry_at) = self.heard_room(j, t);
        if room + ROOM_EPSILON >= pcu.min(storage).min(MIN_PART) {
            return Ok(room.max(0.0));
        }
        Err(Blocked { link: j, retry_at })
    }

    /// A vehicle entering roundabout link `j` from `from` (not itself on the
    /// roundabout) gives way while the circulating link before `j` holds any
    /// vehicle bound for `j` — the priority merge: circulating traffic that
    /// wants the room gets it first. Returns that link; its next departure
    /// wakes the entry.
    fn must_give_way(&self, j: usize, from: Option<usize>) -> Option<usize> {
        let ring = |l: usize| self.network.is_roundabout(LinkId::from_index(l));
        if !ring(j) || from.is_some_and(ring) {
            return None;
        }
        let node = self.network.link_from(LinkId::from_index(j));
        self.network.in_links(node).iter().map(|l| l.index()).find(|&r| {
            Some(r) != from
                && ring(r)
                && self.queues[r].iter().any(|q| {
                    self.vehicles[q.slot as usize]
                        .route
                        .get(q.leg as usize + 1)
                        .is_some_and(|l| l.index() == j)
                })
        })
    }

    /// Link `j`'s room as heard at its upstream end at `t`, and when room
    /// already released downstream will next be heard there.
    fn heard_room(&self, j: usize, t: f64) -> (f64, Option<f64>) {
        // Room freed by an exit reaches the upstream end exactly one wave
        // travel time later; the epsilon keeps that instant inclusive.
        let heard_until = t + TIME_EPSILON;
        let lag = self.wave_lag[j];
        let room =
            room_at(&self.curves[j], Pcu(self.storage[j]), Duration(lag), Duration(heard_until))
                .get();
        let next =
            self.curves[j].first_exit_after(Duration(heard_until - lag)).map(|e| e.get() + lag);
        (room, next)
    }

    /// Whether the next link's inflow headway holds the vehicle back; if so,
    /// the queue is rescheduled for when it will not.
    fn inflow_blocks(&mut self, j: usize, pcu: f64, queue: usize, t: f64, t0: f64) -> bool {
        let clear = self.curves[j].last_entry().get() + pcu / self.inflow_rate[j];
        if clear > t + TIME_EPSILON {
            self.schedule_front(queue, t0, clear);
            true
        } else {
            false
        }
    }

    /// The front vehicle of link `i` moves on at `t`: across the stop line on a
    /// signalised approach, onto the next link where `room` is heard, or out
    /// of the network. Everything that could hold it back has been checked.
    fn advance(
        &mut self,
        i: usize,
        tag: f64,
        t: f64,
        room: f64,
        t0: f64,
        completed: &mut Vec<Trajectory>,
    ) {
        let front = self.queues[i].pop_front().expect("advancing a front vehicle");
        let slot = front.slot;
        let vehicle = self.vehicles[slot as usize];
        if self.first_parked[i] != NONE && self.network.is_roundabout(LinkId::from_index(i)) {
            // Entries giving way to this link's front may go now.
            self.wake(i, t, t0);
        }
        self.curves[i].set_last_exit(Duration(t));
        self.discharged[i] += vehicle.pcu.get();
        self.service_tag[i] = tag;
        let next_leg = front.leg as usize + 1;
        if let Some(onto) = vehicle.route.get(next_leg) {
            let v = &mut self.virtual_time[onto.index()];
            *v = v.max(tag);
        }
        self.schedule_front(i, t0, t);
        if self.stop_delay[i] > 0.0 {
            self.release_all(slot, t, t0);
            let waiting = Queued { ready: t + self.stop_delay[i], ..front };
            self.join(2 * self.links + i, waiting, t0);
        } else {
            self.record_link_bins(i, slot, front.enter, t);
            self.traversals[slot as usize].push(traversal(i, front.enter, t));
            if next_leg < vehicle.route.len() {
                self.enter_from_link(slot, next_leg, t, room, t0);
            } else {
                self.complete(slot, t, t0, completed);
            }
        }
    }

    /// How much of a vehicle of `pcu` moves onto a link with `room`.
    fn part_taken(room: f64, pcu: f64) -> f64 {
        if room + ROOM_EPSILON >= pcu { pcu } else { room.max(0.0) }
    }

    /// A vehicle's front moves from the link it is on onto the link at `leg`
    /// of its route, taking the `room` there; its rear releases as much.
    fn enter_from_link(&mut self, slot: u32, leg: usize, t: f64, room: f64, t0: f64) {
        let s = slot as usize;
        let vehicle = self.vehicles[s];
        let j = vehicle.route[leg].index();
        let take = Self::part_taken(room, vehicle.pcu.get());
        let from = self.parts[s].front().expect("a vehicle on a link has parts").0 as usize;
        self.curves[j].record_in(Pcu(take));
        self.curves[j].set_last_entry(Duration(t));
        self.parts[s].push_front((link_u32(j), take));
        // It straddles the link end it crosses until its rear has cleared it.
        self.straddling_out[from] = slot;
        self.straddling_in[j] = slot;
        self.release_rear(s, 0, take, t, t0);
        let queued = Queued { slot, leg: leg_u32(leg), enter: t, ready: t + self.travel[j] };
        self.join(j, queued, t0);
        if self.straddling_in[j] == slot {
            self.expect_heard(j, t);
        }
    }

    /// A vehicle waiting outside the network — at an origin, or at a stop line
    /// — moves onto the link at `leg` of its route, taking the `room` there;
    /// the rest of it stays outside until more room is heard.
    fn enter_from_outside(
        &mut self,
        slot: u32,
        leg: usize,
        t: f64,
        room: f64,
        uses_inflow: bool,
        t0: f64,
    ) {
        let s = slot as usize;
        let vehicle = self.vehicles[s];
        let pcu = vehicle.pcu.get();
        let j = vehicle.route[leg].index();
        let take = Self::part_taken(room, pcu);
        debug_assert!(self.parts[s].is_empty(), "a vehicle outside the network has no parts");
        self.curves[j].record_in(Pcu(take));
        if uses_inflow {
            self.curves[j].set_last_entry(Duration(t));
        }
        self.parts[s].push_back((link_u32(j), take));
        if take < pcu {
            self.parts[s].push_back((NONE, pcu - take));
            self.straddling_in[j] = slot;
        }
        let queued = Queued { slot, leg: leg_u32(leg), enter: t, ready: t + self.travel[j] };
        self.join(j, queued, t0);
        if self.straddling_in[j] == slot {
            self.expect_heard(j, t);
        }
    }

    /// A vehicle has just straddled link `j`'s upstream end, taking the room
    /// heard there: it closes up when more room is heard.
    fn expect_heard(&mut self, j: usize, t: f64) {
        if let (_, Some(at)) = self.heard_room(j, t) {
            self.schedule_heard(j, at);
        }
    }

    /// The vehicle straddling link `l`'s upstream end moves further onto `l`, as
    /// far as the room heard there allows, releasing its rear.
    fn close_straddle(&mut self, l: usize, t: f64, t0: f64) {
        let slot = self.straddling_in[l];
        if slot == NONE {
            return;
        }
        let s = slot as usize;
        let parts = &self.parts[s];
        let Some(k) =
            parts.iter().take(parts.len().saturating_sub(1)).rposition(|p| p.0 as usize == l)
        else {
            debug_assert!(false, "a straddling vehicle has a part on the link and one behind");
            return;
        };
        let behind: f64 = parts.iter().skip(k + 1).map(|p| p.1).sum();
        let (room, next) = self.heard_room(l, t);
        let take = Self::part_taken(room, behind);
        if take + ROOM_EPSILON >= behind.min(MIN_PART).min(self.storage[l]) {
            self.parts[s][k].1 += take;
            self.curves[l].record_in(Pcu(take));
            self.release_rear(s, k, take, t, t0);
        }
        if self.straddling_in[l] == slot {
            if let Some(at) = next {
                if at > t + TIME_EPSILON {
                    self.schedule_heard(l, at);
                }
            }
        }
    }

    /// Release `amount` PCU from the rear of vehicle `s`, never from its first
    /// `keep + 1` parts. A part released whole clears the link end it
    /// straddled.
    fn release_rear(&mut self, s: usize, keep: usize, mut amount: f64, t: f64, t0: f64) {
        let slot = queue_u32(s);
        while self.parts[s].len() > keep + 1 {
            let last = self.parts[s].len() - 1;
            let (link, part) = self.parts[s][last];
            if part > amount + ROOM_EPSILON {
                if amount > 0.0 {
                    if link != NONE {
                        self.release_on(link as usize, amount, t);
                    }
                    self.parts[s][last].1 = part - amount;
                }
                return;
            }
            amount -= part;
            if link != NONE && part > 0.0 {
                self.release_on(link as usize, part, t);
            }
            self.parts[s].pop_back();
            let ahead = self.parts[s].back().expect("at least `keep + 1` parts remain").0 as usize;
            if self.straddling_in[ahead] == slot {
                self.straddling_in[ahead] = NONE;
                self.wake(ahead, t, t0);
            }
            if link != NONE && self.straddling_out[link as usize] == slot {
                self.straddling_out[link as usize] = NONE;
                self.schedule_front(link as usize, t0, t);
            }
        }
    }

    /// Everything of a vehicle leaves the links it is on.
    fn release_all(&mut self, slot: u32, t: f64, t0: f64) {
        let s = slot as usize;
        self.release_rear(s, 0, f64::INFINITY, t, t0);
        if let Some((link, part)) = self.parts[s].pop_front() {
            if link != NONE {
                self.release_on(link as usize, part, t);
            }
        }
    }

    /// `amount` PCU leaves link `l` at `t`: the room is heard upstream one wave
    /// travel time later, by whoever waits for it then.
    fn release_on(&mut self, l: usize, amount: f64, t: f64) {
        self.curves[l].record_out(Duration(t), Pcu(amount));
        if self.straddling_in[l] != NONE || self.first_parked[l] != NONE {
            self.schedule_heard(l, t + self.wave_lag[l]);
        }
    }

    fn pop_front(&mut self, queue: usize, t0: f64) {
        self.queues[queue].pop_front();
        self.schedule_front(queue, t0, f64::NEG_INFINITY);
    }

    /// File a finished link traversal in the per-bin results, if asked for.
    #[inline]
    fn record_link_bins(&mut self, link: usize, slot: u32, enter: f64, exit: f64) {
        if let Some(recorder) = self.recorder.as_mut() {
            let pcu = self.vehicles[slot as usize].pcu.get();
            recorder.record(LinkId::from_index(link), enter, exit, pcu);
        }
    }

    fn complete(&mut self, slot: u32, t: f64, t0: f64, completed: &mut Vec<Trajectory>) {
        self.release_all(slot, t, t0);
        let s = slot as usize;
        let vehicle = self.vehicles[s];
        completed.push(Trajectory {
            vehicle: vehicle.id,
            departure: vehicle.departure,
            links: std::mem::take(&mut self.traversals[s]),
        });
    }

    /// Hold a queue: retry at a known time, or park until `blocked.link`
    /// releases room or its upstream end.
    fn block(&mut self, queue: usize, blocked: Blocked, t: f64, t0: f64) {
        if let Some(at) = blocked.retry_at {
            if at > t + TIME_EPSILON {
                return self.schedule_front(queue, t0, at);
            }
        }
        let l = blocked.link;
        self.next_parked[queue] = self.first_parked[l];
        self.first_parked[l] = queue_u32(queue);
        self.parked_on[queue] = link_u32(l);
    }

    /// Room has been heard on link `l`, or its upstream end has cleared:
    /// reschedule everything parked on it, no earlier than `at`.
    fn wake(&mut self, l: usize, at: f64, t0: f64) {
        let mut parked = std::mem::replace(&mut self.first_parked[l], NONE);
        while parked != NONE {
            let q = parked as usize;
            parked = std::mem::replace(&mut self.next_parked[q], NONE);
            self.parked_on[q] = NONE;
            self.schedule_front(q, t0, at);
        }
    }
}

fn traversal(link: usize, enter: f64, exit: f64) -> LinkTraversal {
    LinkTraversal {
        link: LinkId::from_index(link),
        enter: floor_to_second(enter),
        exit: floor_to_second(exit),
    }
}

fn link_u32(index: usize) -> u32 {
    u32::try_from(index).expect("link ids are u32 (Foundations §1)")
}

fn queue_u32(index: usize) -> u32 {
    u32::try_from(index).expect("three queues per link fit u32")
}

fn leg_u32(index: usize) -> u32 {
    u32::try_from(index).expect("route lengths fit u32")
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
/// Trips still in flight when `window` ends are **not** returned; `core-sim`
/// counts them as truncated (S57). `level` picks which term of the triangular
/// diagram is in force (S76).
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
    run_ltm_inner(network, turns, vehicles, window, step, level, None).0
}

/// [`run_ltm`], also returning the per-link, per-time-bin results (S163):
/// every traversal that finished inside the window, including those of
/// vehicles still on their way when it ends, in bins of `bin_seconds`.
///
/// # Panics
///
/// Panics if `step` or `bin_seconds` is not positive.
#[must_use]
pub fn run_ltm_binned(
    network: &RoadNetwork,
    turns: &TurnTable,
    vehicles: &[Vehicle],
    window: Duration,
    step: Duration,
    level: FidelityLevel,
    bin_seconds: u32,
) -> (Vec<Trajectory>, LinkBins) {
    let (done, bins) =
        run_ltm_inner(network, turns, vehicles, window, step, level, Some(bin_seconds));
    (done, bins.expect("bins were asked for"))
}

fn run_ltm_inner(
    network: &RoadNetwork,
    turns: &TurnTable,
    vehicles: &[Vehicle],
    window: Duration,
    step: Duration,
    level: FidelityLevel,
    bin_seconds: Option<u32>,
) -> (Vec<Trajectory>, Option<LinkBins>) {
    assert!(step.get() > 0.0, "the loading step must be positive, got {step:?}");
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "window and step are finite and non-negative scenario parameters"
    )]
    let n_steps = (window.get() / step.get()).ceil().max(0.0) as usize;

    let mut sim = LtmNetwork::new(network, turns).with_level(level);
    if let Some(bin_seconds) = bin_seconds {
        sim = sim.with_link_bins(bin_seconds, window.get());
    }
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
    (completed, sim.take_link_bins())
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
    fn a_queued_vehicle_is_24_bytes() {
        assert_eq!(std::mem::size_of::<Queued>(), 24);
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
