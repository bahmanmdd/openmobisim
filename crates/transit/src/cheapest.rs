//! The cheapest journey over a timetable, for a cost that is linear in a journey's parts.
//!
//! # Why beside the earliest arrival
//!
//! [`crate::raptor`] answers the journey of earliest arrival. A traveller who weighs waiting
//! above riding may prefer a journey that arrives later: a slow run boarded at once over a fast
//! one waited for. The earliest arrival per number of rides does not hold that journey, so the
//! search here carries the cost beside the arrival.
//!
//! # The cost
//!
//! A [`CostQuery`] is a [`Query`] with [`Weights`] and, per access and egress stop, the cost
//! already paid to be there and still to pay from there (the walks, which the query's maker
//! prices). A journey costs its access cost, then per second: `first_wait` from the access
//! second to the first boarding, `in_vehicle` on every ride, `transfer_walk` on every footpath
//! (its seconds as given) and `transfer_wait` from the end of a ride or a footpath to the next
//! boarding; `transfer` per boarding after the first; and its egress cost. Every weight is at
//! or above zero. What a journey may do is [`crate::raptor`]'s: the slack at every boarding, at
//! most one footpath between two rides, none before the first or after the last.
//!
//! [`Cheapest::by_rides`] answers, for each number of rides `k` that makes a journey cheaper
//! than every smaller number does, the cheapest journey of at most `k` rides, fewest rides
//! first. Of two journeys of equal cost: the earlier at the door, then the lower egress stop.
//!
//! # How
//!
//! Rounds as in RAPTOR, with a set of labels per stop in place of one arrival (Delling, Pajor
//! and Werneck 2015, section 5). A label is an arrival at the stop, the cost so far, the rides
//! taken and whether a footpath may still be walked from it (not from a label that walked, nor
//! from an access). Label `a` makes label `b` useless when it has no more rides, is there no
//! later, may walk if `b` may, and costs no more than `b` after waiting for `b`'s arrival: `a`
//! can then do whatever `b` does, for no more. An access label and a label that rode are not
//! compared (their next boarding is priced differently). Round `k` boards, from every label of
//! `k − 1` rides, every run that can be caught at each pattern through its stop, not the
//! earliest alone: a later run that is slower reaches a connection further on for less than
//! the earlier run followed by a wait at the stop, where waiting weighs more than riding. A
//! boarding is dropped once it costs as much as the cheapest journey found, and with it the
//! pattern's later runs from that label (they are waited for longer); so is any label (no
//! weight is negative, so neither can end cheaper).
//!
//! # Cost
//!
//! Per query, the labels made (about 48 bytes each) and the patterns through the stops
//! labelled, as RAPTOR's rounds. Until a first journey is found a label boards every later run
//! of a pattern, to the end of the service day, and each boarded run offers a label at every
//! later stop, most dropped at once by the stop's labels: many times RAPTOR's work per
//! pattern. A run asks it once per trip at iteration 0 (Delft, 65 959 trips with the earliest
//! searches: the whole run 6.4 s).

use openmobisim_types::ids::{EntityId, StopId};
use openmobisim_types::time::Second;
use openmobisim_types::units::Duration;
use rayon::prelude::*;

use crate::raptor::{Footpath, Journey, Query, Ride};
use crate::timetable::{Timetable, on_clock};

/// What a second of each part of a journey costs, and a transfer. No unit of its own: the
/// unit of the caller's cost.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Weights {
    /// Per second from the access second to the first boarding.
    pub first_wait: f64,
    /// Per second on a ride.
    pub in_vehicle: f64,
    /// Per second from the end of a ride, or of the footpath after it, to the next boarding.
    pub transfer_wait: f64,
    /// Per second of a footpath between two rides, its seconds as the timetable gives them.
    pub transfer_walk: f64,
    /// Per boarding after the first.
    pub transfer: f64,
}

/// One question: a [`Query`], what its parts cost, and the cost at each access stop (paid to be
/// there at its second) and from each egress stop (to the door), in the query's order.
#[derive(Clone, Debug, PartialEq)]
pub struct CostQuery {
    /// From where and when, to where, with how many rides at most.
    pub query: Query,
    /// The cost of a second of each part, and of a transfer.
    pub weights: Weights,
    /// Per access stop of the query, the cost paid on reaching it.
    pub access_cost: Vec<f64>,
    /// Per egress stop of the query, the cost from it to the door.
    pub egress_cost: Vec<f64>,
}

/// No label: the parent of an access label.
const NONE: u32 = u32::MAX;

