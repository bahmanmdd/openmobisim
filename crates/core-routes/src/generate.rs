//! Route-set generation behind one interface.
//!
//! **A method is a [`RouteSetGenerator`]**: given a search and an origin and a
//! destination node, it returns that pair's routes. Everything else — the
//! store, the parallel driver, the fingerprint, the Python surface — is
//! written against the trait, so a new method (exact label-constrained
//! k-shortest, plateau/via-node, a sampled or learned generator, a multimodal
//! one) is one new type and one line in a [`Registry`]; nothing else changes.
//!
//! Built in:
//!
//! | name | what it does |
//! |---|---|
//! | `penalty` (the default, [`DEFAULT_METHOD`]) | The shortest route, then repeatedly the shortest under penalties on the links already used, keeping routes that stay within a detour bound and an overlap limit |
//! | `shortest` | The single shortest route |
//! | `montecarlo` | The shortest route, then the shortest under **random link costs biased towards the links likely to congest**: distinct routes that avoid what the demand will jam |
//! | `sampled` | The shortest route, then the cheapest by each user class's **generalised cost** (travel time plus extras per link), in preference draws and under random link costs: distinct routes that differ in what travellers value |
//!
//! A method's **options** are numbers by name ([`Options`]), validated when the
//! method is made, and its **descriptor** — its name and every option with
//! defaults filled in — is part of a store's identity, so a set made with
//! other settings is never mistaken for this one.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_types::hash::Fnv1a;
use openmobisim_core_types::ids::{EntityId, LinkId, NodeId};

use crate::search::{Extras, MAX_ROUTES_PER_SET, Route, Search, SearchContext};
use crate::store::{RouteKey, RouteSets};

/// The method used unless another is asked for.
pub const DEFAULT_METHOD: &str = "penalty";

/// A method's options: numbers by name. Unknown names are an error, not ignored.
pub type Options = BTreeMap<String, f64>;

/// Why a method could not be made.
#[derive(Clone, PartialEq, Debug)]
pub enum RouteError {
    /// No method has this name.
    UnknownMethod {
        /// The name asked for.
        name: String,
        /// The names that exist.
        known: Vec<String>,
    },
    /// The method has no such option.
    UnknownOption {
        /// The method.
        method: String,
        /// The option asked for.
        option: String,
        /// The options the method has.
        known: Vec<&'static str>,
    },
    /// An option's value is not allowed.
    BadOption {
        /// The method.
        method: String,
        /// The option.
        option: String,
        /// What is wrong with it.
        reason: String,
    },
}

impl fmt::Display for RouteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownMethod { name, known } => {
                write!(f, "unknown route-set method {name:?}; choose from {known:?}")
            }
            Self::UnknownOption { method, option, known } => {
                write!(f, "method {method:?} has no option {option:?}; its options are {known:?}")
            }
            Self::BadOption { method, option, reason } => {
                write!(f, "method {method:?}, option {option:?}: {reason}")
            }
        }
    }
}

impl std::error::Error for RouteError {}

/// A route-set generation method.
///
/// Implementations must be **deterministic**: the same search, origin and
/// destination give the same routes in the same order, whatever thread runs
/// them and whatever else has been generated (the search leaves no state
/// behind).
pub trait RouteSetGenerator: Send + Sync {
    /// The method's name, as it is selected.
    fn name(&self) -> &str;

    /// The method's name and every option with defaults filled in, as a
    /// canonical string. It is part of a store's identity.
    fn descriptor(&self) -> String;

    /// The routes from `origin` to `destination`, best first, or none if there
    /// is no route. `origin` and `destination` differ.
    fn generate(&self, search: &mut Search<'_>, origin: NodeId, destination: NodeId) -> Vec<Route>;

    /// Whether [`Self::generate_from`] finds the routes from one origin in one search, so that
    /// [`RouteSets::generate`] hands it each origin's pairs together. False by default:
    /// the pairs are then handed out one by one, which spreads the work more evenly.
    fn searches_trees(&self) -> bool {
        false
    }

    /// The routes from `origin` to each of `destinations` (distinct, none of them `origin`), in
    /// their order: one [`Self::generate`] each, unless the method finds them in one search
    /// ([`Self::searches_trees`]).
    fn generate_from(
        &self,
        search: &mut Search<'_>,
        origin: NodeId,
        destinations: &[NodeId],
    ) -> Vec<Vec<Route>> {
        destinations.iter().map(|&d| self.generate(search, origin, d)).collect()
    }

    /// Whether the method reads the demand it generates for (see [`Self::with_demand`]).
    /// A run assembles the demand only if it does, so a method that does not costs nothing.
    fn reads_demand(&self) -> bool {
        false
    }

    /// The same method, made aware of the demand it is about to generate for, or
    /// `None` if it does not read it. Called once, before [`RouteSets::generate`], by whoever
    /// generates for a demand (a run; `route_sets_build`). The Monte Carlo method uses it to
    /// find the links likely to congest ([`congestion_propensity`]).
    fn with_demand(&self, _demand: &Demand<'_>) -> Option<Box<dyn RouteSetGenerator>> {
        None
    }

    /// The extra costs the method wants, if it searches by generalised cost (S298): `None`
    /// (the default) for a method that searches by time. A run that gets `Some` composes the
    /// extras and calls [`Self::with_extras`] before generating.
    fn extras_wanted(&self) -> Option<ExtrasRequest> {
        None
    }

    /// The same method, given the extras it asked for ([`Self::extras_wanted`]), or `None` if
    /// it does not take them. Called once, before [`RouteSets::generate`].
    fn with_extras(&self, _extras: ClassExtras) -> Option<Box<dyn RouteSetGenerator>> {
        None
    }

    /// The same method, prepared for the context it is about to search in (what needs the
    /// context's costs: the A\* bounds of its extras), or `None` if it needs no preparation.
    /// Called once by [`RouteSets::generate_in`], before the searches.
    fn with_context(
        &self,
        _ctx: &crate::search::SearchContext<'_>,
    ) -> Option<Box<dyn RouteSetGenerator>> {
        None
    }
}

/// What a method that searches by **generalised cost** asks the run for (S298): extra seconds
/// per link for every user class, in `draws` preference draws each (draw 0 unperturbed), the
/// coefficients drawn with spread `tau` from `seed`. The run composes them from the choice
/// model's coefficients, the prices and the link values (`core-sim`'s `generalised_cost`) and
/// hands them back with [`RouteSetGenerator::with_extras`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExtrasRequest {
    /// How many perturbed preference draws, besides draw 0.
    pub draws: u32,
    /// The spread of a preference draw: each coefficient times `1 + tau · v`, `v` in [−1, 1).
    pub tau: f64,
    /// The seed of the draws.
    pub seed: u64,
}

