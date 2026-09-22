//! The road network: structure of arrays, built once, then immutable.
//!
//! # Structure of arrays, not array of structures
//!
//! The loading reads one attribute of every link in a pass — all the
//! capacities, then all the storages — and never reads one link's whole record.
//! A `Vec<Link>` would pull a cache line of eight unrelated fields for each
//! four-byte value wanted; parallel `Vec`s pull sixteen useful values per line
//! and vectorise. On a 50 000-link network at 48 steps that is the difference
//! between the loading being memory-bound and being arithmetic-bound.
//!
//! It is also what makes a disruption cheap: a scheduled capacity change writes
//! one new capacity array, and nothing else in the network is touched (§3e).
//!
//! # Identity is assigned here, deterministically
//!
//! [`RoadNetworkBuilder`] takes external ids — OSM node and way ids — and
//! assigns dense `u32` ids **by sorted external id** (Foundations §1). Two
//! builds of the same extract therefore produce the same ids, in the same
//! order, whatever order the file happened to list them in. Without that, every
//! cached artifact and every seeded run silently stops being comparable.

use openmobisim_core_types::diagnostics::{Category, DiagKey, Diagnostics, ElementRef, Severity};
use openmobisim_core_types::ids::{
    EntityId, ExternalIdTable, ExternalIdTableBuilder, LinkId, NodeId,
};
use openmobisim_core_types::units::{Duration, Metres, Pcu};

use crate::csr::Csr;
use crate::defaults::{
    DefaultRow, GlobalMultipliers, LinkParameters, ParameterNote, RoadClass, SignalDefaults,
    default_row,
};
use crate::geometry::{LonLat, Projected, Projection, ProjectionError};

/// Diagnostic codes this module can record.
pub mod codes {
    use openmobisim_core_types::diagnostics::DiagCode;

    /// An OSM `highway` value the defaults table does not model; the link was
    /// treated as `unclassified`.
    pub const UNKNOWN_HIGHWAY_CLASS: DiagCode = DiagCode("unknown_highway_class");
    /// A link's parameters implied no triangular diagram; capacity was reduced.
    pub const CAPACITY_REDUCED: DiagCode = DiagCode("capacity_reduced_to_fit_jam_density");
    /// A link's derived backward wave speed was clamped to its bound.
    pub const WAVE_SPEED_CLAMPED: DiagCode = DiagCode("wave_speed_clamped");
    /// A link referenced a node that was never declared; the link was dropped.
    pub const LINK_REFERENCES_UNKNOWN_NODE: DiagCode = DiagCode("link_references_unknown_node");
    /// A link had zero or negative length; it was given a minimum length.
    pub const DEGENERATE_LINK_LENGTH: DiagCode = DiagCode("degenerate_link_length");
    /// The study area spans far enough in longitude that projection distortion
    /// is material at its edges.
    pub const WIDE_PROJECTION_EXTENT: DiagCode = DiagCode("wide_projection_extent");
}

/// The shortest a link may be, in metres.
///
/// A zero-length link makes free-flow time zero and storage zero, which makes
/// the loading divide by zero and the route search loop. OSM contains them —
/// duplicate nodes, mapping errors — so they are given this length and
/// recorded, rather than rejected. The value is well below the resolution of
/// anything the model claims to represent.
pub const MIN_LINK_LENGTH_M: f64 = 0.5;

/// Degrees from the central meridian past which distortion is worth reporting.
///
/// At 4.5° the scale error is about 0.15 %, which is still far below network
/// geometry error — but a study area this wide usually means the extract is
/// bigger than the author intended.
pub const WIDE_EXTENT_DEGREES: f64 = 4.5;

/// What a caller knows about a link before the defaults table is applied.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LinkSpec {
    /// The OSM road class.
    pub class: RoadClass,
    /// Lanes in this direction, if OSM says.
    pub lanes: Option<u8>,
    /// `maxspeed` in km/h, if OSM says.
    pub maxspeed_km_h: Option<f64>,
    /// Whether the link's downstream node is signalised.
    pub signalised: bool,
    /// Length in metres, if the caller measured it along the way geometry.
    ///
    /// `None` means "straight line between the end nodes", which is right for a
    /// network that has already been split at every geometry point and wrong
    /// for one that has not — so the importer passes the measured value.
    pub length_m: Option<f64>,
    /// Whether the link is part of a roundabout's circulating carriageway
    /// (OSM `junction=roundabout`): traffic entering it gives way to traffic
    /// already circulating (S155).
    pub roundabout: bool,
    /// The link's capacity across all its lanes, in vehicles per hour, if the
    /// caller already knows it (I-y: a link table that states capacity
    /// directly, rather than through `highway` × `lanes`). `None` uses the
    /// class row's `saturation_flow_veh_h_lane × lanes`, as OSM does.
    pub capacity_veh_h: Option<f64>,
}

