//! The bike and walk layers: static-cost networks over the road network's
//! geometry (design §20–§21; S193, S195).
//!
//! A layer is a traversable service context over one network (design §20).
//! Cars are searched every iteration on the [`RoadNetwork`] itself; **bikes and
//! pedestrians have static costs** (§21.1), so each gets a [`StaticNetwork`]:
//! its own directed graph — a bike may ride against a one-way street where
//! `oneway:bicycle=no` allows it, a pedestrian walks both ways along every
//! street — with a travel speed per link. The graph type is [`RoadNetwork`],
//! reused: nodes carry the same external ids (OSM node ids) as the road network's
//! and are projected in the same projection, so a hub can join layers by node.
//! **The motor-traffic parameters of a static network's links are never read**,
//! as for footways in the road network.
//!
//! # The minimal bike layer (S193)
//!
//! Every car link except motorways is also a bike link; dedicated bike
//! infrastructure ([`BikeInfrastructure::Lane`], [`BikeInfrastructure::Separated`])
//! is faster and, under the [`BikeCost::Dedicated`] cost, preferred. Routes are
//! shortest paths under one of the two [`BikeCost`]s. Bikeability link types with
//! their own utility weights, and stochastic bike assignment, come later.
//!
//! # Networks without OSM tags
//!
//! [`StaticNetwork::derive`] makes both layers from a road network alone — the
//! synthetic grid, the toy network, a network read from a link table — by the
//! fallback rung of design §21.4: bikes on every road link but motorways, in
//! the road's direction; pedestrians on every link but motorways and cycleways,
//! both ways.
//!
//! **Cost:** a static network is a [`RoadNetwork`] plus 9 bytes per link (a
//! speed and an infrastructure level); the costs a search reads are a vector of
//! 8 bytes per link, made when asked for.

use std::sync::Arc;

use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::{EntityId, LinkId, NodeId};

use crate::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
use crate::geometry::ProjectionError;
use crate::network::{LinkSpec, RoadNetwork, RoadNetworkBuilder};

/// Every layer a traveller can be on (design §20; S198): where a hub's access
/// point is, and which network a leg of a trip runs on.
///
/// The discriminant is the layer's id and part of what hubs store: append new
/// layers, never renumber.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
#[repr(u8)]
pub enum Layer {
    /// Cars, on the road network.
    Car = 0,
    /// Personal bikes, on the bike layer.
    Bike = 1,
    /// Walking, on the walk layer: the universal access layer (design §21.2).
    Walk = 2,
    /// Scheduled public transport (M3): a stop's platform.
    Transit = 3,
}

impl Layer {
    /// Every layer, in id order.
    pub const ALL: [Layer; 4] = [Layer::Car, Layer::Bike, Layer::Walk, Layer::Transit];

    /// The stable snake_case name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Layer::Car => "car",
            Layer::Bike => "bike",
            Layer::Walk => "walk",
            Layer::Transit => "transit",
        }
    }

    /// The static layer this is, if it is one.
    #[must_use]
    pub const fn as_static(self) -> Option<StaticLayer> {
        match self {
            Layer::Bike => Some(StaticLayer::Bike),
            Layer::Walk => Some(StaticLayer::Walk),
            Layer::Car | Layer::Transit => None,
        }
    }
}

impl From<StaticLayer> for Layer {
    fn from(layer: StaticLayer) -> Self {
        match layer {
            StaticLayer::Bike => Layer::Bike,
            StaticLayer::Walk => Layer::Walk,
        }
    }
}

/// The layers whose costs are static (design §21.1).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum StaticLayer {
    /// Personal bikes.
    Bike,
    /// Walking.
    Walk,
}

impl StaticLayer {
    /// The stable snake_case name, for results files and diagnostics.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            StaticLayer::Bike => "bike",
            StaticLayer::Walk => "walk",
        }
    }
}

