//! The journey search must not depend on the ids of stops, lines and runs (S312).
//!
//! **The cases.** Small timetables whose stops stand at distinct points, each searched twice: with
//! the stops, lines and runs named in one order and with the names reversed, so that their dense
//! ids, which follow the sorted names, come in the other order. The coordinates and the times
//! are the same. Every journey must be the same under both namings, read back by where its stops
//! are:
//!
//! - **Two lines arrive together** (`O → C → B` and `O → D → B`, both leaving `O` at 100 s and
//!   reaching `B` at 400 s): RAPTOR keeps the journey of the pattern it scans first, which was the
//!   one whose first run had the lower id (its trip's name). It is now the one whose stops come
//!   first by where they are (`C`, the western, before `D`).
//! - **Two egress stops tie** (lines to `E1` and to `E2`, both 300 s, both a 60 s walk from the
//!   destination): the tie went to the lower stop id; it now goes to the stop of the lower rank
//!   (the western).
//! - **The pattern groups** (two lines, two runs each): the groups were numbered by their
//!   smallest run id, and the runs that ride the roads are loaded in that order; they are now
//!   ordered by their stops' ranks, their line's key and their times.
//! - **Two stops at one point** (two stations' stops, as real feeds have): they share their rank
//!   and their key, so neither the lines through them nor the journey kept follows their ids.

use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_transit::{
    ALIGHT, BOARD, CallSpec, Footpaths, Journey, JourneyLeg, Raptor, RaptorData, RouteSpec,
    ServiceDate, StopSpec, Timetable, TimetableBuilder,
};
use openmobisim_core_types::ids::NodeId;

/// The stops' points, by their place in the case (not by id).
fn points(n: u32) -> Vec<LonLat> {
    (0..n)
        .map(|i| LonLat::new(4.80 + 0.001 * f64::from(i), 45.70 + 0.0007 * f64::from((i * 7) % 5)))
        .collect()
}

/// A timetable of `n` stops and `runs` (each a line's index and its calls as `(stop, time)` by
/// the stops' places), named `prefix` + a number counted up (`reversed`: down), so the dense ids
/// come in one order or the other.
fn timetable(
    n: usize,
    lines: usize,
    runs: &[(usize, Vec<(usize, u32)>)],
    reversed: bool,
) -> Timetable {
    timetable_at(&points(u32::try_from(n).unwrap()), lines, runs, reversed)
}

/// [`timetable`] with the stops at `at`.
fn timetable_at(
    at: &[LonLat],
    lines: usize,
    runs: &[(usize, Vec<(usize, u32)>)],
    reversed: bool,
) -> Timetable {
    let n = at.len();
    let name = |what: &str, i: usize, count: usize| {
        let k = if reversed { count - 1 - i } else { i };
        format!("{what}{k:03}")
    };
    let mut b = TimetableBuilder::new(ServiceDate::parse("20261009").unwrap());
    // Added in the case's order: the builder's indices are the places.
    for (i, &p) in at.iter().enumerate() {
        b.add_stop(StopSpec {
            external_id: name("s", i, n),
            name: String::new(),
            position: p,
            parent: None,
        });
    }
    for l in 0..lines {
        b.add_route(RouteSpec {
            external_id: name("l", l, lines),
            short_name: String::new(),
            route_type: 3,
        });
    }
    for (r, (line, calls)) in runs.iter().enumerate() {
        let calls: Vec<CallSpec> = calls
            .iter()
            .map(|&(s, t)| CallSpec {
                stop: u32::try_from(s).unwrap(),
                arrival: t,
                departure: t,
                flags: BOARD | ALIGHT,
            })
            .collect();
        b.add_run(name("t", r, runs.len()), u32::try_from(*line).unwrap(), &calls);
    }
    b.build().0
}

/// The dense id of the stop at place `i`.
fn id(t: &Timetable, i: usize, n: usize, reversed: bool) -> NodeId {
    let k = if reversed { n - 1 - i } else { i };
    t.stop_ids().typed_id_of(&format!("s{k:03}")).unwrap()
}

