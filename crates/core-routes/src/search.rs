//! The turn-aware shortest-path search every generator is built on.
//!
//! A search runs over **links**, not nodes: a state is "having just entered
//! link `l`", and the next links are the turns the [`TurnTable`] allows from
//! it. Today the table only removes U-turns, so the costs equal a node-based
//! search's; when turn restrictions or turn delays arrive they drop in here
//! without touching a generator.
//!
//! **Costs** are free-flow travel times (control delay included, S90) over
//! links that carry motor traffic; other links cost infinity. A bike or walk
//! layer searches its own costs instead ([`SearchContext::with_costs`], S195). A generator can
//! multiply any link's cost with [`LinkFactors`] for the next search (the
//! penalty method does), and asks [`Search::max_overlap`] how much of a route lies
//! on routes it has [marked](Search::mark).
//!
//! A search can be **bounded** ([`Search::shortest_within`]): a label whose true
//! (unpenalised) cost already exceeds a bound is dropped, since no way of
//! finishing it can come back under. The penalty method uses this with its
//! detour bound; without it, compounding penalties send the search far past
//! the routes it would accept (a hundredfold cost on a country-sized network,
//! measured in S165). The bound is a resource limit on a constrained search:
//! it never lets a route past the bound through, and on rare occasions can miss
//! one that a fuller search would have found.
//!
//! A generator can also perturb every link's cost at random for the next search
//! ([`Search::set_noise`]): each link's factor is `1 + sigma · u · bias`, `u` a uniform draw
//! that is a **pure function of (seed, link)** — a few integer operations, no generator to
//! carry, no array to fill — so a search is the same for any thread and order. The Monte
//! Carlo method uses it (S176).
//!
//! **Cost of a search:** one label per touched link; the scratch is reset by
//! visiting only what was touched, so a search costs what it explores, not the
//! size of the network. **Scratch per thread:** 20 bytes per link plus the
//! heap.
//!
//! **Determinism:** ties are broken by link id, so a search never depends on
//! heap insertion order (Foundations §1).

use core::cmp::Ordering;
use std::collections::BinaryHeap;
use std::sync::Arc;

use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_types::ids::{EntityId, LinkId, NodeId};

/// The most routes a set can hold: one bit each in a per-link mask.
pub const MAX_ROUTES_PER_SET: usize = 32;

/// One route: the links from an origin node to a destination node.
#[derive(Clone, Debug, PartialEq)]
pub struct Route {
    /// The links in driving order.
    pub links: Vec<LinkId>,
    /// The route's free-flow travel time in seconds, **unpenalised**: what the
    /// route costs, not what a penalised search thought it cost.
    pub cost: f64,
    /// The largest share of this route's cost that it has in common with any
    /// route accepted before it (0 for the first).
    pub overlap: f64,
}

/// What every search shares: the network, its turns, and each link's cost.
#[derive(Debug)]
pub struct SearchContext<'a> {
    /// The network searched.
    pub network: &'a RoadNetwork,
    /// The turns a search may take.
    pub turns: &'a TurnTable,
    cost: Vec<f64>,
}

impl<'a> SearchContext<'a> {
    /// Costs are read from `network` once, here: 8 bytes per link, shared by
    /// every thread.
    #[must_use]
    pub fn new(network: &'a RoadNetwork, turns: &'a TurnTable) -> Self {
        let cost = (0..network.link_count())
            .map(|i| {
                let link = LinkId::new(i);
                if network.link_class(link).carries_motor_traffic() {
                    network.free_flow_time(link).get()
                } else {
                    f64::INFINITY
                }
            })
            .collect();
        Self { network, turns, cost }
    }

    /// A search over `cost` instead of the car's free-flow times (S195): the
    /// bike and walk layers' costs, one per link of `network`, infinite for a
    /// link that must not be used.
    ///
    /// # Panics
    ///
    /// Panics if `cost` does not have one entry per link.
    #[must_use]
    pub fn with_costs(network: &'a RoadNetwork, turns: &'a TurnTable, cost: Vec<f64>) -> Self {
        assert_eq!(cost.len(), network.link_count() as usize, "one cost per link");
        Self { network, turns, cost }
    }

    /// A link's cost in seconds, infinite if a car cannot use it.
    #[inline]
    #[must_use]
    pub fn link_cost(&self, link: LinkId) -> f64 {
        self.cost[link.index()]
    }

    /// The summed cost of `links`.
    #[must_use]
    pub fn route_cost(&self, links: &[LinkId]) -> f64 {
        links.iter().map(|&l| self.link_cost(l)).sum()
    }
}

/// Per-link cost multipliers for the next searches; every link is 1 until set.
#[derive(Debug)]
pub struct LinkFactors {
    factor: Vec<f64>,
    touched: Vec<u32>,
}

