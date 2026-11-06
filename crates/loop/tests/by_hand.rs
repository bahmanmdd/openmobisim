//! The loop by hand: bikes, walkers and transit through the five stages, the gaps, an empty
//! route, an unfinished trip, the refusals and the stopping rule.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "counts of trips and seconds of the window, far below 2^53 and above zero"
)]

mod common;

use common::{VALUES, at, car_link, demand, network, time_only};
use openmobisim_demand::demand::Demand;
use openmobisim_demand::input::TripRow;
use openmobisim_graph::network::{LinkRow, Network, NetworkBuilder};
use openmobisim_loop::assess::{Iteration, stops};
use openmobisim_loop::parameters::Parameters;
use openmobisim_loop::run::{RunError, run};
use openmobisim_transit::timetable::{Call, Timetable, TimetableBuilder};
use openmobisim_types::ids::{EntityId, TripId};
use openmobisim_types::mode::{Mode, ModeSet};
use openmobisim_types::time::{Second, StepGridError};
use openmobisim_types::units::Metres;

/// A link for bikes and walkers only, `length_m` long.
fn path(length_m: f64) -> LinkRow {
    LinkRow {
        modes: ModeSet::of(&[Mode::Bike, Mode::Walk]),
        lanes: None,
        car_speed_km_h: None,
        length_m: Some(length_m),
        capacity_veh_h: None,
        signal_at_downstream_end: false,
    }
}

/// Nodes a and b 1 500 m apart with a path a→b and a street a→b (2 000 m at 50 km/h).
fn two_nodes() -> Network {
    network(
        &[("a", 0.0, 0.0), ("b", 1_500.0, 0.0)],
        &[("ab", "a", "b", path(1_500.0)), ("st", "a", "b", car_link(2_000.0, 50.0, 1_800.0))],
    )
}

/// One trip from `(0, dy)` to `(1 500, dy)` at second 0 with `modes`.
fn trip(dy: f64, modes: &[Mode]) -> TripRow {
    TripRow {
        id: "t".to_string(),
        origin: at(0.0, dy),
        destination: at(1_500.0, dy),
        departure: Second(0),
        class: "default".to_string(),
        modes: ModeSet::of(modes),
    }
}

const WINDOW: (Second, Second) = (Second(0), Second(3_600));

#[test]
fn a_bike_rides_1500_m_in_360_s_and_a_walker_walks_it_in_1125_s() {
    let net = two_nodes();
    for (mode, expected) in [(Mode::Bike, 360.0), (Mode::Walk, 1_125.0)] {
        let dem = demand(&net, &[trip(0.0, &[mode])]);
        let r = run(&net, &dem, None, WINDOW, &time_only(3)).expect("a run");
        assert_eq!(r.trips.mode[0], Some(mode));
        assert!(
            (r.trips.travel_time[0] - expected).abs() < 1e-9,
            "{mode}: {}",
            r.trips.travel_time[0]
        );
        assert!(r.trips.finished[0]);
        assert_eq!(r.iterations.len(), 3, "the cap, since the rule needs five iterations");
        assert!(r.iterations[2].found_share.abs() < 1e-12, "nothing new after iteration 0");
    }
}

#[test]
fn gaps_of_30_m_are_walked_in_22_5_s_each_and_the_ride_starts_on_the_whole_second() {
    // Door to node 22.5 s; the bike departs at 23 s, rides 360 s, walks 22.5 s: 405.5 s.
    let net = two_nodes();
    let dem = demand(&net, &[trip(30.0, &[Mode::Bike])]);
    let r = run(&net, &dem, None, WINDOW, &time_only(1)).expect("a run");
    assert!((r.trips.travel_time[0] - 405.5).abs() < 1e-9, "{}", r.trips.travel_time[0]);
    assert!((r.trips.arrival[0] - 405.5).abs() < 1e-9);
}

#[test]
fn mode_choice_takes_the_cheaper_in_time_the_car_at_144_s_over_the_bike_at_360_s() {
    let net = two_nodes();
    let dem = demand(&net, &[trip(0.0, &[Mode::Car, Mode::Bike, Mode::Walk])]);
    let r = run(&net, &dem, None, WINDOW, &time_only(2)).expect("a run");
    let set = r.set(TripId::new(0));
    assert_eq!(set.iter().map(|a| a.mode).collect::<Vec<_>>(), [Mode::Car, Mode::Bike, Mode::Walk]);
    assert_eq!(r.trips.mode[0], Some(Mode::Car));
    assert!((r.trips.travel_time[0] - 144.0).abs() < 1e-9);
    assert_eq!(r.trips.alternatives[0], 3);
    assert!(r.iterations[1].gap.abs() < 1e-12, "the best is taken");
}

