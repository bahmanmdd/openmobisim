//! Transfers, slack and footpaths on small timetables, every arrival worked out by hand; and
//! the journeys by number of rides (`Raptor::by_rides`) on the same timetables.
//!
//! Stops s00 to s05. Run P: s00 at 1 000 to s01 at 1 300. Runs Q1 and Q2: s01 to s02, leaving
//! at 1 330 and 1 930, 270 s each. Run R: s03 at 1 400 to s04 at 1 700. The traveller is at
//! s00 at 940.

mod common;

use common::{Rows, check_journey, stop_to_stop};
use openmobisim_transit::raptor::{Footpath, Journey, Query, Raptor};
use openmobisim_types::ids::{EntityId, StopId};
use openmobisim_types::time::Second;
use openmobisim_types::units::Duration;

fn rows(footpath: Option<f64>) -> Rows {
    let mut rows = Rows { stops: 6, ..Rows::default() };
    rows.runs.push(("P".into(), vec![(0, 1_000, 1_000), (1, 1_300, 1_300)]));
    rows.runs.push(("Q1".into(), vec![(1, 1_330, 1_330), (2, 1_600, 1_600)]));
    rows.runs.push(("Q2".into(), vec![(1, 1_930, 1_930), (2, 2_200, 2_200)]));
    rows.runs.push(("R".into(), vec![(3, 1_400, 1_400), (4, 1_700, 1_700)]));
    if let Some(s) = footpath {
        rows.footpaths.push((1, 3, s));
    }
    rows
}

fn ask(rows: &Rows, slack: f64, q: &Query) -> Option<Journey> {
    let tt = rows.build();
    let j = Raptor::new(&tt, Duration(slack)).earliest(q);
    if let Some(j) = &j {
        check_journey(rows, &tt, slack, q, j);
    }
    j
}

fn runs(rows: &Rows, j: &Journey) -> Vec<String> {
    let tt = rows.build();
    j.rides.iter().map(|r| tt.run_ids().external(r.run.raw()).to_owned()).collect()
}

/// From s00 at 940 to s02, its egress walk 120 s.
fn to_s02() -> Query {
    Query {
        access: vec![(StopId::new(0), Second(940))],
        egress: vec![(StopId::new(2), Duration(120.0))],
        max_rides: 2,
    }
}

#[test]
fn slack_zero_catches_the_connection_and_slack_sixty_takes_the_next_run() {
    let rows = rows(None);
    // Slack 0: P at 1 000 (at s00 since 940), at s01 at 1 300, Q1 at 1 330 (30 s to spare),
    // at s02 at 1 600, at the door at 1 720. Waits 60 + 30 s, rides 300 + 270 s.
    let j = ask(&rows, 0.0, &to_s02()).expect("a journey");
    assert_eq!(runs(&rows, &j), ["P", "Q1"]);
    assert_eq!(j.arrival, Second(1_720));
    assert_eq!(
        (j.wait(), j.in_vehicle(), j.egress_walk()),
        (Duration(90.0), Duration(570.0), Duration(120.0))
    );
    assert_eq!(j.footpaths, [None]);
    // Slack 60: P still (940 + 60 <= 1 000), but at s01 1 300 + 60 > 1 330: Q2 at 1 930, at
    // s02 at 2 200, at the door at 2 320. Waits 60 + 630 s.
    let j = ask(&rows, 60.0, &to_s02()).expect("a journey");
    assert_eq!(runs(&rows, &j), ["P", "Q2"]);
    assert_eq!(j.arrival, Second(2_320));
    assert_eq!(j.wait(), Duration(690.0));
    // Slack 61 applies at the first boarding too: 940 + 61 > 1 000, and P is the only run.
    assert_eq!(ask(&rows, 61.0, &to_s02()), None);
    // One ride is not enough.
    assert_eq!(ask(&rows, 0.0, &Query { max_rides: 1, ..to_s02() }), None);
}

#[test]
fn a_footpath_makes_the_journey_and_without_it_there_is_none() {
    let q = stop_to_stop(0, 940, 4, 2);
    // P to s01 at 1 300, walk 90 s to s03 (at 1 390), R at 1 400 to s04 at 1 700.
    let rows_with = rows(Some(90.0));
    let j = ask(&rows_with, 0.0, &q).expect("a journey");
    assert_eq!(runs(&rows_with, &j), ["P", "R"]);
    assert_eq!(j.arrival, Second(1_700));
    let walk = Footpath { from: StopId::new(1), to: StopId::new(3), seconds: Duration(90.0) };
    assert_eq!(j.footpaths, [Some(walk)]);
    assert_eq!((j.transfer_walk(), j.wait()), (Duration(90.0), Duration(70.0)));
    assert_eq!(ask(&rows(None), 0.0, &q), None);
    // A walk of 100.5 s reaches s03 at 1 400.5, after R leaves: no journey (rounding the walk
    // down would have caught it). One of 99.5 s catches it.
    assert_eq!(ask(&rows(Some(100.5)), 0.0, &q), None);
    assert_eq!(ask(&rows(Some(99.5)), 0.0, &q).map(|j| j.arrival), Some(Second(1_700)));
    // No footpath before the first ride: a traveller at s01 at 940 does not walk to s03, and
    // no run from s01 reaches s04. One at s03 itself takes R.
    assert_eq!(ask(&rows_with, 0.0, &stop_to_stop(1, 940, 4, 2)), None);
    let j = ask(&rows_with, 0.0, &stop_to_stop(3, 940, 4, 2)).expect("R");
    assert_eq!((runs(&rows_with, &j), j.arrival), (vec!["R".to_owned()], Second(1_700)));
}

