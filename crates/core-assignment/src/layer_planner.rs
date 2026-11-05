//! The static planners: bike and walking legs on their own layers.
//!
//! A **layer** is the network the run builds for a mode ([`StaticNetwork`]): the bike layer
//! (cycleways, painted lanes, and the roads a bike may use, each link with a bike's speed on it)
//! and the walk layer (footpaths and streets, both ways, at the walking speed). A bike or walking
//! leg runs on its layer, between two nodes of it (a trip's ends snapped to the layer's nearest
//! node), along the links the layer numbers.
//!
//! **Static times.** A link's time on a layer is its length over its speed,
//! [`StaticNetwork::link_seconds`]: nobody else on the layer changes it, and nothing of the road
//! network's traffic reaches it (bikes are not loaded on road links). So a bike or walking leg
//! has the same route, time and attributes in every assignment iteration: the chain search plans
//! it **once, at iteration 0**, and keeps it ([`crate::ChainSearch`]); re-pricing it gives the
//! same numbers.
//!
//! **The route.** The route of least search cost from the leg's origin node to its destination
//! node: `core-routes`' label-setting search over links ([`Search::shortest_tree`] with one
//! destination: Dijkstra, the turn table of the layer's graph, the tie rule of `core-routes`,
//! which reads lengths, link counts and node coordinates, never ids). The search cost per link,
//! in seconds-equivalent:
//!
//! - **walking:** the link's time `t_a`, plus `β_km · ℓ_a / 1000 · 60 / β_walk` where the class
//!   weighs kilometres (`β_km` [`Coefficients::per_kilometre`], `β_walk`
//!   [`Coefficients::walk_per_minute`], `ℓ_a` the length in metres);
//! - **bike**, by the run's bike cost ([`BikeCost`]): `Time`, the link's time `t_a` (the
//!   fastest route); `Dedicated`, `t_a` times the layer's multiplier on a link in mixed traffic
//!   (`bike_mixed_cost_factor`, 1.2 by default) or on a painted lane (`bike_lane_cost_factor`),
//!   as [`StaticNetwork::link_costs`] gives it; `Utility` (the default of the other engine), the
//!   class's own cost of the ride: `t_a + (β_km · ℓ_a + β_mixed · m_a + β_lane · p_a +
//!   β_separated · s_a) / 1000 · 60 / β_ivt`, with `m_a`, `p_a` and `s_a` the link's metres in
//!   mixed traffic ([`StaticNetwork::is_mixed_traffic`]), on a painted lane and on a track or
//!   path of its own (each link is one of the three, a ferry none), `β_mixed`
//!   [`Coefficients::bike_mixed_per_kilometre`], `β_lane` [`Coefficients::bike_lane_per_kilometre`],
//!   `β_separated` [`Coefficients::bike_separated_per_kilometre`] and `β_ivt`
//!   [`Coefficients::in_vehicle_per_minute`] (a facility's term is left out where its coefficient
//!   is 0).
//!   So under `Utility` the route minimises the leg's cost in utility units (divided by the
//!   positive `β_ivt / 60`, and less the fixed times, which every route pays alike).
//!
//! The classes whose search costs are equal, to the bit, share one search context; each other
//! class gets its own (the other engine instead searches every class by the coefficients of the
//! class serving the most people: one route per pair).
//!
//! **The leg.** Its links in travel order; its attributes from them: a bike leg's ride,
//! `Σ t_a`, is in-vehicle time, its length `Σ ℓ_a`, its metres in mixed traffic `Σ m_a`, on a
//! painted lane `Σ p_a` and on a track or path of its own `Σ s_a`, and the
//! bike's [`FixedTimes`] (reaching and unlocking the bike before the ride, as walking time;
//! parking and locking it after, as parking time; [`StaticPlanner::bike`]); a walking leg's time
//! `Σ t_a` is walking time, with its length, and no fixed time. A leg whose two ends are one node
//! uses no link and carries no fixed time (as a car leg). The sums are taken link by link in
//! travel order, exactly, with no rounding of the clock.
//!
//! **Cost of a plan.** One search over the links it touches, plus a search scratch of about 44
//! bytes per link of the layer (kept per thread across the trips of one search stage).
//!
//! **Sampled bike route sets, an option** ([`BikeRoutes::Sampled`]; the default is
//! [`BikeRoutes::Shortest`], the one route above). A bike leg then has a **set of routes** per
//! pair of bike-layer nodes, made once, before iteration 0, by `core-routes`' doubly stochastic
//! generator (`Sampled`, Bovy and Fiorenzo-Catalano, 2007) through `RouteSets::generate_in`, as
//! the other engine makes its bike sets: first the anchor, the route of least time; then, for
//! each user class and each of its draws, the route of least **generalised cost** with every
//! link's cost multiplied by `1 + σ · u` from draw 1 on (`u` uniform in `[0, 1)`, a pure function
//! of the origin's coordinates, the class, the draw, the link's coordinates and the options'
//! `seed`: never of an id); a route is kept if it is new, within the detour of the anchor and
//! overlaps the routes kept before by at most the overlap limit, both in that generalised cost.
//! The generalised cost of a link, in seconds:
//!
//! - under [`BikeCost::Utility`], its time plus the class's **extra seconds**,
//!   `(β_km · ℓ_a + β_mixed · m_a + β_lane · p_a + β_separated · s_a) / 1000 · 60 / β_ivt` (the
//!   search cost above less the time), each of the four coefficients of draw `d ≥ 1` multiplied
//!   by `max(0, 1 + τ · v)`, `v` uniform in `[−1, 1)` and a pure function of the options' `seed`,
//!   the class, the draw and the coefficient (a **preference draw**; draw 0 is the class's own
//!   coefficients); a class that weighs none of the four searches by time alone;
//! - under [`BikeCost::Time`], its time; under [`BikeCost::Dedicated`], its dedicated-bike cost:
//!   the same for every class, the draws varying only by the noise.
//!
//! A set's routes are kept in the generator's order (best first by the first class's generalised
//! cost, ties broken by the streets, never by ids), and each becomes a bike chain of every trip
//! of the pair, in that order. The bike layer carries no congestion, so the sets never change.
//! **Cost:** one search tree per origin node for the anchors and one per class and draw, each
//! bounded at the detour; 8 bytes per link per class and draw for the extras; once per run.