/// Extra seconds per link by user class and draw: `extras[class][draw]` has one value per link
/// of the network, or is `None` where the class's cost is travel time alone.
pub type ClassExtras = Vec<Vec<Option<Arc<[f64]>>>>;

/// One trip as a method that reads the demand sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TripDemand {
    /// Its origin-destination pair.
    pub key: RouteKey,
    /// How many people it stands for.
    pub weight: u32,
    /// The second it leaves.
    pub departure: u32,
}

/// The demand a set of routes is generated for: every trip, over a network.
#[derive(Clone, Copy, Debug)]
pub struct Demand<'a> {
    /// The network.
    pub network: &'a RoadNetwork,
    /// Its turns.
    pub turns: &'a TurnTable,
    /// Every trip, in any order.
    pub trips: &'a [TripDemand],
}

/// The length of the departure bins in which [`congestion_propensity`] counts traffic.
pub const PROPENSITY_BIN_SECONDS: u32 = 1800;

/// How likely each link is to congest under `demand`, from 0 to 1, by link id: **the
/// busiest departure half-hour's load over the link's capacity**, with every trip on
/// its pair's free-flow shortest route (the "static congestion propensity").
///
/// For each link, the persons whose shortest route uses it and who leave in one
/// [`PROPENSITY_BIN_SECONDS`]-long bin, over what the link can carry in that time
/// (`capacity × bin`), taking the busiest bin and clipping at 1. A link nobody's shortest
/// route uses is 0; one loaded to capacity or beyond is 1. It is a static, crude measure of
/// where demand meets capacity: it ignores that a vehicle reaches a link after leaving, and
/// that traffic spreads once the shortest routes jam, which is what the routes it helps find
/// are for.
///
/// **Cost:** one shortest route per distinct pair (the [`Shortest`] method, in parallel) and
/// a pass over the trips and the links of their routes; memory 12 bytes per link. Pairs with
/// no route, and trips from a node to itself, count for nothing. The result does not
/// depend on the order of `demand.trips` or on threads: loads are sums of whole numbers.
///
/// # Panics
///
/// Panics if the routes of the pairs exceed the `u32` id space of the store.
#[must_use]
pub fn congestion_propensity(demand: &Demand<'_>) -> Vec<f32> {
    let network = demand.network;
    let links = network.link_count() as usize;
    let mut keys: Vec<RouteKey> =
        demand.trips.iter().map(|t| t.key).filter(|k| k.origin != k.destination).collect();
    keys.sort_unstable();
    keys.dedup();
    let shortest = RouteSets::generate(network, demand.turns, &keys, &Shortest);
    // (bin of departure, pair, persons): sorted, so a bin's trips are together and a pair's
    // within it, whatever the order they came in.
    let mut rows: Vec<(u32, u32, u32)> = demand
        .trips
        .iter()
        .filter_map(|t| {
            let k = shortest.key_index(t.key)?;
            let routed = !shortest.route_range(k).is_empty();
            routed.then(|| {
                (
                    t.departure / PROPENSITY_BIN_SECONDS,
                    u32::try_from(k).expect("keys fit u32"),
                    t.weight,
                )
            })
        })
        .collect();
    rows.sort_unstable();

    let mut peak = vec![0.0_f64; links];
    let mut load = vec![0.0_f64; links];
    let mut touched: Vec<u32> = Vec::new();
    let mut flush = |load: &mut Vec<f64>, touched: &mut Vec<u32>| {
        for &l in touched.iter() {
            let capacity = network.link_parameters(LinkId::new(l)).capacity.get();
            let ratio = if capacity > 0.0 {
                load[l as usize] / (f64::from(PROPENSITY_BIN_SECONDS) * capacity)
            } else {
                1.0
            };
            let p = &mut peak[l as usize];
            *p = p.max(ratio);
            load[l as usize] = 0.0;
        }
        touched.clear();
    };
    let mut i = 0;
    while i < rows.len() {
        let bin = rows[i].0;
        while i < rows.len() && rows[i].0 == bin {
            let key = rows[i].1;
            let mut persons = 0.0;
            while i < rows.len() && rows[i].0 == bin && rows[i].1 == key {
                persons += f64::from(rows[i].2);
                i += 1;
            }
            let first = shortest.route_range(key as usize).start;
            for &l in shortest.route(first).links {
                if load[l as usize] == 0.0 {
                    touched.push(l);
                }
                load[l as usize] += persons;
            }
        }
        flush(&mut load, &mut touched);
    }
    #[allow(clippy::cast_possible_truncation, reason = "a ratio in [0, 1] stored as f32")]
    peak.iter().map(|&r| r.min(1.0) as f32).collect()
}

/// Sort routes by cost, then by their links, so the order is a function of the
/// routes alone.
fn sort_best_first(routes: &mut [Route]) {
    routes
        .sort_by(|a, b| a.cost.total_cmp(&b.cost).then_with(|| a.links.iter().cmp(b.links.iter())));
}

