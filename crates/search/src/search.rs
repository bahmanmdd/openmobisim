//! The fastest route for a mode at the current link times: a time-dependent search over nodes.
//!
//! # What it finds
//!
//! [`Search::fastest`] answers a [`Query`] (origin node, destination node, mode, departure,
//! traveller) with the [`Route`] of earliest arrival, but for the one limit stated with the tie
//! rule below: the links in order and the arrival, in seconds since the window's start. A
//! link's exit is read from the last loading, [`LinkTimes::exit`], for that traveller at the
//! instant the route enters it, which is when the link before it ends. Nothing else is read: no
//! turn, no penalty, no waiting at a node (on first-in-first-out times waiting never helps).
//!
//! # How: label-setting Dijkstra on arrival, goal-directed
//!
//! A node's label is the earliest arrival there found so far; the node of least label is
//! settled next and its out-links are relaxed by reading each one's time at that label. On
//! link times that are first-in-first-out (a later entry never leaves earlier, which the
//! reading guarantees) and depend on the instant of entry alone, the first label a node is
//! settled with is its earliest arrival, so the search is exact (Kaufman and Smith 1993).
//! Because a fastest path on positive times never visits a node twice, the search runs over
//! nodes, not links.
//!
//! The reading depends on one thing more than the instant: the recorded car whose place the
//! traveller took on the link it came by (see [`LinkTimes`]). A node's label carries it, beside
//! the arrival, as the reader for the next link, so a route's arrival is always the reading's
//! own walk of it, [`LinkTimes::route_time`], to the bit. One label is kept per node. What the
//! search guarantees is the earliest arrival over every path where no two routes reach a node
//! at exactly the same instant having taken different places there (one a recorded car's and
//! the other none, or two different cars'); this is checked against every simple path
//! enumerated, over random loadings of one network (`tests/brute_force.rs`). At such a tie it
//! is not exact: see the tie rule below.
//!
//! The order is goal-directed (A\*, Hart, Nilsson and Raphael 1968): a node is keyed by its
//! label plus a lower bound on the rest of the way, the mode's free-flow time from the node to
//! the destination, shrunk by one part in a million. No link is crossed faster than free flow
//! (the reading never falls below it; checked in debug builds), so the bound never
//! overestimates and is consistent, and the shrinking keeps rounding from breaking either: the
//! route is the one plain Dijkstra finds, with less of the network explored.
//!
//! The bound is itself a search, backward from the destination over the links entering each
//! node on free-flow times, and it is run on only as far as it is asked: a node's time to the
//! destination is final once no time still queued is below it. A query's bound therefore
//! settles the nodes its route comes near, not the whole network, and reads the same times as
//! from a backward search run to the end. A [`Search`] keeps the bound of the destination and
//! mode it last served, so queries to one destination share it.
//!
//! # Equal arrivals: the tie rule
//!
//! Where two routes reach a node at exactly the same instant, the search keeps the **shorter
//! in metres**, then the one of **fewer links**, then the one whose last link has the **lower
//! id**. At equal keys the node of earlier arrival is settled first, so every route reaching a
//! node at its earliest arrival is compared before the node is settled: the route is a function
//! of the network, the link times and the query alone.
//!
//! **The limit of one label per node.** The reading of the link after a node depends, beside
//! the instant, on the recorded car whose place the traveller took on the link it came by.
//! Where two routes reach a node at exactly the same instant, one having taken a recorded
//! car's place there and the other not, or the place of another car, the tie rule keeps one
//! without looking at the place. The route it keeps can leave the next link one headway of
//! that link later than the route it drops would, and the queues on later links can add to
//! that difference: one headway is not a bound on the difference in arrival. The route
//! returned is then not the earliest over every path, though its arrival is still the
//! reading's own walk of it. By hand (`tests/fastest.rs`): two links of
//! 72 s into one node, both left at 75 s, one of them in a recorded car's place; the tie rule
//! keeps the other, the shorter in metres, and its route arrives at 152 s where the route it
//! dropped arrives at 148 s.
//!
//! # The tree form: every node within a bound
//!
//! [`Search::reach`] is the same label-setting from one origin without a destination (plain
//! Dijkstra: no goal, no bound on the rest of the way), stopped when the least label is more
//! than a bound after the departure; it returns every node settled, with its earliest arrival
//! (but for the limit of one label per node, above).
//! [`Search::reach_backward`], for a bike or a walker only, is the bound's own search run to a
//! limit: it gives the time from each node to one destination on free-flow times, which do not
//! depend on the instant, so the time to a node is the same read either way (a car's would
//! need the arrival, which is what is sought). Both return their nodes sorted by id. The demand
//! module uses them to find, once per stop, the nodes within walking reach.
//!
//! # Cost
//!
//! The scratch is 40 bytes per node (32 of labels, 8 of the bound), reused across queries: a
//! query resets only the nodes it touched, and a new destination only the nodes its bound
//! touched, so a query costs what it explores, not the size of the network. [`fastest_many`]
//! keeps one scratch per thread.

