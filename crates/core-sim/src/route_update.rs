//! Route updates: growing the choice sets between iterations.
//!
//! Route sets are made once, at free-flow costs. When traffic has rearranged the costs
//! the network holds routes the sets do not, and an equilibration over the fixed sets
//! stops short of the network's own equilibrium: the **network gap**
//! ([`IterationReport::gap_network`](crate::IterationReport::gap_network)) stays large
//! while the gap against the set looks fine (found). A
//! [`RouteUpdate`] closes that: after each loading it may **add routes** to the sets, found on
//! the link times that loading produced, before travellers choose again. **The set is fixed
//! *within* an iteration and may grow *between* iterations**; the
//! equilibrium is then over the set the generator and the update rule produce, which is why
//! the update is part of the run's description.
//!
//! Built in:
//!
//! | name | what it does |
//! |---|---|
//! | `none` (the default, [`DEFAULT_UPDATE`]) | Nothing: the sets stay as the generator made them, and the run costs nothing extra |
//! | `sampled` | `best_response` and, beside it, **sampled candidates** (S298): for each of `draws` preference draws per iteration, one perturbed generalised-cost tree per origin, departure bin and user class (the class's extra seconds per link for length, money and user values, drawn around its coefficients; every link's cost times a random factor), whose route to a pair is added if it is new and its travel time at the bin's middle is within `max_detour` of the set's best there; never to a full set. The set then holds competitive routes, not only ones that were once the fastest |
//! | `best_response` | For every origin–destination pair, the fastest route at the congested times, read from the run's **shared search trees** (one bounded time-dependent search per origin and departure bin, [`crate::trees`]; S294) or, with `tree_step_s` 0, found by one bounded search per pair at the median departure of the pair's trips; it is **added if it is new and at least as fast as the set's best route**: a tie counts, since an equally good alternative is one more to spread over. Column generation, as in MATSim's re-routing, SUMO's `duaIterate` and DTALite |
//!
//! **What `best_response` costs.** With the shared trees (the default) nothing is searched: the
//! run builds the trees for its gap and the update reads each pair's route off its tree, at the
//! bins of the departures it would have searched at (`searches` quantiles of the pair's). With
//! `tree_step_s` 0, one search per pair per iteration (`searches` of them), each
//! bounded by the best time the set already offers, so it explores what could beat that and no
//! more: about 0.5 ms of core time on Stockholm's inner city, 1.2 ms on Seoul, in
//! parallel over pairs. Either way a pair is **not searched or read while its best route is
//! within `slack` (2%) of free flow**, which is most pairs when demand is light or moderate. Then one copy of the store
//! ([`RouteSets::extended`]), and a chooser rebuilt on it. **What it adds to the store:** a
//! pair ends with about four routes; at most `max_routes`. **Light demand adds little,
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
    ClassExtras, ExtrasRequest, MAX_ROUTES_PER_SET, Route, RouteKey, RouteSets, Search,
    SearchContext, search_map,
};
use openmobisim_core_types::ids::{EntityId, LinkId, NodeId, TripId};

use crate::link_times::LinkTimes;
use crate::trees::{DrawTrees, SearchTrees};

/// The share of free flow within which `best_response` does not search a pair (see
/// [`BestResponse::slack`]).
pub const DEFAULT_SLACK: f64 = 0.02;

/// The departure bin of `best_response`'s search trees, in seconds (see
/// [`BestResponse::tree_step_s`]): the equilibration's default cost bin (S294; 0: a search per
/// pair). The run's shared trees ([`crate::trees`]) are binned by it.
pub const DEFAULT_TREE_STEP_S: u32 = 300;

/// The most routes a pair's set holds by default. A traveller is offered at most five of
/// them (`Run::with_choice_max_routes`): the set remembers more, so that a route that
/// congestion made slow can come back, and a full set does not hold the run back (with five
/// stored, Lyon's run ended poor instead of acceptable).
pub const DEFAULT_MAX_ROUTES: u32 = 10;