/// Sort routes by `cost(route)`, then by their links: [`sort_best_first`] in another cost.
fn sort_best_first_by(routes: &mut [Route], cost: impl Fn(&Route) -> f64) {
    let mut keyed: Vec<(f64, Route)> = routes.iter().map(|r| (cost(r), r.clone())).collect();
    keyed.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.links.iter().cmp(b.1.links.iter())));
    for (slot, (_, route)) in routes.iter_mut().zip(keyed) {
        *slot = route;
    }
}

// --- shortest ---------------------------------------------------------------------

/// The single shortest route.
#[derive(Clone, Debug, Default)]
pub struct Shortest;

impl Shortest {
    const OPTIONS: [&'static str; 0] = [];

    /// Make the method from its options (it has none).
    ///
    /// # Errors
    ///
    /// [`RouteError::UnknownOption`] for any option.
    pub fn from_options(options: &Options) -> Result<Self, RouteError> {
        check_known("shortest", options, &Self::OPTIONS)?;
        Ok(Self)
    }
}

impl RouteSetGenerator for Shortest {
    fn name(&self) -> &str {
        "shortest"
    }

    fn descriptor(&self) -> String {
        "shortest".to_string()
    }

    fn generate(&self, search: &mut Search<'_>, origin: NodeId, destination: NodeId) -> Vec<Route> {
        search.shortest(origin, destination).into_iter().collect()
    }

    fn searches_trees(&self) -> bool {
        true
    }

    /// One search tree for all of them ([`Search::shortest_tree`]).
    fn generate_from(
        &self,
        search: &mut Search<'_>,
        origin: NodeId,
        destinations: &[NodeId],
    ) -> Vec<Vec<Route>> {
        search
            .shortest_tree(origin, destinations, f64::INFINITY)
            .into_iter()
            .map(|r| r.into_iter().collect())
            .collect()
    }
}

// --- penalty ----------------------------------------------------------------------

/// The penalty method: find the shortest route, multiply the cost of the links
/// of the route just found and search again, and keep a new route if it is
/// distinct, within `max_detour` of the best cost, and shares at most
/// `max_overlap` of its cost with every route kept before.
///
/// **Cost:** at most `max_attempts` searches per pair, each bounded to routes
/// within `max_detour` of the best, so none explores more than about
/// `max_detour²` times the first. **Guarantee:** none of
/// optimality or completeness — it finds plausible, distinct alternatives, not
/// the k best. The first route is always a shortest one.
#[derive(Clone, Debug, PartialEq)]
pub struct Penalty {
    /// How many routes to keep at most (1 to 32).
    pub max_paths: usize,
    /// No route costs more than this multiple of the best route's cost (at least 1).
    pub max_detour: f64,
    /// A route shares at most this share of its cost with any earlier route (above 0, at most 1).
    pub max_overlap: f64,
    /// The factor applied to the cost of the links of each route found (above 1).
    pub penalty: f64,
    /// How many searches to try at most for one pair.
    pub max_attempts: usize,
}

impl Default for Penalty {
    fn default() -> Self {
        Self { max_paths: 5, max_detour: 1.3, max_overlap: 0.75, penalty: 1.5, max_attempts: 15 }
    }
}

impl Penalty {
    const OPTIONS: [&'static str; 5] =
        ["max_attempts", "max_detour", "max_overlap", "max_paths", "penalty"];

    /// Make the method from its options, defaults for those not given.
    ///
    /// # Errors
    ///
    /// [`RouteError::UnknownOption`] for a name it does not have,
    /// [`RouteError::BadOption`] for a value out of range.
    pub fn from_options(options: &Options) -> Result<Self, RouteError> {
        check_known("penalty", options, &Self::OPTIONS)?;
        let mut m = Self::default();
        let bad = |option: &str, reason: &str| RouteError::BadOption {
            method: "penalty".to_string(),
            option: option.to_string(),
            reason: reason.to_string(),
        };
        let whole = |option: &str, v: f64, lo: usize, hi: usize| -> Result<usize, RouteError> {
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "checked to be a non-negative whole number within a small range"
            )]
            let n = v as usize;
            if v.is_finite() && v >= 0.0 && v.fract() == 0.0 && n >= lo && n <= hi {
                Ok(n)
            } else {
                Err(bad(option, &format!("must be a whole number from {lo} to {hi}, got {v}")))
            }
        };
        if let Some(&v) = options.get("max_paths") {
            m.max_paths = whole("max_paths", v, 1, MAX_ROUTES_PER_SET)?;
            m.max_attempts = (3 * m.max_paths).max(m.max_paths);
        }
        if let Some(&v) = options.get("max_attempts") {
            m.max_attempts = whole("max_attempts", v, 1, 1000)?;
        }
        if let Some(&v) = options.get("max_detour") {
            if !(v.is_finite() && v >= 1.0) {
                return Err(bad("max_detour", &format!("must be at least 1, got {v}")));
            }
            m.max_detour = v;
        }
        if let Some(&v) = options.get("max_overlap") {
            if !(v > 0.0 && v <= 1.0) {
                return Err(bad("max_overlap", &format!("must be above 0 and at most 1, got {v}")));
            }
            m.max_overlap = v;
        }
        if let Some(&v) = options.get("penalty") {
            if !(v.is_finite() && v > 1.0) {
                return Err(bad("penalty", &format!("must be above 1, got {v}")));
            }
            m.penalty = v;
        }
        Ok(m)
    }
}

impl RouteSetGenerator for Penalty {
    fn name(&self) -> &str {
        "penalty"
    }

    fn descriptor(&self) -> String {
        format!(
            "penalty;max_attempts={};max_detour={};max_overlap={};max_paths={};penalty={}",
            self.max_attempts, self.max_detour, self.max_overlap, self.max_paths, self.penalty
        )
    }

