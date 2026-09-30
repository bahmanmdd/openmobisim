//! RAPTOR: round-based public transit routing (Delling, Pajor and Werneck,
//! "Round-Based Public Transit Routing", *Transportation Science* 49(3), 2015;
//! S22, design §18.3).
//!
//! Round `k` finds the earliest arrival at every stop with at most `k` vehicles:
//! it scans every **pattern** that calls at a stop improved in round `k − 1`,
//! boarding the earliest run a passenger there can catch, then relaxes the
//! **walking transfers** from the stops the scan improved. A round is a hub
//! transition (S22): alight, perhaps walk, board.
//!
//! **Patterns** are a [`Timetable`]'s pattern groups split so that no run of a
//! pattern overtakes another at any call — RAPTOR's condition for "the earliest
//! run after a time" to be a binary search. The split is made from the times
//! given ([`RaptorData::new`]): scheduled ones, or a loading's realised ones.
//!
//! **Walking transfers need not be transitively closed.** They come from bounded
//! searches on the walk layer, so a stop can be reachable in two walks and not
//! in one. Textbook RAPTOR then misses journeys that ride to a stop and walk on
//! from it when a walk had already reached that stop earlier. Here each stop keeps
//! its best arrival **by a ride** apart from its best arrival overall, and walks
//! start from every stop a ride improved: the result is exact for single walks
//! between rides, which the brute-force check in the tests confirms.
//!
//! **What a query returns:** the earliest arrival at the destination, given the
//! times the passenger can be at each access stop and the walk from each egress
//! stop; among journeys arriving then, the one with fewest vehicles. At least one
//! vehicle: walking all the way is another mode. [`Raptor::pareto`] returns
//! every journey of the trade-off instead (M4, S201): for each number of
//! vehicles, the earliest arrival, where it beats every journey with fewer —
//! round `k` finds exactly that, so the set costs nothing more than the query.
//!
//! **Riding a chosen line again** ([`RaptorData::ride_line`], M4): a traveller who
//! chose a journey on expected times boards, on the realised ones, the first run
//! of the same line (a GTFS route) they can catch at the same stop, to the same
//! stop.
//!
//! **Boarding slack:** a passenger boards a run only if they are at the stop
//! [`RaptorData::board_slack`] seconds before it leaves, at every boarding.
//!
//! **Cost of a query:** per round, the patterns through the stops improved, each
//! scanned once from the first such stop, with a binary search where a boarding
//! is tried. **Scratch per [`Raptor`]:** about `28 × (rounds + 1)` bytes per stop.

use openmobisim_core_types::ids::{EntityId, NULL_ID, NodeId, TransitRunId};

use crate::timetable::{ALIGHT, BOARD, CallTimes, Timetable, UNKNOWN_TIME};

const INF: u32 = u32::MAX;

/// Walking transfers between stops, by stop.
#[derive(Clone, Debug, Default)]
pub struct Footpaths {
    start: Vec<u32>,
    to: Vec<NodeId>,
    seconds: Vec<u32>,
}

impl Footpaths {
    /// Footpaths among `stop_count` stops, from `(from, to, seconds)` triples:
    /// the shortest of repeated pairs is kept, a stop's path to itself dropped.
    ///
    /// # Panics
    ///
    /// Panics if a stop is not below `stop_count`.
    #[must_use]
    pub fn new(stop_count: u32, mut paths: Vec<(NodeId, NodeId, u32)>) -> Self {
        paths.retain(|p| p.0 != p.1);
        paths.sort_unstable_by_key(|&(f, t, s)| (f, t, s));
        paths.dedup_by(|b, a| a.0 == b.0 && a.1 == b.1);
        let n = stop_count as usize;
        let mut start = vec![0u32; n + 1];
        for &(f, _, _) in &paths {
            assert!(f.index() < n, "a footpath starts at an unknown stop");
            start[f.index() + 1] += 1;
        }
        for i in 0..n {
            start[i + 1] += start[i];
        }
        let to = paths.iter().map(|p| p.1).collect();
        let seconds = paths.iter().map(|p| p.2).collect();
        Self { start, to, seconds }
    }

