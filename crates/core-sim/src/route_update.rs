//! Route updates: growing the choice sets between iterations (S176).
//!
//! Route sets are made once, at free-flow costs. When traffic has rearranged the costs
//! the network holds routes the sets do not, and an equilibration over the fixed sets
//! stops short of the network's own equilibrium: the **network gap**
//! ([`IterationReport::gap_network`](crate::IterationReport::gap_network)) stays large
//! while the gap against the set looks fine (found in S171). A
//! [`RouteUpdate`] closes that: after each loading it may **add routes** to the sets, found on
//! the link times that loading produced, before travellers choose again. **The set is fixed
//! *within* an iteration and may grow *between* iterations** (design §11.1(2)); the
//! equilibrium is then over the set the generator and the update rule produce, which is why
//! the update is part of the run's description.
//!
//! Built in:
//!
//! | name | what it does |
//! |---|---|
//! | `none` (the default, [`DEFAULT_UPDATE`]) | Nothing: the sets stay as the generator made them, and the run costs nothing extra |
//! | `best_response` | For every origin–destination pair, one bounded time-dependent search (at the median departure of the pair's trips) for the fastest route at the congested times; it is **added if it is new and at least as fast as the set's best route**: a tie counts, since an equally good alternative is one more to spread over (S176: without ties, Seoul's grid ended at a network gap of 0.57 instead of 0.12). Column generation, as in MATSim's re-routing, SUMO's `duaIterate` and DTALite |
//!
//! **What `best_response` costs.** One search per pair per iteration (`searches` of them), each
//! bounded by the best time the set already offers, so it explores what could beat that and no
//! more: about 0.5 ms of core time on Stockholm's inner city, 1.2 ms on Seoul (S174), in
//! parallel over pairs. With demand between zones (a few hundred pairs, not one per trip) an
//! update is a small part of a loading. Then one copy of the store
//! ([`RouteSets::extended`]), and a chooser rebuilt on it. **What it adds to the store:** a
//! pair ends with about four routes (S174); at most `max_routes`. **Light demand adds little,
//! not nothing**: where nothing queues no route beats the set's best, but an equally good one
//! still counts, and on a grid of equal blocks there are many.
//!
//! **Determinism.** Each pair's search is a function of the pair, the departure and the link
//! times alone; pairs are searched in parallel in fixed chunks ([`search_map`]) and merged in
//! key order, so the result is the same for any thread count and order of trips.
//!
//! # Extending it
//!
//! A new update (regenerate the whole set on congested times, a per-traveller probit search,
//! a bias from the dynamics) is a type implementing [`RouteUpdate`], added to a [`Registry`]
//! to be selected by name.

use std::collections::BTreeMap;
use std::fmt;

use openmobisim_core_demand::Trips;
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_routes::{
    MAX_ROUTES_PER_SET, Route, RouteKey, RouteSets, Search, SearchContext, search_map,
};
use openmobisim_core_types::ids::{EntityId, LinkId, NodeId, TripId};

use crate::link_times::LinkTimes;

/// The update used unless another is asked for.
pub const DEFAULT_UPDATE: &str = "none";

/// An update's options: numbers by name. Unknown names are an error.
pub type Options = BTreeMap<String, f64>;

/// [`MAX_ROUTES_PER_SET`] as an option's value.
#[allow(clippy::cast_possible_truncation, reason = "a small constant: 32")]
const MAX_ROUTES: u32 = MAX_ROUTES_PER_SET as u32;

/// Everything an update reads: the network, the demand, the sets as they are, and the link
/// times the last loading produced.
pub struct UpdateContext<'a> {
    /// The network.
    pub network: &'a RoadNetwork,
    /// Its turn table.
    pub turns: &'a TurnTable,
    /// The trips.
    pub trips: &'a Trips,
    /// Each trip's origin-destination pair, snapped to nodes, indexed by trip.
    pub trip_keys: &'a [RouteKey],
    /// The sets as they are now.
    pub route_sets: &'a RouteSets,
    /// The times the last loading produced.
    pub times: &'a LinkTimes,
    /// The iteration the routes are for (1 or more): the one that will choose among them.
    pub iteration: u32,
}

/// The routes an update found.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Additions {
    /// `(key index, routes to add)`, in **strictly ascending key order**, one entry per key,
    /// none empty. Routes are new (not in the key's set), from the key's origin to its
    /// destination, and carry their free-flow cost and their overlap with the set.
    pub routes: Vec<(usize, Vec<Route>)>,
    /// How many searches it took.
    pub searches: u32,
}

