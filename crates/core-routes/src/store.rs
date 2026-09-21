//! The route-set store: every route of every key, flat (design §13.1).
//!
//! **Layout.** Paths are one flat array of link ids with offsets (CSR); a key's
//! routes are a range of route numbers; per-route metadata (cost, overlap, a
//! cost stamp) lives in parallel arrays. Everything is plain arrays with no
//! pointers, so it can be written to disk and mapped back later without parsing.
//!
//! **Identity.** A store records the fingerprint of the network it was made
//! from and the [descriptor](crate::RouteSetGenerator::descriptor) of the
//! method that made it; [`RouteSets::identity`] combines them, so a set made
//! with other settings, or on another network, is never mistaken for this one.
//!
//! **Order.** A key's routes are the generator's, best first; a store that has
//! been [extended](RouteSets::extended) (S176: routes added between the iterations of a
//! run) has the added routes after them, in the order they were added, and
//! [`RouteSets::stamps`] says in which iteration each came. Nothing that already
//! has a number changes it except by a fixed shift per key.
//!
//! **Cost.** 4 bytes per link of every route, 12 bytes per route, and 12 bytes
//! per key: a 100-link route costs about 400 bytes.
//!
//! **Keys** are pairs of node ids today; when zones exist they will be pairs of
//! zone ids, and nothing about the layout changes.

use openmobisim_core_graph::link_geometry::NetworkFingerprint;
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_types::ids::{EntityId, NodeId};

use crate::generate::RouteSetGenerator;
use crate::search::{Route, Search, SearchContext};

#[cfg(feature = "parallel")]
use rayon::prelude::*;

/// Keys per unit of parallel work: fixed, so the split never depends on the
/// thread count and the result is the same for any.
const CHUNK: usize = 16;

/// Run `work` on every item of `items`, each thread with a [`Search`] of its own, and
/// return the results **in the order of the items**.
///
/// The items are split into chunks of a fixed size and the chunks joined in order, so
/// the result is the same for any number of threads, and the same without the
/// `parallel` feature, **provided `work` is a function of its item and the context
/// alone** (a `Search` leaves no state behind). This is what generates the sets of many
/// keys ([`RouteSets::generate`]) and what searches for the routes to add to them.
pub fn search_map<'a, T, R, F>(ctx: &'a SearchContext<'a>, items: &[T], work: F) -> Vec<R>
where
    T: Sync,
    R: Send,
    F: Fn(&mut Search<'a>, &T) -> R + Sync,
{
    #[cfg(feature = "parallel")]
    {
        let per_chunk: Vec<Vec<R>> = items
            .par_chunks(CHUNK)
            .map_init(
                || Search::new(ctx),
                |search, chunk| chunk.iter().map(|item| work(search, item)).collect(),
            )
            .collect();
        per_chunk.into_iter().flatten().collect()
    }
    #[cfg(not(feature = "parallel"))]
    {
        let mut search = Search::new(ctx);
        items.iter().map(|item| work(&mut search, item)).collect()
    }
}

/// What a set of routes is keyed by: an origin and a destination node.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct RouteKey {
    /// The origin node's id.
    pub origin: u32,
    /// The destination node's id.
    pub destination: u32,
}

impl RouteKey {
    /// The key for a pair of nodes.
    #[must_use]
    pub fn new(origin: NodeId, destination: NodeId) -> Self {
        Self { origin: origin.raw(), destination: destination.raw() }
    }
}

/// One route as the store holds it.
#[derive(Clone, Copy, Debug)]
pub struct RouteView<'a> {
    /// The links, as raw link ids, in driving order.
    pub links: &'a [u32],
    /// Free-flow travel time in seconds when the route was generated.
    pub cost: f32,
    /// The largest share of its cost shared with a route found before it.
    pub overlap: f32,
}

