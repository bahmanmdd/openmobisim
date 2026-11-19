//! The cheapest journey per number of rides against an exhaustive enumeration on random
//! timetables, and by hand where waiting weighs more than riding.
//!
//! The enumeration reads the rows, not the timetable built from them: from a stop at an
//! instant, every run that can be caught there at every call, left at every later call, there
//! ending at an egress stop or boarding again at that stop or after one footpath; each part
//! priced by the weights. The cost from a state (stop, instant, rides left, whether a ride was
//! taken) does not depend on what led to it, so each is answered once.

mod common;

use std::collections::HashMap;

use common::{Draws, Rows, check_journey};
use openmobisim_transit::cheapest::{Cheapest, CostQuery, Weights};
use openmobisim_transit::raptor::{Journey, Query};
use openmobisim_types::ids::{EntityId, StopId};
use openmobisim_types::time::Second;
use openmobisim_types::units::Duration;

/// The cost of `j` for `q`, from its parts as the module documentation prices them.
fn cost_of(q: &CostQuery, j: &Journey) -> f64 {
    let w = q.weights;
    let a = q.query.access.iter().position(|a| *a == j.access).expect("the access");
    let e = q.query.egress.iter().position(|e| *e == j.egress).expect("the egress");
    let mut cost = q.access_cost[a] + q.egress_cost[e];
    let mut reached = f64::from(j.access.1.get());
    for (i, ride) in j.rides.iter().enumerate() {
        let mut wait = w.first_wait;
        if i > 0 {
            reached = f64::from(j.rides[i - 1].alight.1.get());
            if let Some(f) = j.footpaths[i - 1] {
                cost += w.transfer_walk * f.seconds.get();
                reached += f.seconds.get();
            }
            cost += w.transfer;
            wait = w.transfer_wait;
        }
        cost += wait * (f64::from(ride.board.1.get()) - reached);
        cost += w.in_vehicle * f64::from(ride.alight.1.get() - ride.board.1.get());
    }
    cost
}

/// The exhaustive enumeration.
struct Brute<'a> {
    rows: &'a Rows,
    slack: f64,
    q: &'a CostQuery,
    memo: HashMap<(usize, u64, u8, bool), f64>,
}

impl Brute<'_> {
    /// The least cost still to pay from `stop`, there at `t`, with `left` rides left, `first`
    /// when no ride was taken yet; infinite when no journey ends from here.
    fn from(&mut self, stop: usize, t: f64, left: u8, first: bool) -> f64 {
        if left == 0 {
            return f64::INFINITY;
        }
        if let Some(&c) = self.memo.get(&(stop, t.to_bits(), left, first)) {
            return c;
        }
        let (rows, w) = (self.rows, self.q.weights);
        let mut best = f64::INFINITY;
        for (_, calls) in &rows.runs {
            for (i, &(s, _, dep)) in calls.iter().enumerate() {
                if s != stop || t + self.slack > f64::from(dep) {
                    continue;
                }
                let board = if first {
                    w.first_wait * (f64::from(dep) - t)
                } else {
                    w.transfer_wait * (f64::from(dep) - t) + w.transfer
                };
                for &(to, arr, _) in &calls[i + 1..] {
                    let ridden = board + w.in_vehicle * f64::from(arr - dep);
                    for (e, &(stop_e, _)) in self.q.query.egress.iter().enumerate() {
                        if stop_e.index() == to {
                            best = best.min(ridden + self.q.egress_cost[e]);
                        }
                    }
                    best = best.min(ridden + self.from(to, f64::from(arr), left - 1, false));
                    for f in rows.footpaths.iter().filter(|f| f.0 == to) {
                        let on = self.from(f.1, f64::from(arr) + f.2, left - 1, false);
                        best = best.min(ridden + w.transfer_walk * f.2 + on);
                    }
                }
            }
        }
        self.memo.insert((stop, t.to_bits(), left, first), best);
        best
    }

    /// The least cost of a journey of at most `rides` rides; infinite when there is none.
    fn least(&mut self, rides: u8) -> f64 {
        let q = self.q;
        let mut best = f64::INFINITY;
        for (&(stop, at), &cost) in q.query.access.iter().zip(&q.access_cost) {
            best = best.min(cost + self.from(stop.index(), f64::from(at.get()), rides, true));
        }
        best
    }
}

