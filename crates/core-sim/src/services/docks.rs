//! Docked bikes on the fleet engine: pickups, returns, redirections and overflow, with the bikes
//! counted, and the docks' counts kept bin by bin ([`DockBins`]).
//!
//! **A leg** asks for a bike at its planned dock at its request second (the engine's
//! `Request`). With at least as many bikes as the people it stands for (its weight `w`), it
//! takes `w` bikes and rides: the bikes are docked (`Return`) [`DockDefaults::pickup_s`] plus
//! the ride later, at its planned return dock. Returns come before requests at one second, so a
//! bike returned is a bike found.
//!
//! **An empty dock**: the traveller walks to the nearest neighbour of the dock they stand at
//! that they have not tried, other than the planned one and within
//! [`DockDefaults::redirect_max_m`] of it as the crow flies, and asks again there (a further
//! `Request`, after the walk); a neighbour is tried whether or not it has bikes, as the
//! traveller cannot know. After [`DockDefaults::redirect_max`] docks tried after the planned
//! one, or when no such neighbour is left, the pickup **fails** and the traveller walks from the
//! last dock tried to the destination.
//!
//! **A full return dock** (fewer free slots than `w`): the bike is ridden on to the nearest
//! neighbour not yet tried with room for `w` (a further `Return`, after the ride); with none,
//! it is docked anyway, above capacity, and the **overflow** counted (a bike must be returned).
//! Once docked, the traveller arrives [`DockDefaults::return_s`] plus the walk to the
//! destination later.
//!
//! **An operator's van** ([`super::decisions::Decision::Move`]) takes bikes from a dock at once
//! and, on arriving, docks them into the free slots of the dock it was sent to, the rest into
//! that dock's neighbours with room, nearest first, any left above capacity (counted as
//! overflow).
//!
//! **Cost:** per pickup or return one event; per redirection a look along a short list; per
//! dock and bin 80 bytes, and 40 bytes for the counts since the last policy step.

use openmobisim_core_graph::geometry::ground_distance_metres;

use super::engine::{EventKind, Leg, ServiceLegs};
use super::outcome::{LegOutcome, LegStatus};
use super::{DockDefaults, Docks};

/// How one docked service's docks fared over a day, bin by bin from the window's start.
///
/// Every per-bin field is indexed `dock * bins + bin`.
#[derive(Clone, Debug, PartialEq)]
pub struct DockBins {
    /// The bin length, in seconds.
    pub bin_s: u32,
    /// How many bins.
    pub bins: usize,
    /// The window's start, in seconds from midnight: bin 0 starts here.
    pub start_s: f64,
    /// The window's length, in seconds; the last bin ends with it.
    pub window_s: f64,
    /// The mean bikes docked over the bin.
    pub bikes_mean: Vec<f64>,
    /// The fewest bikes docked at once in the bin.
    pub bikes_min: Vec<f64>,
    /// Seconds of the bin the dock was empty.
    pub empty_s: Vec<f64>,
    /// Seconds of the bin the dock was full (bikes at or above slots).
    pub full_s: Vec<f64>,
    /// Bikes taken (people).
    pub pickups: Vec<f64>,
    /// Bikes returned (people).
    pub returns: Vec<f64>,
    /// Pickups that failed here, the last dock tried (people).
    pub failed: Vec<f64>,
    /// Travellers who found the dock empty or full and went on to another (people).
    pub redirected: Vec<f64>,
    /// Travellers sent on to this dock from another that was empty or full (people), counted
    /// in the bin they were sent.
    pub redirected_in: Vec<f64>,
    /// Of [`Self::redirected`], those who found the dock full when returning (people); the
    /// rest found it empty when picking up.
    pub redirected_no_slot: Vec<f64>,
    /// Bikes docked above capacity, every dock near being full too (people).
    pub over_capacity: Vec<f64>,
    /// The bikes docked when the bin ended (at the window's end for the last bin).
    pub bikes_end: Vec<f64>,
    /// Bikes an operator's van took away.
    pub moved_out: Vec<f64>,
    /// Bikes an operator's van brought.
    pub moved_in: Vec<f64>,
}

