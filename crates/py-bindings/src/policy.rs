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
//! copies.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, PoisonError};

use numpy::IntoPyArray;
use pyo3::exceptions::{PyKeyError, PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple};

use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_sim::services::decisions::{Decision, DecisionRecord};
use openmobisim_core_sim::services::policy::{DayInfo, Policy, PolicyDescription, policy_builtin};
use openmobisim_core_sim::services::view::{ServiceView, View};
use openmobisim_core_sim::services::{ServiceParts, ServiceSetup};
use openmobisim_core_types::ids::{EntityId, NodeId};

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

    fn service(&self, name: &str) -> Result<(u16, &ServiceIds), String> {
        let names: Vec<&str> = self.services.iter().map(|s| s.name.as_str()).collect();
        let k = names
            .iter()
            .position(|n| *n == name)
            .ok_or_else(|| format!("no service {name:?}; the services are {names:?}"))?;
        Ok((u16::try_from(k).expect("services fit u16"), &self.services[k]))
    }

    /// An id at `index` of `ids`, or `None`.
    fn id(ids: &[String], index: f64) -> Option<String> {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "an index")]
        let k = index as usize;
        ids.get(k).cloned()
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

    fn end(&self) {
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
fn view_to_py(py: Python<'_>, view: &View<'_>, ids: &Ids) -> PyResult<Py<PyPolicyView>> {
    let mut sections = Vec::with_capacity(view.services.len());
    for (s, section) in ids.services.iter().zip(&view.services) {
        let d = PyDict::new(py);
        match section {
            ServiceView::BikeDocked(v) => {
                d.set_item("kind", "bike_docked")?;
                d.set_item("dock_id", s.docks.clone())?;
                d.set_item("bikes", v.bikes.to_vec().into_pyarray(py))?;
                d.set_item("slots", v.slots.to_vec().into_pyarray(py))?;
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
                    ("failed", v.failed),
                    ("redirected_in", v.redirected_in),
                    ("redirected_out", v.redirected_out),
                ] {
                    d.set_item(k, a.to_vec().into_pyarray(py))?;
                }
                d.set_item("moving", v.moving)?;
                d.set_item("riding", v.riding)?;
            }
            ServiceView::Taxi(v) => {
                d.set_item("kind", "taxi")?;
                d.set_item("vehicle_id", s.vehicles.clone())?;
                d.set_item("state", v.state.iter().map(|x| x.as_str()).collect::<Vec<_>>())?;
                d.set_item("node", v.node.iter().map(|&n| ids.node(n)).collect::<Vec<_>>())?;
                d.set_item("soc", v.soc.to_vec().into_pyarray(py))?;
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
    };
    Py::new(py, PyPolicyView { step: view.step, inner: Mutex::new(Some(data)) })
}

/// One decision as the adapter hands it (a tuple naming things by id), as the engine's.
fn decision_from_py(
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

/// The decisions taken, as columns with ids.
pub(crate) fn decision_columns<'py>(
    py: Python<'py>,
    records: &[DecisionRecord],
    ids: &Ids,
) -> PyResult<Bound<'py, PyDict>> {
    let n = records.len();
    let param = |r: &DecisionRecord, k: &str| r.parameters.iter().find(|p| p.0 == k).map(|p| p.1);
    let mut text: BTreeMap<&str, Vec<Option<String>>> = BTreeMap::new();
    let mut numbers: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
    for key in ["from_dock", "to_dock", "dock", "vehicle", "node", "charger"] {
        text.insert(key, Vec::with_capacity(n));
    }
    for key in ["n", "level", "to_soc"] {
        numbers.insert(key, Vec::with_capacity(n));
    }
    for r in records {
        let s = &ids.services[usize::from(r.service)];
        for (key, list) in &mut text {
            let id = param(r, key).and_then(|x| match *key {
                "from_dock" | "to_dock" | "dock" => Ids::id(&s.docks, x),
                "vehicle" => Ids::id(&s.vehicles, x),
                "charger" => Ids::id(&s.chargers, x),
                _ => {
                    #[allow(
                        clippy::cast_possible_truncation,
                        clippy::cast_sign_loss,
                        reason = "a node index"
                    )]
                    let k = x as u32;
                    Some(ids.node(NodeId::new(k)))
                }
            });
            list.push(id);
        }
        for (key, list) in &mut numbers {
            list.push(param(r, key).unwrap_or(f64::NAN));
        }
    }
    let d = PyDict::new(py);
    d.set_item("step", records.iter().map(|r| r.step).collect::<Vec<_>>().into_pyarray(py))?;
    d.set_item("t_s", records.iter().map(|r| r.t_s).collect::<Vec<_>>().into_pyarray(py))?;
    d.set_item(
        "service",
        records
            .iter()
            .map(|r| ids.services[usize::from(r.service)].name.clone())
            .collect::<Vec<_>>(),
    )?;
    d.set_item("kind", records.iter().map(|r| r.kind.as_str()).collect::<Vec<_>>())?;
    d.set_item("from_dock", text.remove("from_dock"))?;
    d.set_item("to_dock", text.remove("to_dock"))?;
    d.set_item("n", numbers.remove("n").unwrap_or_default().into_pyarray(py))?;
    d.set_item("dock", text.remove("dock"))?;
    d.set_item("level", numbers.remove("level").unwrap_or_default().into_pyarray(py))?;
    d.set_item("vehicle", text.remove("vehicle"))?;
    d.set_item("node", text.remove("node"))?;
    d.set_item("charger", text.remove("charger"))?;
    d.set_item("to_soc", numbers.remove("to_soc").unwrap_or_default().into_pyarray(py))?;
    d.set_item("accepted", records.iter().map(|r| r.accepted).collect::<Vec<_>>())?;
    d.set_item("reason", records.iter().map(|r| r.reason.clone()).collect::<Vec<_>>())?;
    d.set_item(
        "executed_s",
        records.iter().map(|r| r.executed_s).collect::<Vec<_>>().into_pyarray(py),
    )?;
    d.set_item(
        "finished_s",
        records.iter().map(|r| r.finished_s).collect::<Vec<_>>().into_pyarray(py),
    )?;
    d.set_item("moved", records.iter().map(|r| r.moved).collect::<Vec<_>>().into_pyarray(py))?;
    d.set_item("metres", records.iter().map(|r| r.metres).collect::<Vec<_>>().into_pyarray(py))?;
    d.set_item("from_target", records.iter().map(|r| r.from_target).collect::<Vec<_>>())?;
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
