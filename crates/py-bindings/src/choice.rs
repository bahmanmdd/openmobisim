//! Choice models, seen from Python (S169): the list of models, the batch a
//! Python model is handed, the adapter that lets a Python object be a model,
//! and what each trip chose, as numpy arrays.
//!
//! **A Python model** is any object with a ``choose(batch)`` method that returns,
//! for each situation, the index of the alternative it takes (an integer array
//! one long per situation). It may also say ``name``, ``descriptor`` (both
//! shown in the manifest and part of the run's fingerprint), ``sampled`` (does
//! it draw from ``batch.gumbel``? default yes) and ``attributes`` (the ones it
//! reads, so the run computes only those; default all). It is called once per
//! batch of up to 32 768 trips, with the GIL held.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use numpy::{IntoPyArray, PyArray1, PyReadonlyArray1};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple};

use openmobisim_core_choice::{
    ChoiceBatch, ChoiceError, ChoiceModel, Choices, DEFAULT_MODEL, Registry, gumbel_noise,
};
use openmobisim_core_routes::RouteSets;
use openmobisim_core_sim::{NO_ROUTE, RouteChoices};
use openmobisim_core_types::rng::StreamRng;

/// The choice models that can be selected by name, the default first.
#[pyfunction]
pub fn choice_models() -> Vec<String> {
    let mut names: Vec<String> =
        Registry::builtin().names().into_iter().map(str::to_string).collect();
    names.sort_by_key(|n| (n != DEFAULT_MODEL, n.clone()));
    names
}

/// What a Python choice model is handed: situations × alternatives, as arrays.
///
/// A *situation* is one traveller's one trip; it offers a few *alternatives*
/// (routes), each with the same numeric attributes. Alternative ``a`` of
/// situation ``s`` is at position ``offsets[s] + a`` of every per-alternative
/// array. ``gumbel`` holds each alternative's standard Gumbel error, keyed on
/// (traveller, trip, iteration, alternative identity): add it to a utility and
/// take the largest to sample the logit with the same common random numbers the
/// built-in models use.
#[pyclass(name = "ChoiceBatch", module = "openmobisim._core", frozen)]
pub struct PyChoiceBatch {
    /// The iteration these choices are for.
    #[pyo3(get)]
    iteration: u32,
    /// The attributes on offer, in the order of ``attributes``.
    #[pyo3(get)]
    attribute_names: Vec<String>,
    situations: usize,
    alternatives: usize,
    /// ``situations + 1`` offsets into the per-alternative arrays.
    #[pyo3(get)]
    offsets: Py<PyArray1<u32>>,
    /// Each situation's traveller.
    #[pyo3(get)]
    traveller: Py<PyArray1<u32>>,
    /// Each situation's trip.
    #[pyo3(get)]
    trip: Py<PyArray1<u32>>,
    /// Each alternative's identity: stable when other alternatives come and go.
    #[pyo3(get)]
    identity: Py<PyArray1<u32>>,
    /// Which situation each alternative belongs to.
    #[pyo3(get)]
    situation_of: Py<PyArray1<u32>>,
    /// The attributes, by name: one float64 array per attribute, one value per alternative.
    #[pyo3(get)]
    attributes: Py<PyDict>,
    /// Each alternative's Gumbel error.
    #[pyo3(get)]
    gumbel: Py<PyArray1<f64>>,
}

#[pymethods]
impl PyChoiceBatch {
    /// How many situations.
    fn __len__(&self) -> usize {
        self.situations
    }

    fn __repr__(&self) -> String {
        format!(
            "ChoiceBatch({} situations, {} alternatives, attributes {:?})",
            self.situations, self.alternatives, self.attribute_names
        )
    }
}

fn build_batch(
    py: Python<'_>,
    batch: &ChoiceBatch,
    rng: &StreamRng,
) -> PyResult<Py<PyChoiceBatch>> {
    let attributes = PyDict::new(py);
    for (i, name) in batch.attribute_names().iter().enumerate() {
        attributes.set_item(name, batch.column(i).to_vec().into_pyarray(py))?;
    }
    let mut situation_of = Vec::with_capacity(batch.alternatives());
    for s in 0..batch.situations() {
        let s32 = u32::try_from(s).expect("fewer than 2^32 situations");
        situation_of.extend(std::iter::repeat_n(s32, batch.range(s).len()));
    }
    Py::new(
        py,
        PyChoiceBatch {
            iteration: batch.iteration(),
            attribute_names: batch.attribute_names().to_vec(),
            situations: batch.situations(),
            alternatives: batch.alternatives(),
            offsets: batch.offsets().to_vec().into_pyarray(py).unbind(),
            traveller: batch.travellers().to_vec().into_pyarray(py).unbind(),
            trip: batch.trips().to_vec().into_pyarray(py).unbind(),
            identity: batch.identities().to_vec().into_pyarray(py).unbind(),
            situation_of: situation_of.into_pyarray(py).unbind(),
            attributes: attributes.unbind(),
            gumbel: gumbel_noise(batch, rng).into_pyarray(py).unbind(),
        },
    )
}

