//! The fastest route for a mode at the current link times: a time-dependent search over nodes.
//!
//! # What it finds
//!
//! [`Search::fastest`] answers a [`Query`] (origin node, destination node, mode, departure) with
//! the [`Route`] of earliest arrival: the links in order and the arrival, in seconds since the
//! window's start. A link's time is read from the last loading, [`LinkTimes::time`], at the
//! instant the route enters it, which is when the link before it ends. Nothing else is read:
//! no turn, no penalty, no waiting at a node (on first-in-first-out times waiting never helps).
//!
//! # How: label-setting Dijkstra on arrival, goal-directed
//!
//! A node's label is the earliest arrival there found so far; the node of least label is
//! settled next and its out-links are relaxed by reading each one's time at that label. On
//! first-in-first-out link times (a later entry never leaves earlier, which the reading
//! guarantees) the first label a node is settled with is its earliest arrival, so the search
//! is exact (Kaufman and Smith 1993). Because a fastest path on positive times never visits a
//! node twice, the search runs over nodes, not links.
//!
//! The order is goal-directed (A\*, Hart, Nilsson and Raphael 1968): a node is keyed by its
//! label plus a lower bound on the rest of the way, the straight-line distance in metres to the
//! destination times `k`, the least, over the links the mode may use, of the link's free-flow
//! time over the straight-line distance between its ends (links whose ends coincide skipped).
//! Any path is at least as long as the straight line between its ends (the triangle
//! inequality), and no link is crossed faster than free flow (the reading never falls below
//! it; checked in debug builds), so the bound never overestimates and is consistent: the route
//! is the one plain Dijkstra finds, with less of the network explored. `k` is shrunk by one
//! part in a million so that rounding cannot make it overestimate, and it is computed once
//! per mode in [`Search::new`].
//!
//! # Equal arrivals: the tie rule
//!
//! Where two routes reach a node at exactly the same instant, the search keeps the **shorter
//! in metres**, then the one of **fewer links**, then the one whose last link has the **lower
//! id**. At equal keys the node of earlier arrival is settled first, so every route reaching a
//! node at its earliest arrival is compared before the node is settled: the route is a function
//! of the network, the link times and the query alone.
//!
//! # The tree form: every node within a bound
//!
//! [`Search::reach`] is the same label-setting from one origin without a destination (plain
//! Dijkstra: no goal, no bound on the rest of the way), stopped when the least label is more
//! than a bound after the departure; it returns every node settled, with its earliest arrival.
//! [`Search::reach_backward`], for a bike or a walker only, runs over the links entering each
//! node instead, on free-flow times, and gives the time from each node to one destination:
//! their times do not depend on the instant, so the time to a node is the same read either way
//! (a car's would need the arrival, which is what is sought). Both return their nodes sorted by
//! id. The demand module uses them to find, once per stop, the nodes within walking reach.
//!
//! # Cost
//!
//! The scratch is 24 bytes per node, reused across queries: a query resets only the nodes it
//! touched, so it costs what it explores, not the size of the network. [`fastest_many`] keeps
//! one scratch per thread.

use core::cmp::Ordering;
use std::collections::BinaryHeap;

use openmobisim_graph::network::Network;
use openmobisim_loading::link_times::LinkTimes;
use openmobisim_types::ids::{EntityId, LinkId, NodeId};
use openmobisim_types::mode::Mode;
use openmobisim_types::time::Second;
use openmobisim_types::units::Duration;
use rayon::prelude::*;

/// A route: the links from an origin node to a destination node, in order, and when it arrives.
#[derive(Clone, Debug, PartialEq)]
pub struct Route {
    /// The links in the order they are travelled; empty when the origin is the destination.
    pub links: Vec<LinkId>,
    /// The arrival at the destination, in seconds since the window's start.
    pub arrival: Duration,
}

/// One question to the search: the fastest route by `mode` from `origin` to `destination`,
/// setting out at `departure`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Query {
    /// The node the route starts at.
    pub origin: NodeId,
    /// The node the route ends at.
    pub destination: NodeId,
    /// Car, bike or walk. Transit is not on links.
    pub mode: Mode,
    /// The departure, a second of the clock.
    pub departure: Second,
}

/// A node on the queue: its key (label plus bound), its label, the node.
#[derive(Clone, Copy, Debug)]
struct Entry {
    key: f64,
    arrival: f64,
    node: u32,
}