impl LinkFactors {
    fn new(links: usize) -> Self {
        Self { factor: vec![1.0; links], touched: Vec::new() }
    }

    /// Multiply `link`'s cost by `by` from now on.
    pub fn multiply(&mut self, link: LinkId, by: f64) {
        let f = &mut self.factor[link.index()];
        #[allow(clippy::float_cmp, reason = "1.0 is the exact value of an untouched link")]
        if *f == 1.0 {
            self.touched.push(link.raw());
        }
        *f *= by;
    }

    /// Set every link back to 1, visiting only those changed.
    pub fn reset(&mut self) {
        for &l in &self.touched {
            self.factor[l as usize] = 1.0;
        }
        self.touched.clear();
    }

    #[inline]
    fn get(&self, link: LinkId) -> f64 {
        self.factor[link.index()]
    }
}

/// Random link-cost factors for one search: `1 + sigma · u · bias(link)`.
#[derive(Clone, Debug)]
struct Noise {
    seed: u64,
    sigma: f64,
    /// Per link, in `[0, 1]`: how much of `sigma` the link gets. `None` is 1 everywhere.
    bias: Option<Arc<[f32]>>,
}

impl Noise {
    /// The factor of `link`: a pure function of the seed and the link.
    #[inline]
    fn factor(&self, link: LinkId) -> f64 {
        // The finalizer of SplitMix64 over the seed and the link id.
        let mut x = self.seed ^ u64::from(link.raw()).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        x ^= x >> 30;
        x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        x ^= x >> 27;
        x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
        x ^= x >> 31;
        #[allow(
            clippy::cast_precision_loss,
            reason = "the top 53 bits of a u64 fit an f64 exactly"
        )]
        let u = (x >> 11) as f64 / (1_u64 << 53) as f64;
        let bias = self.bias.as_ref().map_or(1.0, |b| f64::from(b[link.index()]));
        1.0 + self.sigma * u * bias
    }
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    cost: f64,
    link: u32,
}

impl PartialEq for Entry {
    fn eq(&self, other: &Self) -> bool {
        self.cost.to_bits() == other.cost.to_bits() && self.link == other.link
    }
}
impl Eq for Entry {}
impl PartialOrd for Entry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Entry {
    // Reversed: `BinaryHeap` is a max-heap and the cheapest entry pops first;
    // the link id breaks ties.
    fn cmp(&self, other: &Self) -> Ordering {
        other.cost.total_cmp(&self.cost).then_with(|| other.link.cmp(&self.link))
    }
}

const NONE: u32 = u32::MAX;

/// One thread's search state. Cheap to keep and reuse: every method leaves it clean.
#[derive(Debug)]
pub struct Search<'a> {
    ctx: &'a SearchContext<'a>,
    dist: Vec<f64>,
    /// The unpenalised cost of the route each label stands for.
    true_dist: Vec<f64>,
    pred: Vec<u32>,
    touched: Vec<u32>,
    heap: BinaryHeap<Entry>,
    /// Cost multipliers for the next searches.
    pub factors: LinkFactors,
    /// Random cost factors for the next searches, if a generator asked for them.
    noise: Option<Noise>,
    marks: Vec<u32>,
    marked: Vec<u32>,
}

impl<'a> Search<'a> {
    /// A fresh scratch over `ctx`: 20 bytes per link plus 4 for the marks.
    #[must_use]
    pub fn new(ctx: &'a SearchContext<'a>) -> Self {
        let n = ctx.network.link_count() as usize;
        Self {
            ctx,
            dist: vec![f64::INFINITY; n],
            true_dist: vec![0.0; n],
            pred: vec![NONE; n],
            touched: Vec::new(),
            heap: BinaryHeap::new(),
            factors: LinkFactors::new(n),
            noise: None,
            marks: vec![0; n],
            marked: Vec::new(),
        }
    }

    /// Perturb every link's cost at random in the next searches: link `l` costs its cost
    /// times `1 + sigma · u · bias[l]`, `u` a uniform draw in `[0, 1)` that is a pure function
    /// of `seed` and the link. `bias` is per link, in `[0, 1]` (`None` is 1 everywhere): 0
    /// leaves a link's cost alone, 1 lets it change by up to `sigma` times itself. Reported
    /// costs stay the **true** ones, and a bound ([`Self::shortest_within`]) is on them.
    ///
    /// Stays until [`Self::clear_noise`] or [`Self::clear_route_state`]. Costs one integer
    /// hash per link a search touches, nothing when not set.
    ///
    /// # Panics
    ///
    /// Panics (when a link is first costed) if `bias` has fewer entries than the network
    /// has links.
    pub fn set_noise(&mut self, seed: u64, sigma: f64, bias: Option<Arc<[f32]>>) {
        self.noise = Some(Noise { seed, sigma, bias });
    }