/// A journey read back by where its stops are: per leg, the points of the stops it passes
/// (a ride: every stop of its run from boarding to alighting), and its arrival.
fn by_place(t: &Timetable, j: &Journey) -> (u32, Vec<Vec<(u64, u64)>>) {
    let pt = |s: NodeId| {
        let p = t.stop_position(s);
        (p.lon.to_bits(), p.lat.to_bits())
    };
    let legs = j
        .legs
        .iter()
        .map(|l| match *l {
            JourneyLeg::Access { stop, .. } | JourneyLeg::Egress { stop, .. } => vec![pt(stop)],
            JourneyLeg::Ride { board_call, alight_call, .. } => {
                (board_call..=alight_call).map(|c| pt(t.call_stop(c as usize))).collect()
            }
            JourneyLeg::Transfer { from, to, .. } => vec![pt(from), pt(to)],
        })
        .collect();
    (j.arrival, legs)
}

#[test]
fn of_two_lines_arriving_together_the_same_is_kept_under_any_names() {
    // Places: 0 = O, 1 = C, 2 = D, 3 = B (C west of D).
    let runs =
        vec![(0, vec![(0, 100), (1, 250), (3, 400)]), (1, vec![(0, 100), (2, 250), (3, 400)])];
    let found: Vec<_> = [false, true]
        .into_iter()
        .map(|rev| {
            let t = timetable(4, 2, &runs, rev);
            let data = RaptorData::new(&t, t.scheduled(), Footpaths::none(t.stop_count()), 0);
            let mut r = Raptor::new(&data, 2);
            let j = r
                .earliest(&[(id(&t, 0, 4, rev), 0)], &[(id(&t, 3, 4, rev), 0)])
                .expect("a journey");
            by_place(&t, &j)
        })
        .collect();
    assert_eq!(found[0].0, 400);
    assert_eq!(found[0], found[1], "the journey kept follows the names");
    // The western intermediate stop's line: C's point is in the ride.
    let c = points(4)[1];
    assert!(found[0].1[1].contains(&(c.lon.to_bits(), c.lat.to_bits())), "{:?}", found[0]);
}

#[test]
fn of_two_egress_stops_that_tie_the_same_is_taken_under_any_names() {
    // Places: 0 = O, 1 = E1, 2 = E2 (E1 west of E2); both 60 s from the destination.
    let runs = vec![(0, vec![(0, 100), (1, 300)]), (1, vec![(0, 100), (2, 300)])];
    let found: Vec<_> = [false, true]
        .into_iter()
        .map(|rev| {
            let t = timetable(3, 2, &runs, rev);
            let data = RaptorData::new(&t, t.scheduled(), Footpaths::none(t.stop_count()), 0);
            let mut r = Raptor::new(&data, 2);
            // The egress stops listed in id order, as a caller's walk search might give them.
            let mut egress = vec![(id(&t, 1, 3, rev), 60), (id(&t, 2, 3, rev), 60)];
            egress.sort_unstable();
            let j = r.earliest(&[(id(&t, 0, 3, rev), 0)], &egress).expect("a journey");
            by_place(&t, &j)
        })
        .collect();
    assert_eq!(found[0].0, 360);
    assert_eq!(found[0], found[1], "the egress stop taken follows the names");
    let e1 = points(3)[1];
    assert_eq!(found[0].1.last().unwrap()[0], (e1.lon.to_bits(), e1.lat.to_bits()));
}

