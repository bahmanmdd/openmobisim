//! The earliest-arrival journey over a timetable: RAPTOR, round-based public transit routing
//! (Delling, Pajor and Werneck 2015).
//!
//! # What a journey is
//!
//! A [`Query`] gives the access stops, each with the second the traveller is there, the egress
//! stops, each with the walk from it to the door, and the most rides allowed. A journey starts
//! at an access stop at its second; boards a run at a stop only if it is there `slack` seconds
//! before the run leaves (at every boarding, the first included); rides it to a later call;
//! between two rides walks at most one footpath (its seconds added, then the slack applies at
//! the next boarding); takes at most `max_rides` rides; and ends at an egress stop. Its arrival
//! is the second it alights there plus the egress walk, put on the clock rounded up to the whole
//! second. No footpath is walked before the first ride or after the last: the access and egress
//! lists already carry those walks. A journey takes at least one ride: walking all the way is
//! another mode, so a query no ride answers gets `None`.
//!
//! The answer is the journey of **earliest arrival**; among those, the one of **fewest rides**;
//! among those, the one whose **last ride is on the run of lowest id**; among those, the one
//! alighting at the egress stop of lowest id, boarding the last run at the first stop where it
//! can be caught. What comes before the last ride is the earliest way to be at its boarding
//! stop with the rides left, by the fewest rides.
//!
//! [`Raptor::by_rides`] answers, for each number of rides `k` that reaches the door earlier than
//! every smaller number, the journey the same rule gives among those of at most `k` rides, fewest
//! rides first: round `k`'s door arrival is the earliest with at most `k` rides, so the rounds
//! that improve on it are those numbers, and each journey is read back from its round's labels.
//!
//! # How
//!
//! Round `k` finds the earliest arrival at every stop with at most `k` rides. It scans every
//! pattern through a stop improved in round `k − 1`, from the first such stop on, holding the
//! earliest run a traveller can catch there (a binary search over the pattern's departures at
//! the stop, which never fall from one run to the next) and switching to an earlier one wherever
//! the traveller is in time for it. Then it walks the footpaths once, from the stops a ride of
//! this round improved. Each stop keeps its best arrival by a ride apart from its best arrival
//! overall: a walk starts only from a ride, never from a walk, so footpaths need not be closed
//! under composition. A label is kept only if earlier than every earlier one at the stop (local
//! pruning) and than the best arrival at the door found in an earlier round (target pruning):
//! neither can drop a journey of earlier arrival, or of equal arrival and fewer rides. The first
//! round that reaches the earliest door arrival gives the fewest rides. The last ride is then
//! chosen by the tie rule among every run that alights at an egress stop in time and can be
//! caught from the labels of the round before, so the journey depends on the timetable and the
//! query alone.
//!
//! # Cost
//!
//! Per round, the patterns through the stops improved, each scanned once; per query, a reset
//! of only what it touched. The scratch is 28 bytes per stop per round, grown to the most
//! rides any query asked for and reused; [`earliest_many`] keeps one per thread.

use core::mem;

use openmobisim_types::ids::{EntityId, StopId, TransitRunId};
use openmobisim_types::time::Second;
use openmobisim_types::units::Duration;
use rayon::prelude::*;

use crate::timetable::{Timetable, on_clock};

/// No time: a stop not reached.
const INF: u32 = u32::MAX;

/// One question: from where and when, to where, with how many rides at most.
#[derive(Clone, Debug, PartialEq)]
pub struct Query {
    /// The stops the traveller can start from, each with the second they are there.
    pub access: Vec<(StopId, Second)>,
    /// The stops the traveller can end at, each with the walk from it to the door, seconds.
    pub egress: Vec<(StopId, Duration)>,
    /// The most rides a journey may take.
    pub max_rides: u8,
}

/// One ride: a run, from the stop and second it is boarded to the stop and second it is left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ride {
    /// The run ridden.
    pub run: TransitRunId,
    /// The stop boarded at, and the run's departure there.
    pub board: (StopId, Second),
    /// The stop alighted at, and the run's arrival there.
    pub alight: (StopId, Second),
}

/// A footpath walked between two rides.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Footpath {
    /// The stop the ride before it was left at.
    pub from: StopId,
    /// The stop the ride after it is boarded at.
    pub to: StopId,
    /// The walk, seconds, as the timetable gives it.
    pub seconds: Duration,
}

