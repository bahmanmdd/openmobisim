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
//! # Any local lock: don't seal it from outside
//!
//! The roundabout rule above is one case of a general one (I-u idea 1, S157):
//! a vehicle whose own link is not already part of a forming waiting loop may
//! not take the last room on a link when doing so would close that loop back
//! on itself — an outsider does not seal a lock shut, whether or not the ring
//! is tagged `junction=roundabout` (K30) or is a roundabout at all (any
//! junction cluster can close this way).
//! A vehicle already circulating within the forming loop is let through: this
//! is what keeps a loop's own traffic moving for as long as it still can. On
//! its own this is prevention, not a guarantee — a loop can still fill through
//! its own circulation alone; **idea 2 (creep)**, S157's other half, is meant
//! to guarantee a standing lock resolves, but is not built yet: a
//! "give-equals-receive rotation" (every member gives what it moves to the
//! next and receives that much from the one before, so no link's occupancy
//! changes) only manages one round before every member permanently straddles,
//! since occupancy never actually drops. Resolving that needs the
//! straddle-closing and front-crossing paths threaded together iteratively —
//! real design, attempted and set aside rather than shipped half-working.
//!
//! # Priority at merges (S213, when asked for)
//!
//! [`LtmNetwork::with_priority`]: at an unsignalised merge a vehicle gives way while the front
//! vehicle of an approach of higher priority — a higher road class, or the same class and
//! [`PRIORITY_CAPACITY_RATIO`] times the capacity — is bound for the same link and ready, and a
//! departure gives way to every approach: Daganzo's priority merge in vehicle form, the
//! prevention his 1996 analysis of gridlock on loops names. Without it, merges share room by
//! capacity, as described above.
//!
//! # En-route rerouting (S213, when asked for)
//!
//! [`LtmNetwork::with_rerouting`]: a vehicle that has waited at the front of its link, blocked,
//! for a set time is offered a new route by a [`Reroute`] the caller supplies (the loading never
//! routes, design §10.4), with the loading's live estimate of each link's time
//! ([`LiveTimes`]); it may take a few. Its trajectory records the links it actually took (its
//! realised route), and every reroute is recorded ([`RerouteRecord`]). A vehicle that re-routes
//! still waits for room like any other: rerouting changes where it goes, never the physics.
//!
//! # Turn pockets (S217, when asked for)
//!
//! [`LtmNetwork::with_pockets`]: on an approach of two lanes or more, a vehicle at the end of the
//! link may pass vehicles ahead of it that wait for another movement, as long as they fit in
//! their movements' pockets — they wait in their own lanes — and none ahead of it is bound for its
//! own movement: first-in-first-out per movement within the pockets, and for the whole link once a
//! movement's queue spills out of its pocket (Wright et al. 2017's partial first-in-first-out).
//! The pocket is a capacity, not a place; the link's discharge capacity stays shared by its
//! movements. A link of one lane, or with one movement, keeps one first-in-first-out queue.
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
//! # Chained vehicles: a bus from stop to stop
//!
//! A vehicle may be **chained** to another (S199, [`LtmNetwork::depart_after`]):
//! it departs when the one before it arrives, `wait` seconds later and not
//! before `not_before` — a bus's next leg, leaving its stop after the dwell and
//! not before its scheduled departure. Between the two it is outside the
//! network, holding no room, so a bus dwelling at a stop does not block the
//! traffic behind it (a bus bay); it then waits at its next link's origin like
//! any departing vehicle. The release is exact in continuous time, within the
//! step. **Off means absent:** a loading with no chained vehicle carries no
//! chain state at all.
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
//! - **Turn pockets are a capacity per movement, not lanes** (S217): every movement of a link
//!   gets the same pocket, the lanes split evenly among them, whatever the movements' volumes;
//!   a vehicle that passes others is ordered against other approaches by its approach's
//!   service tag, not its own; only the front is offered a new route; and vehicles at a
//!   signalised approach's stop line keep one first-in-first-out queue.

use std::cmp::Ordering;
use std::collections::VecDeque;

use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_types::ids::{EntityId, LinkId, VehicleId};
use openmobisim_core_types::time::Second;
use openmobisim_core_types::units::{Duration, Pcu};

use crate::curves::{LinkCurves, ROOM_EPSILON, room_at};
use crate::events::{Event, EventQueue};
use crate::level0::{LinkTraversal, Trajectory};
use crate::link_bins::{EntryTables, LinkBinRecorder, LinkBins};
use crate::vehicle::Vehicle;

/// The smallest part of a vehicle its front moves onto a link (PCU): room
/// below this is left until more appears, unless the link's whole storage is
/// smaller. About 7 cm of a car; it keeps float-sized slivers from moving.
pub const MIN_PART: f64 = 0.01;

/// Two times closer than this are the same instant.
const TIME_EPSILON: f64 = 1e-9;

/// How many waiting-front links [`LtmNetwork::closes_a_waiting_loop`] walks
/// before giving up. A genuine local lock (a roundabout, a small junction
/// cluster) closes within a handful of links; a chain this long that has not
/// closed is not the kind of local lock idea 1 (S157) targets, and is left
/// alone rather than walked further — a bounded, defendable cost per check,
/// not a claim that no longer chain could ever matter.
const MAX_LOOP_WALK: usize = 64;

/// Under priority ([`LtmNetwork::with_priority`]), how many times another approach's capacity
/// one of the same road class must have to take priority over it. 1.5, uncalibrated: a
/// clearly bigger road, not one a little wider.
pub const PRIORITY_CAPACITY_RATIO: f64 = 1.5;

/// No queue, link or vehicle: the end of an intrusive list, or "outside the
/// network" in a vehicle's parts.
const NONE: u32 = u32::MAX;

/// The movement of a vehicle whose route ends at the end of its link (S217).
const EXIT: u32 = u32::MAX - 1;

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
    /// The front vehicle of this link may have waited long enough to re-route (S213).
    RerouteDue(usize),
}

/// A vehicle that departs when another arrives (S199; see the
/// [module docs](self)): `wait` seconds after the vehicle at index `after` of
/// the same list arrives, and not before `not_before` (seconds).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Chain {
    /// The vehicle, by its index in the list loaded.
    pub vehicle: usize,
    /// The vehicle it follows, by index: earlier in the list.
    pub after: usize,
    /// Seconds after that one arrives.
    pub wait: f64,
    /// The earliest second it may leave.
    pub not_before: f64,
}

/// Per slot: the slot released when it arrives, the release rule of a chained
/// slot, and every slot's departure (a chained one's once released).
#[derive(Debug, Default)]
struct Chains {
    next: Vec<u32>,
    wait: Vec<f64>,
    not_before: Vec<f64>,
    start: Vec<f64>,
}

/// What a loading with chained vehicles records besides trajectories.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Recording {
    /// Nothing more.
    Trajectories,
    /// Per-link results in bins of this many seconds.
    Bins(u32),
    /// Those, and the traversals filed by the bin they entered their link in.
    BinsAndEntry(u32),
}

/// Rules a loading applies beyond the link transmission model itself (S213).
#[derive(Clone, Copy, Debug, Default)]
pub struct Rules {
    /// Priority by road hierarchy at unsignalised merges ([`LtmNetwork::with_priority`]).
    pub priority: bool,
    /// En-route rerouting ([`LtmNetwork::with_rerouting`]), if on: who decides is the
    /// [`Reroute`] given to [`run_ltm_chained`].
    pub reroute: Option<RerouteRule>,
    /// Turn pockets this many metres long ([`LtmNetwork::with_pockets`]); 0: none.
    pub pocket_length_m: f64,
}

/// When a vehicle is offered a new route (S213, [`LtmNetwork::with_rerouting`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RerouteRule {
    /// Seconds a vehicle waits at the front of its link, blocked, before it is offered one;
    /// another offer comes after as long again.
    pub after_s: f64,
    /// The most times one vehicle takes one.
    pub max: u8,
}

/// The loading's estimate of how long a link takes now (S213): what a vehicle that re-routes
/// sees of the traffic around it.
pub trait LiveTimes {
    /// Seconds to cross `link` if entered now: its free-flow time, the time its queue takes to
    /// discharge at capacity, and how long its front vehicle has been blocked, if it is.
    fn live_seconds(&self, link: LinkId) -> f64;
}

