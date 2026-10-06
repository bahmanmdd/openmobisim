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
use openmobisim_core_types::units::{Density, Duration, Flow, Metres, Pcu};

use crate::csr::Csr;
use crate::defaults::{
    DefaultRow, GlobalMultipliers, LinkParameters, ParameterNote, RoadClass, SignalDefaults,
};
use crate::geometry::{LonLat, Projected, Projection, ProjectionError};
use crate::network_options::{ClassTable, NetworkDefaults};

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
    /// The projection to use instead of one chosen from the extent (S195: a
    /// bike or walk layer shares the road network's).
    projection: Option<Projection>,
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

    /// Project in `projection` rather than in one chosen from the nodes'
    /// extent, so that this network shares another's coordinate system (S195).
    #[must_use]
    pub fn with_projection(mut self, projection: Projection) -> Self {
        self.projection = Some(projection);
        self
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
        let defaults = NetworkDefaults {
            classes: ClassTable::shipped(),
            multipliers,
            signals,
            ..NetworkDefaults::shipped()
        };
        self.build_with(&defaults, diagnostics)
    }

    /// [`Self::build`] with every network parameter given by name (S225): the road-class table,
    /// the multipliers, the signal settings and the layers' speeds ([`NetworkDefaults`]). The
    /// network keeps them ([`RoadNetwork::defaults`]).
    ///
    /// # Errors
    ///
    /// As [`Self::build`].
    ///
    /// # Panics
    ///
    /// As [`Self::build`].
    pub fn build_with(
        self,
        defaults: &NetworkDefaults,
        diagnostics: &mut Diagnostics,
    ) -> Result<RoadNetwork, ProjectionError> {
        let multipliers = defaults.multipliers;
        let signals = defaults.effective_signals();
        let node_ids: ExternalIdTable = self.node_ids.build();
        let node_count = node_ids.count();

        // One projection for the whole scenario, chosen from the extent unless
        // it was given.
        let projection = match self.projection {
            Some(p) => p,
            None => Projection::for_extent(self.nodes.iter().map(|(_, p)| *p))?,
        };

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

            let row: DefaultRow = defaults.classes.row(spec.class);
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
            link_closed: Vec::new(),
            defaults: *defaults,
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

    /// Links closed to motor traffic by a scenario edit (S238, [`Self::with_changes`]):
    /// empty when none is, else one flag per link. A closed link keeps its index and its
    /// data but is in no node's out- or in-links, so no search, turn or snap ever uses it.
    link_closed: Vec<bool>,

    /// What it was built with besides its data (S225).
    defaults: NetworkDefaults,
}

/// What a scenario changes in a road network (S238, roadmap I-az: scenario edits), by link
/// index. See [`RoadNetwork::with_changes`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LinkChanges {
    /// Links closed to motor traffic (cars and buses) for the whole run: a road closed for
    /// works or by an accident lasting the day. Walkers and cyclists keep their own layers.
    pub closed: Vec<LinkId>,
    /// Links whose lanes per direction change, with the new count: capacity, jam density and
    /// storage scale with the lanes (a lane closed, or one added).
    pub lanes: Vec<(LinkId, u8)>,
    /// Links whose capacity is multiplied by the factor, storage kept: a bottleneck (works
    /// beside the road, a narrowed junction). The backward wave speed is derived again from
    /// the triangle, with [`crate::defaults::LinkParameters::from_defaults`]' rule.
    pub capacity_factor: Vec<(LinkId, f64)>,
}

impl LinkChanges {
    /// Whether nothing changes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.closed.is_empty() && self.lanes.is_empty() && self.capacity_factor.is_empty()
    }
}

impl RoadNetwork {
    /// The parameters the network was built with besides its data (S225): the road-class table,
    /// the multipliers, the signal settings and the bike and walk layers' speeds.
    #[must_use]
    pub fn defaults(&self) -> &NetworkDefaults {
        &self.defaults
    }

