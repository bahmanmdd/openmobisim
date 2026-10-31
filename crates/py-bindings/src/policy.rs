//! Policies for a fleet day from Python: a built-in one by name, or any Python object with
//! `step(view)`.
//!
//! `openmobisim.fleet_day(..., policy=...)` hands the policy here: `("builtin", None, name,
//! parameters, step_s)` or `("python", adapter, name, parameters, step_s)`, where the adapter
//! (`openmobisim.operator`) calls the user's object and turns its decisions into tuples naming
//! services, docks, vehicles, chargers and road nodes by their ids. This module converts the
//! ids to the engine's indices, hands the policy a [`PyPolicyView`] at each step, and turns the
//! engine's decision records back into columns with ids.
//!
//! **The view is a copy** of what the engine's view borrows (one array per field and step), and
//! it refuses to be read once its step has ended: a policy keeps what it needs as its own
//! copies. Its queries (`travel_times`, `expected_legs`, `nearest`) are the engine's
//! ([`openmobisim_core_sim::services::queries`]), run in Rust without the interpreter lock and
//! cached within the step; they refuse after the step like the rest of the view.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, PoisonError};

use numpy::ndarray::Array2;
use numpy::{IntoPyArray, PyArray2};
use pyo3::exceptions::{PyKeyError, PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple};

use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_sim::services::decisions::{Decision, OperatorTaskTable};
use openmobisim_core_sim::services::policy::{DayInfo, Policy, PolicyDescription, policy_builtin};
use openmobisim_core_sim::services::queries::{NearestKind, Queries};
use openmobisim_core_sim::services::view::{ServiceView, View};
use openmobisim_core_sim::services::{ServiceParts, ServiceSetup};
use openmobisim_core_types::ids::NodeId;

/// One service's ids, for converting between Python's ids and the engine's indices.
struct ServiceIds {
    name: String,
    docks: Vec<String>,
    dock_of: HashMap<String, u32>,
    vehicles: Vec<String>,
    vehicle_of: HashMap<String, u32>,
    chargers: Vec<String>,
    charger_of: HashMap<String, u32>,
}

fn index_of(ids: &[String]) -> HashMap<String, u32> {
    (0_u32..).zip(ids).map(|(i, s)| (s.clone(), i)).collect()
}

/// Every service's ids and the network's road nodes.
pub(crate) struct Ids {
    services: Vec<ServiceIds>,
    net: Arc<RoadNetwork>,
}

impl Ids {
    /// The road network the ids' nodes are of.
    pub(crate) fn network(&self) -> &Arc<RoadNetwork> {
        &self.net
    }

    pub(crate) fn new(setups: &[ServiceSetup], net: Arc<RoadNetwork>) -> Self {
        let services = setups
            .iter()
            .map(|s| {
                let (docks, vehicles, chargers) = match s.parts() {
                    ServiceParts::BikeDocked(d, _) => (d.external.clone(), Vec::new(), Vec::new()),
                    ServiceParts::Taxi(f, _) => {
                        (Vec::new(), f.vehicle_ids.clone(), f.charger_ids.clone())
                    }
                };
                ServiceIds {
                    name: s.name().to_string(),
                    dock_of: index_of(&docks),
                    vehicle_of: index_of(&vehicles),
                    charger_of: index_of(&chargers),
                    docks,
                    vehicles,
                    chargers,
                }
            })
            .collect();
        Self { services, net }
    }

    fn node(&self, n: NodeId) -> String {
        self.net.node_external_ids().external_of(n).unwrap_or_default().to_string()
    }

    /// Road node `id` (a text id, or an integer standing for its text).
    fn node_of(&self, id: &Bound<'_, PyAny>) -> PyResult<NodeId> {
        let text = match id.extract::<String>() {
            Ok(t) => t,
            Err(_) => id.extract::<i64>().map(|i| i.to_string()).map_err(|_| {
                PyValueError::new_err(format!("a road node id is text or an integer, not {id}"))
            })?,
        };
        self.net
            .node_external_ids()
            .typed_id_of::<NodeId>(&text)
            .ok_or_else(|| PyKeyError::new_err(format!("no road node {text:?}")))
    }

