//! The tree form: every node within a bound, forward (`reach`) and backward (`reach_backward`),
//! against networkx's bounded Dijkstra (Hagberg, Schult and Swart 2008, networkx 3.7), and the
//! property that `reach` arrives at each node when `fastest` does.
//!
//! The fixture `fixtures/networkx_reach.txt` is written by `networkx_routes.py reach` (beside
//! this file, not run in CI): a 10 × 10 grid of nodes 100 m apart, each moved by up to 20 m in x
//! and y, a link each way between neighbours whose length is the straight line rounded to
//! 0.01 m; on 30 of the 360 links walking is not allowed, so those streets are one-way on foot.
//! For 8 origins, networkx's `single_source_dijkstra_path_length` by length with a cutoff of
//! 450 m on foot and 600 m by bike, and on foot on the reversed graph. No node lies within
//! 0.01 m of a cutoff (checked by the script), so the sets are the same whichever way the sums
//! round. A walker's time is length over 4.8 km/h, a bike's over 15 km/h.

mod common;

use common::{
    Draws, VALUES, at, free_flow, grid_node, grid_with, l_route_cars, loaded, row, window,
};
use openmobisim_graph::network::{LinkRow, Network, NetworkBuilder};
use openmobisim_search::search::{Query, Search};
use openmobisim_types::ids::{EntityId, NodeId};
use openmobisim_types::mode::{Mode, ModeSet};
use openmobisim_types::time::Second;
use openmobisim_types::units::Duration;

const FIXTURE: &str = include_str!("fixtures/networkx_reach.txt");

/// The departure of every forward reach, in seconds since the window's start (second 0).
const DEPARTURE: u32 = 600;

/// One reach of the fixture: forward or backward, from or to `node`, the mode, the cutoff in
/// metres, and networkx's distance in metres of every node within it, by external id.
struct Reach {
    forward: bool,
    node: String,
    mode: Mode,
    cutoff: f64,
    metres: Vec<(String, f64)>,
}

/// The fixture's network (node ids as written, link ids `l{id}`; every link car and bike at
/// one lane and 30 km/h, walk where the fixture says so, its length as written) and its reaches.
fn read_fixture() -> (Network, Vec<Reach>) {
    let mut b = NetworkBuilder::new();
    let mut reaches = Vec::new();
    for line in FIXTURE.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        let num = |s: &str| s.parse::<f64>().expect("a number");
        match f[0] {
            "node" => b.add_node(f[1], at(num(f[2]), num(f[3]))),
            "link" => {
                let modes = match f[5] {
                    "cbw" => ModeSet::of(&[Mode::Car, Mode::Bike, Mode::Walk]),
                    "cb" => ModeSet::of(&[Mode::Car, Mode::Bike]),
                    other => panic!("unknown modes {other}"),
                };
                let row = LinkRow { modes, length_m: Some(num(f[4])), ..row(1, 30.0) };
                b.add_link(format!("l{}", f[1]), f[2], f[3], row);
            }
            "reach" | "back" => reaches.push(Reach {
                forward: f[0] == "reach",
                node: f[1].to_owned(),
                mode: if f[2] == "walk" { Mode::Walk } else { Mode::Bike },
                cutoff: num(f[3]),
                metres: f[4..]
                    .iter()
                    .map(|s| {
                        let (n, m) = s.split_once(':').expect("node:metres");
                        (n.to_owned(), num(m))
                    })
                    .collect(),
            }),
            other => panic!("unknown record {other}"),
        }
    }
    (b.build(VALUES).expect("the fixture builds"), reaches)
}