/// Why a vehicle is offered a new route (S213, S215).
///
/// Only [`Self::Stuck`] exists yet. The others planned in S215 join here,
/// so records and rerouters already carry the reason: an **alert** to an informed driver about
/// congestion or a disruption ahead (navigation apps), a **disruption** on the route (a closure
/// or a capacity drop), a **new destination** (a taxi dispatched, an electric vehicle sent to
/// charge).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RerouteReason {
    /// Blocked at the front of its link for the rule's time ([`RerouteRule::after_s`]).
    Stuck,
}

impl RerouteReason {
    /// The reason as written in results: `"stuck"`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stuck => "stuck",
        }
    }
}

/// Who decides a new route for a vehicle stuck at the front of its link (S213). The loading
/// never routes (design §10.4); it asks.
pub trait Reroute {
    /// The rest of a new route for `vehicle`, at the end of `current` at second `now` and
    /// planned to go on along `planned` (not empty): links beginning with a legal turn out of
    /// `current` and ending where `planned` ends — or `None` to keep to the plan. `live` is the
    /// loading's own estimate of each link's time now; `reason` says why it is asked, so a
    /// rerouter can treat drivers and triggers differently (an informed share, say).
    fn reroute(
        &mut self,
        vehicle: VehicleId,
        current: LinkId,
        planned: &[LinkId],
        now: f64,
        live: &dyn LiveTimes,
        reason: RerouteReason,
    ) -> Option<Vec<LinkId>>;
}

/// One reroute (S213): who, when, where, and the next link planned and taken instead.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RerouteRecord {
    /// The vehicle.
    pub vehicle: VehicleId,
    /// When, in seconds.
    pub second: f64,
    /// The link at whose end it re-routed.
    pub link: LinkId,
    /// The next link of its route until then.
    pub planned_next: LinkId,
    /// The next link of its new route.
    pub new_next: LinkId,
    /// Why it was offered one.
    pub reason: RerouteReason,
}

/// En-route rerouting's state: per vehicle, nothing until it re-routes.
#[derive(Debug)]
struct Rerouting {
    rule: RerouteRule,
    /// Per slot: which of `routes` replaces its route (`NONE`: its own).
    route_of: Vec<u32>,
    /// Per slot: how many times it re-routed.
    count: Vec<u8>,
    /// Per slot: when it was last offered a route (−∞: never).
    last_offer: Vec<f64>,
    /// The routes of vehicles that re-routed, whole (the links already taken, then the new rest).
    routes: Vec<Box<[LinkId]>>,
    /// Links whose front is due an offer, in the order they became due.
    pending: Vec<u32>,
    records: Vec<RerouteRecord>,
}

/// Turn pockets (S217), when asked for: their sizes, and where a queue waits for a movement
/// other than its front's.
#[derive(Debug)]
struct Pockets {
    /// Per link: what each of its movements' pockets holds, in PCU; 0 without pockets.
    size: Vec<f64>,
    /// Per link, its first movement: one per link leaving its downstream node, then one for any
    /// other; one past the last link's last at the end.
    first: Vec<u32>,
    /// Per movement: its link.
    owner: Vec<u32>,
    /// Per movement: the link its queue waits on for it, or `NONE`, and the next movement
    /// waiting on the same link.
    parked_on: Vec<u32>,
    next_parked: Vec<u32>,
    /// Per link: the first movement waiting on it.
    first_parked: Vec<u32>,
}

/// The vehicles ahead in a queue with turn pockets, by movement — the next link, or [`EXIT`] —
/// and their PCU (S217). A link has few movements; more than eight bar the way.
#[derive(Default)]
struct Ahead {
    n: usize,
    keys: [u32; 8],
    pcu: [f64; 8],
}

impl Ahead {
    fn has(&self, key: u32) -> bool {
        self.keys[..self.n].contains(&key)
    }

    /// Count `pcu` more bound for `key`; false once they no longer fit in a pocket of `pocket`:
    /// they stand in the way of everything behind them.
    fn add(&mut self, key: u32, pcu: f64, pocket: f64) -> bool {
        let k = match self.keys[..self.n].iter().position(|&x| x == key) {
            Some(k) => k,
            None if self.n == self.keys.len() => return false,
            None => {
                self.keys[self.n] = key;
                self.pcu[self.n] = 0.0;
                self.n += 1;
                self.n - 1
            }
        };
        self.pcu[k] += pcu;
        self.pcu[k] <= pocket + ROOM_EPSILON
    }
}

/// What [`run_ltm_chained`] returns.
#[derive(Debug)]
pub struct LtmOutput {
    /// Every vehicle that arrived within the window.
    pub trajectories: Vec<Trajectory>,
    /// The per-link results, if recorded.
    pub link_bins: Option<LinkBins>,
    /// The traversals by entry bin, if recorded.
    pub entry: Option<EntryTables>,
    /// What stood still when the window ended.
    pub lock: LockReport,
    /// Every reroute, in the order they happened (empty without rerouting).
    pub reroutes: Vec<RerouteRecord>,
}

