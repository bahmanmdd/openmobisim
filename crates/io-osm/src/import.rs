//! Turning OSM elements into a road network.
//!
//! # The four steps
//!
//! 1. **Classify the ways.** One pass. Every way is either a road of a known
//!    class or a recorded rejection. While passing, count how many kept ways
//!    use each node.
//! 2. **Read the nodes that matter.** A second pass, keeping only nodes some
//!    kept way references. This is the step that decides whether importing a
//!    large city costs two gigabytes or twelve — an extract's nodes are mostly
//!    buildings and coastline.
//! 3. **Split at junctions.** A link runs from one junction to the next; the
//!    geometry in between becomes its length, measured on the ellipsoid rather
//!    than as a straight line, so a curved street is not shortened.
//! 4. **Contract.** OSM splits a single street into many ways whenever a name
//!    or a speed limit changes, which leaves degree-two nodes that are not
//!    junctions at all. Merging them is what `contract` does, and it typically removes
//!    a fifth to a third of the links — and rather more of the turns, since
//!    turns grow with the square of node degree.
//!
//! # Nothing here fails because of the data
//!
//! A way referencing a node the extract does not contain, a speed limit in a
//! format nobody anticipated, a way with one node: each has a documented
//! fallback and a diagnostic. The only errors returned are failures to *read*
//! the source at all.

use std::collections::{HashMap, HashSet};

use openmobisim_core_graph::connectivity::strong_components;
use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
use openmobisim_core_graph::geometry::{LonLat, Projection, polyline_length_metres};
use openmobisim_core_graph::layers::{
    BikeInfrastructure, StaticLayer, StaticLayerDefaults, StaticLink, StaticNetwork,
    StaticNetworkBuilder,
};
use openmobisim_core_graph::link_geometry::LinkGeometry;
use openmobisim_core_graph::network::{LinkSpec, RoadNetwork, RoadNetworkBuilder};
use openmobisim_core_types::diagnostics::{Category, DiagCode, DiagKey, Diagnostics, Severity};

use crate::source::{OsmError, OsmNode, OsmSource, OsmWay};
use crate::tags::{self, Maxspeed, Rejection};

/// Diagnostic codes this importer records.
pub mod codes {
    use openmobisim_core_types::diagnostics::DiagCode;

    /// A way carried a `highway` value the defaults table does not model.
    pub const UNKNOWN_HIGHWAY_CLASS: DiagCode = DiagCode("osm_unknown_highway_class");
    /// A way was excluded by `access=no` or `access=private`.
    pub const ACCESS_DENIED: DiagCode = DiagCode("osm_access_denied");
    /// A way was tagged `area=yes` and is not a linear road.
    pub const IS_AN_AREA: DiagCode = DiagCode("osm_is_an_area");
    /// A way had fewer than two nodes.
    pub const TOO_FEW_NODES: DiagCode = DiagCode("osm_too_few_nodes");
    /// A `maxspeed` tag could not be interpreted; the class default was used.
    pub const MAXSPEED_UNPARSED: DiagCode = DiagCode("osm_maxspeed_unparsed");
    /// A `maxspeed=none`; the class default was used.
    pub const MAXSPEED_UNLIMITED: DiagCode = DiagCode("osm_maxspeed_unlimited");
    /// A way referenced a node the extract does not contain — normal at the
    /// edge of a bounding-box extract, where ways are cut.
    pub const MISSING_NODE: DiagCode = DiagCode("osm_missing_node");
    /// A way was dropped because too few of its nodes survived.
    pub const WAY_LOST_TO_MISSING_NODES: DiagCode = DiagCode("osm_way_lost_to_missing_nodes");
    /// A way with missing nodes in its middle was imported as separate pieces
    /// rather than bridged across the gap (S172).
    pub const WAY_CUT_INTO_PIECES: DiagCode = DiagCode("osm_way_cut_into_pieces");
    /// A closed way with no junction but its own end could not become a link
    /// (it would start and end at one node) and was dropped (S172).
    pub const CLOSED_LOOP_DROPPED: DiagCode = DiagCode("osm_closed_loop_dropped");
}

/// What the importer does about roads the rest of the network cannot reach, or
/// that cannot reach it (S172).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Connectivity {
    /// Import everything that is a road. Trips between some pairs of nodes have
    /// no route, and the network says nothing about it; see
    /// [`openmobisim_core_graph::connectivity`] to find out.
    #[default]
    Keep,
    /// Keep only the **largest strongly connected component of the drivable
    /// network**: every node a car can use reaches every other. Whole links are
    /// removed, never edited: nothing is added, joined or reversed, and every
    /// link that remains is exactly the link the extract gave. What was removed
    /// is counted in the [`ImportReport`] and returned as [`DroppedLink`]s to
    /// look at.
    ///
    /// Only links a car may use take part (`RoadClass::carries_motor_traffic`):
    /// footways, steps, paths, cycleways and pedestrian streets are neither
    /// tested nor removed, since they belong to the walk and cycle layers and
    /// a footway joining two road fragments does not connect them for a car.
    Strong,
}