/// A Python object used as a choice model.
pub struct PythonChoice {
    model: Py<PyAny>,
    name: String,
    descriptor: String,
    sampled: bool,
    attributes: Option<Vec<String>>,
}

impl PythonChoice {
    /// Wrap `model`, reading its optional ``name``, ``descriptor``, ``sampled``
    /// and ``attributes``.
    ///
    /// # Errors
    ///
    /// `ValueError` if it has no callable ``choose``.
    pub fn new(model: &Bound<'_, PyAny>) -> PyResult<Self> {
        if !model.getattr("choose").is_ok_and(|c| c.is_callable()) {
            return Err(PyValueError::new_err(
                "a choice model is a name from choice_models() or an object with a choose(batch) method",
            ));
        }
        let text = |attr: &str| -> Option<String> {
            let v = model.getattr(attr).ok()?;
            let v = if v.is_callable() { v.call0().ok()? } else { v };
            v.extract::<String>().ok()
        };
        let name = text("name").unwrap_or_else(|| {
            model.get_type().name().map_or_else(|_| "python".to_string(), |n| n.to_string())
        });
        let descriptor = text("descriptor").unwrap_or_else(|| name.clone());
        let sampled = model.getattr("sampled").ok().and_then(|v| v.extract::<bool>().ok());
        let attributes =
            model.getattr("attributes").ok().and_then(|v| v.extract::<Vec<String>>().ok());
        Ok(Self {
            model: model.clone().unbind(),
            name,
            descriptor,
            sampled: sampled.unwrap_or(true),
            attributes,
        })
    }
}

/// `numpy.asarray(obj, dtype)`, so a list, a tuple or an array of another integer type all do.
fn as_array<'py>(
    numpy: &Bound<'py, PyModule>,
    obj: &Bound<'py, PyAny>,
    dtype: &str,
) -> Result<Bound<'py, PyAny>, ChoiceError> {
    numpy.call_method1("asarray", (obj, dtype)).map_err(|e| {
        ChoiceError::BadAnswer(format!("could not read choose()'s answer as {dtype} values: {e}"))
    })
}

fn failed(e: &PyErr) -> ChoiceError {
    ChoiceError::Failed(e.to_string())
}

impl ChoiceModel for PythonChoice {
    fn name(&self) -> &str {
        &self.name
    }

    fn descriptor(&self) -> String {
        self.descriptor.clone()
    }

    fn is_sampled(&self) -> bool {
        self.sampled
    }

    fn required_attributes(&self) -> Option<Vec<String>> {
        self.attributes.clone()
    }

    fn choose(&self, batch: &ChoiceBatch, rng: &StreamRng) -> Result<Choices, ChoiceError> {
        Python::attach(|py| {
            let arg = build_batch(py, batch, rng).map_err(|e| failed(&e))?;
            let answer =
                self.model.bind(py).call_method1("choose", (arg,)).map_err(|e| failed(&e))?;
            // Either the chosen indices, or (indices, probabilities).
            let (chosen_obj, probability_obj) = match answer.cast::<PyTuple>() {
                Ok(t) if t.len() == 2 => (
                    t.get_item(0).map_err(|e| failed(&e))?,
                    Some(t.get_item(1).map_err(|e| failed(&e))?),
                ),
                _ => (answer, None),
            };
            let numpy = py.import("numpy").map_err(|e| failed(&e))?;
            let chosen_array = as_array(&numpy, &chosen_obj, "int64")?;
            let chosen_read: PyReadonlyArray1<'_, i64> = chosen_array.extract().map_err(|_| {
                ChoiceError::BadAnswer(
                    "choose() must return a one-dimensional array of integers".to_string(),
                )
            })?;
            let mut chosen = Vec::with_capacity(chosen_read.len().unwrap_or(0));
            for (s, &c) in chosen_read.as_slice().map_err(|e| failed(&e.into()))?.iter().enumerate()
            {
                chosen.push(u32::try_from(c).map_err(|_| {
                    ChoiceError::BadAnswer(format!("situation {s} chose alternative {c}"))
                })?);
            }
            let probability = match probability_obj {
                Some(p) => {
                    let array = as_array(&numpy, &p, "float64")?;
                    let read: PyReadonlyArray1<'_, f64> = array.extract().map_err(|_| {
                        ChoiceError::BadAnswer(
                            "the probabilities must be a one-dimensional array of numbers"
                                .to_string(),
                        )
                    })?;
                    read.as_slice().map_err(|e| failed(&e.into()))?.to_vec()
                }
                None => vec![f64::NAN; chosen.len()],
            };
            Ok(Choices { chosen, probability })
        })
    }
}