/// What stands still when a loading ends (S213): where the vehicles that did not finish are,
/// and the closed loops of links whose front vehicles wait on one another.
///
/// **The check on the loading's own guarantee** — traffic stops only at jam density — is
/// [`Self::room_waits_with_room`]: a front vehicle waiting for room on its next link while
/// that link has at least [`MIN_PART`] of room heard would be a stop the model cannot explain.
/// It must be 0. (A front can also wait without the link it waits on being full: while it
/// gives way at a roundabout, or behind a vehicle still straddling the link's end; those
/// waits are not counted.)
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LockReport {
    /// Vehicles counted on a link when the loading ended.
    pub on_network: u32,
    /// Vehicles waiting outside the network: at an origin or at a stop line.
    pub outside: u32,
    /// Links whose front vehicle was waiting when the loading ended.
    pub waiting_links: u32,
    /// Closed loops of links whose front vehicles wait on one another, each in waiting order
    /// ([`LtmNetwork::waiting_cycles`]).
    pub loops: Vec<Vec<LinkId>>,
    /// Of the waiting fronts — and with turn pockets, the movements waiting behind them — how
    /// many wait for room on their next link while it has room: 0 unless something is wrong.
    pub room_waits_with_room: u32,
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
    /// How many vehicles straddle each link's downstream end: at most one, unless the link has
    /// turn pockets (S217). Each is the vehicle straddling the upstream end of a link it leads to.
    out_straddlers: Vec<u32>,
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
    /// Chained vehicles (S199), once one is added: absent otherwise.
    chains: Option<Chains>,

    /// Priority by road hierarchy at unsignalised merges (S213), when asked for: per link,
    /// whether a vehicle may be giving way to its front — every motor link into an
    /// unsignalised node, since departures give way to any approach — so its front moving on
    /// wakes them. Empty when priority is off.
    priority_major: Vec<bool>,

    /// At most one event per queue (S210); `4n..5n` are the links' reroute timers (S213).
    events: EventQueue,

    /// En-route rerouting (S213), when asked for.
    rerouting: Option<Rerouting>,

    /// Turn pockets (S217), when asked for.
    pockets: Option<Pockets>,

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
            out_straddlers: vec![0; n],
            heard_due: vec![f64::INFINITY; n],
            virtual_time: vec![0.0; n],
            queues: (0..queues).map(|_| VecDeque::new()).collect(),
            service_tag: vec![f64::NEG_INFINITY; queues],
            next_parked: vec![NONE; queues],
            parked_on: vec![NONE; queues],
            vehicles: Vec::new(),
            traversals: Vec::new(),
            parts: Vec::new(),
            pending: Vec::new(),
            pending_sorted: true,
            chains: None,
            priority_major: Vec::new(),
            events: EventQueue::new(queues + 2 * n),
            rerouting: None,
            pockets: None,
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

    /// The same network, also recording each traversal under the bin it
    /// **entered** its link in (S170; see [`LinkBinRecorder::with_entry_bins`]).
    /// Needs [`Self::with_link_bins`] first; collect with
    /// [`Self::take_link_bins_with_entry`].
    #[must_use]
    pub fn with_entry_bins(mut self) -> Self {
        self.recorder = self.recorder.map(LinkBinRecorder::with_entry_bins);
        self
    }

    /// Tell the recorder about every vehicle still on a link, or waiting at an
    /// origin to enter one, at second `end` (S170; see
    /// [`LinkBinRecorder::record_unfinished`]). Call once, after the last step.
    pub fn record_unfinished(&mut self, end: f64) {
        let n = self.links;
        let Some(recorder) = self.recorder.as_mut() else { return };
        for queue in 0..2 * n {
            let link = LinkId::from_index(queue % n);
            for q in &self.queues[queue] {
                let pcu = self.vehicles[q.slot as usize].pcu.get();
                if queue < n {
                    recorder.record_unfinished(link, q.enter, end, pcu);
                } else {
                    // Still waiting outside the network: `enter` is its departure.
                    recorder.record_origin_wait(link, q.enter, end, pcu);
                }
            }
        }
    }

    /// [`Self::take_link_bins`], and the entry-time table if asked for.
    #[must_use]
    pub fn take_link_bins_with_entry(&mut self) -> Option<(LinkBins, Option<EntryTables>)> {
        self.recorder.take().map(LinkBinRecorder::finish_with_entry)
    }

    /// The per-link, per-bin results recorded so far, if
    /// [`Self::with_link_bins`] asked for them. Recording stops.
    #[must_use]
    pub fn take_link_bins(&mut self) -> Option<LinkBins> {
        self.recorder.take().map(LinkBinRecorder::finish)
    }

    /// Offer a vehicle a new route when it has waited `rule.after_s` at the front of its link,
    /// blocked — for room on its next link, or giving way (S213). The new route comes from the
    /// [`Reroute`] given to [`Self::step_rerouting`]; a vehicle takes at most `rule.max`, and is
    /// offered again only after waiting as long again. Must be set before vehicles are added.
    ///
    /// Cost: per vehicle, 13 bytes (which route, how many, when last offered); per reroute, the
    /// new route and a 32-byte record; one timer per waiting link front.
    ///
    /// # Panics
    ///
    /// Panics if vehicles were already added.
    #[must_use]
    pub fn with_rerouting(mut self, rule: RerouteRule) -> Self {
        assert!(self.vehicles.is_empty(), "rerouting is set before vehicles are added");
        self.rerouting = Some(Rerouting {
            rule,
            route_of: Vec::new(),
            count: Vec::new(),
            last_offer: Vec::new(),
            routes: Vec::new(),
            pending: Vec::new(),
            records: Vec::new(),
        });
        self
    }

    /// Every reroute so far, in the order they happened.
    #[must_use]
    pub fn reroutes(&self) -> &[RerouteRecord] {
        self.rerouting.as_ref().map_or(&[], |r| r.records.as_slice())
    }

    /// The route a vehicle follows: its own, or the one it re-routed to.
    #[inline]
    fn route(&self, slot: u32) -> &[LinkId] {
        if let Some(r) = &self.rerouting {
            let i = r.route_of[slot as usize];
            if i != NONE {
                return &r.routes[i as usize];
            }
        }
        &self.vehicles[slot as usize].route
    }

    /// Give priority by road hierarchy at unsignalised merges (S213): a vehicle entering a
    /// link gives way while the front vehicle of an approach of **higher priority** to the same
    /// node is bound for that link and ready to go — Daganzo's (1995) priority merge in vehicle
    /// form: the minor stream takes what the major one leaves, and a departure (an origin)
    /// enters the stream only when no approach's front wants the link. An approach has priority
    /// over another if its road class is higher, or, in one class, if its capacity is at least
    /// [`PRIORITY_CAPACITY_RATIO`] times the other's; otherwise neither (the capacity-
    /// proportional merge, as without priority). Not at signalised nodes, where the signal
    /// separates the streams, nor into roundabout links, which keep their own rule.
    ///
    /// Why: a merge that shares room in a fixed ratio lets traffic on a closed loop destroy
    /// itself (Daganzo 1996, *The nature of freeway gridlock and how to prevent it*); priority
    /// to the major stream is the remedy that theory names. Cost: one flag per link, and per
    /// entry into a merge a look at the front of each other approach of the node.
    #[must_use]
    pub fn with_priority(mut self) -> Self {
        let net = self.network;
        self.priority_major = (0..self.links)
            .map(|b| {
                let lb = LinkId::from_index(b);
                net.link_class(lb).carries_motor_traffic() && !net.is_signalised(net.link_to(lb))
            })
            .collect();
        self
    }

    /// Whether approach `b` has priority over approach `a` (`None`: a departure from an origin).
    fn outranks(&self, b: usize, a: Option<usize>) -> bool {
        let net = self.network;
        let lb = LinkId::from_index(b);
        if !net.link_class(lb).carries_motor_traffic() {
            return false;
        }
        let Some(a) = a else { return true };
        let (cb, ca) = (net.link_class(lb) as u8, net.link_class(LinkId::from_index(a)) as u8);
        cb < ca
            || (cb == ca && self.inflow_rate[b] >= PRIORITY_CAPACITY_RATIO * self.inflow_rate[a])
    }

    /// The approach a vehicle entering `j` from `from` gives way to under priority, if any: one
    /// of higher priority whose front vehicle is bound for `j` and ready by `t`.
    fn priority_give_way(&self, j: usize, from: Option<usize>, t: f64) -> Option<usize> {
        if self.priority_major.is_empty() {
            return None;
        }
        let net = self.network;
        let lj = LinkId::from_index(j);
        if net.is_roundabout(lj) {
            return None;
        }
        let node = net.link_from(lj);
        if net.is_signalised(node) {
            return None;
        }
        net.in_links(node).iter().map(|l| l.index()).find(|&b| {
            Some(b) != from
                && self.priority_major[b]
                && self.outranks(b, from)
                && self.head_bound_for(b, j, t)
        })
    }

    /// Give approaches of two lanes or more **turn pockets** (S217): a vehicle at the end of such a
    /// link may pass vehicles ahead of it that wait for another movement, as long as those fit in
    /// their movements' pockets — they wait in their own lanes — and none ahead of it is bound
    /// the same way. Each movement's pocket is `length_m` metres of its share of the lanes (the
    /// lanes split evenly among the link's movements, at least one each), never longer than the
    /// link. A link of one lane, or with one movement, keeps one first-in-first-out queue. The
    /// link's discharge capacity stays shared by its movements.
    ///
    /// Why: with one queue per link, a vehicle that cannot enter its next link holds up every
    /// vehicle behind it, including those turning into free streets — full first-in-first-out
    /// at diverges is a known source of unrealistic gridlock (Wright et al. 2017, *On node
    /// models for high-dimensional road networks*), and it made priority at merges hold up more
    /// than it let through (S214). Cost: 16 bytes per link and 12 per movement; and per look at
    /// a link whose front waits, a look at the vehicles behind it as far as the pockets reach.
    #[must_use]
    pub fn with_pockets(mut self, turns: &TurnTable, length_m: f64) -> Self {
        let net = self.network;
        let n = self.links;
        let mut size = Vec::with_capacity(n);
        let mut first = Vec::with_capacity(n + 1);
        let mut owner = Vec::new();
        for i in 0..n {
            let link = LinkId::from_index(i);
            let lanes = f64::from(net.link_lanes(link));
            #[allow(clippy::cast_precision_loss, reason = "a handful of turns")]
            let movements = turns.turns_from(link).len() as f64;
            let length = net.link_length(link).get();
            let pocket = if lanes >= 2.0 && movements >= 2.0 && length > 0.0 && length_m > 0.0 {
                let share = (lanes / movements).max(1.0);
                net.storage(link).get() * (length_m / length).min(1.0) * share / lanes
            } else {
                0.0
            };
            size.push(pocket);
            first.push(link_u32(owner.len()));
            let leaving = net.out_links(net.link_to(link)).len() + 1;
            owner.extend(std::iter::repeat_n(link_u32(i), leaving));
        }
        first.push(link_u32(owner.len()));
        let m = owner.len();
        self.pockets = Some(Pockets {
            size,
            first,
            owner,
            parked_on: vec![NONE; m],
            next_parked: vec![NONE; m],
            first_parked: vec![NONE; n],
        });
        self
    }

    /// What each movement's pocket on link `i` holds, in PCU: 0 without pockets.
    #[inline]
    fn pocket(&self, i: usize) -> f64 {
        self.pockets.as_ref().map_or(0.0, |p| p.size[i])
    }

    /// The movement from link `i` onto link `j`, among `i`'s.
    fn movement(&self, p: &Pockets, i: usize, j: usize) -> usize {
        let net = self.network;
        let leaving = net.out_links(net.link_to(LinkId::from_index(i)));
        let k = leaving.iter().position(|l| l.index() == j).unwrap_or(leaving.len());
        p.first[i] as usize + k
    }

    /// Link `i`'s queue waits on link `on` for its movement onto `j` (S217), besides whatever
    /// its front waits on: room heard on `on`, or its upstream end clearing, wakes the queue.
    fn park_movement(&mut self, i: usize, j: usize, on: usize) {
        let Some(mut p) = self.pockets.take() else { return };
        let m = self.movement(&p, i, j);
        let on32 = link_u32(on);
        if p.parked_on[m] != on32 {
            if p.parked_on[m] != NONE {
                let l = p.parked_on[m] as usize;
                let m32 = link_u32(m);
                if p.first_parked[l] == m32 {
                    p.first_parked[l] = p.next_parked[m];
                } else {
                    let mut at = p.first_parked[l];
                    while at != NONE {
                        let next = p.next_parked[at as usize];
                        if next == m32 {
                            p.next_parked[at as usize] = p.next_parked[m];
                            break;
                        }
                        at = next;
                    }
                }
            }
            p.next_parked[m] = p.first_parked[on];
            p.first_parked[on] = link_u32(m);
            p.parked_on[m] = on32;
        }
        self.pockets = Some(p);
    }

    /// The part on link `i` of the vehicle straddling link `j`'s upstream end, if it came from
    /// `i`: it straddles `i`'s downstream end.
    fn straddle_part(&self, j: usize, i: usize) -> Option<f64> {
        let s = self.straddling_in[j];
        if s == NONE {
            return None;
        }
        let parts = &self.parts[s as usize];
        let k = parts.iter().position(|p| p.0 as usize == j)?;
        parts.get(k + 1).filter(|p| p.0 as usize == i).map(|p| p.1)
    }

    /// The vehicle straddling link `i`'s downstream end that holds up its front, if any: the
    /// only one without turn pockets; with them, the one in the front's movement, or one that
    /// does not fit in its pocket (S217). With no vehicle on the link, any.
    fn straddler_ahead(&self, i: usize) -> Option<u32> {
        if self.out_straddlers[i] == 0 {
            return None;
        }
        let net = self.network;
        let pocket = self.pocket(i);
        let front = self.queues[i].front();
        let front_next =
            front.and_then(|q| self.route(q.slot).get(q.leg as usize + 1)).map(|l| l.index());
        for &j in net.out_links(net.link_to(LinkId::from_index(i))) {
            let j = j.index();
            let Some(part) = self.straddle_part(j, i) else { continue };
            if pocket <= 0.0
                || front.is_none()
                || Some(j) == front_next
                || part > pocket + ROOM_EPSILON
            {
                return Some(self.straddling_in[j]);
            }
        }
        None
    }

    /// Whether a vehicle at the end of link `b` with its way clear is bound for `j` and ready by
    /// `t`: the front, or with turn pockets one behind it that no vehicle ahead bars (S217).
    fn head_bound_for(&self, b: usize, j: usize, t: f64) -> bool {
        let pocket = self.pocket(b);
        let j = link_u32(j);
        let mut ahead = Ahead::default();
        for q in &self.queues[b] {
            if q.ready > t + TIME_EPSILON {
                return false;
            }
            let key =
                self.route(q.slot).get(q.leg as usize + 1).map_or(EXIT, |l| link_u32(l.index()));
            if key == j && !ahead.has(key) {
                return true;
            }
            if !ahead.add(key, self.vehicles[q.slot as usize].pcu.get(), pocket) {
                return false;
            }
        }
        false
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

    /// The link `i`'s front currently waits on, if it is blocked: the link its
    /// queue is parked on, or, while the vehicle ahead still straddles the
    /// link's end, the link that vehicle's front is on. `None` means `i`'s
    /// front is not currently waiting for anything.
    fn front_waits_on(&self, i: usize) -> Option<usize> {
        if let Some(ahead) = self.straddler_ahead(i) {
            return self.parts[ahead as usize].front().map(|p| p.0 as usize);
        }
        (self.parked_on[i] != NONE).then(|| self.parked_on[i] as usize)
    }

    /// Closed loops of links whose front vehicles wait on one another now,
    /// each as its links in waiting order. By the module docs' argument every
    /// link whose room is waited for in such a loop is full, to within
    /// [`MIN_PART`]: this is where traffic has reached jam density.
    #[must_use]
    pub fn waiting_cycles(&self) -> Vec<Vec<LinkId>> {
        const UNSEEN: u8 = 0;
        const ON_PATH: u8 = 1;
        const DONE: u8 = 2;
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
                match self.front_waits_on(at) {
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

    /// What stands still now: see [`LockReport`]. Walks every link and vehicle once.
    ///
    /// # Panics
    ///
    /// Never in practice: counts of links and vehicles fit `u32` (Foundations §1).
    #[must_use]
    pub fn lock_report(&self) -> LockReport {
        let now = self.now;
        let on_network = self.parts.iter().filter(|p| !p.is_empty()).count();
        let outside: usize = (self.links..3 * self.links).map(|q| self.queues[q].len()).sum();
        let mut waiting_links = 0;
        let mut room_waits_with_room = 0;
        for i in 0..self.links {
            let Some(on) = self.front_waits_on(i) else { continue };
            waiting_links += 1;
            let Some(front) = self.queues[i].front() else { continue };
            let next = self.route(front.slot).get(front.leg as usize + 1);
            let waits_for_room =
                self.straddler_ahead(i).is_none() && next.is_some_and(|l| l.index() == on);
            if waits_for_room && self.storage[on].is_finite() {
                let need = MIN_PART.min(self.storage[on]);
                if self.heard_room(on, now).0 >= need + ROOM_EPSILON {
                    room_waits_with_room += 1;
                }
            }
        }
        // With turn pockets, the movements waiting behind a front for room on their own next
        // link (S217) are held to the same guarantee.
        if let Some(p) = &self.pockets {
            let net = self.network;
            for (m, &on) in p.parked_on.iter().enumerate() {
                if on == NONE {
                    continue;
                }
                let (on, i) = (on as usize, p.owner[m] as usize);
                let leaving = net.out_links(net.link_to(LinkId::from_index(i)));
                let waits_for_room = leaving
                    .get(m - p.first[i] as usize)
                    .is_some_and(|l| l.index() == on && self.straddling_in[on] == NONE);
                if waits_for_room && self.storage[on].is_finite() {
                    let need = MIN_PART.min(self.storage[on]);
                    if self.heard_room(on, now).0 >= need + ROOM_EPSILON {
                        room_waits_with_room += 1;
                    }
                }
            }
        }
        let count = |n: usize| u32::try_from(n).expect("counts fit u32");
        LockReport {
            on_network: count(on_network),
            outside: count(outside),
            waiting_links: count(waiting_links),
            loops: self.waiting_cycles(),
            room_waits_with_room: count(room_waits_with_room),
        }
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
        let slot = self.add(vehicle);
        self.pending.push(slot);
        self.pending_sorted = false;
    }

    /// Schedule a vehicle that departs when the vehicle in slot `after` arrives:
    /// `wait` seconds later, and not before `not_before`. Slots are numbered in
    /// the order vehicles are scheduled, from 0, by this and [`Self::depart`]
    /// alike; this returns the new vehicle's. If the one it follows never arrives,
    /// this one never departs. See the [module docs](self).
    ///
    /// # Panics
    ///
    /// As [`Self::depart`]; also if `after` is not a slot, or already has a
    /// vehicle chained to it.
    pub fn depart_after(
        &mut self,
        vehicle: &'a Vehicle,
        after: u32,
        wait: f64,
        not_before: f64,
    ) -> u32 {
        if self.chains.is_none() {
            let n = self.vehicles.len();
            self.chains = Some(Chains {
                next: vec![NONE; n],
                wait: vec![0.0; n],
                not_before: vec![0.0; n],
                start: self.vehicles.iter().map(|v| f64::from(v.departure.get())).collect(),
            });
        }
        let slot = self.add(vehicle);
        let chains = self.chains.as_mut().expect("made above");
        let prev = &mut chains.next[after as usize];
        assert_eq!(*prev, NONE, "one vehicle is chained to another at most");
        *prev = slot;
        chains.wait[slot as usize] = wait;
        chains.not_before[slot as usize] = not_before;
        chains.start[slot as usize] = f64::NAN;
        slot
    }

    fn add(&mut self, vehicle: &'a Vehicle) -> u32 {
        assert!(!vehicle.route.is_empty(), "a vehicle always has a route");
        let slot = u32::try_from(self.vehicles.len()).expect("vehicle count fits u32");
        self.vehicles.push(vehicle);
        if let Some(r) = self.rerouting.as_mut() {
            r.route_of.push(NONE);
            r.count.push(0);
            r.last_offer.push(f64::NEG_INFINITY);
        }
        self.traversals.push(Vec::with_capacity(vehicle.route.len()));
        self.parts.push(VecDeque::new());
        if let Some(c) = self.chains.as_mut() {
            c.next.push(NONE);
            c.wait.push(0.0);
            c.not_before.push(0.0);
            c.start.push(f64::from(vehicle.departure.get()));
        }
        slot
    }

    /// When the vehicle in `slot` departs: its own departure, or a chained
    /// vehicle's release.
    #[inline]
    fn departure_of(&self, slot: u32) -> f64 {
        match &self.chains {
            Some(c) => c.start[slot as usize],
            None => f64::from(self.vehicles[slot as usize].departure.get()),
        }
    }

    /// Advance the loading by `dt`, returning every trip that finished in it,
    /// in the order they finished.
    ///
    /// # Panics
    ///
    /// Panics if `dt` is not positive and finite.
    pub fn step(&mut self, dt: Duration) -> Vec<Trajectory> {
        self.step_inner(dt, None)
    }

    /// [`Self::step`], asking `rr` for a new route for every vehicle due one under
    /// [`Self::with_rerouting`] (S213).
    ///
    /// # Panics
    ///
    /// As [`Self::step`].
    pub fn step_rerouting(&mut self, dt: Duration, rr: &mut dyn Reroute) -> Vec<Trajectory> {
        self.step_inner(dt, Some(rr))
    }

    fn step_inner(&mut self, dt: Duration, mut rr: Option<&mut dyn Reroute>) -> Vec<Trajectory> {
        assert!(dt.get() > 0.0 && dt.is_finite(), "a loading step must be positive, got {dt:?}");
        let (t0, t1) = (self.now, self.now + dt.get());
        let mut completed = Vec::new();
        self.sort_pending();
        self.clock = self.clock.max(t0);

        loop {
            let next_event = self.events.peek().map(|e| e.time);
            let next_departure = self.next_departure_time(t0);
            match (next_event, next_departure) {
                (_, Some(d)) if d < t1 && next_event.is_none_or(|e| d <= e) => {
                    self.clock = self.clock.max(d);
                    self.depart_next(t0);
                }
                (Some(e), _) if e < t1 => {
                    let event = self.events.pop().expect("just peeked");
                    self.clock = self.clock.max(event.time);
                    self.process(event, t0, &mut completed);
                }
                _ => break,
            }
            if let (Some(rr), Some(r)) = (rr.as_deref_mut(), self.rerouting.as_ref()) {
                if !r.pending.is_empty() {
                    self.serve_reroutes(t0, rr);
                }
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

    /// The order of pending departures: later first, so the next is last.
    fn pending_order(&self, a: u32, b: u32) -> Ordering {
        let (va, vb) = (self.vehicles[a as usize], self.vehicles[b as usize]);
        self.departure_of(b)
            .total_cmp(&self.departure_of(a))
            .then((vb.id.raw(), b).cmp(&(va.id.raw(), a)))
    }

    fn sort_pending(&mut self) {
        if !self.pending_sorted {
            let mut pending = std::mem::take(&mut self.pending);
            pending.sort_unstable_by(|&a, &b| self.pending_order(a, b));
            self.pending = pending;
            self.pending_sorted = true;
        }
    }

    fn next_departure_time(&self, t0: f64) -> Option<f64> {
        self.pending.last().map(|&slot| self.departure_of(slot).max(t0))
    }

    /// The next pending vehicle joins its first link's origin queue.
    fn depart_next(&mut self, t0: f64) {
        let slot = self.pending.pop().expect("a departure is pending");
        let vehicle = self.vehicles[slot as usize];
        let at = self.departure_of(slot).max(t0);
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
        } else if queue < 4 * n {
            Place::Heard(queue - 3 * n)
        } else {
            Place::RerouteDue(queue - 4 * n)
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
                let onto = self.route(front.slot).get(front.leg as usize + 1);
                let virtual_now = onto.map_or(0.0, |l| self.virtual_time[l.index()]);
                let tag = virtual_now.max(self.service_tag[queue] + headway);
                Some((ready, tag))
            }
            Place::Origin | Place::StopLine(_) | Place::Heard(_) | Place::RerouteDue(_) => {
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
            let event =
                Event { time: ready.max(not_before).max(self.clock), tag, queue: queue_u32(queue) };
            if queue < self.links && self.pocket(queue) > 0.0 {
                // A queue with turn pockets may wait for several things at once (S217): the
                // earliest look wins, and a look finds everything that can move.
                self.events.schedule_earlier(event);
            } else {
                self.events.schedule(event);
            }
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
        self.events.schedule(Event { time: at, tag: at, queue: queue_u32(queue) });
    }

    fn process(&mut self, event: Event, t0: f64, completed: &mut Vec<Trajectory>) {
        let queue = event.queue as usize;
        if let Place::Heard(l) = self.place(queue) {
            self.heard_due[l] = f64::INFINITY;
            // The straddling vehicle is entitled to the room first.
            self.close_straddle(l, event.time, t0);
            return self.wake(l, event.time, t0);
        }
        if let Place::RerouteDue(i) = self.place(queue) {
            return self.offer_reroute(i, event.time);
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
        let next = self.route(front.slot).get(next_leg).map(|l| l.index());

        match self.place(queue) {
            Place::Link(i) => {
                if self.pocket(i) > 0.0 {
                    return self.process_pockets(i, t, tag, t0, completed);
                }
                if self.out_straddlers[i] > 0 {
                    // The vehicle ahead is still in the way; its rear clearing
                    // the link end reschedules this queue. Waiting behind it counts
                    // towards a reroute (S213).
                    if self.rerouting.is_some() {
                        self.offer_reroute(i, t);
                    }
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
                self.advance(i, 0, tag, t, room, t0, completed);
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
            Place::Heard(_) | Place::RerouteDue(_) => {
                unreachable!("room-heard and reroute events are handled first")
            }
        }
    }

    /// Link `i`'s queue at `t`, with turn pockets (S217): the first vehicle at the end of the link
    /// that can move does — the front, or one behind it whose way is clear: none ahead of it bound
    /// for its movement, and those ahead bound for each other movement within its pocket. If none
    /// can, the queue waits on what holds up each movement within reach, and looks again at the
    /// earliest time one may go.
    fn process_pockets(
        &mut self,
        i: usize,
        t: f64,
        front_tag: f64,
        t0: f64,
        completed: &mut Vec<Trajectory>,
    ) {
        let pocket = self.pocket(i);
        let mut ahead = Ahead::default();
        // Vehicles straddling the link's end are ahead, each in its movement's lane.
        if self.out_straddlers[i] > 0 {
            let net = self.network;
            for &j in net.out_links(net.link_to(LinkId::from_index(i))) {
                let j = j.index();
                let Some(part) = self.straddle_part(j, i) else { continue };
                if !ahead.add(link_u32(j), part, pocket) {
                    // It does not fit in its pocket: everything waits for its rear to clear the
                    // link's end, which looks at this queue again.
                    if self.rerouting.is_some() {
                        self.offer_reroute(i, t);
                    }
                    return;
                }
            }
        }
        let mut look_again = f64::INFINITY;
        let mut front_held = None;
        let mut held = [(0, Blocked { link: 0, retry_at: None }); 8];
        let mut n_held = 0;
        let last_exit = self.curves[i].last_exit().get();
        for k in 0..self.queues[i].len() {
            let q = self.queues[i][k];
            let pcu = self.vehicles[q.slot as usize].pcu.get();
            let next = self.route(q.slot).get(q.leg as usize + 1).map(|l| l.index());
            let key = next.map_or(EXIT, link_u32);
            if ahead.has(key) {
                // Behind a vehicle bound the same way, in its lane.
                if k == 0 && self.rerouting.is_some() {
                    self.offer_reroute(i, t);
                }
                if !ahead.add(key, pcu, pocket) {
                    break;
                }
                continue;
            }
            let headway = pcu / self.discharge_rate[i];
            let ready = q.ready.max(last_exit + headway);
            if ready > t + TIME_EPSILON {
                look_again = look_again.min(ready);
                break;
            }
            let room = match next {
                None => Some(f64::INFINITY),
                Some(j) => {
                    // On a signalised approach the inflow headway applies at the stop line.
                    let clear = self.curves[j].last_entry().get() + pcu / self.inflow_rate[j];
                    if self.stop_delay[i] <= 0.0 && clear > t + TIME_EPSILON {
                        look_again = look_again.min(clear);
                        None
                    } else {
                        match self.admissible(j, pcu, t, Some(i)) {
                            Ok(room) => Some(room),
                            Err(blocked) => {
                                if k == 0 {
                                    if self.rerouting.is_some() {
                                        self.offer_reroute(i, t);
                                    }
                                    front_held = Some(blocked);
                                } else if n_held < held.len() {
                                    held[n_held] = (j, blocked);
                                    n_held += 1;
                                }
                                None
                            }
                        }
                    }
                }
            };
            if let Some(room) = room {
                let tag = if k == 0 {
                    front_tag
                } else {
                    let virtual_now = next.map_or(0.0, |j| self.virtual_time[j]);
                    virtual_now.max(self.service_tag[i] + headway)
                };
                return self.advance(i, k, tag, t, room, t0, completed);
            }
            if !ahead.add(key, pcu, pocket) {
                break;
            }
        }
        // Nothing can move now.
        if let Some(blocked) = front_held {
            self.hold(i, blocked, t, t0);
        }
        for &(j, blocked) in &held[..n_held] {
            match blocked.retry_at {
                Some(at) if at > t + TIME_EPSILON => look_again = look_again.min(at),
                _ => self.park_movement(i, j, blocked.link),
            }
        }
        if look_again.is_finite() {
            self.schedule_front(i, t0, look_again);
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
        if let Some(b) = self.priority_give_way(j, from, t) {
            return Err(Blocked { link: b, retry_at: None });
        }
        let storage = self.storage[j];
        if storage.is_infinite() {
            return Ok(f64::INFINITY);
        }
        let (room, retry_at) = self.heard_room(j, t);
        if room + ROOM_EPSILON >= pcu.min(storage).min(MIN_PART) {
            // Idea 1 (S157): an outsider may not take the last room on `j` when
            // doing so would close a waiting loop back to `j`. A link is down to
            // its last room once anything smaller would already have failed the
            // check just above, so gating the (bounded) walk on that keeps it
            // off the common, uncongested case entirely.
            if room < MIN_PART + ROOM_EPSILON && self.closes_a_waiting_loop(j, from) {
                return Err(Blocked { link: j, retry_at: None });
            }
            return Ok(room.max(0.0));
        }
        Err(Blocked { link: j, retry_at })
    }

    /// Whether granting `j` the room it has left would close a waiting loop
    /// back to `j` itself, sealed by a vehicle from **outside** that loop —
    /// idea 1 (S157), generalising S155's roundabout give-way to any local
    /// lock, tagged or not (K30). Walks `j`'s current chain of waiting fronts
    /// ([`Self::front_waits_on`]) forward; if it returns to `j`, this
    /// admission would complete the cycle. A vehicle whose own link (`from`)
    /// is already a member of that chain is circulating within the loop, not
    /// sealing it from outside, and is let through — this is what keeps a
    /// loop's own traffic moving while it still can. Idea 1 does not, on its
    /// own, guarantee a loop never locks (S156): it can still fill through its
    /// own circulation; idea 2 (creep) is the guarantee.
    fn closes_a_waiting_loop(&self, j: usize, from: Option<usize>) -> bool {
        let mut at = j;
        for _ in 0..MAX_LOOP_WALK {
            let Some(next) = self.front_waits_on(at) else { return false };
            if Some(next) == from {
                return false;
            }
            if next == j {
                return true;
            }
            at = next;
        }
        false
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
                    self.route(q.slot).get(q.leg as usize + 1).is_some_and(|l| l.index() == j)
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

    /// The vehicle at `pos` in link `i`'s queue — its front, or with turn pockets one that
    /// passes it — moves on at `t`: across the stop line on a signalised approach, onto the next
    /// link where `room` is heard, or out of the network. Everything that could hold it back
    /// has been checked.
    #[allow(clippy::too_many_arguments, reason = "the move's where, when, and what it takes")]
    fn advance(
        &mut self,
        i: usize,
        pos: usize,
        tag: f64,
        t: f64,
        room: f64,
        t0: f64,
        completed: &mut Vec<Trajectory>,
    ) {
        let front = self.queues[i].remove(pos).expect("advancing a vehicle in the queue");
        let slot = front.slot;
        let vehicle = self.vehicles[slot as usize];
        if self.waited_on(i)
            && (self.network.is_roundabout(LinkId::from_index(i))
                || self.priority_major.get(i).copied().unwrap_or(false))
        {
            // Entries giving way to this link's front may go now.
            self.wake_giving_way(i, t, t0);
        }
        self.curves[i].set_last_exit(Duration(t));
        self.discharged[i] += vehicle.pcu.get();
        self.service_tag[i] = tag;
        let next_leg = front.leg as usize + 1;
        let route_len = self.route(slot).len();
        if let Some(onto) = self.route(slot).get(next_leg).map(|l| l.index()) {
            let v = &mut self.virtual_time[onto];
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
            if next_leg < route_len {
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
        let j = self.route(slot)[leg].index();
        let take = Self::part_taken(room, vehicle.pcu.get());
        let from = self.parts[s].front().expect("a vehicle on a link has parts").0 as usize;
        self.curves[j].record_in(Pcu(take));
        self.curves[j].set_last_entry(Duration(t));
        self.parts[s].push_front((link_u32(j), take));
        // It straddles the link end it crosses until its rear has cleared it.
        self.out_straddlers[from] += 1;
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
        let pcu = self.vehicles[s].pcu.get();
        let j = self.route(slot)[leg].index();
        if leg == 0 {
            // Out of the origin queue and onto the first link: the wait is the cost of
            // setting out then (S170).
            let departure = self.departure_of(slot);
            if let Some(recorder) = self.recorder.as_mut() {
                recorder.record_origin_wait(LinkId::from_index(j), departure, t, pcu);
            }
        }
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
            if link != NONE {
                // Every part behind a vehicle's front straddles its link's downstream end.
                self.out_straddlers[link as usize] -= 1;
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
        if self.straddling_in[l] != NONE || self.waited_on(l) {
            self.schedule_heard(l, t + self.wave_lag[l]);
        }
    }

    /// Whether a queue waits on link `l`: for its front, or with turn pockets for another
    /// movement (S217).
    #[inline]
    fn waited_on(&self, l: usize) -> bool {
        self.first_parked[l] != NONE
            || self.pockets.as_ref().is_some_and(|p| p.first_parked[l] != NONE)
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
        let departure = match &self.chains {
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a release within the run's u32 clock"
            )]
            Some(c) => Second(c.start[s].floor() as u32),
            None => vehicle.departure,
        };
        completed.push(Trajectory {
            vehicle: vehicle.id,
            departure,
            links: std::mem::take(&mut self.traversals[s]),
        });
        // A vehicle chained to this one is released now (S199).
        let next = self.chains.as_ref().map_or(NONE, |c| c.next[s]);
        if next != NONE {
            let c = self.chains.as_mut().expect("chained");
            let n = next as usize;
            c.start[n] = (t + c.wait[n]).max(c.not_before[n]);
            let at =
                self.pending.partition_point(|&p| self.pending_order(p, next) == Ordering::Less);
            self.pending.insert(at, next);
        }
    }

    /// Hold a queue: retry at a known time, or park until `blocked.link`
    /// releases room or its upstream end.
    fn block(&mut self, queue: usize, blocked: Blocked, t: f64, t0: f64) {
        if queue < self.links && self.rerouting.is_some() {
            self.offer_reroute(queue, t);
        }
        self.hold(queue, blocked, t, t0);
    }

    /// [`Self::block`] without the offer of a new route.
    fn hold(&mut self, queue: usize, blocked: Blocked, t: f64, t0: f64) {
        if let Some(at) = blocked.retry_at {
            if at > t + TIME_EPSILON {
                return self.schedule_front(queue, t0, at);
            }
        }
        if self.parked_on[queue] != NONE {
            self.unpark(queue);
        }
        let l = blocked.link;
        self.next_parked[queue] = self.first_parked[l];
        self.first_parked[l] = queue_u32(queue);
        self.parked_on[queue] = link_u32(l);
    }

    /// Take a parked queue off the list it is parked on.
    fn unpark(&mut self, queue: usize) {
        let l = self.parked_on[queue] as usize;
        let q = queue_u32(queue);
        if self.first_parked[l] == q {
            self.first_parked[l] = self.next_parked[queue];
        } else {
            let mut at = self.first_parked[l];
            while at != NONE {
                let next = self.next_parked[at as usize];
                if next == q {
                    self.next_parked[at as usize] = self.next_parked[queue];
                    break;
                }
                at = next;
            }
        }
        self.next_parked[queue] = NONE;
        self.parked_on[queue] = NONE;
    }

    /// Link `i`'s front vehicle, blocked (for room, or giving way; parked or waiting for room
    /// already released to be heard): if it has waited long enough since it reached the end of
    /// the link, and may still re-route, it is due an offer, served right after the event being
    /// processed; if not yet, its timer is set for when it will have. A timer that fires finds
    /// the front again: the same vehicle still waiting, or another to time afresh.
    fn offer_reroute(&mut self, i: usize, t: f64) {
        let Some(front) = self.queues[i].front().copied() else { return };
        let n = self.links;
        let Some(r) = self.rerouting.as_mut() else { return };
        let slot = front.slot as usize;
        if r.count[slot] >= r.rule.max {
            return;
        }
        let due = front.ready.max(r.last_offer[slot]) + r.rule.after_s;
        if due <= t + TIME_EPSILON {
            r.pending.push(queue_u32(i));
        } else {
            self.events.schedule(Event { time: due, tag: due, queue: queue_u32(4 * n + i) });
        }
    }

    /// Offer every link front due one a new route, in the order they became due (S213).
    fn serve_reroutes(&mut self, t0: f64, rr: &mut dyn Reroute) {
        let Some(pending) = self.rerouting.as_mut().map(|r| std::mem::take(&mut r.pending)) else {
            return;
        };
        let t = self.clock;
        for i in pending {
            let i = i as usize;
            let Some(front) = self.queues[i].front().copied() else { continue };
            let slot = front.slot as usize;
            let leg = front.leg as usize;
            let planned: Vec<LinkId> = self.route(front.slot)[leg + 1..].to_vec();
            if planned.is_empty() {
                continue;
            }
            let vehicle = self.vehicles[slot].id;
            let reason = RerouteReason::Stuck;
            let answer = rr.reroute(vehicle, LinkId::from_index(i), &planned, t, &*self, reason);
            let net = self.network;
            let end = net.link_to(*planned.last().expect("not empty"));
            let accepted = answer.filter(|rest| {
                rest.first().is_some_and(|&f| f != planned[0])
                    && rest.last().is_some_and(|&l| net.link_to(l) == end)
            });
            let r = self.rerouting.as_mut().expect("rerouting is on");
            r.last_offer[slot] = t;
            let Some(rest) = accepted else {
                let due = t + r.rule.after_s;
                let n = self.links;
                self.events.schedule(Event { time: due, tag: due, queue: queue_u32(4 * n + i) });
                continue;
            };
            let mut whole: Vec<LinkId> = Vec::with_capacity(leg + 1 + rest.len());
            whole.extend_from_slice(&self.route(front.slot)[..=leg]);
            whole.extend_from_slice(&rest);
            let r = self.rerouting.as_mut().expect("rerouting is on");
            match r.route_of[slot] {
                NONE => {
                    r.route_of[slot] =
                        u32::try_from(r.routes.len()).expect("fewer reroutes than u32");
                    r.routes.push(whole.into_boxed_slice());
                }
                k => r.routes[k as usize] = whole.into_boxed_slice(),
            }
            r.count[slot] += 1;
            r.records.push(RerouteRecord {
                vehicle,
                second: t,
                link: LinkId::from_index(i),
                planned_next: planned[0],
                new_next: rest[0],
                reason,
            });
            if self.parked_on[i] != NONE {
                self.unpark(i);
            }
            self.schedule_front(i, t0, t);
        }
    }

    /// The node where a queue's front vehicle waits: a link's downstream end, an origin at its
    /// link's upstream end, a stop line at its approach's downstream end.
    fn queue_node(&self, queue: usize) -> usize {
        let n = self.links;
        let net = self.network;
        match self.place(queue) {
            Place::Link(i) | Place::StopLine(i) => net.link_to(LinkId::from_index(i)).index(),
            Place::Origin => net.link_from(LinkId::from_index(queue - n)).index(),
            Place::Heard(_) | Place::RerouteDue(_) => usize::MAX,
        }
    }

    /// Link `l`'s front has moved on: reschedule, no earlier than `at`, the queues parked on `l`
    /// that give way to it — those waiting at its downstream node — and leave parked those
    /// waiting for room on `l`, at its upstream node, whose room is heard later (S218: waking
    /// them too cost every unsignalised merge a look at each of them per vehicle, under priority).
    fn wake_giving_way(&mut self, l: usize, at: f64, t0: f64) {
        let node = self.network.link_to(LinkId::from_index(l)).index();
        let mut parked = std::mem::replace(&mut self.first_parked[l], NONE);
        while parked != NONE {
            let q = parked as usize;
            parked = std::mem::replace(&mut self.next_parked[q], NONE);
            if self.queue_node(q) == node {
                self.parked_on[q] = NONE;
                self.schedule_front(q, t0, at);
            } else {
                self.next_parked[q] = self.first_parked[l];
                self.first_parked[l] = queue_u32(q);
            }
        }
        // The same for movements waiting behind a front; those kept are relinked at the end.
        let net = self.network;
        let mut kept = NONE;
        while let Some(p) = self.pockets.as_mut() {
            let m = p.first_parked[l];
            if m == NONE {
                p.first_parked[l] = kept;
                break;
            }
            let k = m as usize;
            p.first_parked[l] = std::mem::replace(&mut p.next_parked[k], NONE);
            let owner = p.owner[k] as usize;
            if net.link_to(LinkId::from_index(owner)).index() == node {
                p.parked_on[k] = NONE;
                self.schedule_front(owner, t0, at);
            } else {
                p.next_parked[k] = kept;
                kept = m;
            }
        }
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
        // Queues with turn pockets waiting on `l` for a movement other than their front's.
        while let Some(p) = self.pockets.as_mut() {
            let m = p.first_parked[l];
            if m == NONE {
                break;
            }
            let m = m as usize;
            p.first_parked[l] = std::mem::replace(&mut p.next_parked[m], NONE);
            p.parked_on[m] = NONE;
            let owner = p.owner[m] as usize;
            self.schedule_front(owner, t0, at);
        }
    }
}

impl LiveTimes for LtmNetwork<'_> {
    fn live_seconds(&self, link: LinkId) -> f64 {
        let i = link.index();
        let free = self.travel[i] + self.stop_delay[i];
        let rate = self.discharge_rate[i];
        let on = (self.curves[i].cumulative_in() - self.curves[i].cumulative_out()).get();
        // What is on the link beyond what moves freely at capacity is queued; it clears at the
        // discharge rate.
        let queued = ((on - rate * self.travel[i]).max(0.0) / rate).max(0.0);
        let blocked = match (self.front_waits_on(i), self.queues[i].front()) {
            (Some(_), Some(front)) => (self.clock - front.ready).max(0.0),
            _ => 0.0,
        };
        free + queued + blocked
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
        run_ltm_inner(network, turns, vehicles, window, step, level, Some((bin_seconds, false)));
    (done, bins.expect("bins were asked for").0)
}

/// [`run_ltm_binned`], also returning the same traversals filed by the bin they
/// **entered** their link in (S170): what an iterated run reads back as link
/// travel times by time of entry.
///
/// # Panics
///
/// Panics if `step` is not positive.
#[must_use]
pub fn run_ltm_recorded(
    network: &RoadNetwork,
    turns: &TurnTable,
    vehicles: &[Vehicle],
    window: Duration,
    step: Duration,
    level: FidelityLevel,
    bin_seconds: u32,
) -> (Vec<Trajectory>, LinkBins, EntryTables) {
    let (done, bins) =
        run_ltm_inner(network, turns, vehicles, window, step, level, Some((bin_seconds, true)));
    let (exit, tables) = bins.expect("bins were asked for");
    (done, exit, tables.expect("entry bins were asked for"))
}

/// [`run_ltm`] with **chained vehicles** (S199; see the [module docs](self)):
/// each [`Chain`] departs its vehicle when the one it follows arrives. A chained
/// vehicle's own `departure` is not read; its trajectory's departure is its
/// release, floored. `recording` says what else to keep.
///
/// # Panics
///
/// Panics if `step` or a bin length is not positive, or if a chain follows a
/// vehicle that is not earlier in `vehicles`, or two chains follow the same one.
#[must_use]
#[allow(clippy::too_many_arguments, reason = "the loading's inputs, as run_ltm's plus two")]
pub fn run_ltm_chained(
    network: &RoadNetwork,
    turns: &TurnTable,
    vehicles: &[Vehicle],
    chains: &[Chain],
    window: Duration,
    step: Duration,
    level: FidelityLevel,
    recording: Recording,
    rules: Rules,
    rerouter: Option<&mut dyn Reroute>,
) -> LtmOutput {
    let recording = match recording {
        Recording::Trajectories => None,
        Recording::Bins(b) => Some((b, false)),
        Recording::BinsAndEntry(b) => Some((b, true)),
    };
    let (trajectories, bins, lock, reroutes) = run_ltm_inner_chained(
        network, turns, vehicles, chains, window, step, level, recording, rules, rerouter,
    );
    let (link_bins, entry) = match bins {
        Some((b, e)) => (Some(b), e),
        None => (None, None),
    };
    LtmOutput { trajectories, link_bins, entry, lock, reroutes }
}

fn run_ltm_inner(
    network: &RoadNetwork,
    turns: &TurnTable,
    vehicles: &[Vehicle],
    window: Duration,
    step: Duration,
    level: FidelityLevel,
    recording: Option<(u32, bool)>,
) -> (Vec<Trajectory>, Option<(LinkBins, Option<EntryTables>)>) {
    let (done, bins, _, _) = run_ltm_inner_chained(
        network,
        turns,
        vehicles,
        &[],
        window,
        step,
        level,
        recording,
        Rules::default(),
        None,
    );
    (done, bins)
}

/// What a loading leaves: the trajectories, the per-link results, the lock report, the reroutes.
type Inner =
    (Vec<Trajectory>, Option<(LinkBins, Option<EntryTables>)>, LockReport, Vec<RerouteRecord>);

#[allow(clippy::too_many_arguments, reason = "the loading's inputs")]
fn run_ltm_inner_chained(
    network: &RoadNetwork,
    turns: &TurnTable,
    vehicles: &[Vehicle],
    chains: &[Chain],
    window: Duration,
    step: Duration,
    level: FidelityLevel,
    recording: Option<(u32, bool)>,
    rules: Rules,
    mut rerouter: Option<&mut dyn Reroute>,
) -> Inner {
    assert!(step.get() > 0.0, "the loading step must be positive, got {step:?}");
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "window and step are finite and non-negative scenario parameters"
    )]
    let n_steps = (window.get() / step.get()).ceil().max(0.0) as usize;

    let mut sim = LtmNetwork::new(network, turns).with_level(level);
    if rules.priority {
        sim = sim.with_priority();
    }
    if let (Some(rule), true) = (rules.reroute, rerouter.is_some()) {
        sim = sim.with_rerouting(rule);
    }
    if rules.pocket_length_m > 0.0 {
        sim = sim.with_pockets(turns, rules.pocket_length_m);
    }
    // `recording`: the bin length, and whether to also file by entry time (S170).
    let entry_bins = recording.is_some_and(|(_, entry)| entry);
    if let Some((bin_seconds, entry)) = recording {
        sim = sim.with_link_bins(bin_seconds, window.get());
        if entry {
            sim = sim.with_entry_bins();
        }
    }
    if chains.is_empty() {
        for vehicle in vehicles {
            if Duration::from_clock(vehicle.departure) < window {
                sim.depart(vehicle);
            }
        }
    } else {
        let mut follows: Vec<Option<&Chain>> = vec![None; vehicles.len()];
        for c in chains {
            assert!(c.after < c.vehicle, "a chain follows an earlier vehicle");
            assert!(follows[c.vehicle].is_none(), "a vehicle follows one other at most");
            follows[c.vehicle] = Some(c);
        }
        // Slots are given in scheduling order: count them.
        let mut slot_of = vec![NONE; vehicles.len()];
        let mut next_slot = 0u32;
        for (i, vehicle) in vehicles.iter().enumerate() {
            match follows[i] {
                // A vehicle whose leader never entered the loading never departs.
                Some(c) if slot_of[c.after] != NONE => {
                    slot_of[i] = sim.depart_after(vehicle, slot_of[c.after], c.wait, c.not_before);
                    next_slot += 1;
                }
                Some(_) => {}
                None if Duration::from_clock(vehicle.departure) < window => {
                    sim.depart(vehicle);
                    slot_of[i] = next_slot;
                    next_slot += 1;
                }
                None => {}
            }
        }
    }
    let mut completed = Vec::new();
    for _ in 0..n_steps {
        completed.extend(match rerouter.as_deref_mut() {
            Some(rr) => sim.step_rerouting(step, rr),
            None => sim.step(step),
        });
    }
    completed.retain(|t| Duration::from_clock(t.arrival()) <= window);
    if entry_bins {
        sim.record_unfinished(window.get());
    }
    let lock = sim.lock_report();
    let reroutes = sim.reroutes().to_vec();
    (completed, sim.take_link_bins_with_entry(), lock, reroutes)
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

    /// A 2-link ring (`ring0`: r0→r1, `ring1`: r1→r0) plus one outside approach
    /// into r0 (`entry`), **none tagged as a roundabout** — idea 1 (S157) is
    /// the tag-free rule, unlike S155's `must_give_way`. Returns the network
    /// and the three link indices `(ring0, ring1, entry)`.
    fn tiny_ring() -> (RoadNetwork, usize, usize, usize) {
        let mut b = RoadNetworkBuilder::new();
        b.add_node("r0", LonLat::new(4.8000, 45.700));
        b.add_node("r1", LonLat::new(4.8005, 45.700));
        b.add_node("e", LonLat::new(4.7995, 45.700));
        b.add_link("ring0", "r0", "r1", LinkSpec::new(RoadClass::Residential));
        b.add_link("ring1", "r1", "r0", LinkSpec::new(RoadClass::Residential));
        b.add_link("entry", "e", "r0", LinkSpec::new(RoadClass::Residential));
        let network = b
            .build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
            .expect("buildable");
        let (ring0, ring1, entry) =
            (link(&network, "ring0"), link(&network, "ring1"), link(&network, "entry"));
        assert!(!network.is_roundabout(ring0) && !network.is_roundabout(ring1));
        (network, ring0.index(), ring1.index(), entry.index())
    }

    /// **Property (idea 1, S157):** with `ring0`'s own front already parked
    /// waiting for room on `ring1`, and `ring1`'s front already parked waiting
    /// for room on `ring0` — a mutual block, the closed 2-link loop
    /// `ring0 → ring1 → ring0` — a vehicle asking for `ring0`'s one remaining
    /// sliver of room (exactly [`MIN_PART`]) is admitted when nothing is
    /// waiting on it (no loop yet), refused when granting it would be sealing
    /// that loop shut from the outside, and admitted again for a vehicle
    /// coming from `ring1` itself — a loop's own circulation is not what idea
    /// 1 refuses, only an outsider closing it. (The two `parked_on` pointers
    /// are set directly: **S157's exact scenario**, not derived from a full
    /// run, the same hand-built-fixture style `node_model.rs` uses for
    /// `solve_node` — see the module doc's "Any local lock" section for why
    /// this state is physically reachable, not merely convenient.)
    #[test]
    fn an_outsider_may_not_take_the_last_room_that_would_close_a_waiting_loop() {
        let (network, ring0, ring1, entry) = tiny_ring();
        let turns = TurnTable::build(&network, SignalDefaults::SHIPPED);
        let mut sim = LtmNetwork::new(&network, &turns);

        let storage = network.storage(LinkId::from_index(ring0)).get();
        assert!(storage > MIN_PART, "the fixture needs room for more than the last sliver");
        // ring0 is down to exactly its last admissible sliver of room.
        sim.curves[ring0].record_in(Pcu(storage - MIN_PART));
        let t = sim.now().get();

        assert!(
            sim.admissible(ring0, MIN_PART, t, Some(entry)).is_ok(),
            "no loop is waiting yet: the outsider is admitted normally"
        );

        // ring0's front is parked on ring1, and ring1's front is parked on
        // ring0: the loop ring0 -> ring1 -> ring0 is one grant away from closed.
        sim.parked_on[ring0] = link_u32(ring1);
        sim.parked_on[ring1] = link_u32(ring0);

        assert!(
            sim.admissible(ring0, MIN_PART, t, Some(entry)).is_err(),
            "an outsider may not seal the loop shut with ring0's last sliver of room"
        );
        assert!(
            sim.admissible(ring0, MIN_PART, t, Some(ring1)).is_ok(),
            "a vehicle circulating from within the loop itself still gets through"
        );
    }
}
