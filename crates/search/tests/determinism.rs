//! Determinism, the one self-comparison: 2 000 queries on a loaded 20 × 20 grid give the same
//! routes and arrivals, bit for bit, under one thread and under eight, and over two runs built
//! from scratch.

mod common;

use common::{Draws, free_flow, grid_node, grid_with, l_route_cars, loaded, row, window};
use openmobisim_graph::network::LinkRow;
use openmobisim_search::search::{Query, Route, fastest_many};
use openmobisim_types::ids::EntityId;
use openmobisim_types::mode::Mode;
use openmobisim_types::time::Second;
use rayon::ThreadPoolBuilder;

/// An answer as its links' raw ids and its arrival's bits; `None` when unreachable.
type Answer = Option<(Vec<u32>, u64)>;

/// The grid's side, in nodes.
const N: u32 = 20;

/// Build the grid (100 m links of one lane at 50 km/h and 200 veh/h, so queues form), load
/// 6 000 cars on random L-shaped routes over 30 min, and answer 2 000 random queries (one in
/// ten by bike) departing in the first hour, on a pool of `threads` threads. The answers as
/// (links, arrival bits), and how many car answers differ from free flow by over 1 s.
fn run(threads: usize) -> (Vec<Answer>, usize) {
    let net = grid_with(N, |_, _| LinkRow { capacity_veh_h: Some(200.0), ..row(1, 50.0) });
    let mut draws = Draws::new(5);
    let cars = l_route_cars(&net, N, 6_000, 1_800, &mut draws);
    let times = loaded(&net, &cars, window(300, 7_200));
    let queries: Vec<Query> = (0..2_000)
        .map(|i| Query {
            origin: grid_node(&net, draws.below(N), draws.below(N)),
            destination: grid_node(&net, draws.below(N), draws.below(N)),
            mode: if i % 10 == 0 { Mode::Bike } else { Mode::Car },
            departure: Second(draws.below(3_600)),
        })
        .collect();
    let pool = ThreadPoolBuilder::new().num_threads(threads).build().expect("a pool");
    let routes = pool.install(|| fastest_many(&net, &times, &queries));
    let free = free_flow(&net);
    let at_free_flow = fastest_many(&net, &free, &queries);
    let arrival = |r: &Option<Route>| r.as_ref().expect("the grid is connected").arrival.get();
    let delayed =
        routes.iter().zip(&at_free_flow).filter(|(a, b)| arrival(a) > arrival(b) + 1.0).count();
    let bits = routes
        .iter()
        .map(|r| {
            r.as_ref()
                .map(|r| (r.links.iter().map(|l| l.raw()).collect(), r.arrival.get().to_bits()))
        })
        .collect();
    (bits, delayed)
}

#[test]
fn one_and_eight_threads_and_two_runs_give_the_same_routes_to_the_bit() {
    let (one, delayed) = run(1);
    let (eight, _) = run(8);
    let (again, _) = run(8);
    assert_eq!(one.len(), 2_000);
    // The loading must matter, or this compares free-flow searches only.
    assert!(delayed >= 100, "only {delayed} of 2 000 answers met a queue");
    let first_difference = |a: &[Answer], b: &[Answer]| a.iter().zip(b).position(|(x, y)| x != y);
    assert_eq!(first_difference(&one, &eight), None, "1 and 8 threads differ at this query");
    assert_eq!(first_difference(&eight, &again), None, "two runs differ at this query");
    eprintln!("{delayed} of 2 000 answers arrive over 1 s later than at free flow");
}
