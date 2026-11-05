//! Run time, reported and not asserted: 10 000 random car queries on the 100 × 100 grid (the
//! loading tests' fixture, 100 m links of one lane at 50 km/h and 1 800 veh/h) loaded with
//! 66 000 cars on random L-shaped routes over 90 minutes, on one thread and on eight. Run by
//! hand, in a release build:
//!
//! `cargo test --release -p openmobisim-search --test run_time -- --ignored --nocapture`

mod common;

use std::time::Instant;

use common::{Draws, grid, grid_node, l_route_cars, loaded, window};
use openmobisim_search::search::{Query, fastest_many};
use openmobisim_types::mode::Mode;
use openmobisim_types::time::Second;
use rayon::ThreadPoolBuilder;

#[test]
#[ignore = "run time, by hand in a release build (see the module documentation)"]
fn ten_thousand_queries_on_a_loaded_100_by_100_grid() {
    let n = 100;
    let t0 = Instant::now();
    let net = grid(n);
    let mut draws = Draws::new(6);
    let cars = l_route_cars(&net, n, 66_000, 5_400, &mut draws);
    let crossings: usize = cars.iter().map(|v| v.route.len()).sum();
    let times = loaded(&net, &cars, window(900, 7_200));
    eprintln!(
        "grid {n} x {n}, {crossings} crossings loaded and read in {:.3} s",
        t0.elapsed().as_secs_f64()
    );
    let queries: Vec<Query> = (0..10_000)
        .map(|_| Query {
            origin: grid_node(&net, draws.below(n), draws.below(n)),
            destination: grid_node(&net, draws.below(n), draws.below(n)),
            mode: Mode::Car,
            departure: Second(draws.below(5_400)),
        })
        .collect();
    let mut answers = Vec::new();
    for threads in [1, 8] {
        let pool = ThreadPoolBuilder::new().num_threads(threads).build().expect("a pool");
        let t = Instant::now();
        let routes = pool.install(|| fastest_many(&net, &times, &queries));
        let s = t.elapsed().as_secs_f64();
        let links: usize = routes.iter().flatten().map(|r| r.links.len()).sum();
        eprintln!(
            "{threads} thread(s): 10 000 queries in {s:.3} s, {:.1} µs per query; {links} links in all",
            s * 1e6 / 1e4
        );
        answers.push(routes);
    }
    assert_eq!(answers[0], answers[1], "1 and 8 threads give the same answers");
}
