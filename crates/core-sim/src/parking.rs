//! Parking in a run (M4, S201): the parkings of a scenario as hubs on the
//! layers, how full they are over the day, and what that costs a traveller.
//!
//! **Parkings are hubs** (design §3.2): each row of a parking table
//! ([`ParkingRow`]) is snapped to its vehicle's layer (the roads for a car, the
//! bike layer for a bike) and to the walk layer, and becomes a hub with a
//! [`Parking`] resource; rows sharing a `hub_id` share a hub. **A parking need
//! not be a stop** (A7): the traveller walks from it to the stops within
//! [`ParkingDefaults::walk_max_s`], found here once. A parking with no stop
//! within that walk is left out, since in this version only park-and-ride and
//! bike-and-ride use parkings (counted in [`ParkingSetupReport`]).
//!
//! **Soft capacity** (design §17): the time to park is
//! `floor + slope × (1 − availability)`, the availability being the share of a
//! time bin in which the parking had a free space. Nothing is refused: above
//! capacity the stock simply exceeds it, and the overflow is counted. **Choice
//! reads the expected availability**, the average over the iterations so far
//! (the MSA average of the realised ones; empty at the start); **a traveller
//! pays the realised one**. The difference, per traveller who parked, is the hub
//! expectation mismatch of design §11.2.
//!
//! **Cost:** per parking a few dozen bytes and its stops; per time bin 8 bytes
//! of expected availability; per vehicle parked one event.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, OnceLock};

use openmobisim_core_graph::geometry::{LonLat, ground_distance_metres};
use openmobisim_core_graph::hubs::{
    AccessPoint, HubKind, HubSet, HubSpec, Parking, ParkingKind, ParkingRow,
};
use openmobisim_core_graph::layers::Layer;
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_routes::NodeSnapper;
use openmobisim_core_types::ids::{EntityId, NodeId, ResourceId};

use crate::layers::LayerSetup;
use crate::transit::TransitSetup;

/// Named options, as a run's `parking_options`.
pub type Options = std::collections::BTreeMap<String, f64>;

/// Parking's defaults: every one an uncalibrated assumption, overridable by
/// name (S202), part of the defaults table (`DEFAULTS_VERSION` 5; `pr_min_km` 6).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParkingDefaults {
    /// The longest walk from a parking to a stop, in seconds (300 s is 400 m).
    ///
    /// *An assumption, the same as the longest walk between stops.*
    pub walk_max_s: f64,
    /// The longest drive to a park-and-ride, in seconds.
    ///
    /// *An assumption. CITATION OWED: park-and-ride catchments are mostly within
    /// 20–30 minutes' drive.*
    pub reach_car_s: f64,
    /// The longest ride to a bike parking, in seconds of the bike layer's cost.
    ///
    /// *An assumption. CITATION OWED: most Dutch bike-and-ride access trips are
    /// under 5 km.*
    pub reach_bike_s: f64,
    /// The most parkings one trip chooses among (`max_anchor_candidates`, V9);
    /// twice as many are tried. Three (`DEFAULTS_VERSION` 19; five before): on Amsterdam's
    /// morning, 12 % less run time for a fifth less bike-and-ride.
    ///
    /// *A resource limit, and an assumption about how many stations a traveller
    /// weighs.*
    pub candidates: f64,
    /// The speed a candidate's remaining distance to the destination is divided
    /// by to rank candidates before they are tried, in km/h.
    ///
    /// *A ranking device, not behaviour: about a tram's or a train's average
    /// speed.*
    pub rank_speed_km_h: f64,
    /// The time to park a car and walk out of the car park when there is room,
    /// in seconds; also the time to fetch it.
    ///
    /// *An assumption. CITATION OWED.*
    pub floor_car_s: f64,
    /// The time a full car park adds, in seconds, at availability 0 (linear in
    /// the unavailability, design §17.2).
    ///
    /// *An assumption. CITATION OWED: searching for a space.*
    pub slope_car_s: f64,
    /// The time to park a bike and walk out when there is room, in seconds; also
    /// the time to fetch it.
    ///
    /// *An assumption. CITATION OWED.*
    pub floor_bike_s: f64,
    /// The time a full bike parking adds, in seconds, at availability 0.
    ///
    /// *An assumption. CITATION OWED.*
    pub slope_bike_s: f64,
    /// The farthest a parking may be from its layers' nearest node, in metres.
    ///
    /// *An assumption, as for a stop.*
    pub snap_m: f64,
    /// The length of the time bins availability is measured in, in seconds.
    pub bin_s: f64,
    /// The shortest trip, in km as the crow flies, that mode choice offers park-and-ride
    /// and bike-and-ride to (M5; a trip given one of those modes takes it at any length).
    ///
    /// *For compute (A18, roadmap I-ag), and an assumption: nobody drives to a car park
    /// for a 1 km trip. Uncalibrated.*
    pub pr_min_km: f64,
}