    /// Stop perturbing costs.
    pub fn clear_noise(&mut self) {
        self.noise = None;
    }

    /// A link's cost multiplier for the search now: the factors set, and the noise if any.
    #[inline]
    fn factor(&self, link: LinkId) -> f64 {
        let base = self.factors.get(link);
        match &self.noise {
            None => base,
            Some(noise) => base * noise.factor(link),
        }
    }

    /// The context this search runs over.
    #[must_use]
    pub fn context(&self) -> &'a SearchContext<'a> {
        self.ctx
    }

    /// The cheapest route from `origin` to `destination` under the current
    /// [`Self::factors`], or `None` if there is none. `Some` with no links if
    /// they are the same node.
    pub fn shortest(&mut self, origin: NodeId, destination: NodeId) -> Option<Route> {
        self.shortest_within(origin, destination, f64::INFINITY)
    }

    /// [`Self::shortest`], dropping every label whose unpenalised cost exceeds
    /// `max_true_cost`: the route returned, if any, costs at most that. See the
    /// module docs for what the bound trades.
    pub fn shortest_within(
        &mut self,
        origin: NodeId,
        destination: NodeId,
        max_true_cost: f64,
    ) -> Option<Route> {
        if origin == destination {
            return Some(Route { links: Vec::new(), cost: 0.0, overlap: 0.0 });
        }
        let ctx = self.ctx;
        for &l in ctx.network.out_links(origin) {
            let raw = ctx.link_cost(l);
            let c = raw * self.factor(l);
            if c.is_finite() && raw <= max_true_cost && c < self.dist[l.index()] {
                self.set(l.index(), c, raw, NONE);
                self.heap.push(Entry { cost: c, link: l.raw() });
            }
        }
        let mut found = None;
        while let Some(Entry { cost, link }) = self.heap.pop() {
            let l = LinkId::new(link);
            if cost > self.dist[l.index()] {
                continue; // stale: a cheaper way to this link was found later
            }
            if ctx.network.link_to(l) == destination {
                found = Some(link);
                break;
            }
            let true_cost = self.true_dist[l.index()];
            for &t in ctx.turns.turns_from(l) {
                let m = ctx.turns.outgoing(t);
                let raw = ctx.link_cost(m);
                if !raw.is_finite() || true_cost + raw > max_true_cost {
                    continue;
                }
                let nd = cost + raw * self.factor(m);
                if nd < self.dist[m.index()] {
                    self.set(m.index(), nd, true_cost + raw, link);
                    self.heap.push(Entry { cost: nd, link: m.raw() });
                }
            }
        }
        let route = found.map(|last| {
            let mut links = Vec::new();
            let mut at = last;
            while at != NONE {
                links.push(LinkId::new(at));
                at = self.pred[at as usize];
            }
            links.reverse();
            let cost = ctx.route_cost(&links);
            Route { links, cost, overlap: 0.0 }
        });
        self.clean();
        route
    }

    /// The least time to drive from `origin` to `destination` when setting out at
    /// second `departure`, if a link's time depends on when it is entered (S171): the
    /// earliest arrival, found by a search whose labels are arrival times.
    ///
    /// `wait(link, departure)` is the time spent outside the network before the first
    /// link `link` can be entered; `seconds(link, entered)` the time to cross a link
    /// entered at second `entered`. Links a car cannot use are never taken. **The
    /// search is bounded**: a label whose travel time already exceeds `bound` is
    /// dropped, so it explores only what could beat `bound` (a route already known:
    /// the cheapest of a choice set, say). `None` if nothing does. `Some(0)` if the
    /// two are one node.
    ///
    /// The earliest-arrival search is exact when a later entry never means an earlier
    /// exit (FIFO). Times read from time bins can step down at a bin's edge, so it can
    /// miss a route that exploits the step; the error is bounded by that step.
    pub fn fastest_time(
        &mut self,
        origin: NodeId,
        destination: NodeId,
        departure: f64,
        bound: f64,
        wait: &dyn Fn(u32, f64) -> f64,
        seconds: &dyn Fn(u32, f64) -> f64,
    ) -> Option<f64> {
        let found = self.earliest_arrival(origin, destination, departure, bound, wait, seconds);
        self.clean();
        found.map(|(time, _)| time)
    }

