//! The least-cost choice reads no path size.
//!
//! The path size corrects the logit for routes that overlap (Ben-Akiva and Bierlaire 1999); the
//! least cost has no such error to correct. Judged, under the least-cost choice with the cost of
//! time alone (1 per minute): by hand, each trip's cost is the minutes of the alternative it
//! takes, whatever the coefficient of `car_ln_path_size`; and, an invariance, the trips table
//! (mode, arrival, travel time, cost, size of the set), every set, the alternative each
//! traveller takes and the iterations table are identical with the coefficient at 0, at −1
//! (the logit's own value) and at −5; and the least cost reads every coefficient as given but
//! that of the path size, which it reads as 0. How the logit reads it is judged in
//! `path_size_in_the_car_nest.rs`.
//!
//! The network of `path_size_in_a_run.rs`: three car routes O→D of 800, 1 000 and 1 200 m
//! sharing their first 600 m, the two shorter behind bottlenecks, 1 800 car-only travellers one
//! a second, 6 iterations. The sets come to hold two and three overlapping routes, with path
//! sizes from 0.5 to 0.75 (that file's hand values). Were the term read, it would add `β ln PS`
//! to a route's cost: at β = −1 between 0.29 and 0.69 minutes (`−ln 0.75` and `−ln 0.5`), more
//! for a shorter route than for a longer one of the same set, by 0.07 to 0.29 minutes; at −5
//! five times that. At free flow the routes are 1/3 of a minute apart, and the queues bring
//! their times within those margins, so the term, read, moves choices from iteration 1 on, and
//! raises the cost of every route of a set of two or three.
//!
//! Guards, not expected values: every trip finishes at every iteration (the window by hand in
//! that file), and at least 100 travellers end with three car routes.
//!
//! For bikes: a bike has one route and no path size; these travellers are car-only.

mod common;

use common::{TIME_ONLY, car_link, demand, network, print_iterations, stream, time_only};
use openmobisim_loop::parameters::{ChoiceModel, LN_PS, Parameters};
use openmobisim_loop::run::{Run, run};
use openmobisim_types::ids::{EntityId, TripId};
use openmobisim_types::mode::Mode;
use openmobisim_types::time::Second;

/// The run of the header with `coefficient` on `car_ln_path_size`.
fn with(coefficient: f64) -> Run {
    let unlimited = 1.0e9;
    let net = network(
        &[("O", 0.0, 0.0), ("J", 600.0, 0.0), ("D", 800.0, 0.0)],
        &[
            ("approach", "O", "J", car_link(600.0, 36.0, unlimited)),
            ("short", "J", "D", car_link(200.0, 36.0, 1_200.0)),
            ("middle", "J", "D", car_link(400.0, 36.0, 600.0)),
            ("long", "J", "D", car_link(600.0, 36.0, unlimited)),
        ],
    );
    let rows = stream(&net, "c", "O", "D", 3_600.0, 0.5, Second(0));
    assert_eq!(rows.len(), 1_800);
    let dem = demand(&net, &rows);
    let mut row = TIME_ONLY;
    row[LN_PS] = coefficient;
    let parameters = Parameters { coefficients: vec![row], ..time_only(6) };
    let result = run(&net, &dem, None, (Second(0), Second(4 * 3_600)), &parameters).expect("a run");
    println!("the coefficient of ln PS at {coefficient}:");
    print_iterations(&result);
    result
}

#[test]
fn a_least_cost_run_is_the_same_whatever_the_coefficient_of_the_path_size() {
    let plain = with(0.0);
    for it in &plain.iterations {
        assert_eq!(it.unfinished, 0, "iteration {}: unfinished trips", it.iteration);
    }
    let trips = plain.trips.mode.len();
    let three = (0..trips)
        .filter(|&i| {
            plain.set(TripId::from_index(i)).iter().filter(|a| a.mode == Mode::Car).count() == 3
        })
        .count();
    assert!(three >= 100, "only {three} travellers end with three car routes");

    for coefficient in [-1.0, -5.0] {
        let other = with(coefficient);
        for i in 0..trips {
            let taken = other.taken(TripId::from_index(i)).expect("every trip has a car route");
            let (cost, minutes) = (other.trips.cost[i], taken.time / 60.0);
            assert!(
                (cost - minutes).abs() < 1e-9,
                "trip {i} at {coefficient}: cost {cost}, the minutes it takes {minutes}"
            );
        }
        assert_eq!(other.trips, plain.trips, "the trips table at {coefficient}");
        for i in 0..trips {
            let trip = TripId::from_index(i);
            assert_eq!(other.set(trip), plain.set(trip), "trip {i}'s set at {coefficient}");
            assert_eq!(other.taken(trip), plain.taken(trip), "trip {i}'s choice at {coefficient}");
        }
        assert_eq!(other.iterations.len(), plain.iterations.len());
        for (a, b) in other.iterations.iter().zip(&plain.iterations) {
            let (mut a, mut b) = (a.clone(), b.clone());
            (a.stage_seconds, b.stage_seconds) = ([0.0; 5], [0.0; 5]);
            assert_eq!(a, b, "iteration {} at {coefficient}", b.iteration);
        }
    }
}

#[test]
fn the_least_cost_reads_the_coefficients_as_given_but_the_path_size_as_0() {
    let mut row = Parameters::DEFAULT_COEFFICIENTS;
    row[LN_PS] = -0.9;
    let given = Parameters {
        choice: ChoiceModel::Deterministic,
        coefficients: vec![row, row],
        ..Parameters::defaults(2)
    };
    let mut without = row;
    without[LN_PS] = 0.0;
    let by_hand = Parameters { coefficients: vec![without, without], ..given.clone() };
    assert_eq!(given.as_the_choice_reads(), by_hand, "only the path size's coefficient is 0");
}
