//! Fixtures shared by the transit tests: random draws, timetables written as plain rows (so the
//! brute force reads the rows, not the timetable built from them), lines at a headway, and the
//! brute force itself.

#![allow(dead_code, reason = "each test binary uses a different part of this module")]

use std::collections::HashMap;

use openmobisim_transit::raptor::{Journey, Query};
use openmobisim_transit::timetable::{Call, Timetable, TimetableBuilder};
use openmobisim_types::ids::{EntityId, StopId, TransitRunId};
use openmobisim_types::rng::{DrawAddress, DrawBlock, RngKey, Stream, StreamRng};
use openmobisim_types::time::Second;
use openmobisim_types::units::{Duration, Metres};

/// A stream of uniform draws for test inputs: ChaCha8 blocks from `openmobisim-types`, read
/// word by word at addresses (0, 0), (0, 1), ...
#[derive(Clone, Debug)]
pub struct Draws {
    rng: StreamRng,
    block: DrawBlock,
    word: usize,
    next: u32,
}

impl Draws {
    /// The stream of seed `seed`.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        let rng = StreamRng::new(RngKey::new(seed), Stream::Choice);
        let block = rng.block(DrawAddress::from_pair(0, 0));
        Self { rng, block, word: 0, next: 1 }
    }

    /// The next draw in `[0, 1)`.
    pub fn unit(&mut self) -> f64 {
        if self.word == DrawBlock::LEN {
            self.block = self.rng.block(DrawAddress::from_pair(0, self.next));
            self.next += 1;
            self.word = 0;
        }
        self.word += 1;
        self.block.unit(self.word - 1)
    }

    /// The next draw in `0..n`.
    pub fn below(&mut self, n: u32) -> u32 {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a draw in [0, 1) times n, floored, is below n"
        )]
        let k = (self.unit() * f64::from(n)) as u32;
        k
    }

    /// The next draw in `lo..=hi`.
    pub fn between(&mut self, lo: u32, hi: u32) -> u32 {
        lo + self.below(hi - lo + 1)
    }
}

/// A call as a row: stop index, arrival, departure.
pub type RowCall = (usize, u32, u32);

/// A timetable as rows: stops (named `s00`, `s01`, ... so that stop `i` gets id `i`), runs
/// (external id, calls as stop index, arrival, departure), footpaths (from, to, seconds).
#[derive(Clone, Debug, Default)]
pub struct Rows {
    pub stops: usize,
    pub runs: Vec<(String, Vec<RowCall>)>,
    pub footpaths: Vec<(usize, usize, f64)>,
}

/// The external id of stop `i`.
#[must_use]
pub fn stop_name(i: usize) -> String {
    format!("s{i:02}")
}

impl Rows {
    /// The timetable of these rows, stops 100 m apart on a line.
    ///
    /// # Panics
    ///
    /// If the build fails.
    #[must_use]
    pub fn build(&self) -> Timetable {
        let mut b = TimetableBuilder::new();
        for i in 0..self.stops {
            let x = f64::from(u32::try_from(i).unwrap()) * 100.0;
            b.add_stop(stop_name(i), (Metres(x), Metres::ZERO));
        }
        for (name, calls) in &self.runs {
            let calls = calls
                .iter()
                .map(|&(s, a, d)| Call {
                    stop: stop_name(s),
                    arrival: Second(a),
                    departure: Second(d),
                })
                .collect();
            b.add_run(name.clone(), calls);
        }
        for &(f, t, s) in &self.footpaths {
            b.add_footpath(stop_name(f), stop_name(t), Duration(s));
        }
        b.build().expect("the rows build")
    }

    /// Add the runs of a line through `stops` (indices), `rides[i]` seconds from stop `i` to
    /// stop `i + 1`, no dwell, leaving its first stop at `first`, `first + headway`, ... while
    /// before `until`; the runs named `{name}-{n}`.
    pub fn line(
        &mut self,
        name: &str,
        stops: &[usize],
        rides: &[u32],
        first: u32,
        headway: u32,
        until: u32,
    ) {
        let mut t0 = first;
        let mut n = 0;
        while t0 < until {
            let mut t = t0;
            let mut calls = vec![(stops[0], t, t)];
            for (i, &r) in rides.iter().enumerate() {
                t += r;
                calls.push((stops[i + 1], t, t));
            }
            self.runs.push((format!("{name}-{n:04}"), calls));
            n += 1;
            t0 += headway;
        }
    }
}

