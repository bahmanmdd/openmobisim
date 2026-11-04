//! Bounded one-to-many searches by node: every node reachable **from** a
//! source within a cost, or every node **from which** a target is within it.
//!
//! Transit uses them for walking: to the stops around an origin, from the stops
//! around a destination, and between stops. A walk has no turn rules worth
//! modelling, so the search is over nodes (a pedestrian may turn back), with a
//! cost per link — the walk layer's travel times. On a graph without turn
//! restrictions a node search and [`crate::Search`]'s link search agree, except
//! that this one may use a U-turn, which a shortest walk never needs.
//!
//! **Cost of a search:** one label per node touched, reset by visiting only
//! those; **scratch:** 28 bytes per node plus the heap.
//!
//! **Determinism, without ids (S312):** nodes reached at the same cost are listed by their
//! coordinates (west first, then south: [`crate::canonical::node_order`]); where two ways reach
//! a node at exactly the same cost, the path kept is the shorter in metres, then the one of
//! fewer links, then the one whose node next to it comes first by its coordinates, then the
//! shorter link; only parallel links of one length between one pair of nodes fall to their ids.

use core::cmp::Ordering;
use std::collections::BinaryHeap;

use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_types::ids::{EntityId, LinkId, NodeId};

const NONE: u32 = u32::MAX;

#[derive(Clone, Copy, PartialEq, Debug)]
struct Entry {
    cost: f64,
    node: u32,
}

impl Eq for Entry {}

impl PartialOrd for Entry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Entry {
    // Reversed for `BinaryHeap`: the cheapest first, then the lowest id.
    fn cmp(&self, other: &Self) -> Ordering {
        other.cost.total_cmp(&self.cost).then_with(|| other.node.cmp(&self.node))
    }
}

/// One thread's bounded search over a network with a cost per link; reusable.
#[derive(Debug)]
pub struct Reach<'a> {
    network: &'a RoadNetwork,
    costs: &'a [f64],
    dist: Vec<f64>,
    /// The link each node was reached by, for [`Self::path`].
    via: Vec<u32>,
    /// The length in metres and the number of links of the path each node was reached by.
    metres: Vec<f64>,
    hops: Vec<u32>,
    touched: Vec<u32>,
    heap: BinaryHeap<Entry>,
    backward: bool,
}

impl<'a> Reach<'a> {
    /// A search over `network`, a link costing `costs[link]` (infinite: closed).
    ///
    /// # Panics
    ///
    /// Panics if `costs` does not have one entry per link.
    #[must_use]
    pub fn new(network: &'a RoadNetwork, costs: &'a [f64]) -> Self {
        assert_eq!(costs.len(), network.link_count() as usize, "one cost per link");
        let n = network.node_count() as usize;
        Self {
            network,
            costs,
            dist: vec![f64::INFINITY; n],
            via: vec![NONE; n],
            metres: vec![0.0; n],
            hops: vec![0; n],
            touched: Vec::new(),
            heap: BinaryHeap::new(),
            backward: false,
        }
    }

    fn reset(&mut self) {
        for &n in &self.touched {
            self.dist[n as usize] = f64::INFINITY;
            self.via[n as usize] = NONE;
        }
        self.touched.clear();
        self.heap.clear();
    }

    /// Every node that `source` reaches at a cost of at most `bound`, cheapest
    /// first, among those `wanted` marks (by node index), with its cost. The
    /// search is kept until the next one, for [`Self::path`].
    pub fn forward(&mut self, source: NodeId, bound: f64, wanted: &[bool]) -> Vec<(NodeId, f64)> {
        self.run(source, bound, wanted, false)
    }

    /// Every node that reaches `target` at a cost of at most `bound`, cheapest
    /// first, among those `wanted` marks, with its cost.
    pub fn backward(&mut self, target: NodeId, bound: f64, wanted: &[bool]) -> Vec<(NodeId, f64)> {
        self.run(target, bound, wanted, true)
    }

    fn run(
        &mut self,
        start: NodeId,
        bound: f64,
        wanted: &[bool],
        backward: bool,
    ) -> Vec<(NodeId, f64)> {
        self.reset();
        self.backward = backward;
        let mut found = Vec::new();
        let s = start.raw();
        self.dist[s as usize] = 0.0;
        self.metres[s as usize] = 0.0;
        self.hops[s as usize] = 0;
        self.touched.push(s);
        self.heap.push(Entry { cost: 0.0, node: s });
        while let Some(Entry { cost, node }) = self.heap.pop() {
            if cost > self.dist[node as usize] {
                continue;
            }
            if wanted.get(node as usize).copied().unwrap_or(false) {
                found.push((NodeId::new(node), cost));
            }
            let at = NodeId::new(node);
            let links =
                if backward { self.network.in_links(at) } else { self.network.out_links(at) };
            for &link in links {
                let next = if backward {
                    self.network.link_from(link)
                } else {
                    self.network.link_to(link)
                };
                let c = cost + self.costs[link.index()];
                let n = next.index();
                if c > bound {
                    continue;
                }
                let metres = self.metres[node as usize] + self.network.link_length(link).get();
                let hops = self.hops[node as usize] + 1;
                #[allow(clippy::float_cmp, reason = "the tie rule is for exact ties of cost")]
                let tie = c == self.dist[n] && self.prefers(n, link, metres, hops);
                if c < self.dist[n] || tie {
                    if self.dist[n].is_infinite() {
                        self.touched.push(next.raw());
                    }
                    if !tie {
                        self.heap.push(Entry { cost: c, node: next.raw() });
                    }
                    self.dist[n] = c;
                    self.via[n] = link.raw();
                    self.metres[n] = metres;
                    self.hops[n] = hops;
                }
            }
        }
        // At one cost, by the nodes' coordinates rather than the heap's order of ids.
        found.sort_by(|a, b| {
            a.1.total_cmp(&b.1)
                .then_with(|| crate::canonical::node_order(self.network, a.0, b.0))
                .then(a.0.cmp(&b.0))
        });
        found
    }

