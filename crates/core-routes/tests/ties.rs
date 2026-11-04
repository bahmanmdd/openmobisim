//! Routes of exactly equal cost: which one a search keeps must not depend on the ids of links
//! and nodes.
//!
//! The tie rule (the search module's documentation): of routes of exactly the same cost, the
//! shorter in metres; at the same length, the one of fewer links; at that too, the one whose
//! nodes' coordinates come first, read from the downstream end of the route back (the upstream
//! node of the last link first, its east coordinate, then its north). Each case below is built
//! twice, the second time with the link names permuted so that the links' ids, which follow the
//! sorted names, come in another order; every search must keep the same route, by its nodes,
//! and the one the rule names. Costs are given exactly ([`SearchContext::with_costs`]), so the
//! ties are exact.

use std::collections::HashMap;

use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::network::{LinkSpec, RoadNetwork, RoadNetworkBuilder};
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_routes::search::{Search, SearchContext};
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::{EntityId, LinkId, NodeId};

/// A link: its ends, its length (m) and its cost (s).
struct Def {
    from: &'static str,
    to: &'static str,
    metres: f64,
    seconds: f64,
}

/// The network of `nodes` (name, metres east, metres north) and `links`, the k-th link named
/// `names[k]`, with each link's cost as given.
fn build(
    nodes: &[(&str, f64, f64)],
    links: &[Def],
    names: &[&str],
) -> (RoadNetwork, TurnTable, Vec<f64>) {
    let mut b = RoadNetworkBuilder::new();
    for &(n, x, y) in nodes {
        b.add_node(n, LonLat::new(4.8 + x / 77_800.0, 45.7 + y / 110_574.0));
    }
    for (l, name) in links.iter().zip(names) {
        let mut spec = LinkSpec::new(RoadClass::Secondary);
        spec.length_m = Some(l.metres);
        b.add_link(*name, l.from, l.to, spec);
    }
    let net = b
        .build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
        .expect("buildable");
    let turns = TurnTable::build(&net, net.signals());
    let by_name: HashMap<&str, f64> =
        names.iter().copied().zip(links.iter().map(|l| l.seconds)).collect();
    let cost = (0..net.link_count())
        .map(|i| {
            let name = net.link_external_ids().external_of(LinkId::new(i)).expect("named");
            by_name[name]
        })
        .collect();
    (net, turns, cost)
}

fn node(net: &RoadNetwork, name: &str) -> NodeId {
    net.node_external_ids().typed_id_of(name).expect("a node of the case")
}

/// A route by the names of its nodes, from its origin to its destination.
fn nodes_of(net: &RoadNetwork, links: &[LinkId]) -> Vec<String> {
    let name = |n: NodeId| net.node_external_ids().external_of(n).expect("named").to_string();
    let mut out = vec![name(net.link_from(links[0]))];
    out.extend(links.iter().map(|&l| name(net.link_to(l))));
    out
}

/// The route from `O` to `D` each search keeps: the goal-directed one, the tree, the
/// time-dependent least-cost one (what the assignment's car search calls) and the
/// earliest-arrival one, every link's time its cost.
fn routes(net: &RoadNetwork, turns: &TurnTable, cost: &[f64]) -> Vec<Vec<String>> {
    let ctx = SearchContext::with_costs(net, turns, cost.to_vec());
    let mut search = Search::new(&ctx);
    let (o, d) = (node(net, "O"), node(net, "D"));
    let seconds = |l: u32, _at: f64| cost[l as usize];
    let wait = |_l: u32, _at: f64| 0.0;
    let mut targets = vec![false; net.node_count() as usize];
    targets[d.index()] = true;
    let shortest = search.shortest(o, d).expect("a route").links;
    let tree = search.shortest_tree(o, &[d], f64::INFINITY).remove(0).expect("a route").links;
    let cheapest = search
        .cheapest_routes_to(o, &targets, 1, 25_200.0, f64::INFINITY, &wait, &seconds)
        .remove(0)
        .3;
    let fastest =
        search.fastest_route(o, d, 25_200.0, f64::INFINITY, &wait, &seconds).expect("a route").1;
    [shortest, tree, cheapest, fastest].iter().map(|r| nodes_of(net, r)).collect()
}

