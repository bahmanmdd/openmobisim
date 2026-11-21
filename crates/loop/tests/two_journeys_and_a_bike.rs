//! The whole loop at its default parameters, without congestion: a nest of two transit journeys
//! against a bike takes the nested logit's closed-form shares, at the default nest scale and at
//! a second one, written out by hand here.
//!
//! Two nodes O and D, 2 500 m apart. A path O→D for bikes and walkers, 2 500 m: a bike at
//! 15 km/h rides it in 600 s. Two stops, at O and at D: each is 2 500 m from the other end, more
//! than the default `stop_walk_max` of 1 500 m, so each trip's access list holds the stop at O
//! alone and its egress list the stop at D alone, both at a walk of 0 s (no nearer stop is tried
//! alone). Two runs O→D: `slow` leaves at 60 s and arrives at 540 s (8 min in the vehicle),
//! `fast` leaves at 360 s and arrives at 480 s (2 min). Every traveller departs at second 0 from
//! O for D, open to bike and transit only: no car, so nothing queues and no cost ever changes.
//!
//! The set by hand. With the default boarding slack of 60 s a traveller at the stop at 0 s can
//! board a run leaving at 60 s or later, so both runs. The earliest journey with one ride is
//! `fast` (at D at 480 s); a second ride reaches nothing earlier, so it is the one earliest
//! journey. The cheapest journey with one ride for the default class is `slow` (below); a second
//! ride is dearer. So the set holds three alternatives: the bike route, then the two journeys, on
//! different runs.
//!
//! The costs of the default coefficients (`PARAMETERS.md`; times in minutes, no walk between door
//! and node or to the stops, no money): 0.1 per minute riding, 0.1 per minute in the vehicle,
//! 0.145 per minute of first wait (from the arrival at the stop, the slack included), the
//! constants 0 for bike and 0.25 for transit.
//!
//! * Bike, 600 s: `0.1 · 10` = 1.0.
//! * `slow`, 540 s door to door: wait 1 min, ride 8 min: `0.145 · 1 + 0.1 · 8 + 0.25` = 1.195.
//! * `fast`, 480 s door to door: wait 6 min, ride 2 min: `0.145 · 6 + 0.1 · 2 + 0.25` = 1.32.
//!
//! The nested logit with a nest per mode at nest scale μ (Ben-Akiva and Lerman 1985, chapter
//! 10), with `v_j = −c_j / μ` for the two journeys: `I = ln(exp(v_slow) + exp(v_fast))`, the
//! transit nest's inclusive value; `P(transit) = exp(μ I) / (exp(μ I) + exp(−c_bike))` (the bike
//! nest holds one alternative, its inclusive value times μ is `−c_bike` at any μ);
//! `P(j) = P(transit) · exp(v_j) / Σ exp(v_k)`. To six places:
//!
//! * μ = 0.5 (the default): `I = −2.39 + ln(1 + exp(−0.25))` = −1.814061, `μ I` = −0.907030;
//!   transit 0.523226, of which `slow` 0.294145 and `fast` 0.229081; bike 0.476774.
//! * μ = 1: `I = −1.195 + ln(1 + exp(−0.125))` = −0.562401; transit 0.607687, of which `slow`
//!   0.322809 and `fast` 0.284878; bike 0.392313.
//!
//! The transit share at μ = 1 is 0.084461 above that at μ = 0.5. At 36 000 travellers one run's
//! transit share has a binomial standard deviation of 0.002632 at μ = 0.5 and 0.002573 at
//! μ = 1: the two scales' shares are 32 (or 33) such standard deviations apart, 23 of the
//! standard deviation of their difference, so a run that read the nest scale wrong (or not at
//! all) fails the chi-square below at one of the two.
//!
//! Judged, at each μ: every set holds exactly one bike route and two journeys on different runs;
//! every trip's time is its alternative's hand time (exactly: 600, 540, 480 s); the three counts
//! by Pearson's chi-square at the 0.999 quantile with 2 degrees of freedom (13.816; a correct
//! sampler fails once in a thousand seeds), as `no_congestion.rs` does.
//!
//! For bikes: the bike is the nest of one against the transit nest of two; its share is one of
//! the three counts judged (0.476774 at μ = 0.5, 0.392313 at μ = 1), at 600 s exactly.