impl LinkSpec {
    /// A link of `class` with everything else defaulted.
    #[must_use]
    pub const fn new(class: RoadClass) -> Self {
        Self {
            class,
            lanes: None,
            maxspeed_km_h: None,
            signalised: false,
            length_m: None,
            roundabout: false,
            capacity_veh_h: None,
        }
    }
}

/// Builds a [`RoadNetwork`] from external ids.
///
/// Nodes and links may arrive in any order; ids are assigned by sorted external
/// id when [`build`](Self::build) runs.
#[derive(Debug, Default)]
pub struct RoadNetworkBuilder {
    node_ids: ExternalIdTableBuilder,
    nodes: Vec<(String, LonLat)>,
    signalised: Vec<String>,
    links: Vec<(String, String, String, LinkSpec)>,
}

impl RoadNetworkBuilder {
    /// An empty builder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Declare a node at a WGS84 coordinate.
    ///
    /// Declaring the same external id twice keeps the first coordinate.
    pub fn add_node(&mut self, external_id: impl Into<String>, at: LonLat) {
        let id = external_id.into();
        self.node_ids.insert(id.clone());
        self.nodes.push((id, at));
    }

    /// Mark a node as signalised, which gives every approach to it a control
    /// delay and every turn through it a green-time fraction (S90).
    pub fn mark_signalised(&mut self, external_id: impl Into<String>) {
        self.signalised.push(external_id.into());
    }

    /// Declare a directed link. A two-way street is two links.
    pub fn add_link(
        &mut self,
        external_id: impl Into<String>,
        from: impl Into<String>,
        to: impl Into<String>,
        spec: LinkSpec,
    ) {
        self.links.push((external_id.into(), from.into(), to.into(), spec));
    }