/// How the importer should behave.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ImportOptions {
    /// Merge chains of degree-two nodes whose links agree on every parameter.
    ///
    /// On by default. Turning it off keeps every OSM node as a network node,
    /// which is useful when debugging an import against the raw data and
    /// wasteful otherwise.
    pub contract: bool,
    /// Contract the drivable links as if the links a car may not use were not
    /// there (S172). Off by default.
    ///
    /// A footway that crosses a street shares a node with it, and so does every
    /// sidewalk that meets a driveway; the importer counts those as junctions
    /// and, wherever a way touches a node, will not merge the links either side
    /// of it. In a city that maps its sidewalks that splits a third of the car
    /// network into pieces a few metres long, at nodes where no driver has a
    /// choice. With this on, car links are merged across such nodes by the usual
    /// rule (same class, lanes and speed; no signal, stop or barrier; nothing a
    /// car could turn into), the node stays for the footway, and the footway
    /// links are exactly as they were. Only meaningful with `contract`.
    pub contract_drivable: bool,
    /// The global multipliers handed to the defaults table.
    pub multipliers: GlobalMultipliers,
    /// The signal defaults handed to the defaults table.
    pub signals: SignalDefaults,
    /// What to do about roads outside the main strongly connected component.
    /// [`Connectivity::Keep`] by default: a default import behaves as it always
    /// has.
    pub connectivity: Connectivity,
    /// Which static layers to build besides the road network (S195). None by
    /// default in Rust; the Python reader builds both.
    pub layers: LayerOptions,
}

/// Which bike and walk layers an import builds, and with which defaults
/// (S193, S195).
///
/// Each layer is its own graph, read from the same ways in the same pass: its
/// own junctions, its own strong-connectivity trim (under
/// [`Connectivity::Strong`]) and its own contraction (under
/// [`ImportOptions::contract`]), projected in the road network's projection.
/// **The road network is the same with or without them.**
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct LayerOptions {
    /// Build the bike layer ([`crate::tags::bike_way`]).
    pub bike: bool,
    /// Build the walk layer ([`crate::tags::walk_way`]).
    pub walk: bool,
    /// The speeds the layers' links get.
    pub defaults: StaticLayerDefaults,
}

impl LayerOptions {
    /// Both layers, with the shipped defaults.
    pub const BOTH: LayerOptions =
        LayerOptions { bike: true, walk: true, defaults: StaticLayerDefaults::SHIPPED };
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            contract: true,
            contract_drivable: false,
            multipliers: GlobalMultipliers::default(),
            signals: SignalDefaults::SHIPPED,
            connectivity: Connectivity::Keep,
            layers: LayerOptions::default(),
        }
    }
}

/// What an import produced, besides the network.
///
/// Reported in the manifest. The counts are the first thing to look at when a
/// network behaves unexpectedly: an extract whose ways are mostly rejected, or
/// whose contraction removed nothing, is usually not the extract the author
/// thought they had.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ImportReport {
    /// Ways seen in the source.
    pub ways_seen: u64,
    /// Ways kept as roads.
    pub ways_kept: u64,
    /// Nodes seen in the source.
    pub nodes_seen: u64,
    /// Nodes kept because a road uses them.
    pub nodes_kept: u64,
    /// Nodes that are junctions, way ends, or otherwise split a way.
    pub junction_nodes: u64,
    /// Directed links before contraction.
    pub links_before_contraction: u64,
    /// Directed links after contraction.
    pub links_after_contraction: u64,
    /// Nodes removed by contraction.
    pub nodes_contracted: u64,
    /// Closed ways with no junction but their own end, which cannot be a link
    /// and were dropped (S172).
    pub closed_loops_dropped: u64,
    /// Strongly connected components the split graph had, singletons included;
    /// `0` unless [`Connectivity::Strong`] was asked for.
    pub components_before: u64,
    /// Links removed for not being in the largest component (S172).
    pub links_disconnected: u64,
    /// Nodes removed with them.
    pub nodes_disconnected: u64,
    /// Their total length, in whole metres.
    pub length_disconnected_m: u64,
    /// Links a car may use, in the network handed to the graph builder.
    pub drivable_links: u64,
    /// Their total length, in whole metres.
    pub drivable_length_m: u64,
    /// How many of them carry an explicit `maxspeed` (the rest take the class
    /// default) — the coverage of the defaults table's speeds (S172).
    pub drivable_maxspeed_links: u64,
    /// Their length, in whole metres.
    pub drivable_maxspeed_length_m: u64,
    /// How many carry an explicit lane count (the rest take the class default).
    pub drivable_lanes_links: u64,
    /// Their length, in whole metres.
    pub drivable_lanes_length_m: u64,
}

impl ImportReport {
    /// The share of links contraction removed, in `[0, 1]`.
    #[must_use]
    pub fn contraction_ratio(&self) -> f64 {
        if self.links_before_contraction == 0 {
            return 0.0;
        }
        let removed = self.links_before_contraction - self.links_after_contraction;
        #[allow(clippy::cast_precision_loss, reason = "link counts are far below 2^53")]
        let ratio = removed as f64 / self.links_before_contraction as f64;
        ratio
    }
}

/// A directed link, before it is handed to the graph builder.
///
/// Kept separate from `LinkSpec` because contraction works on these, and it
/// needs the endpoints and the parameters that decide whether two links may
/// merge.
#[derive(Clone, Debug)]
struct ProtoLink {
    way_id: i64,
    from: i64,
    to: i64,
    class: RoadClass,
    lanes: Option<u8>,
    maxspeed_km_h: Option<f64>,
    /// Part of a roundabout's circulating carriageway (`junction=roundabout`).
    roundabout: bool,
    /// The bike infrastructure, on the bike layer ([`BikeInfrastructure::Mixed`]
    /// on the road network and the walk layer).
    infrastructure: BikeInfrastructure,
    /// The bike must be walked here, on the bike layer.
    dismount: bool,
    length_m: f64,
    /// This link's own polyline, `from` to `to` (S125). Carried alongside
    /// `length_m` — computed from the same points, at the same place, and
    /// never re-derived from the other — through splitting and contraction so
    /// the geometry artifact can be built once the network's dense ids exist.
    geometry: Vec<LonLat>,
}

