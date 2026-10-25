//! Bike and walk trips in a run: routed on their own layer, once per
//! run, and traversed at the layer's static speeds.
//!
//! A bike or walk trip never enters the car's route sets, its choice or its
//! gap: its layer's costs do not depend on anyone else, so its route is the
//! shortest under the layer's cost ([`BikeCost`] for bikes, time for walkers)
//! and the same in every iteration. **Nothing here runs for a scenario with
//! no bike or walk trips** — a car-only run is what it was.
//!
//! **Bike route sets, an option (S298).** Given [`BikeRouting::Sampled`], the
//! bike layer holds a **route set** per origin-destination pair instead of one
//! route: the `sampled` generator of `openmobisim_core_routes` searches the
//! layer under its bike cost, first the least-cost route, then the cheapest
//! route under each user class's generalised cost (the extra seconds per link
//! the run composed from the choice model, per class and preference draw) with
//! random link costs, keeping the routes within a detour of the first and under
//! an overlap limit. Each trip rides the first route of its pair's set, the
//! least-cost one, until a choice among them records another; a ride longer
//! than mode choice offers is left out when the leg is read, as before. The
//! walk layer always has one route per pair, and [`BikeRouting::Shortest`]
//! (what a run uses) routes bikes exactly as above.
//!
//! **Cost:** a turn table and two cost vectors per layer (built with the
//! run), one shortest-path search per distinct origin-destination pair of the
//! layer's trips (once per run), and O(route links) per trip per loading.
//! Sampled bike sets search one tree per origin for the least-cost routes and
//! one per class and draw, and store every kept route.

use std::sync::Arc;