use std::sync::Arc;

use openmobisim_core_demand::Mode;
use openmobisim_core_graph::defaults::RoadClass;
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::layers::{
    BikeCost, BikeInfrastructure, StaticLayer, StaticLayerDefaults, StaticNetwork,
};
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_routes::generate::RouteSetGenerator;
use openmobisim_core_routes::search::{Search, SearchContext};
use openmobisim_core_routes::{
    ClassExtras, NodeSnapper, Options, RouteError, RouteKey, RouteSets, Sampled,
};
use openmobisim_core_types::ids::{EntityId, LinkId, NodeId, UserClassId};

use crate::attributes::{Attributes, Coefficients};
use crate::chain::{FixedTimes, Leg, Place};
use crate::error::AssignError;
use crate::planner::{CostSource, LegPlanner};
use crate::state::State;

/// One static layer as the assignment reads it: its graph and speeds, and the turns a search
/// may take on it (built by the caller, `TurnTable::build` over the layer's graph).
#[derive(Clone, Copy, Debug)]
pub struct Layer<'a> {
    /// The layer: its graph, every link's speed and bike infrastructure.
    pub network: &'a StaticNetwork,
    /// The turns of the layer's graph.
    pub turns: &'a TurnTable,
}

/// The run's static layers: the bike layer and the walk layer, each only if the run has it
/// (a run with no bike or walking trip needs neither).
#[derive(Clone, Copy, Debug, Default)]
pub struct Layers<'a> {
    /// The bike layer.
    pub bike: Option<Layer<'a>>,
    /// The walk layer.
    pub walk: Option<Layer<'a>>,
}

