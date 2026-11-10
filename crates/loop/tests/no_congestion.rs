//! The whole loop at its default parameters, without congestion: the mode shares of a run are
//! the logit's closed form on the free-flow times, written out by hand here.
//!
//! Two nodes O and D. A street O→D for cars, 2 000 m at 50 km/h with unlimited capacity:
//! 2 000 / (50 000 / 3 600) = 144 s. A path O→D for bikes and walkers, 1 500 m: a bike at
//! 15 km/h takes 360 s, a walker at 4.8 km/h takes 1 125 s. Every trip starts at O and ends at
//! D (no gap to walk). Nothing queues, so no cost ever changes.
//!
//! Each mode offers one alternative, so each nest holds one row and the nested logit is the
//! plain logit whatever the nest scale: `P(mode) = exp(−V_mode) / Σ exp(−V)`, with `V` the
//! cost of the default utilities (times in minutes; no walk between door and node, one car
//! route so `ln PS = 0`, no money):
//!
//! * Car: 0.1 per minute driving, constant 0: `0.1 · 144 / 60` = 0.24.
//! * Bike: 0.1 per minute riding, constant 0: `0.1 · 360 / 60` = 0.6.
//! * Walk: 0.165 per minute walking, constant 0.5: `0.165 · 1 125 / 60 + 0.5` = 3.09375 + 0.5
//!   = 3.59375.
//! * Car, bike, walk: `exp(−0.24)` = 0.786628, `exp(−0.6)` = 0.548812, `exp(−3.59375)` =
//!   0.027495, over their sum 1.362935: 0.5772, 0.4027, 0.0202.
//! * With transit: a run leaves O every 600 s (at 0, 600, ...) and reaches D 300 s later. With
//!   the default boarding slack of 60 s a traveller departing at `d` boards the first run
//!   leaving at or after `d + 60`, so the transit time is `ceil((d + 60) / 600) · 600 + 300 − d`,
//!   between 360 s and 959 s. By its parts: the stops are at O and D themselves, so access and
//!   egress walks are 0 and the traveller is at the stop at `d`; the first wait is
//!   `ceil((d + 60) / 600) · 600 − d` s (60 to 659 s, the slack included); in the vehicle 300 s
//!   (5 min); one ride, no transfer. Its cost is 0.1 per minute in the vehicle, 0.145 per
//!   minute of first wait and the constant 0.25: `0.1 · 5 + 0.145 · wait / 60 + 0.25`, from
//!   0.895 to 2.3426. The shares then differ by traveller; the expected count of a mode is the
//!   sum of the travellers' probabilities.
//!
//! Judged: every trip's time is its mode's hand time (exactly); the counts by Pearson's
//! chi-square at the 0.999 quantile (a correct sampler fails once in a thousand seeds); and,
//! since tastes are fixed and costs never change, nobody switches and nothing new is found
//! after iteration 0, so the stopping rule of the specification ends the run at its first
//! reading of five iterations.
//!
//! For bikes: the bike is one of the modes judged, at 360 s and its closed-form share (0.4027
//! of the trips without transit).

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "counts of trips and seconds of the window, far below 2^53 and above zero"
)]

mod common;

use common::{VALUES, at, network};
use openmobisim_demand::demand::Demand;
use openmobisim_demand::input::TripRow;
use openmobisim_graph::network::{LinkRow, Network};
use openmobisim_loop::parameters::{COLUMNS, Parameters};
use openmobisim_loop::run::{Run, run};
use openmobisim_transit::timetable::{Call, Timetable, TimetableBuilder};
use openmobisim_types::mode::{Mode, ModeSet};
use openmobisim_types::time::Second;
use openmobisim_types::units::Metres;

const CAR_S: f64 = 144.0;
const BIKE_S: f64 = 360.0;
const WALK_S: f64 = 1_125.0;
/// The default coefficients the hand costs read, by column name.
const DEFAULTS: [(&str, f64); 8] = [
    ("car_drive_min", 0.1),
    ("bike_ride_min", 0.1),
    ("walk_min", 0.165),
    ("transit_in_vehicle_min", 0.1),
    ("transit_first_wait_min", 0.145),
    ("is_car", 0.0),
    ("is_bike", 0.0),
    ("is_walk", 0.5),
];
/// The transit constant.
const TRANSIT_CONSTANT: f64 = 0.25;
/// The hand costs of car, bike and walk.
const CAR_COST: f64 = 0.1 * CAR_S / 60.0;
const BIKE_COST: f64 = 0.1 * BIKE_S / 60.0;
const WALK_COST: f64 = 0.165 * WALK_S / 60.0 + 0.5;
/// The 0.999 quantiles of the chi-square distribution, 2 and 3 degrees of freedom (tables).
const CHI_SQUARE_999_DF2: f64 = 13.816;
const CHI_SQUARE_999_DF3: f64 = 16.266;
/// Trips per second of the hour of departures.
const PER_SECOND: usize = 10;