    /// No footpaths among `stop_count` stops.
    #[must_use]
    pub fn none(stop_count: u32) -> Self {
        Self::new(stop_count, Vec::new())
    }

    /// The footpaths from `stop`: `(to, seconds)`.
    pub fn from(&self, stop: NodeId) -> impl Iterator<Item = (NodeId, u32)> + '_ {
        let (a, b) = (self.start[stop.index()] as usize, self.start[stop.index() + 1] as usize);
        self.to[a..b].iter().copied().zip(self.seconds[a..b].iter().copied())
    }

    /// How many.
    #[must_use]
    pub fn len(&self) -> usize {
        self.to.len()
    }

    /// Whether there are none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.to.is_empty()
    }
}

#[derive(Clone, Copy, Debug)]
struct Pattern {
    stops_start: u32,
    len: u32,
    runs_start: u32,
    runs: u32,
    times_start: u32,
}

/// The timetable in RAPTOR's form, for one set of times (see the [module docs](self)).
#[derive(Clone, Debug)]
pub struct RaptorData {
    stop_count: u32,
    patterns: Vec<Pattern>,
    pattern_stops: Vec<NodeId>,
    pattern_flags: Vec<u8>,
    pattern_runs: Vec<TransitRunId>,
    /// The line (the timetable's route) of each pattern.
    pattern_route: Vec<u32>,
    /// The first call index of each of `pattern_runs`.
    pattern_run_call: Vec<u32>,
    arrival: Vec<u32>,
    departure: Vec<u32>,
    stop_patterns_start: Vec<u32>,
    stop_patterns: Vec<(u32, u32)>,
    footpaths: Footpaths,
    board_slack: u32,
}

impl RaptorData {
    /// RAPTOR's form of `timetable` under `times`, with `footpaths` between its
    /// stops and `board_slack` seconds before every boarding.
    ///
    /// # Panics
    ///
    /// Panics if `times` does not have one entry per call of `timetable`.
    #[must_use]
    pub fn new(
        timetable: &Timetable,
        times: &CallTimes,
        footpaths: Footpaths,
        board_slack: u32,
    ) -> Self {
        assert_eq!(times.arrival.len(), timetable.call_count(), "one time per call");
        assert_eq!(times.departure.len(), timetable.call_count(), "one time per call");
        let mut data = Self {
            stop_count: timetable.stop_count(),
            patterns: Vec::new(),
            pattern_stops: Vec::new(),
            pattern_flags: Vec::new(),
            pattern_runs: Vec::new(),
            pattern_route: Vec::new(),
            pattern_run_call: Vec::new(),
            arrival: Vec::new(),
            departure: Vec::new(),
            stop_patterns_start: Vec::new(),
            stop_patterns: Vec::new(),
            footpaths,
            board_slack,
        };
        for group in 0..timetable.group_count() {
            let mut runs: Vec<TransitRunId> = timetable.group_runs(group).to_vec();
            let first = |r: TransitRunId| timetable.run_calls(r).start;
            runs.sort_by_key(|&r| (times.departure[first(r)], r));
            // Greedy split: a run joins the first sub-pattern it does not overtake.
            let mut subs: Vec<Vec<TransitRunId>> = Vec::new();
            for r in runs {
                let calls = timetable.run_calls(r);
                let fits = |last: TransitRunId| {
                    let other = timetable.run_calls(last);
                    calls.clone().zip(other).all(|(c, o)| {
                        times.departure[c] >= times.departure[o]
                            && times.arrival[c] >= times.arrival[o]
                    })
                };
                match subs.iter_mut().find(|s| fits(*s.last().expect("never empty"))) {
                    Some(s) => s.push(r),
                    None => subs.push(vec![r]),
                }
            }
            for sub in subs {
                data.push_pattern(timetable, times, &sub);
            }
        }
        data.index_stops();
        data
    }