impl ParkingDefaults {
    /// The shipped values.
    pub const SHIPPED: ParkingDefaults = ParkingDefaults {
        walk_max_s: 300.0,
        reach_car_s: 1800.0,
        reach_bike_s: 1200.0,
        candidates: 3.0,
        rank_speed_km_h: 30.0,
        floor_car_s: 120.0,
        slope_car_s: 600.0,
        floor_bike_s: 30.0,
        slope_bike_s: 180.0,
        snap_m: 300.0,
        bin_s: 900.0,
        pr_min_km: 3.0,
    };

    /// The names of the options, in the order [`Self::from_options`] reads them.
    pub const NAMES: [&'static str; 12] = [
        "walk_max_s",
        "reach_car_s",
        "reach_bike_s",
        "candidates",
        "rank_speed_km_h",
        "floor_car_s",
        "slope_car_s",
        "floor_bike_s",
        "slope_bike_s",
        "snap_m",
        "bin_s",
        "pr_min_km",
    ];

    /// Every option's value, by name, in [`Self::NAMES`]' order (S230: the parameter listing).
    #[must_use]
    pub fn values(&self) -> Vec<(&'static str, f64)> {
        let mut copy = *self;
        Self::NAMES.iter().filter_map(|&n| copy.slot(n).map(|v| (n, *v))).collect()
    }

    fn slot(&mut self, name: &str) -> Option<&mut f64> {
        Some(match name {
            "walk_max_s" => &mut self.walk_max_s,
            "reach_car_s" => &mut self.reach_car_s,
            "reach_bike_s" => &mut self.reach_bike_s,
            "candidates" => &mut self.candidates,
            "rank_speed_km_h" => &mut self.rank_speed_km_h,
            "floor_car_s" => &mut self.floor_car_s,
            "slope_car_s" => &mut self.slope_car_s,
            "floor_bike_s" => &mut self.floor_bike_s,
            "slope_bike_s" => &mut self.slope_bike_s,
            "snap_m" => &mut self.snap_m,
            "bin_s" => &mut self.bin_s,
            "pr_min_km" => &mut self.pr_min_km,
            _ => return None,
        })
    }

    /// The shipped values with `options` in place of their namesakes.
    ///
    /// # Errors
    ///
    /// [`ParkingError::UnknownOption`] for a name that is not one of
    /// [`Self::NAMES`], [`ParkingError::BadOption`] for a value that is not a
    /// finite non-negative number (and, for `candidates` and `bin_s`, at least 1).
    pub fn from_options(options: &Options) -> Result<Self, ParkingError> {
        let mut d = Self::SHIPPED;
        for (name, &value) in options {
            let Some(slot) = d.slot(name) else {
                return Err(ParkingError::UnknownOption { option: name.clone() });
            };
            let at_least = if matches!(name.as_str(), "candidates" | "bin_s") { 1.0 } else { 0.0 };
            if !(value.is_finite() && value >= at_least) {
                return Err(ParkingError::BadOption {
                    option: name.clone(),
                    reason: format!("must be a finite number of at least {at_least}, got {value}"),
                });
            }
            *slot = value;
        }
        Ok(d)
    }

    /// The candidate count as a number of parkings.
    #[must_use]
    pub fn candidate_count(&self) -> usize {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a small count")]
        let n = self.candidates.max(1.0) as usize;
        n
    }

    /// The time to park (or fetch) a vehicle of `kind` at `availability` (0 to 1).
    #[must_use]
    pub fn parking_seconds(&self, kind: ParkingKind, availability: f64) -> f64 {
        let (floor, slope) = match kind {
            ParkingKind::Car => (self.floor_car_s, self.slope_car_s),
            ParkingKind::Bike => (self.floor_bike_s, self.slope_bike_s),
        };
        floor + slope * (1.0 - availability.clamp(0.0, 1.0))
    }