impl PartialEq for Entry {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Entry {}

impl PartialOrd for Entry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Entry {
    /// Reversed (`BinaryHeap` pops its greatest): the least key first; at equal keys the earlier
    /// arrival (so a node's every predecessor at its earliest arrival is settled before it);
    /// then the lower node id, which decides only which of two equal entries is looked at first.
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .key
            .total_cmp(&self.key)
            .then_with(|| other.arrival.total_cmp(&self.arrival))
            .then_with(|| other.node.cmp(&self.node))
    }
}

/// Where a search stops: when the destination is settled (and the search is goal-directed
/// toward it), or when the least label is more than the bound, in seconds, after the start.
#[derive(Clone, Copy, Debug)]
enum Stop {
    At(NodeId),
    Beyond(f64),
}

/// The column of a link mode in [`Search::per_metre`].
fn column(mode: Mode) -> usize {
    match mode {
        Mode::Car => 0,
        Mode::Bike => 1,
        Mode::Walk => 2,
        Mode::Transit => panic!("transit is not on links: it runs on its timetable"),
    }
}

/// `k` for `mode` (see the module documentation): the least free-flow time per metre of
/// straight line over the links the mode may use, shrunk by one part in a million; 0 (plain
/// Dijkstra) when no such link has its ends apart.
fn per_metre(network: &Network, mode: Mode) -> f64 {
    let mut least = f64::INFINITY;
    for link in LinkId::iter_space(network.link_count()) {
        if !network.modes(link).contains(mode) {
            continue;
        }
        let a = network.node_position(network.upstream(link));
        let chord = a.distance_to(network.node_position(network.downstream(link)));
        if chord > 0.0 {
            least = least.min(network.free_flow_time(link, mode).get() / chord);
        }
    }
    if least.is_finite() { least * (1.0 - 1e-6) } else { 0.0 }
}

/// The search over one network and one table of link times, with its scratch. Reusable: every
/// query leaves the scratch clean.
#[derive(Clone, Debug)]
pub struct Search<'a> {
    network: &'a Network,
    times: &'a LinkTimes,
    /// `k` per link mode: car, bike, walk.
    per_metre: [f64; 3],
    /// Per node: the earliest arrival found, in seconds since the window's start (infinity when
    /// untouched); the link it was reached by (null at the origin); the route's metres and links.
    arrival: Vec<f64>,
    via: Vec<LinkId>,
    metres: Vec<f64>,
    hops: Vec<u32>,
    touched: Vec<NodeId>,
    heap: BinaryHeap<Entry>,
}

impl<'a> Search<'a> {
    /// A search over `network` at the link times `times` (made from a loading on that network):
    /// the bound's `k` for each link mode, and an empty scratch of 24 bytes per node.
    #[must_use]
    pub fn new(network: &'a Network, times: &'a LinkTimes) -> Self {
        let n = network.node_count() as usize;
        Self {
            network,
            times,
            per_metre: [Mode::Car, Mode::Bike, Mode::Walk].map(|m| per_metre(network, m)),
            arrival: vec![f64::INFINITY; n],
            via: vec![LinkId::NULL; n],
            metres: vec![0.0; n],
            hops: vec![0; n],
            touched: Vec::new(),
            heap: BinaryHeap::new(),
        }
    }

    /// The route of earliest arrival for the query's mode: `None` when the destination cannot
    /// be reached on links the mode may use; the empty route arriving at the departure when the
    /// origin is the destination. Where several routes arrive at exactly the same instant, the
    /// tie rule of the module documentation picks one.
    ///
    /// # Panics
    ///
    /// For [`Mode::Transit`], which is not on links, and for a node the network does not have.
    /// In debug builds, if a link time read is below the link's free-flow time.
    pub fn fastest(&mut self, query: &Query) -> Option<Route> {
        let start = self.times.since_start(query.departure).get();
        let found = self.settle(query.origin, query.mode, start, Stop::At(query.destination));
        let route = found.then(|| self.route_to(query.destination));
        self.clean();
        route
    }