impl ProtoLink {
    /// Whether two consecutive links describe the same street, so that merging
    /// them changes nothing a run can observe.
    ///
    /// Deliberately strict: a difference in class, lanes or speed limit is a
    /// difference in the fundamental diagram, and merging across it would
    /// silently pick one of the two.
    fn mergeable_with(&self, next: &Self) -> bool {
        self.class == next.class
            && self.lanes == next.lanes
            && self.roundabout == next.roundabout
            && self.infrastructure == next.infrastructure
            && self.dismount == next.dismount
            && match (self.maxspeed_km_h, next.maxspeed_km_h) {
                (Some(a), Some(b)) => (a - b).abs() < 1e-9,
                (None, None) => true,
                _ => false,
            }
    }
}

/// Why a road was left out of the network, for the ones that can be drawn.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DropReason {
    /// A closed way with no junction but its own end (S172).
    ClosedLoop,
    /// Not in the largest strongly connected component (S172).
    NotStronglyConnected,
}

/// A road the importer left out, with its shape, so that it can be looked at.
#[derive(Clone, Debug)]
pub struct DroppedLink {
    /// The OSM way it came from.
    pub way_id: i64,
    /// Why it was left out.
    pub reason: DropReason,
    /// Its road class, so that what was removed can be summed by kind.
    pub class: RoadClass,
    /// Its polyline, in the direction the link would have run.
    pub geometry: Vec<LonLat>,
}

/// Everything an import produced.
#[derive(Debug)]
pub struct ImportOutput {
    /// The network.
    pub network: RoadNetwork,
    /// The counts.
    pub report: ImportReport,
    /// The street shapes, kept beside the network (S125).
    pub geometry: LinkGeometry,
    /// Roads left out because of what they were (a loop) or where they were (cut
    /// off from the rest), one entry per directed link, in a fixed order.
    pub dropped: Vec<DroppedLink>,
    /// The bike layer, if [`LayerOptions::bike`] asked for it.
    pub bike: Option<LayerImport>,
    /// The walk layer, if [`LayerOptions::walk`] asked for it.
    pub walk: Option<LayerImport>,
}

/// One static layer an import built (S195).
#[derive(Debug)]
pub struct LayerImport {
    /// The layer's graph and speeds.
    pub network: StaticNetwork,
    /// Its links' shapes.
    pub geometry: LinkGeometry,
    /// Its counts.
    pub report: LayerReport,
    /// Its links left out, as for the road network.
    pub dropped: Vec<DroppedLink>,
}

/// What an import made of one static layer.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct LayerReport {
    /// Ways that became links of this layer.
    pub ways_kept: u64,
    /// Directed links before contraction (after the connectivity trim).
    pub links_before_contraction: u64,
    /// Directed links after contraction.
    pub links_after_contraction: u64,
    /// Strongly connected components before the trim (0 if there was none).
    pub components_before: u64,
    /// Directed links the trim removed.
    pub links_disconnected: u64,
    /// Total directed length, in whole metres.
    pub length_m: u64,
    /// Directed length on dedicated bike infrastructure, in whole metres.
    pub dedicated_length_m: u64,
    /// Links given the minimum length because they had none.
    pub degenerate_links: u64,
}

/// Import a road network from an OSM source.
///
/// Returns the network, the import report, and the [`LinkGeometry`] artifact
/// (S125) — the polyline of every street, kept separately from the network
/// rather than as one of its fields, so that nothing which never reads
/// geometry pays for it. [`import_detailed`] also returns the roads left out.
///
/// # Errors
///
/// [`OsmError`] if the source cannot be read, or the graph builder's
/// [`ProjectionError`](openmobisim_core_graph::geometry::ProjectionError) — as an
/// [`OsmError::Format`] — if the extract has no projectable coordinates.
/// Everything else is a diagnostic.
pub fn import(
    source: &dyn OsmSource,
    options: ImportOptions,
    diagnostics: &mut Diagnostics,
) -> Result<(RoadNetwork, ImportReport, LinkGeometry), OsmError> {
    let out = import_detailed(source, options, diagnostics)?;
    Ok((out.network, out.report, out.geometry))
}