    /// The time to fetch a parked vehicle of `kind`: the floor alone.
    #[must_use]
    pub fn fetch_seconds(&self, kind: ParkingKind) -> f64 {
        match kind {
            ParkingKind::Car => self.floor_car_s,
            ParkingKind::Bike => self.floor_bike_s,
        }
    }
}

impl Default for ParkingDefaults {
    fn default() -> Self {
        Self::SHIPPED
    }
}

/// Why parkings could not be set up.
#[derive(Clone, Debug, PartialEq)]
pub enum ParkingError {
    /// No option has this name.
    UnknownOption {
        /// The name given.
        option: String,
    },
    /// An option's value is not allowed.
    BadOption {
        /// The option.
        option: String,
        /// What is wrong with it.
        reason: String,
    },
    /// Two rows of the table share a `parking_id`.
    DuplicateId(String),
}

impl fmt::Display for ParkingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownOption { option } => write!(
                f,
                "parking has no option {option:?}; its options are: {}",
                ParkingDefaults::NAMES.join(", ")
            ),
            Self::BadOption { option, reason } => write!(f, "parking option {option:?}: {reason}"),
            Self::DuplicateId(id) => write!(f, "parking_id {id:?} appears more than once"),
        }
    }
}

impl std::error::Error for ParkingError {}

/// What setting parkings up kept and left out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ParkingSetupReport {
    /// Rows given.
    pub rows: u32,
    /// Kept, car and bike.
    pub kept_car: u32,
    /// Kept bike parkings.
    pub kept_bike: u32,
    /// Left out: farther than [`ParkingDefaults::snap_m`] from its layers, or its
    /// layer is missing.
    pub off_layer: u32,
    /// Left out: no stop within [`ParkingDefaults::walk_max_s`] on foot.
    pub no_stop: u32,
}

/// A scenario's parkings linked to its layers and stops: see the
/// [module docs](self).
#[derive(Debug)]
pub struct ParkingSetup {
    hubs: HubSet,
    defaults: ParkingDefaults,
    road: Arc<RoadNetwork>,
    /// Per parking: its node on its vehicle's layer, its walk node, where it is.
    node: Vec<NodeId>,
    walk_node: Vec<NodeId>,
    position: Vec<LonLat>,
    name: Vec<Option<String>>,
    /// Per parking: its fee per stay in euros, if its row gave one (S248).
    fee_eur: Vec<Option<f64>>,
    /// Per parking: the stops within walking distance and the walk in seconds, CSR.
    stops_start: Vec<u32>,
    stops: Vec<(NodeId, u32)>,
    /// Per kind: which nodes of its layer have a parking, how many such nodes,
    /// and the parkings at each.
    targets: [Vec<bool>; 2],
    target_nodes: [usize; 2],
    at_node: [HashMap<u32, Vec<u32>>; 2],
    report: ParkingSetupReport,
    /// The free-flow drive to every car park, made on first use (S209): see
    /// [`Self::car_free_flow`].
    car_free_flow: OnceLock<CarFreeFlow>,
}

/// The free-flow time to drive from every road node to each node with a car park, within
/// the car's reach (S209): what park-and-ride candidates are
/// picked by, before their routes are found on the congested times. One backward search
/// per car-park node, once per run; 4 bytes per road node per car-park node (8 MB for
/// Amsterdam's 16). The search is node-based, without the turn table's U-turn ban, so a
/// time may be a little under the turn-aware one: a ranking, not a cost.
#[derive(Debug)]
pub(crate) struct CarFreeFlow {
    /// The car-park nodes, ascending.
    pub nodes: Vec<NodeId>,
    /// Per car-park node, the seconds from every road node (infinite beyond the reach).
    seconds: Vec<Vec<f32>>,
}

impl CarFreeFlow {
    /// The free-flow seconds from road node `from` to car-park node number `i`.
    pub(crate) fn seconds(&self, i: usize, from: NodeId) -> f64 {
        f64::from(self.seconds[i][from.index()])
    }
}