    /// Whether node `n`, labelled already at the same cost, is better reached by `link` (a path
    /// of `metres` and `hops` links): the tie rule of the [module docs](self).
    #[allow(clippy::float_cmp, reason = "exact ties of length fall to the next criterion")]
    fn prefers(&self, n: usize, link: LinkId, metres: f64, hops: u32) -> bool {
        let old = self.via[n];
        if old == NONE || old == link.raw() {
            return false;
        }
        if metres != self.metres[n] {
            return metres < self.metres[n];
        }
        if hops != self.hops[n] {
            return hops < self.hops[n];
        }
        let old = LinkId::new(old);
        // The node next to `n` on each path: upstream going forward, downstream going back.
        let next_to = |l: LinkId| {
            if self.backward { self.network.link_to(l) } else { self.network.link_from(l) }
        };
        crate::canonical::node_order(self.network, next_to(link), next_to(old))
            .then_with(|| {
                self.network.link_length(link).get().total_cmp(&self.network.link_length(old).get())
            })
            .then(link.cmp(&old))
            .is_lt()
    }

    /// The links between the last search's start and `node`, in travel order:
    /// from the source to `node` after [`Self::forward`], from `node` to the
    /// target after [`Self::backward`]. Empty if `node` is the start or was not
    /// reached.
    #[must_use]
    pub fn path(&self, node: NodeId) -> Vec<LinkId> {
        let mut links = Vec::new();
        let mut at = node.index();
        while self.via[at] != NONE {
            let link = LinkId::new(self.via[at]);
            links.push(link);
            at = if self.backward {
                self.network.link_to(link)
            } else {
                self.network.link_from(link)
            }
            .index();
        }
        if !self.backward {
            links.reverse();
        }
        links
    }
}

#[cfg(test)]
mod tests {
    use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
    use openmobisim_core_graph::geometry::LonLat;
    use openmobisim_core_graph::network::{LinkSpec, RoadNetworkBuilder};
    use openmobisim_core_types::diagnostics::Diagnostics;

    use super::*;

    /// a → b → c, and a → c directly (costly); d unreachable.
    fn line() -> (RoadNetwork, Vec<f64>) {
        let mut b = RoadNetworkBuilder::new();
        for (i, n) in ["a", "b", "c", "d"].iter().enumerate() {
            #[allow(clippy::cast_precision_loss, reason = "four nodes")]
            b.add_node(*n, LonLat::new(4.9 + 0.001 * i as f64, 52.37));
        }
        for (id, from, to) in [("ab", "a", "b"), ("bc", "b", "c"), ("ac", "a", "c")] {
            b.add_link(id, from, to, LinkSpec::new(RoadClass::Residential));
        }
        let net = b
            .build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
            .unwrap();
        let mut costs = vec![0.0; 3];
        for (id, c) in [("ab", 10.0), ("bc", 5.0), ("ac", 30.0)] {
            let l = net.link_external_ids().typed_id_of::<LinkId>(id).unwrap();
            costs[l.index()] = c;
        }
        (net, costs)
    }

    #[test]
    fn forward_and_backward_within_a_bound_with_paths() {
        let (net, costs) = line();
        let node = |n: &str| net.node_external_ids().typed_id_of::<NodeId>(n).unwrap();
        let mut reach = Reach::new(&net, &costs);
        let everything = vec![true; 4];
        let found = reach.forward(node("a"), 20.0, &everything);
        assert_eq!(found, [(node("a"), 0.0), (node("b"), 10.0), (node("c"), 15.0)]);
        let ids = |links: Vec<LinkId>| -> Vec<String> {
            links.iter().map(|l| net.link_external_ids().external(l.raw()).to_string()).collect()
        };
        assert_eq!(ids(reach.path(node("c"))), ["ab", "bc"]);
        assert!(reach.forward(node("a"), 12.0, &everything).iter().all(|f| f.0 != node("c")));
        let only_c = [false, false, true, false];
        let found = reach.backward(node("c"), 100.0, &[true; 4]);
        assert_eq!(found, [(node("c"), 0.0), (node("b"), 5.0), (node("a"), 15.0)]);
        assert_eq!(ids(reach.path(node("a"))), ["ab", "bc"], "from a to the target c");
        assert_eq!(reach.forward(node("a"), 100.0, &only_c), [(node("c"), 15.0)]);
    }
}