impl Layers<'_> {
    /// No static layer: a run of car trips.
    pub const NONE: Layers<'static> = Layers { bike: None, walk: None };
}

/// How a bike leg's routes are made on the bike layer (see the [module docs](self)).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum BikeRoutes {
    /// One route per pair: the route of least search cost (the default).
    #[default]
    Shortest,
    /// A set of routes per pair, made by the doubly stochastic generator with these options.
    Sampled(SampledRoutes),
}

impl BikeRoutes {
    /// The name the scenario selects it by: `"shortest"` or `"doubly_stochastic"`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Shortest => "shortest",
            Self::Sampled(_) => "doubly_stochastic",
        }
    }
}

/// The options of the doubly stochastic generator of bike route sets: `core-routes`' `Sampled`
/// options, by the same names and with the same defaults (4 draws, `sigma` 0.5, `tau` 0.3,
/// `max_detour` 1.5, `max_overlap` 0.9, `max_paths` 10, `seed` 0; uncalibrated, the generator's
/// own).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SampledRoutes {
    /// Perturbed draws per user class, besides draw 0 (0 to 64).
    pub draws: u32,
    /// The noise on a link's cost from draw 1 on: times `1 + sigma · u`, `u` uniform in `[0, 1)`.
    pub sigma: f64,
    /// The spread of the preference draws: each coefficient but the time's times
    /// `max(0, 1 + tau · v)`, `v` uniform in `[−1, 1)`.
    pub tau: f64,
    /// A kept route's generalised cost is at most this multiple of the anchor's (at least 1).
    pub max_detour: f64,
    /// A kept route shares at most this share of its generalised cost with any route kept
    /// before it (above 0, at most 1).
    pub max_overlap: f64,
    /// The most routes a set keeps (1 to 32).
    pub max_paths: usize,
    /// Which draws: changes the noise and the preference draws.
    pub seed: u64,
}

impl Default for SampledRoutes {
    fn default() -> Self {
        Self::of(&Sampled::default())
    }
}

impl SampledRoutes {
    /// The options of `generator`.
    #[must_use]
    pub fn of(generator: &Sampled) -> Self {
        Self {
            draws: generator.draws,
            sigma: generator.sigma,
            tau: generator.tau,
            max_detour: generator.max_detour,
            max_overlap: generator.max_overlap,
            max_paths: generator.max_paths,
            seed: generator.seed,
        }
    }

    /// The options from their names and values, the generator's defaults for those not given,
    /// checked by the generator.
    ///
    /// # Errors
    ///
    /// The generator's [`RouteError`] for a name it does not have or a value out of range.
    pub fn from_options(options: &Options) -> Result<Self, RouteError> {
        Sampled::from_options(options).map(|g| Self::of(&g))
    }