impl ParkingSetup {
    /// Link `rows` to the roads `road`, the bike layer `bike` (if any) and the
    /// walk layer and stops of `transit`, under `defaults`.
    ///
    /// # Errors
    ///
    /// [`ParkingError::DuplicateId`] if two rows share a `parking_id`.
    ///
    /// # Panics
    ///
    /// Never in practice: counts of parkings and stops fit `u32`.
    pub fn new(
        rows: &[ParkingRow],
        road: Arc<RoadNetwork>,
        bike: Option<&LayerSetup>,
        transit: &TransitSetup,
        defaults: ParkingDefaults,
    ) -> Result<Self, ParkingError> {
        let mut seen = std::collections::HashSet::new();
        for r in rows {
            if !seen.insert(r.parking_id.as_str()) {
                return Err(ParkingError::DuplicateId(r.parking_id.clone()));
            }
        }
        let mut report = ParkingSetupReport {
            rows: u32::try_from(rows.len()).expect("parkings fit u32"),
            ..ParkingSetupReport::default()
        };
        let road_snapper = NodeSnapper::new(&road);
        let bike_snapper = bike.map(|b| NodeSnapper::every_node(b.network().network()));
        let walk_graph = transit.walk().network();
        // Rows in id order, so the result does not depend on the table's order.
        let mut order: Vec<&ParkingRow> = rows.iter().collect();
        order.sort_by(|a, b| a.parking_id.cmp(&b.parking_id));
        struct Kept<'r> {
            row: &'r ParkingRow,
            node: NodeId,
            walk: NodeId,
            stops: Vec<(NodeId, u32)>,
        }
        let mut kept: Vec<Kept<'_>> = Vec::new();
        for row in order {
            let node = match row.kind {
                ParkingKind::Car => {
                    let n = road_snapper.nearest(&road, row.position);
                    (ground_distance_metres(row.position, road.node_lonlat(n)) <= defaults.snap_m)
                        .then_some(n)
                }
                ParkingKind::Bike => match (bike, &bike_snapper) {
                    (Some(b), Some(snapper)) => {
                        let graph = b.network().network();
                        let n = snapper.nearest(graph, row.position);
                        (ground_distance_metres(row.position, graph.node_lonlat(n))
                            <= defaults.snap_m)
                            .then_some(n)
                    }
                    _ => None,
                },
            };
            let walk = transit.walk_node(row.position);
            let walk_ok = ground_distance_metres(row.position, walk_graph.node_lonlat(walk))
                <= defaults.snap_m;
            let (Some(node), true) = (node, walk_ok) else {
                report.off_layer += 1;
                continue;
            };
            let stops = transit.stops_within(walk, defaults.walk_max_s);
            if stops.is_empty() {
                report.no_stop += 1;
                continue;
            }
            match row.kind {
                ParkingKind::Car => report.kept_car += 1,
                ParkingKind::Bike => report.kept_bike += 1,
            }
            kept.push(Kept { row, node, walk, stops });
        }
        // Hubs: one per hub id, or per parking; access points from the first parking
        // of each kind.
        let mut by_hub: Vec<(String, Vec<usize>)> = Vec::new();
        let mut hub_index: HashMap<String, usize> = HashMap::new();
        for (i, k) in kept.iter().enumerate() {
            let hub = k.row.hub_id.clone().unwrap_or_else(|| k.row.parking_id.clone());
            let slot = *hub_index.entry(hub.clone()).or_insert_with(|| {
                by_hub.push((hub, Vec::new()));
                by_hub.len() - 1
            });
            by_hub[slot].1.push(i);
        }
        let specs: Vec<HubSpec> = by_hub
            .iter()
            .map(|(hub, members)| {
                let mut access_points = Vec::new();
                for kind in ParkingKind::ALL {
                    if let Some(&m) = members.iter().find(|&&m| kept[m].row.kind == kind) {
                        access_points.push(AccessPoint { layer: kind.layer(), node: kept[m].node });
                    }
                }
                access_points.push(AccessPoint { layer: Layer::Walk, node: kept[members[0]].walk });
                HubSpec {
                    external_id: hub.clone(),
                    kind: HubKind::Declared,
                    position: kept[members[0]].row.position,
                    access_points,
                    transfer_seconds: 0.0,
                    parkings: members
                        .iter()
                        .map(|&m| Parking {
                            external_id: kept[m].row.parking_id.clone(),
                            kind: kept[m].row.kind,
                            capacity: kept[m].row.capacity,
                            initial_occupancy: kept[m].row.initial_occupancy,
                        })
                        .collect(),
                }
            })
            .collect();
        let hubs = HubSet::build(specs);
        // Per parking, in the hub set's parking order.
        let by_id: HashMap<&str, &Kept<'_>> =
            kept.iter().map(|k| (k.row.parking_id.as_str(), k)).collect();
        let n = hubs.parking_count() as usize;
        let mut setup = Self {
            defaults,
            road,
            node: Vec::with_capacity(n),
            walk_node: Vec::with_capacity(n),
            position: Vec::with_capacity(n),
            name: Vec::with_capacity(n),
            fee_eur: Vec::with_capacity(n),
            stops_start: vec![0],
            stops: Vec::new(),
            targets: [Vec::new(), Vec::new()],
            target_nodes: [0, 0],
            at_node: [HashMap::new(), HashMap::new()],
            report,
            hubs,
            car_free_flow: OnceLock::new(),
        };
        let layer_nodes = [
            setup.road.node_count() as usize,
            bike.map_or(0, |b| b.network().network().node_count() as usize),
        ];
        setup.targets = [vec![false; layer_nodes[0]], vec![false; layer_nodes[1]]];
        for p in 0..n {
            let id = ResourceId::from_index(p);
            let k = by_id[setup.hubs.parking_external_id(id)];
            let kind = k.row.kind.index();
            setup.node.push(k.node);
            setup.walk_node.push(k.walk);
            setup.position.push(k.row.position);
            setup.name.push(k.row.name.clone());
            setup.fee_eur.push(k.row.fee_eur);
            setup.stops.extend_from_slice(&k.stops);
            setup.stops_start.push(u32::try_from(setup.stops.len()).expect("fits u32"));
            if !setup.targets[kind][k.node.index()] {
                setup.targets[kind][k.node.index()] = true;
                setup.target_nodes[kind] += 1;
            }
            setup.at_node[kind]
                .entry(k.node.raw())
                .or_default()
                .push(u32::try_from(p).expect("parkings fit u32"));
        }
        Ok(setup)
    }

    /// The parking hubs.
    #[must_use]
    pub fn hubs(&self) -> &HubSet {
        &self.hubs
    }

    /// The defaults it was set up with.
    #[must_use]
    pub fn defaults(&self) -> ParkingDefaults {
        self.defaults
    }

    /// What was kept and left out.
    #[must_use]
    pub fn report(&self) -> ParkingSetupReport {
        self.report
    }

    /// How many parkings.
    #[must_use]
    pub fn count(&self) -> usize {
        self.node.len()
    }

    /// The road network its car parkings are on.
    #[must_use]
    pub fn road(&self) -> &Arc<RoadNetwork> {
        &self.road
    }

    /// A parking's kind.
    #[must_use]
    pub fn kind(&self, parking: u32) -> ParkingKind {
        self.hubs.parking_kind(ResourceId::new(parking))
    }

    /// A parking's capacity.
    #[must_use]
    pub fn capacity(&self, parking: u32) -> u32 {
        self.hubs.parking_capacity(ResourceId::new(parking))
    }

    /// A parking's occupancy at the start of the day.
    #[must_use]
    pub fn initial_occupancy(&self, parking: u32) -> u32 {
        self.hubs.parking_initial_occupancy(ResourceId::new(parking))
    }

    /// A parking's external id.
    #[must_use]
    pub fn external_id(&self, parking: u32) -> &str {
        self.hubs.parking_external_id(ResourceId::new(parking))
    }

    /// Its hub's external id.
    #[must_use]
    pub fn hub_external_id(&self, parking: u32) -> &str {
        let hub = self.hubs.parking_hub(ResourceId::new(parking));
        self.hubs.external_ids().external(hub.raw())
    }

    /// A parking's name, if it has one.
    #[must_use]
    pub fn name(&self, parking: u32) -> Option<&str> {
        self.name[parking as usize].as_deref()
    }

    /// A parking's fee per stay in euros, if its row gave one (S248, roadmap I-bb U5); else the
    /// run's price for its kind applies ([`crate::prices::Prices`]).
    #[must_use]
    pub fn fee_eur(&self, parking: u32) -> Option<f64> {
        self.fee_eur[parking as usize]
    }

    /// A parking's node on its vehicle's layer.
    #[must_use]
    pub fn node(&self, parking: u32) -> NodeId {
        self.node[parking as usize]
    }

    /// A parking's node on the walk layer.
    #[must_use]
    pub fn walk_node(&self, parking: u32) -> NodeId {
        self.walk_node[parking as usize]
    }

    /// Where a parking is.
    #[must_use]
    pub fn position(&self, parking: u32) -> LonLat {
        self.position[parking as usize]
    }

    /// The stops within walking distance of a parking, and the walk to each in
    /// seconds.
    #[must_use]
    pub fn stops(&self, parking: u32) -> &[(NodeId, u32)] {
        let p = parking as usize;
        &self.stops[self.stops_start[p] as usize..self.stops_start[p + 1] as usize]
    }

    /// The nodes of `kind`'s layer that have a parking, and how many.
    pub(crate) fn targets(&self, kind: ParkingKind) -> (&[bool], usize) {
        (&self.targets[kind.index()], self.target_nodes[kind.index()])
    }

    /// The parkings of `kind` at `node`.
    pub(crate) fn at(&self, kind: ParkingKind, node: NodeId) -> &[u32] {
        self.at_node[kind.index()].get(&node.raw()).map_or(&[], Vec::as_slice)
    }

    /// The free-flow drive from every road node to every car-park node (see
    /// [`CarFreeFlow`]). Searched in parallel by [`Self::prepare_car_free_flow`] before the
    /// planning that needs it; if nobody prepared it, searched here on first use, one car park
    /// after another (no parallel work inside a `OnceLock`'s initialisation: a worker waiting
    /// on its own parallel work may pick up a job that asks for the same cell, and deadlock).
    pub(crate) fn car_free_flow(&self) -> &CarFreeFlow {
        self.car_free_flow.get_or_init(|| {
            let (road, costs, nodes) = self.car_free_flow_inputs();
            let mut reach = openmobisim_core_routes::Reach::new(road, &costs);
            let seconds = nodes.iter().map(|&n| self.car_free_flow_to(&mut reach, n)).collect();
            CarFreeFlow { nodes, seconds }
        })
    }

    /// Search [`Self::car_free_flow`] now, in parallel, if it is not yet: called from outside
    /// any parallel work (the run's own thread), before the planning that reads it.
    pub(crate) fn prepare_car_free_flow(&self) {
        if self.car_free_flow.get().is_some() {
            return;
        }
        let (road, costs, nodes) = self.car_free_flow_inputs();
        let seconds = crate::transit::par_map(
            &nodes,
            1,
            || openmobisim_core_routes::Reach::new(road, &costs),
            |reach, &node| self.car_free_flow_to(reach, node),
        );
        // Only this thread fills it here; another may have filled it meanwhile with the same.
        let _ = self.car_free_flow.set(CarFreeFlow { nodes, seconds });
    }

    /// The road network, its free-flow costs (infinite where cars may not drive) and the
    /// car-park nodes, ascending.
    fn car_free_flow_inputs(&self) -> (&RoadNetwork, Vec<f64>, Vec<NodeId>) {
        let road = &*self.road;
        let costs: Vec<f64> = (0..road.link_count())
            .map(|i| {
                let link = openmobisim_core_types::ids::LinkId::new(i);
                if road.link_class(link).carries_motor_traffic() {
                    road.free_flow_time(link).get()
                } else {
                    f64::INFINITY
                }
            })
            .collect();
        let mut nodes: Vec<NodeId> =
            self.at_node[ParkingKind::Car.index()].keys().map(|&n| NodeId::new(n)).collect();
        nodes.sort_unstable();
        (road, costs, nodes)
    }

    /// The free-flow seconds from every road node to car-park node `node`, within the reach.
    fn car_free_flow_to(
        &self,
        reach: &mut openmobisim_core_routes::Reach<'_>,
        node: NodeId,
    ) -> Vec<f32> {
        let n = self.road.node_count() as usize;
        let every = vec![true; n];
        let mut to = vec![f32::INFINITY; n];
        for (from, secs) in reach.backward(node, self.defaults.reach_car_s, &every) {
            #[allow(clippy::cast_possible_truncation, reason = "seconds as f32")]
            let s = secs as f32;
            to[from.index()] = s;
        }
        to
    }

    /// Bytes held.
    #[must_use]
    pub fn bytes(&self) -> usize {
        self.hubs.bytes()
            + self.node.len() * (4 + 4 + 16 + 24)
            + self.stops_start.len() * 4
            + self.stops.len() * 8
            + self.targets.iter().map(Vec::len).sum::<usize>()
    }
}