/// What a bike link offers a cyclist, from the tags (S193).
///
/// Recorded at three levels because OSM distinguishes them anyway, and later
/// bikeability link types would otherwise need a re-import; the minimal cost
/// treats [`Lane`](Self::Lane) and [`Separated`](Self::Separated) alike, as
/// **dedicated** infrastructure.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
#[repr(u8)]
pub enum BikeInfrastructure {
    /// Riding in mixed traffic, or on a path shared with pedestrians.
    #[default]
    Mixed = 0,
    /// A painted lane on the carriageway (`cycleway=lane`), or a street where
    /// bikes have priority (`cyclestreet`, `bicycle_road`).
    Lane = 1,
    /// A track or path of its own (`highway=cycleway`, `cycleway=track`, a path
    /// with `bicycle=designated`).
    Separated = 2,
}

impl BikeInfrastructure {
    /// Whether the minimal cost treats this as dedicated infrastructure.
    #[inline]
    #[must_use]
    pub const fn is_dedicated(self) -> bool {
        !matches!(self, BikeInfrastructure::Mixed)
    }

    /// The stable snake_case name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            BikeInfrastructure::Mixed => "mixed",
            BikeInfrastructure::Lane => "lane",
            BikeInfrastructure::Separated => "separated",
        }
    }
}

/// How a bike route search costs a link (S193; D4, S195).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum BikeCost {
    /// Travel time only: dedicated infrastructure attracts routes only through
    /// its higher speed.
    Time,
    /// Travel time, with [`StaticLayerDefaults::bike_mixed_cost_factor`] on every
    /// link without dedicated infrastructure: cyclists accept a detour to ride on
    /// it. **The default** (D4).
    #[default]
    Dedicated,
}

impl BikeCost {
    /// The stable snake_case name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            BikeCost::Time => "time",
            BikeCost::Dedicated => "dedicated",
        }
    }

    /// The cost with this name, if there is one.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "time" => Some(BikeCost::Time),
            "dedicated" => Some(BikeCost::Dedicated),
            _ => None,
        }
    }
}

/// The defaults of the static layers: speeds and the dedicated-infrastructure
/// preference (S193). Part of the defaults table; a change is a modelling change
/// and bumps [`crate::DEFAULTS_VERSION`].
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct StaticLayerDefaults {
    /// A bike's speed in mixed traffic, stops included, in km/h.
    ///
    /// About 15 km/h is the urban average in the Netherlands, stop time
    /// included (de Hartog, Boogaard, Nijland and Hoek, *Environmental Health
    /// Perspectives*, 2011, the authors' response in the correspondence on
    /// their 2010 paper).
    pub bike_mixed_km_h: f64,
    /// A bike's speed on dedicated infrastructure, in km/h.
    ///
    /// *CITATION OWED: set 20 % above the mixed-traffic speed for fewer stops
    /// and conflicts; to be replaced by a measured value (for example from
    /// GPS studies of speed by facility type).*
    pub bike_dedicated_km_h: f64,
    /// The multiplier [`BikeCost::Dedicated`] puts on the travel time of a link
    /// without dedicated infrastructure.
    ///
    /// *CITATION OWED: the direction is well established — cyclists detour for
    /// off-street paths and protected facilities (Broach, Dill and Gliebe,
    /// *Transportation Research Part A* 46(10), 2012; for Amsterdam, Ton, Cats,
    /// Duives and Hoogendoorn, *Transportation Research Record* 2662, 2017) —
    /// the value is a placeholder until one is taken from such a model.*
    pub bike_mixed_cost_factor: f64,
    /// The multiplier [`BikeCost::Dedicated`] puts on the travel time of a link with a
    /// painted lane (or a cycle street), against 1 on a separated track (S236, roadmap I-bb:
    /// lane against track).
    ///
    /// *Uncalibrated: 1, a lane counted as a track, as before S236. CITATION OWED: route
    /// choice models find cyclists prefer separated tracks to painted lanes (Broach, Dill and
    /// Gliebe 2012, above); a value between 1 and [`Self::bike_mixed_cost_factor`] is to be
    /// taken from such a model.*
    pub bike_lane_cost_factor: f64,
    /// Walking speed, in km/h: also a bike's speed where the tags say to
    /// dismount.
    ///
    /// 4.8 km/h (1.33 m/s), within the range of measured mean walking speeds
    /// (Knoblauch, Pietrucha and Nitzburg, *Transportation Research Record*
    /// 1538, 1996).
    pub walk_km_h: f64,
    /// A ferry's speed when its way does not say how long the crossing takes
    /// (no `duration` tag), in km/h (S197).
    ///
    /// *An assumption: small urban passenger ferries cross at roughly this
    /// speed, docking included.*
    pub ferry_km_h: f64,
    /// The expected wait for a ferry, in seconds, added to every crossing
    /// (S197): the static stand-in for a timetable until transit (M3) has the
    /// ferry's own.
    ///
    /// *An assumption: half of a 10-minute headway. The free IJ ferries in
    /// Amsterdam run more often at peak; most others less often.*
    pub ferry_wait_s: f64,
}

