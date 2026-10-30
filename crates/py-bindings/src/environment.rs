//! A fleet day stepped from Python: `reset`, `step(decisions)`, `fork` (`openmobisim.fleet_environment`),
//! and `n` of them stepped together ([`PyFleetVectorEnvironment`]), in parallel.
//!
//! The engine owns its city ([`FrozenWorld`]), so it is kept between calls. Each step applies the
//! decisions at the clock, runs to the next step's second, and returns the view there and the
//! reward of what ended in between.
//!
//! [`FrozenWorld`]: openmobisim_core_sim::services::world::FrozenWorld

use std::sync::Arc;

use numpy::{IntoPyArray, PyArray1};
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use rayon::prelude::*;

use openmobisim_core_sim::services::ServiceParts;
use openmobisim_core_sim::services::decisions::Decision;
use openmobisim_core_sim::services::engine::{FleetDay, FleetEngine};
use openmobisim_core_sim::services::frozen::vary_legs;
use openmobisim_core_sim::services::outcome::{FleetOutcome, LegStatus};
use openmobisim_core_sim::services::world::ServiceWorld;

use crate::network::PyNetwork;
use crate::policy::{PyPolicyView, decision_from_py, view_to_py};
use crate::services::{Frozen, RoadTuple, ServiceTuple, TripTuple, frozen, outcome_dict};

/// The standard reward's weights: per cancelled leg (by the people it stands for), per minute
/// waited, per extra minute walked, per van kilometre.
#[derive(Clone, Copy, Debug)]
struct Weights {
    cancelled: f64,
    wait: f64,
    walk: f64,
    km: f64,
}

/// What every episode of an environment shares: the city and the legs, the window, the step,
/// the reward's weights and how each seed varies the legs.
struct Setup {
    frozen: Arc<Frozen>,
    window: (f64, f64),
    step_s: f64,
    weights: Weights,
    /// The share of legs a day keeps, and how far a request may move, in seconds.
    keep: f64,
    jitter_s: f64,
}

/// One episode: its engine and what its rewards have counted.
#[derive(Clone)]
struct Episode {
    engine: FleetEngine,
    /// Per leg, whether its end was counted in a reward.
    counted: Vec<bool>,
    /// Decision records counted in a reward.
    records_seen: usize,
}

/// What one step of an episode gives back.
struct Stepped {
    reward: f64,
    counts: [f64; 4],
    /// Per decision given, `None` if accepted, else why it was refused.
    reasons: Vec<Option<String>>,
    /// The day's tables, once it has ended.
    outcome: Option<FleetOutcome>,
}

impl Setup {
    /// A new episode with `seed` at the window's start, and the counts of what ended before it
    /// (not rewarded).
    fn start(&self, seed: u64) -> Result<(Episode, [f64; 4]), String> {
        let legs = vary_legs(&self.frozen.legs, seed, self.keep, self.jitter_s, self.window.1);
        let day = Arc::new(FleetDay { window: self.window, legs, seed });
        let world: Arc<dyn ServiceWorld> = self.frozen.world.clone();
        let mut engine = FleetEngine::in_world(Arc::clone(&self.frozen.setups), day, world, None);
        engine.advance_to(self.window.0)?;
        let mut episode =
            Episode { engine, counted: vec![false; self.frozen.legs.len()], records_seen: 0 };
        let (_, counts) = episode.reward(self);
        Ok((episode, counts))
    }

