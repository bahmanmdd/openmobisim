//! The road network handle Python holds, and how one is made.
//!
//! A [`PyNetwork`] is opaque for *running* (it is passed straight back into
//! [`crate::pipeline::run_pipeline`]) but it can be *read* as flat numpy
//! arrays — link geometry, lengths, free-flow times, classes — which is what
//! figures and any array-shaped analysis need (S163). Every array is indexed
//! by the link's internal id, the same index the per-link results use.

use std::sync::{Arc, OnceLock};

use numpy::{IntoPyArray, PyArray1, PyArray2, PyArrayMethods};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;

use openmobisim_core_graph::connectivity::{analyse, analyse_by};
use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
use openmobisim_core_graph::examples::{
    manhattan_grid as build_manhattan_grid, toy_network as build_toy_network,
};
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::link_geometry::LinkGeometry;
use openmobisim_core_graph::network::{
    self as network_mod, LinkSpec, RoadNetwork, RoadNetworkBuilder,
};
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_routes::NodeSnapper;
use openmobisim_core_types::diagnostics::{Category, Diagnostics, Severity};
use openmobisim_core_types::ids::{EntityId, LinkId, NodeId};
use openmobisim_io_osm::{
    ClippedSource, Connectivity, DropReason, DroppedLink, ImportOptions, ImportReport, PbfSource,
    Region, import_detailed,
};

/// A link polyline array and its per-link offsets, as numpy arrays.
type GeometryArrays<'py> = (Bound<'py, PyArray2<f64>>, Bound<'py, PyArray1<u32>>);

/// A road network. Made by `manhattan_grid`, `toy_network` or
/// `network_read_osm`; passed to a scenario; read as numpy arrays.
#[pyclass(name = "Network", module = "openmobisim._core")]
pub struct PyNetwork {
    pub(crate) inner: Arc<RoadNetwork>,
    /// Street shapes captured at import (S125); `None` for a network built
    /// from nodes alone, whose links are straight lines between their nodes.
    pub(crate) geometry: Option<Arc<LinkGeometry>>,
    /// Where the network came from, for attribution on figures:
    /// `"osm"` (an OpenStreetMap extract) or `"synthetic"`.
    #[pyo3(get)]
    pub(crate) source: String,
    /// The nearest-drivable-node index, built the first time it is asked for.
    pub(crate) snapper: OnceLock<NodeSnapper>,
    /// What the import did, for a network read from OSM (S172).
    import: Option<Arc<ImportInfo>>,
}

/// What an OSM import did, kept beside the network so that it can be looked at.
struct ImportInfo {
    report: ImportReport,
    /// `(code, severity, count)` for every diagnostic the import recorded.
    diagnostics: Vec<(String, String, u64)>,
    dropped: Vec<DroppedLink>,
    connectivity: &'static str,
    contract_drivable: bool,
    /// The region as the caller gave it, for the record.
    region: Option<String>,
}

impl PyNetwork {
    pub(crate) fn new(
        inner: Arc<RoadNetwork>,
        geometry: Option<Arc<LinkGeometry>>,
        source: &str,
    ) -> Self {
        Self { inner, geometry, source: source.to_string(), snapper: OnceLock::new(), import: None }
    }

    /// The snapper, built on first use.
    pub(crate) fn snapper(&self) -> &NodeSnapper {
        self.snapper.get_or_init(|| NodeSnapper::new(&self.inner))
    }
}

#[pymethods]
impl PyNetwork {
    /// The index of the drivable node nearest to `(lon, lat)`: the node a trip
    /// starting or ending there is routed from or to. Node indices key route
    /// sets.
    fn node_nearest(&self, lon: f64, lat: f64) -> u32 {
        self.snapper().nearest(&self.inner, LonLat::new(lon, lat)).raw()
    }