#[test]
fn reaches_forward_and_backward_are_networkx_bounded_dijkstra() {
    let (net, reaches) = read_fixture();
    assert_eq!((net.node_count(), net.link_count(), reaches.len()), (100, 360, 24));
    let times = free_flow(&net);
    let mut search = Search::new(&net, &times);
    let mut differ = 0;
    for r in &reaches {
        let node = net.node_ids().typed_id_of::<NodeId>(&r.node).expect("a fixture node");
        let speed = if r.mode == Mode::Walk { 4.8 / 3.6 } else { 15.0 / 3.6 };
        let bound = Duration(r.cutoff / speed);
        let (got, start) = if r.forward {
            (search.reach(node, r.mode, Second(DEPARTURE), bound), f64::from(DEPARTURE))
        } else {
            (search.reach_backward(node, r.mode, bound), 0.0)
        };
        let what = format!("{} {} {}", if r.forward { "reach" } else { "back" }, r.node, r.mode);
        let names: Vec<&str> = got.iter().map(|(n, _)| net.node_ids().external(n.raw())).collect();
        let mut expected: Vec<&str> = r.metres.iter().map(|(n, _)| n.as_str()).collect();
        expected.sort_unstable();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, expected, "{what}: the nodes within the cutoff");
        assert!(got.windows(2).all(|w| w[0].0 < w[1].0), "{what}: sorted by node id");
        for (name, metres) in &r.metres {
            let at = got[names.iter().position(|n| n == name).expect("found")].1.get();
            let by_hand = start + metres / speed;
            assert!(
                (at - by_hand).abs() <= 1e-9 * by_hand.max(1.0),
                "{what}: node {name} at {at} s, networkx's {metres} m give {by_hand} s"
            );
        }
        if !r.forward {
            let ahead = search.reach(node, r.mode, Second(0), bound);
            differ += usize::from(ahead.iter().map(|x| x.0).ne(got.iter().map(|x| x.0)));
        }
    }
    // The one-way walk links must matter, or the backward test is the forward one again.
    assert!(differ >= 3, "forward and backward differ for only {differ} of 8 origins");
}

#[test]
fn reach_arrives_at_every_node_when_fastest_does_on_a_loaded_grid() {
    // 20 × 20 grid of 100 m links at 50 km/h and 200 veh/h, 6 000 cars over 30 min (the
    // determinism test's loading): queues form, the times depend on the instant.
    let n = 20;
    let net = grid_with(n, |_, _| LinkRow { capacity_veh_h: Some(200.0), ..row(1, 50.0) });
    let mut draws = Draws::new(7);
    let cars = l_route_cars(&net, n, 6_000, 1_800, &mut draws);
    let times = loaded(&net, &cars, window(300, 7_200));
    let mut search = Search::new(&net, &times);
    let mut compared = 0;
    for i in 0..20 {
        let origin = grid_node(&net, draws.below(n), draws.below(n));
        let departure = Second(draws.below(3_600));
        let mode = if i % 4 == 0 { Mode::Bike } else { Mode::Car };
        let reached = search.reach(origin, mode, departure, Duration(240.0));
        assert!(reached.iter().any(|&(node, _)| node == origin), "the origin is reached");
        for &(node, arrival) in &reached {
            let departure = Duration(f64::from(departure.get()));
            let q = Query { origin, destination: node, mode, departure, trip: None };
            let route = search.fastest(&q).expect("reached, so reachable");
            assert_eq!(
                arrival.get().to_bits(),
                route.arrival.get().to_bits(),
                "from {origin} to {node} by {mode} at {departure}: reach {arrival}, fastest {}",
                route.arrival
            );
            compared += 1;
        }
    }
    assert!(compared >= 1_000, "only {compared} nodes compared");
}

#[test]
fn a_node_just_beyond_the_bound_is_not_reached() {
    // A 3 × 3 grid at free flow: a car crosses a 100 m link at 50 km/h in 7.2 s. Within 7.2 s
    // of (1, 1): itself and its four neighbours; within 7.1 s: itself alone.
    let net = grid_with(3, |_, _| row(1, 50.0));
    let times = free_flow(&net);
    let mut search = Search::new(&net, &times);
    let centre = grid_node(&net, 1, 1);
    let mut near: Vec<NodeId> = [(0, 1), (1, 0), (1, 1), (1, 2), (2, 1)]
        .iter()
        .map(|&(r, c)| grid_node(&net, r, c))
        .collect();
    near.sort_unstable();
    let five = search.reach(centre, Mode::Car, Second(0), Duration(100.0 / (50.0 / 3.6)));
    assert_eq!(five.iter().map(|x| x.0).collect::<Vec<_>>(), near);
    let one = search.reach(centre, Mode::Car, Second(0), Duration(7.1));
    assert_eq!(one, vec![(centre, Duration(0.0))]);
}

#[test]
#[should_panic(expected = "bikes and walkers only")]
fn a_backward_car_reach_is_refused() {
    let net = grid_with(2, |_, _| row(1, 50.0));
    let times = free_flow(&net);
    let _ =
        Search::new(&net, &times).reach_backward(grid_node(&net, 0, 0), Mode::Car, Duration(60.0));
}

#[test]
#[should_panic(expected = "transit is not on links")]
fn a_transit_reach_is_refused() {
    let net = grid_with(2, |_, _| row(1, 50.0));
    let times = free_flow(&net);
    let _ = Search::new(&net, &times).reach(
        grid_node(&net, 0, 0),
        Mode::Transit,
        Second(0),
        Duration(60.0),
    );
}
