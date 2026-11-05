//! The network: structure of arrays, built once from rows, then immutable.
//!
//! # Structure of arrays, not array of structures
//!
//! The loading reads one attribute of every link in a pass (all the capacities, then all the
//! lengths) and never one link's whole record. Parallel `Vec`s pull sixteen useful values per
//! cache line where a `Vec<Link>` would pull one record of unrelated fields.
//!
//! # Identity is assigned here, deterministically
//!
//! [`NetworkBuilder`] takes external ids (OpenStreetMap node and way ids, the ids of a table)
//! and assigns dense `u32` ids **by sorted external id**. Two builds of the same input therefore
//! give the same ids, in the same order, whatever order the input listed them in.
//!
//! # What a link means
//!
//! A link has an upstream node and a downstream node, nothing else about its ends. Its
//! free-flow time for a mode is its length divided by that mode's free-flow speed, nothing else;
//! a signal never changes it. Its capacity is the flow its downstream end passes: the row's
//! value when the row gives one, else lanes times the per-lane capacity; a link without cars has
//! no car speed, no lanes and no capacity limit. A bike is a mode on the
//! same links as a car, with its own free-flow speed: there is one network, no separate layer.
//!
//! # Bad input is an error
//!
//! A link to an undeclared node, a length that is not a positive number, a speed that is not a
//! positive number, an empty network, or a mix of position kinds stops the build with a
//! [`BuildError`] naming the node or link. Nothing is repaired.

use core::fmt;

use openmobisim_types::ids::{EntityId, ExternalIdTable, ExternalIdTableBuilder, LinkId, NodeId};
use openmobisim_types::mode::{Mode, ModeSet};
use openmobisim_types::units::{Duration, Flow, Metres, Speed};

use crate::csr::Csr;
use crate::geometry::{LonLat, Position, Projection, Xy};

/// What the caller knows about a link: one row of a link table.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LinkRow {
    /// The modes allowed on the link: any of car, bike and walk, at least one. Transit runs on
    /// its timetable, not on links.
    pub modes: ModeSet,
    /// The lanes in the link's own direction, at least one. Required when cars are allowed;
    /// `None` when they are not.
    pub lanes: Option<u8>,
    /// The car free-flow speed, in kilometres per hour, above zero. Required when cars are
    /// allowed; `None` when they are not.
    pub car_speed_km_h: Option<f64>,
    /// The length in metres; `None` is the straight line between the two nodes.
    pub length_m: Option<f64>,
    /// The capacity of the downstream end across all lanes, in vehicles per hour, above zero;
    /// `f64::INFINITY` is an unlimited capacity (the downstream end passes any flow). When cars
    /// are allowed, `None` is lanes times [`BuildValues::capacity_per_lane`]; when they are not,
    /// it must be `None` (the link has no capacity limit: nothing is loaded there).
    pub capacity_veh_h: Option<f64>,
    /// Whether a signal stands at the downstream end. Stored, applied nowhere.
    pub signal_at_downstream_end: bool,
}

/// The three values a build needs besides its rows. The caller supplies all three: there is no
/// default here (the table of record is `PARAMETERS.md`).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct BuildValues {
    /// The capacity of one lane, in vehicles per hour per lane, for a row with no capacity.
    pub capacity_per_lane: f64,
    /// The free-flow speed of a bike, in kilometres per hour, on every link.
    pub bike_speed: f64,
    /// The free-flow speed of a walker, in kilometres per hour, on every link.
    pub walk_speed: f64,
}