    fn service(&self, name: &str) -> Result<(u16, &ServiceIds), String> {
        let names: Vec<&str> = self.services.iter().map(|s| s.name.as_str()).collect();
        let k = names
            .iter()
            .position(|n| *n == name)
            .ok_or_else(|| format!("no service {name:?}; the services are {names:?}"))?;
        Ok((u16::try_from(k).expect("services fit u16"), &self.services[k]))
    }
}

/// What a policy sees at one step: the clock, the step's index, a seed for its own draws, and
/// per service (``view["bike_share"]``) a dict of arrays. It cannot be read after its step.
#[pyclass(name = "PolicyView", module = "openmobisim._core", frozen)]
pub struct PyPolicyView {
    step: u32,
    inner: Mutex<Option<ViewData>>,
}

struct ViewData {
    t: f64,
    rng_seed: u64,
    window: (f64, f64),
    names: Vec<String>,
    sections: Vec<Py<PyDict>>,
    zones: Option<Py<PyDict>>,
    queries: Queries,
    ids: Arc<Ids>,
}

impl PyPolicyView {
    fn read<R>(&self, f: impl FnOnce(&ViewData) -> PyResult<R>) -> PyResult<R> {
        let guard = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        match guard.as_ref() {
            Some(d) => f(d),
            None => Err(PyRuntimeError::new_err(format!(
                "this view is of step {}, which has ended: a policy reads its view during \
                 step(view) and keeps copies of what it needs",
                self.step
            ))),
        }
    }

    pub(crate) fn end(&self) {
        *self.inner.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }
}

#[pymethods]
impl PyPolicyView {
    /// The step's second, from midnight.
    #[getter]
    fn t(&self) -> PyResult<f64> {
        self.read(|d| Ok(d.t))
    }

    /// The step's index: 0 at the window's start.
    #[getter]
    fn step(&self) -> PyResult<u32> {
        self.read(|_| Ok(self.step))
    }

    /// A number for the policy's own draws at this step (``numpy.random.default_rng(view.
    /// rng_seed)``): the same day, seed and step give the same number.
    #[getter]
    fn rng_seed(&self) -> PyResult<u64> {
        self.read(|d| Ok(d.rng_seed))
    }

    /// The day's window: its start and end, in seconds from midnight.
    #[getter]
    fn window(&self) -> PyResult<(f64, f64)> {
        self.read(|d| Ok(d.window))
    }

    /// The services' names, in the order given.
    #[getter]
    fn services(&self) -> PyResult<Vec<String>> {
        self.read(|d| Ok(d.names.clone()))
    }