impl StaticLayerDefaults {
    /// The shipped values.
    pub const SHIPPED: StaticLayerDefaults = StaticLayerDefaults {
        bike_mixed_km_h: 15.0,
        bike_dedicated_km_h: 18.0,
        bike_mixed_cost_factor: 1.2,
        bike_lane_cost_factor: 1.0,
        walk_km_h: 4.8,
        ferry_km_h: 10.0,
        ferry_wait_s: 300.0,
    };

    /// The speed of a ferry link of a way `length_m` long, in km/h, such that
    /// crossing the whole way takes the expected wait plus the crossing:
    /// `duration_s` if the way states it, else the way at
    /// [`Self::ferry_km_h`]. A static layer's links have a speed and nothing
    /// else, so the wait is folded into it (S197).
    #[must_use]
    pub fn ferry_speed_km_h(self, length_m: f64, duration_s: Option<f64>) -> f64 {
        let crossing = duration_s
            .filter(|d| d.is_finite() && *d > 0.0)
            .unwrap_or(length_m / (self.ferry_km_h / 3.6));
        let seconds = (self.ferry_wait_s + crossing).max(1.0);
        length_m.max(1.0) / seconds * 3.6
    }

    /// A bike's speed on a link, in km/h: by its infrastructure, walking speed
    /// if it must be walked, and never above the link's speed limit.
    #[must_use]
    pub fn bike_speed_km_h(
        self,
        infrastructure: BikeInfrastructure,
        dismount: bool,
        limit_km_h: Option<f64>,
    ) -> f64 {
        let base = if dismount {
            self.walk_km_h
        } else if infrastructure.is_dedicated() {
            self.bike_dedicated_km_h
        } else {
            self.bike_mixed_km_h
        };
        match limit_km_h {
            Some(limit) if limit.is_finite() && limit > 0.0 => base.min(limit),
            _ => base,
        }
    }
}

impl Default for StaticLayerDefaults {
    fn default() -> Self {
        Self::SHIPPED
    }
}

/// What a static layer knows about one link beyond its geometry.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct StaticLink {
    /// The OSM road class, kept for reporting and figures.
    pub class: RoadClass,
    /// The travel speed in km/h, final: every rule of the layer already applied.
    pub speed_km_h: f64,
    /// The bike infrastructure ([`BikeInfrastructure::Mixed`] on the walk layer).
    pub infrastructure: BikeInfrastructure,
    /// Length in metres, measured along the geometry.
    pub length_m: Option<f64>,
}

/// Builds a [`StaticNetwork`] from external ids, like [`RoadNetworkBuilder`].
#[derive(Debug)]
pub struct StaticNetworkBuilder {
    layer: StaticLayer,
    graph: RoadNetworkBuilder,
    links: Vec<(String, StaticLink)>,
}

impl StaticNetworkBuilder {
    /// An empty builder for `layer`.
    #[must_use]
    pub fn new(layer: StaticLayer) -> Self {
        Self { layer, graph: RoadNetworkBuilder::new(), links: Vec::new() }
    }

    /// Declare a node at a WGS84 coordinate; see [`RoadNetworkBuilder::add_node`].
    pub fn add_node(&mut self, external_id: impl Into<String>, at: crate::geometry::LonLat) {
        self.graph.add_node(external_id, at);
    }

    /// Declare a directed link.
    pub fn add_link(
        &mut self,
        external_id: impl Into<String>,
        from: impl Into<String>,
        to: impl Into<String>,
        link: StaticLink,
    ) {
        let external_id = external_id.into();
        let spec = LinkSpec { length_m: link.length_m, ..LinkSpec::new(link.class) };
        self.graph.add_link(external_id.clone(), from, to, spec);
        self.links.push((external_id, link));
    }