    /// Every option by name, in the order of the names: what a run records.
    #[must_use]
    pub fn values(&self) -> [(&'static str, f64); 7] {
        #[allow(clippy::cast_precision_loss, reason = "small whole numbers, exact in a double")]
        [
            ("draws", f64::from(self.draws)),
            ("max_detour", self.max_detour),
            ("max_overlap", self.max_overlap),
            ("max_paths", self.max_paths as f64),
            ("seed", self.seed as f64),
            ("sigma", self.sigma),
            ("tau", self.tau),
        ]
    }

    /// The generator with these options.
    fn generator(&self) -> Sampled {
        let mut g = Sampled::default();
        g.draws = self.draws;
        g.sigma = self.sigma;
        g.tau = self.tau;
        g.max_detour = self.max_detour;
        g.max_overlap = self.max_overlap;
        g.max_paths = self.max_paths;
        g.seed = self.seed;
        g
    }
}

/// A uniform number in `[−1, 1)`, a pure function of `(seed, class, draw, coefficient)`: the
/// preference draw of one coefficient (SplitMix64's finaliser over the four).
fn preference_draw(seed: u64, class: usize, draw: u32, coefficient: u32) -> f64 {
    let mut x = seed
        ^ ((class as u64) << 40 | u64::from(draw) << 20 | u64::from(coefficient))
            .wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    #[allow(clippy::cast_precision_loss, reason = "the top 53 bits of a u64 fit a double exactly")]
    let u = (x >> 11) as f64 / (1_u64 << 53) as f64;
    2.0 * u - 1.0
}

/// The per-link attributes a bike ride's cost reads beyond its time: kilometres, and metres in
/// mixed traffic, on a painted lane and on a track or path of its own.
struct RideTerms<'v> {
    km: &'v [f64],
    mixed: &'v [f64],
    lane: &'v [f64],
    separated: &'v [f64],
}

impl RideTerms<'_> {
    /// What link `l` costs class `k` beyond its time, in utility units:
    /// `β_km · ℓ + β_mixed · m + β_lane · p + β_separated · s` (kilometres), a facility's term
    /// only where it is weighed, so a class that weighs neither facility costs, to the bit, what
    /// it cost without them.
    fn beyond_time(&self, k: &Coefficients, l: usize) -> f64 {
        let mut sum =
            k.per_kilometre * self.km[l] + k.bike_mixed_per_kilometre * self.mixed[l] / 1000.0;
        if k.bike_lane_per_kilometre != 0.0 {
            sum += k.bike_lane_per_kilometre * self.lane[l] / 1000.0;
        }
        if k.bike_separated_per_kilometre != 0.0 {
            sum += k.bike_separated_per_kilometre * self.separated[l] / 1000.0;
        }
        sum
    }

    /// Class `k`'s extra seconds per link (its cost beyond time over its cost of a second
    /// riding, floored so that time plus extra is never below 0); `None` when every one is 0.
    fn extras(&self, k: &Coefficients, seconds: &[f64]) -> Option<Arc<[f64]>> {
        let per_second = k.in_vehicle_per_minute / 60.0;
        let mut any = false;
        let extras: Vec<f64> = seconds
            .iter()
            .enumerate()
            .map(|(l, &t)| {
                let e = self.beyond_time(k, l) / per_second;
                any |= e != 0.0;
                if t.is_finite() && t + e < 0.0 { -t } else { e }
            })
            .collect();
        any.then(|| Arc::from(extras))
    }
}

/// Class `k`'s coefficients in preference draw `draw` of class `class`: each of the four a
/// ride weighs beyond its time multiplied by `max(0, 1 + tau · v)`; draw 0 (or `tau` 0) is `k`.
fn drawn(k: &Coefficients, seed: u64, class: usize, draw: u32, tau: f64) -> Coefficients {
    if draw == 0 || tau == 0.0 {
        return *k;
    }
    let factor = |c: u32| (1.0 + tau * preference_draw(seed, class, draw, c)).max(0.0);
    Coefficients {
        per_kilometre: k.per_kilometre * factor(0),
        bike_mixed_per_kilometre: k.bike_mixed_per_kilometre * factor(1),
        bike_lane_per_kilometre: k.bike_lane_per_kilometre * factor(2),
        bike_separated_per_kilometre: k.bike_separated_per_kilometre * factor(3),
        ..*k
    }
}

/// What makes a bike layer's sampled route sets: the generator, with its extras, and the search
/// context it searches (the layer's times, or its dedicated-bike costs).
struct SetMaker<'a> {
    generator: Box<dyn RouteSetGenerator>,
    context: SearchContext<'a>,
}

impl std::fmt::Debug for SetMaker<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SetMaker").field("generator", &self.generator.descriptor()).finish()
    }
}