/// [`import`], and the roads that were left out as well.
///
/// # Errors
///
/// As [`import`].
pub fn import_detailed(
    source: &dyn OsmSource,
    options: ImportOptions,
    diagnostics: &mut Diagnostics,
) -> Result<ImportOutput, OsmError> {
    let mut report = ImportReport::default();

    // --- 1. Ways ------------------------------------------------------------
    // The road network's ways and node counts are exactly what they are without
    // layers; the layers count their own (S195).
    let layers = options.layers;
    let mut kept_ways: Vec<OsmWay> = Vec::new();
    let mut layer_only_ways: Vec<OsmWay> = Vec::new();
    let mut reference_count: HashMap<i64, u32> = HashMap::new();
    let mut bike_references: HashMap<i64, u32> = HashMap::new();
    let mut walk_references: HashMap<i64, u32> = HashMap::new();

    source.for_each_way(&mut |way| {
        report.ways_seen += 1;
        let n = way.node_ids.len();
        let bike = layers.bike && tags::bike_way(&way.tags, n).is_some();
        let walk = layers.walk && tags::walk_way(&way.tags, n).is_some();
        for (on, counts) in [(bike, &mut bike_references), (walk, &mut walk_references)] {
            if on {
                for &node in &way.node_ids {
                    *counts.entry(node).or_insert(0) += 1;
                }
            }
        }
        match tags::classify(&way.tags, n) {
            Ok(_) => {
                for &n in &way.node_ids {
                    *reference_count.entry(n).or_insert(0) += 1;
                }
                kept_ways.push(way);
            }
            Err(reason) => {
                record_rejection(diagnostics, reason);
                if bike || walk {
                    layer_only_ways.push(way);
                }
            }
        }
    })?;
    report.ways_kept = kept_ways.len() as u64;

    // --- 2. Nodes -----------------------------------------------------------
    let mut positions: HashMap<i64, LonLat> = HashMap::with_capacity(reference_count.len());
    let mut signalised: HashSet<i64> = HashSet::new();
    let mut forced_splits: HashSet<i64> = HashSet::new();

    let mut road_nodes_kept = 0u64;
    source.for_each_node(&mut |node: OsmNode| {
        report.nodes_seen += 1;
        let on_road = reference_count.contains_key(&node.id);
        if !on_road
            && !bike_references.contains_key(&node.id)
            && !walk_references.contains_key(&node.id)
        {
            return;
        }
        road_nodes_kept += u64::from(on_road);
        positions.insert(node.id, LonLat::new(node.lon, node.lat));
        if tags::node_is_signalised(&node.tags) {
            signalised.insert(node.id);
        }
        if tags::node_splits_way(&node.tags) {
            forced_splits.insert(node.id);
        }
    })?;
    report.nodes_kept = road_nodes_kept;

    // --- 3. Split -----------------------------------------------------------
    let mut is_junction: HashSet<i64> = HashSet::new();
    for (&node, &count) in &reference_count {
        if count >= 2 || forced_splits.contains(&node) {
            is_junction.insert(node);
        }
    }
    for way in &kept_ways {
        // A way's own ends are always junctions, even if nothing else uses them.
        if let Some(&first) = way.node_ids.first() {
            is_junction.insert(first);
        }
        if let Some(&last) = way.node_ids.last() {
            is_junction.insert(last);
        }
    }
    report.junction_nodes = is_junction.len() as u64;

    let mut links: Vec<ProtoLink> = Vec::new();
    let mut dropped: Vec<DroppedLink> = Vec::new();
    for way in &kept_ways {
        let Some(spec) = road_spec(way, diagnostics) else { continue };
        split_way(way, &spec, &positions, &is_junction, diagnostics, &mut links, &mut dropped);
    }
    report.closed_loops_dropped = dropped.len() as u64;

    // --- 3b. Connectivity (optional) ----------------------------------------
    // Before contraction, so that what is dropped is whole split links with
    // their own geometry, and contraction then merges across the gaps that
    // dropping leaves.
    if options.connectivity == Connectivity::Strong {
        let nodes_before = used_nodes(&links).len();
        let (kept, out, components) =
            keep_largest_strong_component(links, |l| l.class.carries_motor_traffic());
        links = kept;
        report.components_before = components;
        report.links_disconnected = out.len() as u64;
        report.nodes_disconnected = (nodes_before - used_nodes(&links).len()) as u64;
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a total length in whole metres is far below 2^63"
        )]
        {
            report.length_disconnected_m =
                out.iter().map(|l| l.length_m).sum::<f64>().round() as u64;
        }
        dropped.extend(out.into_iter().map(|l| DroppedLink {
            way_id: l.way_id,
            reason: DropReason::NotStronglyConnected,
            class: l.class,
            geometry: l.geometry,
        }));
    }
    report.links_before_contraction = links.len() as u64;

    // --- 4. Contract --------------------------------------------------------
    let nodes_before = used_nodes(&links).len();
    if options.contract && options.contract_drivable {
        // Cars first, on their own; the rest keep every node a car link touches
        // as a junction, so what a pedestrian sees is unchanged.
        let (cars, rest): (Vec<ProtoLink>, Vec<ProtoLink>) =
            links.into_iter().partition(|l| l.class.carries_motor_traffic());
        let car_nodes: HashSet<i64> = cars.iter().flat_map(|l| [l.from, l.to]).collect();
        links = contract(cars, &signalised, &forced_splits, &HashSet::new());
        links.extend(contract(rest, &signalised, &forced_splits, &car_nodes));
    } else if options.contract {
        links = contract(links, &signalised, &forced_splits, &HashSet::new());
    }
    report.links_after_contraction = links.len() as u64;
    record_coverage(&links, &mut report);
    report.nodes_contracted = (nodes_before - used_nodes(&links).len()) as u64;

    // --- 5. Hand to the graph builder ---------------------------------------
    let mut builder = RoadNetworkBuilder::new();
    for node in used_nodes(&links) {
        let Some(&p) = positions.get(&node) else { continue };
        builder.add_node(node.to_string(), p);
        if signalised.contains(&node) {
            builder.mark_signalised(node.to_string());
        }
    }
    let mut geometry_by_external_id: HashMap<String, Vec<LonLat>> =
        HashMap::with_capacity(links.len());
    for (i, link) in links.iter().enumerate() {
        let external_id = format!("{}:{i:08}", link.way_id);
        geometry_by_external_id.insert(external_id.clone(), link.geometry.clone());
        builder.add_link(
            external_id,
            link.from.to_string(),
            link.to.to_string(),
            LinkSpec {
                class: link.class,
                lanes: link.lanes,
                maxspeed_km_h: link.maxspeed_km_h,
                signalised: signalised.contains(&link.to),
                length_m: Some(link.length_m),
                roundabout: link.roundabout,
                capacity_veh_h: None,
            },
        );
    }

    let network = builder
        .build(options.multipliers, options.signals, diagnostics)
        .map_err(|e| OsmError::Format(e.to_string()))?;

    let geometry = LinkGeometry::build(&network, &geometry_by_external_id);

    // --- 6. The static layers (S195) ----------------------------------------
    let context = LayerContext {
        ways: &kept_ways,
        more_ways: &layer_only_ways,
        positions: &positions,
        signalised: &signalised,
        forced_splits: &forced_splits,
        options,
        projection: network.projection(),
    };
    let bike = if layers.bike {
        Some(build_layer(StaticLayer::Bike, &bike_references, &context)?)
    } else {
        None
    };
    let walk = if layers.walk {
        Some(build_layer(StaticLayer::Walk, &walk_references, &context)?)
    } else {
        None
    };

    Ok(ImportOutput { network, report, geometry, dropped, bike, walk })
}