    /// Every node `mode` reaches from `origin` within `bound` seconds of `departure`, with its
    /// earliest arrival in seconds since the window's start (as [`Route::arrival`]), by node id;
    /// the origin first among them at the departure. The same label-setting and tie rule as
    /// [`Self::fastest`], without the goal (plain Dijkstra), stopped when the least label is
    /// more than `bound` after the departure. The arrival at each node is the one
    /// [`Self::fastest`] finds to it.
    ///
    /// # Panics
    ///
    /// As [`Self::fastest`]: for [`Mode::Transit`] and for a node the network does not have.
    pub fn reach(
        &mut self,
        origin: NodeId,
        mode: Mode,
        departure: Second,
        bound: Duration,
    ) -> Vec<(NodeId, Duration)> {
        let start = self.times.since_start(departure).get();
        assert!(mode != Mode::Transit, "transit is not on links: it runs on its timetable");
        self.settle(origin, mode, start, Stop::Beyond(bound.get()));
        let out = self.within(start, bound.get());
        self.clean();
        out
    }

    /// For a bike or a walker, whose link times are free flow at every instant: every node from
    /// which `mode` reaches `destination` within `bound` seconds, with that time in seconds, by
    /// node id; the destination among them at 0. Label-setting over the links entering each
    /// node on their free-flow times, stopped when the least label exceeds `bound`.
    ///
    /// # Panics
    ///
    /// For [`Mode::Car`], whose times depend on the instant (a backward search would need the
    /// arrival, not the departure), and for [`Mode::Transit`], which is not on links; for a node
    /// the network does not have.
    pub fn reach_backward(
        &mut self,
        destination: NodeId,
        mode: Mode,
        bound: Duration,
    ) -> Vec<(NodeId, Duration)> {
        assert!(
            matches!(mode, Mode::Bike | Mode::Walk),
            "a backward reach is for bikes and walkers only, whose times are free flow: {mode}"
        );
        self.settle_backward(destination, mode, bound.get());
        let out = self.within(0.0, bound.get());
        self.clean();
        out
    }

    /// Run the search from `origin` until `stop`. Whether the destination was reached (always
    /// false for a bound); the labels are left in place for [`Self::route_to`] and
    /// [`Self::within`].
    fn settle(&mut self, origin: NodeId, mode: Mode, start: f64, stop: Stop) -> bool {
        let net = self.network;
        let (target, k) = match stop {
            Stop::At(destination) => (net.node_position(destination), self.per_metre[column(mode)]),
            Stop::Beyond(_) => (net.node_position(origin), 0.0),
        };
        let rest = |node: NodeId| net.node_position(node).distance_to(target) * k;
        self.label(origin, start, LinkId::NULL, 0.0, 0);
        self.heap.push(Entry { key: start + rest(origin), arrival: start, node: origin.raw() });
        while let Some(Entry { arrival: at, node, .. }) = self.heap.pop() {
            let node = NodeId::new(node);
            if at.total_cmp(&self.arrival[node.index()]) != Ordering::Equal {
                continue; // a later label of a node since reached earlier
            }
            match stop {
                Stop::At(destination) if node == destination => return true,
                Stop::Beyond(bound) if at - start > bound => return false,
                _ => {}
            }
            let (metres, hops) = (self.metres[node.index()], self.hops[node.index()]);
            for &link in net.out_links(node) {
                if !net.modes(link).contains(mode) {
                    continue;
                }
                let time = self.times.time(link, mode, Duration(at)).get();
                debug_assert!(
                    time >= net.free_flow_time(link, mode).get(),
                    "link {link} read {time} s, below its free-flow time"
                );
                let next = net.downstream(link);
                let key = |arrive: f64| arrive + rest(next);
                self.relax(next, (at + time, metres + net.length(link).get(), hops + 1), link, key);
            }
        }
        false
    }

    /// Run the search backward from `destination` over the links entering each node, on the
    /// mode's free-flow times, until the least label exceeds `bound` seconds. Each label is the
    /// time from its node to the destination.
    fn settle_backward(&mut self, destination: NodeId, mode: Mode, bound: f64) {
        let net = self.network;
        self.label(destination, 0.0, LinkId::NULL, 0.0, 0);
        self.heap.push(Entry { key: 0.0, arrival: 0.0, node: destination.raw() });
        while let Some(Entry { arrival: at, node, .. }) = self.heap.pop() {
            let node = NodeId::new(node);
            if at.total_cmp(&self.arrival[node.index()]) != Ordering::Equal {
                continue; // a later label of a node since reached earlier
            }
            if at > bound {
                return;
            }
            let (metres, hops) = (self.metres[node.index()], self.hops[node.index()]);
            for &link in net.in_links(node) {
                if !net.modes(link).contains(mode) {
                    continue;
                }
                let time = net.free_flow_time(link, mode).get();
                let next = net.upstream(link);
                self.relax(
                    next,
                    (at + time, metres + net.length(link).get(), hops + 1),
                    link,
                    |t| t,
                );
            }
        }
    }