    fn generate(&self, search: &mut Search<'_>, origin: NodeId, destination: NodeId) -> Vec<Route> {
        search.clear_route_state();
        let Some(first) = search.shortest(origin, destination) else {
            return Vec::new();
        };
        let best = first.cost;
        search.mark(&first.links, 0);
        let mut last = first.links.clone();
        let mut kept = vec![first];
        for _ in 0..self.max_attempts {
            if kept.len() >= self.max_paths {
                break;
            }
            for &l in &last {
                search.factors.multiply(l, self.penalty);
            }
            // Bounded by the detour limit: a route past it would be rejected anyway, and
            // an unbounded penalised search wanders far beyond it (S165).
            let Some(mut candidate) =
                search.shortest_within(origin, destination, self.max_detour * best)
            else {
                break;
            };
            last.clone_from(&candidate.links);
            if candidate.cost > self.max_detour * best
                || kept.iter().any(|k| k.links == candidate.links)
            {
                continue;
            }
            let overlap = search.max_overlap(&candidate.links);
            if overlap > self.max_overlap {
                continue;
            }
            candidate.overlap = overlap;
            search.mark(&candidate.links, kept.len());
            kept.push(candidate);
        }
        search.clear_route_state();
        sort_best_first(&mut kept);
        kept
    }
}

// --- montecarlo -------------------------------------------------------------------

/// [`MAX_ROUTES_PER_SET`] as the bound of an option given as a number.
#[allow(clippy::cast_precision_loss, reason = "a small constant: 32")]
const MAX_PATHS_BOUND: f64 = MAX_ROUTES_PER_SET as f64;

/// Where a Monte Carlo method's bias comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BiasSource {
    /// No bias: every link's cost is perturbed alike.
    Unbiased,
    /// From the demand ([`congestion_propensity`]), once [`RouteSetGenerator::with_demand`]
    /// has been called; unbiased until then.
    Demand,
    /// An array the caller gave ([`MonteCarlo::with_bias`]).
    Given,
}

impl BiasSource {
    fn name(self) -> &'static str {
        match self {
            Self::Unbiased => "none",
            Self::Demand => "demand",
            Self::Given => "given",
        }
    }
}

/// Monte Carlo generation, biased towards the links likely to congest.
///
/// The shortest route, then, for each of `draws` draws, the shortest route when **every link's
/// cost is multiplied by `1 + sigma · u · bias`**, `u` a uniform draw and `bias` the link's
/// propensity to congest, from 0 to 1 (a pure function of the pair, the draw and the link, so
/// the sets never depend on order or threads). A route is kept if it is new, costs at most
/// `max_detour` times the best route's free-flow cost, and shares at most `max_overlap` of its
/// cost with each route kept before, up to `max_paths`. This is the *simulation* approach to
/// choice-set generation (Bovy and Fiorenzo-Catalano 2007; Ben-Akiva's "labelling" is another
/// thing), with its noise concentrated where the demand is heavy: the routes it finds go
/// round links the demand will jam.
///
/// **The bias is the point.** Unbiased noise (`biased` 0) gives random routes, which are poor
/// routes, and a logit spreads travellers over them: on Stockholm's heavy load its network
/// gap was 0.96, against the penalty method's 0.30 and this method's 0.16. By default
/// the bias is [`congestion_propensity`] of the demand the run generates for; a caller with a
/// better idea gives its own ([`Self::with_bias`]).
///
/// **Cost:** at most `draws` bounded searches per pair, each exploring about what the
/// penalty method's do; about 4 s more than the penalty method for 9 600 pairs at 16 draws,
/// and one shortest route per distinct pair for the bias.
/// **Guarantee:** none of optimality or completeness; the first route is always a shortest one.
#[derive(Clone, Debug)]
pub struct MonteCarlo {
    /// How many random cost draws to try (0 to 1000).
    pub draws: usize,
    /// How far a link's cost may be raised: by up to `sigma` times itself, times its bias (above 0).
    pub sigma: f64,
    /// No route costs more than this multiple of the best route's cost (at least 1).
    pub max_detour: f64,
    /// A route shares at most this share of its cost with any earlier route (above 0, at most 1).
    pub max_overlap: f64,
    /// How many routes to keep at most (1 to 32).
    pub max_paths: usize,
    /// Which draws: changes every draw of every pair, so a study can vary its sets (0 by default).
    pub seed: u64,
    source: BiasSource,
    bias: Option<Arc<[f32]>>,
}

impl Default for MonteCarlo {
    fn default() -> Self {
        Self {
            draws: 16,
            sigma: 4.0,
            max_detour: 2.0,
            max_overlap: 0.9,
            max_paths: 10,
            seed: 0,
            source: BiasSource::Demand,
            bias: None,
        }
    }
}