use core::cmp::Ordering;
use std::collections::BinaryHeap;
use std::sync::Mutex;

use openmobisim_graph::network::Network;
use openmobisim_loading::link_times::{LinkTimes, Reader};
use openmobisim_types::ids::{EntityId, LinkId, NodeId, TripId};
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

/// One question to the search: the fastest route by `mode` from `origin` to `destination` for
/// the traveller of `trip`, setting out at `departure`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Query {
    /// The node the route starts at.
    pub origin: NodeId,
    /// The node the route ends at.
    pub destination: NodeId,
    /// Car, bike or walk. Transit is not on links.
    pub mode: Mode,
    /// The departure, in seconds since the window's start: any instant, not only a whole
    /// second.
    pub departure: Duration,
    /// The traveller: its trip, whose own car in the last loading is not in its way; `None`
    /// for a traveller the loading did not have.
    pub trip: Option<TripId>,
}

impl Query {
    /// The reader of the link times for this query's traveller.
    fn reader(&self) -> Reader {
        self.trip.map_or_else(Reader::stranger, Reader::of)
    }
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

/// The free-flow time by one mode from every node to one destination: the bound on the rest of
/// the way (see the module documentation). A label-setting search over the links entering each
/// node, run on only as far as it is asked.
#[derive(Clone, Debug)]
struct ToGoal {
    /// The destination and the mode it serves; none before the first.
    goal: Option<(NodeId, Mode)>,
    /// Per node, the least time to the destination found so far (infinity when untouched).
    time: Vec<f64>,
    touched: Vec<NodeId>,
    heap: BinaryHeap<Entry>,
}

impl ToGoal {
    /// No destination yet, over `nodes` nodes.
    fn new(nodes: usize) -> Self {
        Self {
            goal: None,
            time: vec![f64::INFINITY; nodes],
            touched: Vec::new(),
            heap: BinaryHeap::new(),
        }
    }

    /// Start over for `destination` by `mode`: its time 0, nothing settled yet. Resets what
    /// the destination before it touched, and nothing else.
    fn start(&mut self, destination: NodeId, mode: Mode) {
        for node in self.touched.drain(..) {
            self.time[node.index()] = f64::INFINITY;
        }
        self.heap.clear();
        self.goal = Some((destination, mode));
        self.offer(destination, 0.0);
    }

    /// Offer `node` the time `time` to the destination: taken and queued when less than it had.
    fn offer(&mut self, node: NodeId, time: f64) {
        let known = &mut self.time[node.index()];
        if time < *known {
            if known.is_infinite() {
                self.touched.push(node);
            }
            *known = time;
            self.heap.push(Entry { key: time, arrival: time, node: node.raw() });
        }
    }

    /// Settle the next node: take the least time off the queue and offer, over each link
    /// entering its node, that time plus the link's free-flow time to the link's upstream node.
    fn settle_next(&mut self, net: &Network) {
        let (Some((_, mode)), Some(Entry { arrival: at, node, .. })) = (self.goal, self.heap.pop())
        else {
            return;
        };
        let node = NodeId::new(node);
        if at.total_cmp(&self.time[node.index()]) != Ordering::Equal {
            return; // a longer time of a node since found nearer
        }
        for &link in net.in_links(node) {
            if net.modes(link).contains(mode) {
                self.offer(net.upstream(link), at + net.free_flow_time(link, mode).get());
            }
        }
    }

