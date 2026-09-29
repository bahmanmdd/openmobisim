//! RAPTOR: hand cases, and a property test against a brute-force search.
//!
//! What is defended: the earliest arrival at the destination, and among the
//! journeys arriving then the fewest vehicles, on any timetable — with runs that
//! overtake each other, calls without boarding or alighting, and walking
//! transfers that are not transitively closed; and that every journey returned
//! is one a passenger could make.

use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_transit::{
    ALIGHT, BOARD, CallSpec, Footpaths, Journey, JourneyLeg, Raptor, RaptorData, RouteSpec,
    ServiceDate, StopSpec, Timetable, TimetableBuilder, UNKNOWN_TIME,
};
use openmobisim_core_types::ids::{EntityId, NodeId};
use proptest::prelude::*;

const BOTH: u8 = BOARD | ALIGHT;

/// A run: its route, and its calls as `(stop, time, flags)`.
type RunCalls = (u32, Vec<(u32, u32, u8)>);
/// A random case: stops, runs, walks `(from, to, seconds)`, origin and destination stops.
type Case = (u32, Vec<RunCalls>, Vec<(u32, u32, u32)>, u32, u32);

/// A timetable of stops `s0..s{n-1}` and runs given as `(route, [(stop, time, flags)])`,
/// with arrival and departure equal.
fn timetable(stops: u32, runs: &[RunCalls]) -> Timetable {
    let mut b = TimetableBuilder::new(ServiceDate::parse("20261009").unwrap());
    // Zero-padded names, so the sorted external ids keep the numbering.
    for s in 0..stops {
        b.add_stop(StopSpec {
            external_id: format!("s{s:03}"),
            name: String::new(),
            position: LonLat::new(4.9, 52.37),
            parent: None,
        });
    }
    let routes = runs.iter().map(|r| r.0).max().map_or(0, |m| m + 1);
    for r in 0..routes {
        b.add_route(RouteSpec {
            external_id: format!("r{r:03}"),
            short_name: r.to_string(),
            route_type: 3,
        });
    }
    for (i, (route, calls)) in runs.iter().enumerate() {
        let calls: Vec<CallSpec> = calls
            .iter()
            .map(|&(stop, t, flags)| CallSpec { stop, arrival: t, departure: t, flags })
            .collect();
        b.add_run(format!("t{i:04}"), *route, &calls);
    }
    b.build().0
}

fn stop(s: u32) -> NodeId {
    NodeId::new(s)
}

