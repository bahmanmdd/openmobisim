//! RAPTOR against every journey enumerated (test 8 of the specification, its second half): 300
//! random timetables of 6 to 12 stops, 3 to 6 stop sequences of 2 to 6 runs (in about half the
//! timetables each run on its own timing, so some overtake), random footpaths, a slack
//! of 0 or 60 s; five queries each, from 1 to 3 access stops to 1 to 3 egress stops, at most 1
//! to 4 rides. The brute force (`common::brute_force`) enumerates every journey by the
//! definition and keeps the earliest arrival, then the fewest rides, then the lowest last run
//! id; RAPTOR must give the same three, or both nothing, and its journey must check out against
//! the rows on its own. On the same queries, the journeys by number of rides (`by_rides`): at
//! each count `k` of rides, the brute force at most `k` rides gives the earliest arrival with
//! at most `k`; `by_rides` must hold exactly the counts that arrive earlier than every smaller
//! one, each with the brute force's arrival, rides and last run.

mod common;

use common::{Draws, Outcome, Rows, brute_force, check_journey};
use openmobisim_transit::raptor::{Query, Raptor};
use openmobisim_transit::timetable::Timetable;
use openmobisim_types::ids::{EntityId, StopId};
use openmobisim_types::time::Second;
use openmobisim_types::units::Duration;

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

