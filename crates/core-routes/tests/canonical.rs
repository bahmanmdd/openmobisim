//! A route set must not depend on the ids of links and nodes (S312): the order of its routes,
//! which twin a set takes, the random draws of the Monte Carlo method and a route's identity.
//!
//! Each case is built twice, the second time with the link and node names permuted so that the
//! ids, which follow the sorted names, come in another order; every set must hold the same
//! routes, by their nodes, in the same order, and the one the rule names (the module
//! `canonical`: the cheapest; among equals the shorter, then the one of fewer links, then the
//! one whose nodes' coordinates come first, read from the downstream end back). Costs are given
//! exactly ([`SearchContext::with_costs`]), so the ties are exact.

use std::collections::HashMap;

use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::network::{LinkSpec, RoadNetwork, RoadNetworkBuilder};
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_routes::search::{Search, SearchContext};
use openmobisim_core_routes::{LinkKeys, MonteCarlo, Penalty, RouteSetGenerator, Shortest};
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
/// `names[k]`, every node `n` named `prefix` + `n`, with each link's cost as given.
fn build(
    nodes: &[(&str, f64, f64)],
    links: &[Def],
    names: &[String],
    prefix: &str,
) -> (RoadNetwork, TurnTable, Vec<f64>) {
    let mut b = RoadNetworkBuilder::new();
    for &(n, x, y) in nodes {
        b.add_node(format!("{prefix}{n}"), LonLat::new(4.8 + x / 77_800.0, 45.7 + y / 110_574.0));
    }
    for (l, name) in links.iter().zip(names) {
        let mut spec = LinkSpec::new(RoadClass::Secondary);
        spec.length_m = Some(l.metres);
        b.add_link(name.clone(), format!("{prefix}{}", l.from), format!("{prefix}{}", l.to), spec);
    }
    let net = b
        .build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
        .expect("buildable");
    let turns = TurnTable::build(&net, net.signals());
    let by_name: HashMap<&str, f64> =
        names.iter().map(String::as_str).zip(links.iter().map(|l| l.seconds)).collect();
    let cost = (0..net.link_count())
        .map(|i| by_name[net.link_external_ids().external_of(LinkId::new(i)).expect("named")])
        .collect();
    (net, turns, cost)
}