/// A journey: rides in order, with at most one footpath between each ride and the next.
#[derive(Clone, Debug, PartialEq)]
pub struct Journey {
    /// The rides, in order; never empty.
    pub rides: Vec<Ride>,
    /// Between ride `i` and ride `i + 1`, the footpath walked, or `None` where the next ride is
    /// boarded at the stop the last one was left at. One fewer than the rides.
    pub footpaths: Vec<Option<Footpath>>,
    /// The access stop and the second the traveller is there.
    pub access: (StopId, Second),
    /// The egress stop and the walk from it to the door, seconds.
    pub egress: (StopId, Duration),
    /// At the door: the last ride's arrival plus the egress walk, rounded up to the second.
    pub arrival: Second,
}

impl Journey {
    /// The seconds spent waiting at stops: over every boarding, the run's departure less the
    /// instant the traveller is at the stop (the access second, or the last ride's arrival plus
    /// the footpath walked). The slack is part of it.
    #[must_use]
    pub fn wait(&self) -> Duration {
        let mut reached = Duration(f64::from(self.access.1.get()));
        let mut total = Duration::ZERO;
        for (i, ride) in self.rides.iter().enumerate() {
            if i > 0 {
                let walk = self.footpaths[i - 1].map_or(Duration::ZERO, |f| f.seconds);
                reached = Duration(f64::from(self.rides[i - 1].alight.1.get())) + walk;
            }
            total += Duration(f64::from(ride.board.1.get())) - reached;
        }
        total
    }

    /// The seconds spent in vehicles: over every ride, arrival less departure.
    #[must_use]
    pub fn in_vehicle(&self) -> Duration {
        let seconds: u32 = self.rides.iter().map(|r| r.alight.1.get() - r.board.1.get()).sum();
        Duration(f64::from(seconds))
    }

    /// The seconds walked on footpaths between rides.
    #[must_use]
    pub fn transfer_walk(&self) -> Duration {
        self.footpaths.iter().flatten().map(|f| f.seconds).sum()
    }

    /// The seconds walked from the egress stop to the door.
    #[must_use]
    pub fn egress_walk(&self) -> Duration {
        self.egress.1
    }

    /// How many rides.
    #[must_use]
    pub fn ride_count(&self) -> usize {
        self.rides.len()
    }
}

/// How a stop was reached by a ride in a round: pattern, the run's rank in it, and the
/// positions boarded and left at.
#[derive(Clone, Copy, Debug)]
struct RideLabel {
    pattern: u32,
    rank: u32,
    board: u32,
    alight: u32,
}

const NO_RIDE: RideLabel = RideLabel { pattern: INF, rank: 0, board: 0, alight: 0 };

/// One round's labels, per stop.
#[derive(Clone, Debug)]
struct Round {
    /// The arrival a traveller can board from, by a ride or a footpath (round 0: the access).
    arr: Vec<u32>,
    /// The arrival by a ride.
    ride_arr: Vec<u32>,
    /// The ride of `ride_arr`.
    ride: Vec<RideLabel>,
    /// Where `arr` was walked from, null when it is a ride's.
    walk_from: Vec<StopId>,
    /// The stops this round labelled.
    touched: Vec<StopId>,
}

impl Round {
    fn new(n: usize) -> Self {
        Self {
            arr: vec![INF; n],
            ride_arr: vec![INF; n],
            ride: vec![NO_RIDE; n],
            walk_from: vec![StopId::NULL; n],
            touched: Vec::new(),
        }
    }

    fn touch(&mut self, s: StopId) {
        if self.arr[s.index()] == INF && self.ride_arr[s.index()] == INF {
            self.touched.push(s);
        }
    }

    fn clean(&mut self) {
        for s in self.touched.drain(..) {
            let i = s.index();
            self.arr[i] = INF;
            self.ride_arr[i] = INF;
            self.ride[i] = NO_RIDE;
            self.walk_from[i] = StopId::NULL;
        }
    }
}

/// The last ride the tie rule picks: run id, egress stop, the position left at, the pattern,
/// the run's rank, the position boarded at.
type LastRide = (TransitRunId, StopId, u32, u32, u32, u32);

/// The search over one timetable at one boarding slack, with its scratch. Reusable: every query
/// leaves the scratch clean.
#[derive(Clone, Debug)]
pub struct Raptor<'a> {
    tt: &'a Timetable,
    slack: u32,
    rounds: Vec<Round>,
    /// Per stop: the best arrival over every round so far, by a ride or a footpath; the best by
    /// a ride; the best with fewer rides than the round being scanned.
    best: Vec<u32>,
    best_ride: Vec<u32>,
    prev: Vec<u32>,
    /// Per stop, the egress walk on the clock (infinite where it is not an egress stop).
    egress: Vec<u32>,
    touched: Vec<StopId>,
    marked: Vec<bool>,
    marked_list: Vec<StopId>,
    /// Per pattern, the first position to scan from this round (infinite when not queued).
    queue_pos: Vec<u32>,
    queue: Vec<u32>,
    ride_improved: Vec<StopId>,
}