impl DockBins {
    fn new(docks: usize, defaults: &DockDefaults, window: (f64, f64)) -> Self {
        let bin_s = defaults.bin_s.max(1.0);
        let window_s = (window.1 - window.0).max(0.0);
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a count of bins"
        )]
        let bins = ((window_s / bin_s).ceil() as usize).max(1);
        let cells = docks * bins;
        Self {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "seconds")]
            bin_s: bin_s as u32,
            bins,
            start_s: window.0,
            window_s,
            bikes_mean: vec![0.0; cells],
            bikes_min: vec![f64::INFINITY; cells],
            empty_s: vec![0.0; cells],
            full_s: vec![0.0; cells],
            pickups: vec![0.0; cells],
            returns: vec![0.0; cells],
            failed: vec![0.0; cells],
            redirected: vec![0.0; cells],
            redirected_in: vec![0.0; cells],
            redirected_no_slot: vec![0.0; cells],
            over_capacity: vec![0.0; cells],
            bikes_end: vec![0.0; cells],
            moved_out: vec![0.0; cells],
            moved_in: vec![0.0; cells],
        }
    }

    /// The length of bin `b` inside the window, in seconds (never 0).
    #[must_use]
    pub fn bin_len(&self, b: usize) -> f64 {
        let whole = f64::from(self.bin_s);
        if b + 1 < self.bins {
            return whole;
        }
        #[allow(clippy::cast_precision_loss, reason = "a count of bins")]
        let left = self.window_s - whole * (self.bins - 1) as f64;
        if left > 0.0 { left.min(whole) } else { whole }
    }

    /// The bin of second `t`, in seconds from midnight (clamped to the window's bins).
    #[must_use]
    pub fn bin_of(&self, t: f64) -> usize {
        self.bin_of_offset(t - self.start_s)
    }

    fn bin_of_offset(&self, s: f64) -> usize {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a bin")]
        let b = (s.max(0.0) / f64::from(self.bin_s)) as usize;
        b.min(self.bins - 1)
    }
}

/// What happened, in a day's log of events ([`DockEvent`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DockEventKind {
    /// A traveller reached a dock to take a bike (the planned one, or one tried after it).
    ArrivedForBike,
    /// They took a bike there.
    TookBike,
    /// They found it without a bike for them.
    FoundNoBike,
    /// They walk on to another dock ([`DockEvent::next_dock`]).
    WalkingToNextDock,
    /// No other dock left to try: they walk from here to their destination.
    GaveUpWalkingToDestination,
    /// A traveller reached a dock to return their bike (the planned one, or one tried after).
    ArrivedToReturn,
    /// They docked their bike there.
    ReturnedBike,
    /// They found it without a free slot for them.
    FoundNoFreeSlot,
    /// They ride on to another dock ([`DockEvent::next_dock`]).
    RidingToNextDock,
    /// Every dock near being full, they docked their bike above capacity here.
    LeftBikeOverCapacity,
    /// They reached the end of the leg (the trip's destination, or for a shared bike to
    /// transit the dock at the stop's hub), riding or on foot.
    ReachedDestination,
    /// An operator's van took bikes from the dock.
    VanTookBikes,
    /// An operator's van brought bikes to the dock.
    VanBroughtBikes,
}

impl DockEventKind {
    /// The stable snake_case name, as `bike_share_events.parquet` writes it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            DockEventKind::ArrivedForBike => "arrived_for_bike",
            DockEventKind::TookBike => "took_bike",
            DockEventKind::FoundNoBike => "found_no_bike",
            DockEventKind::WalkingToNextDock => "walking_to_next_dock",
            DockEventKind::GaveUpWalkingToDestination => "gave_up_walking_to_destination",
            DockEventKind::ArrivedToReturn => "arrived_to_return",
            DockEventKind::ReturnedBike => "returned_bike",
            DockEventKind::FoundNoFreeSlot => "found_no_free_slot",
            DockEventKind::RidingToNextDock => "riding_to_next_dock",
            DockEventKind::LeftBikeOverCapacity => "left_bike_over_capacity",
            DockEventKind::ReachedDestination => "reached_destination",
            DockEventKind::VanTookBikes => "van_took_bikes",
            DockEventKind::VanBroughtBikes => "van_brought_bikes",
        }
    }
}