/// `k` distinct stops of `0..n`, in a random order.
fn distinct(draws: &mut Draws, n: usize, k: usize) -> Vec<usize> {
    let mut all: Vec<usize> = (0..n).collect();
    for i in 0..k {
        let j = i + draws.below(u32::try_from(n - i).unwrap()) as usize;
        all.swap(i, j);
    }
    all.truncate(k);
    all
}

/// A random timetable: 6 to 10 stops; 3 to 6 stop sequences of 2 to 5 distinct stops (a third
/// of those of three or more returning to their first stop at the end), rides of
/// 2 to 15 min and dwells of 0 to 60 s; 2 to 6 runs each leaving in the first 90 min, each run
/// on its own timing (so a later run may be faster, and runs may overtake); each ordered pair
/// of stops a footpath of 30 to 600 s with probability 0.2.
fn random_rows(draws: &mut Draws) -> Rows {
    let n = draws.between(6, 10) as usize;
    let mut rows = Rows { stops: n, ..Rows::default() };
    for p in 0..draws.between(3, 6) {
        let len = draws.between(2, 5) as usize;
        let mut stops = distinct(draws, n, len);
        // A third of the sequences come back to their first stop: a stop visited twice.
        if len > 2 && draws.below(3) == 0 {
            stops.push(stops[0]);
        }
        for r in 0..draws.between(2, 6) {
            let mut t = draws.below(5_400);
            let mut calls = Vec::new();
            for (i, &s) in stops.iter().enumerate() {
                if i > 0 {
                    t += draws.between(120, 900);
                }
                let dwell = draws.below(61);
                calls.push((s, t, t + dwell));
                t += dwell;
            }
            rows.runs.push((format!("p{p}r{r}"), calls));
        }
    }
    for a in 0..n {
        for b in 0..n {
            if a != b && draws.unit() < 0.2 {
                rows.footpaths.push((a, b, 30.0 + 570.0 * draws.unit()));
            }
        }
    }
    rows
}

/// A random question: 1 to 3 access stops reached in the first hour and 1 to 3 egress stops
/// with walks of 0 to 300 s, each with a cost of 0 to 1; at most 1 to 4 rides; weights of 0 to
/// 0.01 per second and 0 to 1 per transfer, in a third of the questions the default
/// proportions (waiting 1.45 times riding), in a third waiting below riding.
fn random_question(draws: &mut Draws, n: usize) -> CostQuery {
    let k = draws.between(1, 3) as usize;
    let access: Vec<_> = distinct(draws, n, k)
        .into_iter()
        .map(|s| (StopId::from_index(s), Second(draws.below(3_600))))
        .collect();
    let k = draws.between(1, 3) as usize;
    let egress: Vec<_> = distinct(draws, n, k)
        .into_iter()
        .map(|s| (StopId::from_index(s), Duration(300.0 * draws.unit())))
        .collect();
    let mut weights = Weights {
        first_wait: 0.01 * draws.unit(),
        in_vehicle: 0.01 * draws.unit(),
        transfer_wait: 0.01 * draws.unit(),
        transfer_walk: 0.01 * draws.unit(),
        transfer: draws.unit(),
    };
    match draws.below(3) {
        0 => {
            let per_min = |x: f64| x / 60.0;
            weights = Weights {
                first_wait: per_min(0.145),
                in_vehicle: per_min(0.1),
                transfer_wait: per_min(0.145),
                transfer_walk: per_min(0.165),
                transfer: 0.5,
            };
        }
        1 => {
            weights.first_wait = 0.5 * weights.in_vehicle;
            weights.transfer_wait = 0.25 * weights.in_vehicle;
        }
        _ => {}
    }
    let access_cost = access.iter().map(|_| draws.unit()).collect();
    let egress_cost = egress.iter().map(|_| draws.unit()).collect();
    let max_rides = u8::try_from(draws.between(1, 4)).unwrap();
    CostQuery { query: Query { access, egress, max_rides }, weights, access_cost, egress_cost }
}