impl MonteCarlo {
    const OPTIONS: [&'static str; 7] =
        ["biased", "draws", "max_detour", "max_overlap", "max_paths", "seed", "sigma"];

    /// Make the method from its options, defaults for those not given: 16 draws, `sigma` 4,
    /// `max_detour` 2, `max_overlap` 0.9, `max_paths` 10, `seed` 0, `biased` 1 (the bias is
    /// the demand's congestion propensity; 0 for none).
    ///
    /// # Errors
    ///
    /// [`RouteError::UnknownOption`] for a name it does not have,
    /// [`RouteError::BadOption`] for a value out of range.
    pub fn from_options(options: &Options) -> Result<Self, RouteError> {
        check_known("montecarlo", options, &Self::OPTIONS)?;
        let mut m = Self::default();
        let bad = |option: &str, reason: String| RouteError::BadOption {
            method: "montecarlo".to_string(),
            option: option.to_string(),
            reason,
        };
        let whole = |option: &str, v: f64, hi: f64| -> Result<u64, RouteError> {
            if v.is_finite() && v >= 0.0 && v.fract() == 0.0 && v <= hi {
                #[allow(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "checked to be a non-negative whole number within range"
                )]
                Ok(v as u64)
            } else {
                Err(bad(option, format!("must be a whole number from 0 to {hi}, got {v}")))
            }
        };
        for (option, &v) in options {
            #[allow(clippy::cast_possible_truncation, reason = "bounded by the checks in `whole`")]
            match option.as_str() {
                "draws" => m.draws = whole(option, v, 1000.0)? as usize,
                "max_paths" => {
                    let n = whole(option, v, MAX_PATHS_BOUND)? as usize;
                    if n == 0 {
                        return Err(bad(
                            option,
                            format!("must be a whole number from 1 to {MAX_ROUTES_PER_SET}, got 0"),
                        ));
                    }
                    m.max_paths = n;
                }
                "seed" => m.seed = whole(option, v, 9_007_199_254_740_992.0)?,
                "biased" => {
                    m.source = match whole(option, v, 1.0)? {
                        0 => BiasSource::Unbiased,
                        _ => BiasSource::Demand,
                    };
                }
                "sigma" => {
                    if !(v.is_finite() && v > 0.0) {
                        return Err(bad(option, format!("must be above 0, got {v}")));
                    }
                    m.sigma = v;
                }
                "max_detour" => {
                    if !(v.is_finite() && v >= 1.0) {
                        return Err(bad(option, format!("must be at least 1, got {v}")));
                    }
                    m.max_detour = v;
                }
                _ => {
                    if !(v > 0.0 && v <= 1.0) {
                        return Err(bad(option, format!("must be above 0 and at most 1, got {v}")));
                    }
                    m.max_overlap = v;
                }
            }
        }
        Ok(m)
    }

    /// The same method with `bias` as its bias: one number from 0 to 1 per link, by link id
    /// (0 leaves a link's cost alone, 1 lets it change by up to `sigma` times itself).
    /// Replaces whatever the demand would give: the method no longer reads it.
    ///
    /// # Errors
    ///
    /// [`RouteError::BadOption`] if a value is not a number from 0 to 1 (X-40: a negative one
    /// could make a link's cost negative, which a shortest-path search does not survive).
    pub fn with_bias(mut self, bias: Vec<f32>) -> Result<Self, RouteError> {
        if let Some((link, value)) =
            bias.iter().enumerate().find(|(_, v)| !(0.0..=1.0).contains(*v))
        {
            return Err(RouteError::BadOption {
                method: "montecarlo".into(),
                option: "bias".into(),
                reason: format!("link {link} has {value}; each value must be from 0 to 1"),
            });
        }
        self.bias = Some(bias.into());
        self.source = BiasSource::Given;
        Ok(self)
    }

    /// How the descriptor names the bias: `none`, `demand`, or for a given one `given:` and a
    /// hash of its values, so two given biases never share a route set's identity.
    fn bias_name(&self) -> String {
        match (self.source, &self.bias) {
            (BiasSource::Given, Some(bias)) => {
                let mut h = Fnv1a::new();
                h.write_u64(bias.len() as u64);
                bias.iter().for_each(|v| h.write_u32(v.to_bits()));
                format!("given:{:016x}", h.finish())
            }
            (source, _) => source.name().to_owned(),
        }
    }

    /// The bias in use, by link id, if it has one: given, or found from the demand.
    #[must_use]
    pub fn bias(&self) -> Option<&[f32]> {
        self.bias.as_deref()
    }

    /// The seed of one draw: a function of the pair, the draw and the method's `seed`.
    fn draw_seed(&self, origin: NodeId, destination: NodeId, draw: usize) -> u64 {
        (u64::from(origin.raw()) << 32 | u64::from(destination.raw()))
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ (draw as u64 + 1).wrapping_mul(0xD6E8_FEB8_6659_FD93)
            ^ self.seed.wrapping_mul(0xA24B_AED4_963E_E407)
    }
}

impl RouteSetGenerator for MonteCarlo {
    fn name(&self) -> &str {
        "montecarlo"
    }

    fn descriptor(&self) -> String {
        format!(
            "montecarlo;bias={};draws={};max_detour={};max_overlap={};max_paths={};seed={};sigma={}",
            self.bias_name(),
            self.draws,
            self.max_detour,
            self.max_overlap,
            self.max_paths,
            self.seed,
            self.sigma
        )
    }

    fn generate(&self, search: &mut Search<'_>, origin: NodeId, destination: NodeId) -> Vec<Route> {
        search.clear_route_state();
        let Some(first) = search.shortest(origin, destination) else {
            return Vec::new();
        };
        let best = first.cost;
        search.mark(&first.links, 0);
        let mut kept = vec![first];
        for draw in 0..self.draws {
            if kept.len() >= self.max_paths {
                break;
            }
            search.set_noise(
                self.draw_seed(origin, destination, draw),
                self.sigma,
                self.bias.clone(),
            );
            let Some(mut candidate) =
                search.shortest_within(origin, destination, self.max_detour * best)
            else {
                continue;
            };
            if candidate.cost > self.max_detour * best
                || kept.iter().any(|k| k.links == candidate.links)
            {
                continue;
            }
            let overlap = search.max_overlap(&candidate.links);
            if overlap > self.max_overlap {
                continue;
            }
            candidate.overlap = overlap;
            search.mark(&candidate.links, kept.len());
            kept.push(candidate);
        }
        search.clear_route_state();
        sort_best_first(&mut kept);
        kept
    }

    fn reads_demand(&self) -> bool {
        self.source == BiasSource::Demand && self.bias.is_none()
    }

    fn with_demand(&self, demand: &Demand<'_>) -> Option<Box<dyn RouteSetGenerator>> {
        self.reads_demand().then(|| {
            let mut prepared = self.clone();
            prepared.bias = Some(congestion_propensity(demand).into());
            Box::new(prepared) as Box<dyn RouteSetGenerator>
        })
    }
}

// --- sampled ----------------------------------------------------------------------

/// Extras prepared for one context: by user class, then by draw; `None` is travel time alone.
type PreparedExtras = Vec<Vec<Option<Extras>>>;