    /// The time from `node` to the destination, infinity where it cannot be reached. The search
    /// is run on only until that time is known: a time is final once no time still queued is
    /// below it, and a node never offered one is known to be out of reach when the queue has
    /// run dry.
    fn time(&mut self, net: &Network, node: NodeId) -> f64 {
        while self.heap.peek().is_some_and(|next| next.arrival < self.time[node.index()]) {
            self.settle_next(net);
        }
        self.time[node.index()]
    }

    /// Every node at most `bound` seconds from the destination, with its time, by node id.
    fn within(&mut self, net: &Network, bound: f64) -> Vec<(NodeId, Duration)> {
        while self.heap.peek().is_some_and(|next| next.arrival <= bound) {
            self.settle_next(net);
        }
        let mut out: Vec<(NodeId, Duration)> = self
            .touched
            .iter()
            .filter(|n| self.time[n.index()] <= bound)
            .map(|&n| (n, Duration(self.time[n.index()])))
            .collect();
        out.sort_unstable_by_key(|&(n, _)| n);
        out
    }
}

/// The search over one network and one table of link times, with its scratch. Reusable: every
/// query leaves the scratch clean.
#[derive(Clone, Debug)]
pub struct Search<'a> {
    network: &'a Network,
    times: &'a LinkTimes,
    /// Per node: the earliest arrival found, in seconds since the window's start (infinity when
    /// untouched); the reader for the links out of it; the link it was reached by (null at the
    /// origin); the route's metres and links.
    arrival: Vec<f64>,
    reader: Vec<Reader>,
    via: Vec<LinkId>,
    metres: Vec<f64>,
    hops: Vec<u32>,
    touched: Vec<NodeId>,
    heap: BinaryHeap<Entry>,
    /// The bound of the destination and mode last served, kept for the next query to them.
    to_goal: ToGoal,
}

impl<'a> Search<'a> {
    /// A search over `network` at the link times `times` (made from a loading on that network),
    /// with an empty scratch of 40 bytes per node.
    #[must_use]
    pub fn new(network: &'a Network, times: &'a LinkTimes) -> Self {
        let n = network.node_count() as usize;
        Self {
            network,
            times,
            arrival: vec![f64::INFINITY; n],
            reader: vec![Reader::stranger(); n],
            via: vec![LinkId::NULL; n],
            metres: vec![0.0; n],
            hops: vec![0; n],
            touched: Vec::new(),
            heap: BinaryHeap::new(),
            to_goal: ToGoal::new(n),
        }
    }

    /// The route of earliest arrival for the query's mode, but for the limit of one label per
    /// node stated in the module documentation: `None` when the destination cannot be reached
    /// on links the mode may use; the empty route arriving at the departure when the origin is
    /// the destination. Where several routes arrive at exactly the same instant, the tie rule of
    /// the module documentation picks one.
    ///
    /// # Panics
    ///
    /// For [`Mode::Transit`], which is not on links, and for a node the network does not have.
    /// In debug builds, if a link time read is below the link's free-flow time.
    pub fn fastest(&mut self, query: &Query) -> Option<Route> {
        assert!(query.mode != Mode::Transit, "transit is not on links: it runs on its timetable");
        if self.to_goal.goal != Some((query.destination, query.mode)) {
            self.to_goal.start(query.destination, query.mode);
        }
        if self.to_goal.time(self.network, query.origin).is_infinite() {
            return None;
        }
        let from = (query.origin, query.departure.get(), query.reader());
        let found = self.settle(from, query.mode, Stop::At(query.destination));
        let route = found.then(|| self.route_to(query.destination));
        self.clean();
        route
    }

