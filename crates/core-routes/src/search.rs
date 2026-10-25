//! The turn-aware shortest-path search every generator is built on.
//!
//! A search runs over **links**, not nodes: a state is "having just entered
//! link `l`", and the next links are the turns the [`TurnTable`] allows from
//! it. Today the table only removes U-turns, so the costs equal a node-based
//! search's; when turn restrictions or turn delays arrive they drop in here
//! without touching a generator.
//!
//! **Costs** are free-flow travel times (control delay included) over
//! links that carry motor traffic; other links cost infinity. A bike or walk
//! layer searches its own costs instead ([`SearchContext::with_costs`]). A generator can
//! multiply any link's cost with [`LinkFactors`] for the next search (the
//! penalty method does), and asks [`Search::max_overlap`] how much of a route lies
//! on routes it has [marked](Search::mark).
//!
//! A search can be **bounded** ([`Search::shortest_within`]): a label whose true
//! (unpenalised) cost already exceeds a bound is dropped, since no way of
//! finishing it can come back under. The penalty method uses this with its
//! detour bound; without it, compounding penalties send the search far past
//! the routes it would accept (a hundredfold cost on a country-sized network,
//! measured). The bound is a resource limit on a constrained search:
//! it never lets a route past the bound through, and on rare occasions can miss
//! one that a fuller search would have found.
//!
//! A generator can also perturb every link's cost at random for the next search
//! ([`Search::set_noise`]): each link's factor is `1 + sigma · u · bias`, `u` a uniform draw
//! that is a **pure function of (seed, link)** — a few integer operations, no generator to
//! carry, no array to fill — so a search is the same for any thread and order. The Monte
//! Carlo method uses it.
//!
//! A generator can also give a search **extras** ([`Search::set_extras`], made by
//! [`SearchContext::extras`]): a static extra cost per link, in seconds, that a user class
//! values beyond travel time (length, money, user link values), so that the search minimises
//! a **generalised cost**, time plus extras. The route's reported `cost` stays its travel time;
//! [`Search::generalised`] gives the generalised cost of any route. A bound is then on the
//! unperturbed generalised cost, and the A\* bound below is rebuilt from the extras.
//!
//! **Goal-directed (A\*):** a search towards one destination
//! ([`Search::shortest_within`]) expands labels in order of their cost plus a
//! lower bound on the rest of the way: the straight-line distance to the
//! destination times the least cost per metre of straight line any link of the
//! context has ([`SearchContext::lower_bound_per_metre`]). The bound never
//! exceeds the true remaining cost and is consistent, so the route found is a
//! least-cost one, as Dijkstra's is (Hart, Nilsson & Raphael 1968); the search
//! only explores less (bike and walk routes ≈ 2.4× fewer seconds on Amsterdam,
//! car routes ≈ 1.7×). Where several routes have the same least cost, the
//! one returned can differ from plain Dijkstra's: the same cost, another of the
//! equal routes. It holds because every cost factor is at least 1
//! ([`LinkFactors::multiply`], the noise of [`Search::set_noise`]).
//!
//! **Cost of a search:** one label per touched link; the scratch is reset by
//! visiting only what was touched, so a search costs what it explores, not the
//! size of the network. **Scratch per thread:** 32 bytes per link (the cost, the true
//! cost, the predecessor, the marks and the factors) plus the heap.
//!
//! **Determinism:** ties are broken by link id, so a search never depends on
//! heap insertion order.

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
    /// The least cost per metre of straight line over every usable link (A\*'s bound).
    per_metre: f64,
}