    /// Assign ids and freeze, projecting in `projection` — the road network's,
    /// so that every layer shares one coordinate system.
    ///
    /// The graph's diagnostics (a degenerate link length, a link to an unknown
    /// node) are recorded in `diagnostics`.
    ///
    /// # Errors
    ///
    /// [`ProjectionError`] as for [`RoadNetworkBuilder::build`].
    pub fn build(
        self,
        projection: crate::geometry::Projection,
        diagnostics: &mut Diagnostics,
    ) -> Result<StaticNetwork, ProjectionError> {
        let network = self.graph.with_projection(projection).build(
            GlobalMultipliers::default(),
            SignalDefaults::SHIPPED,
            diagnostics,
        )?;
        let n = network.link_count() as usize;
        let mut speed = vec![0.0f64; n];
        let mut infrastructure = vec![BikeInfrastructure::Mixed; n];
        let ids = network.link_external_ids();
        for (external, link) in &self.links {
            if let Some(id) = ids.id_of(external) {
                speed[id as usize] = link.speed_km_h / 3.6;
                infrastructure[id as usize] = link.infrastructure;
            }
        }
        Ok(StaticNetwork { layer: self.layer, network: Arc::new(network), speed, infrastructure })
    }
}

/// A bike or walk layer: a directed graph and a speed per link (see the
/// [module docs](self)).
#[derive(Debug)]
pub struct StaticNetwork {
    layer: StaticLayer,
    network: Arc<RoadNetwork>,
    /// Metres per second, per link.
    speed: Vec<f64>,
    infrastructure: Vec<BikeInfrastructure>,
}

impl StaticNetwork {
    /// Both static layers made from a road network alone (design §21.4's
    /// fallback rung): bikes on every link but motorways, in the road's
    /// direction, [`BikeInfrastructure::Separated`] on cycleways and mixed
    /// elsewhere, never above the road's free-flow speed; pedestrians on every
    /// link but motorways and cycleways, in both directions.
    ///
    /// For networks with no OSM tags to say more: the grid, the toy network, a
    /// link table. An OSM import builds its layers from the tags instead.
    ///
    /// # Errors
    ///
    /// [`ProjectionError`] only if the road network's own nodes cannot be
    /// projected, which a built road network rules out.
    ///
    /// # Panics
    ///
    /// Never in practice: node indices come from the road network, which fits
    /// its ids in `u32`.
    pub fn derive(
        road: &RoadNetwork,
        layer: StaticLayer,
        defaults: StaticLayerDefaults,
        diagnostics: &mut Diagnostics,
    ) -> Result<Self, ProjectionError> {
        let mut builder = StaticNetworkBuilder::new(layer);
        let node_ids = road.node_external_ids();
        let link_ids = road.link_external_ids();
        let mut used = vec![false; road.node_count() as usize];
        for i in 0..road.link_count() {
            let link = LinkId::new(i);
            let class = road.link_class(link);
            let from = road.link_from(link);
            let to = road.link_to(link);
            let length_m = Some(road.link_length(link).get());
            let external = link_ids.external(i);
            match layer {
                StaticLayer::Bike => {
                    if !carries_bikes_by_class(class) {
                        continue;
                    }
                    let infrastructure = if class == RoadClass::Cycleway {
                        BikeInfrastructure::Separated
                    } else {
                        BikeInfrastructure::Mixed
                    };
                    let limit = class
                        .carries_motor_traffic()
                        .then(|| road.link_parameters(link).free_flow_speed.as_km_per_hour());
                    let speed_km_h = defaults.bike_speed_km_h(infrastructure, false, limit);
                    builder.add_link(
                        external,
                        node_ids.external(from.raw()),
                        node_ids.external(to.raw()),
                        StaticLink { class, speed_km_h, infrastructure, length_m },
                    );
                }
                StaticLayer::Walk => {
                    if !carries_pedestrians_by_class(class) {
                        continue;
                    }
                    let walk = StaticLink {
                        class,
                        speed_km_h: defaults.walk_km_h,
                        infrastructure: BikeInfrastructure::Mixed,
                        length_m,
                    };
                    builder.add_link(
                        external,
                        node_ids.external(from.raw()),
                        node_ids.external(to.raw()),
                        walk,
                    );
                    // A one-way street is walked both ways; a two-way street
                    // already has its reverse link.
                    let has_reverse = road
                        .out_links(to)
                        .iter()
                        .any(|&back| road.is_reverse_of(link, back) && walkable(road, back));
                    if !has_reverse {
                        builder.add_link(
                            format!("{external}:r"),
                            node_ids.external(to.raw()),
                            node_ids.external(from.raw()),
                            walk,
                        );
                    }
                }
            }
            used[from.index()] = true;
            used[to.index()] = true;
        }
        for (i, &u) in used.iter().enumerate() {
            if u {
                let id = u32::try_from(i).expect("node ids fit u32");
                builder.add_node(node_ids.external(id), road.node_lonlat(NodeId::new(id)));
            }
        }
        builder.build(road.projection(), diagnostics)
    }