/// How full the parkings were over one loading, bin by bin (M4).
#[derive(Clone, Debug, PartialEq)]
pub struct ParkingBins {
    /// The bin length, in seconds.
    pub bin_s: u32,
    /// How many bins.
    pub bins: usize,
    /// The window the bins cover, in seconds: the last bin ends with it, so it is shorter than
    /// the others when the window is not a whole number of bins (X-45).
    pub window_s: f64,
    /// Per parking, then bin (`parking * bins + bin`): vehicles arriving to park
    /// (weighted by traveller weight).
    pub arrivals: Vec<f64>,
    /// Vehicles fetched.
    pub departures: Vec<f64>,
    /// The mean occupancy over the bin.
    pub occupancy_mean: Vec<f64>,
    /// The most vehicles parked at once in the bin.
    pub occupancy_max: Vec<f64>,
    /// Seconds of the bin the parking was full (occupancy at or above capacity).
    pub full_s: Vec<f64>,
    /// The most vehicles above capacity at once in the bin (the soft overflow).
    pub overflow_max: Vec<f64>,
}

/// One vehicle parked or fetched: `(second, parking, change)`.
pub(crate) type ParkingEvent = (f64, u32, f64);

/// What the parkings of one kind did in a run (M4).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ParkingTotals {
    /// Vehicles parked, weighted by traveller weight.
    pub arrivals: f64,
    /// Of those, the ones that found the parking full (weighted).
    pub overflow_arrivals: f64,
    /// The hub expectation mismatch of those who parked, in seconds (`NaN` if none).
    pub mismatch_s: f64,
    /// Vehicles still parked at the end of the window (weighted): left at a hub for
    /// the day (design §23.5).
    pub left_at_end: f64,
}