    fn __getitem__<'py>(&self, py: Python<'py>, name: &str) -> PyResult<Bound<'py, PyDict>> {
        self.read(|d| {
            let k = d.names.iter().position(|n| n == name).ok_or_else(|| {
                PyKeyError::new_err(format!("no service {name:?}; the services are {:?}", d.names))
            })?;
            Ok(d.sections[k].bind(py).clone())
        })
    }

    fn __contains__(&self, name: &str) -> PyResult<bool> {
        self.read(|d| Ok(d.names.iter().any(|n| n == name)))
    }

    /// The zones: a square grid of ``grid_m`` metres over the road network's nodes, a zone per
    /// cell holding a node (``id``, its centre's ``lon`` and ``lat``); ``None`` without zones.
    /// Each service's ``zone`` gives the zone per dock or vehicle (-1: none).
    #[getter]
    fn zones<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyDict>>> {
        self.read(|d| Ok(d.zones.as_ref().map(|z| z.bind(py).clone())))
    }

    /// Seconds from each road node of ``from_nodes`` to each of ``to_nodes``, leaving at the
    /// step's second on the day's road times: a matrix ``[len(from_nodes), len(to_nodes)]``,
    /// ``inf`` where no road gets there within ``max_s`` seconds (nor within 4 hours).
    /// ``service`` (a name, or ``None``) is checked but changes nothing: every service's
    /// vehicles drive the roads.
    #[pyo3(signature = (service, from_nodes, to_nodes, max_s = f64::INFINITY))]
    fn travel_times<'py>(
        &self,
        py: Python<'py>,
        service: Option<&str>,
        from_nodes: Vec<Bound<'py, PyAny>>,
        to_nodes: Vec<Bound<'py, PyAny>>,
        max_s: f64,
    ) -> PyResult<Bound<'py, PyArray2<f64>>> {
        let (q, from, to) = self.read(|d| {
            if let Some(name) = service {
                d.ids.service(name).map_err(PyKeyError::new_err)?;
            }
            let nodes = |list: &[Bound<'py, PyAny>]| -> PyResult<Vec<NodeId>> {
                list.iter().map(|n| d.ids.node_of(n)).collect()
            };
            Ok((d.queries.clone(), nodes(&from_nodes)?, nodes(&to_nodes)?))
        })?;
        let m = py.detach(|| q.travel_times(&from, &to, max_s)).map_err(PyValueError::new_err)?;
        let a = Array2::from_shape_vec((from.len(), to.len()), m.to_vec())
            .map_err(|e| PyValueError::new_err(e.to_string()))?;
        Ok(a.into_pyarray(py))
    }

    /// The people expected to ask for ``service`` per dock (a docked service: pickups at the
    /// dock each leg planned; rows as ``view[service]["dock_id"]``) or per zone (a taxi fleet:
    /// requests by their origin's zone; rows as ``view.zones["id"]``), per time bin of
    /// ``bin_s`` seconds from the window's start: a matrix ``[rows, bins]``. From the day's
    /// legs before the variation a seed draws (``fleet_environment``'s ``legs_keep`` and
    /// ``departure_jitter_s``): the expectation over the days the seeds draw. **In a day
    /// without variation (``fleet_day``, or ``legs_keep=1`` and no jitter) these are the
    /// day's own legs: what the day will ask for.**
    #[pyo3(signature = (service, bin_s = 900.0))]
    fn expected_legs<'py>(
        &self,
        py: Python<'py>,
        service: &str,
        bin_s: f64,
    ) -> PyResult<Bound<'py, PyArray2<f64>>> {
        let e = self.read(|d| {
            let (s, _) = d.ids.service(service).map_err(PyKeyError::new_err)?;
            d.queries.expected_legs(s, bin_s).map_err(PyValueError::new_err)
        })?;
        let a = Array2::from_shape_vec((e.rows, e.bins), e.values.clone())
            .map_err(|err| PyValueError::new_err(err.to_string()))?;
        Ok(a.into_pyarray(py))
    }

    /// ``service``'s ``k`` docks (``kind="docks"``, by their road nodes) or vehicles
    /// (``kind="vehicles"``, by the nodes they are at or driving to) nearest road node
    /// ``node`` by free-flow time: a list of ``(id, seconds)``, nearest first, ties by their
    /// order in the view; those no road reaches are left out.
    fn nearest(
        &self,
        py: Python<'_>,
        service: &str,
        kind: &str,
        node: &Bound<'_, PyAny>,
        k: usize,
    ) -> PyResult<Vec<(String, f64)>> {
        let (q, s, kind, from, ids) = self.read(|d| {
            let (s, _) = d.ids.service(service).map_err(PyKeyError::new_err)?;
            let kind = match kind {
                "docks" => NearestKind::Docks,
                "vehicles" => NearestKind::Vehicles,
                other => {
                    return Err(PyValueError::new_err(format!(
                        "kind is \"docks\" or \"vehicles\", not {other:?}"
                    )));
                }
            };
            Ok((d.queries.clone(), s, kind, d.ids.node_of(node)?, Arc::clone(&d.ids)))
        })?;
        let found = py.detach(|| q.nearest(s, kind, from, k)).map_err(PyValueError::new_err)?;
        let si = &ids.services[usize::from(s)];
        let names = if kind == NearestKind::Docks { &si.docks } else { &si.vehicles };
        Ok(found.into_iter().map(|(i, secs)| (names[i as usize].clone(), secs)).collect())
    }

    fn __repr__(&self) -> String {
        let guard = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        match guard.as_ref() {
            Some(d) => {
                format!("PolicyView(step {}, t {} s, services {:?})", self.step, d.t, d.names)
            }
            None => format!("PolicyView(step {}, ended)", self.step),
        }
    }
}