/// A large random timetable: `stops` stops; `patterns` patterns of 10 to 20 distinct stops,
/// rides of 1 to 5 min and dwells of 0 to 30 s; `runs` runs each on its pattern's timing,
/// leaving the first stop in the first three hours; from each stop three footpaths of 60 to
/// 600 s to stops drawn at random.
#[must_use]
pub fn large_rows(draws: &mut Draws, stops: u32, patterns: u32, runs: u32) -> Rows {
    let n = stops as usize;
    let mut rows = Rows { stops: n, ..Rows::default() };
    for p in 0..patterns {
        let len = draws.between(10, 20) as usize;
        let mut all: Vec<usize> = (0..n).collect();
        for i in 0..len {
            let j = i + draws.below(u32::try_from(n - i).unwrap()) as usize;
            all.swap(i, j);
        }
        let timing: Vec<(u32, u32)> =
            (0..len).map(|_| (draws.between(60, 300), draws.below(31))).collect();
        for r in 0..runs {
            let mut t = draws.below(3 * 3_600);
            let mut calls = Vec::with_capacity(len);
            for (i, &s) in all[..len].iter().enumerate() {
                if i > 0 {
                    t += timing[i].0;
                }
                calls.push((s, t, t + timing[i].1));
                t += timing[i].1;
            }
            rows.runs.push((format!("p{p:02}r{r:02}"), calls));
        }
    }
    for a in 0..n {
        for _ in 0..3 {
            let b = draws.below(stops) as usize;
            if b != a {
                rows.footpaths.push((a, b, 60.0 + 540.0 * draws.unit()));
            }
        }
    }
    rows
}

/// `count` random queries on `stops` stops: 1 to 3 access stops reached in the first two hours,
/// 1 to 3 egress stops with walks of 0 to 300 s, at most 1 to 5 rides.
#[must_use]
pub fn random_queries(draws: &mut Draws, stops: u32, count: usize) -> Vec<Query> {
    let stop = |d: &mut Draws| StopId::new(d.below(stops));
    (0..count)
        .map(|_| {
            let k = draws.between(1, 3);
            let access = (0..k).map(|_| (stop(draws), Second(draws.below(7_200)))).collect();
            let k = draws.between(1, 3);
            let egress = (0..k).map(|_| (stop(draws), Duration(300.0 * draws.unit()))).collect();
            Query { access, egress, max_rides: u8::try_from(draws.between(1, 5)).unwrap() }
        })
        .collect()
}

/// What the brute force compares: the arrival, the rides, the last run's id.
pub type Outcome = (u32, usize, u32);

/// The best journey by the definition, enumerated: from each access stop, every boarding of
/// every run at every call it can be caught at, every later call to leave it at, there either
/// ending (at an egress stop) or, with rides left, boarding again at that stop or after one
/// footpath; every walk and the slack added in real seconds, the door arrival rounded up to the
/// whole second. Ranked by arrival, then rides, then the last run's id. A state (stop, instant
/// ready to board, rides left) has one future whatever led to it, so each is answered once.
#[must_use]
pub fn brute_force(rows: &Rows, tt: &Timetable, slack: f64, q: &Query) -> Option<Outcome> {
    let run_id: Vec<u32> = rows
        .runs
        .iter()
        .map(|(name, _)| tt.run_ids().typed_id_of::<TransitRunId>(name).expect("a run").raw())
        .collect();
    let mut b = Brute { rows, run_id, slack, query: q, memo: HashMap::new() };
    q.access.iter().filter_map(|&(s, t)| b.from(s.index(), f64::from(t.get()), q.max_rides)).min()
}

struct Brute<'a> {
    rows: &'a Rows,
    run_id: Vec<u32>,
    slack: f64,
    query: &'a Query,
    memo: HashMap<(usize, u64, u8), Option<Outcome>>,
}