    /// The WGS84 `(lon, lat)` of the node with external id `name` — for
    /// building demand between named places (a synthetic network's node
    /// names are documented with it; an OSM network's are OSM node ids).
    ///
    /// # Errors
    ///
    /// `ValueError` if there is no such node.
    fn node_lonlat(&self, name: &str) -> PyResult<(f64, f64)> {
        let id = self
            .inner
            .node_external_ids()
            .typed_id_of(name)
            .ok_or_else(|| PyValueError::new_err(format!("no such node: {name:?}")))?;
        let p = self.inner.node_lonlat(id);
        Ok((p.lon, p.lat))
    }

    /// How many nodes.
    #[getter]
    fn node_count(&self) -> u32 {
        self.inner.node_count()
    }

    /// How many directed links.
    #[getter]
    fn link_count(&self) -> u32 {
        self.inner.link_count()
    }

    /// Every link's polyline, as `(coordinates, offsets)`.
    ///
    /// `coordinates` is an `(n_points, 2)` float64 array of WGS84
    /// `(lon, lat)`; link `i`'s points are
    /// `coordinates[offsets[i]:offsets[i + 1]]`, in the link's direction of
    /// travel. A link without stored street geometry is the straight line
    /// between its two nodes. `offsets` is a uint32 array of `link_count + 1`
    /// entries.
    fn link_geometry<'py>(&self, py: Python<'py>) -> PyResult<GeometryArrays<'py>> {
        let n = self.inner.link_count();
        let mut flat: Vec<f64> = Vec::new();
        let mut offsets: Vec<u32> = Vec::with_capacity(n as usize + 1);
        offsets.push(0);
        for raw in 0..n {
            let link = LinkId::new(raw);
            let stored = self
                .geometry
                .as_ref()
                .and_then(|g| g.points(link).filter(|_| g.has_geometry(link)));
            match stored {
                Some(points) => {
                    for p in points {
                        flat.extend([p.lon, p.lat]);
                    }
                }
                None => {
                    for node in [self.inner.link_from(link), self.inner.link_to(link)] {
                        let p = self.inner.node_lonlat(node);
                        flat.extend([p.lon, p.lat]);
                    }
                }
            }
            let end = u32::try_from(flat.len() / 2)
                .map_err(|_| PyValueError::new_err("more than 2^32 geometry points"))?;
            offsets.push(end);
        }
        let rows = flat.len() / 2;
        let coordinates = flat.into_pyarray(py).reshape([rows, 2])?;
        Ok((coordinates, offsets.into_pyarray(py)))
    }

    /// Every link's length, in metres (float64).
    fn link_length_m<'py>(&self, py: Python<'py>) -> Bound<'py, PyArray1<f64>> {
        (0..self.inner.link_count())
            .map(|i| self.inner.link_length(LinkId::new(i)).get())
            .collect::<Vec<_>>()
            .into_pyarray(py)
    }

    /// Every link's free-flow traversal time, in seconds, control delay
    /// included (float64).
    fn link_free_flow_s<'py>(&self, py: Python<'py>) -> Bound<'py, PyArray1<f64>> {
        self.inner.free_flow_times().iter().map(|d| d.get()).collect::<Vec<_>>().into_pyarray(py)
    }

    /// Every link's road class, as the class's number (uint8): the order of
    /// OSM's `highway` values in the defaults table, 0 = motorway.
    fn link_class<'py>(&self, py: Python<'py>) -> Bound<'py, PyArray1<u8>> {
        (0..self.inner.link_count())
            .map(|i| self.inner.link_class(LinkId::new(i)) as u8)
            .collect::<Vec<_>>()
            .into_pyarray(py)
    }

    /// Every link's lane count in its own direction (uint8).
    fn link_lanes<'py>(&self, py: Python<'py>) -> Bound<'py, PyArray1<u8>> {
        (0..self.inner.link_count())
            .map(|i| self.inner.link_lanes(LinkId::new(i)))
            .collect::<Vec<_>>()
            .into_pyarray(py)
    }

    /// Every link's capacity across all its lanes, in PCU per hour (float64):
    /// the most it can discharge, before any signal takes its share.
    fn link_capacity_pcu_h<'py>(&self, py: Python<'py>) -> Bound<'py, PyArray1<f64>> {
        (0..self.inner.link_count())
            .map(|i| self.inner.link_parameters(LinkId::new(i)).capacity.get() * 3600.0)
            .collect::<Vec<_>>()
            .into_pyarray(py)
    }

    /// Every link's storage capacity at jam density, in PCU (float64).
    fn link_storage_pcu<'py>(&self, py: Python<'py>) -> Bound<'py, PyArray1<f64>> {
        self.inner.storages().iter().map(|p| p.get()).collect::<Vec<_>>().into_pyarray(py)
    }

    /// Whether each link may be used by a car (bool): false for footways,
    /// steps, paths, tracks, cycleways and pedestrian streets.
    fn link_drivable<'py>(&self, py: Python<'py>) -> Bound<'py, PyArray1<bool>> {
        (0..self.inner.link_count())
            .map(|i| self.inner.link_class(LinkId::new(i)).carries_motor_traffic())
            .collect::<Vec<_>>()
            .into_pyarray(py)
    }

    /// Whether each link is part of a roundabout's circulating carriageway (bool).
    fn link_roundabout<'py>(&self, py: Python<'py>) -> Bound<'py, PyArray1<bool>> {
        (0..self.inner.link_count())
            .map(|i| self.inner.is_roundabout(LinkId::new(i)))
            .collect::<Vec<_>>()
            .into_pyarray(py)
    }

    /// Whether each node is signalised (bool), indexed by node index.
    fn node_signalised<'py>(&self, py: Python<'py>) -> Bound<'py, PyArray1<bool>> {
        (0..self.inner.node_count())
            .map(|i| self.inner.is_signalised(NodeId::new(i)))
            .collect::<Vec<_>>()
            .into_pyarray(py)
    }

    /// Every link's start node, as a node index (uint32).
    fn link_from<'py>(&self, py: Python<'py>) -> Bound<'py, PyArray1<u32>> {
        (0..self.inner.link_count())
            .map(|i| self.inner.link_from(LinkId::new(i)).raw())
            .collect::<Vec<_>>()
            .into_pyarray(py)
    }

    /// Every link's end node, as a node index (uint32).
    fn link_to<'py>(&self, py: Python<'py>) -> Bound<'py, PyArray1<u32>> {
        (0..self.inner.link_count())
            .map(|i| self.inner.link_to(LinkId::new(i)).raw())
            .collect::<Vec<_>>()
            .into_pyarray(py)
    }

    /// Whether the network is strongly connected, and how far from it if not.
    ///
    /// `mode` is `"car"` (the default: only the links a car may use, so a
    /// footway that joins two roads does not join them) or `"all"` (every
    /// link). Two tests, both linear in links: on the **node graph** (every
    /// node reaches every other: the verdict, `strongly_connected`) and on the
    /// **link graph through the legal turns** (informational: a two-way
    /// triangle is two components there, one for each way round). Returns a
    /// dict of counts.
    #[pyo3(signature = (mode="car"))]
    fn report_connectivity<'py>(
        &self,
        py: Python<'py>,
        mode: &str,
    ) -> PyResult<Bound<'py, PyDict>> {
        let turns = TurnTable::build(&self.inner, SignalDefaults::SHIPPED);
        let r = match mode {
            "car" => analyse_by(&self.inner, &turns, |l| {
                self.inner.link_class(l).carries_motor_traffic()
            }),
            "all" => analyse(&self.inner, &turns),
            other => {
                return Err(PyValueError::new_err(format!(
                    "mode must be \"car\" or \"all\", not {other:?}"
                )));
            }
        };
        let d = PyDict::new(py);
        d.set_item("strongly_connected", r.is_strongly_connected())?;
        d.set_item("nodes", r.nodes)?;
        d.set_item("links", r.links)?;
        d.set_item("node_components", r.node_components)?;
        d.set_item("node_largest", r.node_largest)?;
        d.set_item("links_in_node_largest", r.links_in_node_largest)?;
        d.set_item("link_components", r.link_components)?;
        d.set_item("link_largest", r.link_largest)?;
        d.set_item("sources", r.sources)?;
        d.set_item("sinks", r.sinks)?;
        d.set_item("node_component_sizes_top", r.node_component_sizes_top)?;
        d.set_item("turns", turns.len())?;
        d.set_item("u_turns", turns.u_turn_count())?;
        d.set_item("max_turns_per_node", turns.max_turns_per_node())?;
        Ok(d)
    }

    /// What the import did, for a network read from OSM (`None` otherwise): the
    /// counts (ways and nodes seen and kept, links before and after
    /// contraction, what connectivity removed), the options used, and every
    /// data-quality diagnostic with its count.
    fn report_import<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyDict>>> {
        let Some(info) = &self.import else { return Ok(None) };
        let r = info.report;
        let d = PyDict::new(py);
        d.set_item("ways_seen", r.ways_seen)?;
        d.set_item("ways_kept", r.ways_kept)?;
        d.set_item("nodes_seen", r.nodes_seen)?;
        d.set_item("nodes_kept", r.nodes_kept)?;
        d.set_item("junction_nodes", r.junction_nodes)?;
        d.set_item("links_before_contraction", r.links_before_contraction)?;
        d.set_item("links_after_contraction", r.links_after_contraction)?;
        d.set_item("nodes_contracted", r.nodes_contracted)?;
        d.set_item("closed_loops_dropped", r.closed_loops_dropped)?;
        d.set_item("components_before", r.components_before)?;
        d.set_item("links_disconnected", r.links_disconnected)?;
        d.set_item("nodes_disconnected", r.nodes_disconnected)?;
        d.set_item("length_disconnected_m", r.length_disconnected_m)?;
        d.set_item("drivable_links", r.drivable_links)?;
        d.set_item("drivable_length_m", r.drivable_length_m)?;
        d.set_item("drivable_maxspeed_links", r.drivable_maxspeed_links)?;
        d.set_item("drivable_maxspeed_length_m", r.drivable_maxspeed_length_m)?;
        d.set_item("drivable_lanes_links", r.drivable_lanes_links)?;
        d.set_item("drivable_lanes_length_m", r.drivable_lanes_length_m)?;
        d.set_item("connectivity", info.connectivity)?;
        d.set_item("contract_drivable", info.contract_drivable)?;
        d.set_item("region", info.region.clone())?;
        let diagnostics = PyDict::new(py);
        for (code, severity, count) in &info.diagnostics {
            let row = PyDict::new(py);
            row.set_item("severity", severity)?;
            row.set_item("count", count)?;
            diagnostics.set_item(code, row)?;
        }
        d.set_item("diagnostics", diagnostics)?;
        Ok(Some(d))
    }

    /// The roads the import left out, so that they can be looked at: a closed
    /// loop that could not be a link, or a road outside the largest strongly
    /// connected component. `None` for a network not read from OSM.
    ///
    /// Returns `(coordinates, offsets, way_ids, reasons, classes)`: like
    /// `link_geometry`, a `(n_points, 2)` array of WGS84 `(lon, lat)` and
    /// `offsets` into it, and for each dropped link its OSM way id (int64),
    /// reason (uint8: 0 = closed loop, 1 = not strongly connected) and road
    /// class (uint8, numbered as `link_class`).
    #[allow(clippy::type_complexity, reason = "four flat arrays, documented above")]
    fn report_dropped<'py>(
        &self,
        py: Python<'py>,
    ) -> PyResult<
        Option<(
            Bound<'py, PyArray2<f64>>,
            Bound<'py, PyArray1<u32>>,
            Bound<'py, PyArray1<i64>>,
            Bound<'py, PyArray1<u8>>,
            Bound<'py, PyArray1<u8>>,
        )>,
    > {
        let Some(info) = &self.import else { return Ok(None) };
        let mut flat: Vec<f64> = Vec::new();
        let mut offsets: Vec<u32> = vec![0];
        let mut ways: Vec<i64> = Vec::with_capacity(info.dropped.len());
        let mut reasons: Vec<u8> = Vec::with_capacity(info.dropped.len());
        let mut classes: Vec<u8> = Vec::with_capacity(info.dropped.len());
        for link in &info.dropped {
            for p in &link.geometry {
                flat.extend([p.lon, p.lat]);
            }
            offsets.push(
                u32::try_from(flat.len() / 2)
                    .map_err(|_| PyValueError::new_err("more than 2^32 geometry points"))?,
            );
            ways.push(link.way_id);
            classes.push(link.class as u8);
            reasons.push(match link.reason {
                DropReason::ClosedLoop => 0,
                DropReason::NotStronglyConnected => 1,
            });
        }
        let rows = flat.len() / 2;
        let coordinates = flat.into_pyarray(py).reshape([rows, 2])?;
        Ok(Some((
            coordinates,
            offsets.into_pyarray(py),
            ways.into_pyarray(py),
            reasons.into_pyarray(py),
            classes.into_pyarray(py),
        )))
    }

    fn __repr__(&self) -> String {
        format!(
            "Network({} nodes, {} links{})",
            self.inner.node_count(),
            self.inner.link_count(),
            if self.geometry.is_some() { ", with street geometry" } else { "" }
        )
    }
}