impl Additions {
    /// How many routes in all.
    #[must_use]
    pub fn route_count(&self) -> usize {
        self.routes.iter().map(|(_, r)| r.len()).sum()
    }
}

/// How the sets grow between iterations. See the [module docs](self).
pub trait RouteUpdate: Send + Sync {
    /// The update's name, as it is selected.
    fn name(&self) -> &str;

    /// The name and every option with defaults filled in, canonical; part of the run's
    /// fingerprint and of the grown store's identity.
    fn descriptor(&self) -> String;

    /// Whether the update ever adds anything. `false` means the run never calls
    /// [`Self::update`], builds nothing for it and hashes nothing of it.
    fn is_active(&self) -> bool {
        true
    }

    /// The routes to add, given the state after a loading. Must be a function of `cx`
    /// alone: the same for any thread count.
    fn update(&self, cx: &UpdateContext<'_>) -> Additions;
}

/// Leave the sets as the generator made them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NoRouteUpdate;

impl RouteUpdate for NoRouteUpdate {
    fn name(&self) -> &str {
        "none"
    }
    fn descriptor(&self) -> String {
        "none".to_string()
    }
    fn is_active(&self) -> bool {
        false
    }
    fn update(&self, _cx: &UpdateContext<'_>) -> Additions {
        Additions::default()
    }
}

impl NoRouteUpdate {
    /// Make the update from its options (it has none).
    ///
    /// # Errors
    ///
    /// [`RouteUpdateError::UnknownOption`] for any option.
    pub fn from_options(options: &Options) -> Result<Self, RouteUpdateError> {
        check_known("none", options, &[])?;
        Ok(Self)
    }
}

/// Add each pair's fastest route at the congested times, if it is new and no slower than the
/// set's best.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BestResponse {
    /// How many searches per pair per iteration, at evenly spread quantiles of the pair's
    /// departures (1 is the median). More finds routes that only pay at some hours, at the
    /// cost of more searches and a bigger set, which raises the logit's own gap (S174: 1
    /// measured best).
    pub searches: u32,
    /// The most routes a pair's set may hold (1 to [`MAX_ROUTES_PER_SET`]); a pair at the
    /// limit is not searched.
    pub max_routes: u32,
}

impl Default for BestResponse {
    fn default() -> Self {
        Self { searches: 1, max_routes: 10 }
    }
}

impl BestResponse {
    const OPTIONS: [&'static str; 2] = ["max_routes", "searches"];

    /// Make the update from its options, defaults for those not given.
    ///
    /// # Errors
    ///
    /// [`RouteUpdateError::UnknownOption`] for a name it does not have,
    /// [`RouteUpdateError::BadOption`] for a value out of range.
    pub fn from_options(options: &Options) -> Result<Self, RouteUpdateError> {
        check_known("best_response", options, &Self::OPTIONS)?;
        let mut m = Self::default();
        let whole = |option: &str, v: f64, lo: u32, hi: u32| -> Result<u32, RouteUpdateError> {
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "checked to be a whole number within a small range"
            )]
            let n = v as u32;
            if v.is_finite() && v.fract() == 0.0 && v >= f64::from(lo) && v <= f64::from(hi) {
                Ok(n)
            } else {
                Err(RouteUpdateError::BadOption {
                    update: "best_response".to_string(),
                    option: option.to_string(),
                    reason: format!("must be a whole number from {lo} to {hi}, got {v}"),
                })
            }
        };
        for (option, &v) in options {
            match option.as_str() {
                "searches" => m.searches = whole(option, v, 1, 16)?,
                _ => {
                    m.max_routes = whole(option, v, 1, MAX_ROUTES)?;
                }
            }
        }
        Ok(m)
    }
}

/// One pair's searches: its key index and the range of its departures in the flat list.
struct Job {
    key: usize,
    departures: core::ops::Range<usize>,
}

impl RouteUpdate for BestResponse {
    fn name(&self) -> &str {
        "best_response"
    }

    fn descriptor(&self) -> String {
        format!("best_response;max_routes={};searches={}", self.max_routes, self.searches)
    }