/// Why a network could not be built. Each names what it refuses.
#[derive(Clone, PartialEq, Debug)]
pub enum BuildError {
    /// No node was declared.
    NoNodes,
    /// One of the three build values is not a finite number above zero.
    BadValue {
        /// The value's name, as in `PARAMETERS.md`.
        name: &'static str,
        /// The value given.
        value: f64,
    },
    /// Some nodes are in longitude/latitude and others in metres.
    MixedPositions {
        /// The first node, in sorted order, given in longitude/latitude.
        lon_lat: String,
        /// The first node, in sorted order, given in metres.
        metres: String,
    },
    /// A node is not finite, or outside longitude ±180° and latitude ±84° (the UTM system).
    NotProjectable {
        /// The node.
        node: String,
        /// Its position as given.
        at: LonLat,
    },
    /// A position in metres that is not finite.
    NotFinite {
        /// The node.
        node: String,
    },
    /// The same node external id was declared twice.
    DuplicateNode(String),
    /// The same link external id was declared twice.
    DuplicateLink(String),
    /// A node marked signalised was never declared.
    UnknownSignalNode(String),
    /// A link names a node that was never declared.
    UnknownNode {
        /// The link.
        link: String,
        /// The node it names.
        node: String,
    },
    /// A link allows no mode, or allows transit.
    BadModes {
        /// The link.
        link: String,
        /// The modes it gave.
        modes: ModeSet,
    },
    /// A link has zero lanes.
    NoLanes(String),
    /// A link that allows cars lacks a car field (`lanes` or `car_speed_km_h`).
    MissingCarField {
        /// The link.
        link: String,
        /// The field's name in [`LinkRow`].
        field: &'static str,
    },
    /// A link that does not allow cars gives a car field (`lanes`, `car_speed_km_h` or
    /// `capacity_veh_h`).
    CarFieldWithoutCars {
        /// The link.
        link: String,
        /// The field's name in [`LinkRow`].
        field: &'static str,
    },
    /// A link's car free-flow speed is not a finite number above zero.
    BadSpeed {
        /// The link.
        link: String,
        /// The speed given, in kilometres per hour.
        km_h: f64,
    },
    /// A link's length (given, or the straight line between its nodes) is not a finite number
    /// above zero.
    BadLength {
        /// The link.
        link: String,
        /// The length, in metres.
        metres: f64,
    },
    /// A link's given capacity is not a number above zero (NaN, zero, negative or negative
    /// infinity). Positive infinity, an unlimited capacity, is allowed.
    BadCapacity {
        /// The link.
        link: String,
        /// The capacity given, in vehicles per hour.
        veh_h: f64,
    },
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoNodes => f.write_str("the network has no nodes"),
            Self::BadValue { name, value } => {
                write!(f, "{name} must be a finite number above zero, got {value}")
            }
            Self::MixedPositions { lon_lat, metres } => write!(
                f,
                "node {lon_lat} is in longitude/latitude but node {metres} is in metres; \
                 give every node in one kind"
            ),
            Self::NotProjectable { node, at } => write!(
                f,
                "node {node} at ({}, {}) is not projectable: longitude must be within ±180°, \
                 latitude within ±84°, and both finite",
                at.lon, at.lat
            ),
            Self::NotFinite { node } => write!(f, "node {node} has a position that is not finite"),
            Self::DuplicateNode(node) => write!(f, "node {node} is declared twice"),
            Self::DuplicateLink(link) => write!(f, "link {link} is declared twice"),
            Self::UnknownSignalNode(node) => {
                write!(f, "node {node} is marked signalised but never declared")
            }
            Self::UnknownNode { link, node } => {
                write!(f, "link {link} names node {node}, which is never declared")
            }
            Self::BadModes { link, modes } => write!(
                f,
                "link {link} allows {modes:?}: a link allows at least one of car, bike and walk, \
                 and never transit"
            ),
            Self::NoLanes(link) => write!(f, "link {link} has zero lanes"),
            Self::MissingCarField { link, field } => {
                write!(f, "link {link} allows cars but gives no {field}")
            }
            Self::CarFieldWithoutCars { link, field } => {
                write!(f, "link {link} does not allow cars but gives {field}; leave it empty")
            }
            Self::BadSpeed { link, km_h } => write!(
                f,
                "link {link} has car free-flow speed {km_h} km/h; it must be finite and above zero"
            ),
            Self::BadLength { link, metres } => {
                write!(f, "link {link} has length {metres} m; it must be finite and above zero")
            }
            Self::BadCapacity { link, veh_h } => write!(
                f,
                "link {link} has capacity {veh_h} veh/h; it must be above zero (infinity, \
                 an unlimited capacity, is allowed)"
            ),
        }
    }
}

impl core::error::Error for BuildError {}