#[test]
fn an_origin_at_its_destination_node_walks_its_gaps_only_and_loads_nothing() {
    let net = two_nodes();
    let row = TripRow { destination: at(0.0, -30.0), ..trip(30.0, &[Mode::Car]) };
    let dem = demand(&net, &[row]);
    let r = run(&net, &dem, None, WINDOW, &time_only(1)).expect("a run");
    assert!((r.trips.travel_time[0] - 45.0).abs() < 1e-9, "{}", r.trips.travel_time[0]);
    assert!(r.links.is_empty(), "nothing loaded");
}

#[test]
fn a_walker_still_walking_at_the_window_end_is_unfinished() {
    let net = two_nodes();
    let dem = demand(&net, &[trip(0.0, &[Mode::Walk])]);
    let r = run(&net, &dem, None, (Second(0), Second(600)), &time_only(1)).expect("a run");
    assert!(!r.trips.finished[0]);
    assert!(r.trips.arrival[0].is_nan() && r.trips.travel_time[0].is_nan(), "unknown");
    assert_eq!(r.iterations[0].unfinished, 1);
    assert_eq!(r.iterations[0].finished, 0);
    assert!(r.iterations[0].mean_trip_time.is_nan());
}

/// Stops A at node a and B at node b, one run A 600 → B 900, and the walk network of
/// [`two_nodes`].
fn timetable() -> Timetable {
    let mut b = TimetableBuilder::new();
    b.add_stop("A", (Metres(0.0), Metres(0.0)));
    b.add_stop("B", (Metres(1_500.0), Metres(0.0)));
    b.add_run(
        "r1",
        vec![
            Call { stop: "A".to_string(), arrival: Second(600), departure: Second(600) },
            Call { stop: "B".to_string(), arrival: Second(900), departure: Second(900) },
        ],
    );
    b.build().expect("a timetable")
}

#[test]
fn transit_by_hand_walk_22_5_s_wait_ride_to_900_walk_22_5_s_arrives_at_922_5_s() {
    let net = two_nodes();
    let tt = timetable();
    let rows = [trip(30.0, &[Mode::Walk, Mode::Transit])];
    let dem = Demand::build(&rows, &["default".to_string()], &net, Some(&tt), Metres(1_500.0))
        .expect("a demand");
    let r = run(&net, &dem, Some(&tt), WINDOW, &time_only(2)).expect("a run");
    let set = r.set(TripId::new(0));
    assert_eq!(set.iter().map(|a| a.mode).collect::<Vec<_>>(), [Mode::Walk, Mode::Transit]);
    // The walk: 22.5 s, 1 125 s from second 23, 22.5 s: 1 170.5 s. Transit: 922.5 s.
    assert!((set[0].time - 1_170.5).abs() < 1e-9, "walk {}", set[0].time);
    assert!((set[1].time - 922.5).abs() < 1e-9, "transit {}", set[1].time);
    assert_eq!(r.trips.mode[0], Some(Mode::Transit));
    assert!((r.trips.travel_time[0] - 922.5).abs() < 1e-9);
    assert!(r.trips.finished[0]);
    assert!(r.links.is_empty(), "a journey loads nothing");
}

#[test]
fn with_a_boarding_slack_of_600_s_the_run_at_600_s_cannot_be_boarded_from_a_stop_reached_at_23_s() {
    let net = two_nodes();
    let tt = timetable();
    let rows = [trip(30.0, &[Mode::Walk, Mode::Transit])];
    let dem = Demand::build(&rows, &["default".to_string()], &net, Some(&tt), Metres(1_500.0))
        .expect("a demand");
    let p = Parameters { boarding_slack: 600, ..time_only(1) };
    let r = run(&net, &dem, Some(&tt), WINDOW, &p).expect("a run");
    assert_eq!(r.set(TripId::new(0)).len(), 1, "the walk alone");
    assert_eq!(r.trips.mode[0], Some(Mode::Walk));
    let p = Parameters { boarding_slack: 577, ..time_only(1) };
    let r = run(&net, &dem, Some(&tt), WINDOW, &p).expect("a run");
    assert_eq!(r.trips.mode[0], Some(Mode::Transit), "at the stop at 23 s, 577 s before 600 s");
}

