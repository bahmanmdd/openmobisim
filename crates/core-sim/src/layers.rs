//! Bike and walk trips in a run (S195): routed on their own layer, once per
//! run, and traversed at the layer's static speeds.
//!
//! A bike or walk trip never enters the car's route sets, its choice or its
//! gap: its layer's costs do not depend on anyone else, so its route is the
//! shortest under the layer's cost ([`BikeCost`] for bikes, time for walkers)
//! and the same in every iteration. **Nothing here runs for a scenario with
//! no bike or walk trips** — a car-only run is what it was.
//!
//! **Cost:** a turn table and two cost vectors per layer (built with the
//! run), one shortest-path search per distinct origin-destination pair of the
//! layer's trips (once per run), and O(route links) per trip per loading.

use std::sync::Arc;

use openmobisim_core_demand::{Mode, Trips};
use openmobisim_core_graph::defaults::SignalDefaults;
use openmobisim_core_graph::layers::{BikeCost, StaticLayer, StaticLayerDefaults, StaticNetwork};
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_routes::generate::RouteSetGenerator;
use openmobisim_core_routes::search::{Route, Search, SearchContext};
use openmobisim_core_routes::{NodeSnapper, RouteKey, RouteSets};
use openmobisim_core_types::ids::{EntityId, LinkId, NodeId, TripId};

/// One static layer as a run uses it: its graph, turns, link times and
/// search costs.
#[derive(Debug)]
pub struct LayerSetup {
    network: Arc<StaticNetwork>,
    turns: TurnTable,
    seconds: Vec<f64>,
    costs: Vec<f64>,
    cost: BikeCost,
}

impl LayerSetup {
    /// Prepare `network` for a run: routes by `cost` (bikes; walkers always by
    /// time), with the mixed-traffic multiplier of `defaults`.
    #[must_use]
    pub fn new(network: Arc<StaticNetwork>, cost: BikeCost, defaults: StaticLayerDefaults) -> Self {
        let turns = TurnTable::build(network.network(), SignalDefaults::SHIPPED);
        let seconds = network.link_seconds();
        let costs = network.link_costs(cost, defaults.bike_mixed_cost_factor);
        Self { network, turns, seconds, costs, cost }
    }

    /// The layer's graph and speeds.
    #[must_use]
    pub fn network(&self) -> &Arc<StaticNetwork> {
        &self.network
    }

    /// Every link's travel time in seconds.
    #[must_use]
    pub fn seconds(&self) -> &[f64] {
        &self.seconds
    }

    /// Every link's search cost.
    #[must_use]
    pub fn costs(&self) -> &[f64] {
        &self.costs
    }

    /// The bike cost the layer's routes are searched by.
    #[must_use]
    pub fn cost(&self) -> BikeCost {
        self.cost
    }

    /// The layer's turns.
    #[must_use]
    pub fn turns(&self) -> &TurnTable {
        &self.turns
    }
}

/// The static layers a run has: neither by default.
#[derive(Debug, Default)]
pub struct StaticLayers {
    /// The bike layer, for trips whose mode is [`Mode::Bike`].
    pub bike: Option<LayerSetup>,
    /// The walk layer, for trips whose mode is [`Mode::Walk`].
    pub walk: Option<LayerSetup>,
}

impl StaticLayers {
    /// The layer a mode travels on, if the mode has a static layer and the run
    /// has it.
    #[must_use]
    pub fn for_mode(&self, mode: Mode) -> Option<&LayerSetup> {
        match mode {
            Mode::Bike => self.bike.as_ref(),
            Mode::Walk => self.walk.as_ref(),
            _ => None,
        }
    }

    /// The layer by name.
    #[must_use]
    pub fn get(&self, layer: StaticLayer) -> Option<&LayerSetup> {
        match layer {
            StaticLayer::Bike => self.bike.as_ref(),
            StaticLayer::Walk => self.walk.as_ref(),
        }
    }
}

/// The defaults of mode choice's walk and bike alternatives (S209):
/// how long a walk or a ride may take to be offered to a trip that chooses its mode.
///
/// A trip with a stated walk or bike mode takes it at any length; only a trip choosing its
/// mode is offered a walk or a ride within these times, so the run neither searches nor
/// costs a two-hour walk whose probability is next to nothing. Part of the defaults table
/// ([`openmobisim_core_graph::DEFAULTS_VERSION`] 7), overridable by name
/// ([`Self::from_options`]; `Scenario(mode_options=…)` in Python) and in the run's
/// fingerprint.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ModeDefaults {
    /// The longest walk mode choice offers as a walk alternative, in seconds.
    ///
    /// *Uncalibrated: design §21.1's main-mode cut-off of 30 minutes for a whole trip on
    /// foot. CITATION OWED (walk trip-length distributions of national travel surveys).*
    pub walk_max_s: f64,
    /// The longest ride mode choice offers as a bike alternative, in seconds.
    ///
    /// *Uncalibrated: 60 minutes (S235; design §21.1's 30 until then, which left cyclists
    /// without a ride home and pushed them into bike-and-ride, S233). CITATION OWED (bike
    /// trip-length distributions; e-bikes ride further).*
    pub bike_max_s: f64,
}

