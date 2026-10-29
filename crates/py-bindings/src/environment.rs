//! A fleet day stepped from Python: `reset`, `step(decisions)`, `fork` (`openmobisim.fleet_environment`).
//!
//! The engine owns its city ([`FrozenWorld`]), so it is kept between calls. Each step applies the
//! decisions at the clock, runs to the next step's second, and returns the view there and the
//! reward of what ended in between.

use std::sync::Arc;

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

use openmobisim_core_sim::services::engine::{FleetDay, FleetEngine};
use openmobisim_core_sim::services::outcome::LegStatus;
use openmobisim_core_sim::services::world::ServiceWorld;

use crate::network::PyNetwork;
use crate::policy::{PyPolicyView, decision_from_py, view_to_py};
use crate::services::{Frozen, RoadTuple, ServiceTuple, TripTuple, frozen, outcome_dict};

/// The standard reward's weights: per failed leg (by the people it stands for), per minute
/// waited, per extra minute walked, per van kilometre.
#[derive(Clone, Copy, Debug)]
struct Weights {
    fail: f64,
    wait: f64,
    walk: f64,
    km: f64,
}

/// A fleet day stepped from Python: see the [module docs](self).
#[pyclass(name = "FleetEnvironment", module = "openmobisim._core")]
pub struct PyFleetEnvironment {
    frozen: Arc<Frozen>,
    window: (f64, f64),
    step_s: f64,
    weights: Weights,
    engine: Option<FleetEngine>,
    /// Per leg, whether its end was counted in a reward.
    counted: Vec<bool>,
    /// Decision records counted in a reward.
    records_seen: usize,
    last_view: Option<Py<PyPolicyView>>,
}

impl PyFleetEnvironment {
    fn engine(&mut self) -> PyResult<&mut FleetEngine> {
        self.engine.as_mut().ok_or_else(|| PyRuntimeError::new_err("call reset(seed) first"))
    }

    /// The view at the clock; the previous one ends.
    fn view(&mut self, py: Python<'_>) -> PyResult<Py<PyPolicyView>> {
        if let Some(old) = self.last_view.take() {
            old.get().end();
        }
        let ids = Arc::clone(&self.frozen.ids);
        let engine = self.engine.as_ref().expect("an engine");
        let v = view_to_py(py, &engine.view(), &ids)?;
        self.last_view = Some(v.clone_ref(py));
        Ok(v)
    }

    /// The reward of what ended since the last count, and the counts it is made of.
    fn reward(&mut self) -> (f64, [f64; 4]) {
        let engine = self.engine.as_ref().expect("an engine");
        let legs = &self.frozen.legs;
        let (mut failed, mut wait_min, mut walk_min, mut km) = (0.0, 0.0, 0.0, 0.0);
        for (i, o) in engine.state().legs.iter().enumerate() {
            if self.counted[i] || !matches!(o.status, LegStatus::Served | LegStatus::Failed) {
                continue;
            }
            self.counted[i] = true;
            let w = f64::from(legs[i].weight.max(1));
            if o.status == LegStatus::Failed {
                failed += w;
            }
            if o.waited_s.is_finite() {
                wait_min += w * o.waited_s / 60.0;
            }
            walk_min += w * o.extra_walk_s / 60.0;
        }
        let records = engine.decisions();
        for r in &records[self.records_seen..] {
            if r.accepted && r.metres.is_finite() {
                km += r.metres / 1000.0;
            }
        }
        self.records_seen = records.len();
        let w = self.weights;
        let reward = -(w.fail * failed + w.wait * wait_min + w.walk * walk_min + w.km * km);
        (reward, [failed, wait_min, walk_min, km])
    }

    fn info<'py>(&self, py: Python<'py>, counts: [f64; 4]) -> PyResult<Bound<'py, PyDict>> {
        let info = PyDict::new(py);
        let engine = self.engine.as_ref().expect("an engine");
        info.set_item("t", engine.now())?;
        info.set_item("failed", counts[0])?;
        info.set_item("waited_min", counts[1])?;
        info.set_item("extra_walk_min", counts[2])?;
        info.set_item("van_km", counts[3])?;
        info.set_item("fidelity", "frozen_day")?;
        Ok(info)
    }
}

