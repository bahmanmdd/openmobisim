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
use openmobisim_core_routes::generate::Shortest;
use openmobisim_core_routes::search::SearchContext;
use openmobisim_core_routes::{NodeSnapper, RouteKey, RouteSets};
use openmobisim_core_types::ids::{EntityId, LinkId, TripId};

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

/// Every bike and walk trip's route, made once per run.
#[derive(Debug, Default)]
pub(crate) struct StaticRoutes {
    /// The bike layer's route sets, then the walk layer's.
    sets: [Option<RouteSets>; 2],
    route: Vec<StaticRoute>,
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
    /// Route every trip whose mode has a static layer the run has: snap its
    /// ends to the layer's nodes and take the shortest route between them.
    pub(crate) fn build(layers: &StaticLayers, trips: &Trips) -> Self {
        let total = trips.len() as usize;
        let mut route = vec![StaticRoute::None; total];
        let mut sets: [Option<RouteSets>; 2] = [None, None];
        for layer in [StaticLayer::Bike, StaticLayer::Walk] {
            let Some(setup) = layers.get(layer) else { continue };
            let wanted: Vec<usize> = (0..total)
                .filter(|&i| layer_of(trips.mode(TripId::from_index(i))) == Some(layer))
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
            let layer_sets = RouteSets::generate_in(&ctx, &keys, &Shortest);
            for (&i, key) in wanted.iter().zip(&keys) {
                route[i] = if key.origin == key.destination {
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

    /// The trip's route on its layer.
    pub(crate) fn of(&self, trip: TripId) -> StaticRoute {
        self.route.get(trip.index()).copied().unwrap_or(StaticRoute::None)
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
