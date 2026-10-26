//! Shared search trees (S294): **one bounded time-dependent search per origin node, departure
//! bin and coefficient vector**, on one loading's link times, read by the route update and by
//! the gap alike. A tree of a vector with extras (the run's route cost, S299,
//! [`crate::generalised_cost::RouteCost`]) finds least-cost routes by that vector's generalised
//! cost; a run whose utility weighs time alone has one vector, by time, and the trees below are
//! by time.
//!
//! The route update ([`crate::route_update`]) asks, for every origin–destination pair, whether
//! the network holds a route faster than the pair's set at the congested times; the gap
//! ([`crate::gap`]) asks, for every trip, how much better than the route it took the network's
//! best would have been. Searched trip by trip, the second question would cost more than the
//! loading itself on a city. Both questions are answered at once
//! by one search tree per origin node and departure bin: the trips of one origin whose
//! departures fall in one bin share one [`openmobisim_core_routes::Search::fastest_routes_to`], which finds the fastest
//! route to each of their destinations at the bin's middle, bounded by the slowest of their
//! sets' best times (nothing beyond it can beat a set). The number of searches is then bounded by
//! the origins and bins with trips, not by the trips.
//!
//! **The aggregation.** A tree's routes are the fastest at the bin's middle, not at each trip's own
//! second; a route read for a trip is then re-costed at the trip's own departure before it is
//! compared with the trip's set. The error is the route choice's, not the time's: a route that is
//! fastest at the bin's middle may be second at a trip's own second. The bin is the equilibration's
//! cost bin (`cost_bin_s`, 300 s by default), the unit the whole run costs routes in.
//!
//! **Determinism.** The trees are built in a fixed order (origin, then bin), searched in parallel
//! in fixed chunks ([`search_map`]) and read by position, so the result is the same for any thread
//! count and any order of trips.

use std::sync::Arc;

use openmobisim_core_demand::Trips;
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_routes::{
    ClassExtras, Extras, RouteKey, RouteSets, SearchContext, search_map,
};
use openmobisim_core_types::ids::{EntityId, LinkId, NodeId, TripId};

use crate::generalised_cost::RouteCost;
use crate::link_times::LinkTimes;

/// A route one tree found: the cheapest from the tree's origin to `destination` at the tree's
/// departure, in the tree's cost (the fastest, in a tree searched by time).
#[derive(Clone, Debug, PartialEq)]
pub struct FoundRoute {
    /// The destination node, raw.
    pub destination: u32,
    /// Its travel time at the tree's departure, in seconds.
    pub seconds: f64,
    /// Its cost in the tree's currency, in seconds: its travel time in a tree searched by
    /// time; its travel time plus its links' extra seconds, unperturbed, in a tree searched by
    /// generalised cost. The keys' best ([`Tree::keys`]) are in the same currency.
    pub cost: f64,
    /// Its links, raw, in driving order.
    pub links: Vec<u32>,
}

/// One tree: an origin node and a departure bin, the pairs it serves and the routes it found.
#[derive(Clone, Debug, PartialEq)]
pub struct Tree {
    /// The origin node, raw.
    pub origin: u32,
    /// The departure the tree was searched at: the middle of its bin, in seconds.
    pub departure: f64,
    /// The pairs served: each key's index in the sets and the best time its set offers at
    /// [`Self::departure`], ascending by key.
    pub keys: Vec<(usize, f64)>,
    /// The routes found, ascending by destination; at most one per destination.
    pub routes: Vec<FoundRoute>,
}

impl Tree {
    /// The route found to `destination`, if any.
    #[must_use]
    pub fn route_to(&self, destination: u32) -> Option<&FoundRoute> {
        self.routes
            .binary_search_by_key(&destination, |r| r.destination)
            .ok()
            .map(|i| &self.routes[i])
    }
}

/// What one tree's search returns: the keys served with their best times, and the routes found.
type Searched = (Vec<(usize, f64)>, Vec<FoundRoute>);

/// What one of [`SearchTrees`]' searches returns: a [`Searched`] and the drive it answered with
/// it, if any.
type SearchedWithDrive = (Vec<(usize, f64)>, Vec<FoundRoute>, Option<TreeDrive>);