    fn info<'py>(
        py: Python<'py>,
        t: f64,
        counts: [f64; 4],
        reasons: Option<&[Option<String>]>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let info = PyDict::new(py);
        info.set_item("t", t)?;
        info.set_item("cancelled", counts[0])?;
        info.set_item("waited_min", counts[1])?;
        info.set_item("extra_walk_min", counts[2])?;
        info.set_item("van_km", counts[3])?;
        info.set_item("fidelity", "frozen_day")?;
        if let Some(r) = reasons {
            info.set_item("refused", r.to_vec())?;
        }
        Ok(info)
    }

    /// Per service: its kind, and its docks' ids and slots or its vehicles' ids, in the views'
    /// order.
    fn layout<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let out = PyDict::new(py);
        for s in self.frozen.setups.iter() {
            let d = PyDict::new(py);
            match s.parts() {
                ServiceParts::BikeDocked(docks, _) => {
                    d.set_item("kind", "bike_docked")?;
                    d.set_item("dock_id", docks.external.clone())?;
                    d.set_item("slots", docks.capacity.clone())?;
                }
                ServiceParts::Taxi(fleet, _) => {
                    d.set_item("kind", "taxi")?;
                    d.set_item("vehicle_id", fleet.vehicle_ids.clone())?;
                }
            }
            out.set_item(s.name(), d)?;
        }
        Ok(out)
    }

    /// One list of decisions (as the adapter hands them), as the engine's.
    fn parse(&self, decisions: &Bound<'_, PyAny>) -> PyResult<Vec<Decision>> {
        let ids = &self.frozen.ids;
        let mut parsed = Vec::new();
        if !decisions.is_none() {
            let list: &Bound<'_, PyList> = decisions
                .cast()
                .map_err(|_| PyValueError::new_err("decisions are a list of decisions"))?;
            for item in list.iter() {
                parsed.push(
                    decision_from_py(&item, ids, ids.network()).map_err(PyValueError::new_err)?,
                );
            }
        }
        Ok(parsed)
    }
}

impl Episode {
    /// The reward of what ended since the last count, and the counts it is made of.
    fn reward(&mut self, setup: &Setup) -> (f64, [f64; 4]) {
        let legs = &setup.frozen.legs;
        let (mut cancelled, mut wait_min, mut walk_min, mut km) = (0.0, 0.0, 0.0, 0.0);
        for (i, o) in self.engine.state().legs.iter().enumerate() {
            if self.counted[i] || !matches!(o.status, LegStatus::Completed | LegStatus::Cancelled) {
                continue;
            }
            self.counted[i] = true;
            let w = f64::from(legs[i].weight.max(1));
            if o.status == LegStatus::Cancelled {
                cancelled += w;
            }
            if o.waited_s.is_finite() {
                wait_min += w * o.waited_s / 60.0;
            }
            walk_min += w * o.extra_walk_s / 60.0;
        }
        let records = self.engine.decisions();
        for r in &records[self.records_seen..] {
            if r.accepted && r.metres.is_finite() {
                km += r.metres / 1000.0;
            }
        }
        self.records_seen = records.len();
        let w = setup.weights;
        let reward = -(w.cancelled * cancelled + w.wait * wait_min + w.walk * walk_min + w.km * km);
        (reward, [cancelled, wait_min, walk_min, km])
    }

    /// Apply `decisions` at the clock and run to the next step.
    fn step(&mut self, setup: &Setup, decisions: &[Decision]) -> Result<Stepped, String> {
        let reasons = self.engine.apply(decisions);
        let next = (self.engine.now() + setup.step_s).min(setup.window.1);
        self.engine.advance_to(next)?;
        let (reward, counts) = self.reward(setup);
        let outcome = self.engine.is_done().then(|| self.engine.clone().into_outcome());
        Ok(Stepped { reward, counts, reasons, outcome })
    }
}

/// The view of `episode` at its clock; `last` (the previous one) ends.
fn view_of(
    py: Python<'_>,
    setup: &Setup,
    episode: &Episode,
    last: &mut Option<Py<PyPolicyView>>,
) -> PyResult<Py<PyPolicyView>> {
    if let Some(old) = last.take() {
        old.get().end();
    }
    let v = view_to_py(py, &episode.engine.view(), &setup.frozen.ids)?;
    *last = Some(v.clone_ref(py));
    Ok(v)
}

