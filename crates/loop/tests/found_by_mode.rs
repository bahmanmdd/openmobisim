//! A route counts as found when it beats the routes of its own mode, whatever the traveller's
//! other modes cost.
//!
//! The two routes of test 3 (`two_routes.rs`): a short one of 288 s with a bottleneck of
//! 1 200 veh/h and a long one of 460.8 s, 1 800 car-only trips an hour at even seconds. Beside
//! them 1 800 travellers open to car and walk, leaving at the same seconds, with a footway O→D
//! of 300 m: 225 s on foot, less than any car time, so with the time-only least-cost choice
//! they all walk and add no car.
//!
//! By hand (as `at_iteration_1_the_found_and_switch_shares_are_the_hand_values`): iteration 0
//! puts the 1 800 cars on the short route, whose bottleneck (a 3 s headway) delays car `k`,
//! departing at `2k`, by `k` seconds: it takes `288 + k` s. At iteration 1 each car traveller
//! reads its own time on the short route, so the search finds the long route faster for
//! `288 + k > 460.8`, `k` from 173 on: 1 627 of the 1 800. A walker pricing the car is one
//! more car, one headway behind the car that entered when it would: `291 + k` s, so the long
//! route is faster for `k` from 170 on: 1 630 of the 1 800. For a walker the long route is
//! slower than the walk but faster than the one car route it had: found. The found share at
//! iteration 1 is `(1 627 + 1 630) / 3 600 = 0.9047` (0.4519 if a walker's new car route did
//! not count), exactly.
//!
//! For bikes: the same rule; a bike has one route, found at iteration 0.

mod common;

use common::{at, car_link, network, print_iterations, stream, time_only};
use openmobisim_demand::demand::Demand;
use openmobisim_demand::input::TripRow;
use openmobisim_graph::network::LinkRow;
use openmobisim_loop::run::run;
use openmobisim_types::mode::{Mode, ModeSet};
use openmobisim_types::time::Second;
use openmobisim_types::units::Metres;

#[test]
fn a_faster_car_route_is_found_for_a_traveller_who_walks() {
    let footway = LinkRow {
        modes: ModeSet::of(&[Mode::Walk]),
        lanes: None,
        car_speed_km_h: None,
        length_m: Some(300.0),
        capacity_veh_h: None,
        signal_at_downstream_end: false,
    };
    let net = network(
        &[("O", 0.0, 0.0), ("P", 2_000.0, 100.0), ("Q", 2_000.0, -100.0), ("D", 4_000.0, 0.0)],
        &[
            ("op", "O", "P", car_link(2_000.0, 50.0, 1_200.0)),
            ("pd", "P", "D", car_link(2_000.0, 50.0, 36_000.0)),
            ("oq", "O", "Q", car_link(3_200.0, 50.0, 36_000.0)),
            ("qd", "Q", "D", car_link(3_200.0, 50.0, 36_000.0)),
            ("walk", "O", "D", footway),
        ],
    );
    let mut rows = stream(&net, "c", "O", "D", 1_800.0, 1.0, Second(0));
    let walkers: Vec<TripRow> = rows
        .iter()
        .map(|car| TripRow {
            id: format!("w{}", car.id),
            origin: at(0.0, 0.0),
            destination: at(4_000.0, 0.0),
            modes: ModeSet::of(&[Mode::Car, Mode::Walk]),
            ..car.clone()
        })
        .collect();
    rows.extend(walkers);
    let dem = Demand::build(&rows, &["default".to_string()], &net, None, Metres(1_500.0))
        .expect("a demand");
    let result =
        run(&net, &dem, None, (Second(0), Second(3 * 3_600)), &time_only(2)).expect("a run");
    print_iterations(&result);
    let walking = result.trips.mode.iter().filter(|m| **m == Some(Mode::Walk)).count();
    assert_eq!(walking, 1_800, "the travellers open to walking walk");
    let one = &result.iterations[1];
    assert!((result.iterations[0].found_share - 1.0).abs() < 1e-12, "everyone found at 0");
    let by_hand = (1_627.0 + 1_630.0) / 3_600.0;
    assert!((one.found_share - by_hand).abs() < 1e-12, "found {}", one.found_share);
}
