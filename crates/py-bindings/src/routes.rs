//! Route sets, seen from Python (S165): the store as numpy arrays, a builder,
//! and the list of methods.
//!
//! Everything is a flat array indexed the way the store is: keys sorted, a key's
//! routes a range of route numbers, a route's links a range of the flat link
//! array. `python/openmobisim` turns these into the friendlier `route_sets_*`
//! functions and `viz.map_route`.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, OnceLock};

use numpy::{IntoPyArray, PyArray1};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use openmobisim_core_graph::defaults::SignalDefaults;
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_routes::{DEFAULT_METHOD, LinkIndex, Registry, RouteKey, RouteSets};

use crate::network::PyNetwork;

/// Route sets: the alternative routes of many origin-destination pairs.
///
/// Keys are pairs of node indices (see `Network.node_nearest`), sorted. Route
/// `r` of the store is `links[route_offsets[r]:route_offsets[r + 1]]`; key `k`
/// owns routes `set_offsets[k]:set_offsets[k + 1]`, best first.
#[pyclass(name = "RouteSets", module = "openmobisim._core")]
pub struct PyRouteSets {
    pub(crate) inner: Arc<RouteSets>,
    link_count: u32,
    index: OnceLock<LinkIndex>,
}

impl PyRouteSets {
    pub(crate) fn new(inner: Arc<RouteSets>, link_count: u32) -> Self {
        Self { inner, link_count, index: OnceLock::new() }
    }
}

#[pymethods]
impl PyRouteSets {
    /// The method that made these sets (for example ``"penalty"``).
    #[getter]
    fn method(&self) -> String {
        self.inner.method().to_string()
    }

    /// The method and every option with defaults filled in, as one string.
    #[getter]
    fn descriptor(&self) -> String {
        self.inner.descriptor().to_string()
    }

    /// A short identity of the network and the method with its options, in hex:
    /// equal exactly when both are.
    #[getter]
    fn identity(&self) -> String {
        format!("{:016x}", self.inner.identity())
    }

    /// How many origin-destination pairs have a set.
    #[getter]
    fn key_count(&self) -> usize {
        self.inner.keys().len()
    }

    /// How many routes in all.
    #[getter]
    fn route_count(&self) -> usize {
        self.inner.route_count()
    }

    /// Bytes held.
    #[getter]
    fn bytes(&self) -> usize {
        self.inner.bytes()
    }