    fn update(&self, cx: &UpdateContext<'_>) -> Additions {
        let sets = cx.route_sets;
        let (departures, jobs) = self.plan(cx);
        let ctx = SearchContext::new(cx.network, cx.turns);
        let max_routes = self.max_routes as usize;
        let found: Vec<(Vec<Route>, u32)> = search_map(&ctx, &jobs, |search, job| {
            let set = sets.route_range(job.key);
            let key = sets.keys()[job.key];
            let wait = |link: u32, at: f64| cx.times.origin_wait_seconds(link, at);
            let seconds = |link: u32, at: f64| cx.times.link_seconds(link, at);
            let mut added: Vec<Route> = Vec::new();
            let mut searches = 0_u32;
            for &second in &departures[job.departures.clone()] {
                let departure = f64::from(second);
                if set.len() + added.len() >= max_routes {
                    break;
                }
                // What the set offers at this departure, the routes added a moment ago included.
                let best = set
                    .clone()
                    .map(|r| cx.times.route_seconds(sets.route(r).links, departure))
                    .chain(added.iter().map(|r| {
                        let raw: Vec<u32> = r.links.iter().map(|l| l.raw()).collect();
                        cx.times.route_seconds(&raw, departure)
                    }))
                    .fold(f64::INFINITY, f64::min);
                searches += 1;
                let Some((_, links)) = search.fastest_route(
                    NodeId::new(key.origin),
                    NodeId::new(key.destination),
                    departure,
                    best,
                    &wait,
                    &seconds,
                ) else {
                    continue;
                };
                // The search was bounded by the set's best, so `links` is at least as good. A
                // route the set already holds is what it usually finds: not new.
                if is_known(sets, job.key, &added, &links) {
                    continue;
                }
                let route = with_overlap(search, sets, job.key, &added, links);
                added.push(route);
            }
            (added, searches)
        });
        let mut out = Additions::default();
        for (job, (routes, searches)) in jobs.iter().zip(found) {
            out.searches += searches;
            if !routes.is_empty() {
                out.routes.push((job.key, routes));
            }
        }
        out
    }
}

impl BestResponse {
    /// The searches to make: for each key with routed trips and room to grow, the departures
    /// to search at (evenly spread quantiles of its trips' departures), flat, with each key's
    /// range. Keys ascend.
    fn plan(&self, cx: &UpdateContext<'_>) -> (Vec<u32>, Vec<Job>) {
        let sets = cx.route_sets;
        let max_routes = self.max_routes as usize;
        // (key, departure) packed so that one sort orders by key, then departure.
        let mut pairs: Vec<u64> = (0..cx.trips.len())
            .filter_map(|i| {
                let k = sets.key_index(cx.trip_keys[i as usize])?;
                let room =
                    !sets.route_range(k).is_empty() && sets.route_range(k).len() < max_routes;
                room.then(|| {
                    let departure = cx.trips.departure(TripId::new(i)).get();
                    (k as u64) << 32 | u64::from(departure)
                })
            })
            .collect();
        pairs.sort_unstable();
        let (mut departures, mut jobs) = (Vec::new(), Vec::new());
        let mut from = 0;
        while from < pairs.len() {
            let key = pairs[from] >> 32;
            let len = pairs[from..].iter().take_while(|&&p| p >> 32 == key).count();
            let group = &pairs[from..from + len];
            let start = departures.len();
            let n = (self.searches as usize).min(len);
            for i in 0..n {
                #[allow(
                    clippy::cast_possible_truncation,
                    reason = "the low 32 bits are the departure"
                )]
                let at = group[((2 * i + 1) * len / (2 * n)).min(len - 1)] as u32;
                // Two quantiles that land on the same second are one search.
                if departures.len() == start || departures.last() != Some(&at) {
                    departures.push(at);
                }
            }
            jobs.push(Job { key: key as usize, departures: start..departures.len() });
            from += len;
        }
        (departures, jobs)
    }
}

/// Whether `links` is a route of the key's set already, or one of the `added`.
fn is_known(sets: &RouteSets, key: usize, added: &[Route], links: &[LinkId]) -> bool {
    let same =
        |raw: &[u32]| raw.len() == links.len() && raw.iter().zip(links).all(|(a, b)| *a == b.raw());
    sets.routes(key).any(|r| same(r.links)) || added.iter().any(|a| a.links == links)
}