/// Each search keeps `want` with the links named as built and permuted.
fn check(nodes: &[(&str, f64, f64)], links: &[Def], permuted: &[&str], want: &[&str]) {
    let names: Vec<&str> = (0..links.len()).map(|k| ["l0", "l1", "l2", "l3", "l4"][k]).collect();
    for names in [names.as_slice(), permuted] {
        let (net, turns, cost) = build(nodes, links, names);
        for (search, route) in
            ["shortest", "tree", "cheapest", "fastest"].iter().zip(routes(&net, &turns, &cost))
        {
            assert_eq!(route, want, "{search}, links named {names:?}");
        }
    }
}

/// A square of four equal sides, 316 m and 20 s each: the two routes from `O` to `D` tie in
/// cost, length and links. The rule reads the upstream node of each route's last link: `A` at
/// 100 m east against `B` at 300 m, so the route through `A`.
#[test]
fn equal_routes_by_the_nodes_coordinates_whatever_the_ids() {
    let nodes = [("O", 0.0, 0.0), ("A", 100.0, 300.0), ("B", 300.0, 100.0), ("D", 400.0, 400.0)];
    let side = 316.227_766;
    let links = [
        Def { from: "O", to: "A", metres: side, seconds: 20.0 },
        Def { from: "A", to: "D", metres: side, seconds: 20.0 },
        Def { from: "O", to: "B", metres: side, seconds: 20.0 },
        Def { from: "B", to: "D", metres: side, seconds: 20.0 },
    ];
    check(&nodes, &links, &["l3", "l2", "l1", "l0"], &["O", "A", "D"]);
    check(&nodes, &links, &["l2", "l3", "l0", "l1"], &["O", "A", "D"]);
}

/// The same square with the sides through `B` 300 m long: the costs still tie, and the
/// shorter route, through `B`, is kept although the coordinates would name `A`.
#[test]
fn equal_cost_routes_by_length_first() {
    let nodes = [("O", 0.0, 0.0), ("A", 100.0, 300.0), ("B", 300.0, 100.0), ("D", 400.0, 400.0)];
    let links = [
        Def { from: "O", to: "A", metres: 316.227_766, seconds: 20.0 },
        Def { from: "A", to: "D", metres: 316.227_766, seconds: 20.0 },
        Def { from: "O", to: "B", metres: 300.0, seconds: 20.0 },
        Def { from: "B", to: "D", metres: 300.0, seconds: 20.0 },
    ];
    check(&nodes, &links, &["l3", "l2", "l1", "l0"], &["O", "B", "D"]);
}

/// Two links of 300 m through `A` (20 s each) against three of 200 m through `B` and `C`
/// (10, 10 and 20 s): the same cost, 40 s, and the same length, 600 m; the route of fewer
/// links, through `A`, is kept although the coordinates would name the other (`C`, the
/// upstream node of its last link, lies 150 m east against `A`'s 300 m).
#[test]
fn equal_cost_and_length_by_fewer_links() {
    let nodes = [
        ("O", 0.0, 0.0),
        ("A", 300.0, 100.0),
        ("B", 100.0, 200.0),
        ("C", 150.0, 350.0),
        ("D", 400.0, 400.0),
    ];
    let links = [
        Def { from: "O", to: "A", metres: 300.0, seconds: 20.0 },
        Def { from: "A", to: "D", metres: 300.0, seconds: 20.0 },
        Def { from: "O", to: "B", metres: 200.0, seconds: 10.0 },
        Def { from: "B", to: "C", metres: 200.0, seconds: 10.0 },
        Def { from: "C", to: "D", metres: 200.0, seconds: 20.0 },
    ];
    check(&nodes, &links, &["l4", "l3", "l2", "l1", "l0"], &["O", "A", "D"]);
}