impl<'a> Raptor<'a> {
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
            rounds: Vec::new(),
            best: vec![INF; n],
            best_ride: vec![INF; n],
            prev: vec![INF; n],
            egress: vec![INF; n],
            touched: Vec::new(),
            marked: vec![false; n],
            marked_list: Vec::new(),
            queue_pos: vec![INF; timetable.pattern_count() as usize],
            queue: Vec::new(),
            ride_improved: Vec::new(),
        }
    }

    /// The journey of earliest arrival for `query`, by the rule of the
    /// [module documentation](self); `None` when no journey of one ride or more reaches an egress
    /// stop.
    ///
    /// # Panics
    ///
    /// For a stop the timetable does not have, and for an egress walk that is negative or not
    /// finite.
    pub fn earliest(&mut self, query: &Query) -> Option<Journey> {
        let k_max = usize::from(query.max_rides);
        if k_max == 0 {
            return None;
        }
        let n = self.best.len();
        while self.rounds.len() <= k_max {
            self.rounds.push(Round::new(n));
        }
        for &(s, w) in &query.egress {
            assert!(w.is_finite() && w.get() >= 0.0, "the egress walk from stop {s} is {w}");
            self.egress[s.index()] = self.egress[s.index()].min(on_clock(w));
        }
        for &(s, t) in &query.access {
            let (i, t) = (s.index(), t.get());
            if t < self.rounds[0].arr[i] {
                self.rounds[0].touch(s);
                self.touch(s);
                self.rounds[0].arr[i] = t;
                self.best[i] = self.best[i].min(t);
                self.prev[i] = self.prev[i].min(t);
                self.mark(s);
            }
        }
        let found = self.run_rounds(query, k_max);
        let journey = found.last().map(|&(k, at)| self.journey(query, k, at));
        self.clean(query);
        journey
    }

    /// The journeys of `query` by number of rides: for each count of rides that reaches the door
    /// earlier than every smaller count does, the earliest journey with that many rides, fewest
    /// rides first. The last is [`Raptor::earliest`]'s; empty when there is none.
    ///
    /// # Panics
    ///
    /// As [`Raptor::earliest`].
    pub fn by_rides(&mut self, query: &Query) -> Vec<Journey> {
        let k_max = usize::from(query.max_rides);
        if k_max == 0 {
            return Vec::new();
        }
        let n = self.best.len();
        while self.rounds.len() <= k_max {
            self.rounds.push(Round::new(n));
        }
        for &(s, w) in &query.egress {
            assert!(w.is_finite() && w.get() >= 0.0, "the egress walk from stop {s} is {w}");
            self.egress[s.index()] = self.egress[s.index()].min(on_clock(w));
        }
        for &(s, t) in &query.access {
            let (i, t) = (s.index(), t.get());
            if t < self.rounds[0].arr[i] {
                self.rounds[0].touch(s);
                self.touch(s);
                self.rounds[0].arr[i] = t;
                self.best[i] = self.best[i].min(t);
                self.prev[i] = self.prev[i].min(t);
                self.mark(s);
            }
        }
        let found = self.run_rounds(query, k_max);
        let journeys = found.iter().map(|&(k, at)| self.journey(query, k, at)).collect();
        self.clean(query);
        journeys
    }

    /// The rounds. Every round whose door arrival is earlier than any round before it, with
    /// that arrival, in round order: the last is the earliest arrival.
    fn run_rounds(&mut self, query: &Query, k_max: usize) -> Vec<(usize, u32)> {
        let mut bound = INF;
        let mut found = Vec::new();
        for k in 1..=k_max {
            if self.marked_list.is_empty() {
                break;
            }
            self.queue_patterns();
            let queue = mem::take(&mut self.queue);
            for &p in &queue {
                let pos = mem::replace(&mut self.queue_pos[p as usize], INF);
                self.scan(k, p, pos, bound);
            }
            self.queue = queue;
            self.queue.clear();
            self.walk(k, bound);
            let round = &self.rounds[k];
            let at = query
                .egress
                .iter()
                .map(|&(e, _)| round.ride_arr[e.index()].saturating_add(self.egress[e.index()]))
                .min()
                .unwrap_or(INF);
            if at < bound {
                bound = at;
                found.push((k, at));
            }
            for &s in &self.marked_list {
                self.prev[s.index()] = self.best[s.index()];
            }
        }
        found
    }

    /// Queue the patterns through the stops marked last round, each from its first such
    /// position, in pattern order; unmark the stops.
    fn queue_patterns(&mut self) {
        for &s in &self.marked_list {
            for &(p, pos) in self.tt.patterns_through(s) {
                let q = &mut self.queue_pos[p as usize];
                if *q == INF {
                    self.queue.push(p);
                }
                *q = (*q).min(pos);
            }
            self.marked[s.index()] = false;
        }
        self.marked_list.clear();
        self.queue.sort_unstable();
    }

    /// Scan pattern `p` in round `k` from position `from`.
    fn scan(&mut self, k: usize, p: u32, from: u32, bound: u32) {
        let tt = self.tt;
        let pattern = tt.pattern(p);
        let mut current: Option<(u32, u32)> = None; // (rank, position boarded at)
        for pos in from..pattern.len {
            let stop = tt.stop_at(p, pos);
            let i = stop.index();
            if let Some((rank, board)) = current {
                let t = tt.arrival_at(p, rank, pos);
                if t < self.best_ride[i] && t < bound {
                    self.touch(stop);
                    let round = &mut self.rounds[k];
                    round.touch(stop);
                    if round.ride_arr[i] == INF {
                        self.ride_improved.push(stop);
                    }
                    round.ride_arr[i] = t;
                    round.ride[i] = RideLabel { pattern: p, rank, board, alight: pos };
                    self.best_ride[i] = t;
                    if t < self.best[i] {
                        round.arr[i] = t;
                        round.walk_from[i] = StopId::NULL;
                        self.best[i] = t;
                        self.mark(stop);
                    }
                }
            }
            if self.prev[i] != INF {
                let ready = self.prev[i].saturating_add(self.slack);
                let below = current.map_or(pattern.runs as usize, |(rank, _)| rank as usize);
                let column = &tt.departures_at(p, pos)[..below];
                let r = column.partition_point(|&d| d < ready);
                if r < below {
                    current = Some((u32::try_from(r).expect("a rank"), pos));
                }
            }
        }
    }

    /// Walk the footpaths from the stops a ride improved in round `k`.
    fn walk(&mut self, k: usize, bound: u32) {
        let improved = mem::take(&mut self.ride_improved);
        for &s in &improved {
            let t = self.rounds[k].ride_arr[s.index()];
            for w in self.tt.walks(s) {
                let (to, i) = (w.to, w.to.index());
                let t2 = t.saturating_add(w.clock);
                if t2 < self.best[i] && t2 < bound {
                    self.touch(to);
                    let round = &mut self.rounds[k];
                    round.touch(to);
                    round.arr[i] = t2;
                    round.walk_from[i] = s;
                    self.best[i] = t2;
                    self.mark(to);
                }
            }
        }
        self.ride_improved = improved;
        self.ride_improved.clear();
    }

    fn touch(&mut self, s: StopId) {
        if self.best[s.index()] == INF && self.best_ride[s.index()] == INF {
            self.touched.push(s);
        }
    }

    fn mark(&mut self, s: StopId) {
        if !self.marked[s.index()] {
            self.marked[s.index()] = true;
            self.marked_list.push(s);
        }
    }

    /// The earliest second a traveller is at `stop` with fewer than `k` rides, and the first
    /// round that has it; `None` if not reached.
    fn ready_before(&self, k: usize, stop: StopId) -> Option<(usize, u32)> {
        let i = stop.index();
        let (round, t) = (0..k).map(|j| (j, self.rounds[j].arr[i])).min_by_key(|&(j, t)| (t, j))?;
        (t != INF).then_some((round, t))
    }

    /// The last ride by the tie rule: of every run that leaves an egress stop `e` at
    /// `arrival − egress(e)` and can be caught from the labels of rounds before `k`, the lowest
    /// run id, then the lowest egress stop, then the lowest position left at; boarded at the
    /// first position where it can be caught.
    fn last_ride(&self, query: &Query, k: usize, arrival: u32) -> LastRide {
        let tt = self.tt;
        let mut best: Option<LastRide> = None;
        for &(e, _) in &query.egress {
            let Some(t_e) = arrival.checked_sub(self.egress[e.index()]) else { continue };
            for &(p, j) in tt.patterns_through(e) {
                let column = tt.arrivals_at(p, j);
                let lo = column.partition_point(|&a| a < t_e);
                let hi = column.partition_point(|&a| a <= t_e);
                for rank in u32::try_from(lo).expect("a rank")..u32::try_from(hi).expect("a rank") {
                    let catch = (0..j).find(|&i| {
                        self.ready_before(k, tt.stop_at(p, i)).is_some_and(|(_, t)| {
                            t.saturating_add(self.slack) <= tt.departure_at(p, rank, i)
                        })
                    });
                    if let Some(i) = catch {
                        let c = (tt.run_at(p, rank), e, j, p, rank, i);
                        if best.is_none_or(|b| (c.0, c.1, c.2) < (b.0, b.1, b.2)) {
                            best = Some(c);
                        }
                    }
                }
            }
        }
        best.expect("the round that reached the door has a last ride")
    }

    /// The journey reaching the door at `arrival` in round `k`.
    fn journey(&self, query: &Query, k: usize, arrival: u32) -> Journey {
        let tt = self.tt;
        let (run, e, j, p, rank, i) = self.last_ride(query, k, arrival);
        let ride = |p: u32, rank: u32, board: u32, alight: u32| Ride {
            run: tt.run_at(p, rank),
            board: (tt.stop_at(p, board), Second(tt.departure_at(p, rank, board))),
            alight: (tt.stop_at(p, alight), Second(tt.arrival_at(p, rank, alight))),
        };
        let mut rides = vec![ride(p, rank, i, j)];
        debug_assert_eq!(rides[0].run, run);
        let mut footpaths = Vec::new();
        let (mut stop, mut limit) = (tt.stop_at(p, i), k);
        let access = loop {
            let (round, t) = self.ready_before(limit, stop).expect("the boarding had a time");
            if round == 0 {
                break (stop, Second(t));
            }
            let from = self.rounds[round].walk_from[stop.index()];
            let ride_stop = if from.is_null() {
                footpaths.push(None);
                stop
            } else {
                let w = tt.walks(from).iter().find(|w| w.to == stop).expect("the footpath walked");
                debug_assert_eq!(self.rounds[round].ride_arr[from.index()] + w.clock, t);
                footpaths.push(Some(Footpath { from, to: stop, seconds: w.seconds }));
                from
            };
            let l = self.rounds[round].ride[ride_stop.index()];
            rides.push(ride(l.pattern, l.rank, l.board, l.alight));
            (stop, limit) = (tt.stop_at(l.pattern, l.board), round);
        };
        rides.reverse();
        footpaths.reverse();
        debug_assert_eq!(rides.len(), k, "the journey takes the rounds' rides");
        let walk = query
            .egress
            .iter()
            .filter(|w| w.0 == e)
            .map(|w| w.1)
            .min_by(|a, b| a.get().total_cmp(&b.get()))
            .expect("an egress stop");
        Journey { rides, footpaths, access, egress: (e, walk), arrival: Second(arrival) }
    }

    /// Reset what this query touched, and nothing else.
    fn clean(&mut self, query: &Query) {
        for round in &mut self.rounds {
            round.clean();
        }
        for s in self.touched.drain(..) {
            self.best[s.index()] = INF;
            self.best_ride[s.index()] = INF;
            self.prev[s.index()] = INF;
        }
        for &s in &self.marked_list {
            self.marked[s.index()] = false;
        }
        self.marked_list.clear();
        for &(s, _) in &query.egress {
            self.egress[s.index()] = INF;
        }
    }
}

/// [`Raptor::earliest`] for every query, in parallel (rayon), one scratch per thread. Answer
/// `i` is query `i`'s, and each depends on nothing but its query: the vector is the same, bit for
/// bit, at any number of threads.
///
/// # Panics
///
/// As [`Raptor::new`] and [`Raptor::earliest`].
#[must_use]
pub fn earliest_many(
    timetable: &Timetable,
    boarding_slack: Duration,
    queries: &[Query],
) -> Vec<Option<Journey>> {
    let first = Raptor::new(timetable, boarding_slack);
    queries.par_iter().map_init(|| first.clone(), |raptor, q| raptor.earliest(q)).collect()
}

/// [`Raptor::by_rides`] for every query, in parallel, as [`earliest_many`].
///
/// # Panics
///
/// As [`Raptor::new`] and [`Raptor::earliest`].
#[must_use]
pub fn by_rides_many(
    timetable: &Timetable,
    boarding_slack: Duration,
    queries: &[Query],
) -> Vec<Vec<Journey>> {
    let first = Raptor::new(timetable, boarding_slack);
    queries.par_iter().map_init(|| first.clone(), |raptor, q| raptor.by_rides(q)).collect()
}