    fn push_pattern(&mut self, timetable: &Timetable, times: &CallTimes, runs: &[TransitRunId]) {
        let calls = timetable.run_calls(runs[0]);
        let len = u32::try_from(calls.len()).expect("a run's calls fit u32");
        let to_u32 = |n: usize| u32::try_from(n).expect("RAPTOR arrays fit u32");
        let pattern = Pattern {
            stops_start: to_u32(self.pattern_stops.len()),
            len,
            runs_start: to_u32(self.pattern_runs.len()),
            runs: to_u32(runs.len()),
            times_start: to_u32(self.arrival.len()),
        };
        for c in calls {
            self.pattern_stops.push(timetable.call_stop(c));
            self.pattern_flags.push(timetable.call_flags(c));
        }
        for &r in runs {
            let calls = timetable.run_calls(r);
            self.pattern_runs.push(r);
            self.pattern_run_call.push(to_u32(calls.start));
            for c in calls {
                self.arrival.push(times.arrival[c]);
                self.departure.push(times.departure[c]);
            }
        }
        self.pattern_route.push(timetable.run_route(runs[0]));
        self.patterns.push(pattern);
    }

    fn index_stops(&mut self) {
        let n = self.stop_count as usize;
        let mut pairs: Vec<(u32, u32, u32)> = Vec::new();
        for (p, pattern) in self.patterns.iter().enumerate() {
            for pos in 0..pattern.len {
                let stop = self.pattern_stops[(pattern.stops_start + pos) as usize];
                let p = u32::try_from(p).expect("patterns fit u32");
                pairs.push((stop.raw(), p, pos));
            }
        }
        pairs.sort_unstable();
        let mut start = vec![0u32; n + 1];
        for &(s, _, _) in &pairs {
            start[s as usize + 1] += 1;
        }
        for i in 0..n {
            start[i + 1] += start[i];
        }
        self.stop_patterns_start = start;
        self.stop_patterns = pairs.into_iter().map(|(_, p, pos)| (p, pos)).collect();
    }

    /// How many patterns.
    #[must_use]
    pub fn pattern_count(&self) -> usize {
        self.patterns.len()
    }

    /// How many stops.
    #[must_use]
    pub fn stop_count(&self) -> u32 {
        self.stop_count
    }

    /// The seconds a passenger must be at a stop before a run leaves to board it.
    #[must_use]
    pub fn board_slack(&self) -> u32 {
        self.board_slack
    }

    /// The walking transfers.
    #[must_use]
    pub fn footpaths(&self) -> &Footpaths {
        &self.footpaths
    }

    #[inline]
    fn time_index(&self, p: usize, rank: u32, pos: u32) -> usize {
        let pattern = self.patterns[p];
        (pattern.times_start + rank * pattern.len + pos) as usize
    }

