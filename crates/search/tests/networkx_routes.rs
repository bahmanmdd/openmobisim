//! Test 9: the search's routes equal networkx's shortest paths by length (Hagberg, Schult and
//! Swart 2008, networkx 3.7), for bikes, walkers and free-flow cars on one speed.
//!
//! The fixture `fixtures/networkx_routes.txt` is written by `networkx_routes.py` (beside this
//! file, not run in CI): a 12 × 12 grid of nodes 100 m apart, each moved by up to 20 m in x and
//! y, a link each way between neighbours whose length is the straight line rounded to 0.01 m,
//! and 20 origin-destination pairs at least 4 rows and 4 columns apart, each with exactly one
//! shortest path by length (checked by networkx), with that path and its length. A bike's time
//! on a link is its length over 15 km/h, a walker's over 4.8 km/h, and a car's at free flow
//! over the one car speed, 30 km/h: on every mode the fastest route is the shortest.

mod common;

use common::{ALL_LINK_MODES, VALUES, at, free_flow};
use openmobisim_graph::network::{LinkRow, Network, NetworkBuilder};
use openmobisim_search::search::{Query, Route, Search};
use openmobisim_types::ids::{EntityId, NodeId};
use openmobisim_types::mode::Mode;
use openmobisim_types::units::Duration;

const FIXTURE: &str = include_str!("fixtures/networkx_routes.txt");

/// The departure of every query, in seconds since the window's start (which is second 0).
const DEPARTURE: u32 = 600;

/// One pair of the fixture: origin, destination, networkx's length in metres and node sequence.
struct Pair {
    origin: String,
    destination: String,
    metres: f64,
    nodes: Vec<String>,
}

/// The fixture's network (node ids as written, link ids `l{id}`; every link all modes, one lane,
/// 30 km/h, its length as written) and its pairs.
fn read_fixture() -> (Network, Vec<Pair>) {
    let mut b = NetworkBuilder::new();
    let mut pairs = Vec::new();
    for line in FIXTURE.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        let num = |i: usize| f[i].parse::<f64>().expect("a number");
        match f[0] {
            "node" => b.add_node(f[1], at(num(2), num(3))),
            "link" => {
                let row = LinkRow {
                    modes: ALL_LINK_MODES,
                    lanes: Some(1),
                    car_speed_km_h: Some(30.0),
                    length_m: Some(num(4)),
                    capacity_veh_h: None,
                    signal_at_downstream_end: false,
                };
                b.add_link(format!("l{}", f[1]), f[2], f[3], row);
            }
            "pair" => pairs.push(Pair {
                origin: f[1].to_owned(),
                destination: f[2].to_owned(),
                metres: num(3),
                nodes: f[4..].iter().map(|&s| s.to_owned()).collect(),
            }),
            other => panic!("unknown record {other}"),
        }
    }
    (b.build(VALUES).expect("the fixture builds"), pairs)
}

/// The external ids of the nodes a route passes, from its origin.
fn nodes_of(net: &Network, origin: NodeId, route: &Route) -> Vec<String> {
    let mut nodes = vec![origin];
    nodes.extend(route.links.iter().map(|&l| net.downstream(l)));
    nodes.iter().map(|&n| net.node_ids().external(n.raw()).to_owned()).collect()
}

#[test]
fn bike_walk_and_free_flow_car_routes_are_networkx_shortest_paths() {
    let (net, pairs) = read_fixture();
    assert_eq!((net.node_count(), net.link_count(), pairs.len()), (144, 528, 20));
    let times = free_flow(&net);
    let mut search = Search::new(&net, &times);
    let id = |s: &str| net.node_ids().typed_id_of::<NodeId>(s).expect("a fixture node");
    for p in &pairs {
        let (origin, destination) = (id(&p.origin), id(&p.destination));
        let departure = Duration(f64::from(DEPARTURE));
        let ask = |mode| Query { origin, destination, mode, departure, trip: None };
        let bike = search.fastest(&ask(Mode::Bike)).expect("the grid is connected");
        let walk = search.fastest(&ask(Mode::Walk)).expect("the grid is connected");
        let car = search.fastest(&ask(Mode::Car)).expect("the grid is connected");
        let pair = format!("pair {} to {}", p.origin, p.destination);
        assert_eq!(nodes_of(&net, origin, &bike), p.nodes, "{pair}: the bike's nodes");
        assert_eq!(walk.links, bike.links, "{pair}: the walker's links");
        assert_eq!(car.links, bike.links, "{pair}: the free-flow car's links");
        // Arrival = departure + length / speed, the speed in m/s being km/h / 3.6.
        for (route, km_h, mode) in
            [(&bike, 15.0, "bike"), (&walk, 4.8, "walk"), (&car, 30.0, "car")]
        {
            let by_hand = f64::from(DEPARTURE) + p.metres / (km_h / 3.6);
            let arrival = route.arrival.get();
            assert!(
                (arrival - by_hand).abs() <= 1e-9 * by_hand,
                "{pair}: the {mode} arrives at {arrival} s, networkx's length gives {by_hand} s"
            );
        }
    }
}