/// What the parkings did in a run (M4).
#[derive(Clone, Debug, PartialEq)]
pub struct ParkingResult {
    /// Bin by bin.
    pub bins: ParkingBins,
    /// Per kind ([`ParkingKind::index`]): car parkings, bike parkings.
    pub by_kind: [ParkingTotals; 2],
    /// Vehicles parked, weighted by traveller weight.
    pub arrivals: f64,
    /// Of those, the ones that found the parking full (weighted).
    pub overflow_arrivals: f64,
    /// Per traveller who parked, the mean absolute difference between the parking
    /// time expected at choice and the one paid, in seconds (the hub expectation
    /// mismatch, design §11.2); NaN if nobody parked.
    pub mismatch_s: f64,
}

/// Tally `events` into bins over `window` seconds, each parking starting at
/// its initial occupancy. Returns the bins and, per arrival (an event with a
/// positive change, in the order given), whether it found the parking full.
pub(crate) fn tally(
    setup: &ParkingSetup,
    events: &[ParkingEvent],
    window: f64,
) -> (ParkingBins, Vec<bool>) {
    let bin_s = setup.defaults.bin_s.max(1.0);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a count of bins")]
    let bins = ((window / bin_s).ceil() as usize).max(1);
    let n = setup.count();
    let mut out = ParkingBins {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "seconds")]
        bin_s: bin_s as u32,
        bins,
        window_s: window,
        arrivals: vec![0.0; n * bins],
        departures: vec![0.0; n * bins],
        occupancy_mean: vec![0.0; n * bins],
        occupancy_max: vec![0.0; n * bins],
        full_s: vec![0.0; n * bins],
        overflow_max: vec![0.0; n * bins],
    };
    // Events by parking, in time order (fetches before arrivals at one second, so a
    // space freed is a space found), keeping each arrival's place in `events`.
    let mut order: Vec<usize> = (0..events.len()).collect();
    order.sort_by(|&a, &b| {
        let (ea, eb) = (events[a], events[b]);
        ea.1.cmp(&eb.1).then(ea.0.total_cmp(&eb.0)).then(ea.2.total_cmp(&eb.2)).then(a.cmp(&b))
    });
    let mut full_on_arrival = vec![false; events.len()];
    let bin_of = |t: f64| {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a bin")]
        let b = (t.max(0.0) / bin_s) as usize;
        b.min(bins - 1)
    };
    let mut i = 0;
    for p in 0..n {
        let p32 = u32::try_from(p).expect("parkings fit u32");
        let capacity = f64::from(setup.capacity(p32));
        let mut stock = f64::from(setup.initial_occupancy(p32));
        let mut clock = 0.0;
        let row = p * bins;
        // Close the interval [clock, t) at occupancy `stock`.
        let advance = |out: &mut ParkingBins, from: f64, to: f64, stock: f64| {
            let mut t = from;
            while t < to {
                let b = bin_of(t);
                #[allow(clippy::cast_precision_loss, reason = "a count of bins")]
                let end = (((b + 1) as f64) * bin_s).min(to);
                let span = end - t;
                if span <= 0.0 {
                    break;
                }
                out.occupancy_mean[row + b] += stock * span / out.bin_len(b);
                out.occupancy_max[row + b] = out.occupancy_max[row + b].max(stock);
                out.overflow_max[row + b] = out.overflow_max[row + b].max(stock - capacity);
                if stock >= capacity {
                    out.full_s[row + b] += span;
                }
                t = end;
            }
        };
        while i < order.len() && events[order[i]].1 == p32 {
            let (t, _, change) = events[order[i]];
            let t = t.clamp(0.0, window);
            advance(&mut out, clock, t, stock);
            clock = clock.max(t);
            let b = bin_of(t);
            if change > 0.0 {
                full_on_arrival[order[i]] = stock >= capacity;
                out.arrivals[row + b] += change;
            } else {
                out.departures[row + b] -= change;
            }
            stock += change;
            out.occupancy_max[row + b] = out.occupancy_max[row + b].max(stock);
            out.overflow_max[row + b] = out.overflow_max[row + b].max(stock - capacity);
            i += 1;
        }
        advance(&mut out, clock, window, stock);
    }
    (out, full_on_arrival)
}

