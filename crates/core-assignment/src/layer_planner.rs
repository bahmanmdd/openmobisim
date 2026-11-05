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

use openmobisim_core_demand::Mode;
use openmobisim_core_graph::defaults::RoadClass;
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::layers::{
    BikeCost, BikeInfrastructure, StaticLayer, StaticLayerDefaults, StaticNetwork,
};
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_routes::NodeSnapper;
use openmobisim_core_routes::search::{Search, SearchContext};
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
    /// around the ride.
    ///
    /// # Errors
    ///
    /// [`AssignError::Input`] if `layer` is not a bike layer or a fixed time is not a finite
    /// number of at least 0; [`AssignError::Coefficients`] if, under [`BikeCost::Utility`], a
    /// class's cost of a minute in a vehicle is not a positive finite number or makes a link
    /// cost less than nothing.
    pub fn bike(
        layer: Layer<'a>,
        classes: &[Coefficients],
        cost: BikeCost,
        defaults: StaticLayerDefaults,
        fixed: FixedTimes,
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
        let costs = |c: usize, k: &Coefficients| -> Result<Vec<f64>, AssignError> {
            match cost {
                BikeCost::Time => Ok(seconds.clone()),
                BikeCost::Dedicated => Ok(network.link_costs(
                    BikeCost::Dedicated,
                    defaults.bike_lane_cost_factor,
                    defaults.bike_mixed_cost_factor,
                )),
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
                    let each = |l: usize| {
                        let mut sum = k.per_kilometre * km[l]
                            + k.bike_mixed_per_kilometre * mixed_metres[l] / 1000.0;
                        // A facility's term only where the class weighs it, so a class that
                        // weighs neither searches with the costs it had without them, to the bit.
                        if k.bike_lane_per_kilometre != 0.0 {
                            sum += k.bike_lane_per_kilometre * lane_metres[l] / 1000.0;
                        }
                        if k.bike_separated_per_kilometre != 0.0 {
                            sum += k.bike_separated_per_kilometre * separated_metres[l] / 1000.0;
                        }
                        seconds[l] + sum / per_second
                    };
                    checked(c, (0..seconds.len()).map(each).collect(), &seconds)
                }
            }
        };
        let metres = [mixed_metres.clone(), lane_metres.clone(), separated_metres.clone()];
        Self::new(Mode::Bike, layer, classes, seconds.clone(), metres, fixed, costs)
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