/// One event of a docked service's day, as it happened. [`NONE`] marks a field that does not
/// apply (no leg for a van's event; no dock for an arrival at the destination; no next dock).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DockEvent {
    /// When, in seconds from midnight.
    pub t: f64,
    /// The leg, by its index in the day.
    pub leg: u32,
    /// What happened.
    pub kind: DockEventKind,
    /// Where: the dock.
    pub dock: u32,
    /// The bikes docked there just after.
    pub bikes_after: u32,
    /// The slots free there just after (0 when bikes stand above capacity).
    pub free_slots_after: u32,
    /// The dock walked or ridden on to.
    pub next_dock: u32,
    /// How far, in metres as the crow flies (`NaN` without a next dock); a van's event: the
    /// bikes it took or brought.
    pub to_next_m: f64,
}

/// A field of a [`DockEvent`] that does not apply.
pub const NONE: u32 = u32::MAX;

/// Per dock, the counts since the last policy step (people).
#[derive(Clone, Debug, Default)]
pub(crate) struct SinceStep {
    pub(crate) pickups: Vec<f64>,
    pub(crate) returns: Vec<f64>,
    pub(crate) failed: Vec<f64>,
    pub(crate) redirected_in: Vec<f64>,
    pub(crate) redirected_out: Vec<f64>,
}

impl SinceStep {
    fn new(n: usize) -> Self {
        Self {
            pickups: vec![0.0; n],
            returns: vec![0.0; n],
            failed: vec![0.0; n],
            redirected_in: vec![0.0; n],
            redirected_out: vec![0.0; n],
        }
    }

    pub(crate) fn clear(&mut self) {
        for v in [
            &mut self.pickups,
            &mut self.returns,
            &mut self.failed,
            &mut self.redirected_in,
            &mut self.redirected_out,
        ] {
            v.fill(0.0);
        }
    }
}

/// One docked service's running counts on the engine.
#[derive(Clone, Debug)]
pub(crate) struct DockRun {
    pub(crate) bins: DockBins,
    /// Per dock, the second (from the window's start) its counts are kept up to.
    clock: Vec<f64>,
    /// The most bikes docked above capacity at once, over every dock.
    pub(crate) overflow_max: f64,
    /// Per dock, the counts since the last policy step.
    pub(crate) since: SinceStep,
    /// Per dock, the bikes in operators' vans on their way to it.
    pub(crate) incoming: Vec<u32>,
    /// The day's events in the order handled (`None`: not kept).
    pub(crate) log: Option<Vec<DockEvent>>,
}

/// A docked service's counts the handlers change: per dock its bikes, and the bikes ridden.
pub(crate) struct Fleet<'a> {
    pub(crate) bikes: &'a mut [u32],
    pub(crate) riding: &'a mut u64,
}

/// What a handler asks the engine to schedule next for the leg: at a second, of a kind, at a
/// dock.
pub(crate) type Next = Option<(f64, EventKind, u32)>;

/// What a handler is given about the leg it handles.
pub(crate) struct LegCtx<'a> {
    pub(crate) docks: &'a Docks,
    pub(crate) defaults: &'a DockDefaults,
    pub(crate) legs: &'a dyn ServiceLegs,
    /// The leg, and its index in the day.
    pub(crate) leg: &'a Leg,
    pub(crate) index: usize,
    /// The docks tried since the leg's last pickup.
    pub(crate) tried: &'a mut Vec<u32>,
    pub(crate) out: &'a mut LegOutcome,
}

impl DockRun {
    pub(crate) fn new(docks: &Docks, defaults: &DockDefaults, window: (f64, f64)) -> Self {
        Self {
            bins: DockBins::new(docks.len(), defaults, window),
            clock: vec![0.0; docks.len()],
            overflow_max: 0.0,
            since: SinceStep::new(docks.len()),
            incoming: vec![0; docks.len()],
            log: defaults.log_events.then(Vec::new),
        }
    }

