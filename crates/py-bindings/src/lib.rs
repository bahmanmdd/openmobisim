//! PyO3 bindings for openmobisim.
//!
//! This crate is the **only** one that knows Python exists (Foundations §10).
//! Everything else is usable, testable and benchmarkable without an
//! interpreter, which is what keeps the core honest: a physics crate that
//! cannot be benchmarked without starting Python is a physics crate nobody
//! benchmarks.
//!
//! The compiled module is imported as `openmobisim._core`. It is private: the
//! public Python surface is the `openmobisim` package, which re-exports from here.
//!
//! # Phase 1 scope
//!
//! Deliberately thin. It exists so that the wheel-building machinery — abi3,
//! three operating systems, CI — is proven end to end before there is
//! anything interesting to carry across the boundary. The functions below are
//! the ones the Python test suite needs in order to assert that the boundary
//! works and that the core's determinism survives it.

// PyO3 extracts arguments by value: a `#[pyfunction]` cannot take a borrowed
// slice from Python, so the workspace's `needless_pass_by_value` lint does not
// apply at this boundary.
#![allow(clippy::needless_pass_by_value, reason = "PyO3 extracts #[pyfunction] arguments by value")]

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;

use openmobisim_core_types::{
    CODE_VERSION, RNG_SCHEME_VERSION, VERSION,
    reduce::{fixed_order_sum, is_parallel_build},
    rng::{DrawAddress, RngKey, Stream, StreamRng},
    time::{Second, StepGrid},
};

mod choice;
mod network;
mod pipeline;
mod routes;
use network::{PyNetwork, grid_node_lonlat, manhattan_grid, network_read_osm, toy_network};
use pipeline::{PyLinkBins, PyRunSummary, run_pipeline};
use routes::{PyRouteSets, route_methods, route_sets_build};

/// Facts about the compiled core, as a dict.
///
/// These are the fields that go into `manifest.json` and that a bug report
/// should always quote. Keys:
///
/// | Key | Meaning |
/// |---|---|
/// | `version` | The crate version |
/// | `code_version` | Artifact layout and build-semantics version |
/// | `rng_scheme_version` | Seed-derivation scheme version |
/// | `parallel` | Whether this build reduces in parallel |
/// | `target` | The platform triple the wheel was built for |
/// | `profile` | `debug` or `release` |
#[pyfunction]
fn build_info(py: Python<'_>) -> PyResult<Bound<'_, PyDict>> {
    let d = PyDict::new(py);
    d.set_item("version", VERSION)?;
    d.set_item("code_version", CODE_VERSION)?;
    d.set_item("rng_scheme_version", RNG_SCHEME_VERSION)?;
    d.set_item("parallel", is_parallel_build())?;
    d.set_item("target", env!("OPENMOBISIM_TARGET"))?;
    d.set_item("profile", if cfg!(debug_assertions) { "debug" } else { "release" })?;
    Ok(d)
}

/// Sum `values` with the core's fixed-order reduction.
///
/// Exposed so that the Python test suite can assert the determinism guarantee
/// from the outside, across the boundary that users actually cross.
///
/// # Arguments
///
/// * `values` — the numbers to add.
///
/// # Returns
///
/// The sum, identical on every call with the same input on the same platform.
#[pyfunction]
fn fixed_order_sum_f64(values: Vec<f64>) -> f64 {
    fixed_order_sum(&values)
}

/// The loading step containing `second`.
///
/// # Arguments
///
/// * `origin_second` — the scenario origin, in seconds.
/// * `step_seconds` — the loading step length.
/// * `window_seconds` — the window length; must be a whole number of steps.
/// * `second` — the instant to locate, in seconds from the same origin.
///
/// # Returns
///
/// The zero-based step index, clamped to the window.
///
/// # Errors
///
/// `ValueError` if the grid is not valid — a zero step, a window that is not a
/// whole number of steps, or a window running past the end of the clock.
#[pyfunction]
fn step_of(
    origin_second: u32,
    step_seconds: u32,
    window_seconds: u32,
    second: u32,
) -> PyResult<u32> {
    let grid = StepGrid::new(Second(origin_second), step_seconds, window_seconds)
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    Ok(grid.step_of(Second(second)).get())
}

/// A single uniform draw in `[0, 1)` from the choice stream.
///
/// Exposed so the Python tests can assert that a draw is a pure function of
/// its key: the same arguments give the same number, always, and changing any
/// one component changes it.
///
/// # Arguments
///
/// * `master_seed` — the scenario's master seed.
/// * `traveller`, `trip`, `iteration`, `alternative` — the draw's identity.
///
/// # Returns
///
/// A number in `[0, 1)`.
#[pyfunction]
fn choice_draw(
    master_seed: u64,
    traveller: u32,
    trip: u32,
    iteration: u32,
    alternative: u32,
) -> f64 {
    let rng = StreamRng::new(RngKey::from_seed(master_seed), Stream::Choice);
    rng.unit(DrawAddress::from_quad(traveller, trip, iteration, alternative))
}

/// The private compiled core of the `openmobisim` package.
#[pymodule]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", VERSION)?;
    m.add_function(wrap_pyfunction!(build_info, m)?)?;
    m.add_function(wrap_pyfunction!(fixed_order_sum_f64, m)?)?;
    m.add_function(wrap_pyfunction!(step_of, m)?)?;
    m.add_function(wrap_pyfunction!(choice_draw, m)?)?;
    m.add_class::<PyNetwork>()?;
    m.add_class::<PyRunSummary>()?;
    m.add_class::<PyLinkBins>()?;
    m.add_class::<PyRouteSets>()?;
    m.add_class::<choice::PyChoiceBatch>()?;
    m.add_class::<choice::PyRouteChoices>()?;
    m.add_function(wrap_pyfunction!(choice::choice_models, m)?)?;
    m.add_function(wrap_pyfunction!(route_methods, m)?)?;
    m.add_function(wrap_pyfunction!(route_sets_build, m)?)?;
    m.add_function(wrap_pyfunction!(manhattan_grid, m)?)?;
    m.add_function(wrap_pyfunction!(toy_network, m)?)?;
    m.add_function(wrap_pyfunction!(network_read_osm, m)?)?;
    m.add_function(wrap_pyfunction!(grid_node_lonlat, m)?)?;
    m.add_function(wrap_pyfunction!(run_pipeline, m)?)?;
    m.add_function(wrap_pyfunction!(pipeline::equilibration_strategies, m)?)?;
    Ok(())
}
