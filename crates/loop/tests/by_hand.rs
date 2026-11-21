//! The loop by hand: bikes, walkers and transit through the five stages, the gaps, an empty
//! route, an unfinished trip, the refusals and the stopping rule.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "counts of trips and seconds of the window, far below 2^53 and above zero"
)]

mod common;

use common::{at, car_link, demand, network, time_only};
use openmobisim_demand::demand::Demand;
use openmobisim_demand::input::TripRow;
use openmobisim_graph::network::{LinkRow, Network};
use openmobisim_loop::assess::{Iteration, stops};
use openmobisim_loop::parameters::{ChoiceModel, Parameters};
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
fn gaps_of_30_m_are_walked_in_22_5_s_each_and_the_ride_starts_when_the_walk_ends() {
    // Door to node 22.5 s; the bike departs at 22.5 s, rides 360 s, walks 22.5 s: 405 s.
    let net = two_nodes();
    let dem = demand(&net, &[trip(30.0, &[Mode::Bike])]);
    let r = run(&net, &dem, None, WINDOW, &time_only(1)).expect("a run");
    assert!((r.trips.travel_time[0] - 405.0).abs() < 1e-9, "{}", r.trips.travel_time[0]);
    assert!((r.trips.arrival[0] - 405.0).abs() < 1e-9);
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
    // The walk: 22.5 s, 1 125 s from 22.5 s on, 22.5 s: 1 170 s. Transit: 922.5 s.
    assert!((set[0].time - 1_170.0).abs() < 1e-9, "walk {}", set[0].time);
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
fn a_trip_1501_m_from_its_stop_is_offered_no_journey_and_is_counted_without_a_stop() {
    // By hand: both ends one gap off their nodes, stops A and B on the nodes, one run A 2 000 →
    // B 2 300. At a gap of 1 500 m the walk to A is 1 500 m, in (1 125 s, before the run); at
    // 1 501 m it is out, and so is B's. The trip walks instead (transit and walk are open to
    // it) and is counted in both columns.
    let net = two_nodes();
    let mut b = TimetableBuilder::new();
    b.add_stop("A", (Metres(0.0), Metres(0.0)));
    b.add_stop("B", (Metres(1_500.0), Metres(0.0)));
    let call = |stop: &str, s: u32| Call {
        stop: stop.to_string(),
        arrival: Second(s),
        departure: Second(s),
    };
    b.add_run("r1", vec![call("A", 2_000), call("B", 2_300)]);
    let tt = b.build().expect("a timetable");
    let counts = |gap: f64| {
        let rows = [trip(gap, &[Mode::Walk, Mode::Transit])];
        let dem = Demand::build(&rows, &["default".to_string()], &net, Some(&tt), Metres(1_500.0))
            .expect("a demand");
        let window = (Second(0), Second(7_200));
        let r = run(&net, &dem, Some(&tt), window, &time_only(1)).expect("a run");
        let it = &r.iterations[0];
        (r.set(TripId::new(0)).len(), it.without_transit, it.without_stop, it.without_alternative)
    };
    assert_eq!(counts(1_500.0), (2, 0, 0, 0), "1 500 m: the walk and the journey");
    assert_eq!(counts(1_501.0), (1, 1, 1, 0), "1 501 m: the walk alone, counted twice");
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
    let offer = |r: &openmobisim_loop::run::Run| {
        (r.iterations[0].without_transit, r.iterations[0].without_stop)
    };
    assert_eq!(offer(&one), (1, 0), "stops at both ends, no journey with one ride");
    let two = run(&net, &dem, Some(&tt), WINDOW, &Parameters { max_rides: 2, ..time_only(1) })
        .expect("a run");
    assert_eq!(two.trips.mode[0], Some(Mode::Transit));
    assert_eq!(offer(&two), (0, 0));
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
    let bad = Parameters { nearby_stops: 0, ..p.clone() };
    assert_eq!(run(&net, &dem, None, WINDOW, &bad).err(), Some(RunError::NearbyStopsZero));
    let bad = Parameters { coefficients: vec![], ..p.clone() };
    assert_eq!(
        run(&net, &dem, None, WINDOW, &bad).err(),
        Some(RunError::CoefficientRows { classes: 1, rows: 0 })
    );
    assert_eq!(
        run(&net, &dem, None, (Second(0), Second(0)), &p).err(),
        Some(RunError::Window(StepGridError::ZeroWindow))
    );
    // The logit's step exponent: refused when it is not a positive finite number, `1 / (k + 1)^a`
    // then being no step of the averaging; every other value runs; not read under the least cost.
    let logit = Parameters { choice: ChoiceModel::Logit, ..p.clone() };
    for a in [0.0, -0.5, f64::NAN, f64::INFINITY] {
        let bad = Parameters { step_exponent: a, ..logit.clone() };
        let refused = run(&net, &dem, None, WINDOW, &bad).err();
        assert!(
            matches!(refused, Some(RunError::StepExponent(x)) if x.to_bits() == a.to_bits()),
            "step exponent {a}: {refused:?}"
        );
    }
    for a in [0.25, 2.0] {
        let runs = Parameters { step_exponent: a, ..logit.clone() };
        assert!(run(&net, &dem, None, WINDOW, &runs).is_ok(), "step exponent {a} runs");
    }
    let least_cost = Parameters { choice: ChoiceModel::Deterministic, step_exponent: 0.0, ..p };
    assert!(run(&net, &dem, None, WINDOW, &least_cost).is_ok(), "not read under the least cost");
}

/// The window's repairs, each by hand. A window of 100 s in 60 s bins ends at 120 s, the next
/// whole bin. A trip departing at 0 s is outside a window from 60 s: dropped. A gap of 100 m
/// is walked in 75 s at 4.8 km/h: by car or walk the trip would reach the network after a
/// 60 s window, so both modes are taken out and the trip, left with none, is dropped; with
/// transit beside the car (no timetable) it keeps transit and is run without an alternative.
#[test]
fn what_the_window_cannot_hold_is_set_right_and_noted() {
    let net = two_nodes();
    let p = time_only(1);
    let keys = |r: &openmobisim_loop::run::Run| -> Vec<(&str, u64, Vec<String>)> {
        r.repairs.kinds().iter().map(|k| (k.key, k.count, k.first.clone())).collect()
    };
    let t = || vec!["t".to_owned()];
    let bike = demand(&net, &[trip(0.0, &[Mode::Bike])]);
    let r = run(&net, &bike, None, (Second(0), Second(100)), &p).expect("a run");
    assert_eq!(r.window, (Second(0), Second(120)));
    assert_eq!(keys(&r), [("window_end_moved", 1, vec!["120".to_owned()])]);
    assert!(r.trips.kept[0]);
    let car = demand(&net, &[trip(0.0, &[Mode::Car])]);
    let r = run(&net, &car, None, (Second(60), Second(3_660)), &p).expect("a run");
    assert_eq!(keys(&r), [("trips_outside_window", 1, t())]);
    assert_eq!((r.trips.kept[0], r.trips.mode[0]), (false, None));
    assert_eq!((r.iterations[0].without_alternative, r.iterations[0].finished), (0, 0));
    let far = demand(&net, &[trip(100.0, &[Mode::Car, Mode::Walk])]);
    let r = run(&net, &far, None, (Second(0), Second(60)), &p).expect("a run");
    assert_eq!(keys(&r), [("modes_after_window", 1, t()), ("trips_after_window", 1, t())]);
    assert!(!r.trips.kept[0]);
    let far = demand(&net, &[trip(100.0, &[Mode::Car, Mode::Transit])]);
    let r = run(&net, &far, None, (Second(0), Second(60)), &p).expect("a run");
    assert_eq!(keys(&r), [("modes_after_window", 1, t())]);
    assert!(r.trips.kept[0]);
    assert_eq!(r.iterations[0].without_alternative, 1);
}

fn row(mean: f64, found: f64) -> Iteration {
    Iteration {
        iteration: 0,
        mean_trip_time: mean,
        finished: 1,
        unfinished: 0,
        without_alternative: 0,
        without_transit: 0,
        without_stop: 0,
        offered_share: 1.0,
        switch_share: 0.0,
        found_share: found,
        would_change_share: f64::NAN,
        gap: 0.0,
        stage_seconds: [0.0; 5],
    }
}

/// A lone bike trip finds nothing after iteration 0 and its mean never moves, so the rule stops
/// the run as soon as it is read with five iterations: the floor decides when that is.
#[test]
fn the_rule_is_not_read_before_the_floor() {
    let net = two_nodes();
    let dem = demand(&net, &[trip(0.0, &[Mode::Bike])]);
    for (floor, expected, by_rule) in [(0, 5, true), (12, 12, true), (30, 20, false)] {
        let p = Parameters { iteration_floor: floor, ..time_only(20) };
        let r = run(&net, &dem, None, WINDOW, &p).expect("a run");
        assert_eq!(r.iterations.len(), expected, "floor {floor}");
        assert_eq!(r.stopped_by_rule, by_rule, "floor {floor}");
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