/// A planner of bike or walking legs on their layer. See the [module docs](self).
#[derive(Debug)]
pub struct StaticPlanner<'a> {
    mode: Mode,
    network: &'a RoadNetwork,
    /// Per link, its time in seconds at the layer's speed.
    seconds: Vec<f64>,
    /// Per link, its metres in mixed traffic, on a painted lane and on a track or path of its
    /// own (bike layer; 0 on the walk layer).
    mixed_metres: Vec<f64>,
    lane_metres: Vec<f64>,
    separated_metres: Vec<f64>,
    /// The distinct search contexts, and per user class the one it searches with.
    contexts: Vec<SearchContext<'a>>,
    of_class: Vec<usize>,
    fixed: FixedTimes,
    snapper: NodeSnapper,
    /// The maker of the sampled route sets ([`BikeRoutes::Sampled`]); `None`: one route per pair.
    sets: Option<SetMaker<'a>>,
}

/// Why a class's search costs cannot be used, as an error naming the class.
fn class_error(class: usize, reason: String) -> AssignError {
    AssignError::Coefficients {
        class: u32::try_from(class).expect("fewer than 2^32 classes"),
        reason,
    }
}

impl<'a> StaticPlanner<'a> {
    /// The bike planner on `layer` (the bike layer), its routes by `cost` (under
    /// [`BikeCost::Dedicated`], with the multipliers of `defaults`, the road network's), for user
    /// classes weighing attributes by `classes`, every leg that uses a link with `fixed` times
    /// around the ride; one route per pair, or a sampled set per pair as `routes` says (see the
    /// [module docs](self)).
    ///
    /// # Errors
    ///
    /// [`AssignError::Input`] if `layer` is not a bike layer or a fixed time is not a finite
    /// number of at least 0; [`AssignError::Coefficients`] if, under [`BikeCost::Utility`], a
    /// class's cost of a minute in a vehicle is not a positive finite number or makes a link
    /// cost less than nothing.
    ///
    /// # Panics
    ///
    /// Never: the doubly stochastic generator takes extras.
    pub fn bike(
        layer: Layer<'a>,
        classes: &[Coefficients],
        cost: BikeCost,
        defaults: StaticLayerDefaults,
        fixed: FixedTimes,
        routes: BikeRoutes,
    ) -> Result<Self, AssignError> {
        let network = layer.network;
        if network.layer() != StaticLayer::Bike {
            return Err(AssignError::Input("the bike planner needs the bike layer".to_string()));
        }
        if !fixed.is_valid() {
            return Err(AssignError::Input(format!(
                "the bike's fixed times must be finite numbers of seconds of at least 0, not {} s \
                 before and {} s after the ride",
                fixed.access_seconds, fixed.egress_seconds
            )));
        }
        let graph = network.network();
        let seconds = network.link_seconds();
        let km: Vec<f64> = graph.link_lengths().iter().map(|m| m.get() / 1000.0).collect();
        let mixed_metres: Vec<f64> = (0..graph.link_count())
            .map(|l| {
                let link = LinkId::new(l);
                if network.is_mixed_traffic(link) { graph.link_length(link).get() } else { 0.0 }
            })
            .collect();
        // The metres on each dedicated facility (a ferry is ridden on none).
        let on = |kind: BikeInfrastructure| -> Vec<f64> {
            (0..graph.link_count())
                .map(|l| {
                    let link = LinkId::new(l);
                    let is = network.infrastructure(link) == kind
                        && graph.link_class(link) != RoadClass::Ferry;
                    if is { graph.link_length(link).get() } else { 0.0 }
                })
                .collect()
        };
        let (lane_metres, separated_metres) =
            (on(BikeInfrastructure::Lane), on(BikeInfrastructure::Separated));
        let terms = RideTerms {
            km: &km,
            mixed: &mixed_metres,
            lane: &lane_metres,
            separated: &separated_metres,
        };
        let dedicated = || {
            network.link_costs(
                BikeCost::Dedicated,
                defaults.bike_lane_cost_factor,
                defaults.bike_mixed_cost_factor,
            )
        };
        let costs = |c: usize, k: &Coefficients| -> Result<Vec<f64>, AssignError> {
            match cost {
                BikeCost::Time => Ok(seconds.clone()),
                BikeCost::Dedicated => Ok(dedicated()),
                BikeCost::Utility => {
                    let per_second = k.in_vehicle_per_minute / 60.0;
                    if !(per_second.is_finite() && per_second > 0.0) {
                        return Err(class_error(
                            c,
                            format!(
                                "the cost of a minute riding is {}, not a positive number",
                                k.in_vehicle_per_minute
                            ),
                        ));
                    }
                    let each = |l: usize| seconds[l] + terms.beyond_time(k, l) / per_second;
                    checked(c, (0..seconds.len()).map(each).collect(), &seconds)
                }
            }
        };
        let metres = [mixed_metres.clone(), lane_metres.clone(), separated_metres.clone()];
        let mut planner =
            Self::new(Mode::Bike, layer, classes, seconds.clone(), metres, fixed, costs)?;
        if let BikeRoutes::Sampled(options) = routes {
            let sampled = options.generator();
            // The search costs: the time, with each class's extras on it under the utility;
            // the time or the dedicated-bike cost alone otherwise (no extras: every class alike).
            let (base, generator): (Vec<f64>, Box<dyn RouteSetGenerator>) = match cost {
                BikeCost::Utility => {
                    let extras: ClassExtras = classes
                        .iter()
                        .enumerate()
                        .map(|(c, k)| {
                            (0..=options.draws)
                                .map(|d| {
                                    terms.extras(
                                        &drawn(k, options.seed, c, d, options.tau),
                                        &seconds,
                                    )
                                })
                                .collect()
                        })
                        .collect();
                    let generator = sampled
                        .with_extras(extras)
                        .expect("the doubly stochastic generator takes extras");
                    (seconds.clone(), generator)
                }
                BikeCost::Time => (seconds.clone(), Box::new(sampled)),
                BikeCost::Dedicated => (dedicated(), Box::new(sampled)),
            };
            let context = SearchContext::with_costs(graph, layer.turns, base);
            planner.sets = Some(SetMaker { generator, context });
        }
        Ok(planner)
    }

