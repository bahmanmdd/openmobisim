//! A placeholder shortest-path function (S133) — **not** the Phase 2
//! route-set system.
//!
//! Single alternative, computed fresh every call, no caching, no k-route
//! sequences, no ALT bounds. It exists for one reason: `core-loading`'s flow
//! motor never computes a route, only consumes one (design §10.4), and
//! Phase 2 item 6's real route-set system does not exist yet — so a real run
//! needs *something* to turn a trip's origin and destination into a
//! `Vec<LinkId>` in the meantime. When Phase 2 item 6 lands, this module is
//! deleted wholesale, not migrated (S133 records why it was written this way
//! rather than as a first cut of the real system).

use core::cmp::Ordering;
use std::collections::BinaryHeap;

use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_types::ids::{EntityId, LinkId, NodeId};

/// The network node closest to `point` that a car can use, by straight-line
/// distance in the network's own projection: a node with at least one link
/// that carries motor traffic (S153 — footways and cycleways are not car
/// origins). A network with no such link falls back to the closest node of
/// any kind.
///
/// Brute-force over every node — a placeholder for the real access/egress
/// correction S96 describes (the walk layer, corrected per traveller
/// location, with a spatial index). Fine for the small networks Phase 1
/// runs against; not something to run against a city-scale extract.
///
/// # Panics
///
/// Panics if `network` has no nodes. `RoadNetworkBuilder::build` already
/// refuses to produce an empty network, so this is never reached in
/// practice — the panic exists so an empty network fails loudly here too,
/// rather than returning a meaningless node.
#[must_use]
pub fn nearest_node(network: &RoadNetwork, point: LonLat) -> NodeId {
    let target = network.projection().project(point);
    let closest = |drivable_only: bool| {
        (0..network.node_count())
            .map(NodeId::new)
            .filter(|&node| {
                !drivable_only
                    || network.out_links(node).iter().any(|&l| drivable(network, l))
                    || network.in_links(node).iter().any(|&l| drivable(network, l))
            })
            .min_by(|&a, &b| {
                let da = network.node_position(a).distance_to(target);
                let db = network.node_position(b).distance_to(target);
                da.total_cmp(&db).then_with(|| a.cmp(&b))
            })
    };
    closest(true)
        .or_else(|| closest(false))
        .expect("a built RoadNetwork always has at least one node")
}

/// Whether a car may use `link`: its class carries motor traffic.
fn drivable(network: &RoadNetwork, link: LinkId) -> bool {
    network.link_class(link).carries_motor_traffic()
}

/// The shortest path for a car from `from` to `to`, by free-flow travel time
/// (`RoadNetwork::free_flow_time`, which already includes S90's control
/// delay), over links that carry motor traffic only. `None` if no such path
/// exists. `Some(&[])` if `from == to`.
///
/// Dijkstra over the network's own adjacency (`RoadNetwork::out_links`),
/// with a deterministic tie-break on node id when two frontier entries have
/// the same cost — required by Foundations §1: nothing here may depend on
/// hash-map or heap-insertion order for its result.
#[must_use]
pub fn shortest_path(network: &RoadNetwork, from: NodeId, to: NodeId) -> Option<Vec<LinkId>> {
    if from == to {
        return Some(Vec::new());
    }

    let n = network.node_count() as usize;
    let mut best_cost = vec![f64::INFINITY; n];
    let mut predecessor = vec![LinkId::NULL; n];
    let mut frontier = BinaryHeap::new();

    best_cost[from.index()] = 0.0;
    frontier.push(Frontier { cost: 0.0, node: from });

    while let Some(Frontier { cost, node }) = frontier.pop() {
        if node == to {
            break;
        }
        if cost > best_cost[node.index()] {
            continue; // a stale entry: a cheaper way to `node` was found later
        }
        for &out_link in network.out_links(node) {
            if !drivable(network, out_link) {
                continue;
            }
            let next = network.link_to(out_link);
            let next_cost = cost + network.free_flow_time(out_link).get();
            if next_cost < best_cost[next.index()] {
                best_cost[next.index()] = next_cost;
                predecessor[next.index()] = out_link;
                frontier.push(Frontier { cost: next_cost, node: next });
            }
        }
    }

    if !best_cost[to.index()].is_finite() {
        return None;
    }

    let mut route = Vec::new();
    let mut node = to;
    while node != from {
        let link = predecessor[node.index()];
        route.push(link);
        node = network.link_from(link);
    }
    route.reverse();
    Some(route)
}

/// One entry on Dijkstra's frontier: cost-ordered, node id as a deterministic
/// tie-break, reversed so `std`'s max-heap `BinaryHeap` pops the smallest
/// cost first.
#[derive(Clone, Copy, Debug)]
struct Frontier {
    cost: f64,
    node: NodeId,
}

impl PartialEq for Frontier {
    fn eq(&self, other: &Self) -> bool {
        // Bit-pattern equality, not `==` on the floats: this is here only to
        // satisfy `Ord`'s supertrait bounds, and comparing bits sidesteps
        // any question of what float equality should mean.
        self.cost.to_bits() == other.cost.to_bits() && self.node == other.node
    }
}

impl Eq for Frontier {}

impl PartialOrd for Frontier {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Frontier {
    fn cmp(&self, other: &Self) -> Ordering {
        other.cost.total_cmp(&self.cost).then_with(|| other.node.cmp(&self.node))
    }
}
