//! `max_car_routes` by hand: the car routes an origin-destination pair holds.
//!
//! One pair, O to D, by three routes of two links at 50 km/h: via A, 2 × 1 000 m, 144 s free,
//! its first link 600 veh/h; via B, 2 × 1 100 m, 158.4 s free, its first link 600 veh/h; via C,
//! 2 × 1 200 m, 172.8 s free, ample. 1 800 cars an hour for an hour, time alone, the least
//! cost. Iteration 0 finds A for everyone (the free-flow fastest); A's queue then makes B the
//! fastest, and once B queues too, C. So the pair holds A, then B, then C, unless the limit
//! stops it: at 1 it holds A alone, at 2 A and B, at 3 all three. No other route joins O to D,
//! so a limit of 3 or more is never reached and changes nothing (invariance).

mod common;

use std::collections::BTreeSet;

use common::{car_link, demand, link, network, stream, time_only};
use openmobisim_graph::network::Network;
use openmobisim_loop::parameters::Parameters;
use openmobisim_loop::run::{Run, run};
use openmobisim_types::ids::{EntityId, LinkId, TripId};
use openmobisim_types::mode::Mode;
use openmobisim_types::time::Second;

fn three_routes() -> Network {
    network(
        &[
            ("O", 0.0, 0.0),
            ("A", 1_000.0, 100.0),
            ("B", 1_000.0, 0.0),
            ("C", 1_000.0, -100.0),
            ("D", 2_000.0, 0.0),
        ],
        &[
            ("oa", "O", "A", car_link(1_000.0, 50.0, 600.0)),
            ("ad", "A", "D", car_link(1_000.0, 50.0, 36_000.0)),
            ("ob", "O", "B", car_link(1_100.0, 50.0, 600.0)),
            ("bd", "B", "D", car_link(1_100.0, 50.0, 36_000.0)),
            ("oc", "O", "C", car_link(1_200.0, 50.0, 36_000.0)),
            ("cd", "C", "D", car_link(1_200.0, 50.0, 36_000.0)),
        ],
    )
}

/// The run of the pair with `max_car_routes` at `limit`, ten iterations.
fn run_with(net: &Network, limit: u32) -> Run {
    let rows = stream(net, "t", "O", "D", 1_800.0, 1.0, Second(0));
    let dem = demand(net, &rows);
    let p = Parameters { max_car_routes: limit, ..time_only(10) };
    run(net, &dem, None, (Second(0), Second(7_200)), &p).expect("a run")
}

/// The first link of every car route in any set: the routes the pair holds, named by it.
fn held(net: &Network, r: &Run) -> BTreeSet<&'static str> {
    let first: Vec<LinkId> = (0..r.trips.departure.len())
        .flat_map(|i| r.set(TripId::from_index(i)).iter())
        .filter(|a| a.mode == Mode::Car)
        .filter_map(|a| a.route().and_then(|links| links.first().copied()))
        .collect();
    ["oa", "ob", "oc"].into_iter().filter(|id| first.contains(&link(net, id))).collect()
}

#[test]
fn the_pair_holds_at_most_the_limit_and_without_it_all_three_routes() {
    let net = three_routes();
    let all = run_with(&net, 100);
    assert_eq!(held(&net, &all), BTreeSet::from(["oa", "ob", "oc"]), "the case finds all three");
    assert_eq!(held(&net, &run_with(&net, 1)), BTreeSet::from(["oa"]), "1: the free-flow route");
    assert_eq!(held(&net, &run_with(&net, 2)), BTreeSet::from(["oa", "ob"]), "2: C never offered");
}

#[test]
fn a_limit_the_pair_never_reaches_changes_nothing() {
    let net = three_routes();
    let (three, many) = (run_with(&net, 3), run_with(&net, u32::MAX));
    assert_eq!(three.trips.arrival, many.trips.arrival);
    assert_eq!(three.trips.cost, many.trips.cost);
    assert_eq!(three.iterations.len(), many.iterations.len());
}