    /// The walking planner on `layer` (the walk layer), for user classes weighing attributes by
    /// `classes`. A walking leg has no fixed time.
    ///
    /// # Errors
    ///
    /// [`AssignError::Input`] if `layer` is not a walk layer; [`AssignError::Coefficients`] if a
    /// class weighs kilometres and its cost of a minute walking is not a positive finite number,
    /// or a link would cost less than nothing.
    pub fn walk(layer: Layer<'a>, classes: &[Coefficients]) -> Result<Self, AssignError> {
        let network = layer.network;
        if network.layer() != StaticLayer::Walk {
            return Err(AssignError::Input("the walking planner needs the walk layer".to_string()));
        }
        let graph = network.network();
        let seconds = network.link_seconds();
        let km: Vec<f64> = graph.link_lengths().iter().map(|m| m.get() / 1000.0).collect();
        let costs = |c: usize, k: &Coefficients| -> Result<Vec<f64>, AssignError> {
            #[allow(clippy::float_cmp, reason = "a class that does not weigh kilometres at all")]
            if k.per_kilometre == 0.0 {
                return Ok(seconds.clone());
            }
            let per_second = k.walk_per_minute / 60.0;
            if !(per_second.is_finite() && per_second > 0.0) {
                return Err(class_error(
                    c,
                    format!(
                        "the cost of a minute walking is {}, not a positive number",
                        k.walk_per_minute
                    ),
                ));
            }
            let each = |l: usize| seconds[l] + k.per_kilometre * km[l] / per_second;
            checked(c, (0..seconds.len()).map(each).collect(), &seconds)
        };
        let none = vec![0.0; seconds.len()];
        let metres = [none.clone(), none.clone(), none];
        Self::new(Mode::Walk, layer, classes, seconds.clone(), metres, FixedTimes::NONE, costs)
    }