/// How a label came to its stop.
#[derive(Clone, Copy, Debug)]
enum Via {
    /// The query's access.
    Access,
    /// A ride: the pattern, the run's rank in it, the positions boarded and left at.
    Ride { pattern: u32, rank: u32, board: u32, alight: u32 },
    /// A footpath from `from`, its seconds as given.
    Walk { from: StopId, seconds: Duration },
}

/// A label: at `stop` at second `arr` for `cost`, after `rides` rides.
#[derive(Clone, Copy, Debug)]
struct Label {
    stop: StopId,
    arr: u32,
    /// The cost up to `arr`.
    cost: f64,
    rides: u8,
    /// Whether a footpath may be walked from it: only from a label that rode here.
    may_walk: bool,
    /// The label it was boarded or walked from ([`NONE`] for an access).
    parent: u32,
    via: Via,
}

/// A run boarded while scanning a pattern: its rank, the cost at boarding less the ride's
/// weight times the boarding second (so the cost at a later stop is `offset + weight · arrival`),
/// the label boarded from and the position boarded at.
#[derive(Clone, Copy, Debug)]
struct Boarded {
    rank: u32,
    offset: f64,
    from: u32,
    board: u32,
}

/// The search over one timetable at one boarding slack, with its scratch. Reusable: every query
/// leaves the scratch clean.
#[derive(Clone, Debug)]
pub struct Cheapest<'a> {
    tt: &'a Timetable,
    slack: u32,
    /// Every label made by the query; `bags` and parents are indices into it.
    labels: Vec<Label>,
    /// Per stop, its labels that no other makes useless.
    bags: Vec<Vec<u32>>,
    touched: Vec<StopId>,
    /// The stops labelled in the round, to board from in the next.
    marked: Vec<bool>,
    marked_list: Vec<StopId>,
    /// Per pattern, the first position to scan from this round (`NONE` when not queued).
    queue_pos: Vec<u32>,
    queue: Vec<u32>,
    /// The labels of the round that rode: footpaths are walked from them.
    rode: Vec<u32>,
    /// The cost of the cheapest journey found so far.
    bound: f64,
}

impl<'a> Cheapest<'a> {
    /// A search over `timetable` with `boarding_slack` seconds at every boarding.
    ///
    /// # Panics
    ///
    /// If the slack is not a whole, non-negative number of seconds (the clock is whole seconds).
    #[must_use]
    pub fn new(timetable: &'a Timetable, boarding_slack: Duration) -> Self {
        let s = boarding_slack.get();
        assert!(s >= 0.0 && s.is_finite() && s.fract() == 0.0, "the slack {s} s is not whole");
        let n = timetable.stop_count() as usize;
        Self {
            tt: timetable,
            slack: on_clock(boarding_slack),
            labels: Vec::new(),
            bags: vec![Vec::new(); n],
            touched: Vec::new(),
            marked: vec![false; n],
            marked_list: Vec::new(),
            queue_pos: vec![NONE; timetable.pattern_count() as usize],
            queue: Vec::new(),
            rode: Vec::new(),
            bound: f64::INFINITY,
        }
    }

    /// The journeys of `q` by number of rides: for each count of rides that makes a journey
    /// cheaper than every smaller count does, the cheapest journey with at most that many
    /// rides, fewest rides first; empty when no journey reaches an egress stop.
    ///
    /// # Panics
    ///
    /// For a stop the timetable does not have; an egress walk that is negative or not finite;
    /// costs that are not one per access and egress stop; a weight or a cost that is negative
    /// or not finite.
    pub fn by_rides(&mut self, q: &CostQuery) -> Vec<Journey> {
        let w = q.weights;
        let all = [w.first_wait, w.in_vehicle, w.transfer_wait, w.transfer_walk, w.transfer];
        let fine = |x: &f64| x.is_finite() && *x >= 0.0;
        assert!(all.iter().all(fine), "a weight is negative or not finite: {w:?}");
        assert_eq!(q.access_cost.len(), q.query.access.len(), "a cost per access stop");
        assert_eq!(q.egress_cost.len(), q.query.egress.len(), "a cost per egress stop");
        assert!(q.access_cost.iter().chain(&q.egress_cost).all(fine), "a cost is negative");
        for &(s, walk) in &q.query.egress {
            assert!(walk.is_finite() && walk.get() >= 0.0, "the egress walk from stop {s}");
        }
        for (&(stop, at), &cost) in q.query.access.iter().zip(&q.access_cost) {
            let label = Label {
                stop,
                arr: at.get(),
                cost,
                rides: 0,
                may_walk: false,
                parent: NONE,
                via: Via::Access,
            };
            self.offer(label, &w);
        }
        let mut journeys = Vec::new();
        for k in 1..=q.query.max_rides {
            if self.marked_list.is_empty() {
                break;
            }
            self.queue_patterns();
            let queue = core::mem::take(&mut self.queue);
            for &p in &queue {
                let from = core::mem::replace(&mut self.queue_pos[p as usize], NONE);
                self.scan(k, p, from, &w);
            }
            self.queue = queue;
            self.queue.clear();
            self.walk(&w);
            if let Some((cost, label, egress)) = self.at_the_door(q) {
                if cost < self.bound - 1e-9 {
                    self.bound = cost;
                    journeys.push(self.journey(q, label, egress));
                }
            }
        }
        self.clean();
        journeys
    }