/// Availability per parking and bin from a tally: the share of the bin with a
/// free space.
pub(crate) fn availability(bins: &ParkingBins) -> Vec<f64> {
    bins.full_s
        .iter()
        .enumerate()
        .map(|(k, &f)| (1.0 - f / bins.bin_len(k % bins.bins)).clamp(0.0, 1.0))
        .collect()
}

impl ParkingBins {
    /// The length of bin `b` inside the window, in seconds: a whole bin, or for the last one
    /// what is left of the window (X-45). Never 0, so a share of it is always defined.
    #[must_use]
    pub fn bin_len(&self, b: usize) -> f64 {
        let whole = f64::from(self.bin_s);
        if b + 1 < self.bins {
            return whole;
        }
        #[allow(clippy::cast_precision_loss, reason = "a count of bins")]
        let left = self.window_s - whole * (self.bins - 1) as f64;
        if left > 0.0 { left.min(whole) } else { whole }
    }
}

/// The availability of `parking` at second `t` in a tally's `values` (from
/// [`availability`]).
pub(crate) fn availability_at(values: &[f64], bins: &ParkingBins, parking: u32, t: f64) -> f64 {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a bin")]
    let b = ((t.max(0.0) / f64::from(bins.bin_s)) as usize).min(bins.bins - 1);
    values[parking as usize * bins.bins + b]
}

