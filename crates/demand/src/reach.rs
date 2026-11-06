//! The stops within walking reach of each node, found once per stop.
//!
//! From each stop's node, two searches on the walk network bounded by the walk to a stop at
//! most: [`Search::reach_backward`] gives the walking seconds from every node near it to it,
//! and [`Search::reach`] the seconds from it to every node near it (they differ where a street
//! is open to walkers one way only). The stops are searched in parallel (rayon, one scratch per
//! thread); each search depends on its stop alone, and the results are filed node by node in
//! stop order, so the tables are the same at any thread count.

use openmobisim_graph::network::Network;
use openmobisim_loading::link_times::LinkTimes;
use openmobisim_loading::point_queue::load;
use openmobisim_search::search::Search;
use openmobisim_types::ids::{EntityId, NodeId, StopId};
use openmobisim_types::mode::Mode;
use openmobisim_types::time::{Second, StepGrid};
use openmobisim_types::units::Duration;
use rayon::prelude::*;

/// Per node, a list of (stop, walking seconds), by stop: `start[n]..start[n + 1]` of `items`.
#[derive(Clone, Debug, Default)]
struct PerNode {
    start: Vec<u32>,
    items: Vec<(StopId, f64)>,
}

impl PerNode {
    /// File each stop's `(node, seconds)` list under its nodes, stops in order.
    fn file(nodes: u32, per_stop: &[Vec<(NodeId, Duration)>]) -> Self {
        let mut start = vec![0_u32; nodes as usize + 1];
        for list in per_stop {
            for (node, _) in list {
                start[node.index() + 1] += 1;
            }
        }
        for n in 0..nodes as usize {
            start[n + 1] += start[n];
        }
        let mut fill = start.clone();
        let mut items = vec![(StopId::NULL, f64::NAN); start[nodes as usize] as usize];
        for (s, list) in per_stop.iter().enumerate() {
            for &(node, seconds) in list {
                items[fill[node.index()] as usize] = (StopId::from_index(s), seconds.get());
                fill[node.index()] += 1;
            }
        }
        Self { start, items }
    }

    fn of(&self, node: NodeId) -> &[(StopId, f64)] {
        &self.items[self.start[node.index()] as usize..self.start[node.index() + 1] as usize]
    }
}

/// The stops within walking reach of each node, each way.
#[derive(Clone, Debug, Default)]
pub(crate) struct NearStops {
    to_stop: PerNode,
    from_stop: PerNode,
}

impl NearStops {
    /// Search from the node of every stop (`stop_nodes[s]` is stop `s`'s), on foot, within
    /// `bound` seconds, both ways.
    pub fn new(network: &Network, stop_nodes: &[NodeId], bound: Duration) -> Self {
        let times = free_flow(network);
        let first = Search::new(network, &times);
        let found: Vec<_> = stop_nodes
            .par_iter()
            .map_init(
                || first.clone(),
                |search, &node| {
                    let to = search.reach_backward(node, Mode::Walk, bound);
                    let from = search.reach(node, Mode::Walk, Second::ZERO, bound);
                    (to, from)
                },
            )
            .collect();
        let (to, from): (Vec<_>, Vec<_>) = found.into_iter().unzip();
        let nodes = network.node_count();
        Self { to_stop: PerNode::file(nodes, &to), from_stop: PerNode::file(nodes, &from) }
    }

    /// The stops a walker reaches from `node` within the bound, by stop, with the seconds.
    pub fn stops_reached_from(&self, node: NodeId) -> &[(StopId, f64)] {
        self.to_stop.of(node)
    }

    /// The stops from which a walker reaches `node` within the bound, by stop, with the seconds.
    pub fn stops_reaching(&self, node: NodeId) -> &[(StopId, f64)] {
        self.from_stop.of(node)
    }
}

/// The link times of an empty loading over a window of one second from second 0: free flow on
/// every link, so a walk departing at second 0 arrives at its walking seconds.
fn free_flow(network: &Network) -> LinkTimes {
    let window = StepGrid::new(Second::ZERO, 1, 1).expect("one step of one second");
    let empty = load(network, &[], window).expect("an empty loading refuses nothing");
    LinkTimes::new(network, &empty.links)
}