/// Builds a [`Network`] from rows.
///
/// Nodes and links may arrive in any order; ids are assigned by sorted external id when
/// [`build`](Self::build) runs, and every check is made there.
#[derive(Debug, Default)]
pub struct NetworkBuilder {
    nodes: Vec<(String, Position)>,
    signalised: Vec<String>,
    links: Vec<(String, String, String, LinkRow)>,
}

impl NetworkBuilder {
    /// An empty builder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Declare a node at a position: longitude/latitude (projected once, in the UTM zone of the
    /// nodes' centre) or metres (used as they are). All nodes are of one kind.
    pub fn add_node(&mut self, external_id: impl Into<String>, position: Position) {
        self.nodes.push((external_id.into(), position));
    }

    /// Mark a node as signalised. Stored, applied nowhere.
    pub fn mark_signalised(&mut self, external_id: impl Into<String>) {
        self.signalised.push(external_id.into());
    }

    /// Declare a directed link from `upstream` to `downstream`. A two-way street is two links.
    pub fn add_link(
        &mut self,
        external_id: impl Into<String>,
        upstream: impl Into<String>,
        downstream: impl Into<String>,
        row: LinkRow,
    ) {
        self.links.push((external_id.into(), upstream.into(), downstream.into(), row));
    }

    /// Check every row, assign ids, project, and freeze.
    ///
    /// # Errors
    ///
    /// A [`BuildError`] for the first fault found, nodes before links, each in sorted external
    /// order (so the same input gives the same error whatever its order).
    pub fn build(mut self, values: BuildValues) -> Result<Network, BuildError> {
        let per_lane = positive("capacity_per_lane", values.capacity_per_lane)?;
        let bike = Speed::from_km_per_hour(positive("bike_speed", values.bike_speed)?);
        let walk = Speed::from_km_per_hour(positive("walk_speed", values.walk_speed)?);

        let nodes = place_nodes(&mut self.nodes, &mut self.signalised)?;
        self.links.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        if let Some(w) = self.links.windows(2).find(|w| w[0].0 == w[1].0) {
            return Err(BuildError::DuplicateLink(w[0].0.clone()));
        }
        let mut links = Links::with_capacity(self.links.len());
        let mut link_ids = ExternalIdTableBuilder::new();
        for (external, upstream, downstream, row) in &self.links {
            let up = node_of(&nodes.ids, external, upstream)?;
            let down = node_of(&nodes.ids, external, downstream)?;
            let straight = nodes.xy[up.index()].distance_to(nodes.xy[down.index()]);
            links.push(external, up, down, row, straight, per_lane)?;
            link_ids.insert(external.as_str());
        }

        let node_count = nodes.ids.count();
        let pairs = |ends: &[NodeId]| -> Vec<(NodeId, LinkId)> {
            ends.iter().enumerate().map(|(i, &n)| (n, LinkId::from_index(i))).collect()
        };
        let out_links = Csr::from_pairs(node_count, pairs(&links.upstream));
        let in_links = Csr::from_pairs(node_count, pairs(&links.downstream));
        let n = links.length.len();
        Ok(Network {
            projection: nodes.projection,
            node_ids: nodes.ids,
            link_ids: link_ids.build(),
            node_x: nodes.xy.iter().map(|p| p.x).collect(),
            node_y: nodes.xy.iter().map(|p| p.y).collect(),
            node_signalised: nodes.signalised,
            link_upstream: links.upstream,
            link_downstream: links.downstream,
            link_length: links.length,
            link_speed: [links.car_speed, vec![bike; n], vec![walk; n]],
            link_capacity: links.capacity,
            link_lanes: links.lanes,
            link_modes: links.modes,
            link_signal_downstream: links.signal_downstream,
            out_links,
            in_links,
        })
    }
}

/// `value` if it is a finite number above zero, else the error naming it.
fn positive(name: &'static str, value: f64) -> Result<f64, BuildError> {
    if value.is_finite() && value > 0.0 {
        Ok(value)
    } else {
        Err(BuildError::BadValue { name, value })
    }
}

/// The nodes, checked and placed in metres, in sorted external order.
struct Nodes {
    ids: ExternalIdTable,
    xy: Vec<Xy>,
    signalised: Vec<bool>,
    projection: Option<Projection>,
}