/// The Python view of `view`: copies of its arrays, ids in place of indices.
pub(crate) fn view_to_py(
    py: Python<'_>,
    view: &View<'_>,
    ids: &Arc<Ids>,
) -> PyResult<Py<PyPolicyView>> {
    let zones = view.queries.zones();
    let zone_of_node = |n: NodeId| zones.and_then(|z| z.of_node(n)).map_or(-1, i64::from);
    let mut sections = Vec::with_capacity(view.services.len());
    for (s, section) in ids.services.iter().zip(&view.services) {
        let d = PyDict::new(py);
        match section {
            ServiceView::BikeDocked(v) => {
                d.set_item("kind", "bike_docked")?;
                d.set_item("dock_id", s.docks.clone())?;
                d.set_item(
                    "node",
                    (0..v.position.len())
                        .map(|k| v.road_node.get(k).map(|&n| ids.node(n)))
                        .collect::<Vec<_>>(),
                )?;
                d.set_item("bikes", v.bikes.to_vec().into_pyarray(py))?;
                d.set_item("slots", v.slots.to_vec().into_pyarray(py))?;
                d.set_item("incoming", v.incoming.to_vec().into_pyarray(py))?;
                d.set_item(
                    "lon",
                    v.position.iter().map(|p| p.lon).collect::<Vec<_>>().into_pyarray(py),
                )?;
                d.set_item(
                    "lat",
                    v.position.iter().map(|p| p.lat).collect::<Vec<_>>().into_pyarray(py),
                )?;
                for (k, a) in [
                    ("pickups", v.pickups),
                    ("returns", v.returns),
                    ("no_bike", v.no_bike),
                    ("redirected_in", v.redirected_in),
                    ("redirected_out", v.redirected_out),
                ] {
                    d.set_item(k, a.to_vec().into_pyarray(py))?;
                }
                d.set_item("moving", v.moving)?;
                d.set_item("riding", v.riding)?;
                let zone: Vec<i64> = (0..v.position.len())
                    .map(|k| match v.road_node.get(k) {
                        Some(&n) => zone_of_node(n),
                        None => zones.and_then(|z| z.of_point(v.position[k])).map_or(-1, i64::from),
                    })
                    .collect();
                d.set_item("zone", zone.into_pyarray(py))?;
            }
            ServiceView::Taxi(v) => {
                d.set_item("kind", "taxi")?;
                d.set_item("vehicle_id", s.vehicles.clone())?;
                d.set_item("state", v.state.iter().map(|x| x.as_str()).collect::<Vec<_>>())?;
                d.set_item("node", v.node.iter().map(|&n| ids.node(n)).collect::<Vec<_>>())?;
                d.set_item("until_s", v.until_s.to_vec().into_pyarray(py))?;
                d.set_item("soc", v.soc.to_vec().into_pyarray(py))?;
                d.set_item(
                    "zone",
                    v.node.iter().map(|&n| zone_of_node(n)).collect::<Vec<_>>().into_pyarray(py),
                )?;
                d.set_item(
                    "nearest_charger",
                    v.node
                        .iter()
                        .map(|&n| v.nearest_charger(n).map(|c| s.chargers[c as usize].clone()))
                        .collect::<Vec<_>>(),
                )?;
                d.set_item(
                    "waiting_id",
                    v.waiting.iter().map(|w| w.id).collect::<Vec<_>>().into_pyarray(py),
                )?;
                d.set_item(
                    "waiting_node",
                    v.waiting.iter().map(|w| ids.node(w.origin)).collect::<Vec<_>>(),
                )?;
                d.set_item(
                    "waiting_zone",
                    v.waiting
                        .iter()
                        .map(|w| zone_of_node(w.origin))
                        .collect::<Vec<_>>()
                        .into_pyarray(py),
                )?;
                d.set_item(
                    "waiting_since_s",
                    v.waiting.iter().map(|w| w.since_s).collect::<Vec<_>>().into_pyarray(py),
                )?;
                d.set_item("charger_id", s.chargers.clone())?;
                d.set_item(
                    "charger_node",
                    v.charger_node.iter().map(|&n| ids.node(n)).collect::<Vec<_>>(),
                )?;
                d.set_item("charger_plugs", v.charger_plugs.to_vec().into_pyarray(py))?;
                d.set_item("charger_in_use", v.charger_in_use.clone().into_pyarray(py))?;
                d.set_item("charger_queued", v.charger_queued.clone().into_pyarray(py))?;
            }
        }
        sections.push(d.unbind());
    }
    let data = ViewData {
        t: view.t,
        rng_seed: view.rng_seed,
        window: view.window,
        names: ids.services.iter().map(|s| s.name.clone()).collect(),
        sections,
        zones: match zones {
            Some(z) => {
                let d = PyDict::new(py);
                d.set_item("grid_m", z.grid_m())?;
                d.set_item("id", (0..z.len() as i64).collect::<Vec<_>>().into_pyarray(py))?;
                d.set_item("lon", z.lon().to_vec().into_pyarray(py))?;
                d.set_item("lat", z.lat().to_vec().into_pyarray(py))?;
                Some(d.unbind())
            }
            None => None,
        },
        queries: view.queries.clone(),
        ids: Arc::clone(ids),
    };
    Py::new(py, PyPolicyView { step: view.step, inner: Mutex::new(Some(data)) })
}