/// The plan's counts: trips, legs planned and left out by reason.
fn plan_dict<'py>(py: Python<'py>, setup: &Setup) -> PyResult<Bound<'py, PyDict>> {
    let r = setup.frozen.report;
    let d = PyDict::new(py);
    d.set_item("trips", r.trips)?;
    d.set_item("legs", r.legs)?;
    d.set_item("no_dock", r.no_dock)?;
    d.set_item("same_dock", r.same_dock)?;
    d.set_item("no_ride", r.no_ride)?;
    Ok(d)
}

/// A fleet day stepped from Python: see the [module docs](self).
#[pyclass(name = "FleetEnvironment", module = "openmobisim._core")]
pub struct PyFleetEnvironment {
    setup: Arc<Setup>,
    episode: Option<Episode>,
    last_view: Option<Py<PyPolicyView>>,
}

#[pymethods]
impl PyFleetEnvironment {
    /// Start a day with `seed`: a new engine at the window's start. Returns ``(view, info)``.
    fn reset<'py>(
        &mut self,
        py: Python<'py>,
        seed: u64,
    ) -> PyResult<(Py<PyPolicyView>, Bound<'py, PyDict>)> {
        let (episode, counts) = self.setup.start(seed).map_err(PyValueError::new_err)?;
        let episode = self.episode.insert(episode);
        let view = view_of(py, &self.setup, episode, &mut self.last_view)?;
        Ok((view, Setup::info(py, episode.engine.now(), counts, None)?))
    }

    /// Apply `decisions` (as a policy returns them) at the clock, then run to the next step:
    /// ``(view, reward, terminated, truncated, info)``; ``info["refused"]`` holds, per decision
    /// given, why it was refused (``None``: accepted); at the window's end ``info["outcome"]``
    /// holds the day's tables.
    #[allow(clippy::type_complexity, reason = "Gymnasium's five-tuple")]
    fn step<'py>(
        &mut self,
        py: Python<'py>,
        decisions: &Bound<'py, PyAny>,
    ) -> PyResult<(Py<PyPolicyView>, f64, bool, bool, Bound<'py, PyDict>)> {
        let parsed = self.setup.parse(decisions)?;
        let episode = self
            .episode
            .as_mut()
            .ok_or_else(|| PyRuntimeError::new_err("call reset(seed) first"))?;
        if episode.engine.is_done() {
            return Err(PyRuntimeError::new_err("the day has ended: call reset(seed)"));
        }
        let stepped = episode.step(&self.setup, &parsed).map_err(PyValueError::new_err)?;
        let info = Setup::info(py, episode.engine.now(), stepped.counts, Some(&stepped.reasons))?;
        if let Some(out) = &stepped.outcome {
            info.set_item("outcome", outcome_dict(py, &self.setup.frozen, out)?)?;
        }
        let view = view_of(py, &self.setup, episode, &mut self.last_view)?;
        Ok((view, stepped.reward, stepped.outcome.is_some(), false, info))
    }

    /// A copy of this environment at its clock, to simulate ahead without touching it (a
    /// lookahead's): it shares the city, and its view is its own.
    fn fork(&self) -> PyResult<Self> {
        let episode = self
            .episode
            .as_ref()
            .ok_or_else(|| PyRuntimeError::new_err("call reset(seed) first"))?;
        Ok(Self { setup: Arc::clone(&self.setup), episode: Some(episode.clone()), last_view: None })
    }

    /// `n` environments of this one's day, stepped together; they share the city.
    fn vector(&self, n: usize) -> PyResult<PyFleetVectorEnvironment> {
        if n == 0 {
            return Err(PyValueError::new_err("n must be 1 or more"));
        }
        Ok(PyFleetVectorEnvironment {
            setup: Arc::clone(&self.setup),
            episodes: Vec::new(),
            last_views: (0..n).map(|_| None).collect(),
        })
    }

    /// The engine's clock, in seconds from midnight.
    #[getter]
    fn t(&self) -> PyResult<f64> {
        self.episode
            .as_ref()
            .map(|e| e.engine.now())
            .ok_or_else(|| PyRuntimeError::new_err("call reset(seed) first"))
    }

    /// The window's start and end, in seconds from midnight.
    #[getter]
    fn window(&self) -> (f64, f64) {
        self.setup.window
    }

    /// Per service: its kind, and its docks' ids and slots or its vehicles' ids.
    fn layout<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        self.setup.layout(py)
    }

    /// The plan's counts: trips, legs planned and left out by reason.
    fn plan<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        plan_dict(py, &self.setup)
    }
}