/// A random timetable: 6 to 12 stops; 3 to 6 stop sequences of 2 to 6 distinct stops (no two
/// sequences on the same stops), rides 2 to 15 min and dwells 0 to 60 s; 2 to 6 runs each,
/// leaving the first stop in the first 90 min, all on one timing per sequence or, in about half
/// the timetables, each on its own; each ordered pair of stops a footpath with probability 0.2, of 30 to 600 s.
fn random_rows(draws: &mut Draws) -> Rows {
    let n = draws.between(6, 12) as usize;
    let mut rows = Rows { stops: n, ..Rows::default() };
    let mut sequences: Vec<Vec<usize>> = Vec::new();
    let patterns = draws.between(3, 6) as usize;
    while sequences.len() < patterns {
        let len = draws.between(2, 6.min(u32::try_from(n).unwrap())) as usize;
        let s = distinct(draws, n, len);
        if !sequences.contains(&s) {
            sequences.push(s);
        }
    }
    // In about half the timetables each run has its own timing, so runs on the same stops may
    // overtake (and go to patterns of their own); in the others a pattern's runs share theirs.
    let own_timing = draws.below(2) == 0;
    for (p, stops) in sequences.iter().enumerate() {
        let mut timing: Vec<(u32, u32)> =
            (0..stops.len()).map(|_| (draws.between(120, 900), draws.below(61))).collect();
        for r in 0..draws.between(2, 6) {
            if own_timing && r > 0 {
                timing =
                    (0..stops.len()).map(|_| (draws.between(120, 900), draws.below(61))).collect();
            }
            let mut t = draws.below(5_400);
            let mut calls = Vec::new();
            for (i, &s) in stops.iter().enumerate() {
                if i > 0 {
                    t += timing[i].0;
                }
                calls.push((s, t, t + timing[i].1));
                t += timing[i].1;
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

/// Whether two runs on the same stops overtake: neither is at or behind the other at every call.
fn overtakes(rows: &Rows) -> bool {
    let runs = &rows.runs;
    let behind = |a: &[common::RowCall], b: &[common::RowCall]| {
        a.iter().zip(b).all(|(x, y)| y.1 >= x.1 && y.2 >= x.2)
    };
    runs.iter().enumerate().any(|(i, (_, a))| {
        runs[i + 1..].iter().any(|(_, b)| {
            let same = a.iter().map(|c| c.0).eq(b.iter().map(|c| c.0));
            same && !behind(a, b) && !behind(b, a)
        })
    })
}

/// A random query: 1 to 3 access stops reached in the first hour, 1 to 3 egress stops with
/// walks of 0 to 300 s, at most 1 to 4 rides.
fn random_query(draws: &mut Draws, n: usize) -> Query {
    let k = draws.between(1, 3) as usize;
    let access = distinct(draws, n, k)
        .into_iter()
        .map(|s| (StopId::from_index(s), Second(draws.below(3_600))))
        .collect();
    let k = draws.between(1, 3) as usize;
    let egress = distinct(draws, n, k)
        .into_iter()
        .map(|s| (StopId::from_index(s), Duration(300.0 * draws.unit())))
        .collect();
    Query { access, egress, max_rides: u8::try_from(draws.between(1, 4)).unwrap() }
}

/// `by_rides` against the brute force at each ride count `k` from 1 to the query's most: the
/// brute force at most `k` rides gives the earliest arrival with at most `k` rides (and, among
/// those, the fewest rides and the lowest last run); `by_rides` must give exactly the answers
/// whose arrival is earlier than every smaller `k`'s, fewest rides first.
fn by_rides_agree(
    rows: &Rows,
    tt: &Timetable,
    slack: f64,
    q: &Query,
    raptor: &mut Raptor<'_>,
) -> usize {
    let mut expected = Vec::new();
    for k in 1..=q.max_rides {
        let at_most_k = Query { max_rides: k, ..q.clone() };
        if let Some(o) = brute_force(rows, tt, slack, &at_most_k)
            && expected.last().is_none_or(|&(a, _, _)| o.0 < a)
        {
            expected.push(o);
        }
    }
    let got = raptor.by_rides(q);
    for j in &got {
        check_journey(rows, tt, slack, q, j);
    }
    let outcome: Vec<Outcome> = got
        .iter()
        .map(|j| (j.arrival.get(), j.rides.len(), j.rides.last().expect("a ride").run.raw()))
        .collect();
    assert_eq!(outcome, expected, "by rides, slack {slack} s, {q:?}");
    got.len()
}

#[test]
fn raptor_equals_the_brute_force_on_300_random_timetables() {
    let mut draws = Draws::new(8);
    let (mut queries, mut answered, mut transfers, mut walked, mut overtaking) = (0, 0, 0, 0, 0);
    let mut several = 0;
    for t in 0..300 {
        let rows = random_rows(&mut draws);
        let tt = rows.build();
        if overtakes(&rows) {
            overtaking += 1;
            // Patterns hold no overtaking pair, so this timetable has more of them than stop
            // sequences.
            let mut sequences: Vec<Vec<usize>> =
                rows.runs.iter().map(|r| r.1.iter().map(|c| c.0).collect()).collect();
            sequences.sort();
            sequences.dedup();
            assert!(tt.pattern_count() as usize > sequences.len(), "timetable {t}");
        }
        let slack = if draws.below(2) == 0 { 0.0 } else { 60.0 };
        let mut raptor = Raptor::new(&tt, Duration(slack));
        for _ in 0..5 {
            let q = random_query(&mut draws, rows.stops);
            let expected = brute_force(&rows, &tt, slack, &q);
            let got = raptor.earliest(&q);
            let outcome = got.as_ref().map(|j| {
                (j.arrival.get(), j.rides.len(), j.rides.last().expect("a ride").run.raw())
            });
            assert_eq!(outcome, expected, "timetable {t}, slack {slack} s, {q:?}");
            several += usize::from(by_rides_agree(&rows, &tt, slack, &q, &mut raptor) >= 2);
            if let Some(j) = &got {
                check_journey(&rows, &tt, slack, &q, j);
                answered += 1;
                transfers += usize::from(j.rides.len() >= 2);
                walked += usize::from(j.footpaths.iter().any(Option::is_some));
            }
            queries += 1;
        }
    }
    eprintln!(
        "{overtaking} of 300 timetables hold runs that overtake; {queries} queries: {answered} with a journey, {transfers} of two rides or more, {walked} with a footpath; {several} with two journeys or more by number of rides"
    );
    // The test tests little unless many queries have a journey and many of those transfer.
    assert!(answered >= queries / 2, "only {answered} of {queries} queries have a journey");
    assert!(transfers >= queries / 10, "only {transfers} of {queries} journeys transfer");
}