/// Sampled generation by **generalised cost**: routes drawn with attribute noise and with
/// preference draws per user class (S298).
///
/// First the **anchor**, the travel-time shortest route. Then, for each user class and each of
/// its draws (draw 0 the class's unperturbed generalised cost, draws 1 to `draws` with its
/// coefficients perturbed), the cheapest route under that generalised cost, with every link's
/// cost also multiplied by `1 + sigma · u` from draw 1 on, `u` a uniform draw in `[0, 1)` that is
/// a pure function of the origin, the class, the draw, the link and `seed`. The generalised
/// cost is each link's travel time plus **extras**, extra seconds per link that stand for what
/// the class values beyond time (length, money, bike infrastructure); the run composes them
/// from the choice model and hands them over with [`RouteSetGenerator::with_extras`], asked for
/// by [`RouteSetGenerator::extras_wanted`]. Without extras every draw searches by travel time,
/// and only the noise varies the routes.
///
/// A route is kept if the set holds fewer than `max_paths`, it is new, its cost **in the
/// generalised cost it was searched by** (unperturbed: its travel time plus the class's extras
/// for the draw) is at most `max_detour` times the anchor's in that cost, and it shares at most
/// `max_overlap` of that cost with each route kept before; without extras that cost is the
/// travel time. A set's routes are ordered by the first class's unperturbed generalised cost.
/// A route's `cost` is its travel time, unperturbed, like every method's.
///
/// **Cost:** one search tree per origin for the anchor, and one per class and draw for all the
/// origin's destinations together, each bounded at `max_detour` times the largest generalised
/// cost of the destinations' anchors; the extras are 8 bytes per link per class and draw,
/// shared by every thread. **Guarantee:** none of optimality or completeness; the first route
/// is a travel-time shortest one.
#[derive(Clone, Debug)]
pub struct Sampled {
    /// Perturbed preference draws per user class, besides draw 0 (0 to 64).
    pub draws: u32,
    /// The attribute noise: from draw 1 on, a link's cost is multiplied by `1 + sigma · u`, `u`
    /// uniform in `[0, 1)` (at least 0).
    pub sigma: f64,
    /// The spread of the preference draws, used by the run that composes the extras: each
    /// coefficient times `1 + tau · v`, `v` in `[−1, 1)` (at least 0).
    pub tau: f64,
    /// A kept route's generalised cost is at most this multiple of the anchor's (at least 1).
    pub max_detour: f64,
    /// A kept route shares at most this share of its generalised cost with any route kept
    /// before it (above 0, at most 1).
    pub max_overlap: f64,
    /// How many routes to keep at most (1 to 32).
    pub max_paths: usize,
    /// Which draws: changes the noise of every draw and the run's preference draws (0 by
    /// default).
    pub seed: u64,
    /// The extras the run gave ([`RouteSetGenerator::with_extras`]), by class and draw.
    extras: Option<ClassExtras>,
    /// The same, prepared for a context ([`RouteSetGenerator::with_context`]).
    prepared: Option<PreparedExtras>,
}

impl Default for Sampled {
    fn default() -> Self {
        Self {
            draws: 4,
            sigma: 0.5,
            tau: 0.3,
            max_detour: 1.5,
            max_overlap: 0.9,
            max_paths: 10,
            seed: 0,
            extras: None,
            prepared: None,
        }
    }
}

impl Sampled {
    const OPTIONS: [&'static str; 7] =
        ["draws", "max_detour", "max_overlap", "max_paths", "seed", "sigma", "tau"];

    /// The most perturbed draws per class.
    const MAX_DRAWS: f64 = 64.0;

    /// Make the method from its options, defaults for those not given: 4 draws, `sigma` 0.5,
    /// `tau` 0.3, `max_detour` 1.5, `max_overlap` 0.9, `max_paths` 10, `seed` 0.
    ///
    /// # Errors
    ///
    /// [`RouteError::UnknownOption`] for a name it does not have,
    /// [`RouteError::BadOption`] for a value out of range.
    pub fn from_options(options: &Options) -> Result<Self, RouteError> {
        check_known("sampled", options, &Self::OPTIONS)?;
        let mut m = Self::default();
        let bad = |option: &str, reason: String| RouteError::BadOption {
            method: "sampled".to_string(),
            option: option.to_string(),
            reason,
        };
        let whole = |option: &str, v: f64, lo: f64, hi: f64| -> Result<u64, RouteError> {
            if v.is_finite() && v.fract() == 0.0 && v >= lo && v <= hi {
                #[allow(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "checked to be a non-negative whole number within range"
                )]
                Ok(v as u64)
            } else {
                Err(bad(option, format!("must be a whole number from {lo} to {hi}, got {v}")))
            }
        };
        let at_least = |option: &str, v: f64, lo: f64| -> Result<f64, RouteError> {
            if v.is_finite() && v >= lo {
                Ok(v)
            } else {
                Err(bad(option, format!("must be at least {lo}, got {v}")))
            }
        };
        for (option, &v) in options {
            #[allow(clippy::cast_possible_truncation, reason = "bounded by the checks in `whole`")]
            match option.as_str() {
                "draws" => m.draws = whole(option, v, 0.0, Self::MAX_DRAWS)? as u32,
                "max_paths" => m.max_paths = whole(option, v, 1.0, MAX_PATHS_BOUND)? as usize,
                "seed" => m.seed = whole(option, v, 0.0, 9_007_199_254_740_992.0)?,
                "sigma" => m.sigma = at_least(option, v, 0.0)?,
                "tau" => m.tau = at_least(option, v, 0.0)?,
                "max_detour" => m.max_detour = at_least(option, v, 1.0)?,
                _ => {
                    if !(v > 0.0 && v <= 1.0) {
                        return Err(bad(option, format!("must be above 0 and at most 1, got {v}")));
                    }
                    m.max_overlap = v;
                }
            }
        }
        Ok(m)
    }

    /// How the descriptor names the extras: `none`, or `given:` and a hash of every value of
    /// every class and draw, so two sets of extras never share a route set's identity.
    fn extras_name(&self) -> String {
        let Some(rows) = &self.extras else {
            return "none".to_owned();
        };
        let mut h = Fnv1a::new();
        h.write_u64(rows.len() as u64);
        for row in rows {
            h.write_u64(row.len() as u64);
            for draw in row {
                match draw {
                    None => h.write_bool(false),
                    Some(seconds) => {
                        h.write_bool(true);
                        h.write_u64(seconds.len() as u64);
                        seconds.iter().for_each(|&s| h.write_u64(s.to_bits()));
                    }
                }
            }
        }
        format!("given:{:016x}", h.finish())
    }

    /// The extras prepared for `ctx`: the given ones, or, without them, one class of
    /// `draws + 1` draws by travel time alone.
    fn prepare(&self, ctx: &SearchContext<'_>) -> PreparedExtras {
        match &self.extras {
            Some(rows) => {
                // Classes whose extras are the same in every draw (every class without prices,
                // say) search alike: one row serves them all.
                let mut distinct: Vec<&Vec<Option<Arc<[f64]>>>> = Vec::new();
                for row in rows {
                    let seen = distinct.iter().any(|d| {
                        d.len() == row.len()
                            && d.iter().zip(row.iter()).all(|(a, b)| match (a, b) {
                                (None, None) => true,
                                (Some(a), Some(b)) => a[..] == b[..],
                                _ => false,
                            })
                    });
                    if !seen {
                        distinct.push(row);
                    }
                }
                distinct
                    .into_iter()
                    .map(|row| {
                        row.iter().map(|e| e.as_ref().map(|v| ctx.extras(v.to_vec()))).collect()
                    })
                    .collect()
            }
            None => vec![vec![None; self.draws as usize + 1]],
        }
    }

    /// The noise seed of one class and draw from one origin: a pure function of the three and
    /// the method's `seed`.
    fn seed_of(&self, origin: NodeId, class: usize, draw: usize) -> u64 {
        (u64::from(origin.raw()) << 32 | (class as u64) << 8 | draw as u64)
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ self.seed.wrapping_mul(0xA24B_AED4_963E_E407)
    }
}