    /// Note an event at dock `k` (or [`NONE`]) with its counts just after.
    #[allow(clippy::too_many_arguments, reason = "an event's fields, named")]
    fn note(
        &mut self,
        docks: &Docks,
        bikes: &[u32],
        (t, leg, kind): (f64, u32, DockEventKind),
        k: u32,
        next: u32,
        to_next_m: f64,
    ) {
        let Some(log) = self.log.as_mut() else { return };
        let (b, free) = if k == NONE {
            (NONE, NONE)
        } else {
            let b = bikes[k as usize];
            (b, docks.capacity[k as usize].saturating_sub(b))
        };
        log.push(DockEvent {
            t,
            leg,
            kind,
            dock: k,
            bikes_after: b,
            free_slots_after: free,
            next_dock: next,
            to_next_m,
        });
    }

    /// An operator's van takes `n` bikes (at most those docked) from dock `k` at second `t`.
    pub(crate) fn van_out(&mut self, docks: &Docks, bikes: &mut [u32], k: usize, n: u32, t: f64) {
        let cell = self.touch(docks, bikes, k, t);
        bikes[k] -= n;
        self.bins.moved_out[cell] += f64::from(n);
        self.bins.bikes_min[cell] = self.bins.bikes_min[cell].min(f64::from(bikes[k]));
        let k32 = u32::try_from(k).expect("docks fit u32");
        let what = (t, NONE, DockEventKind::VanTookBikes);
        self.note(docks, bikes, what, k32, NONE, f64::from(n));
    }

    /// An operator's van docks `n` bikes at second `t`: into dock `to`'s free slots, the rest
    /// into its neighbours with room, nearest first, any left at `to` above capacity.
    pub(crate) fn van_in(&mut self, docks: &Docks, bikes: &mut [u32], to: u32, n: u32, t: f64) {
        let mut left = n;
        let targets = std::iter::once(to).chain(docks.neighbours(to).iter().map(|x| x.0));
        for k in targets {
            let k = k as usize;
            let room = docks.capacity[k].saturating_sub(bikes[k]).min(left);
            if room > 0 {
                let cell = self.touch(docks, bikes, k, t);
                bikes[k] += room;
                self.bins.moved_in[cell] += f64::from(room);
                left -= room;
                let k32 = u32::try_from(k).expect("docks fit u32");
                let what = (t, NONE, DockEventKind::VanBroughtBikes);
                self.note(docks, bikes, what, k32, NONE, f64::from(room));
            }
            if left == 0 {
                break;
            }
        }
        if left > 0 {
            let k = to as usize;
            let cell = self.touch(docks, bikes, k, t);
            bikes[k] += left;
            self.bins.moved_in[cell] += f64::from(left);
            self.note(
                docks,
                bikes,
                (t, NONE, DockEventKind::VanBroughtBikes),
                to,
                NONE,
                f64::from(left),
            );
            self.overflow_max =
                self.overflow_max.max(f64::from(bikes[k]) - f64::from(docks.capacity[k]));
        }
    }

    /// Close dock `k`'s interval from its clock to `to` (seconds from midnight) at its count.
    fn advance(&mut self, k: usize, bikes: u32, capacity: u32, to: f64) {
        let out = &mut self.bins;
        let to = (to - out.start_s).clamp(0.0, out.window_s);
        let bin_s = f64::from(out.bin_s);
        let bikes = f64::from(bikes);
        let cap = f64::from(capacity);
        let mut t = self.clock[k];
        while t < to {
            let b = out.bin_of_offset(t);
            #[allow(clippy::cast_precision_loss, reason = "a count of bins")]
            let end = (((b + 1) as f64) * bin_s).min(to);
            let span = end - t;
            if span <= 0.0 {
                break;
            }
            let cell = k * out.bins + b;
            out.bikes_mean[cell] += bikes * span / out.bin_len(b);
            out.bikes_min[cell] = out.bikes_min[cell].min(bikes);
            out.bikes_end[cell] = bikes;
            if bikes <= 0.0 {
                out.empty_s[cell] += span;
            }
            if bikes >= cap {
                out.full_s[cell] += span;
            }
            t = end;
        }
        self.clock[k] = self.clock[k].max(to);
    }