    /// Ride line `route` from stop `board` to stop `alight`, being at `board` at
    /// `at`: on the run of that line, boarding at least [`Self::board_slack`]
    /// seconds after `at`, that reaches `alight` first (the lower pattern on a
    /// tie). A run whose times at either stop are unknown (a bus that did not
    /// arrive) is passed over. `None` if no run of the line does it.
    #[must_use]
    pub fn ride_line(
        &self,
        route: u32,
        board: NodeId,
        alight: NodeId,
        at: u32,
    ) -> Option<JourneyLeg> {
        let ready = at.saturating_add(self.board_slack);
        let b = board.index();
        let (lo, hi) =
            (self.stop_patterns_start[b] as usize, self.stop_patterns_start[b + 1] as usize);
        let mut best: Option<(u32, usize, u32, u32, u32)> = None; // (arrival, pattern, rank, board, alight)
        for &(p, pos) in &self.stop_patterns[lo..hi] {
            let p = p as usize;
            if self.pattern_route[p] != route {
                continue;
            }
            let pattern = self.patterns[p];
            if self.pattern_flags[(pattern.stops_start + pos) as usize] & BOARD == 0 {
                continue;
            }
            let Some(to) = (pos + 1..pattern.len).find(|&q| {
                let at = (pattern.stops_start + q) as usize;
                self.pattern_stops[at] == alight && self.pattern_flags[at] & ALIGHT != 0
            }) else {
                continue;
            };
            let Some(first) = self.earliest_run(p, pos, ready) else { continue };
            for rank in first..pattern.runs {
                let dep = self.departure[self.time_index(p, rank, pos)];
                let arr = self.arrival[self.time_index(p, rank, to)];
                if dep == UNKNOWN_TIME {
                    break;
                }
                if arr != UNKNOWN_TIME {
                    if best.is_none_or(|(a, bp, ..)| arr < a || (arr == a && p < bp)) {
                        best = Some((arr, p, rank, pos, to));
                    }
                    break;
                }
            }
        }
        let (arrival, p, rank, pos, to) = best?;
        let pattern = self.patterns[p];
        let slot = (pattern.runs_start + rank) as usize;
        let first_call = self.pattern_run_call[slot];
        Some(JourneyLeg::Ride {
            run: self.pattern_runs[slot],
            board_call: first_call + pos,
            alight_call: first_call + to,
            board_stop: board,
            alight_stop: alight,
            departure: self.departure[self.time_index(p, rank, pos)],
            arrival,
        })
    }

    /// The earliest run of pattern `p` leaving position `pos` at `t` or later.
    fn earliest_run(&self, p: usize, pos: u32, t: u32) -> Option<u32> {
        let runs = self.patterns[p].runs;
        let (mut lo, mut hi) = (0u32, runs);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self.departure[self.time_index(p, mid, pos)] < t {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        (lo < runs && self.departure[self.time_index(p, lo, pos)] != UNKNOWN_TIME).then_some(lo)
    }
}

/// One leg of a [`Journey`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JourneyLeg {
    /// Walked from the origin to `stop`, arriving at `arrival`.
    Access {
        /// The stop.
        stop: NodeId,
        /// When the passenger is there.
        arrival: u32,
    },
    /// Rode `run` from its call `board_call` to its call `alight_call`.
    Ride {
        /// The run.
        run: TransitRunId,
        /// The call boarded at (an index into the timetable's calls).
        board_call: u32,
        /// The call alighted at.
        alight_call: u32,
        /// The stop boarded at.
        board_stop: NodeId,
        /// The stop alighted at.
        alight_stop: NodeId,
        /// When it left the boarding stop.
        departure: u32,
        /// When it reached the alighting stop.
        arrival: u32,
    },
    /// Walked from one stop to another.
    Transfer {
        /// From this stop.
        from: NodeId,
        /// To this one.
        to: NodeId,
        /// Taking this long.
        seconds: u32,
    },
    /// Walked from `stop` to the destination.
    Egress {
        /// The stop.
        stop: NodeId,
        /// Taking this long.
        seconds: u32,
    },
}

/// A journey found by [`Raptor::earliest`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Journey {
    /// When the passenger reaches the destination.
    pub arrival: u32,
    /// Its legs: access, then rides with walks between, then egress.
    pub legs: Vec<JourneyLeg>,
}

impl Journey {
    /// How many vehicles it takes.
    #[must_use]
    pub fn rides(&self) -> usize {
        self.legs.iter().filter(|l| matches!(l, JourneyLeg::Ride { .. })).count()
    }
}

#[derive(Clone, Copy, Debug)]
struct RideLabel {
    pattern: u32,
    rank: u32,
    board: u32,
    alight: u32,
}

const NO_RIDE: RideLabel = RideLabel { pattern: NULL_ID, rank: 0, board: 0, alight: 0 };