/// How much faster than a full set's best a route must be to replace one of its routes, in
/// seconds: a tie keeps the set as it is.
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
    /// The whole seconds after a car trip's departure its car enters the roads (S295): the
    /// departures searched and costed start there, as the loading does.
    pub car_start_s: u32,
    /// Each trip's origin-destination pair, snapped to nodes, indexed by trip.
    pub trip_keys: &'a [RouteKey],
    /// The sets as they are now.
    pub route_sets: &'a RouteSets,
    /// The times the last loading produced.
    pub times: &'a LinkTimes,
    /// The iteration the routes are for: the one that will choose among them (0 while the
    /// free-flow loading is built up).
    pub iteration: u32,
    /// The trips whose pairs are searched, by trip (one group of the free-flow loading's
    /// increments); every trip's if `None`.
    pub active: Option<&'a [bool]>,
    /// The run's shared search trees on `times` ([`crate::trees`]), for the same trips: an
    /// update with search trees reads them instead of searching, when their bin is the one it
    /// asks for. `None` where the run built none.
    pub trees: Option<&'a SearchTrees>,
    /// Each trip's user class index, by trip (the `sampled` update's per-class draws); may be
    /// empty for an update that reads no class.
    pub trip_class: &'a [u32],
    /// The extra seconds per link by class and preference draw an update asked for
    /// ([`RouteUpdate::extras_wanted`]), composed by the run; `None` where none asked.
    pub extras: Option<&'a ClassExtras>,
}