/// `manhattan_grid(n, block_metres, signals)` — the N3 fixture (S105),
/// matching `06_INTERFACE_V0.md` §3's `ms.examples.manhattan_grid`
/// parameter for parameter.
///
/// # Errors
///
/// `ValueError` if `n < 2` — a grid needs at least one edge to be a network.
#[pyfunction]
pub fn manhattan_grid(n: u32, block_metres: f64, signals: bool) -> PyResult<PyNetwork> {
    if n < 2 {
        return Err(PyValueError::new_err("manhattan_grid needs at least 2x2 nodes"));
    }
    let (network, diagnostics) = build_manhattan_grid(n, block_metres, signals);
    debug_assert!(diagnostics.is_empty(), "a clean synthetic grid produces no diagnostics");
    Ok(PyNetwork::new(Arc::new(network), None, "synthetic"))
}

/// The toy network's road part (I-m, S161): sixteen nodes and links on which
/// every loading number can be checked by hand.
#[pyfunction]
pub fn toy_network() -> PyNetwork {
    let (network, diagnostics) = build_toy_network();
    debug_assert!(diagnostics.is_empty(), "the toy network builds cleanly");
    PyNetwork::new(Arc::new(network), None, "synthetic")
}

/// Read a road network from an OpenStreetMap `.osm.pbf` extract, with the
/// defaults table's parameters and street geometry kept for maps.
///
/// `region` cuts a study area out of the extract — a rectangle
/// `(west, south, east, north)` or a polygon, a list of `(lon, lat)` vertices —
/// and `connectivity` is `"keep"` (everything that is a road) or `"strong"`
/// (only the largest strongly connected part). `contract_drivable` merges car
/// links across nodes that only footways touch (the crossings and sidewalk
/// joins of a city that maps its sidewalks), leaving the footways as they were.
///
/// # Errors
///
/// `ValueError` if the file cannot be read, the region or `connectivity` is not
/// valid, or the result holds no usable road network.
#[pyfunction]
#[pyo3(signature = (path, contract=true, region=None, connectivity="keep", contract_drivable=false))]
pub fn network_read_osm(
    py: Python<'_>,
    path: &str,
    contract: bool,
    region: Option<&Bound<'_, PyAny>>,
    connectivity: &str,
    contract_drivable: bool,
) -> PyResult<PyNetwork> {
    let connectivity_mode = match connectivity {
        "keep" => Connectivity::Keep,
        "strong" => Connectivity::Strong,
        other => {
            return Err(PyValueError::new_err(format!(
                "connectivity must be \"keep\" or \"strong\", not {other:?}"
            )));
        }
    };
    let region = region.map(parse_region).transpose()?;
    let region_text = region.as_ref().map(|r| format!("{r:?}"));
    let path = path.to_owned();
    let out = py
        .detach(move || {
            let options = ImportOptions {
                contract,
                contract_drivable,
                connectivity: connectivity_mode,
                ..ImportOptions::default()
            };
            let mut diagnostics = Diagnostics::new();
            let source = PbfSource::new(&path);
            let out = match &region {
                Some(r) => {
                    import_detailed(&ClippedSource::new(&source, r), options, &mut diagnostics)
                }
                None => import_detailed(&source, options, &mut diagnostics),
            }?;
            Ok::<_, openmobisim_io_osm::OsmError>((out, diagnostics))
        })
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    let (out, diagnostics) = out;
    let rows = diagnostics
        .rows()
        .into_iter()
        .map(|row| (row.key.code.0.to_string(), format!("{:?}", row.key.severity), row.count))
        .collect();
    let mut network = PyNetwork::new(Arc::new(out.network), Some(Arc::new(out.geometry)), "osm");
    network.import = Some(Arc::new(ImportInfo {
        report: out.report,
        diagnostics: rows,
        dropped: out.dropped,
        connectivity: match connectivity_mode {
            Connectivity::Keep => "keep",
            Connectivity::Strong => "strong",
        },
        contract_drivable,
        region: region_text,
    }));
    Ok(network)
}