    /// Bring dock `k`'s counts up to second `t` and note its count there; its cell.
    fn touch(&mut self, docks: &Docks, bikes: &[u32], k: usize, t: f64) -> usize {
        self.advance(k, bikes[k], docks.capacity[k], t);
        let cell = k * self.bins.bins + self.bins.bin_of(t);
        self.bins.bikes_min[cell] = self.bins.bikes_min[cell].min(f64::from(bikes[k]));
        cell
    }

    /// Close every dock's counts at second `to` (seconds from midnight); a bin never reached
    /// keeps a minimum of 0.
    pub(crate) fn finish(&mut self, docks: &Docks, bikes: &[u32], to: f64) {
        for (k, (&b, &cap)) in bikes.iter().zip(&docks.capacity).enumerate() {
            self.advance(k, b, cap, to);
        }
        for v in &mut self.bins.bikes_min {
            if !v.is_finite() {
                *v = 0.0;
            }
        }
    }

    /// A leg asks for a bike at dock `dock` at second `t`.
    pub(crate) fn pickup(
        &mut self,
        fleet: &mut Fleet<'_>,
        c: &mut LegCtx<'_>,
        dock: u32,
        t: f64,
    ) -> Next {
        let k = dock as usize;
        let leg = c.leg;
        let w = leg.weight;
        let wf = f64::from(w);
        let cell = self.touch(c.docks, fleet.bikes, k, t);
        let me = u32::try_from(c.index).expect("legs fit u32");
        self.note(
            c.docks,
            fleet.bikes,
            (t, me, DockEventKind::ArrivedForBike),
            dock,
            NONE,
            f64::NAN,
        );
        if fleet.bikes[k] >= w {
            fleet.bikes[k] -= w;
            *fleet.riding += u64::from(w);
            self.bins.pickups[cell] += wf;
            self.since.pickups[k] += wf;
            self.bins.bikes_min[cell] = self.bins.bikes_min[cell].min(f64::from(fleet.bikes[k]));
            self.note(c.docks, fleet.bikes, (t, me, DockEventKind::TookBike), dock, NONE, f64::NAN);
            c.out.status = LegStatus::Riding;
            c.out.picked = Some(dock);
            c.out.picked_s = t;
            let ride = if dock == leg.pickup_dock {
                leg.ride_s
            } else {
                c.legs.ride_s(leg.service, dock, leg.return_dock)
            };
            c.tried.clear();
            return Some((t + c.defaults.pickup_s + ride, EventKind::Return, leg.return_dock));
        }
        c.tried.push(dock);
        c.out.failed_for_bike += 1;
        self.note(c.docks, fleet.bikes, (t, me, DockEventKind::FoundNoBike), dock, NONE, f64::NAN);
        // The nearest untried dock from here, near the planned one.
        let planned = c.docks.position[leg.pickup_dock as usize];
        let next = if c.tried.len() <= c.defaults.redirect_count() {
            c.docks.neighbours(dock).iter().copied().find(|&(o, _)| {
                !c.tried.contains(&o)
                    && o != leg.pickup_dock
                    && ground_distance_metres(planned, c.docks.position[o as usize])
                        <= c.defaults.redirect_max_m
            })
        } else {
            None
        };
        // Go on only if a bike there is worth it: from that dock, walking on to the destination
        // must be slower than taking a bike there, riding to the return dock and walking from it
        // (so a traveller never goes on to their return dock itself, to ride 0 m). Otherwise
        // they give up here and walk to their destination. Cost: one walk search per dock tried
        // after the first, the ride kept per dock pair.
        let next = next.filter(|&(o, _)| {
            let walk_on = c.legs.walk_to_destination_s(c.index, o);
            let ride = c.legs.ride_s(leg.service, o, leg.return_dock);
            let by_bike = c.defaults.pickup_s + ride + c.defaults.return_s + leg.egress_s;
            walk_on > by_bike || walk_on.is_nan()
        });
        if let Some((o, walk)) = next {
            self.bins.redirected[cell] += wf;
            let to = o as usize * self.bins.bins + self.bins.bin_of(t);
            self.bins.redirected_in[to] += wf;
            self.since.redirected_out[k] += wf;
            self.since.redirected_in[o as usize] += wf;
            c.out.extra_walk_s += walk;
            c.out.redirections += 1;
            let m = ground_distance_metres(c.docks.position[k], c.docks.position[o as usize]);
            c.out.extra_walk_m += m;
            let what = (t, me, DockEventKind::WalkingToNextDock);
            self.note(c.docks, fleet.bikes, what, dock, o, m);
            return Some((t + walk, EventKind::Request, o));
        }
        self.bins.failed[cell] += wf;
        self.since.failed[k] += wf;
        c.out.status = LegStatus::Failed;
        c.out.picked_s = t;
        c.out.arrived_s = t + c.legs.walk_to_destination_s(c.index, dock);
        let what = (t, me, DockEventKind::GaveUpWalkingToDestination);
        self.note(c.docks, fleet.bikes, what, dock, NONE, f64::NAN);
        let what = (c.out.arrived_s, me, DockEventKind::ReachedDestination);
        self.note(c.docks, fleet.bikes, what, NONE, NONE, f64::NAN);
        c.tried.clear();
        None
    }