/// The routes an update found.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Additions {
    /// `(key index, routes to add)`, in **strictly ascending key order**, one entry per key,
    /// none empty. Routes are new (not in the key's set), from the key's origin to its
    /// destination, and carry their free-flow cost and their overlap with the set.
    pub routes: Vec<(usize, Vec<Route>)>,
    /// How many searches it took; with search trees ([`BestResponse::tree_step_s`]), how many
    /// pairs the trees were read for, each once (one tree answers many, and a pair's trips may
    /// span several bins), so that the routes added per search mean the same either way.
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

    /// The departure bin, in seconds, of the search trees this update reads
    /// ([`UpdateContext::trees`]), or `None` for an update that searches on its own. The run
    /// bins its shared trees by it, so that the update and the gaps read the same trees; with
    /// `None` the trees are binned by the equilibration's cost bin, for the gaps alone.
    fn tree_step_s(&self) -> Option<u32> {
        None
    }

    /// The routes to add, given the state after a loading. Must be a function of `cx`
    /// alone: the same for any thread count.
    fn update(&self, cx: &UpdateContext<'_>) -> Additions;

    /// The most routes a set may hold, if the update caps it: a set its additions would take
    /// past this makes room ([`make_room`]).
    fn max_routes(&self) -> Option<usize> {
        None
    }

    /// The extra costs the update wants, if it searches by generalised cost (S298): `None`
    /// (the default) for one that searches by time. The run composes them and passes them in
    /// [`UpdateContext::extras`].
    fn extras_wanted(&self) -> Option<ExtrasRequest> {
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
    /// cost of more searches and a bigger set, which raises the logit's own gap (1
    /// measured best). With search trees ([`Self::tree_step_s`]) the pair is read off the trees
    /// at the bins of the same departures, so the sets grow by the same rule.
    pub searches: u32,
    /// The most routes a pair's set may hold (1 to [`MAX_ROUTES_PER_SET`]; [`DEFAULT_MAX_ROUTES`]).
    /// **A full set is still searched:** a route at least [`REPLACE_GAIN_S`] faster than
    /// its best takes the place of a route nobody chose in the last choice (never its free-flow
    /// shortest one, which the slack rests on); if every route is in use, it waits. No trip is
    /// moved. Ties do not replace, so equal routes do not cycle in and out.
    pub max_routes: u32,
    /// A pair is **not searched** while the set's best route at its departure is within this share
    /// of the route's free-flow time (0 to 1, 0 searches every pair, [`DEFAULT_SLACK`]). No
    /// route can be faster than free flow, so a pair this close to it has at most this much to gain:
    /// **the regret left is bounded by `slack`**. Measured (`msa` × 8–10 iterations, the
    /// disequilibrium unchanged to the third decimal): at 2%, Manhattan's searches fell from 62 877
    /// to 7 569 and the run from 5.9 s to 4.0 s, Singapore's from 27 576 to 313, Stockholm's inner
    /// city (heavy) from 78 512 to 46 205 (−8% of the run); at 5%, 58% of Stockholm's searches.
    /// Assumes the method's routes include a free-flow shortest one, as every built-in method's do.
    pub slack: f64,
    /// **Search trees** ([`DEFAULT_TREE_STEP_S`], the equilibration's default cost bin): above
    /// 0, the trips' departures are binned in steps of this many seconds and the pairs of one
    /// origin in one bin share **one search tree**, which finds each one's fastest route at the
    /// bin's middle at once ([`crate::trees`]); a pair is read at the bins of its
    /// [`Self::searches`] departures. The run builds the trees once per loading, binned by
    /// this step, and its gaps read the same trees, so the update searches nothing of its own
    /// (a larger step: fewer trees, and the gaps' network best read at coarser bins).
    /// Approximate: the routes are the fastest at the bin's middle. 0 searches each pair on its
    /// own, at the departures [`Self::searches`] picks (exact at those seconds; one search per
    /// pair per iteration, which costs more than the trees wherever pairs share origins), and the
    /// run bins the gaps' trees by the cost bin.
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

    fn tree_step_s(&self) -> Option<u32> {
        (self.tree_step_s > 0).then_some(self.tree_step_s)
    }

    fn name(&self) -> &str {
        "best_response"
    }

    fn descriptor(&self) -> String {
        format!(
            "best_response;max_routes={};searches={};slack={};tree_step_s={}",
            self.max_routes, self.searches, self.slack, self.tree_step_s
        )
    }

    fn update(&self, cx: &UpdateContext<'_>) -> Additions {
        if self.tree_step_s > 0 {
            let step = f64::from(self.tree_step_s);
            // The run's shared trees, binned by this step; the update's own if the run built
            // none or binned them otherwise (a caller outside a run).
            return match cx.trees {
                Some(trees) if trees.step_s().to_bits() == step.to_bits() => {
                    self.update_from_trees(cx, trees)
                }
                _ => {
                    let own = SearchTrees::build(
                        cx.network,
                        cx.turns,
                        cx.times,
                        cx.trips,
                        cx.trip_keys,
                        cx.route_sets,
                        cx.active,
                        cx.car_start_s,
                        step,
                    );
                    self.update_from_trees(cx, &own)
                }
            };
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

impl BestResponse {
    /// [`RouteUpdate::update`] from search trees ([`Self::tree_step_s`]): each pair is read at
    /// the bins of the departures [`Self::plan`] picks for it (its [`Self::searches`] quantiles;
    /// the median with 1), as a search per pair would search at those departures, so the sets
    /// grow by the same rule either way (S294). The tree's route to the pair's destination is
    /// kept if the pair is worth searching (past the slack) and the route is new
    /// and no slower than the pair's own best at the tree's departure (a full set: faster by
    /// [`REPLACE_GAIN_S`]). Then the kept routes are made routes of their sets (their overlap),
    /// key by key.
    fn update_from_trees(&self, cx: &UpdateContext<'_>, trees: &SearchTrees) -> Additions {
        let (by_key, searches) = self.candidates_from_trees(cx, trees);
        let mut out = cost_candidates(cx, by_key, self.max_routes as usize);
        out.searches = searches;
        out
    }

    /// The routes the trees offer each key, by key index, in the order read, and how many pairs
    /// were read: the first half of [`Self::update_from_trees`].
    fn candidates_from_trees(
        &self,
        cx: &UpdateContext<'_>,
        trees: &SearchTrees,
    ) -> (BTreeMap<usize, Vec<Vec<LinkId>>>, u32) {
        let sets = cx.route_sets;
        let max_routes = self.max_routes as usize;
        let (departures, jobs) = self.plan(cx);
        // The pairs read, each once however many departures it is read at: what a search per
        // pair would have counted, which the stopping rule weighs the routes added against
        // (`route_growth_tolerance`).
        let mut searches = 0_u32;
        let mut by_key: BTreeMap<usize, Vec<Vec<LinkId>>> = BTreeMap::new();
        for job in &jobs {
            let k = job.key;
            let key = sets.keys()[k];
            let set = sets.route_range(k);
            if key.origin == key.destination || set.is_empty() {
                continue;
            }
            let full = set.len() >= max_routes;
            let free_flow =
                set.clone().map(|r| f64::from(sets.route(r).cost)).fold(f64::INFINITY, f64::min);
            let mut read = false;
            for &second in &departures[job.departures.clone()] {
                let Some(tree) = trees.tree(key.origin, f64::from(second)) else { continue };
                let Ok(at) = tree.keys.binary_search_by_key(&k, |&(k, _)| k) else { continue };
                let best = tree.keys[at].1;
                // At most `slack` left to gain here: nothing beats free flow.
                if self.slack > 0.0 && best <= free_flow * (1.0 + self.slack) {
                    continue;
                }
                read = true;
                let Some(found) = tree.route_to(key.destination) else { continue };
                // At least as fast as the set's best: a tie counts (S176), but for a full set,
                // which takes only a route that beats its best (S268).
                let good = if full {
                    found.seconds < best - REPLACE_GAIN_S
                } else {
                    found.seconds <= best + 1e-9
                };
                if !good {
                    continue;
                }
                let links: Vec<LinkId> = found.links.iter().map(|&l| LinkId::new(l)).collect();
                let known = by_key.get(&k).is_some_and(|added| added.contains(&links));
                if !known && !is_known(sets, k, &[], &links) {
                    by_key.entry(k).or_default().push(links);
                }
            }
            if read {
                searches += 1;
            }
        }
        (by_key, searches)
    }

    /// The searches to make: for each key with routed trips (full sets too), the
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
                    let departure = cx.trips.departure(TripId::new(i)).get() + cx.car_start_s;
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

/// The candidates of each key made routes of their sets (their free-flow cost and their overlap
/// with the set and with the candidates before them), as many as the set has room for under
/// `max_routes` (a full set takes one); keys ascend. `searches` is left 0 for the caller to set.
fn cost_candidates(
    cx: &UpdateContext<'_>,
    by_key: BTreeMap<usize, Vec<Vec<LinkId>>>,
    max_routes: usize,
) -> Additions {
    let sets = cx.route_sets;
    let ctx = SearchContext::new(cx.network, cx.turns);
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
    let mut out = Additions::default();
    for ((k, _), added) in groups.iter().zip(routes) {
        if !added.is_empty() {
            out.routes.push((*k, added));
        }
    }
    out
}

/// The sampled update (S298): the best response, and beside it candidates drawn on the
/// congested **generalised** costs. See the [module docs](self).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sampled {
    /// The best-response part: its `searches`, `max_routes`, `slack` and `tree_step_s` (above 0:
    /// the sampled update reads trees only).
    pub inner: BestResponse,
    /// Perturbed preference draws per iteration (0 to 16; 1 by default): each one perturbed tree
    /// per origin, departure bin and user class.
    pub draws: u32,
    /// Attribute noise: a link's cost times `1 + sigma · u`, `u` uniform in [0, 1) (0.5 by
    /// default; at least 0).
    pub sigma: f64,
    /// The preference draws' spread: each coefficient but time's times `1 + tau · v`, `v` uniform
    /// in [−1, 1) (0.3 by default; at least 0).
    pub tau: f64,
    /// A sampled candidate's travel time at the bin's middle is at most this times the set's
    /// best there (1.5 by default; at least 1).
    pub max_detour: f64,
    /// The seed of the draws (0 by default); each iteration draws afresh from it.
    pub seed: u64,
}

impl Default for Sampled {
    fn default() -> Self {
        Self {
            inner: BestResponse::default(),
            draws: 1,
            sigma: 0.5,
            tau: 0.3,
            max_detour: 1.5,
            seed: 0,
        }
    }
}

impl Sampled {
    const OPTIONS: [&'static str; 9] = [
        "draws",
        "max_detour",
        "max_routes",
        "searches",
        "seed",
        "sigma",
        "slack",
        "tau",
        "tree_step_s",
    ];

    /// Make the update from its options, defaults for those not given.
    ///
    /// # Errors
    ///
    /// [`RouteUpdateError::UnknownOption`] for a name it does not have,
    /// [`RouteUpdateError::BadOption`] for a value out of range (`tree_step_s` 0 among them:
    /// the sampled update reads trees).
    pub fn from_options(options: &Options) -> Result<Self, RouteUpdateError> {
        check_known("sampled", options, &Self::OPTIONS)?;
        let bad = |option: &str, reason: String| RouteUpdateError::BadOption {
            update: "sampled".to_string(),
            option: option.to_string(),
            reason,
        };
        let mut inner_options = Options::new();
        let mut m = Self::default();
        for (option, &v) in options {
            match option.as_str() {
                "draws" => {
                    if !(v.is_finite() && v.fract() == 0.0 && (0.0..=16.0).contains(&v)) {
                        return Err(bad(
                            option,
                            format!("must be a whole number from 0 to 16, got {v}"),
                        ));
                    }
                    #[allow(
                        clippy::cast_possible_truncation,
                        clippy::cast_sign_loss,
                        reason = "checked to be a whole number from 0 to 16"
                    )]
                    {
                        m.draws = v as u32;
                    }
                }
                "sigma" | "tau" => {
                    if !(v.is_finite() && v >= 0.0) {
                        return Err(bad(option, format!("must be at least 0, got {v}")));
                    }
                    if option == "sigma" {
                        m.sigma = v;
                    } else {
                        m.tau = v;
                    }
                }
                "max_detour" => {
                    if !(v.is_finite() && v >= 1.0) {
                        return Err(bad(option, format!("must be at least 1, got {v}")));
                    }
                    m.max_detour = v;
                }
                "seed" => {
                    if !(v.is_finite()
                        && v >= 0.0
                        && v.fract() == 0.0
                        && v <= 9_007_199_254_740_992.0)
                    {
                        return Err(bad(
                            option,
                            format!("must be a whole number from 0 to 2^53, got {v}"),
                        ));
                    }
                    #[allow(
                        clippy::cast_possible_truncation,
                        clippy::cast_sign_loss,
                        reason = "checked to be a whole number within range"
                    )]
                    {
                        m.seed = v as u64;
                    }
                }
                _ => {
                    inner_options.insert(option.clone(), v);
                }
            }
        }
        m.inner = BestResponse::from_options(&inner_options).map_err(|e| match e {
            RouteUpdateError::BadOption { option, reason, .. } => {
                RouteUpdateError::BadOption { update: "sampled".to_string(), option, reason }
            }
            other => other,
        })?;
        if m.inner.tree_step_s == 0 {
            return Err(bad(
                "tree_step_s",
                "must be above 0: the sampled update reads search trees".to_string(),
            ));
        }
        Ok(m)
    }
}

