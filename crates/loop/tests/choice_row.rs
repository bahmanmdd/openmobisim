//! The choice row of each mode by hand: the sixteen columns of `COLUMNS`, times in minutes, an
//! alternative filling only its mode's columns; and the path size, computed for car routes only.
//!
//! Car, bike and walk: nodes a (0, 0) and b (1 500, 0); a path a→b of 1 500 m for bikes and
//! walkers, a street a→b of 2 000 m at 50 km/h (144 s). The trip goes from (0, 30) to
//! (1 500, 30) at second 0: a gap of 30 m at each end, walked at 4.8 km/h in 22.5 s. Every
//! network leg is entered on the whole second its walk ends, 23 s, so each door-to-door time is
//! 22.5 + 0.5 + the network time + 22.5 s, and the half second of waiting for the whole second
//! goes with the network time (door to door less the two gap walks):
//!
//! * car: 189.5 s; `car_drive_min` (189.5 − 45) / 60 = 144.5 / 60, `walk_min` 45 / 60 = 0.75,
//!   `car_ln_path_size` 0 (one car route), `is_car` 1;
//! * bike: 405.5 s; `bike_ride_min` 360.5 / 60, `walk_min` 0.75, `is_bike` 1;
//! * walk: 1 170.5 s; `walk_min` 1 170.5 / 60, `is_walk` 1.
//!
//! Transit with one transfer: nodes a (0, 0), b (1 500, 0), c (3 500, 0), walk paths a→b and
//! b→c; stops A at a, B1 and B2 at b, C at c; run r1 A at 600 s to B1 at 900 s, run r2 B2 at
//! 1 200 s to C at 1 500 s, a footpath B1→B2 of 90 s; the default boarding slack 60 s. The trip
//! from (0, 30) to (3 500, 30) at second 0 reaches A after 22.5 s (the only stop within 1 500 m:
//! B1 and B2 are 1 530 m away), boards r1 at 600 (23 + 60 ≤ 600), is at B1 at 900, walks 90 s to
//! B2 (990), boards r2 at 1 200 (990 + 60 ≤ 1 200), is at C at 1 500, walks 22.5 s: at the door
//! at 1 522.5 s. Its parts: access 22.5 s = 0.375 min; first wait 600 − 22.5 = 577.5 s =
//! 9.625 min (the slack included); in the vehicle 300 + 300 = 600 s = 10 min; transfer wait
//! 1 200 − 990 = 210 s = 3.5 min; transfer walk 90 s = 1.5 min; egress 22.5 s = 0.375 min; one
//! transfer; `is_transit` 1. The six times add up to 1 522.5 s. At the default coefficients its
//! cost is 0.1 · 10 + 0.165 · 0.375 + 0.165 · 0.375 + 0.145 · 9.625 + 0.145 · 3.5 + 0.165 · 1.5
//! + 0.5 · 1 + 0.25 = 4.024375.
//!
//! Path size (Ben-Akiva and Bierlaire 1999): two routes a→m→b sharing a→m (1 000 m) and each
//! with its own m→b (500 m): `PS = (1 000 / 1 500) / 2 + (500 / 1 500) / 1 = 2/3` for each, so
//! two car routes have `ln PS = ln(2/3)`; the same two routes by bike or on foot have 0.
//!
//! For bikes: the bike's row holds its riding minutes and its gap walks apart, and its path
//! size is 0 whatever its routes overlap.

mod common;

use common::{VALUES, at, car_link, demand, network, time_only};
use openmobisim_demand::demand::Demand;
use openmobisim_demand::input::TripRow;
use openmobisim_graph::network::LinkRow;
use openmobisim_loop::alternative::{Alternative, Leg, path_sizes};
use openmobisim_loop::parameters::{COLUMNS, Parameters};
use openmobisim_loop::run::run;
use openmobisim_transit::timetable::{Call, TimetableBuilder};
use openmobisim_types::ids::{EntityId, TripId};
use openmobisim_types::mode::{Mode, ModeSet};
use openmobisim_types::time::Second;
use openmobisim_types::units::{Duration, Metres};

const WINDOW: (Second, Second) = (Second(0), Second(3_600));

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

/// A row from (column name, value) pairs, every other column 0.
fn row(values: &[(&str, f64)]) -> [f64; 16] {
    let mut x = [0.0; 16];
    for &(name, v) in values {
        x[COLUMNS.iter().position(|&c| c == name).expect("a column")] = v;
    }
    x
}

/// `got` equals `want` column by column, to 1e-12.
fn same_row(label: &str, got: &[f64; 16], want: &[f64; 16]) {
    for (k, (g, w)) in got.iter().zip(want).enumerate() {
        assert!((g - w).abs() < 1e-12, "{label}, {}: {g} against {w}", COLUMNS[k]);
    }
}