/// A route of `links` with its free-flow cost and its overlap with the routes the key already
/// holds and the `added` ones before it (the share of its cost it has in common with the one
/// it shares most with, as for every route of a set).
fn with_overlap(
    search: &mut Search<'_>,
    sets: &RouteSets,
    key: usize,
    added: &[Route],
    links: Vec<LinkId>,
) -> Route {
    search.clear_route_state();
    let mut index = 0;
    for r in sets.routes(key) {
        let raw: Vec<LinkId> = r.links.iter().map(|&l| LinkId::new(l)).collect();
        search.mark(&raw, index);
        index += 1;
    }
    for a in added {
        search.mark(&a.links, index);
        index += 1;
    }
    let overlap = search.max_overlap(&links);
    search.clear_route_state();
    let cost = search.context().route_cost(&links);
    Route { links, cost, overlap }
}

/// Why an update could not be made.
#[derive(Clone, PartialEq, Debug)]
pub enum RouteUpdateError {
    /// No update has this name.
    UnknownUpdate {
        /// The name asked for.
        name: String,
        /// The names that exist.
        known: Vec<String>,
    },
    /// The update has no such option.
    UnknownOption {
        /// The update.
        update: String,
        /// The option asked for.
        option: String,
        /// The options it has.
        known: Vec<&'static str>,
    },
    /// An option's value is not allowed.
    BadOption {
        /// The update.
        update: String,
        /// The option.
        option: String,
        /// What is wrong with it.
        reason: String,
    },
}

impl fmt::Display for RouteUpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownUpdate { name, known } => {
                write!(f, "no route update called {name:?}; the updates are: {}", known.join(", "))
            }
            Self::UnknownOption { update, option, known } if known.is_empty() => {
                write!(f, "route update {update:?} has no option {option:?}; it has no options")
            }
            Self::UnknownOption { update, option, known } => write!(
                f,
                "route update {update:?} has no option {option:?}; its options are: {}",
                known.join(", ")
            ),
            Self::BadOption { update, option, reason } => {
                write!(f, "route update {update:?}, option {option:?}: {reason}")
            }
        }
    }
}

impl std::error::Error for RouteUpdateError {}

fn check_known(
    update: &str,
    options: &Options,
    known: &[&'static str],
) -> Result<(), RouteUpdateError> {
    match options.keys().find(|k| !known.contains(&k.as_str())) {
        Some(option) => Err(RouteUpdateError::UnknownOption {
            update: update.to_string(),
            option: option.clone(),
            known: known.to_vec(),
        }),
        None => Ok(()),
    }
}

// --- the registry -----------------------------------------------------------------

/// Makes an update from its options.
pub type Factory = fn(&Options) -> Result<Box<dyn RouteUpdate>, RouteUpdateError>;

/// Updates by name; a researcher's is added with [`Registry::register`].
#[derive(Clone, Debug)]
pub struct Registry {
    updates: Vec<(String, Factory)>,
}

impl Registry {
    /// The built-in updates: `none` and `best_response`.
    #[must_use]
    pub fn builtin() -> Self {
        let mut r = Self { updates: Vec::new() };
        r.register("none", |o| Ok(Box::new(NoRouteUpdate::from_options(o)?)));
        r.register("best_response", |o| Ok(Box::new(BestResponse::from_options(o)?)));
        r
    }

    /// Add an update under `name`, or replace one of that name.
    pub fn register(&mut self, name: &str, factory: Factory) {
        match self.updates.iter_mut().find(|(n, _)| n == name) {
            Some(entry) => entry.1 = factory,
            None => self.updates.push((name.to_string(), factory)),
        }
    }

    /// The names that can be selected, in registration order.
    #[must_use]
    pub fn names(&self) -> Vec<&str> {
        self.updates.iter().map(|(n, _)| n.as_str()).collect()
    }

    /// Make the update called `name` from `options`.
    ///
    /// # Errors
    ///
    /// [`RouteUpdateError::UnknownUpdate`] if there is none, or the update's own validation.
    pub fn create(
        &self,
        name: &str,
        options: &Options,
    ) -> Result<Box<dyn RouteUpdate>, RouteUpdateError> {
        match self.updates.iter().find(|(n, _)| n == name) {
            Some((_, factory)) => factory(options),
            None => Err(RouteUpdateError::UnknownUpdate {
                name: name.to_string(),
                known: self.names().into_iter().map(str::to_string).collect(),
            }),
        }
    }
}

/// Make a built-in update by name; see [`Registry::builtin`].
///
/// # Errors
///
/// As [`Registry::create`].
pub fn update(name: &str, options: &Options) -> Result<Box<dyn RouteUpdate>, RouteUpdateError> {
    Registry::builtin().create(name, options)
}
