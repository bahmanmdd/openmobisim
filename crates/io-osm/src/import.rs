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

use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
use openmobisim_core_graph::geometry::{LonLat, polyline_length_metres};
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
    /// The global multipliers handed to the defaults table.
    pub multipliers: GlobalMultipliers,
    /// The signal defaults handed to the defaults table.
    pub signals: SignalDefaults,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            contract: true,
            multipliers: GlobalMultipliers::default(),
            signals: SignalDefaults::SHIPPED,
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
            && match (self.maxspeed_km_h, next.maxspeed_km_h) {
                (Some(a), Some(b)) => (a - b).abs() < 1e-9,
                (None, None) => true,
                _ => false,
            }
    }
}

/// Import a road network from an OSM source.
///
/// Returns the network, the import report, and the [`LinkGeometry`] artifact
/// (S125) — the polyline of every street, kept separately from the network
/// rather than as one of its fields, so that nothing which never reads
/// geometry pays for it.
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
    let mut report = ImportReport::default();

    // --- 1. Ways ------------------------------------------------------------
    let mut kept_ways: Vec<OsmWay> = Vec::new();
    let mut reference_count: HashMap<i64, u32> = HashMap::new();

    source.for_each_way(&mut |way| {
        report.ways_seen += 1;
        match tags::classify(&way.tags, way.node_ids.len()) {
            Ok(_) => {
                for &n in &way.node_ids {
                    *reference_count.entry(n).or_insert(0) += 1;
                }
                kept_ways.push(way);
            }
            Err(reason) => record_rejection(diagnostics, reason),
        }
    })?;
    report.ways_kept = kept_ways.len() as u64;

    // --- 2. Nodes -----------------------------------------------------------
    let mut positions: HashMap<i64, LonLat> = HashMap::with_capacity(reference_count.len());
    let mut signalised: HashSet<i64> = HashSet::new();
    let mut forced_splits: HashSet<i64> = HashSet::new();

    source.for_each_node(&mut |node: OsmNode| {
        report.nodes_seen += 1;
        if !reference_count.contains_key(&node.id) {
            return;
        }
        positions.insert(node.id, LonLat::new(node.lon, node.lat));
        if tags::node_is_signalised(&node.tags) {
            signalised.insert(node.id);
        }
        if tags::node_splits_way(&node.tags) {
            forced_splits.insert(node.id);
        }
    })?;
    report.nodes_kept = positions.len() as u64;

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
    for way in &kept_ways {
        split_way(way, &positions, &is_junction, diagnostics, &mut links);
    }
    report.links_before_contraction = links.len() as u64;

    // --- 4. Contract --------------------------------------------------------
    let nodes_before = used_nodes(&links).len();
    if options.contract {
        links = contract(links, &signalised, &forced_splits);
    }
    report.links_after_contraction = links.len() as u64;
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
            },
        );
    }

    let network = builder
        .build(options.multipliers, options.signals, diagnostics)
        .map_err(|e| OsmError::Format(e.to_string()))?;

    let geometry = LinkGeometry::build(&network, &geometry_by_external_id);

    Ok((network, report, geometry))
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

/// Split one way into directed links between its junction nodes.
fn split_way(
    way: &OsmWay,
    positions: &HashMap<i64, LonLat>,
    is_junction: &HashSet<i64>,
    diagnostics: &mut Diagnostics,
    out: &mut Vec<ProtoLink>,
) {
    let Ok(class) = tags::classify(&way.tags, way.node_ids.len()) else { return };
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
    let maxspeed_km_h = speed.value_km_h();
    // Only `roundabout`: OSM's `circular` does not imply that entering traffic
    // gives way (S155).
    let roundabout = crate::source::tag_of(&way.tags, "junction") == Some("roundabout");

    // Drop nodes the extract does not contain — normal where a bounding box
    // cuts a way — and record it once per way rather than once per node.
    let mut present: Vec<i64> = Vec::with_capacity(way.node_ids.len());
    let mut missing = 0u32;
    for &n in &way.node_ids {
        if positions.contains_key(&n) {
            present.push(n);
        } else {
            missing += 1;
        }
    }
    if missing > 0 {
        diagnostics.record_n(
            DiagKey::run_level(Category::DataQuality, codes::MISSING_NODE, Severity::Info),
            u64::from(missing),
        );
    }
    if present.len() < 2 {
        diagnostics.record(DiagKey::run_level(
            Category::DataQuality,
            codes::WAY_LOST_TO_MISSING_NODES,
            Severity::Info,
        ));
        return;
    }

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

        // A segment that starts and ends at the same node is a closed loop with
        // no junction on it — a roundabout mapped as one way, say. It cannot
        // become a link, and splitting it further is the importer's job on the
        // next pass, not a special case here.
        if from != to {
            if direction.has_forward() {
                out.push(ProtoLink {
                    way_id: way.id,
                    from,
                    to,
                    class,
                    lanes: lane_counts.forward,
                    maxspeed_km_h,
                    roundabout,
                    length_m,
                    geometry: geometry.clone(),
                });
            }
            if direction.has_backward() {
                let mut backward_geometry = geometry.clone();
                backward_geometry.reverse();
                out.push(ProtoLink {
                    way_id: way.id,
                    from: to,
                    to: from,
                    class,
                    lanes: lane_counts.backward,
                    maxspeed_km_h,
                    roundabout,
                    length_m,
                    geometry: backward_geometry,
                });
            }
        }

        segment_start = i;
    }
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
/// * merging would not create a link from a node to itself.
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
            if signalised.contains(&node) || forced_splits.contains(&node) {
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