use openmobisim_core_demand::{Mode, Trips};
use openmobisim_core_graph::hubs::ParkingKind;
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
    /// time), with the painted-lane and mixed-traffic multipliers of `defaults`.
    #[must_use]
    pub fn new(network: Arc<StaticNetwork>, cost: BikeCost, defaults: StaticLayerDefaults) -> Self {
        let turns = TurnTable::build(network.network(), network.network().signals());
        let seconds = network.link_seconds();
        let costs = network.link_costs(
            cost,
            defaults.bike_lane_cost_factor,
            defaults.bike_mixed_cost_factor,
        );
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

/// The defaults of mode choice's walk and bike alternatives:
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
    /// *Uncalibrated: the main-mode cut-off of 30 minutes for a whole trip on
    /// foot; England's average walking trip took 18 minutes in 2022 (Department for Transport,
    /// *Walking factsheet* 2022,
    /// <https://assets.publishing.service.gov.uk/media/64e71a3d20ae89000df26d2b/walking-factsheet-2022.pdf>).*
    pub walk_max_s: f64,
    /// The longest ride mode choice offers as a bike alternative, in seconds.
    ///
    /// *Uncalibrated: 60 minutes (a shorter limit leaves cyclists
    /// without a ride home and pushes them into bike-and-ride). About 80 % of Dutch bicycle
    /// trips are under 5 km (KiM, *Cycling facts: new insights*, 2020,
    /// <https://english.kimnet.nl/documents/2020/11/03/cycling-facts-new-insights>); the limit
    /// keeps the longer ones, e-bikes' among them.*
    pub bike_max_s: f64,
    /// Whether a trip left with no alternative at all — no mode it may use reaches its
    /// destination within the limits — travels by car instead, counted and reported apart; a trip
    /// with no car path either stays without one. For a traveller without a car the car stands
    /// for a lift or a taxi: no car of theirs moves. **Experimental, off by default** (`0`):
    /// such trips stay unsimulated, and the run reports each with why every mode gave
    /// it none (`RunResult::no_alternative`); `1` turns the fallback on.
    pub fallback_car: bool,
    /// The time a car trip takes before its drive, reaching the car and setting off, in
    /// seconds. **Where the fixed times apply:** in a run that compares modes or has a
    /// timetable, every trip's time is door to door, so every car trip pays this time at its
    /// departure, at home and when the car is fetched from a car park on the way back: the
    /// choice counts it (as walk time) and the loading enters the car on the roads this long
    /// after the departure. A run of stated car trips alone, with no timetable, is a car
    /// assignment: its times are the drives and it pays none of the fixed times, so it compares
    /// with the benchmarks as they count. *Uncalibrated: 120 s, an assumption with no published
    /// source; 0 switches it off.*
    pub car_access_s: f64,
    /// The time a car trip takes after its drive, finding a place and walking from it, in
    /// seconds: paid wherever a car is left, at the destination and at a car park alike (a car
    /// park adds its occupancy's time on top, `parking_options` `slope_car_s`), in the choice
    /// (as parking time) and in the loading (the trip ends this long after the car arrives),
    /// under the rule of [`Self::car_access_s`]. *Uncalibrated: 300 s, an assumption; cruising
    /// for parking alone averaged 8 minutes in sixteen downtown studies (Shoup 2006, *Cruising
    /// for parking*, Transport Policy 13(6), 479–486,
    /// <https://doi.org/10.1016/j.tranpol.2006.05.005>), far above a city-wide mean that counts
    /// home and workplace parking; 0 switches it off.*
    pub car_egress_s: f64,
    /// The time a bike trip takes before its ride, reaching and unlocking the bike, in seconds:
    /// paid as [`Self::car_access_s`] is, at the departure and when the bike is fetched from a
    /// bike parking, in the choice (as walk time) and in the loading. *Uncalibrated: 60 s, an
    /// assumption with no published source; 0 switches it off.*
    pub bike_access_s: f64,
    /// The time a bike trip takes after its ride, parking and locking the bike, in seconds: paid
    /// wherever a bike is left, at the destination and at a bike parking alike (a bike parking
    /// adds its occupancy's time on top, `parking_options` `slope_bike_s`), in the choice (as
    /// parking time) and in the loading. *Uncalibrated: 60 s, an assumption with no published
    /// source; 0 switches it off.*
    pub bike_egress_s: f64,
    /// How close to a motorway-only node a trip's end must lie to join the roads there, in
    /// metres. A trip's end snaps to the nearest **street** node — one with a road
    /// a car can enter from; motorways, trunk roads and their ramps are not — unless it lies
    /// within this distance of a node that has only such roads, which it then takes: a through
    /// trip placed on the motorway by design keeps its node, an address beside the motorway does
    /// not start on it. Read by every car trip, with or without mode choice. *Uncalibrated:
    /// 50 m, an assumption (a gateway row lies on its node, an address never within 50 m of a
    /// carriageway's node); 0 sends every trip to a street, `inf` restores the plain nearest
    /// drivable node.*
    pub car_snap_motorway_m: f64,
}

impl ModeDefaults {
    /// The time to reach a vehicle of `kind` before its leg ([`Self::car_access_s`],
    /// [`Self::bike_access_s`]).
    #[must_use]
    pub fn access_s(&self, kind: ParkingKind) -> f64 {
        match kind {
            ParkingKind::Car => self.car_access_s,
            ParkingKind::Bike => self.bike_access_s,
        }
    }

    /// The time to leave a vehicle of `kind` after its leg ([`Self::car_egress_s`],
    /// [`Self::bike_egress_s`]).
    #[must_use]
    pub fn egress_s(&self, kind: ParkingKind) -> f64 {
        match kind {
            ParkingKind::Car => self.car_egress_s,
            ParkingKind::Bike => self.bike_egress_s,
        }
    }

    /// The shipped values.
    pub const SHIPPED: ModeDefaults = ModeDefaults {
        walk_max_s: 1800.0,
        bike_max_s: 3600.0,
        fallback_car: false,
        car_access_s: 120.0,
        car_egress_s: 300.0,
        bike_access_s: 60.0,
        bike_egress_s: 60.0,
        car_snap_motorway_m: 50.0,
    };

