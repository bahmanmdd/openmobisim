//! Landmark lower bounds (ALT: A\*, landmarks, triangle inequality; Goldberg & Harrelson 2005)
//! for goal-directed searches.
//!
//! A [`Landmarks`] holds, for a few nodes spread over the network (the landmarks), the least
//! cost from each landmark to every node and from every node to each landmark, on **lower-bound
//! link costs**: costs no search step can go below (a car link's free-flow time plus its class's
//! extra, since a link's time is never below its free-flow time). Turns are ignored, which only
//! lowers the costs. By the triangle inequality, the cost from node `v` to node `t` is at least
//! `d(v, L) − d(t, L)` and `d(L, t) − d(L, v)` for every landmark `L`; the largest of these is
//! [`Landmarks::bound`], and the least of it over several targets is a bound towards the nearest.
//!
//! **Consistent:** for a link `u → v` of lower-bound cost `c`, the bound at `u` exceeds the
//! bound at `v` by at most `c` (each term moves by at most `c`, by the same triangle
//! inequality), so a search ordered by its cost plus the bound settles every label as the plain
//! search does (Hart, Nilsson & Raphael 1968). The bound is shrunk by one part in a million, so
//! that the rounding of the sums never breaks this and a step of positive cost always raises the
//! search's key.
//!
//! **Cost:** two searches per landmark over the nodes when built; 16 bytes per node per
//! landmark; one bound is two reads per landmark per target.

use std::collections::BinaryHeap;

use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_types::ids::{EntityId, NodeId};

/// The share a bound is shrunk by against rounding.
const SHRINK: f64 = 1e-6;

/// Lower-bound costs from and to a few landmarks. See the [module docs](self).
#[derive(Debug, Clone)]
pub struct Landmarks {
    /// Per landmark, the least cost from it to each node (`f64::INFINITY` where none).
    from: Vec<Vec<f64>>,
    /// Per landmark, the least cost from each node to it.
    to: Vec<Vec<f64>>,
}

/// A node and its cost so far, cheapest first (ties by node).
#[derive(Clone, Copy, PartialEq)]
struct Item(f64, u32);
impl Eq for Item {}
impl PartialOrd for Item {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Item {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        other.0.total_cmp(&self.0).then_with(|| other.1.cmp(&self.1))
    }
}

/// The least cost from `start` to every node (`forward`) or from every node to `start`, over
/// the links of finite `cost`, turns ignored.
fn costs(network: &RoadNetwork, cost: &[f64], start: NodeId, forward: bool) -> Vec<f64> {
    let mut dist = vec![f64::INFINITY; network.node_count() as usize];
    let mut heap = BinaryHeap::new();
    dist[start.index()] = 0.0;
    heap.push(Item(0.0, start.raw()));
    while let Some(Item(d, n)) = heap.pop() {
        let node = NodeId::new(n);
        if d > dist[node.index()] {
            continue;
        }
        let links = if forward { network.out_links(node) } else { network.in_links(node) };
        for &l in links {
            let c = cost[l.index()];
            if !c.is_finite() {
                continue;
            }
            let next = if forward { network.link_to(l) } else { network.link_from(l) };
            let nd = d + c;
            if nd < dist[next.index()] {
                dist[next.index()] = nd;
                heap.push(Item(nd, next.raw()));
            }
        }
    }
    dist
}

impl Landmarks {
    /// `count` landmarks over `network` with lower-bound link costs `cost` (one per link,
    /// infinite for a link that is never used, none below 0): the first is the node farthest
    /// from node 0, each next the node farthest from those chosen (by the least of the costs
    /// from them), so that they spread to the network's edges.
    ///
    /// # Panics
    ///
    /// Panics if `cost` does not have one value per link or holds a negative value.
    #[must_use]
    pub fn new(network: &RoadNetwork, cost: &[f64], count: usize) -> Self {
        assert_eq!(cost.len(), network.link_count() as usize, "one cost per link");
        assert!(cost.iter().all(|&c| c >= 0.0), "lower-bound costs are not negative");
        let mut out = Self { from: Vec::new(), to: Vec::new() };
        if network.node_count() == 0 {
            return out;
        }
        // The least cost from the chosen landmarks to each node: the next is its largest.
        let mut nearest = costs(network, cost, NodeId::new(0), true);
        for _ in 0..count {
            let Some(next) = nearest
                .iter()
                .enumerate()
                .filter(|(_, d)| d.is_finite())
                .max_by(|a, b| a.1.total_cmp(b.1))
                .map(|(i, _)| NodeId::from_index(i))
            else {
                break;
            };
            let from = costs(network, cost, next, true);
            for (n, d) in nearest.iter_mut().zip(&from) {
                *n = n.min(*d);
            }
            out.to.push(costs(network, cost, next, false));
            out.from.push(from);
        }
        out
    }

    /// A lower bound on the cost from `node` to the nearest of `targets` (0 with none, and
    /// for a node with no landmark term).
    #[must_use]
    pub fn bound(&self, node: NodeId, targets: &[NodeId]) -> f64 {
        let v = node.index();
        let mut least = f64::INFINITY;
        for t in targets {
            let t = t.index();
            let mut best = 0.0_f64;
            for (from, to) in self.from.iter().zip(&self.to) {
                for gap in [to[v] - to[t], from[t] - from[v]] {
                    if gap.is_finite() {
                        best = best.max(gap);
                    }
                }
            }
            least = least.min(best);
        }
        if least.is_finite() { least * (1.0 - SHRINK) } else { 0.0 }
    }
}
