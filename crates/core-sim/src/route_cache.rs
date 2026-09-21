//! A cache of generated route sets, for runs that repeat the same network and demand (S178).
//!
//! Generating route sets is the largest single cost of a run that iterates (S177: 48% of the best
//! method's time on Stockholm's inner city), and it is a **pure function of its inputs**: the
//! network, the origin–destination pairs, the method and its options, and, for a method that reads
//! the demand ([`RouteSetGenerator::reads_demand`]), the demand itself. A study that runs the same
//! scenario many times, changing only the equilibration, the choice model, the flow level or a
//! seed, generates the same sets each time. A [`RouteSetCache`] hands them back instead.
//!
//! **What is cached** is the store as the method made it, before any route update grew it: a run
//! that grows its sets does so on a copy, and the cached store is never changed. **The key** is a
//! hash of exactly the inputs above (see [`generation_key`]; the network's whole content, as the
//! run's fingerprint hashes it, not only its ids), so a hit is the store a miss would
//! have made, bit for bit; nothing about a run's results depends on whether the cache was used.
//!
//! **Cost:** a lookup is a hash and a lock; a store kept costs its size (4 bytes per link of every
//! route and 12 per route, about 17 MB at country-network scale), at most `capacity` of them, the
//! least recently used evicted first. A run without a cache pays nothing.

use std::sync::{Arc, Mutex};

use openmobisim_core_graph::link_geometry::NetworkFingerprint;
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_routes::{RouteKey, RouteSetGenerator, RouteSets, TripDemand};
use openmobisim_core_types::hash::Fnv1a;

/// How many stores a cache keeps unless it is asked for another number.
pub const DEFAULT_CAPACITY: usize = 4;

/// Generated route sets, kept for the next run that asks for the same. Shared by handle
/// (`Arc<RouteSetCache>`), safe to use from several runs at once.
#[derive(Debug)]
pub struct RouteSetCache {
    capacity: usize,
    inner: Mutex<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    /// Least recently used first.
    entries: Vec<(u64, Arc<RouteSets>)>,
    hits: u64,
    misses: u64,
}

/// What a cache has done so far.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CacheStats {
    /// Sets handed back without generating.
    pub hits: u64,
    /// Sets generated because they were not there.
    pub misses: u64,
    /// Stores held now.
    pub held: usize,
}

impl Default for RouteSetCache {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }
}

impl RouteSetCache {
    /// A cache keeping at most `capacity` stores (at least 1).
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self { capacity: capacity.max(1), inner: Mutex::new(Inner::default()) }
    }

    /// The store for `key`: the one held, or the one `generate` makes (which is then held).
    /// `generate` runs **outside** the lock, so runs that miss do not wait for one another; two
    /// that miss on the same key at once both generate, and the later store replaces the earlier
    /// with the same content.
    ///
    /// # Panics
    ///
    /// Panics if the lock is poisoned, that is, if another thread panicked while holding it.
    pub fn get_or_generate(
        &self,
        key: u64,
        generate: impl FnOnce() -> RouteSets,
    ) -> Arc<RouteSets> {
        {
            let mut inner = self.inner.lock().expect("the cache lock is not poisoned");
            if let Some(at) = inner.entries.iter().position(|(k, _)| *k == key) {
                let entry = inner.entries.remove(at);
                let sets = entry.1.clone();
                inner.entries.push(entry);
                inner.hits += 1;
                return sets;
            }
        }
        let sets = Arc::new(generate());
        let mut inner = self.inner.lock().expect("the cache lock is not poisoned");
        inner.misses += 1;
        inner.entries.retain(|(k, _)| *k != key);
        inner.entries.push((key, sets.clone()));
        while inner.entries.len() > self.capacity {
            inner.entries.remove(0);
        }
        sets
    }

    /// What the cache has done, and what it holds.
    ///
    /// # Panics
    ///
    /// Panics if the lock is poisoned.
    #[must_use]
    pub fn stats(&self) -> CacheStats {
        let inner = self.inner.lock().expect("the cache lock is not poisoned");
        CacheStats { hits: inner.hits, misses: inner.misses, held: inner.entries.len() }
    }

    /// Forget every store (the counters stay).
    ///
    /// # Panics
    ///
    /// Panics if the lock is poisoned.
    pub fn clear(&self) {
        self.inner.lock().expect("the cache lock is not poisoned").entries.clear();
    }
}

/// The key of the sets a run would generate: a hash of the network, the method with its options,
/// and what the method reads. A method that does **not** read the demand depends only on the
/// distinct pairs asked for (in any order); one that does depends on every trip's pair, weight and
/// departure.
#[must_use]
pub fn generation_key(
    network: &RoadNetwork,
    generator: &dyn RouteSetGenerator,
    trip_keys: &[RouteKey],
    demand: Option<&[TripDemand]>,
) -> u64 {
    let mut h = Fnv1a::new();
    h.write_str("openmobisim-route-sets");
    h.write_u32(openmobisim_core_types::CODE_VERSION);
    // The network's whole content, not only its ids: the same ids with other lengths or speeds
    // are other routes.
    crate::identity::hash_network(&mut h, network, NetworkFingerprint::of(network).value());
    h.write_str(generator.name());
    h.write_str(&generator.descriptor());
    if let Some(trips) = demand {
        h.write_bool(true);
        h.write_u32(u32::try_from(trips.len()).unwrap_or(u32::MAX));
        for t in trips {
            h.write_u32(t.key.origin);
            h.write_u32(t.key.destination);
            h.write_u32(t.weight);
            h.write_u32(t.departure);
        }
    } else {
        h.write_bool(false);
        let mut keys: Vec<RouteKey> =
            trip_keys.iter().copied().filter(|k| k.origin != k.destination).collect();
        keys.sort_unstable();
        keys.dedup();
        h.write_u32(u32::try_from(keys.len()).unwrap_or(u32::MAX));
        for k in keys {
            h.write_u32(k.origin);
            h.write_u32(k.destination);
        }
    }
    h.finish()
}