    /// The keys as ``(origin_nodes, destination_nodes)``, uint32 arrays, sorted.
    fn keys<'py>(&self, py: Python<'py>) -> (Bound<'py, PyArray1<u32>>, Bound<'py, PyArray1<u32>>) {
        let k = self.inner.keys();
        (
            k.iter().map(|k| k.origin).collect::<Vec<_>>().into_pyarray(py),
            k.iter().map(|k| k.destination).collect::<Vec<_>>().into_pyarray(py),
        )
    }

    /// ``key_count + 1`` offsets into the routes (uint32).
    fn set_offsets<'py>(&self, py: Python<'py>) -> Bound<'py, PyArray1<u32>> {
        self.inner.set_start().to_vec().into_pyarray(py)
    }

    /// ``route_count + 1`` offsets into ``links()`` (uint32).
    fn route_offsets<'py>(&self, py: Python<'py>) -> Bound<'py, PyArray1<u32>> {
        self.inner.route_start().to_vec().into_pyarray(py)
    }

    /// Every route's links, one flat uint32 array of link indices.
    fn links<'py>(&self, py: Python<'py>) -> Bound<'py, PyArray1<u32>> {
        self.inner.links().to_vec().into_pyarray(py)
    }

    /// Every route's free-flow cost in seconds (float32).
    fn costs<'py>(&self, py: Python<'py>) -> Bound<'py, PyArray1<f32>> {
        self.inner.costs().to_vec().into_pyarray(py)
    }

    /// Every route's largest share of its cost shared with a route found
    /// before it (float32; 0 for the first).
    fn overlaps<'py>(&self, py: Python<'py>) -> Bound<'py, PyArray1<f32>> {
        self.inner.overlaps().to_vec().into_pyarray(py)
    }

    /// The position of the pair ``(origin_node, destination_node)`` among the
    /// keys, or ``None`` if it has no set.
    fn find(&self, origin: u32, destination: u32) -> Option<usize> {
        self.inner.key_index(RouteKey { origin, destination })
    }

    /// The routes of the key at `key_index`, best first, each as a uint32
    /// array of link indices.
    ///
    /// # Errors
    ///
    /// `ValueError` if `key_index` is out of range.
    fn routes<'py>(
        &self,
        py: Python<'py>,
        key_index: usize,
    ) -> PyResult<Vec<Bound<'py, PyArray1<u32>>>> {
        if key_index >= self.inner.keys().len() {
            return Err(PyValueError::new_err(format!("no key {key_index}")));
        }
        Ok(self.inner.routes(key_index).map(|r| r.links.to_vec().into_pyarray(py)).collect())
    }

    /// The route numbers that use `link`, ascending (uint32): the inverted
    /// index, built the first time it is asked for.
    fn link_routes<'py>(&self, py: Python<'py>, link: u32) -> PyResult<Bound<'py, PyArray1<u32>>> {
        if link >= self.link_count {
            return Err(PyValueError::new_err(format!("no link {link}")));
        }
        let index = self.index.get_or_init(|| self.inner.link_index(self.link_count));
        Ok(index.routes_using(link).to_vec().into_pyarray(py))
    }

    /// The key (position among the keys) that route number `route` belongs to.
    ///
    /// # Errors
    ///
    /// `ValueError` if `route` is out of range.
    fn route_key(&self, route: usize) -> PyResult<usize> {
        if route >= self.inner.route_count() {
            return Err(PyValueError::new_err(format!("no route {route}")));
        }
        Ok(self.inner.key_of_route(route))
    }

    fn __repr__(&self) -> String {
        format!(
            "RouteSets({} pairs, {} routes, method {:?})",
            self.inner.keys().len(),
            self.inner.route_count(),
            self.inner.method()
        )
    }
}

pub(crate) fn to_options(options: Option<HashMap<String, f64>>) -> BTreeMap<String, f64> {
    options.unwrap_or_default().into_iter().collect()
}

/// The route-set methods that can be selected, the default first.
#[pyfunction]
pub fn route_methods() -> Vec<String> {
    let mut names: Vec<String> =
        Registry::builtin().names().into_iter().map(str::to_string).collect();
    names.sort_by_key(|n| (n != DEFAULT_METHOD, n.clone()));
    names
}

/// Make route sets for origin-destination pairs given as coordinates.
///
/// Each pair ``(origin_lon, origin_lat, destination_lon, destination_lat)`` is
/// snapped to the nearest drivable nodes, and its set is generated by `method`
/// (``"penalty"`` by default) with `options` (numbers by name; unknown names
/// and out-of-range values are refused). Pairs that snap to the same node are
/// dropped.
///
/// # Errors
///
/// `ValueError` for an unknown method, an unknown or out-of-range option.
#[pyfunction]
#[pyo3(signature = (network, od_lonlat, method="penalty", options=None))]
pub fn route_sets_build(
    py: Python<'_>,
    network: PyRef<'_, PyNetwork>,
    od_lonlat: Vec<(f64, f64, f64, f64)>,
    method: &str,
    options: Option<HashMap<String, f64>>,
) -> PyResult<PyRouteSets> {
    let generator = Registry::builtin()
        .create(method, &to_options(options))
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    let snapper = network.snapper();
    let keys: Vec<RouteKey> = od_lonlat
        .iter()
        .map(|&(a, b, c, d)| {
            RouteKey::new(
                snapper.nearest(&network.inner, LonLat::new(a, b)),
                snapper.nearest(&network.inner, LonLat::new(c, d)),
            )
        })
        .collect();
    let net = network.inner.clone();
    let sets = py.detach(move || {
        let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);
        RouteSets::generate(&net, &turns, &keys, generator.as_ref())
    });
    Ok(PyRouteSets::new(Arc::new(sets), network.inner.link_count()))
}