/// One decision as the adapter hands it (a tuple naming things by id), as the engine's.
pub(crate) fn decision_from_py(
    item: &Bound<'_, PyAny>,
    ids: &Ids,
    net: &RoadNetwork,
) -> Result<Decision, String> {
    let t: &Bound<'_, PyTuple> =
        item.cast().map_err(|_| format!("a decision is a tuple, not {item}"))?;
    let get = |k: usize| t.get_item(k).map_err(|_| format!("decision {t} is too short"));
    let text = |k: usize| -> Result<String, String> {
        get(k)?.extract::<String>().map_err(|_| format!("decision {t}: item {k} is not text"))
    };
    let kind = text(0)?;
    let (service, s) = ids.service(&text(1)?)?;
    let dock = |k: usize| -> Result<u32, String> {
        let id = text(k)?;
        s.dock_of.get(&id).copied().ok_or_else(|| format!("no dock {id:?} in service {:?}", s.name))
    };
    let vehicle = |k: usize| -> Result<u32, String> {
        let id = text(k)?;
        s.vehicle_of
            .get(&id)
            .copied()
            .ok_or_else(|| format!("no vehicle {id:?} in service {:?}", s.name))
    };
    let count = |k: usize| -> Result<u32, String> {
        get(k)?.extract::<u32>().map_err(|_| format!("decision {t}: item {k} is not a count"))
    };
    let expected = |n: usize| -> Result<(), String> {
        if t.len() == n { Ok(()) } else { Err(format!("a {kind} decision has {n} items, got {t}")) }
    };
    match kind.as_str() {
        "move" => {
            expected(5)?;
            Ok(Decision::Move { service, from_dock: dock(2)?, to_dock: dock(3)?, n: count(4)? })
        }
        "target" => {
            expected(4)?;
            Ok(Decision::Target { service, dock: dock(2)?, level: count(3)? })
        }
        "relocate" => {
            expected(4)?;
            let id = text(3)?;
            let node = net
                .node_external_ids()
                .typed_id_of::<NodeId>(&id)
                .ok_or_else(|| format!("no road node {id:?}"))?;
            Ok(Decision::Relocate { service, vehicle: vehicle(2)?, node })
        }
        "charge" => {
            expected(5)?;
            let id = text(3)?;
            let charger = s
                .charger_of
                .get(&id)
                .copied()
                .ok_or_else(|| format!("no charger {id:?} in service {:?}", s.name))?;
            let to_soc = get(4)?
                .extract::<f64>()
                .map_err(|_| format!("decision {t}: to_soc is not a number"))?;
            Ok(Decision::Charge { service, vehicle: vehicle(2)?, charger, to_soc })
        }
        other => Err(format!(
            "unknown decision kind {other:?}; the kinds are move, target, relocate, charge"
        )),
    }
}