/// What every static layer is built from.
struct LayerContext<'a> {
    ways: &'a [OsmWay],
    more_ways: &'a [OsmWay],
    positions: &'a HashMap<i64, LonLat>,
    signalised: &'a HashSet<i64>,
    forced_splits: &'a HashSet<i64>,
    options: ImportOptions,
    projection: Projection,
}

/// The links a way makes on `layer`, if it is part of it.
fn layer_spec(layer: StaticLayer, way: &OsmWay) -> Option<WaySpec> {
    let n = way.node_ids.len();
    match layer {
        StaticLayer::Bike => {
            let bike = tags::bike_way(&way.tags, n)?;
            let attrs =
                |infrastructure| LinkAttrs { lanes: None, infrastructure, dismount: bike.dismount };
            Some(WaySpec {
                class: bike.class,
                maxspeed_km_h: tags::maxspeed(&way.tags).value_km_h(),
                roundabout: false,
                forward: bike.forward.map(attrs),
                backward: bike.backward.map(attrs),
            })
        }
        StaticLayer::Walk => {
            let class = tags::walk_way(&way.tags, n)?;
            let attrs = LinkAttrs {
                lanes: None,
                infrastructure: BikeInfrastructure::Mixed,
                dismount: false,
            };
            Some(WaySpec {
                class,
                maxspeed_km_h: None,
                roundabout: false,
                forward: Some(attrs),
                backward: Some(attrs),
            })
        }
    }
}

/// Build one static layer: its junctions, links, connectivity trim, contraction
/// and graph, from the ways its rules admit.
///
/// Data-quality conditions the road import already recorded for the same ways
/// (missing nodes, loops) are not recorded twice; the layer's own counts are in
/// its [`LayerReport`].
fn build_layer(
    layer: StaticLayer,
    references: &HashMap<i64, u32>,
    cx: &LayerContext<'_>,
) -> Result<LayerImport, OsmError> {
    let mut scratch = Diagnostics::new();
    let mut report = LayerReport::default();
    let specs: Vec<(&OsmWay, WaySpec)> = cx
        .ways
        .iter()
        .chain(cx.more_ways)
        .filter_map(|w| layer_spec(layer, w).map(|spec| (w, spec)))
        .collect();
    report.ways_kept = specs.len() as u64;

    let mut is_junction: HashSet<i64> = references
        .iter()
        .filter(|&(node, &count)| count >= 2 || cx.forced_splits.contains(node))
        .map(|(&node, _)| node)
        .collect();
    for (way, _) in &specs {
        is_junction.extend(way.node_ids.first().copied());
        is_junction.extend(way.node_ids.last().copied());
    }

    let mut links: Vec<ProtoLink> = Vec::new();
    let mut dropped: Vec<DroppedLink> = Vec::new();
    for (way, spec) in &specs {
        split_way(way, spec, cx.positions, &is_junction, &mut scratch, &mut links, &mut dropped);
    }
    if cx.options.connectivity == Connectivity::Strong {
        let (kept, out, components) = keep_largest_strong_component(links, |_| true);
        links = kept;
        report.components_before = components;
        report.links_disconnected = out.len() as u64;
        dropped.extend(out.into_iter().map(|l| DroppedLink {
            way_id: l.way_id,
            reason: DropReason::NotStronglyConnected,
            class: l.class,
            geometry: l.geometry,
        }));
    }
    report.links_before_contraction = links.len() as u64;
    if cx.options.contract {
        links = contract(links, cx.signalised, cx.forced_splits, &HashSet::new());
    }
    report.links_after_contraction = links.len() as u64;

    let defaults = cx.options.layers.defaults;
    let mut builder = StaticNetworkBuilder::new(layer);
    for node in used_nodes(&links) {
        let Some(&p) = cx.positions.get(&node) else { continue };
        builder.add_node(node.to_string(), p);
    }
    let mut geometry_by_external_id: HashMap<String, Vec<LonLat>> =
        HashMap::with_capacity(links.len());
    let (mut length, mut dedicated) = (0.0f64, 0.0f64);
    for (i, link) in links.iter().enumerate() {
        let external_id = format!("{}:{i:08}", link.way_id);
        geometry_by_external_id.insert(external_id.clone(), link.geometry.clone());
        let speed_km_h = match layer {
            StaticLayer::Bike => {
                defaults.bike_speed_km_h(link.infrastructure, link.dismount, link.maxspeed_km_h)
            }
            StaticLayer::Walk => defaults.walk_km_h,
        };
        length += link.length_m;
        if link.infrastructure.is_dedicated() {
            dedicated += link.length_m;
        }
        builder.add_link(
            external_id,
            link.from.to_string(),
            link.to.to_string(),
            StaticLink {
                class: link.class,
                speed_km_h,
                infrastructure: link.infrastructure,
                length_m: Some(link.length_m),
            },
        );
    }
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a total length in whole metres is far below 2^63"
    )]
    {
        report.length_m = length.round() as u64;
        report.dedicated_length_m = dedicated.round() as u64;
    }
    let mut build_diagnostics = Diagnostics::new();
    let network = builder
        .build(cx.projection, &mut build_diagnostics)
        .map_err(|e| OsmError::Format(e.to_string()))?;
    report.degenerate_links =
        build_diagnostics.count_of(openmobisim_core_graph::network::codes::DEGENERATE_LINK_LENGTH);
    let geometry = LinkGeometry::build(network.network(), &geometry_by_external_id);
    Ok(LayerImport { network, geometry, report, dropped })
}