#![allow(clippy::cast_precision_loss, reason = "counts of trips, far below 2^53")]

mod common;

use common::{VALUES, at, network};
use openmobisim_demand::demand::Demand;
use openmobisim_demand::input::TripRow;
use openmobisim_graph::network::{LinkRow, Network};
use openmobisim_loop::alternative::{Alternative, Leg};
use openmobisim_loop::parameters::{COLUMNS, Parameters};
use openmobisim_loop::run::run;
use openmobisim_transit::timetable::{Call, Timetable, TimetableBuilder};
use openmobisim_types::ids::{EntityId, TripId};
use openmobisim_types::mode::{Mode, ModeSet};
use openmobisim_types::time::Second;
use openmobisim_types::units::Metres;

/// The hand times, door to door, seconds: the bike, `slow`, `fast`.
const TIMES_S: [f64; 3] = [600.0, 540.0, 480.0];
/// The hand costs: the bike, `slow`, `fast`.
const COSTS: [f64; 3] = [0.1 * 10.0, 0.145 + 0.1 * 8.0 + 0.25, 0.145 * 6.0 + 0.1 * 2.0 + 0.25];
/// The default coefficients the hand costs read, by column name.
const DEFAULTS: [(&str, f64); 5] = [
    ("bike_ride_min", 0.1),
    ("transit_in_vehicle_min", 0.1),
    ("transit_first_wait_min", 0.145),
    ("is_bike", 0.0),
    ("is_transit", 0.25),
];
/// The 0.999 quantile of the chi-square distribution, 2 degrees of freedom (tables).
const CHI_SQUARE_999_DF2: f64 = 13.816;
/// Travellers, all departing at second 0.
const TRAVELLERS: usize = 36_000;

/// The path O→D and the two runs of the header.
fn path_and_stops() -> (Network, Timetable) {
    assert!((VALUES.bike_speed - 15.0).abs() < 1e-12, "the speed the hand time is written for");
    let path = LinkRow {
        modes: ModeSet::of(&[Mode::Bike, Mode::Walk]),
        lanes: None,
        car_speed_km_h: None,
        length_m: Some(2_500.0),
        capacity_veh_h: None,
        signal_at_downstream_end: false,
    };
    let net = network(&[("O", 0.0, 0.0), ("D", 2_500.0, 0.0)], &[("path", "O", "D", path)]);
    let mut b = TimetableBuilder::new();
    b.add_stop("O", (Metres(0.0), Metres(0.0)));
    b.add_stop("D", (Metres(2_500.0), Metres(0.0)));
    for (id, dep, arr) in [("slow", 60, 540), ("fast", 360, 480)] {
        b.add_run(
            id.to_string(),
            vec![
                Call { stop: "O".to_string(), arrival: Second(dep), departure: Second(dep) },
                Call { stop: "D".to_string(), arrival: Second(arr), departure: Second(arr) },
            ],
        );
    }
    (net, b.build().expect("a timetable"))
}

/// The nested logit by its closed form (the header's formulas) at nest scale `mu`: the bike,
/// `slow`, `fast`.
fn by_hand(mu: f64) -> [f64; 3] {
    let [bike, slow, fast] = COSTS;
    let inclusive = ((-slow / mu).exp() + (-fast / mu).exp()).ln();
    let transit = (mu * inclusive).exp() / ((mu * inclusive).exp() + (-bike).exp());
    let within = |c: f64| (-c / mu).exp() / inclusive.exp();
    [1.0 - transit, transit * within(slow), transit * within(fast)]
}