/// The largest share of `candidate`'s cost that lies on links of any one of `kept` (each given
/// as its links, sorted), a link costing `cost(link)`; 0 for a route of no cost.
fn overlap_with(cost: impl Fn(LinkId) -> f64, candidate: &[LinkId], kept: &[Vec<LinkId>]) -> f64 {
    let total: f64 = candidate.iter().map(|&l| cost(l)).sum();
    if total <= 0.0 {
        return 0.0;
    }
    kept.iter()
        .map(|sorted| {
            candidate
                .iter()
                .filter(|l| sorted.binary_search(l).is_ok())
                .map(|&l| cost(l))
                .sum::<f64>()
                / total
        })
        .fold(0.0, f64::max)
}

/// A link's unperturbed cost in `ctx` under `extras`: its time, plus its extra if there are any.
fn generalised_link(ctx: &SearchContext<'_>, extras: Option<&Extras>, link: LinkId) -> f64 {
    match extras {
        None => ctx.link_cost(link),
        Some(e) => ctx.link_cost(link) + e.seconds()[link.index()],
    }
}

/// The unperturbed cost of `links` in `ctx` under `extras`: their time if there are none, to
/// the bit what [`SearchContext::route_cost`] gives.
fn generalised_route(ctx: &SearchContext<'_>, extras: Option<&Extras>, links: &[LinkId]) -> f64 {
    match extras {
        None => ctx.route_cost(links),
        Some(_) => links.iter().map(|&l| generalised_link(ctx, extras, l)).sum(),
    }
}

impl RouteSetGenerator for Sampled {
    fn name(&self) -> &str {
        "sampled"
    }

    fn descriptor(&self) -> String {
        format!(
            "sampled;draws={};extras={};max_detour={};max_overlap={};max_paths={};seed={};sigma={};tau={}",
            self.draws,
            self.extras_name(),
            self.max_detour,
            self.max_overlap,
            self.max_paths,
            self.seed,
            self.sigma,
            self.tau
        )
    }

    fn generate(&self, search: &mut Search<'_>, origin: NodeId, destination: NodeId) -> Vec<Route> {
        self.generate_from(search, origin, &[destination]).pop().unwrap_or_default()
    }

    fn searches_trees(&self) -> bool {
        true
    }

