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
//! # What cannot be used is repaired, counted, and the build goes on
//!
//! A row the network cannot use as given is set right or left out, and noted in the network's
//! [`Repairs`] (`Network::repairs`), one kind per rule (`src/network/rows.rs` lists them), so that
//! its caller reports each kind with its count and first ids; nothing is set right silently. Only
//! what leaves no network to build stops it with a [`BuildError`]: no node, a build value that is
//! not a finite number above zero, nodes of both position kinds.

use core::fmt;

use openmobisim_types::fingerprint::Fingerprint;
use openmobisim_types::ids::{EntityId, ExternalIdTable, LinkId, NodeId};
use openmobisim_types::mode::{Mode, ModeSet};
use openmobisim_types::repairs::Repairs;
use openmobisim_types::units::{Duration, Flow, Metres, Speed};

use crate::csr::Csr;
use crate::geometry::{Hemisphere, Position, Projection, Xy};

mod drawing;
mod rows;
pub use drawing::{LinkDrawing, SHAPES_UNREAD};

/// The key and phrase of a link of 0 m, left out: the graph's, and the OSM reader's, which leaves
/// such a piece out before it takes the car network's strong component.
pub const LINKS_OF_NO_LENGTH: (&str, &str) = (
    "links_of_no_length",
    "links of no length (a node to itself, or two nodes at one position): left out",
);

/// What the caller knows about a link: one row of a link table.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LinkRow {
    /// The modes allowed on the link: any of car, bike and walk, at least one (transit runs on
    /// its timetable, not on links: taken out of the set, noted; a link left with none is
    /// dropped, noted).
    pub modes: ModeSet,
    /// The lanes in the link's own direction, at least one, when cars are allowed (a car link
    /// without lanes, or with 0, loses its car mode, noted); ignored when they are not.
    pub lanes: Option<u8>,
    /// The car free-flow speed, in kilometres per hour, above zero, when cars are allowed (a
    /// car link without one loses its car mode, noted); ignored when they are not.
    pub car_speed_km_h: Option<f64>,
    /// The length in metres; `None` is the straight line between the two nodes (a length not
    /// finite or not above zero is replaced by it, noted; a link whose straight line is 0 m is
    /// dropped, noted).
    pub length_m: Option<f64>,
    /// The capacity of the downstream end across all lanes, in vehicles per hour, above zero;
    /// `f64::INFINITY` is an unlimited capacity (the downstream end passes any flow). When cars
    /// are allowed, `None` (or a value not above zero, noted) is lanes times
    /// [`BuildValues::capacity_per_lane`]; when they are not, it is ignored (the link has no
    /// capacity limit: nothing is loaded there).
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

/// Why a network could not be built: nothing is left to build one from, or a value it cannot
/// guess. Each names what it refuses.
#[derive(Clone, PartialEq, Debug)]
pub enum BuildError {
    /// No node was declared, or none could be placed.
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
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoNodes => f.write_str("the network has no node that can be placed"),
            Self::BadValue { name, value } => {
                write!(f, "{name} must be a finite number above zero, got {value}")
            }
            Self::MixedPositions { lon_lat, metres } => write!(
                f,
                "node {lon_lat} is in longitude/latitude but node {metres} is in metres; \
                 give every node in one kind"
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
    links: Vec<rows::GivenLink>,
    /// The row each link id was last given at: a drawing belongs to it.
    last_row: std::collections::HashMap<String, usize>,
    /// Drawings given for a link id no row was given for (yet), and for a row given one already.
    stray_drawings: Vec<String>,
    second_drawings: Vec<String>,
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
        let id = external_id.into();
        self.last_row.insert(id.clone(), self.links.len());
        let (upstream, downstream) = (upstream.into(), downstream.into());
        self.links.push(rows::GivenLink { id, upstream, downstream, row, drawing: None });
    }

