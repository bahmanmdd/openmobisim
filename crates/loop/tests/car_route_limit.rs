//! `max_car_routes` by hand: the car routes an origin-destination pair holds.
//!
//! One pair, O to D, by three routes of two links at 50 km/h: via A, 2 × 1 000 m, 144 s free,
//! its first link `oa` 600 veh/h; via B, 2 × 1 100 m, 158.4 s free, its first link `ob` 600
//! veh/h; via C, 2 × 1 200 m, 172.8 s free, ample. Besides the pair's 1 800 cars an hour, two
//! other pairs, O to A and O to B, send 600 cars an hour each over `oa` and `ob`: each link is at
//! its capacity before any of the pair's cars, so a car of the pair on it starts a queue that
//! grows for the rest of the hour. Time alone, the least cost. Iteration 0: the pair finds A
//! (the free-flow fastest). A then queues and B does not, so at iteration 1 the search finds B.
//! Whatever share of the pair moves to B, both links then carry more than their capacity, so
//! their queues grow, and once A's delay passes 28.8 s and B's 14.4 s, C is the fastest: the
//! search finds it at iteration 2. So the pair holds A, then B, then C, unless the limit stops it:
//! at 1 it holds A alone, at 2 A and B, at 3 all three. No other route joins O to D, so a limit
//! of 3 or more is never reached and changes nothing (invariance).

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

/// The run of the pair and the two other pairs with `max_car_routes` at `limit`, ten iterations.
fn run_with(net: &Network, limit: u32) -> Run {
    let mut rows = stream(net, "t", "O", "D", 1_800.0, 1.0, Second(0));
    rows.extend(stream(net, "a", "O", "A", 600.0, 1.0, Second(0)));
    rows.extend(stream(net, "b", "O", "B", 600.0, 1.0, Second(0)));
    let dem = demand(net, &rows);
    let p = Parameters { max_car_routes: limit, ..time_only(10) };
    run(net, &dem, None, (Second(0), Second(7_200)), &p).expect("a run")
}

/// The routes of the pair O to D in any set, each named by its last link (the other pairs'
/// routes end at A or B).
fn held(net: &Network, r: &Run) -> BTreeSet<&'static str> {
    let last: Vec<LinkId> = (0..r.trips.departure.len())
        .flat_map(|i| r.set(TripId::from_index(i)).iter())
        .filter(|a| a.mode == Mode::Car)
        .filter_map(|a| a.route().and_then(|links| links.last().copied()))
        .collect();
    ["ad", "bd", "cd"].into_iter().filter(|id| last.contains(&link(net, id))).collect()
}

#[test]
fn the_pair_holds_at_most_the_limit_and_without_it_all_three_routes() {
    let net = three_routes();
    let all = run_with(&net, 100);
    assert_eq!(held(&net, &all), BTreeSet::from(["ad", "bd", "cd"]), "the case finds all three");
    assert_eq!(held(&net, &run_with(&net, 1)), BTreeSet::from(["ad"]), "1: the free-flow route");
    assert_eq!(held(&net, &run_with(&net, 2)), BTreeSet::from(["ad", "bd"]), "2: C never offered");
}

#[test]
fn a_limit_the_pair_never_reaches_changes_nothing() {
    let net = three_routes();
    let (three, many) = (run_with(&net, 3), run_with(&net, u32::MAX));
    assert_eq!(three.trips.arrival, many.trips.arrival);
    assert_eq!(three.trips.cost, many.trips.cost);
    assert_eq!(three.trips.mode, many.trips.mode);
    assert_eq!(three.iterations.len(), many.iterations.len());
}

/// One pair by two routes of 2 000 m at 50 km/h, A = 1 003.6 + 996.4 m and B = 1 027.6 + 972.4
/// m, no queue: equal in exact arithmetic, but their free-flow times, summed from each departure
/// second in floating point, differ in the last place, one way for some seconds and the other
/// way for others (checked by hand in Python: A first at 185 of the 3 600 seconds, B at 58), so
/// at iteration 0 the pair's travellers find both routes.
fn two_tied_routes() -> Network {
    network(
        &[("O", 0.0, 0.0), ("A", 1_000.0, 50.0), ("B", 1_000.0, -50.0), ("D", 2_000.0, 0.0)],
        &[
            ("oa", "O", "A", car_link(1_003.6, 50.0, 36_000.0)),
            ("ad", "A", "D", car_link(996.4, 50.0, 36_000.0)),
            ("ob", "O", "B", car_link(1_027.6, 50.0, 36_000.0)),
            ("bd", "B", "D", car_link(2_000.0 - 1_027.6, 50.0, 36_000.0)),
        ],
    )
}

#[test]
fn a_travellers_first_car_route_is_never_refused_so_no_car_driver_is_left_without_one() {
    let net = two_tied_routes();
    let rows = stream(&net, "t", "O", "D", 3_600.0, 1.0, Second(0));
    let dem = demand(&net, &rows);
    let run_at = |limit: u32| {
        let p = Parameters { max_car_routes: limit, ..time_only(1) };
        run(&net, &dem, None, (Second(0), Second(7_200)), &p).expect("a run")
    };
    let first_links = |r: &Run| -> BTreeSet<LinkId> {
        (0..r.trips.departure.len())
            .flat_map(|i| r.set(TripId::from_index(i)).iter())
            .filter_map(|a| a.route().and_then(|links| links.first().copied()))
            .collect()
    };
    let free = run_at(u32::MAX);
    assert_eq!(first_links(&free).len(), 2, "the case: both routes found at iteration 0");
    let one = run_at(1);
    let without = (0..one.trips.departure.len())
        .filter(|&i| !one.set(TripId::from_index(i)).iter().any(|a| a.mode == Mode::Car))
        .count();
    assert_eq!(without, 0, "every car driver holds a car route");
    assert!(one.trips.finished.iter().all(|&f| f), "and every trip finishes");
    assert_eq!(first_links(&one).len(), 2, "both first routes kept: the pair above its limit");
}