#[test]
fn the_car_bike_and_walk_rows_hold_their_minutes_and_their_gap_walks() {
    assert!((VALUES.walk_speed - 4.8).abs() < 1e-12, "30 m in 22.5 s");
    let net = network(
        &[("a", 0.0, 0.0), ("b", 1_500.0, 0.0)],
        &[("ab", "a", "b", path(1_500.0)), ("st", "a", "b", car_link(2_000.0, 50.0, 1_800.0))],
    );
    let trip = TripRow {
        id: "t".to_string(),
        origin: at(0.0, 30.0),
        destination: at(1_500.0, 30.0),
        departure: Second(0),
        class: "default".to_string(),
        modes: ModeSet::of(&[Mode::Car, Mode::Bike, Mode::Walk]),
    };
    let r = run(&net, &demand(&net, &[trip]), None, WINDOW, &time_only(1)).expect("a run");
    let set = r.set(TripId::new(0));
    assert_eq!(set.iter().map(|a| a.mode).collect::<Vec<_>>(), [Mode::Car, Mode::Bike, Mode::Walk]);
    let want = [
        (189.0, row(&[("car_drive_min", 144.0 / 60.0), ("walk_min", 0.75), ("is_car", 1.0)])),
        (405.0, row(&[("bike_ride_min", 360.0 / 60.0), ("walk_min", 0.75), ("is_bike", 1.0)])),
        (1_170.0, row(&[("walk_min", 1_170.0 / 60.0), ("is_walk", 1.0)])),
    ];
    for (a, (time, x)) in set.iter().zip(&want) {
        assert!((a.time - time).abs() < 1e-9, "{}: {} s against {time} s", a.mode, a.time);
        same_row(&a.mode.to_string(), &a.attributes(), x);
    }
}

#[test]
fn a_journey_with_one_transfer_has_each_part_in_minutes() {
    let net = network(
        &[("a", 0.0, 0.0), ("b", 1_500.0, 0.0), ("c", 3_500.0, 0.0)],
        &[("ab", "a", "b", path(1_500.0)), ("bc", "b", "c", path(2_000.0))],
    );
    let mut t = TimetableBuilder::new();
    for (stop, x) in [("A", 0.0), ("B1", 1_500.0), ("B2", 1_500.0), ("C", 3_500.0)] {
        t.add_stop(stop, (Metres(x), Metres(0.0)));
    }
    let call = |stop: &str, s: u32| Call {
        stop: stop.to_string(),
        arrival: Second(s),
        departure: Second(s),
    };
    t.add_run("r1", vec![call("A", 600), call("B1", 900)]);
    t.add_run("r2", vec![call("B2", 1_200), call("C", 1_500)]);
    t.add_footpath("B1", "B2", Duration(90.0));
    let tt = t.build().expect("a timetable");
    let trip = TripRow {
        id: "t".to_string(),
        origin: at(0.0, 30.0),
        destination: at(3_500.0, 30.0),
        departure: Second(0),
        class: "default".to_string(),
        modes: ModeSet::of(&[Mode::Transit]),
    };
    let dem = Demand::build(&[trip], &["default".to_string()], &net, Some(&tt), Metres(1_500.0))
        .expect("a demand");
    let r = run(&net, &dem, Some(&tt), WINDOW, &time_only(1)).expect("a run");
    let set = r.set(TripId::new(0));
    assert_eq!(set.len(), 1, "no journey of one ride, one of two");
    let journey = &set[0];
    assert!((journey.time - 1_522.5).abs() < 1e-9, "{} s", journey.time);
    let want = row(&[
        ("transit_in_vehicle_min", 10.0),
        ("transit_access_min", 0.375),
        ("transit_egress_min", 0.375),
        ("transit_first_wait_min", 9.625),
        ("transit_transfer_wait_min", 3.5),
        ("transit_transfer_walk_min", 1.5),
        ("transit_transfers", 1.0),
        ("is_transit", 1.0),
    ]);
    let x = journey.attributes();
    same_row("transit", &x, &want);
    // The nine times add up to the door to door, so the time-only cost is it in minutes.
    let time_only_cost = time_only(1).cost(0, &x);
    assert!((time_only_cost - 1_522.5 / 60.0).abs() < 1e-12, "{time_only_cost}");
    let cost = Parameters::defaults(1).cost(0, &x);
    assert!((cost - 4.024_375).abs() < 1e-12, "the default cost {cost}");
    assert!((r.trips.travel_time[0] - 1_522.5).abs() < 1e-9);
}

#[test]
fn path_size_is_computed_for_car_routes_and_is_0_for_bike_and_walk_routes() {
    let all = LinkRow {
        modes: ModeSet::of(&[Mode::Car, Mode::Bike, Mode::Walk]),
        lanes: Some(1),
        car_speed_km_h: Some(50.0),
        length_m: None,
        capacity_veh_h: Some(1_800.0),
        signal_at_downstream_end: false,
    };
    let net = network(
        &[("a", 0.0, 0.0), ("m", 1_000.0, 0.0), ("b", 1_500.0, 0.0)],
        &[
            ("am", "a", "m", LinkRow { length_m: Some(1_000.0), ..all }),
            ("mb1", "m", "b", LinkRow { length_m: Some(500.0), ..all }),
            ("mb2", "m", "b", LinkRow { length_m: Some(500.0), ..all }),
        ],
    );
    let id = |s: &str| common::link(&net, s);
    let mut set = Vec::new();
    for mode in [Mode::Car, Mode::Bike, Mode::Walk] {
        for last in ["mb1", "mb2"] {
            let legs = vec![
                Leg::Walk(Duration(0.0)),
                Leg::Route { mode, links: vec![id("am"), id(last)] },
                Leg::Walk(Duration(0.0)),
            ];
            // A stale value, to see it rewritten.
            set.push(Alternative { mode, legs, time: 100.0, ln_path_size: 0.5 });
        }
    }
    path_sizes(&net, &mut set);
    let ln_two_thirds = (2.0_f64 / 3.0).ln();
    for a in &set {
        let want = if a.mode == Mode::Car { ln_two_thirds } else { 0.0 };
        assert!((a.ln_path_size - want).abs() < 1e-12, "{}: {}", a.mode, a.ln_path_size);
        let k = COLUMNS.iter().position(|&c| c == "car_ln_path_size").expect("a column");
        assert!((a.attributes()[k] - want).abs() < 1e-12, "{}", a.mode);
    }
}