/// A model from a name (with options) or from a Python object.
///
/// # Errors
///
/// `ValueError` for an unknown name, an unknown or out-of-range option, options
/// given with a Python object, or an object that is not a model.
pub fn make_choice_model(
    spec: Option<&Bound<'_, PyAny>>,
    options: Option<HashMap<String, f64>>,
) -> PyResult<Arc<dyn ChoiceModel>> {
    let to_error = |e: ChoiceError| PyValueError::new_err(e.to_string());
    let opts: BTreeMap<String, f64> = options.clone().unwrap_or_default().into_iter().collect();
    match spec {
        None => Ok(Arc::from(Registry::builtin().create(DEFAULT_MODEL, &opts).map_err(to_error)?)),
        Some(s) => {
            if let Ok(name) = s.extract::<String>() {
                Ok(Arc::from(Registry::builtin().create(&name, &opts).map_err(to_error)?))
            } else {
                if options.is_some_and(|o| !o.is_empty()) {
                    return Err(PyValueError::new_err(
                        "choice_options applies to the built-in models; give a model of your own its settings in its constructor",
                    ));
                }
                Ok(Arc::new(PythonChoice::new(s)?))
            }
        }
    }
}

/// What every trip chose, as numpy arrays (one entry per trip).
///
/// ``route`` is the route taken as an index into the run's route sets and
/// ``rank`` its position in its pair's set (0 is the best); ``pair`` is the
/// pair's index in the route sets and ``alternatives`` how many routes it had.
/// All three are ``-1`` (``alternatives`` 0) for a trip with no route. ``probability``
/// is what the model gave the route taken (``nan`` if it gave none) and
/// ``weight`` how many people the trip stands for.
#[pyclass(name = "RouteChoices", module = "openmobisim._core", frozen)]
pub struct PyRouteChoices {
    trips: usize,
    /// The route taken.
    #[pyo3(get)]
    route: Py<PyArray1<i64>>,
    /// Its position in its pair's set.
    #[pyo3(get)]
    rank: Py<PyArray1<i64>>,
    /// Its pair's index in the route sets.
    #[pyo3(get)]
    pair: Py<PyArray1<i64>>,
    /// How many routes the trip could choose from.
    #[pyo3(get)]
    alternatives: Py<PyArray1<u32>>,
    /// The probability the model gave the route taken.
    #[pyo3(get)]
    probability: Py<PyArray1<f64>>,
    /// The trip's traveller weight.
    #[pyo3(get)]
    weight: Py<PyArray1<u32>>,
}

impl PyRouteChoices {
    pub(crate) fn new(
        py: Python<'_>,
        choices: &RouteChoices,
        sets: &RouteSets,
    ) -> PyResult<Py<Self>> {
        let n = choices.len();
        let (mut route, mut rank, mut pair) =
            (Vec::with_capacity(n), Vec::with_capacity(n), Vec::with_capacity(n));
        for &r in &choices.route {
            if r == NO_ROUTE {
                route.push(-1);
                rank.push(-1);
                pair.push(-1);
            } else {
                let k = sets.key_of_route(r as usize);
                route.push(i64::from(r));
                pair.push(i64::try_from(k).expect("few pairs"));
                rank.push(
                    i64::from(r) - i64::try_from(sets.route_range(k).start).expect("few routes"),
                );
            }
        }
        Py::new(
            py,
            Self {
                trips: n,
                route: route.into_pyarray(py).unbind(),
                rank: rank.into_pyarray(py).unbind(),
                pair: pair.into_pyarray(py).unbind(),
                alternatives: choices.alternatives.clone().into_pyarray(py).unbind(),
                probability: choices.probability.clone().into_pyarray(py).unbind(),
                weight: choices.weight.clone().into_pyarray(py).unbind(),
            },
        )
    }
}

#[pymethods]
impl PyRouteChoices {
    /// How many trips.
    fn __len__(&self) -> usize {
        self.trips
    }

    fn __repr__(&self) -> String {
        format!("RouteChoices({} trips)", self.trips)
    }
}