impl RouteUpdate for Sampled {
    fn max_routes(&self) -> Option<usize> {
        Some(self.inner.max_routes as usize)
    }

    fn tree_step_s(&self) -> Option<u32> {
        Some(self.inner.tree_step_s)
    }

    fn name(&self) -> &str {
        "sampled"
    }

    fn descriptor(&self) -> String {
        format!(
            "sampled;draws={};max_detour={};max_routes={};searches={};seed={};sigma={};slack={};tau={};tree_step_s={}",
            self.draws,
            self.max_detour,
            self.inner.max_routes,
            self.inner.searches,
            self.seed,
            self.sigma,
            self.inner.slack,
            self.tau,
            self.inner.tree_step_s
        )
    }

    fn extras_wanted(&self) -> Option<ExtrasRequest> {
        Some(ExtrasRequest { draws: self.draws, tau: self.tau, seed: self.seed })
    }

    fn update(&self, cx: &UpdateContext<'_>) -> Additions {
        let step = f64::from(self.inner.tree_step_s);
        // Draw 0: the best response, read from the run's trees (or the update's own).
        let own;
        let trees = match cx.trees {
            Some(trees) if trees.step_s().to_bits() == step.to_bits() => trees,
            _ => {
                own = SearchTrees::build(
                    cx.network,
                    cx.turns,
                    cx.times,
                    cx.trips,
                    cx.trip_keys,
                    cx.route_sets,
                    cx.active,
                    cx.car_start_s,
                    step,
                );
                &own
            }
        };
        let (mut by_key, searches) = self.inner.candidates_from_trees(cx, trees);
        // Draws 1…: perturbed trees per class; a candidate within the detour of the set's best
        // at the bin enters a set with room.
        let sets = cx.route_sets;
        let max_routes = self.inner.max_routes as usize;
        let time_only: ClassExtras = vec![vec![None; self.draws as usize + 1]];
        let extras: &ClassExtras = cx.extras.filter(|e| !e.is_empty()).unwrap_or(&time_only);
        let (departures, jobs) = self.inner.plan(cx);
        let iteration_seed =
            self.seed ^ u64::from(cx.iteration).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        for draw in 1..=self.draws as usize {
            let drawn = DrawTrees::build(
                cx.network,
                cx.turns,
                cx.times,
                cx.trips,
                cx.trip_keys,
                cx.trip_class,
                sets,
                cx.active,
                cx.car_start_s,
                step,
                extras,
                draw,
                iteration_seed,
                self.sigma,
                self.max_detour,
            );
            for job in &jobs {
                let k = job.key;
                let key = sets.keys()[k];
                let set = sets.route_range(k);
                if key.origin == key.destination || set.is_empty() || set.len() >= max_routes {
                    continue;
                }
                let free_flow = set
                    .clone()
                    .map(|r| f64::from(sets.route(r).cost))
                    .fold(f64::INFINITY, f64::min);
                for &second in &departures[job.departures.clone()] {
                    for class in 0..extras.len() {
                        #[allow(clippy::cast_possible_truncation, reason = "a handful of classes")]
                        let Some(tree) = drawn.tree(key.origin, class as u32, f64::from(second))
                        else {
                            continue;
                        };
                        let Ok(at) = tree.keys.binary_search_by_key(&k, |&(k, _)| k) else {
                            continue;
                        };
                        let best = tree.keys[at].1;
                        if self.inner.slack > 0.0 && best <= free_flow * (1.0 + self.inner.slack) {
                            continue;
                        }
                        let Some(found) = tree.route_to(key.destination) else { continue };
                        if found.seconds > self.max_detour * best + 1e-9 {
                            continue;
                        }
                        let links: Vec<LinkId> =
                            found.links.iter().map(|&l| LinkId::new(l)).collect();
                        let known = by_key.get(&k).is_some_and(|added| added.contains(&links));
                        if !known && !is_known(sets, k, &[], &links) {
                            by_key.entry(k).or_default().push(links);
                        }
                    }
                }
            }
        }
        let mut out = cost_candidates(cx, by_key, max_routes);
        out.searches = searches;
        out
    }
}

/// Room in the sets that `additions` would take past `max_routes`: each such set drops
/// routes **nobody chose** in the last choice (by `chosen` and `weight` per trip, as
/// `RouteChoices` hold them, and never a route `kept` names: one a trip choosing its mode drives
/// as its car alternative, which no route choice names), the highest free-flow cost first,
/// never its free-flow shortest one. Additions that cannot fit are dropped. Returns which of the
/// store's routes to remove.
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
    kept: &[bool],
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
        let mut candidates: Vec<usize> = range
            .filter(|&r| r != shortest && usage[r] == 0 && !kept.get(r).copied().unwrap_or(false))
            .collect();
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
    /// The built-in updates: `none`, `best_response` and `sampled`.
    #[must_use]
    pub fn builtin() -> Self {
        let mut r = Self { updates: Vec::new() };
        r.register("none", |o| Ok(Box::new(NoRouteUpdate::from_options(o)?)));
        r.register("best_response", |o| Ok(Box::new(BestResponse::from_options(o)?)));
        r.register("sampled", |o| Ok(Box::new(Sampled::from_options(o)?)));
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