/// One thread's RAPTOR state: reusable, and clean after every query.
#[derive(Debug)]
pub struct Raptor<'a> {
    data: &'a RaptorData,
    rounds: usize,
    /// Best arrival overall, best with at least one vehicle, and best by a ride.
    best: Vec<u32>,
    best_ridden: Vec<u32>,
    best_ride: Vec<u32>,
    /// Best arrival with fewer vehicles than the round being scanned.
    prev: Vec<u32>,
    /// Per round: the arrival improved in it (overall), by a ride, and how.
    arr: Vec<Vec<u32>>,
    ride_arr: Vec<Vec<u32>>,
    ride: Vec<Vec<RideLabel>>,
    walk_from: Vec<Vec<u32>>,
    touched: Vec<Vec<u32>>,
    egress: Vec<u32>,
    marked: Vec<bool>,
    marked_list: Vec<u32>,
    queue_pos: Vec<u32>,
    queue: Vec<u32>,
    ride_improved: Vec<u32>,
}

impl<'a> Raptor<'a> {
    /// A search over `data` taking at most `max_rides` vehicles.
    #[must_use]
    pub fn new(data: &'a RaptorData, max_rides: usize) -> Self {
        let n = data.stop_count as usize;
        let rounds = max_rides + 1;
        Self {
            data,
            rounds,
            best: vec![INF; n],
            best_ridden: vec![INF; n],
            best_ride: vec![INF; n],
            prev: vec![INF; n],
            arr: vec![vec![INF; n]; rounds],
            ride_arr: vec![vec![INF; n]; rounds],
            ride: vec![vec![NO_RIDE; n]; rounds],
            walk_from: vec![vec![NULL_ID; n]; rounds],
            touched: vec![Vec::new(); rounds],
            egress: vec![INF; n],
            marked: vec![false; n],
            marked_list: Vec::new(),
            queue_pos: vec![INF; data.patterns.len()],
            queue: Vec::new(),
            ride_improved: Vec::new(),
        }
    }

    fn reset(&mut self) {
        for k in 0..self.rounds {
            for &s in &self.touched[k] {
                let s = s as usize;
                self.best[s] = INF;
                self.best_ridden[s] = INF;
                self.best_ride[s] = INF;
                self.prev[s] = INF;
                self.arr[k][s] = INF;
                self.ride_arr[k][s] = INF;
                self.ride[k][s] = NO_RIDE;
                self.walk_from[k][s] = NULL_ID;
            }
            self.touched[k].clear();
        }
        for &s in &self.marked_list {
            self.marked[s as usize] = false;
        }
        self.marked_list.clear();
    }

    fn touch(&mut self, k: usize, s: usize) {
        if self.arr[k][s] == INF && self.ride_arr[k][s] == INF {
            self.touched[k].push(u32::try_from(s).expect("stops fit u32"));
        }
    }

    /// What a new arrival at `s` must beat: the best so far — at an egress stop,
    /// the best with a vehicle, since the destination needs one and the walk
    /// there from the origin does not count.
    #[inline]
    fn limit(&self, s: usize) -> u32 {
        if self.egress[s] == INF { self.best[s] } else { self.best_ridden[s] }
    }

    /// Record `t` as round `k`'s arrival at `s`, overall.
    #[inline]
    fn improve(&mut self, k: usize, s: usize, t: u32) {
        self.arr[k][s] = t;
        self.best[s] = self.best[s].min(t);
        self.best_ridden[s] = self.best_ridden[s].min(t);
        self.mark(s);
    }

    fn mark(&mut self, s: usize) {
        if !self.marked[s] {
            self.marked[s] = true;
            self.marked_list.push(u32::try_from(s).expect("stops fit u32"));
        }
    }

    /// The earliest journey from the `access` stops (each with the time the
    /// passenger can be there) to the destination, reached from the `egress`
    /// stops (each with its walk in seconds); `None` if no journey with at least
    /// one and at most `max_rides` vehicles exists. See the [module docs](self).
    #[must_use]
    pub fn earliest(
        &mut self,
        access: &[(NodeId, u32)],
        egress: &[(NodeId, u32)],
    ) -> Option<Journey> {
        let found = self.search(access, egress);
        let journey = found.last().map(|&(k, e, at)| self.reconstruct(k, e, at));
        self.finish(egress);
        journey
    }