    /// The names of the options.
    pub const NAMES: [&'static str; 8] = [
        "walk_max_s",
        "bike_max_s",
        "fallback_car",
        "car_access_s",
        "car_egress_s",
        "bike_access_s",
        "bike_egress_s",
        "car_snap_motorway_m",
    ];

    /// Every option's value, by name, in [`Self::NAMES`]' order (the parameter listing).
    #[must_use]
    pub fn values(&self) -> Vec<(&'static str, f64)> {
        vec![
            ("walk_max_s", self.walk_max_s),
            ("bike_max_s", self.bike_max_s),
            ("fallback_car", f64::from(u8::from(self.fallback_car))),
            ("car_access_s", self.car_access_s),
            ("car_egress_s", self.car_egress_s),
            ("bike_access_s", self.bike_access_s),
            ("bike_egress_s", self.bike_egress_s),
            ("car_snap_motorway_m", self.car_snap_motorway_m),
        ]
    }

    /// The shipped values with `options` in place of their namesakes.
    ///
    /// # Errors
    ///
    /// The name and the list of known names for an unknown option; the reason for a
    /// value that is not a positive number (`inf` offers every walk and ride; a time or a
    /// distance may be 0), or for `fallback_car` not 0 or 1.
    pub fn from_options(options: &std::collections::BTreeMap<String, f64>) -> Result<Self, String> {
        let mut d = Self::SHIPPED;
        for (name, &value) in options {
            if name == "fallback_car" {
                // A flag: exactly 0 or 1.
                d.fallback_car = match value.to_bits() {
                    b if b == 1.0_f64.to_bits() => true,
                    b if b == 0.0_f64.to_bits() => false,
                    _ => {
                        return Err(format!(
                            "mode_options \"fallback_car\" is 0 or 1, got {value}"
                        ));
                    }
                };
                continue;
            }
            // A limit must be above 0; a time may be 0 (off).
            let (slot, may_be_zero) = match name.as_str() {
                "walk_max_s" => (&mut d.walk_max_s, false),
                "bike_max_s" => (&mut d.bike_max_s, false),
                "car_access_s" => (&mut d.car_access_s, true),
                "car_egress_s" => (&mut d.car_egress_s, true),
                "bike_access_s" => (&mut d.bike_access_s, true),
                "bike_egress_s" => (&mut d.bike_egress_s, true),
                "car_snap_motorway_m" => (&mut d.car_snap_motorway_m, true),
                _ => {
                    return Err(format!(
                        "mode_options has no {name:?}; the options are: {}",
                        Self::NAMES.join(", ")
                    ));
                }
            };
            if value.is_nan() || value < 0.0 || (!may_be_zero && value <= 0.0) {
                let bound = if may_be_zero { "0 or above" } else { "above 0" };
                return Err(format!("mode_options {name:?} must be {bound}, got {value}"));
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

/// A traveller class's own choice-set limits: the longest walk and ride
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
            ..*run
        }
    }
}

/// Class `class`'s limits in `classes` (by class index); none listed: the run's.
pub(crate) fn class_limits(classes: &[ClassLimits], class: usize) -> ClassLimits {
    classes.get(class).copied().unwrap_or_default()
}

/// How bike routes are made (S298): one shortest route per pair, or sampled sets.
#[derive(Clone, Debug)]
pub enum BikeRouting {
    /// One least-cost route per pair under the layer's bike cost: today's behaviour.
    Shortest,
    /// Sampled sets: the `sampled` generator on the bike layer, with the extras per class and
    /// draw the run composed from the choice model (`extras[class][draw]`, `None`: time alone).
    Sampled {
        /// The generator, with its options (draws, noise, detour, overlap, size, seed).
        generator: openmobisim_core_routes::Sampled,
        /// Extra seconds per bike-layer link, by user class and preference draw.
        extras: openmobisim_core_routes::ClassExtras,
    },
}

impl BikeRouting {
    /// The generator with its extras, ready for the bike layer; `None` for
    /// [`Self::Shortest`].
    fn generator(&self) -> Option<Box<dyn RouteSetGenerator>> {
        match self {
            Self::Shortest => None,
            Self::Sampled { generator, extras } => Some(
                generator.with_extras(extras.clone()).expect("the sampled method takes extras"),
            ),
        }
    }