fn rides(j: &Journey) -> Vec<(u32, u32, u32, u32)> {
    j.legs
        .iter()
        .filter_map(|l| match *l {
            JourneyLeg::Ride { board_stop, alight_stop, departure, arrival, .. } => {
                Some((board_stop.raw(), alight_stop.raw(), departure, arrival))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn a_change_needs_the_boarding_slack() {
    // Line A: s0 100 → s1 200. Line B at s1: one run at 230 (too soon after 200 with a
    // 60 s slack), one at 260 (just enough), each to s2 in 100 s.
    let t = timetable(
        3,
        &[
            (0, vec![(0, 100, BOTH), (1, 200, BOTH)]),
            (1, vec![(1, 230, BOTH), (2, 330, BOTH)]),
            (1, vec![(1, 260, BOTH), (2, 360, BOTH)]),
        ],
    );
    let data = RaptorData::new(&t, t.scheduled(), Footpaths::none(3), 60);
    let mut raptor = Raptor::new(&data, 8);
    let j = raptor.earliest(&[(stop(0), 30)], &[(stop(2), 0)]).expect("a journey");
    assert_eq!(j.arrival, 360);
    assert_eq!(rides(&j), [(0, 1, 100, 200), (1, 2, 260, 360)]);
    // Too late for the first run at s0 (arriving at 41 + 60 > 100): nothing.
    assert!(raptor.earliest(&[(stop(0), 41)], &[(stop(2), 0)]).is_none());
    // Exactly the slack: boards.
    assert!(raptor.earliest(&[(stop(0), 40)], &[(stop(2), 0)]).is_some());
}

#[test]
fn a_faster_later_run_is_split_off_and_found() {
    // Same route and stops; the express leaves later and arrives earlier.
    let t = timetable(
        2,
        &[(0, vec![(0, 100, BOTH), (1, 1000, BOTH)]), (0, vec![(0, 200, BOTH), (1, 400, BOTH)])],
    );
    assert_eq!(t.group_count(), 1);
    let data = RaptorData::new(&t, t.scheduled(), Footpaths::none(2), 0);
    assert_eq!(data.pattern_count(), 2, "the overtaking run is a pattern of its own");
    let mut raptor = Raptor::new(&data, 8);
    let j = raptor.earliest(&[(stop(0), 0)], &[(stop(1), 0)]).unwrap();
    assert_eq!(j.arrival, 400);
}

#[test]
fn no_boarding_or_alighting_where_the_feed_says_not() {
    // s0 → s1 → s2; no alighting at s1, no boarding at s1.
    let t = timetable(
        3,
        &[
            (0, vec![(0, 100, BOARD), (1, 200, BOARD), (2, 300, ALIGHT)]),
            (1, vec![(0, 100, BOTH), (1, 200, ALIGHT), (2, 300, BOTH)]),
        ],
    );
    let data = RaptorData::new(&t, t.scheduled(), Footpaths::none(3), 0);
    let mut raptor = Raptor::new(&data, 8);
    // To s1: only the second run lets you off there.
    let j = raptor.earliest(&[(stop(0), 0)], &[(stop(1), 0)]).unwrap();
    assert_eq!(
        t.run_ids().external(match j.legs[1] {
            JourneyLeg::Ride { run, .. } => run.raw(),
            _ => unreachable!(),
        }),
        "t0001"
    );
    // From s1: the second run does not take passengers there, the first does.
    let j = raptor.earliest(&[(stop(1), 0)], &[(stop(2), 0)]).unwrap();
    assert_eq!(rides(&j), [(1, 2, 200, 300)]);
}

#[test]
fn a_walk_from_a_stop_reached_earlier_by_walking_is_not_lost() {
    // s0 → s1 by run A (arrives 200) and s0 → s2 by run B (arrives 100); walks s2→s1
    // (50 s) and s1→s3 (50 s), none s2→s3. The best at s1 is by walking from s2 (150);
    // the only way to s3 is to ride to s1 and walk on (250). Textbook RAPTOR with
    // non-closed footpaths drops the ride to s1 (not better than 150) and never
    // reaches s3.
    let t = timetable(
        4,
        &[(0, vec![(0, 10, BOTH), (1, 200, BOTH)]), (1, vec![(0, 10, BOTH), (2, 100, BOTH)])],
    );
    let walks = Footpaths::new(4, vec![(stop(2), stop(1), 50), (stop(1), stop(3), 50)]);
    let data = RaptorData::new(&t, t.scheduled(), walks, 0);
    let mut raptor = Raptor::new(&data, 8);
    let j = raptor.earliest(&[(stop(0), 0)], &[(stop(3), 0)]).expect("ride to s1, walk on");
    assert_eq!(j.arrival, 250);
    assert_eq!(rides(&j), [(0, 1, 10, 200)]);
}

#[test]
fn among_equal_arrivals_the_fewest_vehicles() {
    // Direct s0 → s2 arriving 500; or s0 → s1 → s2 arriving 500 too.
    let t = timetable(
        3,
        &[
            (0, vec![(0, 100, BOTH), (1, 200, BOTH)]),
            (1, vec![(1, 300, BOTH), (2, 500, BOTH)]),
            (2, vec![(0, 150, BOTH), (2, 500, BOTH)]),
        ],
    );
    let data = RaptorData::new(&t, t.scheduled(), Footpaths::none(3), 0);
    let mut raptor = Raptor::new(&data, 8);
    let j = raptor.earliest(&[(stop(0), 0)], &[(stop(2), 0)]).unwrap();
    assert_eq!((j.arrival, j.rides()), (500, 1));
}

#[test]
fn unknown_realised_times_cannot_be_boarded_or_alighted() {
    let t = timetable(3, &[(0, vec![(0, 100, BOTH), (1, 200, BOTH), (2, 300, BOTH)])]);
    let mut times = t.scheduled().clone();
    times.arrival[2] = UNKNOWN_TIME;
    times.departure[2] = UNKNOWN_TIME;
    let data = RaptorData::new(&t, &times, Footpaths::none(3), 0);
    let mut raptor = Raptor::new(&data, 8);
    assert!(raptor.earliest(&[(stop(0), 0)], &[(stop(2), 0)]).is_none());
    assert!(raptor.earliest(&[(stop(0), 0)], &[(stop(1), 0)]).is_some());
}

/// The earliest arrival and its fewest vehicles by exhaustive dynamic programming
/// over every run, boarding and alighting: no patterns, no FIFO, no pruning.
fn brute_force(
    t: &Timetable,
    walks: &[(u32, u32, u32)],
    slack: u32,
    max_rides: usize,
    access: &[(u32, u32)],
    egress: &[(u32, u32)],
) -> Option<(u32, usize)> {
    let n = t.stop_count() as usize;
    let times = t.scheduled();
    let mut reach = vec![u32::MAX; n]; // best with ≤ k − 1 vehicles
    for &(s, a) in access {
        reach[s as usize] = reach[s as usize].min(a);
    }
    let mut best: Option<(u32, usize)> = None;
    for k in 1..=max_rides {
        let mut by_ride = vec![u32::MAX; n];
        for r in 0..t.run_count() {
            let calls: Vec<usize> =
                t.run_calls(openmobisim_core_types::ids::TransitRunId::new(r)).collect();
            for (i, &ci) in calls.iter().enumerate() {
                let s = t.call_stop(ci).index();
                if t.call_flags(ci) & BOARD == 0 || reach[s] == u32::MAX {
                    continue;
                }
                if reach[s].saturating_add(slack) > times.departure[ci] {
                    continue;
                }
                for &cj in &calls[i + 1..] {
                    if t.call_flags(cj) & ALIGHT != 0 {
                        let s2 = t.call_stop(cj).index();
                        by_ride[s2] = by_ride[s2].min(times.arrival[cj]);
                    }
                }
            }
        }
        let mut after = by_ride.clone();
        for &(f, to, w) in walks {
            if f != to && by_ride[f as usize] != u32::MAX {
                let v = by_ride[f as usize] + w;
                after[to as usize] = after[to as usize].min(v);
            }
        }
        for &(e, w) in egress {
            let a = after[e as usize];
            if a != u32::MAX {
                let at = a + w;
                if best.is_none_or(|(b, _)| at < b) {
                    best = Some((at, k));
                }
            }
        }
        for s in 0..n {
            reach[s] = reach[s].min(after[s]);
        }
    }
    best
}

fn arb_case() -> impl Strategy<Value = Case> {
    (3u32..8).prop_flat_map(|n| {
        let run = (0u32..3, prop::collection::vec((0..n, 1u32..120, 1u8..4), 2..6), 0u32..600)
            .prop_map(|(route, steps, start)| {
                let mut t = start;
                let calls = steps
                    .into_iter()
                    .map(|(s, dt, flags)| {
                        t += dt;
                        (s, t, flags)
                    })
                    .collect::<Vec<_>>();
                (route, calls)
            });
        (
            Just(n),
            prop::collection::vec(run, 1..14),
            prop::collection::vec((0..n, 0..n, 0u32..200), 0..8),
            0..n,
            0..n,
        )
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(600))]

    /// RAPTOR's earliest arrival and vehicle count equal the exhaustive search's,
    /// and its journey is one a passenger could make.
    #[test]
    fn raptor_equals_brute_force((n, runs, walks, from, to) in arb_case(), slack in 0u32..90, depart in 0u32..700) {
        let t = timetable(n, &runs);
        let footpaths = Footpaths::new(
            n,
            walks.iter().map(|&(a, b, w)| (stop(a), stop(b), w)).collect(),
        );
        // The same walks, deduplicated as `Footpaths` keeps them.
        let mut kept: Vec<(u32, u32, u32)> = walks.clone();
        kept.retain(|w| w.0 != w.1);
        kept.sort_unstable();
        kept.dedup_by(|b, a| a.0 == b.0 && a.1 == b.1);
        let data = RaptorData::new(&t, t.scheduled(), footpaths, slack);
        let mut raptor = Raptor::new(&data, 4);
        let access = [(from, depart)];
        let egress = [(to, 7)];
        let expected = brute_force(&t, &kept, slack, 4, &access, &egress);
        let got = raptor.earliest(&[(stop(from), depart)], &[(stop(to), 7)]);
        prop_assert_eq!(got.as_ref().map(|j| (j.arrival, j.rides())), expected);
        if let Some(j) = got {
            // The legs chain: each starts no earlier than the one before ends.
            let mut clock = 0u32;
            let mut at_stop = None;
            for leg in &j.legs {
                match *leg {
                    JourneyLeg::Access { stop, arrival } => {
                        prop_assert_eq!((stop.raw(), arrival), (from, depart));
                        clock = arrival;
                        at_stop = Some(stop);
                    }
                    JourneyLeg::Ride { board_stop, alight_stop, departure, arrival, board_call, alight_call, .. } => {
                        prop_assert_eq!(Some(board_stop), at_stop);
                        prop_assert!(clock + slack <= departure, "boarded with the slack");
                        prop_assert!(t.call_flags(board_call as usize) & BOARD != 0);
                        prop_assert!(t.call_flags(alight_call as usize) & ALIGHT != 0);
                        prop_assert_eq!(t.call_stop(alight_call as usize), alight_stop);
                        clock = arrival;
                        at_stop = Some(alight_stop);
                    }
                    JourneyLeg::Transfer { from: f, to: s, seconds } => {
                        prop_assert_eq!(Some(f), at_stop);
                        prop_assert!(kept.iter().any(|w| (w.0, w.1, w.2) == (f.raw(), s.raw(), seconds)));
                        clock += seconds;
                        at_stop = Some(s);
                    }
                    JourneyLeg::Egress { stop, seconds } => {
                        prop_assert_eq!(Some(stop), at_stop);
                        prop_assert_eq!(stop.raw(), to);
                        prop_assert_eq!(clock + seconds, j.arrival);
                    }
                }
            }
        }
    }
}