/// The least cost per metre of straight-line distance between a link's ends, over
/// every link with a finite cost and ends apart; 0 (no bound: plain Dijkstra) if
/// there is none or a cost is 0. A path's cost is at least this times the straight
/// line from its start to its end, by the triangle inequality in the projected
/// plane, so the bound needs nothing but the costs and the node positions. Shrunk by
/// one part in a million against rounding.
fn lower_bound_per_metre(network: &RoadNetwork, cost: &[f64]) -> f64 {
    let mut least = f64::INFINITY;
    for (i, &c) in cost.iter().enumerate() {
        if !c.is_finite() {
            continue;
        }
        let link = LinkId::new(u32::try_from(i).expect("link ids are u32"));
        let a = network.node_position(network.link_from(link));
        let b = network.node_position(network.link_to(link));
        let chord = (a.x - b.x).hypot(a.y - b.y);
        if chord > 0.0 {
            least = least.min(c / chord);
        }
    }
    if least.is_finite() && least > 0.0 { least * (1.0 - 1e-6) } else { 0.0 }
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
            .collect::<Vec<f64>>();
        let per_metre = lower_bound_per_metre(network, &cost);
        Self { network, turns, cost, per_metre }
    }

    /// A search over `cost` instead of the car's free-flow times: the
    /// bike and walk layers' costs, one per link of `network`, infinite for a
    /// link that must not be used.
    ///
    /// # Panics
    ///
    /// Panics if `cost` does not have one entry per link.
    #[must_use]
    pub fn with_costs(network: &'a RoadNetwork, turns: &'a TurnTable, cost: Vec<f64>) -> Self {
        assert_eq!(cost.len(), network.link_count() as usize, "one cost per link");
        let per_metre = lower_bound_per_metre(network, &cost);
        Self { network, turns, cost, per_metre }
    }

    /// The least cost per metre of straight-line distance over every usable link:
    /// the factor A\* turns a straight-line distance into a lower bound on cost
    /// (see the [module docs](self)). 0 where no bound holds.
    #[must_use]
    pub fn lower_bound_per_metre(&self) -> f64 {
        self.per_metre
    }

    /// Extras for searches over this context: `seconds[l]` is what link `l` costs beyond its
    /// travel time, in seconds, by link id. A link's generalised cost, time plus extra, must not
    /// be negative (the composer floors it): a negative cost breaks every shortest-path search.
    /// Works out the A\* bound over the generalised costs.
    ///
    /// # Panics
    ///
    /// Panics if `seconds` does not have one entry per link, or if a usable link's time plus
    /// extra is negative or not a number.
    #[must_use]
    pub fn extras(&self, seconds: Vec<f64>) -> Extras {
        assert_eq!(seconds.len(), self.cost.len(), "one extra per link");
        let combined: Vec<f64> = self
            .cost
            .iter()
            .zip(&seconds)
            .enumerate()
            .map(|(i, (&c, &e))| {
                if c.is_finite() {
                    let g = c + e;
                    assert!(g.is_finite() && g >= 0.0, "link {i}: time {c} plus extra {e} is {g}");
                    g
                } else {
                    f64::INFINITY
                }
            })
            .collect();
        let per_metre = lower_bound_per_metre(self.network, &combined);
        let max_ratio = self
            .cost
            .iter()
            .zip(&seconds)
            .filter(|(c, _)| c.is_finite() && **c > 0.0)
            .map(|(c, e)| e / c)
            .fold(0.0_f64, f64::max);
        Extras { seconds: seconds.into(), per_metre, max_ratio }
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

    /// Multiply `link`'s cost by `by` from now on. `by` is at least 1: a factor
    /// below 1 would let a search's lower bound overestimate (see the module docs).
    ///
    /// # Panics
    ///
    /// In debug builds, if `by` is below 1.
    pub fn multiply(&mut self, link: LinkId, by: f64) {
        debug_assert!(by >= 1.0, "a cost factor must be at least 1, got {by}");
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

/// Static extra cost per link, in seconds: what a user class values beyond travel time
/// (length, money, user link values), unperturbed. Made by [`SearchContext::extras`], which
/// checks it and works out the A\* bound a search with it needs; set on a search with
/// [`Search::set_extras`]. Shared by handle: 8 bytes per link once, however many searches.
#[derive(Clone, Debug)]
pub struct Extras {
    seconds: Arc<[f64]>,
    /// The least generalised cost per metre of straight line over every usable link.
    per_metre: f64,
    /// The largest extra over any usable link's time: a route's generalised cost is at most
    /// `1 + max_ratio` times its time.
    max_ratio: f64,
}

impl Extras {
    /// The extra seconds of each link, by link id.
    #[must_use]
    pub fn seconds(&self) -> &[f64] {
        &self.seconds
    }

    /// The least generalised cost (time plus extra) per metre of straight-line distance, the
    /// A\* bound's factor; 0 where no bound holds.
    #[must_use]
    pub fn per_metre(&self) -> f64 {
        self.per_metre
    }

    /// The largest extra over any usable link's time (0 or more): a route's generalised cost
    /// is at most `1 + max_ratio` times its travel time, which bounds a search by generalised
    /// cost for routes within a detour in time.
    #[must_use]
    pub fn max_ratio(&self) -> f64 {
        self.max_ratio
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

/// Where an earliest-arrival search starts: every link out of a node, or the turns out of a link.
#[derive(Clone, Copy)]
enum Start {
    Node(NodeId),
    AfterLink(LinkId),
}

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
    /// Extra cost per link for the next searches, if a generator gave them.
    extras: Option<Extras>,
    /// The clock time at each label, for the time-dependent generalised-cost tree.
    time: Vec<f64>,
    marks: Vec<u32>,
    marked: Vec<u32>,
}

impl<'a> Search<'a> {
    /// A fresh scratch over `ctx`: 32 bytes per link.
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
            extras: None,
            time: vec![0.0; n],
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
        if let Some(b) = &bias {
            let links = self.ctx.network.link_count() as usize;
            assert_eq!(b.len(), links, "a bias has one value per link of the network (X-40)");
        }
        self.noise = Some(Noise { seed, sigma, bias });
    }

    /// Stop perturbing costs.
    pub fn clear_noise(&mut self) {
        self.noise = None;
    }

    /// Search the next routes by generalised cost: each link's time plus `extras`' seconds for
    /// it (see [`SearchContext::extras`]). Bounds ([`Self::shortest_within`],
    /// [`Self::shortest_tree`], [`Self::cheapest_routes_to`]) are then on the unperturbed
    /// generalised cost; a route's reported `cost` stays its travel time. Stays until
    /// [`Self::clear_extras`] or [`Self::clear_route_state`]. The earliest-arrival searches
    /// ([`Self::fastest_time`] and its kin) ignore extras: they are searches for time.
    ///
    /// # Panics
    ///
    /// Panics if the extras do not have one value per link of the network.
    pub fn set_extras(&mut self, extras: Option<Extras>) {
        if let Some(e) = &extras {
            let links = self.ctx.network.link_count() as usize;
            assert_eq!(e.seconds.len(), links, "extras have one value per link of the network");
        }
        self.extras = extras;
    }

    /// Search by time alone again.
    pub fn clear_extras(&mut self) {
        self.extras = None;
    }

    /// The generalised cost of `links` under the extras set: their time plus their extras,
    /// unperturbed; their time alone if none are set.
    #[must_use]
    pub fn generalised(&self, links: &[LinkId]) -> f64 {
        links.iter().map(|&l| self.raw(l)).sum()
    }

    /// A link's extra seconds under the extras set, 0 if none are.
    #[inline]
    fn extra(&self, link: LinkId) -> f64 {
        self.extras.as_ref().map_or(0.0, |e| e.seconds[link.index()])
    }

    /// A link's unperturbed generalised cost: its time plus its extra.
    #[inline]
    fn raw(&self, link: LinkId) -> f64 {
        self.ctx.link_cost(link) + self.extra(link)
    }

    /// The A\* bound's factor for the costs searched now: the extras' if set, the context's.
    #[inline]
    fn per_metre(&self) -> f64 {
        self.extras.as_ref().map_or(self.ctx.per_metre, |e| e.per_metre)
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
    /// module docs for what the bound trades. Under extras ([`Self::set_extras`]) the cost
    /// searched and bounded is the generalised one; the route's `cost` is still its time.
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
        // A*: the heap holds cost so far plus a lower bound on the rest (S209); with no
        // bound (0) it is Dijkstra exactly.
        let target = ctx.network.node_position(destination);
        let k = self.per_metre();
        let rest = |l: LinkId| {
            if k > 0.0 {
                let p = ctx.network.node_position(ctx.network.link_to(l));
                (p.x - target.x).hypot(p.y - target.y) * k
            } else {
                0.0
            }
        };
        for &l in ctx.network.out_links(origin) {
            let raw = self.raw(l);
            let c = raw * self.factor(l);
            let h = rest(l);
            if c.is_finite() && raw + h <= max_true_cost && c < self.dist[l.index()] {
                self.set(l.index(), c, raw, NONE);
                self.heap.push(Entry { cost: c + h, link: l.raw() });
            }
        }
        let mut found = None;
        while let Some(Entry { cost: priority, link }) = self.heap.pop() {
            let l = LinkId::new(link);
            let cost = self.dist[l.index()];
            if priority > cost + rest(l) {
                continue; // stale: a cheaper way to this link was found later
            }
            if ctx.network.link_to(l) == destination {
                found = Some(link);
                break;
            }
            let true_cost = self.true_dist[l.index()];
            for &t in ctx.turns.turns_from(l) {
                let m = ctx.turns.outgoing(t);
                let raw = self.raw(m);
                if !raw.is_finite() {
                    continue;
                }
                // The bound counts the rest of the way too: no route through `m` can come
                // back under it, since the lower bound never exceeds the true cost left.
                let h = rest(m);
                if true_cost + raw + h > max_true_cost {
                    continue;
                }
                let nd = cost + raw * self.factor(m);
                if nd < self.dist[m.index()] {
                    self.set(m.index(), nd, true_cost + raw, link);
                    self.heap.push(Entry { cost: nd + h, link: m.raw() });
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

    /// The least-cost route from `origin` to each of `destinations` (distinct nodes, none of
    /// them `origin`), in their order, if it costs at most `max_true_cost` unpenalised: one
    /// search for all of them, plain Dijkstra, which stops once it has reached them all.
    /// Each route is a least-cost one, as [`Self::shortest_within`]'s is; where several routes
    /// have the same least cost, it can be another of them.
    pub fn shortest_tree(
        &mut self,
        origin: NodeId,
        destinations: &[NodeId],
        max_true_cost: f64,
    ) -> Vec<Option<Route>> {
        let ctx = self.ctx;
        let mut wanted = vec![false; ctx.network.node_count() as usize];
        for d in destinations {
            wanted[d.index()] = true;
        }
        // The last link of the route to each destination reached, by node.
        let mut last: std::collections::HashMap<u32, u32> =
            std::collections::HashMap::with_capacity(destinations.len());
        for &l in ctx.network.out_links(origin) {
            let raw = self.raw(l);
            let c = raw * self.factor(l);
            if c.is_finite() && raw <= max_true_cost && c < self.dist[l.index()] {
                self.set(l.index(), c, raw, NONE);
                self.heap.push(Entry { cost: c, link: l.raw() });
            }
        }
        while last.len() < destinations.len() {
            let Some(Entry { cost, link }) = self.heap.pop() else { break };
            let l = LinkId::new(link);
            if cost > self.dist[l.index()] {
                continue; // stale: a cheaper way to this link was found later
            }
            let to = ctx.network.link_to(l);
            if wanted[to.index()] {
                last.entry(to.raw()).or_insert(link);
            }
            let true_cost = self.true_dist[l.index()];
            for &t in ctx.turns.turns_from(l) {
                let m = ctx.turns.outgoing(t);
                let raw = self.raw(m);
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
        let routes = destinations
            .iter()
            .map(|d| {
                let &end = last.get(&d.raw())?;
                let mut links = Vec::new();
                let mut at = end;
                while at != NONE {
                    links.push(LinkId::new(at));
                    at = self.pred[at as usize];
                }
                links.reverse();
                let cost = ctx.route_cost(&links);
                Some(Route { links, cost, overlap: 0.0 })
            })
            .collect();
        self.clean();
        routes
    }

    /// The least time to drive from `origin` to `destination` when setting out at
    /// second `departure`, if a link's time depends on when it is entered: the
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
    ///
    /// **`seconds(link, t)` must never be below the context's cost of `link`** (its
    /// free-flow time, for a car context): the search is goal-directed with a lower
    /// bound built from those costs (A\*, the [module docs](self)), and a faster time
    /// would let it miss the fastest route. A loading's link times meet this, since no
    /// vehicle crosses a link faster than free flow.
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

    /// Every node marked in `targets` that can be reached from `origin` leaving at
    /// `departure` within `bound` seconds: its earliest arrival, in seconds after
    /// `departure`, and the route that takes it, in the order they are reached (M4:
    /// the parkings around a park-and-ride trip's origin, in one search). `origin`
    /// itself, if marked, comes first with no links. The search stops once
    /// `wanted` targets are found or nothing within the bound is left.
    ///
    /// Same costs, turns and exactness as [`Self::fastest_route`]; one search
    /// instead of one per target.
    ///
    /// # Panics
    ///
    /// Panics if `targets` has fewer entries than the network has nodes.
    #[allow(
        clippy::too_many_arguments,
        reason = "the time-dependent search's own arguments, plus the targets"
    )]
    pub fn fastest_routes_to(
        &mut self,
        origin: NodeId,
        targets: &[bool],
        wanted: usize,
        departure: f64,
        bound: f64,
        wait: &dyn Fn(u32, f64) -> f64,
        seconds: &dyn Fn(u32, f64) -> f64,
    ) -> Vec<(NodeId, f64, Vec<LinkId>)> {
        let ctx = self.ctx;
        let mut found: Vec<(NodeId, f64, u32)> = Vec::new();
        let mut seen: Vec<u32> = Vec::new();
        if targets[origin.index()] {
            found.push((origin, 0.0, NONE));
            seen.push(origin.raw());
        }
        let limit = departure + bound;
        if found.len() < wanted {
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
        }
        while found.len() < wanted {
            let Some(Entry { cost, link }) = self.heap.pop() else { break };
            let l = LinkId::new(link);
            if cost > self.dist[l.index()] {
                continue;
            }
            let to = ctx.network.link_to(l);
            if targets[to.index()] && !seen.contains(&to.raw()) {
                seen.push(to.raw());
                found.push((to, cost - departure, link));
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
        let out = found
            .into_iter()
            .map(|(node, time, last)| {
                let mut links = Vec::new();
                let mut at = last;
                while at != NONE {
                    links.push(LinkId::new(at));
                    at = self.pred[at as usize];
                }
                links.reverse();
                (node, time, links)
            })
            .collect();
        self.clean();
        out
    }

    /// [`Self::fastest_routes_to`] by **generalised cost**: every node marked in `targets`
    /// reachable from `origin` leaving at `departure`, with its unperturbed generalised cost
    /// (the seconds driven, the wait before the first link included, plus the extras of its
    /// links), its travel time in seconds after `departure`, and its route, in the order found
    /// (cheapest first by the perturbed cost). The search minimises
    /// `factor(l) · (seconds(l, entered) + extra(l))` over the links, each link's time read at
    /// the clock time its label reaches it, under the factors, noise and extras set
    /// ([`Self::set_noise`], [`Self::set_extras`]); with none of them set it is
    /// [`Self::fastest_routes_to`] in all but the rounding of sums. **Bounded** on the
    /// unperturbed generalised cost: a label past `bound` is dropped. `origin` itself, if
    /// marked, comes first at no cost. Stops once `wanted` targets are found.
    ///
    /// Exact when every link's generalised cost is non-negative ([`SearchContext::extras`]
    /// checks that) and a later entry never means an earlier exit (FIFO), as a loading's times
    /// are; the clock time at a node is that of its least-cost label, the usual approximation
    /// of time-dependent generalised-cost routing.
    ///
    /// # Panics
    ///
    /// Panics if `targets` has fewer entries than the network has nodes.
    #[allow(
        clippy::too_many_arguments,
        reason = "the time-dependent search's own arguments, plus the targets"
    )]
    pub fn cheapest_routes_to(
        &mut self,
        origin: NodeId,
        targets: &[bool],
        wanted: usize,
        departure: f64,
        bound: f64,
        wait: &dyn Fn(u32, f64) -> f64,
        seconds: &dyn Fn(u32, f64) -> f64,
    ) -> Vec<(NodeId, f64, f64, Vec<LinkId>)> {
        let ctx = self.ctx;
        let mut found: Vec<(NodeId, f64, f64, u32)> = Vec::new();
        let mut seen: Vec<u32> = Vec::new();
        if targets[origin.index()] {
            found.push((origin, 0.0, 0.0, NONE));
            seen.push(origin.raw());
        }
        if found.len() < wanted {
            for &l in ctx.network.out_links(origin) {
                if !ctx.link_cost(l).is_finite() {
                    continue;
                }
                let waited = wait(l.raw(), departure);
                let entered = departure + waited;
                let s = seconds(l.raw(), entered);
                let step = s + self.extra(l);
                // The wait is spent outside the network: unperturbed, in time and in cost.
                let true_cost = waited + step;
                if true_cost > bound {
                    continue;
                }
                let cost = waited + step * self.factor(l);
                if cost < self.dist[l.index()] {
                    self.set(l.index(), cost, true_cost, NONE);
                    self.time[l.index()] = entered + s;
                    self.heap.push(Entry { cost, link: l.raw() });
                }
            }
        }
        while found.len() < wanted {
            let Some(Entry { cost, link }) = self.heap.pop() else { break };
            let l = LinkId::new(link);
            if cost > self.dist[l.index()] {
                continue;
            }
            let to = ctx.network.link_to(l);
            let (at, true_so_far) = (self.time[l.index()], self.true_dist[l.index()]);
            if targets[to.index()] && !seen.contains(&to.raw()) {
                seen.push(to.raw());
                found.push((to, true_so_far, at - departure, link));
            }
            for &t in ctx.turns.turns_from(l) {
                let m = ctx.turns.outgoing(t);
                if !ctx.link_cost(m).is_finite() {
                    continue;
                }
                let s = seconds(m.raw(), at);
                let step = s + self.extra(m);
                let true_cost = true_so_far + step;
                if true_cost > bound {
                    continue;
                }
                let nd = cost + step * self.factor(m);
                if nd < self.dist[m.index()] {
                    self.set(m.index(), nd, true_cost, link);
                    self.time[m.index()] = at + s;
                    self.heap.push(Entry { cost: nd, link: m.raw() });
                }
            }
        }
        let out = found
            .into_iter()
            .map(|(node, generalised, time, last)| {
                let mut links = Vec::new();
                let mut at = last;
                while at != NONE {
                    links.push(LinkId::new(at));
                    at = self.pred[at as usize];
                }
                links.reverse();
                (node, generalised, time, links)
            })
            .collect();
        self.clean();
        out
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
        self.earliest_arrival_from(
            Start::Node(origin),
            destination,
            departure,
            bound,
            wait,
            seconds,
        )
    }

    /// The route after `link` — beginning with one of its legal turns, at second `departure` —
    /// that reaches `destination` earliest, and that time, in seconds after `departure`: where
    /// a vehicle at the end of `link` can go from there (en-route rerouting). Same costs,
    /// bound, A\* and exactness as [`Self::fastest_route`]. `Some` with no links if `link`
    /// already ends at `destination`.
    pub fn fastest_route_after(
        &mut self,
        link: LinkId,
        destination: NodeId,
        departure: f64,
        bound: f64,
        seconds: &dyn Fn(u32, f64) -> f64,
    ) -> Option<(f64, Vec<LinkId>)> {
        let found = self.earliest_arrival_from(
            Start::AfterLink(link),
            destination,
            departure,
            bound,
            &|_, _| 0.0,
            seconds,
        );
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

    fn earliest_arrival_from(
        &mut self,
        start: Start,
        destination: NodeId,
        departure: f64,
        bound: f64,
        wait: &dyn Fn(u32, f64) -> f64,
        seconds: &dyn Fn(u32, f64) -> f64,
    ) -> Option<(f64, u32)> {
        let ctx = self.ctx;
        let start_node = match start {
            Start::Node(n) => n,
            Start::AfterLink(l) => ctx.network.link_to(l),
        };
        if start_node == destination {
            return Some((0.0, NONE));
        }
        let limit = departure + bound;
        // A* (S209): a link's time is never below its free-flow time (the loading lets no
        // vehicle cross faster), so the context's free-flow bound per metre of straight line
        // is a lower bound on the rest of the way here too. A label that cannot reach the
        // destination within the bound even at that rate is dropped.
        let target = ctx.network.node_position(destination);
        let k = ctx.per_metre;
        let rest = |l: LinkId| {
            if k > 0.0 {
                let p = ctx.network.node_position(ctx.network.link_to(l));
                (p.x - target.x).hypot(p.y - target.y) * k
            } else {
                0.0
            }
        };
        let seeds: Vec<LinkId> = match start {
            Start::Node(n) => ctx.network.out_links(n).to_vec(),
            Start::AfterLink(from) => {
                ctx.turns.turns_from(from).iter().map(|&t| ctx.turns.outgoing(t)).collect()
            }
        };
        for l in seeds {
            if !ctx.link_cost(l).is_finite() {
                continue;
            }
            let entered = departure + wait(l.raw(), departure);
            let arrive = entered + seconds(l.raw(), entered);
            let h = rest(l);
            if arrive + h <= limit && arrive < self.dist[l.index()] {
                self.set(l.index(), arrive, 0.0, NONE);
                self.heap.push(Entry { cost: arrive + h, link: l.raw() });
            }
        }
        let mut found = None;
        while let Some(Entry { cost: priority, link }) = self.heap.pop() {
            let l = LinkId::new(link);
            let cost = self.dist[l.index()];
            if priority > cost + rest(l) {
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
                let h = rest(m);
                if arrive + h <= limit && arrive < self.dist[m.index()] {
                    self.set(m.index(), arrive, 0.0, link);
                    self.heap.push(Entry { cost: arrive + h, link: m.raw() });
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
        self.extras = None;
        for &l in &self.marked {
            self.marks[l as usize] = 0;
        }
        self.marked.clear();
        self.factors.reset();
    }
}