/// `n` episodes of one fleet day stepped together: each call crosses into Rust once for all of
/// them, and their engines run in parallel with Python's lock released. Seed `k` gives the same
/// day here as in a single environment.
#[pyclass(name = "FleetVectorEnvironment", module = "openmobisim._core")]
pub struct PyFleetVectorEnvironment {
    setup: Arc<Setup>,
    episodes: Vec<Episode>,
    last_views: Vec<Option<Py<PyPolicyView>>>,
}

#[pymethods]
impl PyFleetVectorEnvironment {
    /// Start `n` days, one per seed: ``(views, infos)``.
    #[allow(clippy::type_complexity, reason = "a vector of Gymnasium's pairs")]
    fn reset<'py>(
        &mut self,
        py: Python<'py>,
        seeds: Vec<u64>,
    ) -> PyResult<(Vec<Py<PyPolicyView>>, Vec<Bound<'py, PyDict>>)> {
        let n = self.last_views.len();
        if seeds.len() != n {
            return Err(PyValueError::new_err(format!(
                "reset takes {n} seeds, one per environment, got {}",
                seeds.len()
            )));
        }
        let setup = Arc::clone(&self.setup);
        let started: Vec<(Episode, [f64; 4])> = py
            .detach(|| seeds.par_iter().map(|&s| setup.start(s)).collect::<Result<_, _>>())
            .map_err(PyValueError::new_err)?;
        let (episodes, counts): (Vec<_>, Vec<_>) = started.into_iter().unzip();
        self.episodes = episodes;
        let mut views = Vec::with_capacity(n);
        let mut infos = Vec::with_capacity(n);
        for ((e, last), c) in self.episodes.iter().zip(&mut self.last_views).zip(counts) {
            views.push(view_of(py, &setup, e, last)?);
            infos.push(Setup::info(py, e.engine.now(), c, None)?);
        }
        Ok((views, infos))
    }

    /// Apply one list of decisions per environment and run each to its next step:
    /// ``(views, rewards, terminated, truncated, infos)``, ``rewards`` an array.
    #[allow(clippy::type_complexity, reason = "Gymnasium's vector five-tuple")]
    fn step<'py>(
        &mut self,
        py: Python<'py>,
        decisions: &Bound<'py, PyAny>,
    ) -> PyResult<(
        Vec<Py<PyPolicyView>>,
        Bound<'py, PyArray1<f64>>,
        Vec<bool>,
        Vec<bool>,
        Vec<Bound<'py, PyDict>>,
    )> {
        let n = self.last_views.len();
        let lists: &Bound<'py, PyList> = decisions
            .cast()
            .map_err(|_| PyValueError::new_err("decisions are a list of decision lists"))?;
        if lists.len() != n {
            return Err(PyValueError::new_err(format!(
                "step takes {n} decision lists, one per environment, got {}",
                lists.len()
            )));
        }
        if self.episodes.len() != n {
            return Err(PyRuntimeError::new_err("call reset(seeds) first"));
        }
        if self.episodes.iter().any(|e| e.engine.is_done()) {
            return Err(PyRuntimeError::new_err("the day has ended: call reset(seeds)"));
        }
        let parsed =
            lists.iter().map(|l| self.setup.parse(&l)).collect::<PyResult<Vec<Vec<Decision>>>>()?;
        let setup = Arc::clone(&self.setup);
        let episodes = &mut self.episodes;
        let stepped: Vec<Stepped> = py
            .detach(|| {
                episodes
                    .par_iter_mut()
                    .zip(parsed.par_iter())
                    .map(|(e, d)| e.step(&setup, d))
                    .collect::<Result<_, _>>()
            })
            .map_err(PyValueError::new_err)?;
        let mut views = Vec::with_capacity(n);
        let mut rewards = Vec::with_capacity(n);
        let mut terminated = Vec::with_capacity(n);
        let mut infos = Vec::with_capacity(n);
        for ((e, last), s) in self.episodes.iter().zip(&mut self.last_views).zip(&stepped) {
            let info = Setup::info(py, e.engine.now(), s.counts, Some(&s.reasons))?;
            if let Some(out) = &s.outcome {
                info.set_item("outcome", outcome_dict(py, &setup.frozen, out)?)?;
            }
            views.push(view_of(py, &setup, e, last)?);
            rewards.push(s.reward);
            terminated.push(s.outcome.is_some());
            infos.push(info);
        }
        Ok((views, rewards.into_pyarray(py), terminated, vec![false; n], infos))
    }

    /// How many environments.
    fn __len__(&self) -> usize {
        self.last_views.len()
    }

    /// Every environment's clock, in seconds from midnight.
    #[getter]
    fn t(&self) -> PyResult<Vec<f64>> {
        if self.episodes.is_empty() {
            return Err(PyRuntimeError::new_err("call reset(seeds) first"));
        }
        Ok(self.episodes.iter().map(|e| e.engine.now()).collect())
    }

    /// The window's start and end, in seconds from midnight.
    #[getter]
    fn window(&self) -> (f64, f64) {
        self.setup.window
    }

    /// Per service: its kind, and its docks' ids and slots or its vehicles' ids.
    fn layout<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        self.setup.layout(py)
    }

    /// The plan's counts: trips, legs planned and left out by reason.
    fn plan<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        plan_dict(py, &self.setup)
    }
}