#[test]
fn stop_ranks_and_line_keys_do_not_follow_the_names() {
    let runs =
        vec![(0, vec![(0, 100), (1, 250), (3, 400)]), (1, vec![(0, 100), (2, 250), (3, 400)])];
    let (a, b) = (timetable(4, 2, &runs, false), timetable(4, 2, &runs, true));
    for i in 0..4 {
        let (sa, sb) = (id(&a, i, 4, false), id(&b, i, 4, true));
        assert_eq!(a.stop_rank(sa), b.stop_rank(sb), "stop {i}'s rank");
        assert_eq!(a.stop_key(sa), b.stop_key(sb), "stop {i}'s key");
    }
    // Line l000 of one naming is line l001 of the other.
    let line = |t: &Timetable, l: usize, rev: bool| {
        let k = if rev { 1 - l } else { l };
        t.route_ids().id_of(&format!("l{k:03}")).unwrap()
    };
    for l in 0..2 {
        assert_eq!(
            a.route_key(line(&a, l, false)),
            b.route_key(line(&b, l, true)),
            "line {l}'s key"
        );
    }
    assert_ne!(a.route_key(0), a.route_key(1), "two lines through other stops differ");
}

/// The pattern groups and their runs, read back by where the runs call and when: the order the
/// runs that ride the roads are loaded in, and RAPTOR's patterns are split from.
fn groups_by_place(t: &Timetable) -> Vec<Vec<Vec<(u64, u64, u32)>>> {
    (0..t.group_count())
        .map(|g| {
            t.group_runs(g)
                .iter()
                .map(|&r| {
                    t.run_calls(r)
                        .map(|c| {
                            let p = t.stop_position(t.call_stop(c));
                            (p.lon.to_bits(), p.lat.to_bits(), t.scheduled().departure[c])
                        })
                        .collect()
                })
                .collect()
        })
        .collect()
}

#[test]
fn the_pattern_groups_and_their_runs_come_in_an_order_with_no_id_in_it() {
    // Two lines, two runs each, one of them at the same first departure as the other line's.
    let runs = vec![
        (0, vec![(0, 100), (1, 250), (3, 400)]),
        (1, vec![(0, 100), (2, 250), (3, 400)]),
        (0, vec![(0, 700), (1, 850), (3, 1000)]),
        (1, vec![(0, 400), (2, 550), (3, 700)]),
    ];
    let (a, b) = (timetable(4, 2, &runs, false), timetable(4, 2, &runs, true));
    assert_eq!(a.group_count(), 2);
    assert_eq!(groups_by_place(&a), groups_by_place(&b), "the groups follow the names");
}

#[test]
fn stops_at_one_point_are_told_apart_by_their_lines_not_their_ids() {
    // Places 1 and 2 at one point (two stations' stops, as Delft's feed has); 0 = O, 3 = C,
    // 4 = D, 5 = B. Line 0: O → X1 → C → B, line 1: O → X2 → D → B, both at B at 500 s.
    let n = 6;
    let mut at = points(6);
    at[2] = at[1];
    let runs = vec![
        (0, vec![(0, 100), (1, 200), (3, 350), (5, 500)]),
        (1, vec![(0, 100), (2, 200), (4, 350), (5, 500)]),
    ];
    let build = |rev: bool| {
        let t = timetable_at(&at, 2, &runs, rev);
        let data = RaptorData::new(&t, t.scheduled(), Footpaths::none(t.stop_count()), 0);
        let mut r = Raptor::new(&data, 2);
        let j =
            r.earliest(&[(id(&t, 0, n, rev), 0)], &[(id(&t, 5, n, rev), 0)]).expect("a journey");
        let keys: Vec<u64> = (0..2)
            .map(|l| {
                let k = if rev { 1 - l } else { l };
                t.route_key(t.route_ids().id_of(&format!("l{k:03}")).unwrap())
            })
            .collect();
        let ranks: Vec<u32> = (0..n).map(|i| t.stop_rank(id(&t, i, n, rev))).collect();
        (by_place(&t, &j), keys, ranks)
    };
    let (a, b) = (build(false), build(true));
    assert_eq!(a.2[1], a.2[2], "stops at one point share their rank");
    assert_eq!(a.1, b.1, "the lines' keys follow the stops' names");
    assert_eq!(a.0, b.0, "the journey kept follows the stops' names");
    let c = at[3];
    assert!(a.0.1[1].contains(&(c.lon.to_bits(), c.lat.to_bits())), "through C, west of D");
}
