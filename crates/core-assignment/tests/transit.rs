//! Transit legs: the walking limits and the path size by shared rides, from values derived by
//! hand. (The journeys themselves, the split and the run's properties are defended by the
//! Python tests of the one-loop engine with a timetable, `test_one_loop_transit.py`.)

use openmobisim_core_assignment::transit::ride_path_sizes;
use openmobisim_core_assignment::{
    Attributes, Chain, FixedTimes, Leg, Place, Ride, walking_within_limits,
};
use openmobisim_core_demand::Mode;
use openmobisim_core_types::ids::{EntityId, NodeId, TransitRunId};

const MIN: f64 = 60.0;

/// The user's limits (2026-11-05): 30 minutes to the first stop, 30 from the last, one hour of
/// walking in all.
#[test]
fn a_journey_walking_more_than_an_hour_in_all_is_refused() {
    let (access_max, total_max) = (30.0 * MIN, 60.0 * MIN);
    // 25 + 10 + 28 = 63 minutes: refused, though each end is within its 30 minutes.
    assert!(!walking_within_limits(25.0 * MIN, &[10.0 * MIN], 28.0 * MIN, access_max, total_max));
    // 25 + 5 + 28 = 58 minutes: kept.
    assert!(walking_within_limits(25.0 * MIN, &[5.0 * MIN], 28.0 * MIN, access_max, total_max));
    // Exactly at the limits: kept.
    assert!(walking_within_limits(30.0 * MIN, &[], 30.0 * MIN, access_max, total_max));
    // An end beyond its 30 minutes: refused, whatever the total.
    assert!(!walking_within_limits(31.0 * MIN, &[], 1.0 * MIN, access_max, total_max));
    assert!(!walking_within_limits(1.0 * MIN, &[], 31.0 * MIN, access_max, total_max));
}

/// A ride of `line` from stop `a` to stop `b`, leaving at `at` and taking `seconds`.
fn ride(line: u32, a: u32, b: u32, at: u32, seconds: u32) -> Ride {
    Ride {
        line,
        run: TransitRunId::new(line),
        board_stop: NodeId::new(a),
        alight_stop: NodeId::new(b),
        board_call: 0,
        alight_call: 0,
        departure: at,
        arrival: at + seconds,
    }
}

/// A transit chain of `rides` taking `total` seconds door to door (the rest walking).
fn chain(rides: Vec<Ride>, total: f64) -> Chain {
    let riding: f64 = rides.iter().map(|r| f64::from(r.arrival - r.departure)).sum();
    Chain::new(vec![Leg {
        mode: Mode::Transit,
        from: Place::Walk(NodeId::new(0)),
        to: Place::Walk(NodeId::new(1)),
        departure: 0.0,
        attributes: Attributes {
            in_vehicle_seconds: riding,
            walk_seconds: total - riding,
            ..Attributes::default()
        },
        fixed: FixedTimes::NONE,
        links: Vec::new(),
        rides,
    }])
}

/// Two journeys share their first ride (line 1, stop 0 → stop 1, 600 s): the first then rides
/// line 2 for 600 s and takes 1 500 s door to door, the second line 3 for 900 s and 1 800 s.
/// Path size by time share: (600 / 2 + 600 + 300) / 1 500 = 0.8 and (600 / 2 + 900 + 300) /
/// 1 800 = 5 / 6. A journey sharing nothing has 1 (ln 0).
#[test]
fn the_path_size_counts_the_shared_rides_by_their_share_of_the_time() {
    let a = chain(vec![ride(1, 0, 1, 100, 600), ride(2, 1, 2, 800, 600)], 1500.0);
    let b = chain(vec![ride(1, 0, 1, 100, 600), ride(3, 1, 2, 900, 900)], 1800.0);
    let c = chain(vec![ride(4, 0, 2, 100, 1200)], 1300.0);
    let ln = ride_path_sizes(&[&a, &b, &c]);
    assert!((ln[0] - 0.8_f64.ln()).abs() < 1e-12, "{ln:?}");
    assert!((ln[1] - (5.0_f64 / 6.0).ln()).abs() < 1e-12, "{ln:?}");
    assert!(ln[2].abs() < 1e-12, "{ln:?}");
    // The same line between the same stops on another run is the same ride.
    let later = chain(vec![ride(1, 0, 1, 700, 600), ride(2, 1, 2, 1400, 600)], 1500.0);
    let ln = ride_path_sizes(&[&later, &b]);
    assert!((ln[0] - 0.8_f64.ln()).abs() < 1e-12, "{ln:?}");
}

/// Two chains riding the same lines between the same stops are the same alternative, whatever
/// the runs and times; another line is another alternative.
#[test]
fn a_transit_chain_is_known_by_its_lines_and_stops() {
    let a = chain(vec![ride(1, 0, 1, 100, 600)], 900.0);
    let later = chain(vec![ride(1, 0, 1, 700, 600)], 900.0);
    let other = chain(vec![ride(2, 0, 1, 100, 600)], 900.0);
    assert!(a.same_alternative(&later));
    assert_eq!(a.identity(), later.identity());
    assert!(!a.same_alternative(&other));
    assert!(a.moves());
}
