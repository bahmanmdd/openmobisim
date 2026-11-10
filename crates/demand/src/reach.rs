//! The stops within walking reach of each node, found once per stop.
//!
//! From each stop's node, two searches on the walk network bounded by the walk to a stop at
//! most: [`Search::reach_backward`] gives the walking seconds from every node near it to it,
//! and [`Search::reach`] the seconds from it to every node near it (they differ where a street
//! is open to walkers one way only). The stops are searched in parallel (rayon, one scratch per
//! thread); each search depends on its stop alone, and the results are filed node by node in
//! stop order, so the tables are the same at any thread count.
//!
//! For a trip end with no stop within the bound, [`nearest_stops`] gives the one stop with the
//! least whole walk, by an unbounded search from the end's node.

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

/// The stops within walking reach of each node, each way. Public for the GTFS reader, which
/// derives the footpaths between stops from it.
#[derive(Clone, Debug, Default)]
pub struct NearStops {
    to_stop: PerNode,
    from_stop: PerNode,
}

impl NearStops {
    /// Search from the node of every stop (`stop_nodes[s]` is stop `s`'s), on foot, within
    /// `bound` seconds, both ways.
    #[must_use]
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
    #[must_use]
    pub fn stops_reached_from(&self, node: NodeId) -> &[(StopId, f64)] {
        self.to_stop.of(node)
    }

    /// The stops from which a walker reaches `node` within the bound, by stop, with the seconds.
    #[must_use]
    pub fn stops_reaching(&self, node: NodeId) -> &[(StopId, f64)] {
        self.from_stop.of(node)
    }
}

/// The stop with the least whole walk from or to an end, with its walking seconds; `None` when
/// no stop can be reached on foot.
pub(crate) type Nearest = Option<(StopId, Duration)>;

/// For each of `ends` (a walk node and its gap in metres), the stop with the least whole walk:
/// the end's gap, the walk between the end's node and the stop's node, and the stop's gap
/// (`stops[s]`, node and gap), at the network's walk speed; ties to the lower stop id. With its
/// walking seconds; `None` when no stop can be reached on foot. `to_stop`: the walk from the end
/// to the stop (an origin's, [`Search::reach`]), else from the stop to the end (a destination's,
/// [`Search::reach_backward`]).
///
/// One search per distinct node, in parallel, bounded first at twice `from` seconds, the bound
/// doubled while an end's least whole walk is not below its gap plus the bound, and dropped after
/// 16 times `from`. A stop the bounded search does not reach is walked more than the bound, so its
/// whole walk is at least the end's gap plus the bound: the least walk below that is the least of
/// all, ties included. Each end's result depends on its node and gap alone, so it is the same at
/// any thread count.
pub(crate) fn nearest_stops(
    network: &Network,
    stops: &[(NodeId, f64)],
    ends: &[(NodeId, f64)],
    to_stop: bool,
    from: Duration,
) -> Vec<Nearest> {
    let speed = network.walk_speed().get();
    let mut by_node: Vec<(NodeId, usize)> =
        ends.iter().enumerate().map(|(i, &(node, _))| (node, i)).collect();
    by_node.sort_unstable();
    let groups: Vec<&[(NodeId, usize)]> = by_node.chunk_by(|a, b| a.0 == b.0).collect();
    let times = free_flow(network);
    let first = Search::new(network, &times);
    let found: Vec<Vec<(usize, Nearest)>> = groups
        .par_iter()
        .map_init(
            || first.clone(),
            |search, group| {
                let node = group[0].0;
                let mut bound = 2.0 * from.get();
                loop {
                    let walk = if to_stop {
                        search.reach(node, Mode::Walk, Second::ZERO, Duration(bound))
                    } else {
                        search.reach_backward(node, Mode::Walk, Duration(bound))
                    };
                    let best: Vec<_> = group
                        .iter()
                        .map(|&(_, i)| (i, least_walk(&walk, stops, ends[i].1, speed)))
                        .collect();
                    let sure = |&(i, b): &(usize, Nearest)| {
                        b.is_some_and(|(_, t)| t.get() < ends[i].1 / speed + bound)
                    };
                    if bound.is_infinite() || best.iter().all(sure) {
                        return best;
                    }
                    bound = if bound >= 16.0 * from.get() { f64::INFINITY } else { 2.0 * bound };
                }
            },
        )
        .collect();
    let mut out = vec![None; ends.len()];
    for (i, best) in found.into_iter().flatten() {
        out[i] = best;
    }
    out
}

/// Of the stops whose nodes `walk` reaches (by node, with the walking seconds), the one with the
/// least whole walk from an end of `gap` metres, ties to the lower stop id; with its seconds.
fn least_walk(
    walk: &[(NodeId, Duration)],
    stops: &[(NodeId, f64)],
    gap: f64,
    speed: f64,
) -> Nearest {
    let mut best: Nearest = None;
    for (s, &(node, stop_gap)) in stops.iter().enumerate() {
        let Ok(at) = walk.binary_search_by_key(&node, |w| w.0) else { continue };
        let total = gap / speed + walk[at].1.get() + stop_gap / speed;
        if best.is_none_or(|(_, b)| total < b.get()) {
            best = Some((StopId::from_index(s), Duration(total)));
        }
    }
    best
}

/// The link times of an empty loading over a window of one second from second 0: free flow on
/// every link, so a walk departing at second 0 arrives at its walking seconds.
fn free_flow(network: &Network) -> LinkTimes {
    let window = StepGrid::new(Second::ZERO, 1, 1).expect("one step of one second");
    let empty = load(network, &[], window).expect("an empty loading refuses nothing");
    LinkTimes::new(network, &empty.links)
}