    /// Whether label `a` makes label `b` useless at their stop (see the module documentation).
    fn covers(a: &Label, b: &Label, w: &Weights) -> bool {
        if (a.rides == 0) != (b.rides == 0)
            || a.rides > b.rides
            || a.arr > b.arr
            || (b.may_walk && !a.may_walk)
        {
            return false;
        }
        let wait = if a.rides == 0 { w.first_wait } else { w.transfer_wait };
        a.cost + wait * f64::from(b.arr - a.arr) <= b.cost
    }

    /// Add `label` to its stop's bag unless a label there makes it useless or it cannot end
    /// cheaper than the cheapest journey found; drop the labels it makes useless.
    fn offer(&mut self, label: Label, w: &Weights) {
        if label.cost >= self.bound {
            return;
        }
        let i = label.stop.index();
        if self.bags[i].iter().any(|&l| Self::covers(&self.labels[l as usize], &label, w)) {
            return;
        }
        let labels = &self.labels;
        self.bags[i].retain(|&l| !Self::covers(&label, &labels[l as usize], w));
        if self.bags[i].is_empty() {
            // Once per time the bag fills from empty: cleaning a stop twice is harmless.
            self.touched.push(label.stop);
        }
        let at = u32::try_from(self.labels.len()).expect("fewer than 2^32 labels");
        self.labels.push(label);
        self.bags[i].push(at);
        if matches!(label.via, Via::Ride { .. }) {
            self.rode.push(at);
        }
        if !self.marked[i] {
            self.marked[i] = true;
            self.marked_list.push(label.stop);
        }
    }

    /// Queue the patterns through the stops labelled last round, each from its first such
    /// position, in pattern order; unmark the stops.
    fn queue_patterns(&mut self) {
        for &s in &self.marked_list {
            for &(p, pos) in self.tt.patterns_through(s) {
                let q = &mut self.queue_pos[p as usize];
                if *q == NONE {
                    self.queue.push(p);
                }
                *q = (*q).min(pos);
            }
            self.marked[s.index()] = false;
        }
        self.marked_list.clear();
        self.queue.sort_unstable();
    }

    /// Scan pattern `p` in round `k` from position `from`: leave every boarded run at each
    /// stop, and board from each of the stop's labels of `k − 1` rides.
    fn scan(&mut self, k: u8, p: u32, from: u32, w: &Weights) {
        let tt = self.tt;
        let pattern = tt.pattern(p);
        let mut boarded: Vec<Boarded> = Vec::new();
        for pos in from..pattern.len {
            let stop = tt.stop_at(p, pos);
            for b in &boarded {
                let arr = tt.arrival_at(p, b.rank, pos);
                let via = Via::Ride { pattern: p, rank: b.rank, board: b.board, alight: pos };
                let cost = b.offset + w.in_vehicle * f64::from(arr);
                let label =
                    Label { stop, arr, cost, rides: k, may_walk: true, parent: b.from, via };
                self.offer(label, w);
            }
            if pos + 1 == pattern.len {
                break;
            }
            let departures = tt.departures_at(p, pos);
            for n in 0..self.bags[stop.index()].len() {
                let at = self.bags[stop.index()][n];
                let label = self.labels[at as usize];
                if label.rides != k - 1 {
                    continue;
                }
                let (wait, transfer) = if label.rides == 0 {
                    (w.first_wait, 0.0)
                } else {
                    (w.transfer_wait, w.transfer)
                };
                let ready = label.arr.saturating_add(self.slack);
                let first = departures.partition_point(|&d| d < ready);
                for (rank, &dep) in departures.iter().enumerate().skip(first) {
                    let cost = label.cost + wait * f64::from(dep - label.arr) + transfer;
                    if cost >= self.bound {
                        break;
                    }
                    let offset = cost - w.in_vehicle * f64::from(dep);
                    let rank = u32::try_from(rank).expect("a rank");
                    // The same run boarded for less makes this boarding useless, and the
                    // other way round; runs are not compared with one another here (their
                    // labels are, at every stop they reach).
                    match boarded.iter_mut().find(|b| b.rank == rank) {
                        Some(b) if b.offset <= offset => {}
                        Some(b) => *b = Boarded { rank, offset, from: at, board: pos },
                        None => boarded.push(Boarded { rank, offset, from: at, board: pos }),
                    }
                }
            }
        }
    }

