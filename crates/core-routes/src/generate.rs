//! Route-set generation behind one interface (design §13.6).
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
//! | `montecarlo` | The shortest route, then the shortest under **random link costs biased towards the links likely to congest** (S176): distinct routes that avoid what the demand will jam |
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
use openmobisim_core_types::ids::{EntityId, LinkId, NodeId};

use crate::search::{MAX_ROUTES_PER_SET, Route, Search};
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

    /// Whether the method reads the demand it generates for (see [`Self::with_demand`]).
    /// A run assembles the demand only if it does, so a method that does not costs nothing.
    fn reads_demand(&self) -> bool {
        false
    }

    /// The same method, made aware of the demand it is about to generate for (S176), or
    /// `None` if it does not read it. Called once, before [`RouteSets::generate`], by whoever
    /// generates for a demand (a run; `route_sets_build`). The Monte Carlo method uses it to
    /// find the links likely to congest ([`congestion_propensity`]).
    fn with_demand(&self, _demand: &Demand<'_>) -> Option<Box<dyn RouteSetGenerator>> {
        None
    }
}

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
/// its pair's free-flow shortest route (S176; S174's "static congestion propensity").
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

/// Monte Carlo generation, biased towards the links likely to congest (S176).
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
/// gap was 0.96, against the penalty method's 0.30 and this method's 0.16 (S174). By default
/// the bias is [`congestion_propensity`] of the demand the run generates for; a caller with a
/// better idea gives its own ([`Self::with_bias`]).
///
/// **Cost:** at most `draws` bounded searches per pair, each exploring about what the
/// penalty method's do; about 4 s more than the penalty method for 9 600 pairs at 16 draws
/// (S174), and one shortest route per distinct pair for the bias.
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
    /// the demand's congestion propensity; 0 for none). These are the settings S174 measured.
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
    #[must_use]
    pub fn with_bias(mut self, bias: Vec<f32>) -> Self {
        self.bias = Some(bias.into());
        self.source = BiasSource::Given;
        self
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
            self.source.name(),
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
    /// The built-in methods: `penalty`, `shortest` and `montecarlo`.
    #[must_use]
    pub fn builtin() -> Self {
        let mut r = Self { methods: Vec::new() };
        r.register("penalty", |o| Ok(Box::new(Penalty::from_options(o)?)));
        r.register("shortest", |o| Ok(Box::new(Shortest::from_options(o)?)));
        r.register("montecarlo", |o| Ok(Box::new(MonteCarlo::from_options(o)?)));
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
