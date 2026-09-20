//! The road network handle Python holds, and how one is made.
//!
//! A [`PyNetwork`] is opaque for *running* (it is passed straight back into
//! [`crate::pipeline::run_pipeline`]) but it can be *read* as flat numpy
//! arrays — link geometry, lengths, free-flow times, classes — which is what
//! figures and any array-shaped analysis need (S163). Every array is indexed
//! by the link's internal id, the same index the per-link results use.

use std::sync::Arc;

use numpy::{IntoPyArray, PyArray1, PyArray2, PyArrayMethods};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use openmobisim_core_graph::examples::{
    manhattan_grid as build_manhattan_grid, toy_network as build_toy_network,
};
use openmobisim_core_graph::link_geometry::LinkGeometry;
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::{EntityId, LinkId};
use openmobisim_io_osm::{ImportOptions, PbfSource, import};

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
}

#[pymethods]
impl PyNetwork {
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

    /// Every link's storage capacity at jam density, in PCU (float64).
    fn link_storage_pcu<'py>(&self, py: Python<'py>) -> Bound<'py, PyArray1<f64>> {
        self.inner.storages().iter().map(|p| p.get()).collect::<Vec<_>>().into_pyarray(py)
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
    Ok(PyNetwork { inner: Arc::new(network), geometry: None, source: "synthetic".into() })
}

/// The toy network's road part (I-m, S161): sixteen nodes and links on which
/// every loading number can be checked by hand.
#[pyfunction]
pub fn toy_network() -> PyNetwork {
    let (network, diagnostics) = build_toy_network();
    debug_assert!(diagnostics.is_empty(), "the toy network builds cleanly");
    PyNetwork { inner: Arc::new(network), geometry: None, source: "synthetic".into() }
}

/// Read a road network from an OpenStreetMap `.osm.pbf` extract, with the
/// defaults table's parameters and street geometry kept for maps.
///
/// # Errors
///
/// `ValueError` if the file cannot be read or holds no usable road network.
#[pyfunction]
#[pyo3(signature = (path, contract=true))]
pub fn network_read_osm(py: Python<'_>, path: &str, contract: bool) -> PyResult<PyNetwork> {
    let path = path.to_owned();
    let (network, _, geometry) = py
        .detach(move || {
            let options = ImportOptions { contract, ..ImportOptions::default() };
            import(&PbfSource::new(&path), options, &mut Diagnostics::new())
        })
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    Ok(PyNetwork {
        inner: Arc::new(network),
        geometry: Some(Arc::new(geometry)),
        source: "osm".into(),
    })
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
