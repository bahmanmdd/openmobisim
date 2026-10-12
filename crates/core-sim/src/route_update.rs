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
//! parallel over pairs, and **not made for a pair whose best route is already within `slack` (2%) of
//! free flow**, which is most pairs when demand is light or moderate. With demand between zones (a few hundred pairs, not one per trip) an
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

/// The share of free flow within which `best_response` does not search a pair (S178; see
/// [`BestResponse::slack`]).
pub const DEFAULT_SLACK: f64 = 0.02;

/// The step `best_response` rounds departures to for its search trees (S267; see
/// [`BestResponse::tree_step_s`]).
pub const DEFAULT_TREE_STEP_S: u32 = 0;

/// The most routes a pair's set holds by default (S176). A traveller is offered at most five of
/// them (`Run::with_choice_max_routes`, S268): the set remembers more, so that a route that
/// congestion made slow can come back, and a full set does not hold the run back (with five
/// stored, Lyon's run ended poor instead of acceptable, S268).
pub const DEFAULT_MAX_ROUTES: u32 = 10;

/// How much faster than a full set's best a route must be to replace one of its routes, in
/// seconds (S268): a tie keeps the set as it is.
pub const REPLACE_GAIN_S: f64 = 1.0;

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
    /// The iteration the routes are for: the one that will choose among them (0 while the
    /// free-flow loading is built up, S223).
    pub iteration: u32,
    /// The trips whose pairs are searched, by trip (S223: one group of the free-flow loading's
    /// increments); every trip's if `None`.
    pub active: Option<&'a [bool]>,
}

/// The routes an update found.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Additions {
    /// `(key index, routes to add)`, in **strictly ascending key order**, one entry per key,
    /// none empty. Routes are new (not in the key's set), from the key's origin to its
    /// destination, and carry their free-flow cost and their overlap with the set.
    pub routes: Vec<(usize, Vec<Route>)>,
    /// How many searches it took; with search trees ([`BestResponse::tree_step_s`]), how many
    /// pairs the trees searched for (one tree answers many), so that the routes added per search
    /// mean the same either way.
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

    /// The most routes a set may hold, if the update caps it: a set its additions would take
    /// past this makes room ([`make_room`], S268).
    fn max_routes(&self) -> Option<usize> {
        None
    }
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
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BestResponse {
    /// How many searches per pair per iteration, at evenly spread quantiles of the pair's
    /// departures (1 is the median). More finds routes that only pay at some hours, at the
    /// cost of more searches and a bigger set, which raises the logit's own gap (S174: 1
    /// measured best).
    pub searches: u32,
    /// The most routes a pair's set may hold (1 to [`MAX_ROUTES_PER_SET`]; [`DEFAULT_MAX_ROUTES`]).
    /// **A full set is still searched (S268):** a route at least [`REPLACE_GAIN_S`] faster than
    /// its best takes the place of a route nobody chose in the last choice (never its free-flow
    /// shortest one, which the slack rests on); if every route is in use, it waits. No trip is
    /// moved. Ties do not replace, so equal routes do not cycle in and out.
    pub max_routes: u32,
    /// A pair is **not searched** while the set's best route at its departure is within this share
    /// of the route's free-flow time (0 to 1; 0 searches every pair; [`DEFAULT_SLACK`], S178). No
    /// route can be faster than free flow, so a pair this close to it has at most this much to gain:
    /// **the regret left is bounded by `slack`**. Measured (S178, `msa` × 8–10 iterations, the
    /// disequilibrium unchanged to the third decimal): at 2%, Manhattan's searches fell from 62 877
    /// to 7 569 and the run from 5.9 s to 4.0 s, Singapore's from 27 576 to 313, Stockholm's inner
    /// city (heavy) from 78 512 to 46 205 (−8% of the run); at 5%, 58% of Stockholm's searches.
    /// Assumes the method's routes include a free-flow shortest one, as every built-in method's do.
    pub slack: f64,
    /// **Search trees** (S267): 0 searches each pair on its own, at its departures; above 0, a
    /// pair's departures are rounded to this step (in seconds, to its middle) and the pairs of one
    /// origin at one rounded departure share **one search tree**, which finds each one's fastest
    /// route at once ([`openmobisim_core_routes::Search::fastest_routes_to`]). Approximate: the
    /// routes are the fastest at the rounded departure. Worth it when trips start at a few points
    /// per zone (`demand_from_od`'s `points_per_zone`), so that many pairs share an origin.
    pub tree_step_s: u32,
}