/// The routes of many keys.
#[derive(Clone, Debug, PartialEq)]
pub struct RouteSets {
    network: NetworkFingerprint,
    method: String,
    descriptor: String,
    /// The descriptor of the update that has grown the sets since they were generated
    /// (S176); empty for a store as its generator made it.
    update: String,
    keys: Vec<RouteKey>,
    /// `keys.len() + 1` offsets into the route arrays.
    set_start: Vec<u32>,
    /// `route_count + 1` offsets into `links`.
    route_start: Vec<u32>,
    links: Vec<u32>,
    cost: Vec<f32>,
    overlap: Vec<f32>,
    /// When each route's cost was taken: 0 is free flow at generation. A later
    /// cost update stamps the routes it re-costs, so a stale cost can be told
    /// from a fresh one. A route added to a grown store ([`RouteSets::extended`]) is
    /// stamped with the iteration that added it, and its cost is its free-flow cost.
    stamp: Vec<u32>,
}

impl RouteSets {
    /// Generate the routes for `keys` with `generator`.
    ///
    /// Keys are sorted and de-duplicated, and a key whose origin and
    /// destination are the same node is dropped (nothing to route). A key with
    /// **no route** is kept with an empty set, so the fact is not searched for
    /// again. **The result is a function of the network, the keys and the
    /// generator alone** — the same for any thread count and any order of
    /// `keys`.
    ///
    /// # Panics
    ///
    /// Panics if the links or the routes of the store exceed the `u32` id space,
    /// which needs billions of link entries.
    #[must_use]
    pub fn generate(
        network: &RoadNetwork,
        turns: &TurnTable,
        keys: &[RouteKey],
        generator: &dyn RouteSetGenerator,
    ) -> Self {
        let mut keys: Vec<RouteKey> =
            keys.iter().copied().filter(|k| k.origin != k.destination).collect();
        keys.sort_unstable();
        keys.dedup();

        let ctx = SearchContext::new(network, turns);
        let per_key: Vec<Vec<Route>> = search_map(&ctx, &keys, |search, k| {
            generator.generate(search, NodeId::new(k.origin), NodeId::new(k.destination))
        });

        let mut sets = Self {
            network: NetworkFingerprint::of(network),
            method: generator.name().to_string(),
            descriptor: generator.descriptor(),
            update: String::new(),
            keys,
            set_start: vec![0],
            route_start: vec![0],
            links: Vec::new(),
            cost: Vec::new(),
            overlap: Vec::new(),
            stamp: Vec::new(),
        };
        for routes in per_key {
            for route in routes {
                sets.links.extend(route.links.iter().map(|l| l.raw()));
                sets.route_start.push(u32::try_from(sets.links.len()).expect("links fit u32"));
                #[allow(clippy::cast_possible_truncation, reason = "f32 costs are the stored form")]
                {
                    sets.cost.push(route.cost as f32);
                    sets.overlap.push(route.overlap as f32);
                }
                sets.stamp.push(0);
            }
            sets.set_start.push(u32::try_from(sets.cost.len()).expect("routes fit u32"));
        }
        sets
    }

