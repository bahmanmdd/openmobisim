//! Run time, reported and not asserted: cars on straight routes across a grid (the network
//! tests' fixture, 100 m links of one lane at 50 km/h and 1 800 veh/h). The small case runs in
//! the suite and checks conservation; the large one is run by hand, in a release build:
//!
//! `cargo test --release -p openmobisim-loading --test run_time -- --ignored --nocapture`

mod common;

use std::time::Instant;

use common::{grid, grid_route, vehicle, window};
use openmobisim_graph::network::Network;
use openmobisim_loading::input::Vehicle;
use openmobisim_loading::point_queue::{Loading, load};
use openmobisim_types::ids::{EntityId, LinkId};
use openmobisim_types::mode::Mode;

/// The `4n` straight routes across an `n × n` grid: each row east and west, each column north
/// and south, end to end.
fn straight_routes(net: &Network, n: u32) -> Vec<Vec<LinkId>> {
    let mut routes = Vec::new();
    for i in 0..n {
        let row: Vec<(u32, u32)> = (0..n).map(|c| (i, c)).collect();
        let column: Vec<(u32, u32)> = (0..n).map(|r| (r, i)).collect();
        for nodes in [row, column] {
            let mut back = nodes.clone();
            back.reverse();
            routes.push(grid_route(net, &nodes));
            routes.push(grid_route(net, &back));
        }
    }
    routes
}

/// `cars` cars spread evenly over `seconds`, car `i` on route `i mod routes`.
fn demand(routes: &[Vec<LinkId>], cars: u32, seconds: u32) -> Vec<Vehicle> {
    let r = u32::try_from(routes.len()).unwrap();
    (0..cars)
        .map(|i| {
            let departure =
                u32::try_from(u64::from(i) * u64::from(seconds) / u64::from(cars)).unwrap();
            vehicle(i, Mode::Car, &routes[(i % r) as usize], departure)
        })
        .collect()
}

/// Load `cars` over `seconds` on an `n × n` grid in a two-hour window of 15-min bins; print the
/// time. The loading and the crossings.
fn timed(n: u32, cars: u32, seconds: u32) -> (Loading, Vec<Vehicle>) {
    let t0 = Instant::now();
    let net = grid(n);
    let built = t0.elapsed();
    let vehicles = demand(&straight_routes(&net, n), cars, seconds);
    let crossings: usize = vehicles.iter().map(|v| v.route.len()).sum();
    let t1 = Instant::now();
    let run = load(&net, &vehicles, window(900, 7_200)).unwrap();
    let loaded = t1.elapsed();
    #[allow(clippy::cast_precision_loss, reason = "a count of crossings, far below 2^52")]
    let per_crossing = loaded.as_secs_f64() * 1e9 / crossings as f64;
    eprintln!(
        "grid {n} x {n}: built in {:.3} s; {cars} cars, {crossings} crossings loaded in {:.3} s, \
         {per_crossing:.1} ns per crossing; {} finished, {} unfinished, {} rows",
        built.as_secs_f64(),
        loaded.as_secs_f64(),
        run.finished.len(),
        run.unfinished.len(),
        run.links.len()
    );
    (run, vehicles)
}

#[test]
fn every_car_on_a_20_by_20_grid_is_counted_once_on_every_link_of_its_route() {
    // Conservation: 8 000 cars over 30 min on the 80 straight routes, 100 cars each, far below
    // capacity; every car arrives within the window, and each link's cars in the table are the
    // cars of the one route through it, 100, by hand.
    let (run, vehicles) = timed(20, 8_000, 1_800);
    assert!(run.unfinished.is_empty());
    assert_eq!(run.finished.len(), vehicles.len());
    let t = &run.links;
    let mut per_link = vec![0_u32; 20 * 19 * 4];
    for r in 0..t.len() {
        per_link[t.links()[r].index()] += t.cars()[r];
    }
    assert!(per_link.iter().all(|&c| c == 100), "every link carries the 100 cars of its route");
}

#[test]
#[ignore = "run time, by hand in a release build (see the module documentation)"]
fn a_100_by_100_grid_loads_66_000_cars_over_90_minutes() {
    let (run, vehicles) = timed(100, 66_000, 5_400);
    assert_eq!(run.finished.len() + run.unfinished.len(), vehicles.len());
}