/// A drive one tree's search answered beside its pairs ([`SearchTrees::build_with_drives`]):
/// the tree's (origin, bin), the nodes sought (ascending) and the route to each of them reached
/// within the drive's bound, ascending by node (the origin itself, if sought, with no links).
pub type TreeDrive = (DriveKey, Box<[u32]>, Box<[(u32, Box<[LinkId]>)]>);

/// A shared drive's key: the road node it leaves, its step of the day, and the coefficient
/// vector it is searched by (0 where the run's route cost is time alone; S299).
pub type DriveKey = (u32, u64, u32);

/// Drives to search with the trees: per (origin node, bin) the nodes sought, and the bound of
/// every drive in seconds ([`SearchTrees::build_with_drives`]).
pub type DrivesWanted<'w> = (&'w [(DriveKey, Vec<NodeId>)], f64);

/// What the trees' searches minimise (S299): the run's route cost, and each trip's user class
/// index by trip (its vector is its class's).
#[derive(Clone, Copy, Debug)]
pub struct TreeCost<'a> {
    /// The run's route cost.
    pub cost: &'a RouteCost,
    /// Each trip's user class index, by trip.
    pub trip_class: &'a [u32],
}

/// The trees of one loading: see the [module docs](self).
#[derive(Clone, Debug, PartialEq)]
pub struct SearchTrees {
    step_s: f64,
    /// Each tree's (origin, bin, coefficient vector), ascending, in step with [`Self::trees`].
    heads: Vec<(u32, u64, u32)>,
    trees: Vec<Tree>,
    /// How many pairs the trees searched for, in all.
    pairs: u32,
}

impl SearchTrees {
    /// No trees: what a run with no next choice builds.
    #[must_use]
    pub fn empty(step_s: f64) -> Self {
        Self { step_s, heads: Vec::new(), trees: Vec::new(), pairs: 0 }
    }

    /// The trees for the trips of `trips` whose pair (`trip_keys`, by trip) has routes in `sets`
    /// and whose origin is not its destination, those `active` says if given, at the `times` of
    /// the last loading, with departures binned in steps of `step_s` seconds (at least 1).
    ///
    /// # Panics
    ///
    /// Panics if `step_s` is not positive.
    #[must_use]
    #[allow(clippy::too_many_arguments, reason = "the loading's inputs, the trips and the bin")]
    pub fn build(
        network: &RoadNetwork,
        turns: &TurnTable,
        times: &LinkTimes,
        trips: &Trips,
        trip_keys: &[RouteKey],
        sets: &RouteSets,
        active: Option<&[bool]>,
        car_start_s: u32,
        step_s: f64,
        cost: Option<TreeCost<'_>>,
    ) -> Self {
        let inputs = (network, turns, times);
        Self::build_with_drives(
            inputs,
            trips,
            trip_keys,
            sets,
            active,
            car_start_s,
            step_s,
            None,
            cost,
        )
        .0
    }