    /// Assign ids, project, apply the defaults table and freeze.
    ///
    /// Everything that can go wrong with the *data* is resolved by a documented
    /// rule and recorded in `diagnostics`; the only failures returned are ones
    /// that make the scenario meaningless — an empty network, or coordinates
    /// that cannot be projected at all.
    ///
    /// # Errors
    ///
    /// [`ProjectionError`] if no node is projectable or the extent is empty.
    ///
    /// # Panics
    ///
    /// Panics if the network exceeds the `u32` id space — more than about four
    /// billion links. That is a bug in whatever produced the input, not a data
    /// condition, so it raises (§3c).
    pub fn build(
        self,
        multipliers: GlobalMultipliers,
        signals: SignalDefaults,
        diagnostics: &mut Diagnostics,
    ) -> Result<RoadNetwork, ProjectionError> {
        let node_ids: ExternalIdTable = self.node_ids.build();
        let node_count = node_ids.count();

        // One projection for the whole scenario, chosen from the extent.
        let projection = Projection::for_extent(self.nodes.iter().map(|(_, p)| *p))?;

        let mut node_x = vec![0.0f64; node_count as usize];
        let mut node_y = vec![0.0f64; node_count as usize];
        let mut node_lonlat = vec![LonLat::default(); node_count as usize];
        let mut widest = 0.0f64;
        for (external, p) in &self.nodes {
            let Some(id) = node_ids.id_of(external) else { continue };
            let xy = projection.project(*p);
            node_x[id as usize] = xy.x;
            node_y[id as usize] = xy.y;
            node_lonlat[id as usize] = *p;
            widest = widest.max(projection.zone().degrees_from_central_meridian(p.lon));
        }
        if widest > WIDE_EXTENT_DEGREES {
            diagnostics.record_run_level(
                Category::DataQuality,
                codes::WIDE_PROJECTION_EXTENT,
                Severity::Warning,
            );
        }

        let mut node_signalised = vec![false; node_count as usize];
        for external in &self.signalised {
            if let Some(id) = node_ids.id_of(external) {
                node_signalised[id as usize] = true;
            }
        }

        // Links, in the order their external ids sort — the same determinism
        // rule as nodes, so link ids are reproducible too.
        let mut link_order: Vec<usize> = (0..self.links.len()).collect();
        link_order.sort_unstable_by(|&a, &b| self.links[a].0.cmp(&self.links[b].0));

        let mut link_ids = ExternalIdTableBuilder::with_capacity(self.links.len());
        let mut link_from = Vec::with_capacity(self.links.len());
        let mut link_to = Vec::with_capacity(self.links.len());
        let mut link_class = Vec::with_capacity(self.links.len());
        let mut link_roundabout = Vec::with_capacity(self.links.len());
        let mut link_lanes = Vec::with_capacity(self.links.len());
        let mut link_length = Vec::with_capacity(self.links.len());
        let mut link_params: Vec<LinkParameters> = Vec::with_capacity(self.links.len());

        for &i in &link_order {
            let (external, from, to, spec) = &self.links[i];
            let (Some(from_id), Some(to_id)) = (node_ids.id_of(from), node_ids.id_of(to)) else {
                diagnostics.record(DiagKey::run_level(
                    Category::DataQuality,
                    codes::LINK_REFERENCES_UNKNOWN_NODE,
                    Severity::Warning,
                ));
                continue;
            };

            let from_node = NodeId::new(from_id);
            let to_node = NodeId::new(to_id);

            let straight = Projected::new(node_x[from_id as usize], node_y[from_id as usize])
                .distance_to(Projected::new(node_x[to_id as usize], node_y[to_id as usize]));
            let mut length = spec.length_m.unwrap_or(straight);
            let degenerate = !(length.is_finite() && length >= MIN_LINK_LENGTH_M);
            if degenerate {
                length = MIN_LINK_LENGTH_M;
            }

            let row: DefaultRow = default_row(spec.class);
            let lanes = spec.lanes.unwrap_or(row.lanes_per_direction);
            let (params, note) = LinkParameters::from_defaults(
                row,
                lanes,
                spec.signalised || node_signalised[to_id as usize],
                signals,
                multipliers,
                spec.maxspeed_km_h,
                spec.capacity_veh_h,
            );

            let new_id = u32::try_from(link_from.len()).expect("link count fits u32");
            let element = ElementRef::new(openmobisim_core_types::ids::EntityKind::Link, new_id);
            if degenerate {
                diagnostics.record(DiagKey::new(
                    Category::DataQuality,
                    codes::DEGENERATE_LINK_LENGTH,
                    Severity::Warning,
                    element,
                ));
            }
            match note {
                ParameterNote::Consistent => {}
                ParameterNote::CapacityReducedToFitJamDensity => diagnostics.record(DiagKey::new(
                    Category::DataQuality,
                    codes::CAPACITY_REDUCED,
                    Severity::Warning,
                    element,
                )),
                ParameterNote::WaveSpeedClamped => diagnostics.record(DiagKey::new(
                    Category::DataQuality,
                    codes::WAVE_SPEED_CLAMPED,
                    Severity::Info,
                    element,
                )),
            }

            link_ids.insert(external.clone());
            link_from.push(from_node);
            link_to.push(to_node);
            link_class.push(spec.class);
            link_roundabout.push(spec.roundabout);
            link_lanes.push(lanes);
            link_length.push(Metres(length));
            link_params.push(params);
        }

        let link_count = u32::try_from(link_from.len()).expect("link count fits u32");

        let link_free_flow_time: Vec<Duration> =
            link_params.iter().zip(&link_length).map(|(p, &l)| p.free_flow_time(l)).collect();
        let link_storage: Vec<Pcu> =
            link_params.iter().zip(&link_length).map(|(p, &l)| p.storage(l)).collect();

        let out_links = Csr::from_pairs(
            node_count,
            (0..link_count).map(|i| (link_from[i as usize], LinkId::new(i))),
        );
        let in_links = Csr::from_pairs(
            node_count,
            (0..link_count).map(|i| (link_to[i as usize], LinkId::new(i))),
        );

        Ok(RoadNetwork {
            projection,
            node_ids,
            link_ids: link_ids.build(),
            node_x,
            node_y,
            node_lonlat,
            node_signalised,
            link_from,
            link_to,
            link_class,
            link_roundabout,
            link_lanes,
            link_length,
            link_params,
            link_free_flow_time,
            link_storage,
            out_links,
            in_links,
        })
    }
}

/// A built road network. Immutable, shared by every run in a batch.
#[derive(Debug)]
pub struct RoadNetwork {
    projection: Projection,
    node_ids: ExternalIdTable,
    link_ids: ExternalIdTable,

    node_x: Vec<f64>,
    node_y: Vec<f64>,
    node_lonlat: Vec<LonLat>,
    node_signalised: Vec<bool>,

    link_from: Vec<NodeId>,
    link_to: Vec<NodeId>,
    link_class: Vec<RoadClass>,
    link_roundabout: Vec<bool>,
    link_lanes: Vec<u8>,
    link_length: Vec<Metres>,
    link_params: Vec<LinkParameters>,
    link_free_flow_time: Vec<Duration>,
    link_storage: Vec<Pcu>,

    out_links: Csr<NodeId, LinkId>,
    in_links: Csr<NodeId, LinkId>,
}

impl RoadNetwork {
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

    /// The projection every coordinate here is in.
    #[inline]
    #[must_use]
    pub fn projection(&self) -> Projection {
        self.projection
    }

    /// A node's position in projected metres.
    #[inline]
    #[must_use]
    pub fn node_position(&self, node: NodeId) -> Projected {
        Projected::new(self.node_x[node.index()], self.node_y[node.index()])
    }