    /// The signal settings the network was built with, the green-fraction multiplier applied:
    /// what its turn table's signal capacities are to use ([`crate::TurnTable::build`]).
    #[must_use]
    pub fn signals(&self) -> SignalDefaults {
        self.defaults.effective_signals()
    }

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

    /// Whether a link is closed to motor traffic by a scenario edit (S238).
    #[inline]
    #[must_use]
    pub fn is_closed(&self, link: LinkId) -> bool {
        self.link_closed.get(link.index()).copied().unwrap_or(false)
    }

    /// Whether a car can use a link: its class carries motor traffic and it is not closed.
    #[inline]
    #[must_use]
    pub fn is_drivable(&self, link: LinkId) -> bool {
        self.link_class(link).carries_motor_traffic() && !self.is_closed(link)
    }

    /// The same network with `changes` made (S238): closed links taken out of every node's
    /// links (their index and data kept, so per-link results line up with the original's),
    /// lanes and capacities changed. Changes add to those the network already has.
    ///
    /// # Errors
    ///
    /// A message for a link index out of range, a lane count of 0 (close the link instead) or
    /// a capacity factor that is not a finite number above 0.
    pub fn with_changes(&self, changes: &LinkChanges) -> Result<RoadNetwork, String> {
        let n = self.link_count() as usize;
        let check = |l: LinkId| {
            if l.index() < n {
                Ok(())
            } else {
                Err(format!("there is no link {} in a network of {n} links", l.index()))
            }
        };
        let mut params = self.link_params.clone();
        let mut lanes = self.link_lanes.clone();
        for &(l, new) in &changes.lanes {
            check(l)?;
            if new == 0 {
                return Err(format!("link {}: 0 lanes; close the link instead", l.index()));
            }
            let ratio = f64::from(new) / f64::from(lanes[l.index()].max(1));
            let p = &mut params[l.index()];
            p.capacity = Flow::from_veh_per_hour(p.capacity.as_veh_per_hour() * ratio);
            p.jam_density = Density::from_veh_per_km(p.jam_density.as_veh_per_km() * ratio);
            lanes[l.index()] = new;
        }
        for &(l, factor) in &changes.capacity_factor {
            check(l)?;
            if !(factor.is_finite() && factor > 0.0) {
                return Err(format!(
                    "link {}: a capacity factor is a finite number above 0, got {factor}; close the \
                     link instead of 0",
                    l.index()
                ));
            }
            params[l.index()] = params[l.index()].with_capacity_times(factor);
        }
        let mut closed = if self.link_closed.is_empty() && !changes.closed.is_empty() {
            vec![false; n]
        } else {
            self.link_closed.clone()
        };
        for &l in &changes.closed {
            check(l)?;
            closed[l.index()] = true;
        }
        let open = |i: u32| closed.get(i as usize).is_none_or(|c| !c);
        let link_count = self.link_count();
        let out_links = Csr::from_pairs(
            self.node_count(),
            (0..link_count)
                .filter(|&i| open(i))
                .map(|i| (self.link_from[i as usize], LinkId::new(i))),
        );
        let in_links = Csr::from_pairs(
            self.node_count(),
            (0..link_count)
                .filter(|&i| open(i))
                .map(|i| (self.link_to[i as usize], LinkId::new(i))),
        );
        let link_storage =
            params.iter().zip(&self.link_length).map(|(p, &l)| p.storage(l)).collect();
        Ok(RoadNetwork {
            projection: self.projection,
            node_ids: self.node_ids.clone(),
            link_ids: self.link_ids.clone(),
            node_x: self.node_x.clone(),
            node_y: self.node_y.clone(),
            node_lonlat: self.node_lonlat.clone(),
            node_signalised: self.node_signalised.clone(),
            link_from: self.link_from.clone(),
            link_to: self.link_to.clone(),
            link_class: self.link_class.clone(),
            link_roundabout: self.link_roundabout.clone(),
            link_lanes: lanes,
            link_length: self.link_length.clone(),
            link_free_flow_time: self.link_free_flow_time.clone(),
            link_params: params,
            link_storage,
            out_links,
            in_links,
            link_closed: closed,
            defaults: self.defaults,
        })
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