/// Sort and check the nodes, choose the projection if they are in longitude/latitude, and mark
/// the signalised ones.
fn place_nodes(
    nodes: &mut [(String, Position)],
    signalised: &mut [String],
) -> Result<Nodes, BuildError> {
    if nodes.is_empty() {
        return Err(BuildError::NoNodes);
    }
    nodes.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    if let Some(w) = nodes.windows(2).find(|w| w[0].0 == w[1].0) {
        return Err(BuildError::DuplicateNode(w[0].0.clone()));
    }
    let first_lon_lat = nodes.iter().find(|(_, p)| matches!(p, Position::LonLat(_)));
    let first_metres = nodes.iter().find(|(_, p)| matches!(p, Position::Metres(_)));
    let (xy, projection) = match (first_lon_lat, first_metres) {
        (Some((a, _)), Some((b, _))) => {
            return Err(BuildError::MixedPositions { lon_lat: a.clone(), metres: b.clone() });
        }
        (Some(_), None) => project_all(nodes)?,
        _ => (metres_all(nodes)?, None),
    };

    let mut ids = ExternalIdTableBuilder::new();
    ids.extend(nodes.iter().map(|(id, _)| id.as_str()));
    let ids = ids.build();
    let mut flags = vec![false; nodes.len()];
    signalised.sort_unstable();
    for external in signalised.iter() {
        let id =
            ids.id_of(external).ok_or_else(|| BuildError::UnknownSignalNode(external.clone()))?;
        flags[id as usize] = true;
    }
    Ok(Nodes { ids, xy, signalised: flags, projection })
}

/// Project nodes that are all in longitude/latitude, in the UTM zone of their centre.
fn project_all(nodes: &[(String, Position)]) -> Result<(Vec<Xy>, Option<Projection>), BuildError> {
    let mut points = Vec::with_capacity(nodes.len());
    for (node, position) in nodes {
        let Position::LonLat(at) = *position else { unreachable!("the caller checked the kind") };
        if !at.is_projectable() {
            return Err(BuildError::NotProjectable { node: node.clone(), at });
        }
        points.push(at);
    }
    let projection = Projection::for_extent(points.iter().copied())
        .expect("every point was checked projectable and there is at least one");
    Ok((points.iter().map(|&p| projection.project(p)).collect(), Some(projection)))
}

/// Take nodes that are all in metres as they are.
fn metres_all(nodes: &[(String, Position)]) -> Result<Vec<Xy>, BuildError> {
    nodes
        .iter()
        .map(|(node, position)| {
            let Position::Metres(xy) = *position else {
                unreachable!("the caller checked the kind")
            };
            if xy.x.is_finite() && xy.y.is_finite() {
                Ok(xy)
            } else {
                Err(BuildError::NotFinite { node: node.clone() })
            }
        })
        .collect()
}

/// The internal id of a node a link names.
fn node_of(ids: &ExternalIdTable, link: &str, node: &str) -> Result<NodeId, BuildError> {
    ids.typed_id_of(node)
        .ok_or_else(|| BuildError::UnknownNode { link: link.to_owned(), node: node.to_owned() })
}

/// The link columns as they are filled, in sorted external order.
struct Links {
    upstream: Vec<NodeId>,
    downstream: Vec<NodeId>,
    length: Vec<Metres>,
    car_speed: Vec<Speed>,
    capacity: Vec<Flow>,
    lanes: Vec<u8>,
    modes: Vec<ModeSet>,
    signal_downstream: Vec<bool>,
}

impl Links {
    fn with_capacity(n: usize) -> Self {
        Self {
            upstream: Vec::with_capacity(n),
            downstream: Vec::with_capacity(n),
            length: Vec::with_capacity(n),
            car_speed: Vec::with_capacity(n),
            capacity: Vec::with_capacity(n),
            lanes: Vec::with_capacity(n),
            modes: Vec::with_capacity(n),
            signal_downstream: Vec::with_capacity(n),
        }
    }