    /// [`Self::fastest_time`], and the route that takes that time: the links of the
    /// earliest-arrival route, in driving order (none if the two are one node).
    ///
    /// Same bound, same exactness. Where several routes take the least time the one
    /// returned is fixed by link ids (the heap breaks ties by them), so the same table of
    /// times always gives the same route. Costs a walk back along the predecessors, the
    /// route's length, on top of the search.
    pub fn fastest_route(
        &mut self,
        origin: NodeId,
        destination: NodeId,
        departure: f64,
        bound: f64,
        wait: &dyn Fn(u32, f64) -> f64,
        seconds: &dyn Fn(u32, f64) -> f64,
    ) -> Option<(f64, Vec<LinkId>)> {
        let found = self.earliest_arrival(origin, destination, departure, bound, wait, seconds);
        let route = found.map(|(time, last)| {
            let mut links = Vec::new();
            let mut at = last;
            while at != NONE {
                links.push(LinkId::new(at));
                at = self.pred[at as usize];
            }
            links.reverse();
            (time, links)
        });
        self.clean();
        route
    }

    /// The earliest-arrival search behind [`Self::fastest_time`] and
    /// [`Self::fastest_route`]: the travel time and the last link of the route found.
    /// **Leaves its labels in place** so the caller can read the predecessors, and must
    /// [`Self::clean`] them.
    fn earliest_arrival(
        &mut self,
        origin: NodeId,
        destination: NodeId,
        departure: f64,
        bound: f64,
        wait: &dyn Fn(u32, f64) -> f64,
        seconds: &dyn Fn(u32, f64) -> f64,
    ) -> Option<(f64, u32)> {
        if origin == destination {
            return Some((0.0, NONE));
        }
        let ctx = self.ctx;
        let limit = departure + bound;
        for &l in ctx.network.out_links(origin) {
            if !ctx.link_cost(l).is_finite() {
                continue;
            }
            let entered = departure + wait(l.raw(), departure);
            let arrive = entered + seconds(l.raw(), entered);
            if arrive <= limit && arrive < self.dist[l.index()] {
                self.set(l.index(), arrive, 0.0, NONE);
                self.heap.push(Entry { cost: arrive, link: l.raw() });
            }
        }
        let mut found = None;
        while let Some(Entry { cost, link }) = self.heap.pop() {
            let l = LinkId::new(link);
            if cost > self.dist[l.index()] {
                continue;
            }
            if ctx.network.link_to(l) == destination {
                found = Some((cost - departure, link));
                break;
            }
            for &t in ctx.turns.turns_from(l) {
                let m = ctx.turns.outgoing(t);
                if !ctx.link_cost(m).is_finite() {
                    continue;
                }
                let arrive = cost + seconds(m.raw(), cost);
                if arrive <= limit && arrive < self.dist[m.index()] {
                    self.set(m.index(), arrive, 0.0, link);
                    self.heap.push(Entry { cost: arrive, link: m.raw() });
                }
            }
        }
        found
    }

    fn set(&mut self, link: usize, dist: f64, true_dist: f64, pred: u32) {
        if self.dist[link].is_infinite() {
            self.touched.push(u32::try_from(link).expect("link ids are u32"));
        }
        self.dist[link] = dist;
        self.true_dist[link] = true_dist;
        self.pred[link] = pred;
    }

    fn clean(&mut self) {
        for &l in &self.touched {
            self.dist[l as usize] = f64::INFINITY;
            self.pred[l as usize] = NONE;
        }
        self.touched.clear();
        self.heap.clear();
    }

    /// Remember that `links` belong to route number `index` (below
    /// [`MAX_ROUTES_PER_SET`]) of the set being built.
    ///
    /// # Panics
    ///
    /// Panics if `index` is [`MAX_ROUTES_PER_SET`] or more.
    pub fn mark(&mut self, links: &[LinkId], index: usize) {
        assert!(
            index < MAX_ROUTES_PER_SET,
            "a route set holds at most {MAX_ROUTES_PER_SET} routes"
        );
        for &l in links {
            let m = &mut self.marks[l.index()];
            if *m == 0 {
                self.marked.push(l.raw());
            }
            *m |= 1 << index;
        }
    }

    /// The largest share of `links`' cost that it has in common with any one
    /// marked route.
    #[must_use]
    pub fn max_overlap(&self, links: &[LinkId]) -> f64 {
        let mut shared = [0.0f64; MAX_ROUTES_PER_SET];
        let mut total = 0.0;
        for &l in links {
            let c = self.ctx.link_cost(l);
            total += c;
            let mut m = self.marks[l.index()];
            while m != 0 {
                shared[m.trailing_zeros() as usize] += c;
                m &= m - 1;
            }
        }
        if total <= 0.0 {
            return 0.0;
        }
        shared.iter().fold(0.0f64, |a, &s| a.max(s / total))
    }

    /// Forget every mark, every factor and any noise, ready for the next key.
    pub fn clear_route_state(&mut self) {
        self.noise = None;
        for &l in &self.marked {
            self.marks[l as usize] = 0;
        }
        self.marked.clear();
        self.factors.reset();
    }
}