impl ModeDefaults {
    /// The shipped values.
    pub const SHIPPED: ModeDefaults = ModeDefaults { walk_max_s: 1800.0, bike_max_s: 3600.0 };

    /// The names of the options.
    pub const NAMES: [&'static str; 2] = ["walk_max_s", "bike_max_s"];

    /// Every option's value, by name, in [`Self::NAMES`]' order (S230: the parameter listing).
    #[must_use]
    pub fn values(&self) -> Vec<(&'static str, f64)> {
        vec![("walk_max_s", self.walk_max_s), ("bike_max_s", self.bike_max_s)]
    }

    /// The shipped values with `options` in place of their namesakes.
    ///
    /// # Errors
    ///
    /// The name and the list of known names for an unknown option; the reason for a
    /// value that is not a positive number (`inf` offers every walk and ride).
    pub fn from_options(options: &std::collections::BTreeMap<String, f64>) -> Result<Self, String> {
        let mut d = Self::SHIPPED;
        for (name, &value) in options {
            let slot = match name.as_str() {
                "walk_max_s" => &mut d.walk_max_s,
                "bike_max_s" => &mut d.bike_max_s,
                _ => {
                    return Err(format!(
                        "mode_options has no {name:?}; the options are: {}",
                        Self::NAMES.join(", ")
                    ));
                }
            };
            if value.is_nan() || value <= 0.0 {
                return Err(format!("mode_options {name:?} must be above 0, got {value}"));
            }
            *slot = value;
        }
        Ok(d)
    }

    /// The longest leg on `layer` mode choice offers, in seconds.
    #[must_use]
    pub fn max_seconds(&self, layer: StaticLayer) -> f64 {
        match layer {
            StaticLayer::Bike => self.bike_max_s,
            StaticLayer::Walk => self.walk_max_s,
        }
    }
}

impl Default for ModeDefaults {
    fn default() -> Self {
        Self::SHIPPED
    }
}

/// A traveller class's own choice-set limits (S235, roadmap I-ax): the longest walk and ride
/// mode choice offers its travellers, and the longest walk to or from a stop they take. Each
/// `None` is the run's ([`ModeDefaults`], `TransitDefaults::access_walk_max_s`).
///
/// A limit bounds a choice set, not a preference: the minutes within it are weighed by the
/// class's coefficients. Every value is the user's, uncalibrated; a run's fingerprint and
/// manifest record those given.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct ClassLimits {
    /// The longest walk offered as a walk alternative, in seconds.
    pub walk_max_s: Option<f64>,
    /// The longest ride offered as a bike alternative, in seconds.
    pub bike_max_s: Option<f64>,
    /// The longest walk from an origin to a stop, or from a stop to a destination, in seconds.
    pub access_walk_max_s: Option<f64>,
}