    /// Walk the footpaths from the labels that rode this round and are still in their bags.
    fn walk(&mut self, w: &Weights) {
        let rode = core::mem::take(&mut self.rode);
        for &at in &rode {
            let label = self.labels[at as usize];
            if !self.bags[label.stop.index()].contains(&at) {
                continue;
            }
            for path in self.tt.walks(label.stop) {
                // The footpath's seconds as given are walked; the rest of the whole second the
                // clock rounds it up to is waited.
                let waited = f64::from(path.clock) - path.seconds.get();
                let walked = Label {
                    stop: path.to,
                    arr: label.arr.saturating_add(path.clock),
                    cost: label.cost
                        + w.transfer_walk * path.seconds.get()
                        + w.transfer_wait * waited,
                    rides: label.rides,
                    may_walk: false,
                    parent: at,
                    via: Via::Walk { from: label.stop, seconds: path.seconds },
                };
                self.offer(walked, w);
            }
        }
        self.rode = rode;
        self.rode.clear();
    }

    /// The cheapest end among the labels that rode to an egress stop: its cost at the door, the
    /// label and the place of its egress stop in the query; of equal costs the earlier at the
    /// door, then the lower egress stop, then the label made first.
    fn at_the_door(&self, q: &CostQuery) -> Option<(f64, u32, usize)> {
        let mut best: Option<(f64, u32, StopId, u32, usize)> = None;
        for (e, (&(stop, walk), &cost)) in q.query.egress.iter().zip(&q.egress_cost).enumerate() {
            for &at in &self.bags[stop.index()] {
                let label = &self.labels[at as usize];
                if !label.may_walk {
                    continue;
                }
                let c = (label.cost + cost, label.arr.saturating_add(on_clock(walk)), stop, at, e);
                let better = best.is_none_or(|b| {
                    c.0.total_cmp(&b.0).then((c.1, c.2, c.3).cmp(&(b.1, b.2, b.3))).is_lt()
                });
                if better {
                    best = Some(c);
                }
            }
        }
        best.map(|b| (b.0, b.3, b.4))
    }

    /// The journey that ends with label `last` at egress stop `egress` of the query.
    fn journey(&self, q: &CostQuery, last: u32, egress: usize) -> Journey {
        let tt = self.tt;
        let (stop, walk) = q.query.egress[egress];
        let arrival = Second(self.labels[last as usize].arr.saturating_add(on_clock(walk)));
        let (mut rides, mut footpaths) = (Vec::new(), Vec::new());
        let mut at = last;
        let access = loop {
            let label = self.labels[at as usize];
            let Via::Ride { pattern, rank, board, alight } = label.via else {
                unreachable!("a journey is read back ride by ride");
            };
            rides.push(Ride {
                run: tt.run_at(pattern, rank),
                board: (tt.stop_at(pattern, board), Second(tt.departure_at(pattern, rank, board))),
                alight: (label.stop, Second(tt.arrival_at(pattern, rank, alight))),
            });
            let from = self.labels[label.parent as usize];
            match from.via {
                Via::Access => break (from.stop, Second(from.arr)),
                Via::Ride { .. } => {
                    footpaths.push(None);
                    at = label.parent;
                }
                Via::Walk { from: left, seconds } => {
                    footpaths.push(Some(Footpath { from: left, to: from.stop, seconds }));
                    at = from.parent;
                }
            }
        };
        rides.reverse();
        footpaths.reverse();
        Journey { rides, footpaths, access, egress: (stop, walk), arrival }
    }

    /// Reset what this query touched, and nothing else.
    fn clean(&mut self) {
        for s in self.touched.drain(..) {
            self.bags[s.index()].clear();
        }
        for &s in &self.marked_list {
            self.marked[s.index()] = false;
        }
        self.marked_list.clear();
        self.labels.clear();
        self.rode.clear();
        self.bound = f64::INFINITY;
    }
}

/// [`Cheapest::by_rides`] for every query, in parallel (rayon), one scratch per thread. Answer
/// `i` is query `i`'s, and each depends on nothing but its query: the vector is the same, bit
/// for bit, at any number of threads.
///
/// # Panics
///
/// As [`Cheapest::new`] and [`Cheapest::by_rides`].
#[must_use]
pub fn cheapest_by_rides_many(
    timetable: &Timetable,
    boarding_slack: Duration,
    queries: &[CostQuery],
) -> Vec<Vec<Journey>> {
    let first = Cheapest::new(timetable, boarding_slack);
    queries.par_iter().map_init(|| first.clone(), |search, q| search.by_rides(q)).collect()
}