    /// Check one row and append it. `straight` is the distance between its nodes, in metres;
    /// `per_lane` the capacity of one lane, in vehicles per hour.
    fn push(
        &mut self,
        link: &str,
        upstream: NodeId,
        downstream: NodeId,
        row: &LinkRow,
        straight: f64,
        per_lane: f64,
    ) -> Result<(), BuildError> {
        if row.modes.is_empty() || row.modes.contains(Mode::Transit) {
            return Err(BuildError::BadModes { link: link.to_owned(), modes: row.modes });
        }
        let metres = row.length_m.unwrap_or(straight);
        if !(metres.is_finite() && metres > 0.0) {
            return Err(BuildError::BadLength { link: link.to_owned(), metres });
        }
        let (car_speed, lanes, capacity) = car_fields(link, row, per_lane)?;
        self.upstream.push(upstream);
        self.downstream.push(downstream);
        self.length.push(Metres(metres));
        self.car_speed.push(car_speed);
        self.capacity.push(capacity);
        self.lanes.push(lanes);
        self.modes.push(row.modes);
        self.signal_downstream.push(row.signal_at_downstream_end);
        Ok(())
    }
}

/// The car speed, lanes and capacity of a row, checked. A link without cars gives none of
/// them and gets no speed (NaN, never read), 0 lanes and an infinite capacity.
fn car_fields(link: &str, row: &LinkRow, per_lane: f64) -> Result<(Speed, u8, Flow), BuildError> {
    if !row.modes.contains(Mode::Car) {
        let given = [
            ("lanes", row.lanes.is_some()),
            ("car_speed_km_h", row.car_speed_km_h.is_some()),
            ("capacity_veh_h", row.capacity_veh_h.is_some()),
        ];
        if let Some(&(field, _)) = given.iter().find(|(_, g)| *g) {
            return Err(BuildError::CarFieldWithoutCars { link: link.to_owned(), field });
        }
        return Ok((Speed(f64::NAN), 0, Flow::INFINITE));
    }
    let missing = |field| BuildError::MissingCarField { link: link.to_owned(), field };
    let lanes = row.lanes.ok_or_else(|| missing("lanes"))?;
    if lanes == 0 {
        return Err(BuildError::NoLanes(link.to_owned()));
    }
    let km_h = row.car_speed_km_h.ok_or_else(|| missing("car_speed_km_h"))?;
    if !(km_h.is_finite() && km_h > 0.0) {
        return Err(BuildError::BadSpeed { link: link.to_owned(), km_h });
    }
    // Positive infinity is an unlimited capacity, kept as `Flow::INFINITE` (∞ / 3 600 = ∞).
    let veh_h = row.capacity_veh_h.unwrap_or(f64::from(lanes) * per_lane);
    if veh_h.is_nan() || veh_h <= 0.0 {
        return Err(BuildError::BadCapacity { link: link.to_owned(), veh_h });
    }
    Ok((Speed::from_km_per_hour(km_h), lanes, Flow::from_veh_per_hour(veh_h)))
}

/// A built network. Immutable; every later module reads it.
#[derive(Debug)]
pub struct Network {
    projection: Option<Projection>,
    node_ids: ExternalIdTable,
    link_ids: ExternalIdTable,

    node_x: Vec<f64>,
    node_y: Vec<f64>,
    node_signalised: Vec<bool>,

    link_upstream: Vec<NodeId>,
    link_downstream: Vec<NodeId>,
    link_length: Vec<Metres>,
    /// Free-flow speed per link, one column per link mode: car, bike, walk.
    link_speed: [Vec<Speed>; 3],
    link_capacity: Vec<Flow>,
    link_lanes: Vec<u8>,
    link_modes: Vec<ModeSet>,
    link_signal_downstream: Vec<bool>,

    out_links: Csr<NodeId, LinkId>,
    in_links: Csr<NodeId, LinkId>,
}

/// The column of a link mode in [`Network::link_speed`].
fn speed_column(mode: Mode) -> usize {
    match mode {
        Mode::Car => 0,
        Mode::Bike => 1,
        Mode::Walk => 2,
        Mode::Transit => panic!("transit is not on links: it runs on its timetable"),
    }
}

impl Network {
    /// How many nodes.
    #[inline]
    #[must_use]
    pub fn node_count(&self) -> u32 {
        self.node_ids.count()
    }