    /// These sets with routes added to some keys (S176: the routes a run finds between
    /// iterations, see `core-sim`'s `RouteUpdate`).
    ///
    /// `additions` are `(key index, routes)` in **strictly ascending key order**, one entry
    /// per key; a key's new routes go **after its existing ones**, in the order given, each
    /// stamped `stamp` (the iteration that adds them: not 0, which marks a route as the
    /// generator made it). `update` is the descriptor of whatever found them, kept apart
    /// from the generator's and part of the new store's [identity](Self::identity).
    ///
    /// Returns the new store and, per key, how many routes were added to the keys **before**
    /// it: route `r` of key `k` of *this* store is route `r + shift[k]` of the new one.
    /// Every route keeps its links, cost, overlap and stamp; the routes of a key that got
    /// nothing keep their order. **Cost:** one copy of the store (4 bytes per link and 12 per
    /// route), a few milliseconds at the size of a country's network.
    ///
    /// # Panics
    ///
    /// Panics if `additions` is not in strictly ascending key order or names a key the store
    /// does not have, or if the links or routes of the result exceed the `u32` id space.
    #[must_use]
    pub fn extended(
        &self,
        additions: &[(usize, Vec<Route>)],
        stamp: u32,
        update: &str,
    ) -> (Self, Vec<u32>) {
        assert!(
            additions.windows(2).all(|w| w[0].0 < w[1].0),
            "additions must be in strictly ascending key order, one entry per key"
        );
        assert!(
            additions.last().is_none_or(|a| a.0 < self.keys.len()),
            "an addition names a key the store does not have"
        );
        let added: usize = additions.iter().map(|(_, routes)| routes.len()).sum();
        let added_links: usize =
            additions.iter().flat_map(|(_, routes)| routes).map(|r| r.links.len()).sum();
        let mut out = Self {
            network: self.network,
            method: self.method.clone(),
            descriptor: self.descriptor.clone(),
            update: update.to_string(),
            keys: self.keys.clone(),
            set_start: Vec::with_capacity(self.set_start.len()),
            route_start: Vec::with_capacity(self.route_start.len() + added),
            links: Vec::with_capacity(self.links.len() + added_links),
            cost: Vec::with_capacity(self.cost.len() + added),
            overlap: Vec::with_capacity(self.cost.len() + added),
            stamp: Vec::with_capacity(self.cost.len() + added),
        };
        out.set_start.push(0);
        out.route_start.push(0);
        let mut shift = Vec::with_capacity(self.keys.len());
        let mut before = 0_u32;
        let mut next = additions.iter().peekable();
        for k in 0..self.keys.len() {
            shift.push(before);
            for r in self.route_range(k) {
                let view = self.route(r);
                out.links.extend_from_slice(view.links);
                out.route_start.push(u32::try_from(out.links.len()).expect("links fit u32"));
                out.cost.push(view.cost);
                out.overlap.push(view.overlap);
                out.stamp.push(self.stamp[r]);
            }
            if let Some((_, routes)) = next.next_if(|(key, _)| *key == k) {
                for route in routes {
                    out.links.extend(route.links.iter().map(|l| l.raw()));
                    out.route_start.push(u32::try_from(out.links.len()).expect("links fit u32"));
                    #[allow(
                        clippy::cast_possible_truncation,
                        reason = "f32 costs are the stored form"
                    )]
                    {
                        out.cost.push(route.cost as f32);
                        out.overlap.push(route.overlap as f32);
                    }
                    out.stamp.push(stamp);
                    before += 1;
                }
            }
            out.set_start.push(u32::try_from(out.cost.len()).expect("routes fit u32"));
        }
        (out, shift)
    }

    /// The name of the method that made this store.
    #[must_use]
    pub fn method(&self) -> &str {
        &self.method
    }

    /// The method's descriptor: its name and every option.
    #[must_use]
    pub fn descriptor(&self) -> &str {
        &self.descriptor
    }

    /// The descriptor of the update that grew these sets after they were generated
    /// ([`Self::extended`]): empty for a store as its method made it.
    #[must_use]
    pub fn update(&self) -> &str {
        &self.update
    }

    /// A number that is equal exactly when the network and the method with all
    /// its options are the same, and the sets have been grown by the same update if
    /// they have (FNV-1a over the fingerprint, descriptor and update).
    #[must_use]
    pub fn identity(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut feed = |bytes: &[u8]| {
            for &b in bytes {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x0100_0000_01b3);
            }
        };
        feed(&self.network.value().to_le_bytes());
        feed(self.descriptor.as_bytes());
        // A store nobody has grown hashes as it always did.
        if !self.update.is_empty() {
            feed(b"+update:");
            feed(self.update.as_bytes());
        }
        h
    }

    /// Whether this store was made from `network`.
    #[must_use]
    pub fn matches(&self, network: &RoadNetwork) -> bool {
        self.network == NetworkFingerprint::of(network)
    }

    /// The keys, sorted.
    #[must_use]
    pub fn keys(&self) -> &[RouteKey] {
        &self.keys
    }

    /// How many routes in all.
    #[must_use]
    pub fn route_count(&self) -> usize {
        self.cost.len()
    }

    /// The position of `key` among the keys, if it has a set.
    #[must_use]
    pub fn key_index(&self, key: RouteKey) -> Option<usize> {
        self.keys.binary_search(&key).ok()
    }

    /// The route numbers of the key at `key_index`.
    ///
    /// # Panics
    ///
    /// Panics if `key_index` is out of range.
    #[must_use]
    pub fn route_range(&self, key_index: usize) -> core::ops::Range<usize> {
        self.set_start[key_index] as usize..self.set_start[key_index + 1] as usize
    }

    /// Route number `route`.
    ///
    /// # Panics
    ///
    /// Panics if `route` is out of range.
    #[must_use]
    pub fn route(&self, route: usize) -> RouteView<'_> {
        RouteView {
            links: &self.links
                [self.route_start[route] as usize..self.route_start[route + 1] as usize],
            cost: self.cost[route],
            overlap: self.overlap[route],
        }
    }

    /// The routes of the key at `key_index`, best first.
    pub fn routes(&self, key_index: usize) -> impl Iterator<Item = RouteView<'_>> {
        self.route_range(key_index).map(|r| self.route(r))
    }

    /// The best route of `key`, if it has one.
    #[must_use]
    pub fn best(&self, key: RouteKey) -> Option<RouteView<'_>> {
        let i = self.key_index(key)?;
        let range = self.route_range(i);
        (!range.is_empty()).then(|| self.route(range.start))
    }

    /// The key that route number `route` belongs to.
    #[must_use]
    pub fn key_of_route(&self, route: usize) -> usize {
        self.set_start.partition_point(|&s| s as usize <= route) - 1
    }

    /// Every route's links, one flat array; route `r` is `links[route_start[r]..route_start[r + 1]]`.
    #[must_use]
    pub fn links(&self) -> &[u32] {
        &self.links
    }

    /// `route_count + 1` offsets into [`Self::links`].
    #[must_use]
    pub fn route_start(&self) -> &[u32] {
        &self.route_start
    }

    /// `keys + 1` offsets into the routes.
    #[must_use]
    pub fn set_start(&self) -> &[u32] {
        &self.set_start
    }

    /// Every route's cost in seconds.
    #[must_use]
    pub fn costs(&self) -> &[f32] {
        &self.cost
    }

    /// Every route's overlap with the routes found before it.
    #[must_use]
    pub fn overlaps(&self) -> &[f32] {
        &self.overlap
    }

    /// Every route's stamp: 0 for a route the generator made, otherwise the iteration
    /// that added it ([`Self::extended`]).
    #[must_use]
    pub fn stamps(&self) -> &[u32] {
        &self.stamp
    }

    /// The bytes this store holds.
    #[must_use]
    pub fn bytes(&self) -> usize {
        size_of::<RouteKey>() * self.keys.len()
            + 4 * (self.set_start.len() + self.route_start.len() + self.links.len())
            + 12 * self.cost.len()
    }

    /// The inverted index: for each link, the routes that use it.
    ///
    /// # Panics
    ///
    /// Panics if there are more routes than the `u32` id space holds.
    #[must_use]
    pub fn link_index(&self, link_count: u32) -> LinkIndex {
        let mut start = vec![0u32; link_count as usize + 1];
        for &l in &self.links {
            start[l as usize + 1] += 1;
        }
        for i in 1..start.len() {
            start[i] += start[i - 1];
        }
        let mut fill = start.clone();
        let mut routes = vec![0u32; self.links.len()];
        for r in 0..self.route_count() {
            for &l in &self.links[self.route_start[r] as usize..self.route_start[r + 1] as usize] {
                routes[fill[l as usize] as usize] = u32::try_from(r).expect("routes fit u32");
                fill[l as usize] += 1;
            }
        }
        LinkIndex { start, routes }
    }
}

/// For each link, the route numbers that use it (ascending).
#[derive(Clone, Debug, PartialEq)]
pub struct LinkIndex {
    start: Vec<u32>,
    routes: Vec<u32>,
}

impl LinkIndex {
    /// The routes that use `link`.
    ///
    /// # Panics
    ///
    /// Panics if `link` is out of range.
    #[must_use]
    pub fn routes_using(&self, link: u32) -> &[u32] {
        &self.routes[self.start[link as usize] as usize..self.start[link as usize + 1] as usize]
    }
}