impl ClassLimits {
    /// The names of the limits, as [`Self::from_options`] takes them.
    pub const NAMES: [&'static str; 3] = ["walk_max_s", "bike_max_s", "access_walk_max_s"];

    /// The limits named in `options`, the others the run's.
    ///
    /// # Errors
    ///
    /// The name and the list of names for an unknown limit; the reason for a value that is
    /// not above 0 (`inf` is allowed for a walk or a ride, as in [`ModeDefaults`], not for
    /// the walk to a stop, which is searched).
    pub fn from_options(options: &std::collections::BTreeMap<String, f64>) -> Result<Self, String> {
        let mut out = Self::default();
        for (name, &value) in options {
            let (slot, finite) = match name.as_str() {
                "walk_max_s" => (&mut out.walk_max_s, false),
                "bike_max_s" => (&mut out.bike_max_s, false),
                "access_walk_max_s" => (&mut out.access_walk_max_s, true),
                _ => {
                    return Err(format!(
                        "a class has no limit {name:?}; the limits are: {}",
                        Self::NAMES.join(", ")
                    ));
                }
            };
            if value.is_nan() || value <= 0.0 || (finite && value.is_infinite()) {
                return Err(format!(
                    "a class's {name:?} must be a {}number above 0, got {value}",
                    if finite { "finite " } else { "" }
                ));
            }
            *slot = Some(value);
        }
        Ok(out)
    }

    /// Whether the class says no limit of its own.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// The limits given, by name, in [`Self::NAMES`]' order.
    #[must_use]
    pub fn values(&self) -> Vec<(&'static str, f64)> {
        [
            ("walk_max_s", self.walk_max_s),
            ("bike_max_s", self.bike_max_s),
            ("access_walk_max_s", self.access_walk_max_s),
        ]
        .into_iter()
        .filter_map(|(n, v)| v.map(|v| (n, v)))
        .collect()
    }

    /// The class's longest walk and ride, its own where given, `run`'s otherwise.
    #[must_use]
    pub fn modes(&self, run: &ModeDefaults) -> ModeDefaults {
        ModeDefaults {
            walk_max_s: self.walk_max_s.unwrap_or(run.walk_max_s),
            bike_max_s: self.bike_max_s.unwrap_or(run.bike_max_s),
        }
    }
}

/// Class `class`'s limits in `classes` (by class index); none listed: the run's.
pub(crate) fn class_limits(classes: &[ClassLimits], class: usize) -> ClassLimits {
    classes.get(class).copied().unwrap_or_default()
}

/// Shortest routes, searched only as far as a bound for the keys only mode-choice trips use
/// (their walk or ride is offered only up to [`ModeDefaults`]' times, or their class's), in
/// full for the rest (a trip given the mode takes it at any length).
struct ShortestFor {
    /// Keys searched within a bound, sorted, each with its bound in cost: the longest any
    /// of its trips is offered (S235: a class's own limit bounds only its pairs).
    bounded: Vec<(RouteKey, f64)>,
}

impl RouteSetGenerator for ShortestFor {
    fn name(&self) -> &str {
        "shortest"
    }

    fn descriptor(&self) -> String {
        "shortest".to_string()
    }

    fn generate(&self, search: &mut Search<'_>, origin: NodeId, destination: NodeId) -> Vec<Route> {
        let key = RouteKey::new(origin, destination);
        if let Ok(i) = self.bounded.binary_search_by_key(&key, |&(k, _)| k) {
            search.shortest_within(origin, destination, self.bounded[i].1).into_iter().collect()
        } else {
            search.shortest(origin, destination).into_iter().collect()
        }
    }
}

/// A trip's route on its static layer, as [`StaticRoutes`] holds it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum StaticRoute {
    /// Not a trip on a static layer, or its layer is missing.
    None,
    /// Origin and destination are one node: nothing to traverse.
    Here,
    /// The layer has no route between them.
    Unreachable,
    /// The route with this index in its layer's sets.
    Route(u32),
}

/// Every bike and walk trip's route, made once per run; and, for a trip choosing its
/// mode (M5), its route on each of the two layers.
#[derive(Debug, Default)]
pub(crate) struct StaticRoutes {
    /// The bike layer's route sets, then the walk layer's.
    sets: [Option<RouteSets>; 2],
    /// Per layer, per trip.
    route: [Vec<StaticRoute>; 2],
}

fn slot(layer: StaticLayer) -> usize {
    match layer {
        StaticLayer::Bike => 0,
        StaticLayer::Walk => 1,
    }
}

fn layer_of(mode: Mode) -> Option<StaticLayer> {
    match mode {
        Mode::Bike => Some(StaticLayer::Bike),
        Mode::Walk => Some(StaticLayer::Walk),
        _ => None,
    }
}