    /// One search tree for the anchors, then one per user class and draw, each for every
    /// destination whose set is not yet full.
    fn generate_from(
        &self,
        search: &mut Search<'_>,
        origin: NodeId,
        destinations: &[NodeId],
    ) -> Vec<Vec<Route>> {
        let ctx = search.context();
        let owned;
        let prepared: &PreparedExtras = if let Some(p) = &self.prepared {
            p
        } else {
            owned = self.prepare(ctx);
            &owned
        };

        // The anchors: the travel-time shortest routes, no extras, no noise.
        search.clear_route_state();
        let anchors = search.shortest_tree(origin, destinations, f64::INFINITY);
        let mut sets: Vec<Vec<Route>> =
            anchors.iter().map(|a| a.iter().cloned().collect()).collect();
        // Each kept route's links, sorted, for the overlap.
        let mut sorted: Vec<Vec<Vec<LinkId>>> = sets
            .iter()
            .map(|set| {
                set.iter()
                    .map(|r| {
                        let mut links = r.links.clone();
                        links.sort_unstable();
                        links
                    })
                    .collect()
            })
            .collect();
        let anchored: Vec<usize> =
            (0..destinations.len()).filter(|&i| anchors[i].is_some()).collect();

        'classes: for (class, row) in prepared.iter().enumerate() {
            for (draw, extras) in row.iter().enumerate().take(self.draws as usize + 1) {
                if draw == 0 && extras.is_none() {
                    continue; // time alone, unperturbed: the anchor again
                }
                let active: Vec<usize> =
                    anchored.iter().copied().filter(|&i| sets[i].len() < self.max_paths).collect();
                if active.is_empty() {
                    break 'classes;
                }
                search.set_extras(extras.clone());
                if draw == 0 {
                    search.clear_noise();
                } else {
                    search.set_noise(self.seed_of(origin, class, draw), self.sigma, None);
                }
                // The bound is on the generalised cost; a route within the detour in time may
                // carry up to `1 + max_ratio` times its time in generalised cost, so the bound
                // allows for that: the longest anchor's time, times the detour, times that.
                let ratio = extras.as_ref().map_or(0.0, Extras::max_ratio);
                let bound = self.max_detour
                    * (1.0 + ratio)
                    * anchored
                        .iter()
                        .filter_map(|&i| anchors[i].as_ref())
                        .map(|a| a.cost)
                        .fold(0.0, f64::max);
                let targets: Vec<NodeId> = active.iter().map(|&i| destinations[i]).collect();
                let found = search.shortest_tree(origin, &targets, bound);
                // A candidate is judged in the cost it was searched by, unperturbed: its travel
                // time plus this class's extras for the draw (its time alone without extras),
                // against the anchor's in the same cost.
                let judged = extras.as_ref();
                for (&i, candidate) in active.iter().zip(found) {
                    let Some(mut candidate) = candidate else { continue };
                    let Some(anchor) = &anchors[i] else { continue };
                    let (cost, anchor_cost) = match judged {
                        None => (candidate.cost, anchor.cost),
                        Some(_) => (
                            generalised_route(ctx, judged, &candidate.links),
                            generalised_route(ctx, judged, &anchor.links),
                        ),
                    };
                    if cost > self.max_detour * anchor_cost
                        || sets[i].iter().any(|k| k.links == candidate.links)
                    {
                        continue;
                    }
                    let overlap = overlap_with(
                        |l| generalised_link(ctx, judged, l),
                        &candidate.links,
                        &sorted[i],
                    );
                    if overlap > self.max_overlap {
                        continue;
                    }
                    candidate.overlap = overlap;
                    let mut links = candidate.links.clone();
                    links.sort_unstable();
                    sorted[i].push(links);
                    sets[i].push(candidate);
                }
            }
        }
        search.clear_route_state();
        // Best first by the first class's generalised cost, unperturbed (its time where it has
        // no extras): the order a set's routes keep.
        let first = prepared.first().and_then(|row| row.first()).and_then(Option::as_ref);
        for set in &mut sets {
            match first {
                None => sort_best_first(set),
                Some(_) => sort_best_first_by(set, |r| generalised_route(ctx, first, &r.links)),
            }
        }
        sets
    }

    fn extras_wanted(&self) -> Option<ExtrasRequest> {
        Some(ExtrasRequest { draws: self.draws, tau: self.tau, seed: self.seed })
    }

    fn with_extras(&self, extras: ClassExtras) -> Option<Box<dyn RouteSetGenerator>> {
        let mut given = self.clone();
        given.extras = Some(extras);
        given.prepared = None;
        Some(Box::new(given))
    }

    fn with_context(&self, ctx: &SearchContext<'_>) -> Option<Box<dyn RouteSetGenerator>> {
        let mut prepared = self.clone();
        prepared.prepared = Some(self.prepare(ctx));
        Some(Box::new(prepared))
    }
}

fn check_known(method: &str, options: &Options, known: &[&'static str]) -> Result<(), RouteError> {
    for name in options.keys() {
        if !known.contains(&name.as_str()) {
            return Err(RouteError::UnknownOption {
                method: method.to_string(),
                option: name.clone(),
                known: known.to_vec(),
            });
        }
    }
    Ok(())
}

// --- the registry -----------------------------------------------------------------

/// Makes a method from its options.
pub type Factory = fn(&Options) -> Result<Box<dyn RouteSetGenerator>, RouteError>;

/// Methods by name. [`Registry::builtin`] has the built-in ones; a researcher's
/// method is added with [`Registry::register`] and then selected by name like
/// any other.
#[derive(Clone, Debug)]
pub struct Registry {
    methods: Vec<(String, Factory)>,
}

impl Registry {
    /// The built-in methods: `penalty`, `shortest`, `montecarlo` and `sampled`.
    #[must_use]
    pub fn builtin() -> Self {
        let mut r = Self { methods: Vec::new() };
        r.register("penalty", |o| Ok(Box::new(Penalty::from_options(o)?)));
        r.register("shortest", |o| Ok(Box::new(Shortest::from_options(o)?)));
        r.register("montecarlo", |o| Ok(Box::new(MonteCarlo::from_options(o)?)));
        r.register("sampled", |o| Ok(Box::new(Sampled::from_options(o)?)));
        r
    }

    /// Add a method under `name`, or replace one of that name.
    pub fn register(&mut self, name: &str, factory: Factory) {
        match self.methods.iter_mut().find(|(n, _)| n == name) {
            Some(entry) => entry.1 = factory,
            None => self.methods.push((name.to_string(), factory)),
        }
    }

    /// The names that can be selected, in registration order.
    #[must_use]
    pub fn names(&self) -> Vec<&str> {
        self.methods.iter().map(|(n, _)| n.as_str()).collect()
    }

    /// Make the method called `name` from `options`.
    ///
    /// # Errors
    ///
    /// [`RouteError::UnknownMethod`] if there is none, or whatever the
    /// method's own validation says about its options.
    pub fn create(
        &self,
        name: &str,
        options: &Options,
    ) -> Result<Box<dyn RouteSetGenerator>, RouteError> {
        match self.methods.iter().find(|(n, _)| n == name) {
            Some((_, factory)) => factory(options),
            None => Err(RouteError::UnknownMethod {
                name: name.to_string(),
                known: self.names().into_iter().map(str::to_string).collect(),
            }),
        }
    }
}

/// Make a built-in method by name; see [`Registry::builtin`].
///
/// # Errors
///
/// As [`Registry::create`].
pub fn generator(name: &str, options: &Options) -> Result<Box<dyn RouteSetGenerator>, RouteError> {
    Registry::builtin().create(name, options)
}

/// The default method with its default options.
#[must_use]
pub fn default_generator() -> Box<dyn RouteSetGenerator> {
    Box::new(Penalty::default())
}