/// The expected availability per parking and bin, across iterations: the
/// average of the realised ones so far (design §11.3), empty at the start except
/// where a parking starts the day full.
#[derive(Clone, Debug)]
pub(crate) struct ExpectedAvailability {
    bin_s: f64,
    bins: usize,
    values: Vec<f64>,
    seen: u32,
}

impl ExpectedAvailability {
    /// Before any loading: from the initial occupancy alone.
    pub(crate) fn initial(setup: &ParkingSetup, window: f64) -> Self {
        let (bins, _) = tally(setup, &[], window);
        Self { bin_s: f64::from(bins.bin_s), bins: bins.bins, values: availability(&bins), seen: 0 }
    }

    /// Average in a loading's realised availability.
    pub(crate) fn absorb(&mut self, realised: &[f64]) {
        self.seen += 1;
        let w = 1.0 / f64::from(self.seen);
        for (v, r) in self.values.iter_mut().zip(realised) {
            *v += w * (r - *v);
        }
    }

    /// The expected availability of `parking` at second `t`.
    pub(crate) fn at(&self, parking: u32, t: f64) -> f64 {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a bin")]
        let b = ((t.max(0.0) / self.bin_s) as usize).min(self.bins - 1);
        self.values[parking as usize * self.bins + b]
    }
}