/// Record a way rejection, except the overwhelmingly common "it is not a road".
fn record_rejection(diagnostics: &mut Diagnostics, reason: Rejection) {
    let code: DiagCode = match reason {
        // Most of an extract is buildings, boundaries and rivers. Counting them
        // would drown every other line of the report.
        Rejection::NotAHighway => return,
        Rejection::UnknownHighwayClass => codes::UNKNOWN_HIGHWAY_CLASS,
        Rejection::IsAnArea => codes::IS_AN_AREA,
        Rejection::AccessDenied => codes::ACCESS_DENIED,
        Rejection::TooFewNodes => codes::TOO_FEW_NODES,
    };
    diagnostics.record(DiagKey::run_level(Category::DataQuality, code, Severity::Info));
}

/// The links one way makes: its class and speed limit, and the attributes of
/// the link in each direction it has one.
#[derive(Clone, Copy, Debug)]
struct WaySpec {
    class: RoadClass,
    maxspeed_km_h: Option<f64>,
    roundabout: bool,
    forward: Option<LinkAttrs>,
    backward: Option<LinkAttrs>,
}

/// What differs between a way's two directions.
#[derive(Clone, Copy, Debug)]
struct LinkAttrs {
    lanes: Option<u8>,
    infrastructure: BikeInfrastructure,
    dismount: bool,
}

/// The links a way makes on the road network, recording a speed limit that
/// could not be read. `None` if the way is not a road.
fn road_spec(way: &OsmWay, diagnostics: &mut Diagnostics) -> Option<WaySpec> {
    let class = tags::classify(&way.tags, way.node_ids.len()).ok()?;
    let direction = tags::direction(&way.tags, class);
    let lane_counts = tags::lanes(&way.tags, direction);

    let speed = tags::maxspeed(&way.tags);
    if speed.is_noteworthy() {
        let code = match speed {
            Maxspeed::Unparsed => codes::MAXSPEED_UNPARSED,
            _ => codes::MAXSPEED_UNLIMITED,
        };
        diagnostics.record(DiagKey::run_level(Category::DataQuality, code, Severity::Info));
    }
    let attrs =
        |lanes| LinkAttrs { lanes, infrastructure: BikeInfrastructure::Mixed, dismount: false };
    Some(WaySpec {
        class,
        maxspeed_km_h: speed.value_km_h(),
        // Only `roundabout`: OSM's `circular` does not imply that entering
        // traffic gives way (S155).
        roundabout: crate::source::tag_of(&way.tags, "junction") == Some("roundabout"),
        forward: direction.has_forward().then(|| attrs(lane_counts.forward)),
        backward: direction.has_backward().then(|| attrs(lane_counts.backward)),
    })
}