    /// Every journey of the arrival-against-vehicles trade-off, fewest vehicles
    /// first: for each number of vehicles, the earliest arrival, where it is
    /// earlier than with any fewer. The last is [`Self::earliest`]'s journey.
    /// Empty if there is none.
    #[must_use]
    pub fn pareto(&mut self, access: &[(NodeId, u32)], egress: &[(NodeId, u32)]) -> Vec<Journey> {
        let found = self.search(access, egress);
        let journeys = found.iter().map(|&(k, e, at)| self.reconstruct(k, e, at)).collect();
        self.finish(egress);
        journeys
    }

    fn finish(&mut self, egress: &[(NodeId, u32)]) {
        for &(s, _) in egress {
            self.egress[s.index()] = INF;
        }
        self.reset();
    }

    /// The rounds, and per round that improved the arrival at the destination,
    /// `(round, egress stop, arrival)`. Leaves the labels for reconstruction.
    fn search(
        &mut self,
        access: &[(NodeId, u32)],
        egress: &[(NodeId, u32)],
    ) -> Vec<(usize, usize, u32)> {
        let data = self.data;
        for &(s, w) in egress {
            let e = &mut self.egress[s.index()];
            *e = (*e).min(w);
        }
        for &(s, t) in access {
            let i = s.index();
            if t < self.arr[0][i] {
                self.touch(0, i);
                self.arr[0][i] = t;
                self.best[i] = self.best[i].min(t);
                self.prev[i] = self.prev[i].min(t);
                self.mark(i);
            }
        }
        let mut bound = INF;
        let mut found: Vec<(usize, usize, u32)> = Vec::new();

        for k in 1..self.rounds {
            if self.marked_list.is_empty() {
                break;
            }
            // The patterns through the stops improved last round, each from its
            // first such position.
            for &s in &self.marked_list {
                let s = s as usize;
                let (a, b) = (
                    data.stop_patterns_start[s] as usize,
                    data.stop_patterns_start[s + 1] as usize,
                );
                for &(p, pos) in &data.stop_patterns[a..b] {
                    let q = &mut self.queue_pos[p as usize];
                    if *q == INF {
                        self.queue.push(p);
                    }
                    *q = (*q).min(pos);
                }
            }
            for &s in &self.marked_list {
                self.marked[s as usize] = false;
            }
            self.marked_list.clear();
            self.queue.sort_unstable();

            let queue = std::mem::take(&mut self.queue);
            for &p in &queue {
                let pos0 = std::mem::replace(&mut self.queue_pos[p as usize], INF);
                self.scan(k, p as usize, pos0, bound);
            }
            self.queue = queue;
            self.queue.clear();

            // Walks from the stops a ride improved this round.
            let improved = std::mem::take(&mut self.ride_improved);
            for &s in &improved {
                let t = self.ride_arr[k][s as usize];
                for (to, w) in data.footpaths.from(NodeId::new(s)) {
                    let i = to.index();
                    let t2 = t.saturating_add(w);
                    if t2 < self.limit(i) && t2 < bound {
                        self.touch(k, i);
                        self.improve(k, i, t2);
                        self.walk_from[k][i] = s;
                    }
                }
            }
            self.ride_improved = improved;
            self.ride_improved.clear();

            // The destination, from the stops improved this round.
            let mut round_best: Option<(usize, u32)> = None;
            for &(e, _) in egress {
                let i = e.index();
                let t = self.arr[k][i];
                if t != INF {
                    let at = t.saturating_add(self.egress[i]);
                    // A tie within the round goes to the lower stop id, whatever the order given.
                    let tie = at == bound && round_best.is_some_and(|(fe, _)| i < fe);
                    if at < bound || tie {
                        bound = at;
                        round_best = Some((i, at));
                    }
                }
            }
            if let Some((i, at)) = round_best {
                found.push((k, i, at));
            }
            for &s in &self.marked_list {
                self.prev[s as usize] = self.best[s as usize];
            }
        }
        found
    }