    /// [`Self::build`], each tree also answering the **drives** `drives` asks for at its origin
    /// and bin (S299): drives searched from the same node, leaving at the same bin's middle, on
    /// the same times, to the nodes sought, bounded by the drives' bound (park-and-ride's shared
    /// drives to their car parks, `crate::itinerary_choice`, when their step is the trees' bin).
    /// The tree's search seeks its destinations and the drive's nodes at once, bounded by the
    /// larger bound; each keeps what it reached within its own bound, which is exactly what
    /// each search alone would have found
    /// ([`openmobisim_core_routes::Search::fastest_arrivals_to`]). A drive at an origin and bin
    /// without a tree is not searched here. Returns the trees, the same as [`Self::build`]'s, and
    /// the drives answered, ascending by (origin, bin).
    ///
    /// **By the route cost** (S299): with `cost` given and not time alone, each tree is also of
    /// one coefficient vector, the trips of a class searching by its class's vector
    /// ([`RouteCost::vector_of_class`]): a tree of a vector with extras minimises the travel time
    /// plus the extras ([`openmobisim_core_routes::Search::cheapest_routes_to`]), its keys' best
    /// and its routes' [`FoundRoute::cost`] are in that generalised cost, and it answers no
    /// drive (a drive is a search by time). Without, every tree is of vector 0 and searched by
    /// time, as above, to the bit.
    ///
    /// # Panics
    ///
    /// As [`Self::build`].
    #[must_use]
    #[allow(clippy::too_many_arguments, reason = "the loading's inputs, the trips and the bin")]
    #[allow(clippy::too_many_lines, reason = "one pass: group, search, split")]
    pub fn build_with_drives(
        (network, turns, times): (&RoadNetwork, &TurnTable, &LinkTimes),
        trips: &Trips,
        trip_keys: &[RouteKey],
        sets: &RouteSets,
        active: Option<&[bool]>,
        car_start_s: u32,
        step_s: f64,
        drives: Option<DrivesWanted<'_>>,
        cost: Option<TreeCost<'_>>,
    ) -> (Self, Vec<TreeDrive>) {
        assert!(step_s > 0.0, "a departure bin has a positive length");
        let cost = cost.filter(|c| !c.cost.is_time_only());
        let vector_of = |i: usize| {
            cost.map_or(0, |c| {
                c.cost.vector_of_class(c.trip_class.get(i).copied().unwrap_or(0) as usize)
            })
        };
        // (origin, bin, vector, key), sorted and deduplicated: the trees in a fixed order, each
        // key once per vector.
        let mut items: Vec<(u32, u64, u32, usize)> = (0..trips.len())
            .filter(|&i| active.is_none_or(|a| a[i as usize]))
            .filter_map(|i| {
                let key = trip_keys[i as usize];
                if key.origin == key.destination {
                    return None;
                }
                let k = sets.key_index(key)?;
                if sets.route_range(k).is_empty() {
                    return None;
                }
                // When the car enters the roads (S295): `car_start_s` after the departure.
                let departure = f64::from(trips.departure(TripId::new(i)).get() + car_start_s);
                #[allow(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "a bin count"
                )]
                let slot = (departure / step_s).floor() as u64;
                Some((key.origin, slot, vector_of(i as usize), k))
            })
            .collect();
        items.sort_unstable();
        items.dedup();
        let mut heads: Vec<(u32, u64, u32)> = Vec::new();
        let mut groups: Vec<(f64, Vec<usize>)> = Vec::new();
        for (origin, slot, vector, k) in items {
            if heads.last() == Some(&(origin, slot, vector)) {
                groups.last_mut().expect("a head has a group").1.push(k);
            } else {
                heads.push((origin, slot, vector));
                #[allow(clippy::cast_precision_loss, reason = "a bin count")]
                let departure = (slot as f64 + 0.5) * step_s;
                groups.push((departure, vec![k]));
            }
        }
        // Each tree's drive, if one is wanted at its origin and bin: the nodes sought, ascending.
        let mut sought: Vec<Option<Box<[u32]>>> = vec![None; heads.len()];
        let drive_bound = drives.map_or(0.0, |(_, bound)| bound);
        // Drives are searches by time: only trees by time answer them, all of them without a cost.
        if let (Some((wanted, _)), None) = (drives, cost) {
            let mut by_head: std::collections::BTreeMap<usize, Vec<u32>> =
                std::collections::BTreeMap::new();
            for (head, nodes) in wanted {
                if let Ok(i) = heads.binary_search(head) {
                    by_head.entry(i).or_default().extend(nodes.iter().map(|n| n.raw()));
                }
            }
            for (i, mut nodes) in by_head {
                nodes.sort_unstable();
                nodes.dedup();
                sought[i] = Some(nodes.into_boxed_slice());
            }
        }
        let ctx = SearchContext::new(network, turns);
        // Each vector's extras, prepared for this context (their A* bounds); none by time.
        let prepared: Vec<Option<Extras>> = cost.map_or_else(Vec::new, |c| {
            c.cost.vectors().iter().map(|v| v.as_ref().map(|e| ctx.extras(e.to_vec()))).collect()
        });
        let node_count = network.node_count() as usize;
        let wait = |link: u32, at: f64| times.origin_wait_seconds(link, at);
        let seconds = |link: u32, at: f64| times.link_seconds(link, at);
        let items: Vec<_> = heads.iter().zip(&groups).zip(&sought).collect();
        let searched: Vec<SearchedWithDrive> = search_map(&ctx, &items, |search, item| {
            let ((&(origin, slot, vector), (departure, keys)), drive) = *item;
            let extras = prepared.get(vector as usize).and_then(Option::as_ref);
            // A route's cost in the tree's currency: its time at the tree's departure, plus its
            // extras under the tree's vector where it has some.
            let cost_of = |links: &[u32]| {
                let time = times.route_seconds(links, *departure);
                match extras {
                    None => time,
                    Some(e) => time + links.iter().map(|&l| e.seconds()[l as usize]).sum::<f64>(),
                }
            };
            // Each key's best at the tree's departure: the search's bound is the slowest.
            let keys: Vec<(usize, f64)> = keys
                .iter()
                .map(|&k| {
                    let best = sets
                        .route_range(k)
                        .map(|r| cost_of(sets.route(r).links))
                        .fold(f64::INFINITY, f64::min);
                    (k, best)
                })
                .collect();
            let bound = keys.iter().map(|&(_, b)| b).fold(0.0, f64::max);
            let mut targets = vec![false; node_count];
            let mut wanted = 0;
            for &(k, _) in &keys {
                let d = sets.keys()[k].destination as usize;
                if !targets[d] {
                    targets[d] = true;
                    wanted += 1;
                }
            }
            if let Some(e) = extras {
                search.clear_route_state();
                search.set_extras(Some(e.clone()));
                let mut routes: Vec<FoundRoute> = search
                    .cheapest_routes_to(
                        NodeId::new(origin),
                        &targets,
                        wanted,
                        *departure,
                        bound,
                        &wait,
                        &seconds,
                    )
                    .into_iter()
                    .filter(|(_, _, _, links)| !links.is_empty())
                    .map(|(node, generalised, seconds, links)| FoundRoute {
                        destination: node.raw(),
                        seconds,
                        cost: generalised,
                        links: links.iter().map(|l| l.raw()).collect(),
                    })
                    .collect();
                search.clear_route_state();
                routes.sort_unstable_by_key(|r| r.destination);
                return (keys, routes, None);
            }
            let Some(drive) = drive else {
                let mut routes: Vec<FoundRoute> = search
                    .fastest_routes_to(
                        NodeId::new(origin),
                        &targets,
                        wanted,
                        *departure,
                        bound,
                        &wait,
                        &seconds,
                    )
                    .into_iter()
                    .filter(|(_, _, links)| !links.is_empty())
                    .map(|(node, seconds, links)| FoundRoute {
                        destination: node.raw(),
                        seconds,
                        cost: seconds,
                        links: links.iter().map(|l| l.raw()).collect(),
                    })
                    .collect();
                routes.sort_unstable_by_key(|r| r.destination);
                return (keys, routes, None);
            };
            // One search for the pairs and the drive: each keeps what it reached within its
            // own bound, the limits computed as each search alone computes its own.
            let mut all = targets.clone();
            let mut all_wanted = wanted;
            for &n in drive.iter() {
                if !all[n as usize] {
                    all[n as usize] = true;
                    all_wanted += 1;
                }
            }
            let (tree_limit, drive_limit) = (*departure + bound, *departure + drive_bound);
            let found = search.fastest_arrivals_to(
                NodeId::new(origin),
                &all,
                all_wanted,
                *departure,
                bound.max(drive_bound),
                &wait,
                &seconds,
            );
            let mut routes: Vec<FoundRoute> = Vec::new();
            let mut reached: Vec<(u32, Box<[LinkId]>)> = Vec::new();
            for (node, arrival, links) in found {
                if arrival <= drive_limit && drive.binary_search(&node.raw()).is_ok() {
                    reached.push((node.raw(), links.clone().into_boxed_slice()));
                }
                if arrival <= tree_limit && targets[node.index()] && !links.is_empty() {
                    let seconds = arrival - *departure;
                    routes.push(FoundRoute {
                        destination: node.raw(),
                        seconds,
                        cost: seconds,
                        links: links.iter().map(|l| l.raw()).collect(),
                    });
                }
            }
            routes.sort_unstable_by_key(|r| r.destination);
            reached.sort_unstable_by_key(|r| r.0);
            (
                keys,
                routes,
                Some(((origin, slot, vector), drive.clone(), reached.into_boxed_slice())),
            )
        });
        let mut pairs = 0_u32;
        let mut answered: Vec<TreeDrive> = Vec::new();
        let trees: Vec<Tree> = heads
            .iter()
            .zip(groups)
            .zip(searched)
            .map(|((&(origin, _, _), (departure, _)), (keys, routes, drive))| {
                pairs += u32::try_from(keys.len()).expect("few keys per tree");
                answered.extend(drive);
                Tree { origin, departure, keys, routes }
            })
            .collect();
        (Self { step_s, heads, trees, pairs }, answered)
    }

    /// The length of a departure bin, in seconds.
    #[must_use]
    pub fn step_s(&self) -> f64 {
        self.step_s
    }

    /// The trees, in their fixed order (origin, then bin).
    #[must_use]
    pub fn trees(&self) -> &[Tree] {
        &self.trees
    }

    /// How many trees there are: the searches made.
    #[must_use]
    pub fn len(&self) -> usize {
        self.trees.len()
    }

    /// Whether no tree was built.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.trees.is_empty()
    }

    /// How many pairs the trees searched for, in all: what a search per pair would have
    /// counted, so that routes added per search mean the same either way.
    #[must_use]
    pub fn pairs(&self) -> u32 {
        self.pairs
    }

    /// The tree of `origin` for a trip departing at `departure` seconds, if one was built: of
    /// coefficient vector 0 (every tree of a run whose cost is time alone).
    #[must_use]
    pub fn tree(&self, origin: u32, departure: f64) -> Option<&Tree> {
        self.tree_of(origin, 0, departure)
    }

    /// The tree of `origin` and coefficient vector `vector` for a trip departing at `departure`
    /// seconds, if one was built.
    #[must_use]
    pub fn tree_of(&self, origin: u32, vector: u32, departure: f64) -> Option<&Tree> {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a bin count")]
        let slot = (departure / self.step_s).floor() as u64;
        self.heads.binary_search(&(origin, slot, vector)).ok().map(|i| &self.trees[i])
    }

    /// The route the tree of `origin` and `departure` found to `destination`, if any: the
    /// fastest at the bin's middle, to be re-costed at the trip's own departure.
    #[must_use]
    pub fn route(&self, origin: u32, destination: u32, departure: f64) -> Option<&FoundRoute> {
        self.tree(origin, departure)?.route_to(destination)
    }

    /// [`Self::route`] in the tree of coefficient vector `vector`: the cheapest at the bin's
    /// middle in that vector's generalised cost.
    #[must_use]
    pub fn route_of(
        &self,
        origin: u32,
        vector: u32,
        destination: u32,
        departure: f64,
    ) -> Option<&FoundRoute> {
        self.tree_of(origin, vector, departure)?.route_to(destination)
    }
}