    /// How bike routes are made, as a canonical string: `shortest`, or the sampled
    /// generator's descriptor, whose `extras=given:<hash>` field names the extras.
    #[must_use]
    pub fn descriptor(&self) -> String {
        self.generator().map_or_else(|| "shortest".to_owned(), |g| g.descriptor())
    }
}

/// Shortest routes, searched only as far as a bound for the keys only mode-choice trips use
/// (their walk or ride is offered only up to [`ModeDefaults`]' times, or their class's), in
/// full for the rest (a trip given the mode takes it at any length).
struct ShortestFor {
    /// Keys searched within a bound, sorted, each with its bound in cost: the longest any
    /// of its trips is offered (a class's own limit bounds only its pairs).
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
        match self.bound(origin, destination) {
            Some(bound) => search.shortest_within(origin, destination, bound).into_iter().collect(),
            None => search.shortest(origin, destination).into_iter().collect(),
        }
    }

    fn searches_trees(&self) -> bool {
        true
    }

    /// One search tree for all of them, as far as the loosest of their bounds; a pair's
    /// route is kept if it is within the pair's own.
    fn generate_from(
        &self,
        search: &mut Search<'_>,
        origin: NodeId,
        destinations: &[NodeId],
    ) -> Vec<Vec<Route>> {
        let bounds: Vec<f64> =
            destinations.iter().map(|&d| self.bound(origin, d).unwrap_or(f64::INFINITY)).collect();
        let widest = bounds.iter().copied().fold(0.0, f64::max);
        search
            .shortest_tree(origin, destinations, widest)
            .into_iter()
            .zip(bounds)
            .map(|(route, bound)| route.filter(|r| r.cost <= bound).into_iter().collect())
            .collect()
    }
}