/// A Python object as a policy: `step(view)` returning decisions (through the adapter), with
/// optional `day_start(day)`. A copied engine shares the Python object (its state included).
pub(crate) struct PythonPolicy {
    obj: Py<PyAny>,
    step_s: f64,
    description: PolicyDescription,
    ids: Arc<Ids>,
    /// The Python exception that stopped the day, for the binding to raise as it was.
    error: Arc<Mutex<Option<PyErr>>>,
    /// `day_start`'s failure: the first step returns it.
    failed: Option<String>,
}

impl Clone for PythonPolicy {
    fn clone(&self) -> Self {
        Python::attach(|py| Self {
            obj: self.obj.clone_ref(py),
            step_s: self.step_s,
            description: self.description.clone(),
            ids: Arc::clone(&self.ids),
            error: Arc::clone(&self.error),
            failed: self.failed.clone(),
        })
    }
}

impl PythonPolicy {
    fn raised(&self, e: PyErr) -> String {
        let msg = e.to_string();
        *self.error.lock().unwrap_or_else(PoisonError::into_inner) = Some(e);
        msg
    }
}

impl Policy for PythonPolicy {
    fn describe(&self) -> PolicyDescription {
        self.description.clone()
    }

    fn step_s(&self) -> f64 {
        self.step_s
    }

    fn day_start(&mut self, day: &DayInfo<'_>) {
        let result = Python::attach(|py| -> PyResult<()> {
            let obj = self.obj.bind(py);
            if !obj.getattr("day_start").is_ok_and(|f| f.is_callable()) {
                return Ok(());
            }
            let d = PyDict::new(py);
            d.set_item("window", day.window)?;
            d.set_item("seed", day.seed)?;
            d.set_item(
                "services",
                day.services.iter().map(ServiceSetup::name).collect::<Vec<_>>(),
            )?;
            obj.call_method1("day_start", (d,))?;
            Ok(())
        });
        if let Err(e) = result {
            self.failed = Some(self.raised(e));
        }
    }

    fn step(&mut self, view: &View<'_>, out: &mut Vec<Decision>) -> Result<(), String> {
        if let Some(m) = self.failed.take() {
            return Err(m);
        }
        Python::attach(|py| {
            let v = view_to_py(py, view, &self.ids).map_err(|e| self.raised(e))?;
            let answer = self.obj.bind(py).call_method1("step", (v.clone_ref(py),));
            v.get().end();
            let answer = answer.map_err(|e| self.raised(e))?;
            if answer.is_none() {
                return Ok(());
            }
            let items = answer
                .try_iter()
                .map_err(|_| format!("step(view) must return a list of decisions, not {answer}"))?;
            for item in items {
                let item = item.map_err(|e| self.raised(e))?;
                let d = decision_from_py(&item, &self.ids, &self.ids.net)
                    .map_err(|e| format!("step {} (t {} s): {e}", view.step, view.t))?;
                out.push(d);
            }
            Ok(())
        })
    }
}

