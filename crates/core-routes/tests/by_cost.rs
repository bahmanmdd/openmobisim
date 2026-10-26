//! A method that searches by time, made to search by the run's route cost (S299).
//!
//! What is defended, and how each is checked, on a two-road diamond by hand:
//!
//! * **one time-only vector is the method itself**: the same routes, costs and overlaps;
//! * **an extra on the shorter road** (a toll, say) makes the longer one the least-cost route
//!   of the penalty, shortest and Monte Carlo methods, first in the set, with its travel time as
//!   its cost; the detour limit is judged in the generalised cost;
//! * **two vectors** give a set holding each vector's least-cost route, ordered by the first's;
//! * **the descriptor** names the cost, the name stays the method's.

use std::sync::Arc;

use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::network::{LinkSpec, RoadNetwork, RoadNetworkBuilder};
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_routes::{
    ByCost, MonteCarlo, Options, Penalty, RouteKey, RouteSetGenerator, RouteSets, Shortest,
};
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::{EntityId, LinkId, NodeId};

/// A, B, C, D: the top road A-B-D is 200 m, the bottom road A-C-D is 240 m; one-way.
fn diamond() -> RoadNetwork {
    let mut b = RoadNetworkBuilder::new();
    for (n, x, y) in [("A", 0.0, 0.0), ("B", 100.0, 60.0), ("C", 100.0, -60.0), ("D", 200.0, 0.0)] {
        b.add_node(n, LonLat::new(4.8 + x / 77_800.0, 45.7 + y / 110_574.0));
    }
    for (name, from, to, len) in [
        ("ab", "A", "B", 100.0),
        ("bd", "B", "D", 100.0),
        ("ac", "A", "C", 120.0),
        ("cd", "C", "D", 120.0),
    ] {
        let mut spec = LinkSpec::new(RoadClass::Residential);
        spec.length_m = Some(len);
        b.add_link(name, from, to, spec);
    }
    b.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
        .expect("buildable")
}

fn link(net: &RoadNetwork, name: &str) -> LinkId {
    net.link_external_ids().typed_id_of::<LinkId>(name).expect("a link")
}

fn key(net: &RoadNetwork) -> RouteKey {
    let node = |n: &str| net.node_external_ids().typed_id_of::<NodeId>(n).expect("a node");
    RouteKey::new(node("A"), node("D"))
}

/// Each route of the one key: (links, cost, overlap).
fn routes(net: &RoadNetwork, g: &dyn RouteSetGenerator) -> Vec<(Vec<u32>, f32, f32)> {
    let turns = TurnTable::build(net, SignalDefaults::SHIPPED);
    let sets = RouteSets::generate(net, &turns, &[key(net)], g);
    sets.routes(0).map(|r| (r.links.to_vec(), r.cost, r.overlap)).collect()
}

/// `seconds` on link `on`, 0 elsewhere.
fn extra(net: &RoadNetwork, on: &str, seconds: f64) -> Arc<[f64]> {
    let mut e = vec![0.0; net.link_count() as usize];
    e[link(net, on).index()] = seconds;
    e.into()
}

fn methods() -> Vec<(&'static str, Arc<dyn RouteSetGenerator>)> {
    let penalty = Penalty::from_options(&Options::new()).expect("valid");
    let montecarlo = MonteCarlo::from_options(&Options::new()).expect("valid");
    vec![
        ("penalty", Arc::new(penalty)),
        ("shortest", Arc::new(Shortest)),
        ("montecarlo", Arc::new(montecarlo)),
    ]
}

#[test]
fn one_time_only_vector_is_the_method_itself() {
    let net = diamond();
    for (label, method) in methods() {
        let by = ByCost::new(method.clone(), vec![None]);
        assert_eq!(routes(&net, &by), routes(&net, method.as_ref()), "{label}");
        assert_eq!(by.name(), method.name());
    }
}

#[test]
fn an_extra_on_the_shorter_road_makes_the_longer_one_the_least_cost_route() {
    let net = diamond();
    let id = |n: &str| link(&net, n).raw();
    let time = |n: &str| net.free_flow_time(link(&net, n)).get();
    let bottom_time = time("ac") + time("cd");
    for (label, method) in methods() {
        let plain = routes(&net, method.as_ref());
        assert_eq!(plain[0].0, vec![id("ab"), id("bd")], "{label}: by time, the top road");
        let by = ByCost::new(method.clone(), vec![Some(extra(&net, "ab", 60.0))]);
        let set = routes(&net, &by);
        assert_eq!(set[0].0, vec![id("ac"), id("cd")], "{label}: by the cost, the bottom road");
        assert!((f64::from(set[0].1) - bottom_time).abs() < 1e-3, "{label}: its travel time");
        assert!(by.descriptor().starts_with(&method.descriptor()), "{label}");
        assert!(by.descriptor().contains(";cost=given:"), "{label}");
    }
    // The detour is judged in the generalised cost: with the toll the top road costs 60 s more
    // than its time, past 1.3 times the bottom's cost, so the penalty method keeps only the
    // bottom; without it, both roads are within 1.3 of the top's time and both are kept.
    let penalty: Arc<dyn RouteSetGenerator> =
        Arc::new(Penalty::from_options(&Options::new()).expect("valid"));
    assert_eq!(routes(&net, penalty.as_ref()).len(), 2);
    let tolled = ByCost::new(penalty, vec![Some(extra(&net, "ab", 60.0))]);
    assert_eq!(routes(&net, &tolled).len(), 1, "the tolled top road is past the detour");
}

#[test]
fn two_vectors_give_each_one_s_least_cost_route_ordered_by_the_first() {
    let net = diamond();
    let id = |n: &str| link(&net, n).raw();
    let shortest: Arc<dyn RouteSetGenerator> = Arc::new(Shortest);
    let both = ByCost::new(shortest.clone(), vec![Some(extra(&net, "ab", 60.0)), None]);
    let set = routes(&net, &both);
    assert_eq!(
        set.iter().map(|r| r.0.clone()).collect::<Vec<_>>(),
        vec![vec![id("ac"), id("cd")], vec![id("ab"), id("bd")]],
        "the first vector's (the bottom road) first, then the time-only vector's"
    );
    let swapped = ByCost::new(shortest, vec![None, Some(extra(&net, "ab", 60.0))]);
    assert_eq!(routes(&net, &swapped)[0].0, vec![id("ab"), id("bd")], "ordered by the first");
    assert_ne!(both.descriptor(), swapped.descriptor());
}