impl ShortestFor {
    /// The bound in cost a pair is searched within, `None` for a pair searched in full.
    fn bound(&self, origin: NodeId, destination: NodeId) -> Option<f64> {
        let key = RouteKey::new(origin, destination);
        let i = self.bounded.binary_search_by_key(&key, |&(k, _)| k).ok()?;
        Some(self.bounded[i].1)
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
/// mode, its route on each of the two layers.
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
    /// layer (its class's limit or the run's). `bike` says how the bike layer's routes are
    /// made: with [`BikeRouting::Sampled`] each pair has a set, generated in full (the
    /// mode-choice bound is applied when a leg is read), and each trip points at the first
    /// route of its pair's set.
    pub(crate) fn build(
        layers: &StaticLayers,
        trips: &Trips,
        also: &dyn Fn(TripId, StaticLayer) -> bool,
        max_seconds: &dyn Fn(TripId, StaticLayer) -> f64,
        bike: &BikeRouting,
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
            let ctx = SearchContext::with_costs(graph, &setup.turns, setup.costs.clone());
            let sampled = match layer {
                StaticLayer::Bike => bike.generator(),
                StaticLayer::Walk => None,
            };
            let layer_sets = if let Some(generator) = sampled {
                // Sampled bike sets (S298), searched in full: a ride longer than mode choice
                // offers is left out when its leg is read (`leg`).
                RouteSets::generate_in(&ctx, &keys, generator.as_ref())
            } else {
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
                RouteSets::generate_in(&ctx, &keys, &ShortestFor { bounded })
            };
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

    /// The route sets of `layer`, if the run routed any trip on it.
    pub(crate) fn sets(&self, layer: StaticLayer) -> Option<&RouteSets> {
        self.sets[slot(layer)].as_ref()
    }

    /// The range of route indices of the trip's pair in `layer`'s sets: one route, or a
    /// sampled bike set. `None` where the trip has no route there (not routed on the layer,
    /// unreachable, or origin and destination one node).
    pub(crate) fn routes_of(
        &self,
        trip: TripId,
        layer: StaticLayer,
    ) -> Option<core::ops::Range<usize>> {
        let StaticRoute::Route(index) = self.of(trip, layer) else { return None };
        let sets = self.sets[slot(layer)].as_ref()?;
        Some(sets.route_range(sets.key_of_route(index as usize)))
    }

    /// Record route `index` of the trip's pair as its route on `layer` (a choice among its
    /// set).
    ///
    /// # Panics
    ///
    /// Panics if the trip has no route on `layer`, or if `index` is not in its pair's range
    /// ([`Self::routes_of`]).
    pub(crate) fn set_route(&mut self, trip: TripId, layer: StaticLayer, index: u32) {
        let range = self.routes_of(trip, layer).expect("the trip has a route on the layer");
        assert!(
            range.contains(&(index as usize)),
            "route {index} is not one of the trip's routes {range:?}"
        );
        self.route[slot(layer)][trip.index()] = StaticRoute::Route(index);
    }

    /// The trip's route on `layer` as a leg to choose: its links, seconds and metres.
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

    /// Every route of the trip's pair on `layer` as a leg to choose (S298), in the set's order
    /// (the shortest first), those taking at most `max_seconds`: several with bike route sets,
    /// one where the layer routes by the shortest route alone, one empty leg where the trip's
    /// ends are one node, none where it has no route there.
    pub(crate) fn legs(
        &self,
        layers: &StaticLayers,
        trip: TripId,
        layer: StaticLayer,
        max_seconds: f64,
    ) -> Vec<crate::itinerary_choice::StaticLeg> {
        let Some(setup) = layers.get(layer) else { return Vec::new() };
        match self.of(trip, layer) {
            StaticRoute::None | StaticRoute::Unreachable => Vec::new(),
            StaticRoute::Here => vec![crate::itinerary_choice::StaticLeg {
                links: Vec::new(),
                seconds: 0.0,
                metres: 0.0,
            }],
            StaticRoute::Route(_) => {
                let graph = setup.network.network();
                let range = self.routes_of(trip, layer).expect("a routed trip has a range");
                range
                    .filter_map(|r| {
                        let links = self.links(layer, u32::try_from(r).expect("routes fit u32"));
                        let seconds: f64 = links.iter().map(|l| setup.seconds[l.index()]).sum();
                        (seconds <= max_seconds).then(|| crate::itinerary_choice::StaticLeg {
                            seconds,
                            metres: links.iter().map(|&l| graph.link_length(l).get()).sum(),
                            links,
                        })
                    })
                    .collect()
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

#[cfg(test)]
mod tests {
    //! Bike route sets (S298): the shortest option as before, sampled sets that hold more
    //! than one route, extras that move them, determinism, and recording a chosen route.

    use std::collections::BTreeMap;

    use openmobisim_core_demand::{ClassDefaults, Ownership, RawTrip, build_travellers};
    use openmobisim_core_graph::LonLat;
    use openmobisim_core_graph::examples::node_name;
    use openmobisim_core_graph::network::RoadNetwork;
    use openmobisim_core_routes::{ClassExtras, Sampled, Shortest};
    use openmobisim_core_types::diagnostics::Diagnostics;
    use openmobisim_core_types::time::Second;

    use super::*;

    /// A 5 × 5 two-way grid of 200 m blocks, unsignalised.
    fn grid() -> RoadNetwork {
        openmobisim_core_graph::manhattan_grid(5, 200.0, false).0
    }

    fn layers(road: &RoadNetwork) -> StaticLayers {
        let d = StaticLayerDefaults::SHIPPED;
        let derive = |layer| {
            Arc::new(
                StaticNetwork::derive(road, layer, d, &mut Diagnostics::new()).expect("derives"),
            )
        };
        StaticLayers {
            bike: Some(LayerSetup::new(derive(StaticLayer::Bike), BikeCost::Dedicated, d)),
            walk: Some(LayerSetup::new(derive(StaticLayer::Walk), BikeCost::Dedicated, d)),
        }
    }

    /// Grid node `(row, col)`'s id in `network` (a road or a layer: they share node names).
    fn node(network: &RoadNetwork, row: u32, col: u32) -> NodeId {
        NodeId::new(network.node_external_ids().id_of(&node_name(row, col)).expect("a grid node"))
    }

    fn at(road: &RoadNetwork, row: u32, col: u32) -> LonLat {
        road.node_lonlat(node(road, row, col))
    }

    type Cell = (u32, u32);

    /// One trip per `(from, to, mode)`, each by its own traveller.
    fn trips(road: &RoadNetwork, rows: &[(Cell, Cell, Mode)]) -> Trips {
        let raw: Vec<RawTrip> = rows
            .iter()
            .enumerate()
            .map(|(i, &(from, to, mode))| RawTrip {
                traveller_id: format!("t{i:02}"),
                trip_seq: 0,
                origin: at(road, from.0, from.1),
                destination: at(road, to.0, to.1),
                departure_time: Second(0),
                user_class: "everyone".to_string(),
                weight: None,
                mode: Some(mode),
            })
            .collect();
        let owning = ClassDefaults::new()
            .with_default("everyone", Ownership { car: true, bike: true, transit_pass: false });
        build_travellers(raw, Vec::new(), &owning, 1, &mut Diagnostics::new()).expect("builds").1
    }

    /// Bike pairs across the grid, a walk and a car trip.
    fn mixed_rows() -> Vec<(Cell, Cell, Mode)> {
        vec![
            ((0, 0), (4, 4), Mode::Bike),
            ((4, 0), (0, 4), Mode::Bike),
            ((0, 0), (4, 4), Mode::Bike),
            ((1, 0), (3, 4), Mode::Bike),
            ((0, 2), (4, 2), Mode::Bike),
            ((2, 2), (2, 2), Mode::Bike),
            ((0, 0), (0, 2), Mode::Walk),
            ((0, 0), (4, 4), Mode::Car),
        ]
    }

    fn build(layers: &StaticLayers, trips: &Trips, bike: &BikeRouting) -> StaticRoutes {
        StaticRoutes::build(layers, trips, &|_, _| false, &|_, _| f64::INFINITY, bike)
    }

    fn sampled(generator: Sampled, extras: ClassExtras) -> BikeRouting {
        BikeRouting::Sampled { generator, extras }
    }

    fn four_draws() -> Sampled {
        let mut g = Sampled::default();
        g.draws = 4;
        g
    }

    fn bike_trips(trips: &Trips) -> impl Iterator<Item = TripId> + '_ {
        (0..trips.len()).map(TripId::new).filter(|&t| trips.mode(t) == Mode::Bike)
    }

    /// The links of each route of the trip's pair on the bike layer, in the set's order.
    fn routes(r: &StaticRoutes, trip: TripId) -> Vec<Vec<u32>> {
        let sets = r.sets(StaticLayer::Bike).expect("bike sets");
        r.routes_of(trip, StaticLayer::Bike)
            .map(|range| range.map(|i| sets.route(i).links.to_vec()).collect())
            .unwrap_or_default()
    }

    #[test]
    fn the_shortest_option_routes_as_before() {
        let road = grid();
        let layers = layers(&road);
        let trips = trips(&road, &mixed_rows());
        let shortest = build(&layers, &trips, &BikeRouting::Shortest);
        for trip in bike_trips(&trips) {
            match shortest.of(trip, StaticLayer::Bike) {
                StaticRoute::Here => assert!(shortest.routes_of(trip, StaticLayer::Bike).is_none()),
                route => {
                    let range = shortest.routes_of(trip, StaticLayer::Bike).expect("routed");
                    assert_eq!(range.len(), 1, "one route per pair");
                    let start = u32::try_from(range.start).expect("fits");
                    assert_eq!(route, StaticRoute::Route(start));
                }
            }
        }
        // The bike sets are the plain shortest generator's, searched under the bike cost.
        let setup = layers.bike.as_ref().expect("a bike layer");
        let graph = setup.network.network();
        let ctx = SearchContext::with_costs(graph, &setup.turns, setup.costs.clone());
        let keys: Vec<RouteKey> = bike_trips(&trips)
            .map(|t| {
                let snapper = NodeSnapper::every_node(graph);
                RouteKey::new(
                    snapper.nearest(graph, trips.origin(t)),
                    snapper.nearest(graph, trips.destination(t)),
                )
            })
            .collect();
        let plain = RouteSets::generate_in(&ctx, &keys, &Shortest);
        assert_eq!(shortest.sets(StaticLayer::Bike), Some(&plain));
        // Sampled bike sets leave the walk layer exactly as it was.
        let with_sets = build(&layers, &trips, &sampled(four_draws(), vec![vec![None; 5]]));
        assert_eq!(shortest.sets(StaticLayer::Walk), with_sets.sets(StaticLayer::Walk));
        for trip in (0..trips.len()).map(TripId::new) {
            assert_eq!(shortest.of(trip, StaticLayer::Walk), with_sets.of(trip, StaticLayer::Walk));
        }
    }

    #[test]
    fn sampled_sets_hold_several_connected_loop_free_routes_within_the_detour() {
        let road = grid();
        let layers = layers(&road);
        let trips = trips(&road, &mixed_rows());
        let generator = four_draws();
        let detour = generator.max_detour;
        let shortest = build(&layers, &trips, &BikeRouting::Shortest);
        let sets = build(&layers, &trips, &sampled(generator, vec![vec![None; 5]]));
        let setup = layers.bike.as_ref().expect("a bike layer");
        let graph = setup.network.network();
        let cost = |links: &[u32]| -> f64 { links.iter().map(|&l| setup.costs[l as usize]).sum() };
        let mut several = 0;
        for trip in bike_trips(&trips) {
            let StaticRoute::Route(first) = sets.of(trip, StaticLayer::Bike) else { continue };
            let range = sets.routes_of(trip, StaticLayer::Bike).expect("routed");
            assert_eq!(first as usize, range.start, "a trip rides the first route of its set");
            let found = routes(&sets, trip);
            several += usize::from(found.len() >= 2);
            // The first is a least-cost route: it costs what the shortest option's does.
            let best = cost(&routes(&shortest, trip)[0]);
            assert!((cost(&found[0]) - best).abs() <= 1e-9 * best, "the first is least-cost");
            for links in &found {
                let ids: Vec<LinkId> = links.iter().map(|&l| LinkId::new(l)).collect();
                for pair in ids.windows(2) {
                    assert_eq!(graph.link_to(pair[0]), graph.link_from(pair[1]), "connected");
                }
                let mut nodes: Vec<NodeId> = std::iter::once(graph.link_from(ids[0]))
                    .chain(ids.iter().map(|&l| graph.link_to(l)))
                    .collect();
                let visited = nodes.len();
                nodes.sort_unstable();
                nodes.dedup();
                assert_eq!(nodes.len(), visited, "loop-free");
                assert!(cost(links) <= detour * best * (1.0 + 1e-9), "within the detour");
            }
        }
        assert!(several >= 1, "at least one bike pair has two routes or more");
    }

    #[test]
    fn extras_move_the_sets_off_a_costly_row() {
        let road = grid();
        let layers = layers(&road);
        // Along row 2, corner to corner: the shortest route takes the row's four links.
        let trips = trips(&road, &[((2, 0), (2, 4), Mode::Bike)]);
        let setup = layers.bike.as_ref().expect("a bike layer");
        let graph = setup.network.network();
        let row: Vec<NodeId> = (0..5).map(|c| node(graph, 2, c)).collect();
        let on_row = |l: u32| {
            let l = LinkId::new(l);
            row.contains(&graph.link_from(l)) && row.contains(&graph.link_to(l))
        };
        let extra: Vec<f64> =
            (0..graph.link_count()).map(|l| if on_row(l) { 300.0 } else { 0.0 }).collect();
        // Draw 0 only, no noise; a detour of six blocks against four needs `max_detour` 2.
        let mut generator = Sampled::default();
        generator.draws = 0;
        generator.max_detour = 2.0;
        let avoids = |r: &StaticRoutes| {
            routes(r, TripId::new(0)).iter().any(|links| !links.iter().any(|&l| on_row(l)))
        };
        let shortest = build(&layers, &trips, &BikeRouting::Shortest);
        let with_extras =
            build(&layers, &trips, &sampled(generator.clone(), vec![vec![Some(extra.into())]]));
        let time_alone = build(&layers, &trips, &sampled(generator, vec![vec![None]]));
        assert!(!avoids(&shortest), "the shortest route runs along the row");
        assert!(!avoids(&time_alone), "without extras the set is the shortest route alone");
        assert!(avoids(&with_extras), "the extras add a route off the row");
        assert_eq!(routes(&with_extras, TripId::new(0))[0], routes(&shortest, TripId::new(0))[0]);
    }

    #[test]
    fn sampled_sets_are_deterministic_and_do_not_depend_on_the_trips_order() {
        let road = grid();
        let layers = layers(&road);
        let rows = mixed_rows();
        let routing = sampled(four_draws(), vec![vec![None; 5]]);
        let trips_a = trips(&road, &rows);
        let a = build(&layers, &trips_a, &routing);
        let again = build(&layers, &trips_a, &routing);
        assert_eq!(a.sets(StaticLayer::Bike), again.sets(StaticLayer::Bike));
        // The same trips in another order.
        let mut shuffled = rows.clone();
        shuffled.reverse();
        shuffled.rotate_left(3);
        assert_ne!(shuffled, rows);
        let trips_b = trips(&road, &shuffled);
        let b = build(&layers, &trips_b, &routing);
        assert_eq!(a.sets(StaticLayer::Bike), b.sets(StaticLayer::Bike));
        // Trip by trip, mapped back by its ends: the same routes.
        let by_pair = |r: &StaticRoutes, trips: &Trips| -> BTreeMap<[u64; 4], Vec<Vec<u32>>> {
            bike_trips(trips)
                .map(|t| {
                    let (o, d) = (trips.origin(t), trips.destination(t));
                    (
                        [o.lon.to_bits(), o.lat.to_bits(), d.lon.to_bits(), d.lat.to_bits()],
                        routes(r, t),
                    )
                })
                .collect()
        };
        let pairs = by_pair(&a, &trips_a);
        assert!(pairs.values().any(|r| r.len() >= 2), "sets to compare");
        assert_eq!(pairs, by_pair(&b, &trips_b));
    }

    /// The sampled build and a trip whose pair has two routes or more.
    fn with_a_set() -> (StaticRoutes, TripId) {
        let road = grid();
        let layers = layers(&road);
        let trips = trips(&road, &mixed_rows());
        let r = build(&layers, &trips, &sampled(four_draws(), vec![vec![None; 5]]));
        let trip = bike_trips(&trips)
            .find(|&t| r.routes_of(t, StaticLayer::Bike).is_some_and(|range| range.len() >= 2))
            .expect("a pair with a set");
        (r, trip)
    }

    #[test]
    fn set_route_records_a_route_of_the_trip_s_set() {
        let (mut r, trip) = with_a_set();
        let range = r.routes_of(trip, StaticLayer::Bike).expect("routed");
        let second = u32::try_from(range.start + 1).expect("fits");
        r.set_route(trip, StaticLayer::Bike, second);
        assert_eq!(r.of(trip, StaticLayer::Bike), StaticRoute::Route(second));
        assert_eq!(r.routes_of(trip, StaticLayer::Bike), Some(range), "the same set");
    }

    #[test]
    #[should_panic(expected = "is not one of the trip's routes")]
    fn set_route_refuses_a_route_outside_the_trip_s_set() {
        let (mut r, trip) = with_a_set();
        let range = r.routes_of(trip, StaticLayer::Bike).expect("routed");
        r.set_route(trip, StaticLayer::Bike, u32::try_from(range.end).expect("fits"));
    }
}