    /// A leg's bikes reach dock `dock` at second `t` to be docked.
    pub(crate) fn ret(
        &mut self,
        fleet: &mut Fleet<'_>,
        c: &mut LegCtx<'_>,
        dock: u32,
        t: f64,
    ) -> Next {
        let k = dock as usize;
        let leg = c.leg;
        let w = leg.weight;
        let wf = f64::from(w);
        let cell = self.touch(c.docks, fleet.bikes, k, t);
        let cap = c.docks.capacity[k];
        let me = u32::try_from(c.index).expect("legs fit u32");
        self.note(
            c.docks,
            fleet.bikes,
            (t, me, DockEventKind::ArrivedToReturn),
            dock,
            NONE,
            f64::NAN,
        );
        let room = u64::from(fleet.bikes[k]) + u64::from(w) <= u64::from(cap);
        if !room {
            c.out.failed_for_slot += 1;
            self.note(
                c.docks,
                fleet.bikes,
                (t, me, DockEventKind::FoundNoFreeSlot),
                dock,
                NONE,
                f64::NAN,
            );
            c.tried.push(dock);
            let bikes = &*fleet.bikes;
            let next = c.docks.neighbours(dock).iter().copied().find(|&(o, _)| {
                !c.tried.contains(&o)
                    && u64::from(bikes[o as usize]) + u64::from(w)
                        <= u64::from(c.docks.capacity[o as usize])
            });
            if let Some((o, _)) = next {
                self.bins.redirected[cell] += wf;
                let to = o as usize * self.bins.bins + self.bins.bin_of(t);
                self.bins.redirected_in[to] += wf;
                self.since.redirected_out[k] += wf;
                self.since.redirected_in[o as usize] += wf;
                self.bins.redirected_no_slot[cell] += wf;
                c.out.redirections += 1;
                let m = ground_distance_metres(c.docks.position[k], c.docks.position[o as usize]);
                let what = (t, me, DockEventKind::RidingToNextDock);
                self.note(c.docks, fleet.bikes, what, dock, o, m);
                let ride = c.legs.ride_s(leg.service, dock, o);
                return Some((t + ride, EventKind::Return, o));
            }
        }
        fleet.bikes[k] += w;
        *fleet.riding -= u64::from(w);
        self.bins.returns[cell] += wf;
        self.since.returns[k] += wf;
        self.overflow_max = self.overflow_max.max(f64::from(fleet.bikes[k]) - f64::from(cap));
        let kind = if room {
            DockEventKind::ReturnedBike
        } else {
            self.bins.over_capacity[cell] += wf;
            DockEventKind::LeftBikeOverCapacity
        };
        self.note(c.docks, fleet.bikes, (t, me, kind), dock, NONE, f64::NAN);
        let egress = if dock == leg.return_dock {
            leg.egress_s
        } else {
            c.legs.walk_to_destination_s(c.index, dock)
        };
        c.out.status = LegStatus::Served;
        c.out.returned = Some(dock);
        c.out.returned_s = t;
        c.out.overflow = !room;
        c.out.arrived_s = t + c.defaults.return_s + egress;
        let what = (c.out.arrived_s, me, DockEventKind::ReachedDestination);
        self.note(c.docks, fleet.bikes, what, NONE, NONE, f64::NAN);
        c.tried.clear();
        None
    }
}