    /// The same layer with `changes` made to its links' bike infrastructure (S238, scenario
    /// edits): a street given a painted lane or a separated track, or one taken away. A changed
    /// link rides at its new infrastructure's speed under `defaults` (`bike_dedicated_km_h` for
    /// a lane or a track, `bike_mixed_km_h` in mixed traffic); its costs and the attributes a
    /// choice model sees follow. The graph is shared, so link indices stay as they were.
    ///
    /// # Errors
    ///
    /// A message for a layer that is not the bike layer, a link index out of range, or a ferry
    /// link (a crossing has no bike infrastructure).
    pub fn with_infrastructure(
        &self,
        changes: &[(LinkId, BikeInfrastructure)],
        defaults: StaticLayerDefaults,
    ) -> Result<Self, String> {
        if self.layer != StaticLayer::Bike {
            return Err("bike infrastructure is changed on the bike layer".to_string());
        }
        let n = self.speed.len();
        let mut speed = self.speed.clone();
        let mut infrastructure = self.infrastructure.clone();
        for &(link, kind) in changes {
            let i = link.index();
            if i >= n {
                return Err(format!("there is no link {i} on a bike layer of {n} links"));
            }
            if self.network.link_class(link) == RoadClass::Ferry {
                return Err(format!("bike link {i} is a ferry crossing; it has no infrastructure"));
            }
            infrastructure[i] = kind;
            speed[i] = defaults.bike_speed_km_h(kind, false, None) / 3.6;
        }
        Ok(Self { layer: self.layer, network: Arc::clone(&self.network), speed, infrastructure })
    }

    /// Which layer this is.
    #[must_use]
    pub fn layer(&self) -> StaticLayer {
        self.layer
    }

    /// The graph.
    #[must_use]
    pub fn network(&self) -> &RoadNetwork {
        &self.network
    }

    /// The graph, shared: for a handle that outlives this borrow.
    #[must_use]
    pub fn network_arc(&self) -> Arc<RoadNetwork> {
        Arc::clone(&self.network)
    }

    /// A link's travel speed, in metres per second.
    #[inline]
    #[must_use]
    pub fn speed(&self, link: LinkId) -> f64 {
        self.speed[link.index()]
    }

    /// A link's bike infrastructure.
    #[inline]
    #[must_use]
    pub fn infrastructure(&self, link: LinkId) -> BikeInfrastructure {
        self.infrastructure[link.index()]
    }

    /// Every link's bike infrastructure, in link order.
    #[must_use]
    pub fn infrastructures(&self) -> &[BikeInfrastructure] {
        &self.infrastructure
    }

    /// Every link's travel time in seconds, in link order.
    #[must_use]
    pub fn link_seconds(&self) -> Vec<f64> {
        (0..self.network.link_count())
            .map(|i| {
                let link = LinkId::new(i);
                self.network.link_length(link).get() / self.speed(link)
            })
            .collect()
    }