    /// Repair or leave out every row the network cannot use (`src/network/rows.rs`, each noted
    /// in [`Network::repairs`]), assign ids, project, and freeze.
    ///
    /// # Errors
    ///
    /// A [`BuildError`]: a build value that is not a finite number above zero, nodes of both
    /// position kinds, or no node left to place.
    pub fn build(self, values: BuildValues) -> Result<Network, BuildError> {
        let per_lane = positive("capacity_per_lane", values.capacity_per_lane)?;
        let bike = Speed::from_km_per_hour(positive("bike_speed", values.bike_speed)?);
        let walk = Speed::from_km_per_hour(positive("walk_speed", values.walk_speed)?);
        let mut repairs = Repairs::new();
        let nodes = rows::place_nodes(self.nodes, self.signalised, &mut repairs)?;
        let links = rows::links(self.links, &nodes, per_lane, &mut repairs);
        let node_count = nodes.ids.count();
        let pairs = |ends: &[NodeId]| -> Vec<(NodeId, LinkId)> {
            ends.iter().enumerate().map(|(i, &n)| (n, LinkId::from_index(i))).collect()
        };
        let out_links = Csr::from_pairs(node_count, pairs(&links.upstream));
        let in_links = Csr::from_pairs(node_count, pairs(&links.downstream));
        let n = links.length.len();
        let given = drawing::Given { stray: self.stray_drawings, second: self.second_drawings };
        let drawings = drawing::build(&links.drawings, given, nodes.projection, &mut repairs);
        Ok(Network {
            projection: nodes.projection,
            node_ids: nodes.ids,
            link_ids: links.ids,
            node_x: nodes.xy.iter().map(|p| p.x).collect(),
            node_y: nodes.xy.iter().map(|p| p.y).collect(),
            node_signalised: nodes.signalised,
            link_upstream: links.upstream,
            link_downstream: links.downstream,
            link_length: links.length,
            link_speed: [links.car_speed, vec![bike; n], vec![walk; n]],
            bike_speed: bike,
            walk_speed: walk,
            link_capacity: links.capacity,
            link_lanes: links.lanes,
            link_modes: links.modes,
            link_signal_downstream: links.signal_downstream,
            out_links,
            in_links,
            drawings,
            repairs,
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
    /// The build values' bike and walk speeds, the same on every link.
    bike_speed: Speed,
    walk_speed: Speed,
    /// Free-flow speed per link, one column per link mode: car, bike, walk.
    link_speed: [Vec<Speed>; 3],
    link_capacity: Vec<Flow>,
    link_lanes: Vec<u8>,
    link_modes: Vec<ModeSet>,
    link_signal_downstream: Vec<bool>,

    out_links: Csr<NodeId, LinkId>,
    in_links: Csr<NodeId, LinkId>,
    /// For drawing only: each link's shape and road class (not in the fingerprint).
    drawings: drawing::Drawings,
    /// What the build repaired or left out (not in the fingerprint).
    repairs: Repairs,
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

    /// The free-flow speed of a bike, in metres per second, the same on every link: the build
    /// value `bike_speed` given to the builder (in km/h there).
    #[inline]
    #[must_use]
    pub fn bike_speed(&self) -> Speed {
        self.bike_speed
    }

    /// The free-flow speed of a walker, in metres per second, the same on every link: the build
    /// value `walk_speed` given to the builder (in km/h there).
    #[inline]
    #[must_use]
    pub fn walk_speed(&self) -> Speed {
        self.walk_speed
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

    /// What the build repaired or left out, by kind (`src/network/rows.rs`): each with its
    /// count and the ids of the first five rows. Not in the fingerprint.
    #[must_use]
    pub fn repairs(&self) -> &Repairs {
        &self.repairs
    }

    /// The fingerprint of the network as built (`openmobisim_types::fingerprint`), fed in this
    /// order: the projection (0, or 1 then its zone number and hemisphere, 0 north and 1 south);
    /// the bike and walk speeds (m/s); the node count, then per node in id order its external
    /// id, x and y (metres) and signal mark; the link count, then per link in id order its
    /// external id, upstream and downstream node ids, length (m), lanes, modes, car free-flow
    /// speed (m/s, `NaN` on a link without cars), capacity (veh/s, infinite on a link without
    /// cars) and downstream-end signal mark. Two builds of the same rows, in any order, have the
    /// same fingerprint; a change to any value a run reads changes it.
    #[must_use]
    pub fn fingerprint(&self) -> u64 {
        let mut h = Fingerprint::new();
        match self.projection {
            None => h.u8(0),
            Some(p) => {
                h.u8(1);
                h.u8(p.zone().number());
                h.u8(match p.zone().hemisphere() {
                    Hemisphere::North => 0,
                    Hemisphere::South => 1,
                });
            }
        }
        h.f64(self.bike_speed.get());
        h.f64(self.walk_speed.get());
        h.u32(self.node_count());
        for (i, id) in self.node_ids.iter().enumerate() {
            h.str(id);
            h.f64(self.node_x[i]);
            h.f64(self.node_y[i]);
            h.flag(self.node_signalised[i]);
        }
        h.u32(self.link_count());
        for (i, id) in self.link_ids.iter().enumerate() {
            h.str(id);
            h.u32(self.link_upstream[i].raw());
            h.u32(self.link_downstream[i].raw());
            h.f64(self.link_length[i].get());
            h.u8(self.link_lanes[i]);
            h.modes(self.link_modes[i]);
            h.f64(self.link_speed[speed_column(Mode::Car)][i].get());
            h.f64(self.link_capacity[i].get());
            h.flag(self.link_signal_downstream[i]);
        }
        h.finish()
    }
}