    /// How many directed links.
    #[inline]
    #[must_use]
    pub fn link_count(&self) -> u32 {
        self.link_ids.count()
    }

    /// The nodes' external ids, in internal-id order.
    #[inline]
    #[must_use]
    pub fn node_ids(&self) -> &ExternalIdTable {
        &self.node_ids
    }

    /// The links' external ids, in internal-id order.
    #[inline]
    #[must_use]
    pub fn link_ids(&self) -> &ExternalIdTable {
        &self.link_ids
    }

    /// The projection the nodes were placed with, or `None` if they were given in metres.
    /// Anything else in longitude/latitude (a stop, a zone's centre) is projected with it.
    #[inline]
    #[must_use]
    pub fn projection(&self) -> Option<Projection> {
        self.projection
    }

    /// A node's position, in metres.
    #[inline]
    #[must_use]
    pub fn node_position(&self, node: NodeId) -> Xy {
        Xy::new(self.node_x[node.index()], self.node_y[node.index()])
    }

    /// Whether a node is signalised. Stored, applied nowhere.
    #[inline]
    #[must_use]
    pub fn is_signalised(&self, node: NodeId) -> bool {
        self.node_signalised[node.index()]
    }

    /// A link's upstream node.
    #[inline]
    #[must_use]
    pub fn upstream(&self, link: LinkId) -> NodeId {
        self.link_upstream[link.index()]
    }

    /// A link's downstream node.
    #[inline]
    #[must_use]
    pub fn downstream(&self, link: LinkId) -> NodeId {
        self.link_downstream[link.index()]
    }

    /// A link's length.
    #[inline]
    #[must_use]
    pub fn length(&self, link: LinkId) -> Metres {
        self.link_length[link.index()]
    }

    /// A link's free-flow speed for a mode (car, bike or walk).
    ///
    /// # Panics
    ///
    /// For [`Mode::Transit`], which is not on links, and for [`Mode::Car`] on a link that does
    /// not allow cars, which has no car speed.
    #[inline]
    #[must_use]
    pub fn free_flow_speed(&self, link: LinkId, mode: Mode) -> Speed {
        assert!(
            mode != Mode::Car || self.modes(link).contains(Mode::Car),
            "link {link} does not allow cars: it has no car speed"
        );
        self.link_speed[speed_column(mode)][link.index()]
    }

    /// A link's free-flow time for a mode: its length divided by the mode's free-flow speed,
    /// nothing else. A signal never changes it.
    ///
    /// # Panics
    ///
    /// For [`Mode::Transit`], which is not on links, and for [`Mode::Car`] on a link that does
    /// not allow cars.
    #[inline]
    #[must_use]
    pub fn free_flow_time(&self, link: LinkId, mode: Mode) -> Duration {
        self.length(link) / self.free_flow_speed(link, mode)
    }

    /// The flow a link's downstream end passes, in vehicles per second; infinite (no limit) on a
    /// link that does not allow cars.
    #[inline]
    #[must_use]
    pub fn capacity(&self, link: LinkId) -> Flow {
        self.link_capacity[link.index()]
    }

    /// A link's lanes in its own direction; 0 on a link that does not allow cars.
    #[inline]
    #[must_use]
    pub fn lanes(&self, link: LinkId) -> u8 {
        self.link_lanes[link.index()]
    }

    /// The modes allowed on a link.
    #[inline]
    #[must_use]
    pub fn modes(&self, link: LinkId) -> ModeSet {
        self.link_modes[link.index()]
    }

    /// Whether a signal stands at a link's downstream end. Stored, applied nowhere.
    #[inline]
    #[must_use]
    pub fn has_signal_downstream(&self, link: LinkId) -> bool {
        self.link_signal_downstream[link.index()]
    }

    /// The links leaving a node, in link-id order.
    #[inline]
    #[must_use]
    pub fn out_links(&self, node: NodeId) -> &[LinkId] {
        self.out_links.targets(node)
    }

    /// The links entering a node, in link-id order.
    #[inline]
    #[must_use]
    pub fn in_links(&self, node: NodeId) -> &[LinkId] {
        self.in_links.targets(node)
    }
}