#[test]
fn equal_arrivals_go_to_fewer_rides_then_to_the_lower_last_run() {
    // Add run A0: s01, s05, s02, at s02 at 1 600 like Q1. "A0" sorts before "Q1", so it is the
    // lower run id: the last ride is A0.
    let mut rows = rows(None);
    rows.runs.push(("A0".into(), vec![(1, 1_310, 1_310), (5, 1_450, 1_450), (2, 1_600, 1_600)]));
    let j = ask(&rows, 0.0, &to_s02()).expect("a journey");
    assert_eq!(runs(&rows, &j), ["P", "A0"]);
    assert_eq!(j.arrival, Second(1_720));
    // A direct run D from s00 at 1 000 to s02 at 1 600 arrives as early in one ride: it wins.
    rows.runs.push(("D".into(), vec![(0, 1_000, 1_000), (2, 1_600, 1_600)]));
    let j = ask(&rows, 0.0, &to_s02()).expect("a journey");
    assert_eq!(runs(&rows, &j), ["D"]);
    assert_eq!(j.arrival, Second(1_720));
}

#[test]
fn a_walk_only_answer_is_no_journey() {
    // s02 is both where the traveller is and an egress stop: no ride, no journey.
    let rows = rows(None);
    let q = Query {
        access: vec![(StopId::new(2), Second(940))],
        egress: vec![(StopId::new(2), Duration(0.0))],
        max_rides: 3,
    };
    assert_eq!(ask(&rows, 0.0, &q), None);
}

/// Each journey of `by_rides` as (its runs, its door arrival), every one checked against the
/// rows on its own.
fn by_rides(rows: &Rows, slack: f64, q: &Query) -> Vec<(Vec<String>, Second)> {
    let tt = rows.build();
    let all = Raptor::new(&tt, Duration(slack)).by_rides(q);
    for j in &all {
        check_journey(rows, &tt, slack, q, j);
    }
    all.iter().map(|j| (runs(rows, j), j.arrival)).collect()
}

#[test]
fn a_slow_direct_run_and_a_faster_connection_are_two_journeys_fewest_rides_first() {
    // Add a slow direct run D: s00 at 1 000 to s02 at 2 000. Slack 0, from s00 at 940 to s02
    // (egress walk 120 s). One ride: D, at s02 at 2 000, at the door at 2 120. Two rides: P and
    // Q1 as in the first test, at the door at 1 600 + 120 = 1 720, earlier: both, D first.
    let mut rows = rows(None);
    rows.runs.push(("D".into(), vec![(0, 1_000, 1_000), (2, 2_000, 2_000)]));
    let two = vec![
        (vec!["D".to_owned()], Second(2_120)),
        (vec!["P".to_owned(), "Q1".to_owned()], Second(1_720)),
    ];
    assert_eq!(by_rides(&rows, 0.0, &to_s02()), two);
    // The last is the earliest journey.
    let tt = rows.build();
    let earliest = Raptor::new(&tt, Duration(0.0)).earliest(&to_s02()).expect("a journey");
    assert_eq!((runs(&rows, &earliest), earliest.arrival), two[1]);
    // At most one ride: D alone.
    let one = Query { max_rides: 1, ..to_s02() };
    assert_eq!(by_rides(&rows, 0.0, &one), [(vec!["D".to_owned()], Second(2_120))]);
    // Slack 60: P then Q2 reaches the door at 2 200 + 120 = 2 320, later than D's 2 120, so two
    // rides improve on nothing: D alone.
    assert_eq!(by_rides(&rows, 60.0, &to_s02()), [(vec!["D".to_owned()], Second(2_120))]);
}

#[test]
fn a_direct_run_as_early_as_the_connection_is_the_one_journey() {
    // D from s00 at 1 000 to s02 at 1 600, as early as P then Q1 (door at 1 720): two rides do
    // not arrive earlier than one, so one journey, D. And D at 1 500 (door at 1 620), earlier
    // still: one journey, D.
    for (at_s02, door) in [(1_600, 1_720), (1_500, 1_620)] {
        let mut rows = rows(None);
        rows.runs.push(("D".into(), vec![(0, 1_000, 1_000), (2, at_s02, at_s02)]));
        assert_eq!(by_rides(&rows, 0.0, &to_s02()), [(vec!["D".to_owned()], Second(door))]);
    }
    // No journey at all: nothing (slack 61 misses P, the only run from s00 without D).
    assert_eq!(by_rides(&rows(None), 61.0, &to_s02()), []);
}