/// The two namings: links `l0, l1, …` and nodes as given, then the link names reversed and
/// every node name prefixed so that the nodes sort in another order too.
fn namings(links: usize) -> [(Vec<String>, &'static str); 2] {
    let names: Vec<String> = (0..links).map(|k| format!("l{k}")).collect();
    let reversed: Vec<String> = names.iter().rev().cloned().collect();
    [(names, ""), (reversed, "z")]
}

fn node(net: &RoadNetwork, prefix: &str, name: &str) -> NodeId {
    net.node_external_ids().typed_id_of(&format!("{prefix}{name}")).expect("a node of the case")
}

/// A route by the names of its nodes (the naming's prefix taken off), origin to destination.
fn nodes_of(net: &RoadNetwork, prefix: &str, links: &[LinkId]) -> String {
    let name = |n: NodeId| {
        let full = net.node_external_ids().external_of(n).expect("named");
        full.strip_prefix(prefix).unwrap_or(full).to_string()
    };
    let mut out = vec![name(net.link_from(links[0]))];
    out.extend(links.iter().map(|&l| name(net.link_to(l))));
    out.concat()
}

/// The set `generator` makes from `O` to `D` under each naming, as node strings in the set's
/// order, and every route's identity ([`LinkKeys::route_identity`]).
fn sets(
    nodes: &[(&str, f64, f64)],
    links: &[Def],
    generator: &dyn RouteSetGenerator,
) -> Vec<(Vec<String>, Vec<u32>)> {
    namings(links.len())
        .iter()
        .map(|(names, prefix)| {
            let (net, turns, cost) = build(nodes, links, names, prefix);
            let ctx = SearchContext::with_costs(&net, &turns, cost);
            let mut search = Search::new(&ctx);
            let set =
                generator.generate(&mut search, node(&net, prefix, "O"), node(&net, prefix, "D"));
            let keys = LinkKeys::new(&net);
            (
                set.iter().map(|r| nodes_of(&net, prefix, &r.links)).collect(),
                set.iter().map(|r| keys.route_identity(&r.links)).collect(),
            )
        })
        .collect()
}

/// A square of four equal sides, 316 m and 20 s each: the two routes from `O` to `D` tie in
/// cost, length and links. The penalty method finds both; the set lists first the one whose
/// last link's upstream node lies further west, `A` at 100 m east against `B` at 300 m. Before
/// S312 the order followed the links' ids, and the reversed naming listed `OBD` first.
#[test]
fn a_set_is_ordered_by_the_tie_rule_whatever_the_ids() {
    let nodes = [("O", 0.0, 0.0), ("A", 100.0, 300.0), ("B", 300.0, 100.0), ("D", 400.0, 400.0)];
    let side = 316.227_766;
    let links = [
        Def { from: "O", to: "A", metres: side, seconds: 20.0 },
        Def { from: "A", to: "D", metres: side, seconds: 20.0 },
        Def { from: "O", to: "B", metres: side, seconds: 20.0 },
        Def { from: "B", to: "D", metres: side, seconds: 20.0 },
    ];
    for (set, ids) in sets(&nodes, &links, &Penalty::default()) {
        assert_eq!(set, ["OAD", "OBD"]);
        assert_eq!(ids, sets(&nodes, &links, &Penalty::default())[0].1, "the same identities");
    }
}

/// Three ways into `D`: through `A` (40 s, the least-cost route), through `B` (41 s) and through
/// `C` (40.5 s), all within the twins' tolerance. The set takes one twin, and at one node the
/// cheapest: `OCD`. Before S312 the twin was the first link into `D` by id, so `OBD` under one
/// of the two namings.
#[test]
fn the_twin_taken_at_a_node_is_the_cheapest_whatever_the_ids() {
    let nodes = [
        ("O", 0.0, 0.0),
        ("A", 100.0, 300.0),
        ("B", 300.0, 100.0),
        ("C", 200.0, 200.0),
        ("D", 400.0, 400.0),
    ];
    let links = [
        Def { from: "O", to: "A", metres: 316.0, seconds: 20.0 },
        Def { from: "A", to: "D", metres: 316.0, seconds: 20.0 },
        Def { from: "O", to: "B", metres: 316.0, seconds: 20.0 },
        Def { from: "B", to: "D", metres: 316.0, seconds: 21.0 },
        Def { from: "O", to: "C", metres: 283.0, seconds: 20.0 },
        Def { from: "C", to: "D", metres: 283.0, seconds: 20.5 },
    ];
    let shortest = Shortest { max_paths: 2, twin_tolerance: 0.1 };
    for (set, _) in sets(&nodes, &links, &shortest) {
        assert_eq!(set, ["OAD", "OCD"]);
    }
}

/// A grid of 4 × 4 nodes, 200 m apart, every link 20 s: many routes of equal cost from one
/// corner to the other. The Monte Carlo method's draws perturb each link's cost by a factor that
/// is a function of the link's key and of the pair's nodes' keys, never of their ids: the same
/// routes in the same order under both namings. Before S312 the factors followed the ids.
#[test]
fn the_monte_carlo_draws_follow_the_streets_not_the_ids() {
    let names = ["O", "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "D"];
    let nodes: Vec<(&str, f64, f64)> = names
        .iter()
        .enumerate()
        .zip(0_u32..)
        .map(|((_, &n), i)| (n, 200.0 * f64::from(i % 4), 200.0 * f64::from(i / 4)))
        .collect();
    let mut links = Vec::new();
    for i in 0..16 {
        let (x, y) = (i % 4, i / 4);
        if x < 3 {
            links.push(Def { from: names[i], to: names[i + 1], metres: 200.0, seconds: 20.0 });
        }
        if y < 3 {
            links.push(Def { from: names[i], to: names[i + 4], metres: 200.0, seconds: 20.0 });
        }
    }
    let mut mc = MonteCarlo::default();
    mc.max_paths = 6;
    mc.max_overlap = 0.95;
    let both = sets(&nodes, &links, &mc);
    assert!(both[0].0.len() >= 3, "the draws find several routes: {:?}", both[0].0);
    assert_eq!(both[0], both[1]);
}