impl Brute<'_> {
    /// The best continuation from `stop`, ready to board at `t`, with `left` rides left: its
    /// arrival, its rides, its last run.
    fn from(&mut self, stop: usize, t: f64, left: u8) -> Option<Outcome> {
        if left == 0 {
            return None;
        }
        if let Some(&o) = self.memo.get(&(stop, t.to_bits(), left)) {
            return o;
        }
        let mut best: Option<Outcome> = None;
        let rows = self.rows;
        for (r, (_, calls)) in rows.runs.iter().enumerate() {
            for (i, &(s, _, dep)) in calls.iter().enumerate() {
                if s != stop || t + self.slack > f64::from(dep) {
                    continue;
                }
                for &(to, arr, _) in &calls[i + 1..] {
                    let a = f64::from(arr);
                    for &(e, w) in &self.query.egress {
                        if e.index() == to {
                            best = better(best, Some((ceil(a + w.get()), 1, self.run_id[r])));
                        }
                    }
                    let mut next = vec![(to, a)];
                    next.extend(
                        rows.footpaths.iter().filter(|f| f.0 == to).map(|f| (f.1, a + f.2)),
                    );
                    for (s2, t2) in next {
                        if let Some((at, n, last)) = self.from(s2, t2, left - 1) {
                            best = better(best, Some((at, n + 1, last)));
                        }
                    }
                }
            }
        }
        self.memo.insert((stop, t.to_bits(), left), best);
        best
    }
}

/// The better of two outcomes, `None` the worst.
fn better(a: Option<Outcome>, b: Option<Outcome>) -> Option<Outcome> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (x, None) => x,
        (None, y) => y,
    }
}

/// `x` rounded up to the whole second.
#[must_use]
pub fn ceil(x: f64) -> u32 {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a clock second")]
    let s = x.ceil() as u32;
    s
}

/// Check a journey against the rows and the query, independently of how it was found: each
/// ride on its run's calls, every boarding in time, every footpath given, the arrival the last
/// ride's plus the egress walk rounded up, and the components adding up to the door instant.
///
/// # Panics
///
/// On the first thing that does not hold.
pub fn check_journey(rows: &Rows, tt: &Timetable, slack: f64, q: &Query, j: &Journey) {
    assert!(!j.rides.is_empty() && j.rides.len() <= usize::from(q.max_rides));
    assert_eq!(j.footpaths.len(), j.rides.len() - 1);
    assert!(q.access.contains(&j.access), "{j:?}: the access is the query's");
    assert!(q.egress.contains(&j.egress), "{j:?}: the egress is the query's");
    let mut reached = f64::from(j.access.1.get());
    let mut at = j.access.0;
    for (i, ride) in j.rides.iter().enumerate() {
        if i > 0 {
            let prev = j.rides[i - 1].alight;
            reached = f64::from(prev.1.get());
            at = prev.0;
            if let Some(f) = j.footpaths[i - 1] {
                assert_eq!(f.from, at);
                let given = (f.from.index(), f.to.index(), f.seconds.get());
                assert!(rows.footpaths.contains(&given), "{j:?}: footpath {given:?} not given");
                reached += f.seconds.get();
                at = f.to;
            }
        }
        assert_eq!(ride.board.0, at, "{j:?}: ride {i} boards where the traveller is");
        assert!(reached + slack <= f64::from(ride.board.1.get()), "{j:?}: ride {i} missed");
        let name = tt.run_ids().external(ride.run.raw());
        let calls = &rows.runs.iter().find(|r| r.0 == name).expect("the run").1;
        let b = calls.iter().position(|c| (c.0, c.2) == (ride.board.0.index(), ride.board.1.get()));
        let b = b.unwrap_or_else(|| panic!("{j:?}: ride {i} boards at no call of its run"));
        assert!(
            calls[b + 1..]
                .iter()
                .any(|c| (c.0, c.1) == (ride.alight.0.index(), ride.alight.1.get())),
            "{j:?}: ride {i} leaves at no later call of its run"
        );
    }
    let last = j.rides.last().expect("a ride").alight;
    assert_eq!(last.0, j.egress.0);
    let door = f64::from(last.1.get()) + j.egress.1.get();
    assert_eq!(j.arrival.get(), ceil(door));
    let sum = f64::from(j.access.1.get())
        + j.wait().get()
        + j.in_vehicle().get()
        + j.transfer_walk().get()
        + j.egress_walk().get();
    assert!((sum - door).abs() < 1e-6, "{j:?}: the components add to {sum} s, the door {door} s");
}

/// A query from stop `from` at second `t` to stop `to` with no egress walk.
#[must_use]
pub fn stop_to_stop(from: usize, t: u32, to: usize, max_rides: u8) -> Query {
    Query {
        access: vec![(StopId::from_index(from), Second(t))],
        egress: vec![(StopId::from_index(to), Duration::ZERO)],
        max_rides,
    }
}