    fn new(
        mode: Mode,
        layer: Layer<'a>,
        classes: &[Coefficients],
        seconds: Vec<f64>,
        [mixed_metres, lane_metres, separated_metres]: [Vec<f64>; 3],
        fixed: FixedTimes,
        costs: impl Fn(usize, &Coefficients) -> Result<Vec<f64>, AssignError>,
    ) -> Result<Self, AssignError> {
        let network = layer.network.network();
        let mut vectors: Vec<Vec<f64>> = Vec::new();
        let mut of_class = Vec::with_capacity(classes.len());
        for (c, k) in classes.iter().enumerate() {
            let v = costs(c, k)?;
            let same = |w: &Vec<f64>| w.iter().zip(&v).all(|(a, b)| a.to_bits() == b.to_bits());
            match vectors.iter().position(same) {
                Some(i) => of_class.push(i),
                None => {
                    of_class.push(vectors.len());
                    vectors.push(v);
                }
            }
        }
        let contexts = vectors
            .into_iter()
            .map(|v| SearchContext::with_costs(network, layer.turns, v))
            .collect();
        Ok(Self {
            mode,
            network,
            seconds,
            mixed_metres,
            lane_metres,
            separated_metres,
            contexts,
            of_class,
            fixed,
            snapper: NodeSnapper::every_node(network),
            sets: None,
        })
    }