    /// Every link's cost for a route search, in link order: its travel time,
    /// times `mixed_factor` where [`BikeCost::Dedicated`] applies to a link
    /// without dedicated infrastructure (a ferry is not mixed traffic, S197), and
    /// times `lane_factor` on a painted lane (S236; 1 counts a lane as a track).
    /// On the walk layer the cost is the time.
    #[must_use]
    pub fn link_costs(&self, cost: BikeCost, lane_factor: f64, mixed_factor: f64) -> Vec<f64> {
        let mut seconds = self.link_seconds();
        if self.layer == StaticLayer::Bike && cost == BikeCost::Dedicated {
            for (i, (s, infrastructure)) in seconds.iter_mut().zip(&self.infrastructure).enumerate()
            {
                let ferry = self.network.link_class(LinkId::from_index(i)) == RoadClass::Ferry;
                match infrastructure {
                    BikeInfrastructure::Mixed if !ferry => *s *= mixed_factor,
                    // Off means absent: at 1 the cost is the time, bit for bit.
                    #[allow(clippy::float_cmp, reason = "1 exactly leaves the time untouched")]
                    BikeInfrastructure::Lane if lane_factor != 1.0 => *s *= lane_factor,
                    _ => {}
                }
            }
        }
        seconds
    }

    /// Bytes held, graph included.
    #[must_use]
    pub fn bytes(&self) -> usize {
        self.network.bytes()
            + self.speed.len() * core::mem::size_of::<f64>()
            + self.infrastructure.len()
    }
}

/// Whether a road link of `class` is a bike link when no tags say more.
fn carries_bikes_by_class(class: RoadClass) -> bool {
    !matches!(
        class,
        RoadClass::Motorway
            | RoadClass::MotorwayLink
            | RoadClass::Footway
            | RoadClass::Pedestrian
            | RoadClass::Busway
    )
}

/// Whether a road link of `class` is a walk link when no tags say more.
fn carries_pedestrians_by_class(class: RoadClass) -> bool {
    class.carries_pedestrians()
}

fn walkable(road: &RoadNetwork, link: LinkId) -> bool {
    carries_pedestrians_by_class(road.link_class(link))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp, reason = "the speeds are copied from the table, not computed")]

    use super::*;

    #[test]
    fn bike_speed_follows_infrastructure_dismount_and_limit() {
        let d = StaticLayerDefaults::SHIPPED;
        assert_eq!(d.bike_speed_km_h(BikeInfrastructure::Mixed, false, None), 15.0);
        assert_eq!(d.bike_speed_km_h(BikeInfrastructure::Lane, false, None), 18.0);
        assert_eq!(d.bike_speed_km_h(BikeInfrastructure::Separated, false, None), 18.0);
        assert_eq!(d.bike_speed_km_h(BikeInfrastructure::Separated, true, None), 4.8);
        // A living street's walking-pace limit binds; a 50 km/h limit does not.
        assert_eq!(d.bike_speed_km_h(BikeInfrastructure::Mixed, false, Some(7.0)), 7.0);
        assert_eq!(d.bike_speed_km_h(BikeInfrastructure::Lane, false, Some(50.0)), 18.0);
    }

    #[test]
    fn a_ferry_link_takes_its_wait_and_its_crossing() {
        let d = StaticLayerDefaults::SHIPPED;
        // 500 m at 10 km/h is 180 s; with the 300 s wait, 480 s: 3.75 km/h.
        assert!((d.ferry_speed_km_h(500.0, None) - 3.75).abs() < 1e-12);
        // A stated 4-minute crossing: 540 s for 500 m.
        assert!((d.ferry_speed_km_h(500.0, Some(240.0)) - 500.0 / 540.0 * 3.6).abs() < 1e-12);
    }

    #[test]
    fn cost_names_round_trip_and_dedicated_is_the_default() {
        for cost in [BikeCost::Time, BikeCost::Dedicated] {
            assert_eq!(BikeCost::from_name(cost.as_str()), Some(cost));
        }
        assert_eq!(BikeCost::from_name("fastest"), None);
        assert_eq!(BikeCost::default(), BikeCost::Dedicated);
    }

    #[test]
    fn only_mixed_is_not_dedicated() {
        assert!(!BikeInfrastructure::Mixed.is_dedicated());
        assert!(BikeInfrastructure::Lane.is_dedicated());
        assert!(BikeInfrastructure::Separated.is_dedicated());
    }
}