fn street_and_path() -> Network {
    assert!((VALUES.bike_speed - 15.0).abs() < 1e-12 && (VALUES.walk_speed - 4.8).abs() < 1e-12);
    let street = LinkRow {
        modes: ModeSet::of(&[Mode::Car]),
        lanes: Some(1),
        car_speed_km_h: Some(50.0),
        length_m: Some(2_000.0),
        capacity_veh_h: Some(f64::INFINITY),
        signal_at_downstream_end: false,
    };
    let path = LinkRow {
        modes: ModeSet::of(&[Mode::Bike, Mode::Walk]),
        lanes: None,
        car_speed_km_h: None,
        length_m: Some(1_500.0),
        capacity_veh_h: None,
        signal_at_downstream_end: false,
    };
    network(
        &[("O", 0.0, 0.0), ("D", 1_500.0, 0.0)],
        &[("street", "O", "D", street), ("path", "O", "D", path)],
    )
}

/// `PER_SECOND` trips at every second of the first hour, O to D, open to `modes`.
fn trips(modes: &[Mode]) -> Vec<TripRow> {
    (0..3_600 * PER_SECOND)
        .map(|i| TripRow {
            id: format!("t{i:06}"),
            origin: at(0.0, 0.0),
            destination: at(1_500.0, 0.0),
            departure: Second((i / PER_SECOND) as u32),
            class: "default".to_string(),
            modes: ModeSet::of(modes),
        })
        .collect()
}

/// A run leaving O at every multiple of 600 s up to two hours, at D 300 s later.
fn timetable() -> Timetable {
    let mut b = TimetableBuilder::new();
    b.add_stop("O", (Metres(0.0), Metres(0.0)));
    b.add_stop("D", (Metres(1_500.0), Metres(0.0)));
    for k in 0..=12_u32 {
        let (dep, arr) = (Second(600 * k), Second(600 * k + 300));
        b.add_run(
            format!("r{k:02}"),
            vec![
                Call { stop: "O".to_string(), arrival: dep, departure: dep },
                Call { stop: "D".to_string(), arrival: arr, departure: arr },
            ],
        );
    }
    b.build().expect("a timetable")
}

/// The transit time by hand of a traveller departing at second `d`.
fn transit_s(d: u32) -> f64 {
    let run = (d + 60).div_ceil(600) * 600;
    f64::from(run + 300 - d)
}

/// The transit cost by hand of a traveller departing at second `d`: 5 min in the vehicle at
/// 0.1, the first wait (the run's departure less `d`, minutes) at 0.145, the constant 0.25.
fn transit_cost(d: u32) -> f64 {
    let wait = f64::from((d + 60).div_ceil(600) * 600 - d);
    0.1 * 5.0 + 0.145 * wait / 60.0 + TRANSIT_CONSTANT
}

/// That the defaults are the ones the hand costs are written for.
fn defaults_are_the_hand_costs() {
    let b = Parameters::DEFAULT_COEFFICIENTS;
    for (name, value) in DEFAULTS.into_iter().chain([("is_transit", TRANSIT_CONSTANT)]) {
        let k = COLUMNS.iter().position(|&c| c == name).expect("a column");
        assert!(b[k].to_bits() == value.to_bits(), "{name}: {} against {value}", b[k]);
    }
}

/// The logit by its formula over `costs`.
fn logit(costs: &[f64]) -> Vec<f64> {
    let total: f64 = costs.iter().map(|v| (-v).exp()).sum();
    costs.iter().map(|v| (-v).exp() / total).collect()
}

/// Pearson's statistic of `counts` against the expected counts.
fn chi_square(counts: &[f64], expected: &[f64]) -> f64 {
    counts.iter().zip(expected).map(|(c, e)| (c - e).powi(2) / e).sum()
}

/// What holds in every run without congestion: every trip finishes, nobody switches and nothing
/// is found after iteration 0, and the rule stops the run at its first reading.
fn settled_at_once(result: &Run) {
    common::print_iterations(result);
    assert!(result.trips.finished.iter().all(|&f| f), "every trip finishes");
    for it in &result.iterations[1..] {
        assert!(it.switch_share.abs() < 1e-15, "a switch at iteration {}", it.iteration);
        assert!(it.found_share.abs() < 1e-15, "a find at iteration {}", it.iteration);
    }
    assert!(result.stopped_by_rule, "the rule stops the run");
    assert_eq!(result.iterations.len(), 5, "at the first five iterations");
}