    /// The layer's graph.
    #[must_use]
    pub fn network(&self) -> &'a RoadNetwork {
        self.network
    }

    /// The fixed times of every leg of this planner that uses a link.
    #[must_use]
    pub fn fixed_times(&self) -> FixedTimes {
        self.fixed
    }

    /// Whether this planner makes a sampled set of routes per pair ([`BikeRoutes::Sampled`]).
    #[must_use]
    pub fn has_route_sets(&self) -> bool {
        self.sets.is_some()
    }

    /// The sampled route sets of the pairs `keys` (pairs of this planner's layer nodes; a pair
    /// of one node has no set), each set best first; `None` for a planner of one route per pair.
    /// The sets do not depend on the order of `keys` or on the number of threads.
    #[must_use]
    pub fn route_sets(&self, keys: &[RouteKey]) -> Option<RouteSets> {
        let maker = self.sets.as_ref()?;
        Some(RouteSets::generate_in(&maker.context, keys, maker.generator.as_ref()))
    }

    /// The generator of the sampled route sets, with every option, as a canonical string;
    /// `None` for a planner of one route per pair.
    #[must_use]
    pub fn route_sets_descriptor(&self) -> Option<String> {
        self.sets.as_ref().map(|m| m.generator.descriptor())
    }

    /// The place on this planner's layer a point is snapped to: the layer's nearest node.
    #[must_use]
    pub fn place(&self, point: LonLat) -> Place {
        let node = self.snapper.nearest(self.network, point);
        match self.mode {
            Mode::Bike => Place::Bike(node),
            _ => Place::Walk(node),
        }
    }

    /// The node of `place` if it is on this planner's layer.
    fn node(&self, place: Place) -> Option<NodeId> {
        match (self.mode, place) {
            (Mode::Bike, Place::Bike(n)) | (Mode::Walk, Place::Walk(n)) => Some(n),
            _ => None,
        }
    }

    /// The attributes of a leg along `links`: see the [module docs](self).
    fn price(&self, links: &[LinkId]) -> Attributes {
        let mut time = 0.0;
        let mut length = 0.0;
        let (mut mixed, mut lane, mut separated) = (0.0, 0.0, 0.0);
        for &l in links {
            time += self.seconds[l.index()];
            length += self.network.link_length(l).get();
            mixed += self.mixed_metres[l.index()];
            lane += self.lane_metres[l.index()];
            separated += self.separated_metres[l.index()];
        }
        let fixed = self.fixed_of(links);
        let attributes = match self.mode {
            Mode::Bike => Attributes {
                in_vehicle_seconds: time,
                length_metres: length,
                bike_mixed_metres: mixed,
                bike_lane_metres: lane,
                bike_separated_metres: separated,
                ..Attributes::default()
            },
            _ => Attributes { walk_seconds: time, length_metres: length, ..Attributes::default() },
        };
        fixed.added_to(attributes)
    }

    /// The fixed times of a leg along `links`: the planner's, or none for a leg of no link.
    fn fixed_of(&self, links: &[LinkId]) -> FixedTimes {
        if links.is_empty() { FixedTimes::NONE } else { self.fixed }
    }

    /// The leg along `links` (a route of this planner's layer from `from` to `to`) departing at
    /// `departure`: how a route found elsewhere (given by hand, or read back) becomes a leg.
    #[must_use]
    pub fn leg_along(&self, from: Place, to: Place, departure: f64, links: Vec<LinkId>) -> Leg {
        let attributes = self.price(&links);
        let fixed = self.fixed_of(&links);
        Leg { mode: self.mode, from, to, departure, attributes, fixed, links }
    }

    /// A search over the context of `class`.
    pub(crate) fn search_for(&self, class: UserClassId) -> Option<Search<'_>> {
        let &c = self.of_class.get(class.index())?;
        Some(Search::new(&self.contexts[c]))
    }

    /// The index of the search context `class` searches with.
    pub(crate) fn context_of(&self, class: UserClassId) -> Option<usize> {
        self.of_class.get(class.index()).copied()
    }

    /// A search over context `context`.
    pub(crate) fn search_in(&self, context: usize) -> Search<'_> {
        Search::new(&self.contexts[context])
    }

    /// The route of least search cost from `origin` to `destination` by `search`; `None` if
    /// there is none, no links if they are one node.
    pub(crate) fn route(
        search: &mut Search<'_>,
        origin: NodeId,
        destination: NodeId,
    ) -> Option<Vec<LinkId>> {
        if origin == destination {
            return Some(Vec::new());
        }
        let found = search.shortest_tree(origin, &[destination], f64::INFINITY);
        found.into_iter().next().flatten().map(|r| r.links)
    }
}

/// `costs` if every link that has a time costs a finite number of at least 0; the error naming
/// class `class` otherwise.
fn checked(class: usize, costs: Vec<f64>, seconds: &[f64]) -> Result<Vec<f64>, AssignError> {
    for (l, (&c, &t)) in costs.iter().zip(seconds).enumerate() {
        if t.is_finite() && !(c.is_finite() && c >= 0.0) {
            return Err(class_error(
                class,
                format!("link {l} of the layer would cost {c} seconds-equivalent"),
            ));
        }
    }
    Ok(costs)
}

impl LegPlanner for StaticPlanner<'_> {
    fn mode(&self) -> Mode {
        self.mode
    }

    /// The leg of least search cost on the layer; the state and the cost source are not read
    /// (a layer's times are static).
    fn plan(
        &self,
        from: Place,
        to: Place,
        departure: f64,
        class: UserClassId,
        _state: &State,
        _cost_source: CostSource,
    ) -> Option<Leg> {
        let (origin, destination) = (self.node(from)?, self.node(to)?);
        let mut search = self.search_for(class)?;
        let links = Self::route(&mut search, origin, destination)?;
        Some(self.leg_along(from, to, departure, links))
    }

    /// The same attributes again: a layer's times do not change.
    fn reprice(&self, leg: &mut Leg, _state: &State) {
        leg.attributes = self.price(&leg.links);
    }
}
