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
    /// from a fresh one.
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
        let work = |search: &mut Search<'_>, chunk: &[RouteKey]| -> Vec<Vec<Route>> {
            chunk
                .iter()
                .map(|k| {
                    generator.generate(search, NodeId::new(k.origin), NodeId::new(k.destination))
                })
                .collect()
        };
        #[cfg(feature = "parallel")]
        let per_chunk: Vec<Vec<Vec<Route>>> = keys
            .par_chunks(CHUNK)
            .map_init(|| Search::new(&ctx), |search, chunk| work(search, chunk))
            .collect();
        #[cfg(not(feature = "parallel"))]
        let per_chunk: Vec<Vec<Vec<Route>>> = {
            let mut search = Search::new(&ctx);
            keys.chunks(CHUNK).map(|chunk| work(&mut search, chunk)).collect()
        };

        let mut sets = Self {
            network: NetworkFingerprint::of(network),
            method: generator.name().to_string(),
            descriptor: generator.descriptor(),
            keys,
            set_start: vec![0],
            route_start: vec![0],
            links: Vec::new(),
            cost: Vec::new(),
            overlap: Vec::new(),
            stamp: Vec::new(),
        };
        for routes in per_chunk.into_iter().flatten() {
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

    /// A number that is equal exactly when the network and the method with all
    /// its options are the same (FNV-1a over the fingerprint and descriptor).
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

    /// Every route's cost stamp.
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