impl Default for BestResponse {
    fn default() -> Self {
        Self {
            searches: 1,
            max_routes: DEFAULT_MAX_ROUTES,
            slack: DEFAULT_SLACK,
            tree_step_s: DEFAULT_TREE_STEP_S,
        }
    }
}

impl BestResponse {
    const OPTIONS: [&'static str; 4] = ["max_routes", "searches", "slack", "tree_step_s"];

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
                "tree_step_s" => m.tree_step_s = whole(option, v, 0, 86_400)?,
                "slack" => {
                    if !(v.is_finite() && (0.0..=1.0).contains(&v)) {
                        return Err(RouteUpdateError::BadOption {
                            update: "best_response".to_string(),
                            option: option.to_string(),
                            reason: format!("must be from 0 to 1, got {v}"),
                        });
                    }
                    m.slack = v;
                }
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
    fn max_routes(&self) -> Option<usize> {
        Some(self.max_routes as usize)
    }

    fn name(&self) -> &str {
        "best_response"
    }

    fn descriptor(&self) -> String {
        let trees = if self.tree_step_s > 0 {
            format!(";tree_step_s={}", self.tree_step_s)
        } else {
            String::new()
        };
        format!(
            "best_response;max_routes={};searches={};slack={}{trees}",
            self.max_routes, self.searches, self.slack
        )
    }

    fn update(&self, cx: &UpdateContext<'_>) -> Additions {
        if self.tree_step_s > 0 {
            return self.update_by_trees(cx);
        }
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
            // A full set may take one route, which replaces one of its own (S268).
            let full = set.len() >= max_routes;
            let room = max_routes.saturating_sub(set.len()).max(1);
            // The set's free-flow shortest time: no route is faster than it.
            let free_flow =
                set.clone().map(|r| f64::from(sets.route(r).cost)).fold(f64::INFINITY, f64::min);
            for &second in &departures[job.departures.clone()] {
                let departure = f64::from(second);
                if added.len() >= room {
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
                // At most `slack` left to gain here: nothing beats free flow.
                if self.slack > 0.0 && best <= free_flow * (1.0 + self.slack) {
                    continue;
                }
                searches += 1;
                let Some((time, links)) = search.fastest_route(
                    NodeId::new(key.origin),
                    NodeId::new(key.destination),
                    departure,
                    best,
                    &wait,
                    &seconds,
                ) else {
                    continue;
                };
                if full && time >= best - REPLACE_GAIN_S {
                    continue;
                }
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

/// Routes a search tree kept, by key index, and how many pairs it searched for.
type TreeFound = (Vec<(usize, Vec<LinkId>)>, u32);

/// One search tree: an origin, a rounded departure, and the keys it serves.
struct Tree {
    origin: u32,
    departure: f64,
    keys: Vec<usize>,
}

impl BestResponse {
    /// [`RouteUpdate::update`] with search trees (S267, [`Self::tree_step_s`]): the planned
    /// departures rounded to the step, the keys grouped by origin and rounded departure, one
    /// [`Search::fastest_routes_to`] per group bounded by the slowest of its keys' best times; a
    /// key's route is kept if it is new and no slower than the key's own best. Then the kept
    /// routes are made routes of their sets (their overlap), key by key.
    fn update_by_trees(&self, cx: &UpdateContext<'_>) -> Additions {
        let sets = cx.route_sets;
        let max_routes = self.max_routes as usize;
        let step = f64::from(self.tree_step_s);
        let (departures, jobs) = self.plan(cx);
        // (origin, rounded departure in steps, key), sorted: the trees in a fixed order.
        let mut items: Vec<(u32, u64, usize)> = Vec::new();
        for job in &jobs {
            let key = sets.keys()[job.key];
            if key.origin == key.destination {
                continue;
            }
            let mut seen: Vec<u64> = Vec::new();
            for &second in &departures[job.departures.clone()] {
                #[allow(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "a step count"
                )]
                let slot = (f64::from(second) / step).floor() as u64;
                if !seen.contains(&slot) {
                    seen.push(slot);
                    items.push((key.origin, slot, job.key));
                }
            }
        }
        items.sort_unstable();
        let mut trees: Vec<Tree> = Vec::new();
        for (origin, slot, key) in items {
            #[allow(clippy::cast_precision_loss, reason = "a step count")]
            let departure = (slot as f64 + 0.5) * step;
            match trees.last_mut() {
                Some(t) if t.origin == origin && t.departure.to_bits() == departure.to_bits() => {
                    t.keys.push(key);
                }
                _ => trees.push(Tree { origin, departure, keys: vec![key] }),
            }
        }
        let ctx = SearchContext::new(cx.network, cx.turns);
        let node_count = cx.network.node_count() as usize;
        let wait = |link: u32, at: f64| cx.times.origin_wait_seconds(link, at);
        let seconds = |link: u32, at: f64| cx.times.link_seconds(link, at);
        let found: Vec<TreeFound> = search_map(&ctx, &trees, |search, tree| {
            // Each key's best at the rounded departure, and whether it is worth searching.
            let mut wanted: Vec<(usize, f64)> = Vec::new();
            for &k in &tree.keys {
                let set = sets.route_range(k);
                if set.is_empty() {
                    continue;
                }
                let full = set.len() >= max_routes;
                let free_flow = set
                    .clone()
                    .map(|r| f64::from(sets.route(r).cost))
                    .fold(f64::INFINITY, f64::min);
                let best = set
                    .map(|r| cx.times.route_seconds(sets.route(r).links, tree.departure))
                    .fold(f64::INFINITY, f64::min);
                if self.slack > 0.0 && best <= free_flow * (1.0 + self.slack) {
                    continue;
                }
                // A full set takes only a route that beats its best (S268): the bound to beat.
                wanted.push((k, if full { best - REPLACE_GAIN_S } else { best }));
            }
            if wanted.is_empty() {
                return (Vec::new(), 0);
            }
            let mut targets = vec![false; node_count];
            for &(k, _) in &wanted {
                targets[sets.keys()[k].destination as usize] = true;
            }
            let bound = wanted.iter().map(|&(_, b)| b).fold(0.0, f64::max);
            let routes = search.fastest_routes_to(
                NodeId::new(tree.origin),
                &targets,
                wanted.len(),
                tree.departure,
                bound,
                &wait,
                &seconds,
            );
            let mut kept = Vec::new();
            for (node, time, links) in routes {
                for &(k, best) in &wanted {
                    // At least as fast as the set's best: a tie counts (S176), but for a full
                    // set, whose bound is below its best (S268).
                    if sets.keys()[k].destination == node.raw()
                        && time <= best + 1e-9
                        && !links.is_empty()
                        && !is_known(sets, k, &[], &links)
                    {
                        kept.push((k, links.clone()));
                    }
                }
            }
            // Counted as the pairs searched for, as a search per pair would be: what the
            // stopping rule weighs the routes added against (`route_growth_tolerance`).
            (kept, u32::try_from(wanted.len()).expect("few keys per tree"))
        });
        let mut searches = 0_u32;
        let mut by_key: BTreeMap<usize, Vec<Vec<LinkId>>> = BTreeMap::new();
        for (kept, n) in found {
            searches += n;
            for (k, links) in kept {
                by_key.entry(k).or_default().push(links);
            }
        }
        let groups: Vec<(usize, Vec<Vec<LinkId>>)> = by_key.into_iter().collect();
        let routes: Vec<Vec<Route>> = search_map(&ctx, &groups, |search, (k, candidates)| {
            let mut added: Vec<Route> = Vec::new();
            for links in candidates {
                if added.len() >= max_routes.saturating_sub(sets.route_range(*k).len()).max(1) {
                    break;
                }
                if !is_known(sets, *k, &added, links) {
                    let route = with_overlap(search, sets, *k, &added, links.clone());
                    added.push(route);
                }
            }
            added
        });
        let mut out = Additions { searches, ..Additions::default() };
        for ((k, _), added) in groups.iter().zip(routes) {
            if !added.is_empty() {
                out.routes.push((*k, added));
            }
        }
        out
    }

    /// The searches to make: for each key with routed trips (full sets too, S268), the
    /// departures to search at (evenly spread quantiles of its trips' departures), flat, with
    /// each key's range. Keys ascend.
    fn plan(&self, cx: &UpdateContext<'_>) -> (Vec<u32>, Vec<Job>) {
        let sets = cx.route_sets;
        // (key, departure) packed so that one sort orders by key, then departure.
        let mut pairs: Vec<u64> = (0..cx.trips.len())
            .filter(|&i| cx.active.is_none_or(|a| a[i as usize]))
            .filter_map(|i| {
                let k = sets.key_index(cx.trip_keys[i as usize])?;
                // A full set is searched too: a better route replaces one of its own (S268).
                (!sets.route_range(k).is_empty()).then(|| {
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

/// Room in the sets that `additions` would take past `max_routes` (S268): each such set drops
/// routes **nobody chose** in the last choice (by `chosen` and `weight` per trip, as
/// `RouteChoices` hold them), the highest free-flow cost first, never its free-flow shortest
/// one. Additions that cannot fit are dropped. Returns which of the store's routes to remove.
///
/// # Panics
///
/// Panics if an addition names a key whose set is empty (a store never holds one).
#[must_use]
pub fn make_room(
    sets: &RouteSets,
    additions: &mut Vec<(usize, Vec<Route>)>,
    chosen: &[u32],
    weight: &[u32],
    max_routes: usize,
) -> Vec<bool> {
    let mut usage = vec![0_u64; sets.route_count()];
    for (&r, &w) in chosen.iter().zip(weight) {
        if let Some(u) = usage.get_mut(r as usize) {
            *u += u64::from(w);
        }
    }
    let mut removed = vec![false; sets.route_count()];
    for (k, routes) in additions.iter_mut() {
        let range = sets.route_range(*k);
        let range_len = range.len();
        let excess = (range_len + routes.len()).saturating_sub(max_routes);
        if excess == 0 {
            continue;
        }
        let cost = |r: usize| sets.route(r).cost;
        let shortest = range
            .clone()
            .min_by(|&a, &b| cost(a).total_cmp(&cost(b)).then(a.cmp(&b)))
            .expect("a set is never empty");
        // Only a route nobody chose makes room: no trip is moved, so the averaging of the
        // iterations is not disturbed (a forced move set back Lyon's convergence, S268).
        let mut candidates: Vec<usize> =
            range.filter(|&r| r != shortest && usage[r] == 0).collect();
        candidates.sort_by(|&a, &b| cost(b).total_cmp(&cost(a)).then(a.cmp(&b)));
        // The new routes that fit once every removable route is gone; none: nothing removed. A
        // set may already hold more than the limit (a generator's, under a lower limit).
        let keep_new = routes.len().saturating_sub(excess - excess.min(candidates.len()));
        if keep_new == 0 {
            routes.clear();
            continue;
        }
        let needed = (range_len + keep_new).saturating_sub(max_routes);
        for &r in &candidates[..needed] {
            removed[r] = true;
        }
        routes.truncate(keep_new);
    }
    additions.retain(|(_, routes)| !routes.is_empty());
    removed
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