#[test]
fn the_cheapest_journeys_equal_the_enumeration_on_300_random_timetables() {
    let mut draws = Draws::new(7);
    let (mut asked, mut answered, mut later_than_earliest) = (0, 0, 0);
    for _ in 0..300 {
        let rows = random_rows(&mut draws);
        let tt = rows.build();
        for slack in [0.0, 60.0] {
            let mut search = Cheapest::new(&tt, Duration(slack));
            for _ in 0..20 {
                let q = random_question(&mut draws, rows.stops);
                let found = search.by_rides(&q);
                let mut brute = Brute { rows: &rows, slack, q: &q, memo: HashMap::new() };
                // The enumeration's least cost at each number of rides, kept where cheaper
                // than every smaller number's.
                let mut want: Vec<(u8, f64)> = Vec::new();
                for k in 1..=q.query.max_rides {
                    let c = brute.least(k);
                    if c < want.last().map_or(f64::INFINITY, |w| w.1) - 1e-9 {
                        want.push((k, c));
                    }
                }
                asked += 1;
                answered += usize::from(!want.is_empty());
                assert_eq!(found.len(), want.len(), "{q:?}: {found:?} against {want:?}");
                for (j, &(k, c)) in found.iter().zip(&want) {
                    check_journey(&rows, &tt, slack, &q.query, j);
                    assert!(j.ride_count() <= usize::from(k), "{j:?}: more than {k} rides");
                    let cost = cost_of(&q, j);
                    assert!((cost - c).abs() < 1e-9, "{q:?}: {j:?} costs {cost}, least {c}");
                }
                // How often the cheapest is not the earliest: the reason for this search.
                if let Some(last) = found.last() {
                    let earliest = openmobisim_transit::raptor::Raptor::new(&tt, Duration(slack))
                        .earliest(&q.query)
                        .expect("a journey");
                    later_than_earliest += usize::from(last.arrival > earliest.arrival);
                }
            }
        }
    }
    println!(
        "{asked} questions, {answered} with a journey; the cheapest arrives after the earliest in {later_than_earliest}"
    );
    assert!(answered > asked / 4 && later_than_earliest > 100, "the cases are exercised");
}

#[test]
fn a_slow_run_boarded_at_once_is_cheaper_than_a_fast_run_waited_for() {
    // Stops A and B. A slow run leaves A at 60 s and reaches B at 720 s; a fast run leaves A
    // at 600 s and reaches B at 660 s. A traveller at A at second 0, slack 60 s, waiting at
    // 0.145 a minute and riding at 0.1: the fast run costs 10 · 0.145 + 1 · 0.1 = 1.55, the
    // slow run 1 · 0.145 + 11 · 0.1 = 1.245. The earliest arrival is the fast run's; the
    // cheapest journey is the slow run's.
    let mut rows = Rows { stops: 2, ..Rows::default() };
    rows.runs.push(("slow".to_owned(), vec![(0, 60, 60), (1, 720, 720)]));
    rows.runs.push(("fast".to_owned(), vec![(0, 600, 600), (1, 660, 660)]));
    let tt = rows.build();
    let q = CostQuery {
        query: common::stop_to_stop(0, 0, 1, 4),
        weights: Weights {
            first_wait: 0.145 / 60.0,
            in_vehicle: 0.1 / 60.0,
            transfer_wait: 0.145 / 60.0,
            transfer_walk: 0.165 / 60.0,
            transfer: 0.5,
        },
        access_cost: vec![0.0],
        egress_cost: vec![0.0],
    };
    let found = Cheapest::new(&tt, Duration(60.0)).by_rides(&q);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].arrival, Second(720), "the slow run");
    assert!((cost_of(&q, &found[0]) - 1.245).abs() < 1e-12, "{}", cost_of(&q, &found[0]));
    let earliest = openmobisim_transit::raptor::Raptor::new(&tt, Duration(60.0))
        .earliest(&q.query)
        .expect("a journey");
    assert_eq!(earliest.arrival, Second(660), "the fast run");
    assert!((cost_of(&q, &earliest) - 1.55).abs() < 1e-12);
}