/// A fleet environment on `network` (`openmobisim.fleet_environment` checks and documents the
/// inputs).
#[pyfunction(name = "_fleet_environment")]
#[pyo3(signature = (network, services, trips, road_times, window, step_s, weights, keep, jitter_s, zone_grid_m, from_run=None))]
#[allow(clippy::too_many_arguments, reason = "fleet_environment's arguments, checked in fleet.py")]
pub fn fleet_environment(
    network: &PyNetwork,
    services: Vec<ServiceTuple<'_>>,
    trips: Vec<TripTuple>,
    road_times: Option<RoadTuple>,
    window: (f64, f64),
    step_s: f64,
    weights: (f64, f64, f64, f64),
    keep: f64,
    jitter_s: f64,
    zone_grid_m: f64,
    from_run: Option<crate::frozen_day::FrozenArg>,
) -> PyResult<PyFleetEnvironment> {
    if !(step_s.is_finite() && step_s > 0.0) {
        return Err(PyValueError::new_err(format!("step_s must be above 0, got {step_s}")));
    }
    if !(keep > 0.0 && keep <= 1.0) {
        return Err(PyValueError::new_err(format!("legs_keep must be in (0, 1], got {keep}")));
    }
    if !(jitter_s.is_finite() && jitter_s >= 0.0) {
        return Err(PyValueError::new_err(format!(
            "departure_jitter_s must be 0 or more, got {jitter_s}"
        )));
    }
    // A run's frozen day (S302): its legs as the run played them, in place of the trips.
    let f = match &from_run {
        Some(arg) => crate::frozen_day::frozen_read(network, &services, arg, zone_grid_m)?,
        None => frozen(network, &services, trips, road_times, zone_grid_m)?,
    };
    Ok(PyFleetEnvironment {
        setup: Arc::new(Setup {
            frozen: Arc::new(f),
            window,
            step_s,
            weights: Weights {
                cancelled: weights.0,
                wait: weights.1,
                walk: weights.2,
                km: weights.3,
            },
            keep,
            jitter_s,
        }),
        episode: None,
        last_view: None,
    })
}