    /// Offer `node` the label `(arrive, metres, hops)` by `link`; when it improves (the tie
    /// rule), take it, and queue the node at `key(arrive)` if it arrives earlier than before.
    fn relax(
        &mut self,
        node: NodeId,
        (arrive, metres, hops): (f64, f64, u32),
        link: LinkId,
        key: impl Fn(f64) -> f64,
    ) {
        if self.improves(node, arrive, metres, hops, link) {
            let fresh = arrive < self.arrival[node.index()];
            self.label(node, arrive, link, metres, hops);
            if fresh {
                self.heap.push(Entry { key: key(arrive), arrival: arrive, node: node.raw() });
            }
        }
    }

    /// Every labelled node whose label is at most `bound` seconds after `start`, with its label,
    /// by node id. Called once the search has stopped beyond the bound, when every such label is
    /// settled.
    fn within(&self, start: f64, bound: f64) -> Vec<(NodeId, Duration)> {
        let mut out: Vec<(NodeId, Duration)> = self
            .touched
            .iter()
            .filter(|n| self.arrival[n.index()] - start <= bound)
            .map(|&n| (n, Duration(self.arrival[n.index()])))
            .collect();
        out.sort_unstable_by_key(|&(n, _)| n);
        out
    }

    /// Whether reaching `node` at `arrive` by `link`, `metres` and `hops` long, is better than
    /// its label: earlier, or at the same instant shorter, then of fewer links, then by a link
    /// of lower id (the tie rule).
    #[allow(clippy::float_cmp, reason = "the tie rule is for exact ties of arrival and length")]
    fn improves(&self, node: NodeId, arrive: f64, metres: f64, hops: u32, link: LinkId) -> bool {
        let i = node.index();
        if arrive != self.arrival[i] {
            return arrive < self.arrival[i];
        }
        if metres != self.metres[i] {
            return metres < self.metres[i];
        }
        if hops != self.hops[i] {
            return hops < self.hops[i];
        }
        !self.via[i].is_null() && link < self.via[i]
    }

    /// Label `node`: reached at `arrival` by `via`, the route `metres` and `hops` long.
    fn label(&mut self, node: NodeId, arrival: f64, via: LinkId, metres: f64, hops: u32) {
        let i = node.index();
        if self.arrival[i].is_infinite() {
            self.touched.push(node);
        }
        self.arrival[i] = arrival;
        self.via[i] = via;
        self.metres[i] = metres;
        self.hops[i] = hops;
    }

    /// The route to a settled `destination`, walked back along the links each node was
    /// reached by.
    fn route_to(&self, destination: NodeId) -> Route {
        let mut links = Vec::with_capacity(self.hops[destination.index()] as usize);
        let mut node = destination;
        while let Some(link) = self.via[node.index()].to_option() {
            links.push(link);
            node = self.network.upstream(link);
        }
        links.reverse();
        Route { links, arrival: Duration(self.arrival[destination.index()]) }
    }

    /// Reset what this query touched, and nothing else.
    fn clean(&mut self) {
        for node in self.touched.drain(..) {
            self.arrival[node.index()] = f64::INFINITY;
            self.via[node.index()] = LinkId::NULL;
        }
        self.heap.clear();
    }
}

/// [`Search::fastest`] for every query, in parallel (rayon), one scratch per thread. Result `i`
/// answers query `i`, and each answer depends on nothing but its query: the vector is the same,
/// bit for bit, for any number of threads.
///
/// # Panics
///
/// As [`Search::fastest`], for any query.
#[must_use]
pub fn fastest_many(network: &Network, times: &LinkTimes, queries: &[Query]) -> Vec<Option<Route>> {
    let first = Search::new(network, times);
    queries.par_iter().map_init(|| first.clone(), |search, q| search.fastest(q)).collect()
}