/// Split one way into directed links between its junction nodes.
///
/// Nodes the extract does not contain — normal where a bounding box cuts a way,
/// and what a [`crate::region::ClippedSource`] produces on purpose — break the
/// way into **pieces**, each a run of consecutive nodes that are present. A
/// piece is split at its junctions like any way; nothing is bridged across a
/// gap, because a straight link through territory the extract does not hold is
/// a street that does not exist (S172; before, such a way was joined up).
fn split_way(
    way: &OsmWay,
    spec: &WaySpec,
    positions: &HashMap<i64, LonLat>,
    is_junction: &HashSet<i64>,
    diagnostics: &mut Diagnostics,
    out: &mut Vec<ProtoLink>,
    dropped: &mut Vec<DroppedLink>,
) {
    let WaySpec { class, maxspeed_km_h, roundabout, .. } = *spec;

    // Runs of consecutive nodes the extract contains; record the missing ones
    // once per way rather than once per node.
    let mut pieces: Vec<Vec<i64>> = Vec::new();
    let mut current: Vec<i64> = Vec::new();
    let mut missing = 0u32;
    for &n in &way.node_ids {
        if positions.contains_key(&n) {
            current.push(n);
        } else {
            missing += 1;
            if !current.is_empty() {
                pieces.push(std::mem::take(&mut current));
            }
        }
    }
    if !current.is_empty() {
        pieces.push(current);
    }
    if missing > 0 {
        diagnostics.record_n(
            DiagKey::run_level(Category::DataQuality, codes::MISSING_NODE, Severity::Info),
            u64::from(missing),
        );
    }
    let usable = pieces.iter().filter(|p| p.len() >= 2).count();
    if usable == 0 {
        diagnostics.record(DiagKey::run_level(
            Category::DataQuality,
            codes::WAY_LOST_TO_MISSING_NODES,
            Severity::Info,
        ));
        return;
    }
    if missing > 0 {
        diagnostics.record(DiagKey::run_level(
            Category::DataQuality,
            codes::WAY_CUT_INTO_PIECES,
            Severity::Info,
        ));
    }

    for present in pieces.iter().filter(|p| p.len() >= 2) {
        let mut segment_start = 0usize;
        for i in 1..present.len() {
            let at_end = i + 1 == present.len();
            if !is_junction.contains(&present[i]) && !at_end {
                continue;
            }

            let geometry: Vec<LonLat> =
                present[segment_start..=i].iter().map(|n| positions[n]).collect();
            let length_m = polyline_length_metres(&geometry);
            let (from, to) = (present[segment_start], present[i]);

            // A segment that starts and ends at the same node is a closed loop
            // with no junction on it but its own end. It cannot become a link
            // (a link from a node to itself is not a street anyone can route
            // through), so it is dropped — and recorded, with its shape, so that
            // dropping it is a thing that can be looked at (S172).
            if from == to {
                diagnostics.record(DiagKey::run_level(
                    Category::DataQuality,
                    codes::CLOSED_LOOP_DROPPED,
                    Severity::Info,
                ));
                dropped.push(DroppedLink {
                    way_id: way.id,
                    reason: DropReason::ClosedLoop,
                    class,
                    geometry,
                });
            } else {
                if let Some(attrs) = spec.forward {
                    out.push(ProtoLink {
                        way_id: way.id,
                        from,
                        to,
                        class,
                        lanes: attrs.lanes,
                        maxspeed_km_h,
                        roundabout,
                        infrastructure: attrs.infrastructure,
                        dismount: attrs.dismount,
                        length_m,
                        geometry: geometry.clone(),
                    });
                }
                if let Some(attrs) = spec.backward {
                    let mut backward_geometry = geometry;
                    backward_geometry.reverse();
                    out.push(ProtoLink {
                        way_id: way.id,
                        from: to,
                        to: from,
                        class,
                        lanes: attrs.lanes,
                        maxspeed_km_h,
                        roundabout,
                        infrastructure: attrs.infrastructure,
                        dismount: attrs.dismount,
                        length_m,
                        geometry: backward_geometry,
                    });
                }
            }

            segment_start = i;
        }
    }
}

/// Count the drivable links, and how many say their own speed and lane count.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a length in whole metres is far below 2^63"
)]
fn record_coverage(links: &[ProtoLink], report: &mut ImportReport) {
    let metres = |l: &ProtoLink| l.length_m.round() as u64;
    for l in links.iter().filter(|l| l.class.carries_motor_traffic()) {
        report.drivable_links += 1;
        report.drivable_length_m += metres(l);
        if l.maxspeed_km_h.is_some() {
            report.drivable_maxspeed_links += 1;
            report.drivable_maxspeed_length_m += metres(l);
        }
        if l.lanes.is_some() {
            report.drivable_lanes_links += 1;
            report.drivable_lanes_length_m += metres(l);
        }
    }
}

/// Keep only the drivable links inside the largest strongly connected component
/// of the drivable node graph; links a car may not use are kept untouched.
/// `drivable` says which links take part: the car's on the road network, all
/// of them on a static layer.
///
/// Returns the links kept (in their original order), the links removed (same
/// order), and how many components the drivable graph had. A link is kept when
/// both its ends are in the largest component — exactly the links a path
/// between two of its nodes can use, so the kept part is again strongly
/// connected, and one round is enough. Nothing else about a kept link changes.
///
/// The largest is the one with the most nodes; the first found on a tie,
/// which is a pure function of the sorted node ids, so the choice is
/// deterministic.
///
/// Cost: linear in links, and a few bytes per node and link while it runs.
fn keep_largest_strong_component(
    links: Vec<ProtoLink>,
    drivable: impl Fn(&ProtoLink) -> bool,
) -> (Vec<ProtoLink>, Vec<ProtoLink>, u64) {
    let drivable_links: Vec<&ProtoLink> = links.iter().filter(|l| drivable(l)).collect();

    // Dense ids for the nodes drivable links touch.
    let mut nodes: Vec<i64> = drivable_links.iter().flat_map(|l| [l.from, l.to]).collect();
    nodes.sort_unstable();
    nodes.dedup();
    let index = |n: i64| -> u32 {
        u32::try_from(nodes.binary_search(&n).expect("every link end is a used node"))
            .expect("node count fits u32")
    };

    let mut offsets = vec![0u32; nodes.len() + 1];
    for l in &drivable_links {
        offsets[index(l.from) as usize + 1] += 1;
    }
    for i in 0..nodes.len() {
        offsets[i + 1] += offsets[i];
    }
    let mut targets = vec![0u32; drivable_links.len()];
    let mut fill = offsets.clone();
    for l in &drivable_links {
        let f = index(l.from) as usize;
        targets[fill[f] as usize] = index(l.to);
        fill[f] += 1;
    }

    let components = strong_components(&offsets, &targets);
    let Some(largest) = components.largest() else {
        return (links, Vec::new(), 0);
    };
    let count = components.count() as u64;

    let (mut kept, mut out) = (Vec::with_capacity(links.len()), Vec::new());
    for l in links {
        let inside = !drivable(&l)
            || (components.component_of(index(l.from) as usize) == largest
                && components.component_of(index(l.to) as usize) == largest);
        if inside {
            kept.push(l);
        } else {
            out.push(l);
        }
    }
    (kept, out, count)
}