impl StaticRoutes {
    /// Route every trip whose mode has a static layer the run has, and every trip
    /// `also` names for a layer: snap its ends to the layer's nodes and take the shortest
    /// route between them. `max_seconds` is the longest leg mode choice offers a trip on a
    /// layer (its class's limit or the run's).
    pub(crate) fn build(
        layers: &StaticLayers,
        trips: &Trips,
        also: &dyn Fn(TripId, StaticLayer) -> bool,
        max_seconds: &dyn Fn(TripId, StaticLayer) -> f64,
    ) -> Self {
        let total = trips.len() as usize;
        let mut route = [vec![StaticRoute::None; total], vec![StaticRoute::None; total]];
        let mut sets: [Option<RouteSets>; 2] = [None, None];
        for layer in [StaticLayer::Bike, StaticLayer::Walk] {
            let Some(setup) = layers.get(layer) else { continue };
            let stated = |trip: TripId| layer_of(trips.mode(trip)) == Some(layer);
            let wanted: Vec<usize> = (0..total)
                .filter(|&i| {
                    let trip = TripId::from_index(i);
                    stated(trip) || also(trip, layer)
                })
                .collect();
            if wanted.is_empty() {
                continue;
            }
            let graph = setup.network.network();
            let snapper = NodeSnapper::every_node(graph);
            let keys: Vec<RouteKey> = wanted
                .iter()
                .map(|&i| {
                    let trip = TripId::from_index(i);
                    RouteKey::new(
                        snapper.nearest(graph, trips.origin(trip)),
                        snapper.nearest(graph, trips.destination(trip)),
                    )
                })
                .collect();
            // Keys only mode-choice trips use are searched no further than the longest leg
            // mode choice offers any of them, in cost: a leg of that many seconds costs at
            // most that times the layer's largest cost per second (the dedicated-bike
            // multiplier).
            let mut full: Vec<RouteKey> = wanted
                .iter()
                .zip(&keys)
                .filter(|&(&i, _)| stated(TripId::from_index(i)))
                .map(|(_, &k)| k)
                .collect();
            full.sort_unstable();
            let per_second = setup
                .costs
                .iter()
                .zip(&setup.seconds)
                .filter(|&(c, t)| c.is_finite() && *t > 0.0)
                .fold(1.0_f64, |m, (c, t)| m.max(c / t));
            let mut bounded: Vec<(RouteKey, f64)> = wanted
                .iter()
                .zip(&keys)
                .filter(|&(_, k)| full.binary_search(k).is_err())
                .map(|(&i, &k)| (k, max_seconds(TripId::from_index(i), layer) * per_second))
                .collect();
            bounded.sort_unstable_by(|a, b| a.0.cmp(&b.0).then(b.1.total_cmp(&a.1)));
            bounded.dedup_by_key(|&mut (k, _)| k);
            let ctx = SearchContext::with_costs(graph, &setup.turns, setup.costs.clone());
            let layer_sets = RouteSets::generate_in(&ctx, &keys, &ShortestFor { bounded });
            for (&i, key) in wanted.iter().zip(&keys) {
                route[slot(layer)][i] = if key.origin == key.destination {
                    StaticRoute::Here
                } else {
                    match layer_sets.key_index(*key).map(|k| layer_sets.route_range(k)) {
                        Some(range) if !range.is_empty() => StaticRoute::Route(
                            u32::try_from(range.start).expect("route indices fit u32"),
                        ),
                        _ => StaticRoute::Unreachable,
                    }
                };
            }
            sets[slot(layer)] = Some(layer_sets);
        }
        Self { sets, route }
    }

    /// The trip's route on `layer`.
    pub(crate) fn of(&self, trip: TripId, layer: StaticLayer) -> StaticRoute {
        self.route[slot(layer)].get(trip.index()).copied().unwrap_or(StaticRoute::None)
    }

    /// The trip's route on `layer` as a leg to choose (M5): its links, seconds and metres.
    /// `None` if it has none there, or if it takes longer than `max_seconds` (mode choice's
    /// cut-off, [`ModeDefaults`]).
    pub(crate) fn leg(
        &self,
        layers: &StaticLayers,
        trip: TripId,
        layer: StaticLayer,
        max_seconds: f64,
    ) -> Option<crate::itinerary_choice::StaticLeg> {
        let setup = layers.get(layer)?;
        match self.of(trip, layer) {
            StaticRoute::None | StaticRoute::Unreachable => None,
            StaticRoute::Here => Some(crate::itinerary_choice::StaticLeg {
                links: Vec::new(),
                seconds: 0.0,
                metres: 0.0,
            }),
            StaticRoute::Route(index) => {
                let links = self.links(layer, index);
                let graph = setup.network.network();
                let seconds: f64 = links.iter().map(|l| setup.seconds[l.index()]).sum();
                (seconds <= max_seconds).then(|| crate::itinerary_choice::StaticLeg {
                    seconds,
                    metres: links.iter().map(|&l| graph.link_length(l).get()).sum(),
                    links,
                })
            }
        }
    }

    /// The links of route `index` on `layer`.
    pub(crate) fn links(&self, layer: StaticLayer, index: u32) -> Vec<LinkId> {
        let sets = self.sets[slot(layer)].as_ref().expect("a routed layer has sets");
        sets.route(index as usize).links.iter().map(|&l| LinkId::new(l)).collect()
    }
}

/// The static layer a mode travels on, if it has one.
#[must_use]
pub fn static_layer_of(mode: Mode) -> Option<StaticLayer> {
    layer_of(mode)
}