/// Perturbed trees (S298): one bounded time-dependent **generalised-cost** search per origin node,
/// departure bin and user class, under one preference draw's extras and attribute noise, on one
/// loading's link times. The `sampled` route update reads them for the candidates it adds beside
/// the best response: routes that are competitive under what a class values, not only the fastest.
///
/// Built like [`SearchTrees`] (a fixed order of (origin, bin, class), `search_map` in fixed chunks,
/// read by position), so the result is the same for any thread count and order of trips. A tree's
/// noise is a pure function of its (origin, bin, class), the draw and the seed.
#[derive(Clone, Debug, PartialEq)]
pub struct DrawTrees {
    step_s: f64,
    /// Each tree's (origin, bin, class), ascending, in step with [`Self::trees`]; the class is
    /// the representative of every class with the same extras.
    heads: Vec<(u32, u64, u32)>,
    trees: Vec<Tree>,
    /// Each class's representative: the first class whose extras for the draw are the same.
    representative: Vec<u32>,
}

impl DrawTrees {
    /// The trees of preference draw `draw` (1 or more) for the trips of `trips` whose pair has
    /// routes in `sets` (those `active` says, if given), each trip under its class
    /// (`trip_class`, by trip): the search of a tree minimises each link's time at the moment it
    /// is entered plus the class's extra seconds for the draw (`extras[class][draw]`, time alone
    /// where `None`), every link's cost times `1 + sigma · u` (`u` uniform, from `seed`, the
    /// tree and the draw), and is bounded by `max_detour` times the largest, over the pairs it
    /// serves, of the set's best cost at the bin's middle. A tree's currency is the class's
    /// generalised cost for the draw, unperturbed (its travel time where the class has no
    /// extras): its `keys` carry the set's best in it, and each route found its travel time and
    /// its cost in it ([`FoundRoute::cost`]).
    ///
    /// # Panics
    ///
    /// Panics if `step_s` is not positive, or if `extras` has a row without entry `draw`.
    #[must_use]
    #[allow(
        clippy::too_many_arguments,
        reason = "the loading's inputs, the trips, the bin and the draw's parameters"
    )]
    pub fn build(
        network: &RoadNetwork,
        turns: &TurnTable,
        times: &LinkTimes,
        trips: &Trips,
        trip_keys: &[RouteKey],
        trip_class: &[u32],
        sets: &RouteSets,
        active: Option<&[bool]>,
        car_start_s: u32,
        step_s: f64,
        extras: &ClassExtras,
        draw: usize,
        seed: u64,
        sigma: f64,
        max_detour: f64,
    ) -> Self {
        assert!(step_s > 0.0, "a departure bin has a positive length");
        // Classes whose extras for this draw are the same (every class without prices, say)
        // share one tree: each class is mapped to the first with the same extras.
        let same = |a: &Option<Arc<[f64]>>, b: &Option<Arc<[f64]>>| match (a, b) {
            (None, None) => true,
            (Some(a), Some(b)) => a[..] == b[..],
            _ => false,
        };
        let representative: Vec<u32> = (0..extras.len())
            .map(|c| {
                let mine = extras[c]
                    .get(draw)
                    .unwrap_or_else(|| panic!("extras have no entry for draw {draw}"));
                let first = (0..c).find(|&d| same(&extras[d][draw], mine)).unwrap_or(c);
                u32::try_from(first).expect("few classes")
            })
            .collect();
        let mut items: Vec<(u32, u64, u32, usize)> = (0..trips.len())
            .filter(|&i| active.is_none_or(|a| a[i as usize]))
            .filter_map(|i| {
                let key = trip_keys[i as usize];
                if key.origin == key.destination {
                    return None;
                }
                let k = sets.key_index(key)?;
                if sets.route_range(k).is_empty() {
                    return None;
                }
                let departure = f64::from(trips.departure(TripId::new(i)).get() + car_start_s);
                #[allow(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "a bin count"
                )]
                let slot = (departure / step_s).floor() as u64;
                let class = trip_class.get(i as usize).copied().unwrap_or(0);
                Some((
                    key.origin,
                    slot,
                    representative.get(class as usize).copied().unwrap_or(0),
                    k,
                ))
            })
            .collect();
        items.sort_unstable();
        items.dedup();
        let mut heads: Vec<(u32, u64, u32)> = Vec::new();
        let mut groups: Vec<(f64, Vec<usize>)> = Vec::new();
        for (origin, slot, class, k) in items {
            if heads.last() == Some(&(origin, slot, class)) {
                groups.last_mut().expect("a head has a group").1.push(k);
            } else {
                heads.push((origin, slot, class));
                #[allow(clippy::cast_precision_loss, reason = "a bin count")]
                let departure = (slot as f64 + 0.5) * step_s;
                groups.push((departure, vec![k]));
            }
        }
        let ctx = SearchContext::new(network, turns);
        // The draw's extras per class, prepared for this context (their A* bounds).
        let prepared: Vec<Option<Extras>> = extras
            .iter()
            .map(|row| {
                row.get(draw)
                    .unwrap_or_else(|| panic!("extras have no entry for draw {draw}"))
                    .as_ref()
                    .map(|v| ctx.extras(v.to_vec()))
            })
            .collect();
        let node_count = network.node_count() as usize;
        let wait = |link: u32, at: f64| times.origin_wait_seconds(link, at);
        let seconds = |link: u32, at: f64| times.link_seconds(link, at);
        let searched: Vec<Searched> =
            search_map(&ctx, &heads.iter().zip(&groups).collect::<Vec<_>>(), |search, item| {
                let (&(origin, slot, class), (departure, keys)) = *item;
                let extra = prepared.get(class as usize).cloned().flatten();
                let seconds_of = extra.as_ref().map(Extras::seconds);
                // A route's cost in the tree's currency: its travel time at the tree's departure,
                // plus its links' extras for the draw, unperturbed, where the class has extras.
                let cost_of = |links: &[u32]| {
                    let time = times.route_seconds(links, *departure);
                    match seconds_of {
                        None => time,
                        Some(e) => time + links.iter().map(|&l| e[l as usize]).sum::<f64>(),
                    }
                };
                // Each key's best at the tree's departure, in that currency: what the update
                // compares a candidate with. The search is bounded by the detour times the
                // slowest best, in the same cost.
                let mut slowest = 0.0_f64;
                let keys: Vec<(usize, f64)> = keys
                    .iter()
                    .map(|&k| {
                        let best = sets
                            .route_range(k)
                            .map(|r| cost_of(sets.route(r).links))
                            .fold(f64::INFINITY, f64::min);
                        slowest = slowest.max(best);
                        (k, best)
                    })
                    .collect();
                let bound = max_detour * slowest;
                let by_generalised = seconds_of.is_some();
                search.clear_route_state();
                search.set_extras(extra.clone());
                search.set_noise(draw_seed(origin, slot, class, draw, seed), sigma, None);
                let mut targets = vec![false; node_count];
                let mut wanted = 0;
                for &(k, _) in &keys {
                    let d = sets.keys()[k].destination as usize;
                    if !targets[d] {
                        targets[d] = true;
                        wanted += 1;
                    }
                }
                let mut routes: Vec<FoundRoute> = search
                    .cheapest_routes_to(
                        NodeId::new(origin),
                        &targets,
                        wanted,
                        *departure,
                        bound,
                        &wait,
                        &seconds,
                    )
                    .into_iter()
                    .filter(|(_, _, _, links)| !links.is_empty())
                    .map(|(node, generalised, seconds, links)| FoundRoute {
                        destination: node.raw(),
                        seconds,
                        cost: if by_generalised { generalised } else { seconds },
                        links: links.iter().map(|l| l.raw()).collect(),
                    })
                    .collect();
                routes.sort_unstable_by_key(|r| r.destination);
                search.clear_route_state();
                (keys, routes)
            });
        let trees: Vec<Tree> = heads
            .iter()
            .zip(groups)
            .zip(searched)
            .map(|((&(origin, _, _), (departure, _)), (keys, routes))| Tree {
                origin,
                departure,
                keys,
                routes,
            })
            .collect();
        Self { step_s, heads, trees, representative }
    }

    /// The length of a departure bin, in seconds.
    #[must_use]
    pub fn step_s(&self) -> f64 {
        self.step_s
    }

    /// The trees, in their fixed order (origin, bin, class).
    #[must_use]
    pub fn trees(&self) -> &[Tree] {
        &self.trees
    }

    /// How many trees there are: the searches made.
    #[must_use]
    pub fn len(&self) -> usize {
        self.trees.len()
    }

    /// Whether no tree was built.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.trees.is_empty()
    }

    /// The tree of `origin` and `class` for a trip departing at `departure` seconds, if built.
    #[must_use]
    pub fn tree(&self, origin: u32, class: u32, departure: f64) -> Option<&Tree> {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a bin count")]
        let slot = (departure / self.step_s).floor() as u64;
        let class = self.representative.get(class as usize).copied().unwrap_or(class);
        self.heads.binary_search(&(origin, slot, class)).ok().map(|i| &self.trees[i])
    }
}

/// The noise seed of one perturbed tree: a pure function of the tree, the draw and the seed.
fn draw_seed(origin: u32, slot: u64, class: u32, draw: usize, seed: u64) -> u64 {
    (u64::from(origin) << 32 | (slot & 0xFF_FFFF) << 8 | u64::from(class & 0xFF))
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (draw as u64 + 1).wrapping_mul(0xD6E8_FEB8_6659_FD93)
        ^ seed.wrapping_mul(0xA24B_AED4_963E_E407)
}