/// A policy as `fleet.py` hands it: `(kind, object, name, parameters, step_s)`.
pub(crate) type PolicyTuple<'py> =
    (String, Option<Bound<'py, PyAny>>, String, BTreeMap<String, f64>, f64);

/// Where a Python policy leaves the exception that stopped the day.
pub(crate) type Raised = Arc<Mutex<Option<PyErr>>>;

/// The policy `fleet.py` asked for, and the Python exception slot of a Python one.
pub(crate) fn policy_from_py(
    policy: Option<PolicyTuple<'_>>,
    ids: Arc<Ids>,
) -> PyResult<(Option<Box<dyn Policy>>, Raised)> {
    let error = Arc::new(Mutex::new(None));
    let Some((kind, obj, name, parameters, step_s)) = policy else { return Ok((None, error)) };
    let policy: Box<dyn Policy> = match (kind.as_str(), obj) {
        ("builtin", _) => policy_builtin(&name, &parameters).map_err(PyValueError::new_err)?,
        ("python", Some(obj)) => Box::new(PythonPolicy {
            obj: obj.unbind(),
            step_s,
            description: PolicyDescription { name, parameters: parameters.into_iter().collect() },
            ids,
            error: Arc::clone(&error),
            failed: None,
        }),
        (other, _) => return Err(PyValueError::new_err(format!("unknown policy kind {other:?}"))),
    };
    Ok((Some(policy), error))
}

/// The decisions taken, as columns with ids: `t`'s columns, a number a row does not have `NaN`,
/// an id `None`.
pub(crate) fn decision_columns<'py>(
    py: Python<'py>,
    t: &OperatorTaskTable,
) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item("step", t.step.clone().into_pyarray(py))?;
    d.set_item("t_s", t.t_s.clone().into_pyarray(py))?;
    d.set_item("service", t.service.clone())?;
    d.set_item("kind", t.kind.clone())?;
    d.set_item("from_dock", t.from_dock.clone())?;
    d.set_item("to_dock", t.to_dock.clone())?;
    d.set_item("n", t.n.clone().into_pyarray(py))?;
    d.set_item("dock", t.dock.clone())?;
    d.set_item("level", t.level.clone().into_pyarray(py))?;
    d.set_item("vehicle", t.vehicle.clone())?;
    d.set_item("node", t.node.clone())?;
    d.set_item("charger", t.charger.clone())?;
    d.set_item("to_soc", t.to_soc.clone().into_pyarray(py))?;
    d.set_item("accepted", t.accepted.clone())?;
    d.set_item("reason", t.reason.clone())?;
    d.set_item("executed_s", t.executed_s.clone().into_pyarray(py))?;
    d.set_item("finished_s", t.finished_s.clone().into_pyarray(py))?;
    d.set_item("moved", t.moved.clone().into_pyarray(py))?;
    d.set_item("metres", t.metres.clone().into_pyarray(py))?;
    d.set_item("from_target", t.from_target.clone())?;
    Ok(d)
}

/// A policy's name and parameters as a dict, or `None`.
pub(crate) fn description_to_py<'py>(
    py: Python<'py>,
    d: Option<&PolicyDescription>,
) -> PyResult<Option<Bound<'py, PyDict>>> {
    let Some(d) = d else { return Ok(None) };
    let out = PyDict::new(py);
    out.set_item("name", d.name.clone())?;
    let p = PyDict::new(py);
    for (k, v) in &d.parameters {
        p.set_item(k, v)?;
    }
    out.set_item("parameters", p)?;
    Ok(Some(out))
}