/// Every node some link touches.
fn used_nodes(links: &[ProtoLink]) -> Vec<i64> {
    let mut set: HashSet<i64> = HashSet::with_capacity(links.len());
    for l in links {
        set.insert(l.from);
        set.insert(l.to);
    }
    let mut v: Vec<i64> = set.into_iter().collect();
    // Sorted so the order handed to the graph builder is a pure function of the
    // input, not of hash iteration order (Foundations §1).
    v.sort_unstable();
    v
}

/// Merge chains of degree-two nodes whose links agree on every parameter.
///
/// # What is and is not contracted
///
/// A node is contracted only when **every** one of these holds:
///
/// * it is not signalised and carries no barrier or stop tag — those change
///   what happens at the node, which is the whole reason it is a node;
/// * it has exactly one incoming and one outgoing link (a one-way street), or
///   exactly two of each which pair up as reverses (a two-way street);
/// * the links to be merged agree on class, lanes and speed limit;
/// * merging would not create a link from a node to itself;
/// * it is not in `also_blocked` (nodes the caller needs to keep).
///
/// The last condition is what protects cul-de-sacs: contracting the far end of
/// a stub would turn `A → X → A` into `A → A`, which is not a street.
///
/// Lengths add, so the merged link is exactly as long as the chain it replaces.
#[must_use]
fn contract(
    links: Vec<ProtoLink>,
    signalised: &HashSet<i64>,
    forced_splits: &HashSet<i64>,
    also_blocked: &HashSet<i64>,
) -> Vec<ProtoLink> {
    let mut links = links;

    loop {
        let mut incoming: HashMap<i64, Vec<usize>> = HashMap::new();
        let mut outgoing: HashMap<i64, Vec<usize>> = HashMap::new();
        for (i, l) in links.iter().enumerate() {
            incoming.entry(l.to).or_default().push(i);
            outgoing.entry(l.from).or_default().push(i);
        }

        // Candidates in a deterministic order: hash iteration order must never
        // influence which contraction happens first, because the result would
        // then depend on it.
        let mut candidates: Vec<i64> = incoming.keys().copied().collect();
        candidates.sort_unstable();

        let mut removed = vec![false; links.len()];
        let mut merged: Vec<ProtoLink> = Vec::new();
        let mut any = false;

        for node in candidates {
            if signalised.contains(&node)
                || forced_splits.contains(&node)
                || also_blocked.contains(&node)
            {
                continue;
            }
            let (Some(ins), Some(outs)) = (incoming.get(&node), outgoing.get(&node)) else {
                continue;
            };
            if ins.len() != outs.len() || ins.is_empty() || ins.len() > 2 {
                continue;
            }
            if ins.iter().chain(outs).any(|&i| removed[i]) {
                continue;
            }

            // Pair each approach with the exit that is not its own reverse.
            let mut pairs: Vec<(usize, usize)> = Vec::with_capacity(ins.len());
            let mut ok = true;
            for &i in ins {
                let a = &links[i];
                let Some(&o) = outs.iter().find(|&&o| links[o].to != a.from) else {
                    ok = false;
                    break;
                };
                if pairs.iter().any(|&(_, already)| already == o) {
                    ok = false;
                    break;
                }
                let b = &links[o];
                if !a.mergeable_with(b) || a.from == b.to {
                    ok = false;
                    break;
                }
                pairs.push((i, o));
            }
            if !ok || pairs.len() != ins.len() {
                continue;
            }

            for (i, o) in pairs {
                let a = &links[i];
                let b = &links[o];
                debug_assert!(
                    a.geometry.last() == b.geometry.first(),
                    "contraction is merging links whose geometry does not share a point"
                );
                let mut geometry = a.geometry.clone();
                geometry.extend_from_slice(&b.geometry[1..]);
                merged.push(ProtoLink {
                    way_id: a.way_id,
                    from: a.from,
                    to: b.to,
                    class: a.class,
                    lanes: a.lanes,
                    maxspeed_km_h: a.maxspeed_km_h,
                    roundabout: a.roundabout,
                    infrastructure: a.infrastructure,
                    dismount: a.dismount,
                    length_m: a.length_m + b.length_m,
                    geometry,
                });
                removed[i] = true;
                removed[o] = true;
                any = true;
            }
        }

        if !any {
            return links;
        }

        let mut next: Vec<ProtoLink> = Vec::with_capacity(links.len());
        for (i, l) in links.into_iter().enumerate() {
            if !removed[i] {
                next.push(l);
            }
        }
        next.extend(merged);
        // Deterministic order, independent of the order contractions happened.
        next.sort_by(|a, b| {
            (a.from, a.to, a.way_id)
                .cmp(&(b.from, b.to, b.way_id))
                .then(a.length_m.total_cmp(&b.length_m))
        });
        links = next;
    }
}