/// The answer to `q` on `rows` with the access at place 0 of the query, and without it.
fn with_and_without_the_first_access(rows: &Rows, q: &CostQuery) -> (Vec<Journey>, Vec<Journey>) {
    let tt = rows.build();
    let mut without = q.clone();
    without.query.access.remove(0);
    without.access_cost.remove(0);
    let mut search = Cheapest::new(&tt, Duration(0.0));
    (search.by_rides(q), search.by_rides(&without))
}

#[test]
fn an_access_no_cheapest_journey_uses_does_not_change_the_answer() {
    // A property (invariance): removing an access from which every journey costs more than the
    // cheapest leaves the answer as it was. Two runs of one pattern leave B at 300 s and reach C
    // at 400 s; boarded at B by a traveller there at second 0 for nothing, they cost the same to
    // the bit. A dear access (cost 10, against 0.89 for a journey from B) can catch only
    // some of the runs upstream.
    let weights = Weights {
        first_wait: 0.145 / 60.0,
        in_vehicle: 0.1 / 60.0,
        transfer_wait: 0.145 / 60.0,
        transfer_walk: 0.165 / 60.0,
        transfer: 0.5,
    };
    let at = |s: usize, t: u32| (StopId::from_index(s), Second(t));
    // Stops A, B, C. The dear access at A catches the later run alone.
    let mut late = Rows { stops: 3, ..Rows::default() };
    late.runs.push(("r0".to_owned(), vec![(0, 100, 100), (1, 300, 300), (2, 400, 400)]));
    late.runs.push(("r1".to_owned(), vec![(0, 200, 200), (1, 300, 300), (2, 400, 400)]));
    let late_q = CostQuery {
        query: Query {
            access: vec![at(0, 150), at(1, 0)],
            egress: vec![(StopId::from_index(2), Duration(0.0))],
            max_rides: 1,
        },
        weights,
        access_cost: vec![10.0, 0.0],
        egress_cost: vec![0.0],
    };
    // Stops A, A2, B, C. The dear access at A catches the earlier run (and the later); a
    // second access at A2, kept in both, catches the later run alone.
    let mut early = Rows { stops: 4, ..Rows::default() };
    early
        .runs
        .push(("r0".to_owned(), vec![(0, 100, 100), (1, 150, 150), (2, 300, 300), (3, 400, 400)]));
    early
        .runs
        .push(("r1".to_owned(), vec![(0, 120, 120), (1, 250, 250), (2, 300, 300), (3, 400, 400)]));
    let early_q = CostQuery {
        query: Query {
            access: vec![at(0, 50), at(1, 200), at(2, 0)],
            egress: vec![(StopId::from_index(3), Duration(0.0))],
            max_rides: 1,
        },
        weights,
        access_cost: vec![10.0, 10.0, 0.0],
        egress_cost: vec![0.0],
    };
    for (case, rows, q) in [("later run", &late, &late_q), ("earlier run", &early, &early_q)] {
        let (with, without) = with_and_without_the_first_access(rows, q);
        assert_eq!(with.len(), 1, "{case}: one journey");
        assert_eq!(with, without, "{case}: the dear access changed the answer");
    }
}