/// Car, bike and walk at `seed`: the chi-square statistic of the counts (2 degrees of freedom).
fn three_modes(seed: u64) -> f64 {
    let net = street_and_path();
    let modes = [Mode::Car, Mode::Bike, Mode::Walk];
    let rows = trips(&modes);
    let dem = Demand::build(&rows, &["default".to_string()], &net, None, Metres(1_500.0))
        .expect("a demand");
    defaults_are_the_hand_costs();
    let parameters = Parameters { seed, ..Parameters::defaults(1) };
    let result = run(&net, &dem, None, (Second(0), Second(7_200)), &parameters).expect("a run");
    settled_at_once(&result);
    let times = [CAR_S, BIKE_S, WALK_S];
    let mut counts = [0.0; 3];
    for i in 0..rows.len() {
        let m = modes.iter().position(|&m| Some(m) == result.trips.mode[i]).expect("a mode");
        assert!((result.trips.travel_time[i] - times[m]).abs() < 1e-9, "trip {i}");
        counts[m] += 1.0;
    }
    let p = logit(&[CAR_COST, BIKE_COST, WALK_COST]);
    assert!((p[0] - 0.5772).abs() < 5e-5 && (p[1] - 0.4027).abs() < 5e-5, "the hand shares");
    let expected: Vec<f64> = p.iter().map(|p| p * rows.len() as f64).collect();
    let statistic = chi_square(&counts, &expected);
    println!("seed {seed}: car, bike, walk {counts:?} against {expected:.1?}: {statistic:.2}");
    statistic
}

/// The four modes at `seed`: the chi-square statistic of the counts (3 degrees of freedom).
fn four_modes(seed: u64) -> f64 {
    let net = street_and_path();
    let tt = timetable();
    let modes = [Mode::Car, Mode::Bike, Mode::Walk, Mode::Transit];
    let rows = trips(&modes);
    let dem = Demand::build(&rows, &["default".to_string()], &net, Some(&tt), Metres(1_500.0))
        .expect("a demand");
    defaults_are_the_hand_costs();
    let parameters = Parameters { seed, ..Parameters::defaults(1) };
    assert_eq!(parameters.boarding_slack, 60, "the default the hand time is written for");
    let result =
        run(&net, &dem, Some(&tt), (Second(0), Second(7_200)), &parameters).expect("a run");
    settled_at_once(&result);
    let (mut counts, mut expected) = ([0.0; 4], [0.0; 4]);
    for i in 0..rows.len() {
        let d = result.trips.departure[i].get();
        let times = [CAR_S, BIKE_S, WALK_S, transit_s(d)];
        let m = modes.iter().position(|&m| Some(m) == result.trips.mode[i]).expect("a mode");
        assert!(
            (result.trips.travel_time[i] - times[m]).abs() < 1e-9,
            "trip {i} departing at {d} by {:?}: {} against {}",
            modes[m],
            result.trips.travel_time[i],
            times[m]
        );
        counts[m] += 1.0;
        let costs = [CAR_COST, BIKE_COST, WALK_COST, transit_cost(d)];
        for (e, p) in expected.iter_mut().zip(logit(&costs)) {
            *e += p;
        }
    }
    let statistic = chi_square(&counts, &expected);
    println!(
        "seed {seed}: car, bike, walk, transit {counts:?} against {expected:.1?}: {statistic:.2}"
    );
    statistic
}

#[test]
fn car_bike_and_walk_take_their_closed_form_shares_and_their_hand_times() {
    let statistic = three_modes(0);
    assert!(statistic < CHI_SQUARE_999_DF2, "chi-square {statistic}");
}

#[test]
fn with_transit_every_traveller_has_its_own_hand_time_and_the_counts_their_expected_sums() {
    let statistic = four_modes(0);
    assert!(statistic < CHI_SQUARE_999_DF3, "chi-square {statistic}");
}

/// A measurement, not a test: the statistic over 20 seeds, to see that seed 0 is not a lucky
/// one. Under a correct sampler about one seed in twenty exceeds the 0.95 quantile (5.99 with
/// 2 degrees of freedom, 7.81 with 3).
#[test]
#[ignore = "a measurement over 20 seeds; run by hand with --ignored --nocapture"]
fn the_statistics_over_twenty_seeds() {
    let three: Vec<f64> = (0..20).map(three_modes).collect();
    let four: Vec<f64> = (0..20).map(four_modes).collect();
    println!("three modes, 2 degrees of freedom: {three:.2?}");
    println!("four modes, 3 degrees of freedom: {four:.2?}");
}