    /// A node's original WGS84 coordinate, for outputs.
    #[inline]
    #[must_use]
    pub fn node_lonlat(&self, node: NodeId) -> LonLat {
        self.node_lonlat[node.index()]
    }

    /// Whether a node is signalised.
    #[inline]
    #[must_use]
    pub fn is_signalised(&self, node: NodeId) -> bool {
        self.node_signalised[node.index()]
    }

    /// A link's upstream node.
    #[inline]
    #[must_use]
    pub fn link_from(&self, link: LinkId) -> NodeId {
        self.link_from[link.index()]
    }

    /// A link's downstream node.
    #[inline]
    #[must_use]
    pub fn link_to(&self, link: LinkId) -> NodeId {
        self.link_to[link.index()]
    }

    /// A link's road class.
    #[inline]
    #[must_use]
    pub fn link_class(&self, link: LinkId) -> RoadClass {
        self.link_class[link.index()]
    }

    /// Whether a link is part of a roundabout's circulating carriageway (S155).
    #[inline]
    #[must_use]
    pub fn is_roundabout(&self, link: LinkId) -> bool {
        self.link_roundabout[link.index()]
    }

    /// A link's lane count in its own direction.
    #[inline]
    #[must_use]
    pub fn link_lanes(&self, link: LinkId) -> u8 {
        self.link_lanes[link.index()]
    }

    /// A link's length.
    #[inline]
    #[must_use]
    pub fn link_length(&self, link: LinkId) -> Metres {
        self.link_length[link.index()]
    }

    /// A link's fundamental-diagram parameters.
    #[inline]
    #[must_use]
    pub fn link_parameters(&self, link: LinkId) -> LinkParameters {
        self.link_params[link.index()]
    }

    /// A link's free-flow traversal time, including the control delay.
    #[inline]
    #[must_use]
    pub fn free_flow_time(&self, link: LinkId) -> Duration {
        self.link_free_flow_time[link.index()]
    }

    /// How many vehicles a link holds at jam density.
    #[inline]
    #[must_use]
    pub fn storage(&self, link: LinkId) -> Pcu {
        self.link_storage[link.index()]
    }

    /// The links leaving a node.
    #[inline]
    #[must_use]
    pub fn out_links(&self, node: NodeId) -> &[LinkId] {
        self.out_links.targets(node)
    }

    /// The links entering a node.
    #[inline]
    #[must_use]
    pub fn in_links(&self, node: NodeId) -> &[LinkId] {
        self.in_links.targets(node)
    }

    /// Whether `b` is the reverse of `a` — the same street, the other way.
    #[inline]
    #[must_use]
    pub fn is_reverse_of(&self, a: LinkId, b: LinkId) -> bool {
        self.link_from(b) == self.link_to(a) && self.link_to(b) == self.link_from(a)
    }

    /// The external id table for nodes, for outputs.
    #[inline]
    #[must_use]
    pub fn node_external_ids(&self) -> &ExternalIdTable {
        &self.node_ids
    }

    /// The external id table for links, for outputs.
    #[inline]
    #[must_use]
    pub fn link_external_ids(&self) -> &ExternalIdTable {
        &self.link_ids
    }

    /// Whole arrays, for the loading's vectorised passes.
    ///
    /// Handing out slices rather than making callers loop over accessors is the
    /// point of the structure-of-arrays layout; a per-element accessor in a
    /// per-step loop would give all of the layout's cost and none of its
    /// benefit.
    #[inline]
    #[must_use]
    pub fn link_lengths(&self) -> &[Metres] {
        &self.link_length
    }

    /// Every link's free-flow traversal time.
    #[inline]
    #[must_use]
    pub fn free_flow_times(&self) -> &[Duration] {
        &self.link_free_flow_time
    }

    /// Every link's storage capacity.
    #[inline]
    #[must_use]
    pub fn storages(&self) -> &[Pcu] {
        &self.link_storage
    }

    /// Every link's fundamental-diagram parameters.
    #[inline]
    #[must_use]
    pub fn link_parameter_slice(&self) -> &[LinkParameters] {
        &self.link_params
    }

    /// Approximate bytes held, for the manifest's memory figures.
    #[must_use]
    pub fn bytes(&self) -> usize {
        let n = self.node_count() as usize;
        let l = self.link_count() as usize;
        n * (2 * size_of::<f64>() + size_of::<LonLat>() + 1)
            + l * (2 * size_of::<NodeId>()
                + size_of::<RoadClass>()
                + 2
                + size_of::<Metres>()
                + size_of::<LinkParameters>()
                + size_of::<Duration>()
                + size_of::<Pcu>())
            + self.out_links.bytes()
            + self.in_links.bytes()
            + self.node_ids.arena_bytes()
            + self.link_ids.arena_bytes()
    }
}