#[pymethods]
impl PyFleetEnvironment {
    /// Start a day with `seed`: a new engine at the window's start. Returns ``(view, info)``.
    fn reset<'py>(
        &mut self,
        py: Python<'py>,
        seed: u64,
    ) -> PyResult<(Py<PyPolicyView>, Bound<'py, PyDict>)> {
        let day = Arc::new(FleetDay { window: self.window, legs: self.frozen.legs.clone(), seed });
        let world: Arc<dyn ServiceWorld> = self.frozen.world.clone();
        let mut engine = FleetEngine::in_world(Arc::clone(&self.frozen.setups), day, world, None);
        engine.advance_to(self.window.0).map_err(PyValueError::new_err)?;
        self.engine = Some(engine);
        self.counted = vec![false; self.frozen.legs.len()];
        self.records_seen = 0;
        let (_, counts) = self.reward();
        let view = self.view(py)?;
        Ok((view, self.info(py, counts)?))
    }

    /// Apply `decisions` (as a policy returns them) at the clock, then run to the next step:
    /// ``(view, reward, terminated, truncated, info)``; at the window's end ``info["outcome"]``
    /// holds the day's tables.
    #[allow(clippy::type_complexity, reason = "Gymnasium's five-tuple")]
    fn step<'py>(
        &mut self,
        py: Python<'py>,
        decisions: &Bound<'py, PyAny>,
    ) -> PyResult<(Py<PyPolicyView>, f64, bool, bool, Bound<'py, PyDict>)> {
        let ids = Arc::clone(&self.frozen.ids);
        let net = Arc::clone(ids.network());
        let mut parsed = Vec::new();
        if !decisions.is_none() {
            let list: &Bound<'py, PyList> = decisions
                .cast()
                .map_err(|_| PyValueError::new_err("decisions are a list of decisions"))?;
            for item in list.iter() {
                parsed.push(decision_from_py(&item, &ids, &net).map_err(PyValueError::new_err)?);
            }
        }
        let step_s = self.step_s;
        let end = self.window.1;
        let engine = self.engine()?;
        if engine.is_done() {
            return Err(PyRuntimeError::new_err("the day has ended: call reset(seed)"));
        }
        engine.apply(&parsed);
        let next = (engine.now() + step_s).min(end);
        engine.advance_to(next).map_err(PyValueError::new_err)?;
        let terminated = engine.is_done();
        let (reward, counts) = self.reward();
        let info = self.info(py, counts)?;
        if terminated {
            let out = self.engine.as_ref().expect("an engine").clone().into_outcome();
            info.set_item("outcome", outcome_dict(py, &self.frozen, &out)?)?;
        }
        let view = self.view(py)?;
        Ok((view, reward, terminated, false, info))
    }

    /// A copy of this environment at its clock, to simulate ahead without touching it (a
    /// lookahead's): it shares the city, and its view is its own.
    fn fork(&self) -> PyResult<Self> {
        let engine = self
            .engine
            .as_ref()
            .ok_or_else(|| PyRuntimeError::new_err("call reset(seed) first"))?;
        Ok(Self {
            frozen: Arc::clone(&self.frozen),
            window: self.window,
            step_s: self.step_s,
            weights: self.weights,
            engine: Some(engine.clone()),
            counted: self.counted.clone(),
            records_seen: self.records_seen,
            last_view: None,
        })
    }

    /// The engine's clock, in seconds from midnight.
    #[getter]
    fn t(&self) -> PyResult<f64> {
        self.engine
            .as_ref()
            .map(FleetEngine::now)
            .ok_or_else(|| PyRuntimeError::new_err("call reset(seed) first"))
    }

    /// The plan's counts: trips, legs planned and left out by reason.
    fn plan<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let r = self.frozen.report;
        let d = PyDict::new(py);
        d.set_item("trips", r.trips)?;
        d.set_item("legs", r.legs)?;
        d.set_item("no_dock", r.no_dock)?;
        d.set_item("same_dock", r.same_dock)?;
        d.set_item("no_ride", r.no_ride)?;
        Ok(d)
    }
}

/// A fleet environment on `network` (`openmobisim.fleet_environment` checks and documents the
/// inputs).
#[pyfunction(name = "_fleet_environment")]
#[allow(clippy::too_many_arguments, reason = "fleet_environment's arguments, checked in fleet.py")]
pub fn fleet_environment(
    network: &PyNetwork,
    services: Vec<ServiceTuple<'_>>,
    trips: Vec<TripTuple>,
    road_times: Option<RoadTuple>,
    window: (f64, f64),
    step_s: f64,
    weights: (f64, f64, f64, f64),
) -> PyResult<PyFleetEnvironment> {
    if !(step_s.is_finite() && step_s > 0.0) {
        return Err(PyValueError::new_err(format!("step_s must be above 0, got {step_s}")));
    }
    let f = frozen(network, &services, trips, road_times)?;
    let n = f.legs.len();
    Ok(PyFleetEnvironment {
        frozen: Arc::new(f),
        window,
        step_s,
        weights: Weights { fail: weights.0, wait: weights.1, walk: weights.2, km: weights.3 },
        engine: None,
        counted: vec![false; n],
        records_seen: 0,
        last_view: None,
    })
}