    fn scan(&mut self, k: usize, p: usize, pos0: u32, bound: u32) {
        let data = self.data;
        let pattern = data.patterns[p];
        let mut current: Option<(u32, u32)> = None; // (rank, board position)
        for pos in pos0..pattern.len {
            let at = (pattern.stops_start + pos) as usize;
            let stop = data.pattern_stops[at].index();
            let flags = data.pattern_flags[at];
            if let Some((rank, board)) = current {
                let t = data.arrival[data.time_index(p, rank, pos)];
                if flags & ALIGHT != 0 && t != UNKNOWN_TIME && t < self.best_ride[stop] && t < bound
                {
                    self.touch(k, stop);
                    if self.ride_arr[k][stop] == INF {
                        self.ride_improved.push(u32::try_from(stop).expect("stops fit u32"));
                    }
                    self.ride_arr[k][stop] = t;
                    self.best_ride[stop] = t;
                    self.ride[k][stop] = RideLabel {
                        pattern: u32::try_from(p).expect("patterns fit u32"),
                        rank,
                        board,
                        alight: pos,
                    };
                    if t < self.limit(stop) {
                        self.improve(k, stop, t);
                        self.walk_from[k][stop] = NULL_ID;
                    }
                }
            }
            if flags & BOARD != 0 && self.prev[stop] != INF {
                let ready = self.prev[stop].saturating_add(data.board_slack);
                let can_do_better = match current {
                    None => true,
                    Some((rank, _)) => ready <= data.departure[data.time_index(p, rank, pos)],
                };
                if can_do_better {
                    if let Some(r) = data.earliest_run(p, pos, ready) {
                        if current.is_none_or(|(rank, _)| r < rank) {
                            current = Some((r, pos));
                        }
                    }
                }
            }
        }
    }

    fn reconstruct(&self, k: usize, egress_stop: usize, arrival: u32) -> Journey {
        let data = self.data;
        let mut legs = vec![JourneyLeg::Egress {
            stop: NodeId::from_index(egress_stop),
            seconds: self.egress[egress_stop],
        }];
        let (mut round, mut stop) = (k, egress_stop);
        loop {
            if round == 0 {
                legs.push(JourneyLeg::Access {
                    stop: NodeId::from_index(stop),
                    arrival: self.arr[0][stop],
                });
                break;
            }
            let from = self.walk_from[round][stop];
            let ride_stop = if from == NULL_ID {
                stop
            } else {
                let f = from as usize;
                legs.push(JourneyLeg::Transfer {
                    from: NodeId::new(from),
                    to: NodeId::from_index(stop),
                    seconds: self.arr[round][stop] - self.ride_arr[round][f],
                });
                f
            };
            let label = self.ride[round][ride_stop];
            let p = label.pattern as usize;
            let pattern = data.patterns[p];
            let run_slot = (pattern.runs_start + label.rank) as usize;
            let first_call = data.pattern_run_call[run_slot];
            let board_stop = data.pattern_stops[(pattern.stops_start + label.board) as usize];
            legs.push(JourneyLeg::Ride {
                run: data.pattern_runs[run_slot],
                board_call: first_call + label.board,
                alight_call: first_call + label.alight,
                board_stop,
                alight_stop: NodeId::from_index(ride_stop),
                departure: data.departure[data.time_index(p, label.rank, label.board)],
                arrival: data.arrival[data.time_index(p, label.rank, label.alight)],
            });
            // The arrival the boarding used: the best of the earlier rounds at the stop
            // (the earliest round with it, so the fewest vehicles).
            let b = board_stop.index();
            round = (0..round)
                .filter(|&j| self.arr[j][b] != INF)
                .min_by_key(|&j| (self.arr[j][b], j))
                .expect("the boarding had a time");
            stop = b;
        }
        legs.reverse();
        Journey { arrival, legs }
    }
}
