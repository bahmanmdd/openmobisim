//! The path size a choice reads is the path size of the set it chooses from: a car route the
//! search has just added is in the set's path sizes at that same iteration's choice.
//!
//! Three nodes O, J, D. Two car routes O→J→D share the 99 m link O→J and differ in their last
//! metre (J→D by `one` or by `two`); a bike path O→D. Every coefficient is 0 but the path
//! size's (−1), so a cost is `−ln PS` and no time enters the choice; the nest scale is the
//! default 0.5. Four travellers a second for an hour, each open to car and bike.
//!
//! By hand. Iteration 0, free flow: the search finds one car route (the tie in time goes to
//! `one`) and the bike; the car's path size is 1, both costs are 0, so the car's share is 1/2.
//! Half the travellers drive over `one`, 7 200 veh/h into its 3 600 veh/h: a queue. Iteration
//! 1: the search finds the route over `two` for every traveller who met the queue. The two car
//! routes share 99 of their 100 m: `PS = 0.99 / 2 + 0.01 = 0.505` each, a cost of `−ln 0.505`,
//! and with the nests' scale 0.5 the car nest weighs `(2 · 0.505²)^0.5 = 0.505 √2` against the
//! bike's 1: the car's share is `0.505 √2 / (1 + 0.505 √2) = 0.41663`. (With the path sizes of
//! the set before it grew, both 1, it would be `√2 / (1 + √2) = 0.58579`.) At iteration 1 a
//! traveller chooses again with probability 1/2 and otherwise keeps the choice of iteration 0,
//! so after two iterations the car's share among the travellers with two car routes is
//! `1/2 · 1/2 + 1/2 · 0.41663 = 0.45831` (0.54289 with the stale path sizes), and 1/2 among
//! those with one. Judged at the binomial sampling error of the travellers, 3.29 standard
//! deviations (0.999 two-sided).
//!
//! For bikes: the bike is the alternative outside the car nest; it has no path size (0 in its
//! row) and its share is one less the car's.

#![allow(clippy::cast_precision_loss, reason = "counts of trips, far below 2^53")]

mod common;

use common::{at, car_link, network};
use openmobisim_demand::demand::Demand;
use openmobisim_demand::input::TripRow;
use openmobisim_graph::network::LinkRow;
use openmobisim_loop::parameters::{COLUMNS, Parameters};
use openmobisim_loop::run::run;
use openmobisim_types::ids::{EntityId, TripId};
use openmobisim_types::mode::{Mode, ModeSet};
use openmobisim_types::time::Second;
use openmobisim_types::units::Metres;

#[test]
fn a_car_route_added_by_the_search_is_in_the_path_sizes_of_the_choice_that_follows() {
    let bike = LinkRow {
        modes: ModeSet::of(&[Mode::Bike]),
        lanes: None,
        car_speed_km_h: None,
        length_m: Some(100.0),
        capacity_veh_h: None,
        signal_at_downstream_end: false,
    };
    let net = network(
        &[("O", 0.0, 0.0), ("J", 99.0, 0.0), ("D", 100.0, 0.0)],
        &[
            ("common", "O", "J", car_link(99.0, 36.0, 1.0e9)),
            ("one", "J", "D", car_link(1.0, 36.0, 3_600.0)),
            ("two", "J", "D", car_link(1.0, 36.0, 1.0e9)),
            ("path", "O", "D", bike),
        ],
    );
    let rows: Vec<TripRow> = (0..4 * 3_600_u32)
        .map(|i| TripRow {
            id: format!("t{i:05}"),
            origin: at(0.0, 0.0),
            destination: at(100.0, 0.0),
            departure: Second(i / 4),
            class: "default".to_string(),
            modes: ModeSet::of(&[Mode::Car, Mode::Bike]),
        })
        .collect();
    let dem = Demand::build(&rows, &["default".to_string()], &net, None, Metres(1_500.0))
        .expect("a demand");
    assert_eq!(COLUMNS[11], "car_ln_path_size");
    let mut b = [0.0; 16];
    b[11] = -1.0;
    let p = Parameters {
        iteration_cap: 2,
        iteration_floor: 2,
        coefficients: vec![b],
        ..Parameters::defaults(1)
    };
    assert!((p.nest_scale - 0.5).abs() < 1e-12, "the default nest scale");
    let result = run(&net, &dem, None, (Second(0), Second(3 * 3_600)), &p).expect("a run");
    assert_eq!(result.iterations.len(), 2);

    let fresh = 0.505 * 2.0_f64.sqrt() / (1.0 + 0.505 * 2.0_f64.sqrt());
    let (mut two_routes, mut cars_of_two, mut one_route, mut cars_of_one) = (0.0, 0.0, 0.0, 0.0);
    for i in 0..rows.len() {
        let id = TripId::from_index(i);
        let car = f64::from(u8::from(result.trips.mode[i] == Some(Mode::Car)));
        match result.trips.alternatives[i] {
            3 => {
                for a in result.set(id).iter().filter(|a| a.mode == Mode::Car) {
                    assert!((a.ln_path_size - 0.505_f64.ln()).abs() < 1e-12, "PS = 0.505");
                }
                two_routes += 1.0;
                cars_of_two += car;
            }
            2 => {
                one_route += 1.0;
                cars_of_one += car;
            }
            n => panic!("trip {i} has {n} alternatives"),
        }
    }
    println!(
        "{two_routes} travellers with two car routes: car share {:.5} (by hand {:.5}); {one_route} with one: {:.5} (by hand 0.5)",
        cars_of_two / two_routes,
        0.25 + 0.5 * fresh,
        cars_of_one / one_route
    );
    assert!(two_routes > 14_000.0, "nearly every traveller met the queue: {two_routes}");
    let want = 0.25 + 0.5 * fresh;
    let sd = (want * (1.0 - want) / two_routes).sqrt();
    assert!(
        (cars_of_two / two_routes - want).abs() <= 3.29 * sd,
        "car share {} against {want} ± {sd}",
        cars_of_two / two_routes
    );
}