/// Which of the three `a` is: 0 the bike, 1 `slow` (boarded at 60 s), 2 `fast` (boarded at
/// 360 s), by its mode and, for a journey of one ride, the second its run leaves O.
fn which(a: &Alternative) -> usize {
    let journey = a.legs.iter().find_map(|l| if let Leg::Journey(j) = l { Some(j) } else { None });
    match (a.mode, journey) {
        (Mode::Bike, None) => 0,
        (Mode::Transit, Some(j)) if j.rides.len() == 1 => match j.rides[0].board.1.get() {
            60 => 1,
            360 => 2,
            other => panic!("a run leaving at {other} s"),
        },
        other => panic!("not one of the three: {other:?}"),
    }
}

/// The run at nest scale `mu`, the other parameters the defaults: the sets and every trip's time
/// checked, the counts of the bike, `slow` and `fast` printed; the chi-square statistic of the
/// counts against the hand shares.
fn at_scale(mu: f64) -> f64 {
    let b = Parameters::DEFAULT_COEFFICIENTS;
    for (name, value) in DEFAULTS {
        let k = COLUMNS.iter().position(|&c| c == name).expect("a column");
        assert!(b[k].to_bits() == value.to_bits(), "{name}: {} against {value}", b[k]);
    }
    let (net, tt) = path_and_stops();
    let rows: Vec<TripRow> = (0..TRAVELLERS)
        .map(|i| TripRow {
            id: format!("t{i:06}"),
            origin: at(0.0, 0.0),
            destination: at(2_500.0, 0.0),
            departure: Second(0),
            class: "default".to_string(),
            modes: ModeSet::of(&[Mode::Bike, Mode::Transit]),
        })
        .collect();
    let dem = Demand::build(&rows, &["default".to_string()], &net, Some(&tt), Metres(1_500.0))
        .expect("a demand");
    let parameters = Parameters { nest_scale: mu, ..Parameters::defaults(1) };
    assert_eq!(parameters.boarding_slack, 60, "the default the hand set is written for");
    let started = std::time::Instant::now();
    let result =
        run(&net, &dem, Some(&tt), (Second(0), Second(3_600)), &parameters).expect("a run");
    let seconds = started.elapsed().as_secs_f64();
    let mut counts = [0.0; 3];
    for i in 0..rows.len() {
        let set = result.set(TripId::from_index(i));
        // The bike route, then the earliest journey, then the cheapest: the three, each once.
        let found: Vec<usize> = set.iter().map(which).collect();
        assert_eq!(found, [0, 2, 1], "trip {i}'s set");
        for a in set {
            assert!(a.time.to_bits() == TIMES_S[which(a)].to_bits(), "trip {i}: {}", a.time);
        }
        let k = which(result.taken(TripId::from_index(i)).expect("an alternative"));
        assert!(
            result.trips.travel_time[i].to_bits() == TIMES_S[k].to_bits(),
            "trip {i}: {} against {}",
            result.trips.travel_time[i],
            TIMES_S[k]
        );
        counts[k] += 1.0;
    }
    let p = by_hand(mu);
    let expected: Vec<f64> = p.iter().map(|p| p * rows.len() as f64).collect();
    let statistic: f64 = counts.iter().zip(&expected).map(|(c, e)| (c - e).powi(2) / e).sum();
    println!(
        "nest scale {mu}: bike, slow, fast {counts:?} against {expected:.1?} (shares {p:.6?}): chi-square {statistic:.2}; the run {seconds:.1} s"
    );
    statistic
}

#[test]
fn at_the_default_nest_scale_two_journeys_and_a_bike_take_the_nested_logit_shares() {
    let p = by_hand(0.5);
    assert!((p[0] - 0.476_774).abs() < 1e-6 && (p[1] - 0.294_145).abs() < 1e-6, "{p:?}");
    let statistic = at_scale(0.5);
    assert!(statistic < CHI_SQUARE_999_DF2, "chi-square {statistic}");
}

#[test]
fn at_nest_scale_1_two_journeys_and_a_bike_take_the_plain_logit_shares() {
    let p = by_hand(1.0);
    assert!((p[0] - 0.392_313).abs() < 1e-6 && (p[1] - 0.322_809).abs() < 1e-6, "{p:?}");
    let statistic = at_scale(1.0);
    assert!(statistic < CHI_SQUARE_999_DF2, "chi-square {statistic}");
}