/// Build a network from plain column arrays (I-y): one entry per node, one
/// per link — a two-way street is two rows, as `RoadNetworkBuilder::add_link`
/// documents. `openmobisim.network_read_table` does the file-or-rows
/// resolution, the TNTP/CSV parsing and every unit conversion in Python
/// (there is no reason for a second, Rust-side table parser); this function
/// does only what `RoadNetworkBuilder` already does for an OSM import, one
/// row at a time, so nothing about how a link becomes a fundamental diagram
/// is duplicated.
///
/// An optional link column (`lanes`, `length_m`, `capacity_veh_h`,
/// `free_flow_km_h`) is `None` for a row the table did not state; that link's
/// value then comes from `link_class`'s row of the defaults table, exactly as
/// an OSM way with no `lanes`/`maxspeed` tag does. An empty `link_class` value
/// also defaults, quietly (no row is required to state its class); a
/// non-empty one this table does not model falls back the same way an
/// unfamiliar OSM `highway` tag does, with a diagnostic.
///
/// # Errors
///
/// `ValueError` if an array's length disagrees with `node_ids`/`link_ids`, a
/// link names a `from`/`to` node that is not in `node_ids`, or the result
/// holds no usable network — the same failures
/// [`RoadNetworkBuilder::build`] can report for any other input.
#[pyfunction]
#[allow(clippy::too_many_arguments, reason = "one array per column of a plain link/node table")]
#[pyo3(signature = (
    node_ids, node_lon, node_lat, node_signalised,
    link_ids, link_from, link_to, link_class, link_lanes,
    link_length_m, link_capacity_veh_h, link_free_flow_km_h, link_signalised, link_roundabout,
))]
pub fn network_from_columns(
    node_ids: Vec<String>,
    node_lon: Vec<f64>,
    node_lat: Vec<f64>,
    node_signalised: Vec<bool>,
    link_ids: Vec<String>,
    link_from: Vec<String>,
    link_to: Vec<String>,
    link_class: Vec<String>,
    link_lanes: Vec<Option<u8>>,
    link_length_m: Vec<Option<f64>>,
    link_capacity_veh_h: Vec<Option<f64>>,
    link_free_flow_km_h: Vec<Option<f64>>,
    link_signalised: Vec<bool>,
    link_roundabout: Vec<bool>,
) -> PyResult<PyNetwork> {
    let nodes = node_ids.len();
    for (name, len) in [
        ("node_lon", node_lon.len()),
        ("node_lat", node_lat.len()),
        ("node_signalised", node_signalised.len()),
    ] {
        if len != nodes {
            return Err(PyValueError::new_err(format!(
                "{name} has {len} entries, node_ids has {nodes}"
            )));
        }
    }
    let links = link_ids.len();
    for (name, len) in [
        ("link_from", link_from.len()),
        ("link_to", link_to.len()),
        ("link_class", link_class.len()),
        ("link_lanes", link_lanes.len()),
        ("link_length_m", link_length_m.len()),
        ("link_capacity_veh_h", link_capacity_veh_h.len()),
        ("link_free_flow_km_h", link_free_flow_km_h.len()),
        ("link_signalised", link_signalised.len()),
        ("link_roundabout", link_roundabout.len()),
    ] {
        if len != links {
            return Err(PyValueError::new_err(format!(
                "{name} has {len} entries, link_ids has {links}"
            )));
        }
    }

    let mut builder = RoadNetworkBuilder::new();
    for i in 0..nodes {
        builder.add_node(node_ids[i].clone(), LonLat::new(node_lon[i], node_lat[i]));
        if node_signalised[i] {
            builder.mark_signalised(node_ids[i].clone());
        }
    }

    let mut diagnostics = Diagnostics::new();
    for i in 0..links {
        let class = if link_class[i].is_empty() {
            RoadClass::Unclassified
        } else {
            RoadClass::from_osm_highway(&link_class[i]).unwrap_or_else(|| {
                diagnostics.record_run_level(
                    Category::DataQuality,
                    network_mod::codes::UNKNOWN_HIGHWAY_CLASS,
                    Severity::Warning,
                );
                RoadClass::Unclassified
            })
        };
        let spec = LinkSpec {
            class,
            lanes: link_lanes[i],
            maxspeed_km_h: link_free_flow_km_h[i],
            signalised: link_signalised[i],
            length_m: link_length_m[i],
            roundabout: link_roundabout[i],
            capacity_veh_h: link_capacity_veh_h[i],
        };
        builder.add_link(link_ids[i].clone(), link_from[i].clone(), link_to[i].clone(), spec);
    }

    let network = builder
        .build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut diagnostics)
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    Ok(PyNetwork::new(Arc::new(network), None, "table"))
}