    /// Every node `mode` reaches from `origin` within `bound` seconds of `departure`, with its
    /// earliest arrival in seconds since the window's start (as [`Route::arrival`]; but for the
    /// limit of one label per node stated in the module documentation), by node id; the origin
    /// first among them at the departure. The same label-setting and tie rule as
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
        self.settle((origin, start, Reader::stranger()), mode, Stop::Beyond(bound.get()));
        let out = self.within(start, bound.get());
        self.clean();
        out
    }

    /// For a bike or a walker, whose link times are free flow at every instant: every node from
    /// which `mode` reaches `destination` within `bound` seconds, with that time in seconds, by
    /// node id; the destination among them at 0. The bound's search over the links entering each
    /// node on their free-flow times, run until the least time exceeds `bound`.
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
        self.to_goal.start(destination, mode);
        self.to_goal.within(self.network, bound.get())
    }

    /// Run the search from `origin` at `start` for `reader` until `stop`. Whether the
    /// destination was reached (always false for a bound); the labels are left in place for
    /// [`Self::route_to`] and [`Self::within`].
    fn settle(
        &mut self,
        (origin, start, reader): (NodeId, f64, Reader),
        mode: Mode,
        stop: Stop,
    ) -> bool {
        let net = self.network;
        self.label(origin, (start, 0.0, 0), LinkId::NULL, reader);
        let key = start + self.rest(origin, stop);
        self.heap.push(Entry { key, arrival: start, node: origin.raw() });
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
            let reader = self.reader[node.index()];
            for &link in net.out_links(node) {
                if !net.modes(link).contains(mode) {
                    continue;
                }
                // No link is left before its free-flow time: one that cannot reach its
                // downstream node before that node's label, nor at the same instant (the tie
                // rule's case), cannot improve it and is not read.
                let next = net.downstream(link);
                let soonest = at + net.free_flow_time(link, mode).get();
                if soonest > self.arrival[next.index()] {
                    continue;
                }
                let (exit, onward) = self.times.exit(link, mode, Duration(at), reader);
                debug_assert!(
                    exit.get() >= soonest,
                    "link {link} is left at {} s, before its free-flow time after {at} s",
                    exit.get()
                );
                let (arrive, metres) = (exit.get(), metres + net.length(link).get());
                // Take the label when it improves (the tie rule); queue the node when it
                // arrives earlier than before.
                if self.improves(next, arrive, metres, hops + 1, link) {
                    let earlier = arrive < self.arrival[next.index()];
                    self.label(next, (arrive, metres, hops + 1), link, onward);
                    if earlier {
                        let key = arrive + self.rest(next, stop);
                        self.heap.push(Entry { key, arrival: arrive, node: next.raw() });
                    }
                }
            }
        }
        false
    }

    /// The bound on the rest of the way from `node`: toward a destination, the free-flow time
    /// to it, shrunk by one part in a million; none for a search stopped beyond a bound.
    fn rest(&mut self, node: NodeId, stop: Stop) -> f64 {
        match stop {
            Stop::At(_) => self.to_goal.time(self.network, node) * (1.0 - 1e-6),
            Stop::Beyond(_) => 0.0,
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

    /// Label `node`: reached at `arrival` by `via`, the route `metres` and `hops` long, `reader`
    /// the reader for the links out of it.
    fn label(
        &mut self,
        node: NodeId,
        (arrival, metres, hops): (f64, f64, u32),
        via: LinkId,
        reader: Reader,
    ) {
        let i = node.index();
        if self.arrival[i].is_infinite() {
            self.touched.push(node);
        }
        self.arrival[i] = arrival;
        self.reader[i] = reader;
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

/// [`Search::fastest`] for every query, in parallel (rayon), one search per thread. Result `i`
/// answers query `i`, and each answer depends on nothing but its query: the vector is the same,
/// bit for bit, for any number of threads.
///
/// # Panics
///
/// As [`Search::fastest`], for any query.
#[must_use]
pub fn fastest_many(network: &Network, times: &LinkTimes, queries: &[Query]) -> Vec<Option<Route>> {
    // In the order of (mode, destination): a thread working through a stretch of them keeps one
    // bound for the queries to one destination.
    let mut order: Vec<usize> = (0..queries.len()).collect();
    order.sort_unstable_by_key(|&i| (queries[i].mode.as_u8(), queries[i].destination, i));
    // One search per thread of the pool, made when the thread first answers a query. A lone
    // query is answered on the calling thread, which need not be of the pool: it takes the first.
    let searches: Vec<Mutex<Option<Search>>> =
        (0..rayon::current_num_threads()).map(|_| Mutex::new(None)).collect();
    let answered: Vec<Option<Route>> = order
        .par_iter()
        .map(|&i| {
            let thread = rayon::current_thread_index().unwrap_or(0);
            let mut search = searches[thread].lock().expect("no search panicked before");
            search.get_or_insert_with(|| Search::new(network, times)).fastest(&queries[i])
        })
        .collect();
    let mut out = vec![None; queries.len()];
    for (&i, answer) in order.iter().zip(answered) {
        out[i] = answer;
    }
    out
}