#[test]
fn max_rides_1_refuses_a_two_ride_journey_that_max_rides_2_takes() {
    // Stops A (node a), B (node b) and C 2 000 m past b on a second path, beyond the 1 500 m
    // walk to a stop; runs A→B at 600 and B→C at 1 000, so A→C needs two rides.
    let net = network(
        &[("a", 0.0, 0.0), ("b", 1_500.0, 0.0), ("c", 3_500.0, 0.0)],
        &[("ab", "a", "b", path(1_500.0)), ("bc", "b", "c", path(2_000.0))],
    );
    let mut t = TimetableBuilder::new();
    t.add_stop("A", (Metres(0.0), Metres(0.0)));
    t.add_stop("B", (Metres(1_500.0), Metres(0.0)));
    t.add_stop("C", (Metres(3_500.0), Metres(0.0)));
    let call = |stop: &str, s: u32| Call {
        stop: stop.to_string(),
        arrival: Second(s),
        departure: Second(s),
    };
    t.add_run("r1", vec![call("A", 600), call("B", 900)]);
    t.add_run("r2", vec![call("B", 1_000), call("C", 1_300)]);
    let tt = t.build().expect("a timetable");
    let rows = [TripRow { destination: at(3_500.0, 0.0), ..trip(0.0, &[Mode::Transit]) }];
    let dem = Demand::build(&rows, &["default".to_string()], &net, Some(&tt), Metres(1_500.0))
        .expect("a demand");
    let one = run(&net, &dem, Some(&tt), WINDOW, &Parameters { max_rides: 1, ..time_only(1) })
        .expect("a run");
    assert_eq!(one.trips.mode[0], None, "no journey with one ride");
    assert_eq!(one.iterations[0].without_alternative, 1);
    let two = run(&net, &dem, Some(&tt), WINDOW, &Parameters { max_rides: 2, ..time_only(1) })
        .expect("a run");
    assert_eq!(two.trips.mode[0], Some(Mode::Transit));
    assert!((two.trips.travel_time[0] - 1_300.0).abs() < 1e-9, "{}", two.trips.travel_time[0]);
}

#[test]
fn refusals_name_what_they_refuse() {
    let net = two_nodes();
    let dem = demand(&net, &[trip(0.0, &[Mode::Car])]);
    let p = time_only(1);
    let bad = Parameters { link_bin: 0, ..p.clone() };
    assert_eq!(run(&net, &dem, None, WINDOW, &bad).err(), Some(RunError::LinkBinZero));
    let bad = Parameters { iteration_cap: 0, ..p.clone() };
    assert_eq!(run(&net, &dem, None, WINDOW, &bad).err(), Some(RunError::IterationCapZero));
    let bad = Parameters { max_rides: 0, ..p.clone() };
    assert_eq!(run(&net, &dem, None, WINDOW, &bad).err(), Some(RunError::MaxRidesZero));
    let bad = Parameters { coefficients: vec![], ..p.clone() };
    assert_eq!(
        run(&net, &dem, None, WINDOW, &bad).err(),
        Some(RunError::CoefficientRows { classes: 1, rows: 0 })
    );
    assert_eq!(
        run(&net, &dem, None, (Second(0), Second(100)), &p).err(),
        Some(RunError::Window(StepGridError::WindowNotWholeSteps {
            window_seconds: 100,
            step_seconds: 60
        }))
    );
    assert_eq!(
        run(&net, &dem, None, (Second(60), Second(3_660)), &p).err(),
        Some(RunError::DepartureOutsideWindow("t".to_string()))
    );
    // A gap of 100 m walked in 75 s would reach the network at the end of a 60 s window.
    let far = demand(&net, &[trip(100.0, &[Mode::Car])]);
    assert_eq!(
        run(&net, &far, None, (Second(0), Second(60)), &p).err(),
        Some(RunError::StartsAfterWindow("t".to_string(), Mode::Car))
    );
}

fn row(mean: f64, found: f64) -> Iteration {
    Iteration {
        iteration: 0,
        mean_trip_time: mean,
        finished: 1,
        unfinished: 0,
        without_alternative: 0,
        switch_share: 0.0,
        found_share: found,
        gap: 0.0,
        stage_seconds: [0.0; 5],
    }
}

#[test]
fn the_stopping_rule_by_hand() {
    let flat: Vec<Iteration> = (0..5).map(|_| row(1_000.0, 0.005)).collect();
    assert!(stops(&flat), "five flat means and found at the limit");
    assert!(!stops(&flat[..4]), "four are too few");
    let mut off = flat.clone();
    off[1] = row(1_003.1, 0.0);
    assert!(!stops(&off), "a mean 0.31 % off");
    off[1] = row(1_003.0, 0.0);
    assert!(stops(&off), "a mean 0.30 % off");
    let mut found = flat.clone();
    found[4] = row(1_000.0, 0.0051);
    assert!(!stops(&found), "found above 0.5 %");
    let mut earlier = flat.clone();
    earlier[2] = row(1_000.0, 0.5);
    assert!(stops(&earlier), "only the last iteration's found share is read");
    let mut none = flat;
    none[4] = row(f64::NAN, 0.0);
    assert!(!stops(&none), "no finished trip, no mean");
}

#[test]
fn the_build_values_are_the rulingss() {
    let _ = NetworkBuilder::new();
    assert!((VALUES.walk_speed - 4.8).abs() < 1e-12 && (VALUES.bike_speed - 15.0).abs() < 1e-12);
}