/// A rectangle `(west, south, east, north)` or a list of `(lon, lat)` vertices.
fn parse_region(value: &Bound<'_, PyAny>) -> PyResult<Region> {
    let made = if let Ok((w, s, e, n)) = value.extract::<(f64, f64, f64, f64)>() {
        Region::bbox(w, s, e, n)
    } else if let Ok(vertices) = value.extract::<Vec<(f64, f64)>>() {
        Region::polygon(vertices)
    } else {
        return Err(PyValueError::new_err(
            "region must be (west, south, east, north) or a list of (lon, lat) vertices",
        ));
    };
    made.map_err(|e| PyValueError::new_err(e.to_string()))
}

/// The WGS84 lon/lat of grid position `(row, col)` — grid-specific
/// introspection (not a general `Network` method: a network built any other
/// way has no row/col positions to look up), so that `python/openmobisim`'s
/// `examples.fixed_car_trips` can build demand between named grid positions
/// without duplicating `core_graph::examples::node_name`'s convention.
///
/// # Errors
///
/// `ValueError` if `(row, col)` is not a node `network` was built with.
#[pyfunction]
pub fn grid_node_lonlat(network: PyRef<'_, PyNetwork>, row: u32, col: u32) -> PyResult<(f64, f64)> {
    let name = openmobisim_core_graph::examples::node_name(row, col);
    let id = network
        .inner
        .node_external_ids()
        .typed_id_of(&name)
        .ok_or_else(|| PyValueError::new_err(format!("no such grid node: ({row}, {col})")))?;
    let point = network.inner.node_lonlat(id);
    Ok((point.lon, point.lat))
}
