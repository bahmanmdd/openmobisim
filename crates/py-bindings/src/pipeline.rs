//! The Phase 1 pipeline, exposed to Python: a grid network, demand as a
//! plain list of rows or a `trips.parquet` path, one run, four output
//! artifacts.
//!
//! Deliberately low-level — a network handle, a function, a summary of
//! plain fields. `python/openmobisim`'s `Scenario`/`Run`/`Table` classes are
//! where this becomes the ergonomic API the interface doc describes
//! (behaviour in Python, physics in Rust, Foundations §10); this module's
//! job is only to move data across the boundary correctly.
//!
//! # The demand schema Python sees
//!
//! A trip row is the exact field order of `core_demand::RawTrip`:
//! `(traveller_id, trip_seq, origin_lon, origin_lat, destination_lon,
//! destination_lat, departure_time_s, user_class, weight)`. There is one
//! shape here, not a "simple" one and a "general" one — a traveller whose
//! only available mode is a car, at a fixed departure time, is what this
//! same schema already expresses when `class_defaults` gives their class a
//! car and nothing else (S127); it is not a different input format.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use numpy::{IntoPyArray, PyArray1};

use openmobisim_core_demand::{
    ClassDefaults, Ownership, RawPerson, RawTrip, build_travellers, read_persons_parquet,
    read_trips_parquet,
};
use openmobisim_core_graph::defaults::SignalDefaults;
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_loading::{FidelityLevel, LinkBins};
use openmobisim_core_sim::{FlowMotor, Run as CoreRun};
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::time::Second;
use openmobisim_core_types::units::Duration;

use crate::network::PyNetwork;
use crate::routes::{PyRouteSets, to_options};
use openmobisim_core_routes::Registry;
use openmobisim_io_parquet::manifest::Manifest;
use openmobisim_io_parquet::{
    write_diagnostics, write_events, write_kpis, write_link_bins, write_manifest,
};

/// One `trips.parquet` row, as a plain tuple in `RawTrip`'s field order.
type TripRow = (String, u32, f64, f64, f64, f64, u32, String, Option<u32>);

fn trip_from_row(row: TripRow) -> RawTrip {
    let (
        traveller_id,
        trip_seq,
        origin_lon,
        origin_lat,
        destination_lon,
        destination_lat,
        departure_time_s,
        user_class,
        weight,
    ) = row;
    RawTrip {
        traveller_id,
        trip_seq,
        origin: LonLat::new(origin_lon, origin_lat),
        destination: LonLat::new(destination_lon, destination_lat),
        departure_time: Second(departure_time_s),
        user_class,
        weight,
    }
}

/// One `persons.parquet` row, as a plain tuple in `RawPerson`'s field order.
type PersonRow = (String, Option<bool>, Option<bool>, Option<bool>, Option<String>);

fn person_from_row(row: PersonRow) -> RawPerson {
    let (traveller_id, owns_car, owns_bike, has_transit_pass, user_class) = row;
    RawPerson { traveller_id, owns_car, owns_bike, has_transit_pass, user_class }
}

/// What one run produced, as plain fields. `python/openmobisim`'s `Run` class
/// wraps this into something with `.kpis()`/`.diagnostics()`/`.events()`/
/// `.manifest()` methods, each reading the corresponding path.
#[pyclass(name = "RunSummary", module = "openmobisim._core")]
pub struct PyRunSummary {
    /// Path to the written `kpis.parquet`.
    #[pyo3(get)]
    pub kpis_path: String,
    /// Path to the written `diagnostics.parquet`.
    #[pyo3(get)]
    pub diagnostics_path: String,
    /// Path to the written `events.parquet`.
    #[pyo3(get)]
    pub events_path: String,
    /// Path to the written `manifest.json`.
    #[pyo3(get)]
    pub manifest_path: String,
    /// Path to the written `link_bins.parquet`, if the run recorded per-link
    /// results (S168).
    #[pyo3(get)]
    pub link_bins_path: Option<String>,
    /// The run's master seed (S168).
    #[pyo3(get)]
    pub master_seed: u64,
    /// The run's fingerprint: 16 hex digits of a hash of every input that
    /// decides its results (S168).
    #[pyo3(get)]
    pub fingerprint: String,
    /// Every trip in the demand.
    #[pyo3(get)]
    pub total_trips: u32,
    /// Arrived within the simulation window.
    #[pyo3(get)]
    pub completed: u32,
    /// Still in progress when the window ended (S57).
    #[pyo3(get)]
    pub truncated: u32,
    /// No owned car was at the trip's origin.
    #[pyo3(get)]
    pub no_vehicle_available: u32,
    /// A car was available, but no path existed to the destination.
    #[pyo3(get)]
    pub no_feasible_path: u32,
    /// Total travel time, traveller-weight-scaled (S135, confirmed S136).
    #[pyo3(get)]
    pub total_travel_time_s: f64,
    /// Per-link, per-time-bin results, if the run asked for them (S163).
    #[pyo3(get)]
    pub link_bins: Option<Py<PyLinkBins>>,
    /// The route sets the trips were routed from (S165).
    #[pyo3(get)]
    pub route_sets: Option<Py<PyRouteSets>>,
}

/// Per-link, per-time-bin results (S163), as numpy columns.
///
/// One row per (bin, link) that saw traffic, sorted by bin then link. A row
/// counts the traversals of the link that *finished* in the bin, including
/// those of trips still under way when the window ended.
#[pyclass(name = "LinkBins", module = "openmobisim._core")]
pub struct PyLinkBins {
    inner: LinkBins,
}

#[pymethods]
impl PyLinkBins {
    /// The length of one time bin, in seconds.
    #[getter]
    fn bin_seconds(&self) -> u32 {
        self.inner.bin_seconds()
    }

    /// How many rows.
    fn __len__(&self) -> usize {
        self.inner.len()
    }

    /// Each row's bin index (uint32); bin `b` covers `[b, b + 1) × bin_seconds`.
    fn bins<'py>(&self, py: Python<'py>) -> Bound<'py, PyArray1<u32>> {
        self.inner.bins().to_vec().into_pyarray(py)
    }

    /// Each row's link, as an index into the network's link arrays (uint32).
    fn links<'py>(&self, py: Python<'py>) -> Bound<'py, PyArray1<u32>> {
        self.inner.links().to_vec().into_pyarray(py)
    }

    /// Each row's number of finished traversals, unweighted (uint32).
    fn crossings<'py>(&self, py: Python<'py>) -> Bound<'py, PyArray1<u32>> {
        self.inner.crossings().to_vec().into_pyarray(py)
    }

    /// Each row's traffic that left the link, in PCU, traveller weight and
    /// vehicle size included (float64).
    fn pcu<'py>(&self, py: Python<'py>) -> Bound<'py, PyArray1<f64>> {
        self.inner.pcu().to_vec().into_pyarray(py)
    }

    /// Each row's PCU-weighted traversal time, in PCU·seconds (float64).
    fn pcu_seconds<'py>(&self, py: Python<'py>) -> Bound<'py, PyArray1<f64>> {
        self.inner.pcu_seconds().to_vec().into_pyarray(py)
    }

    fn __repr__(&self) -> String {
        format!("LinkBins({} rows, {} s bins)", self.inner.len(), self.inner.bin_seconds())
    }
}

fn to_value_error<E: std::fmt::Display>(e: E) -> PyErr {
    PyValueError::new_err(e.to_string())
}

/// Run the whole Phase 1 pipeline: demand through S133's placeholder
/// routing through level-0 loading, writing all four output artifacts to
/// `output_dir`.
///
/// Exactly one of `trips`/`trips_path` must be given, and at most one of
/// `persons`/`persons_path` — the in-memory and file forms of the same
/// schema (S97), never both at once for the same input.
///
/// # Errors
///
/// `ValueError` if the trips/persons arguments are not exactly one form
/// each, a given file cannot be read, the demand is empty, or writing an
/// output artifact fails.
#[pyfunction]
#[pyo3(signature = (
    network, run_id, output_dir,
    trips=None, trips_path=None,
    persons=None, persons_path=None,
    class_defaults=None, default_weight=1, window_s=86_400,
    flow_level=0, flow_step_s=300, link_bin_s=None,
    route_method="penalty", route_options=None, master_seed=0,
))]
#[allow(
    clippy::too_many_arguments,
    reason = "the Phase 1 pipeline boundary; python/openmobisim.Scenario is where this becomes ergonomic"
)]
pub fn run_pipeline(
    py: Python<'_>,
    network: PyRef<'_, PyNetwork>,
    run_id: &str,
    output_dir: &str,
    trips: Option<Vec<TripRow>>,
    trips_path: Option<&str>,
    persons: Option<Vec<PersonRow>>,
    persons_path: Option<&str>,
    class_defaults: Option<HashMap<String, (bool, bool, bool)>>,
    default_weight: u32,
    window_s: u32,
    flow_level: u32,
    flow_step_s: u32,
    link_bin_s: Option<u32>,
    route_method: &str,
    route_options: Option<HashMap<String, f64>>,
    master_seed: u64,
) -> PyResult<PyRunSummary> {
    // Refuse a bad method or option before doing any work.
    let generator = Registry::builtin()
        .create(route_method, &to_options(route_options))
        .map_err(to_value_error)?;
    let raw_trips = match (trips, trips_path) {
        (Some(rows), None) => rows.into_iter().map(trip_from_row).collect(),
        (None, Some(path)) => read_trips_parquet(path).map_err(to_value_error)?,
        _ => return Err(PyValueError::new_err("give exactly one of trips or trips_path")),
    };
    let raw_persons = match (persons, persons_path) {
        (Some(rows), None) => rows.into_iter().map(person_from_row).collect(),
        (None, Some(path)) => read_persons_parquet(path).map_err(to_value_error)?,
        (None, None) => Vec::new(),
        (Some(_), Some(_)) => {
            return Err(PyValueError::new_err("give at most one of persons or persons_path"));
        }
    };

    let mut class_defaults_built = ClassDefaults::new();
    if let Some(map) = class_defaults {
        for (class, (car, bike, transit_pass)) in map {
            class_defaults_built =
                class_defaults_built.with_default(class, Ownership { car, bike, transit_pass });
        }
    }

    let mut build_diagnostics = Diagnostics::new();
    let (travellers, trips_table) = build_travellers(
        raw_trips,
        raw_persons,
        &class_defaults_built,
        default_weight,
        &mut build_diagnostics,
    )
    .map_err(to_value_error)?;
    let travellers = Arc::new(travellers);
    let trips_table = Arc::new(trips_table);

    let mut run =
        CoreRun::new(network.inner.clone(), travellers.clone(), trips_table, Second(window_s));
    let level = match flow_level {
        0 => None,
        2 => Some(FidelityLevel::PointQueue),
        3 => Some(FidelityLevel::SpatialQueue),
        4 => Some(FidelityLevel::Full),
        other => {
            return Err(PyValueError::new_err(format!(
                "flow_level must be 0 (free flow), 2, 3 or 4, got {other}"
            )));
        }
    };
    if let Some(level) = level {
        if flow_step_s == 0 {
            return Err(PyValueError::new_err("flow_step_s must be positive"));
        }
        let turns = Arc::new(TurnTable::build(&network.inner, SignalDefaults::SHIPPED));
        run = run.with_flow_motor(FlowMotor::Ltm {
            turns,
            step: Duration(f64::from(flow_step_s)),
            level,
        });
    }
    run = run.with_route_generator(Arc::from(generator));
    if let Some(bin) = link_bin_s {
        if bin == 0 {
            return Err(PyValueError::new_err("link_bin_s must be positive"));
        }
        run = run.with_link_bins(bin);
    }
    run = run.with_master_seed(master_seed);
    // What went in, taken before it runs (S168).
    let description = run.description();
    let mut run_diagnostics = Diagnostics::new();
    let result = run.execute(&mut run_diagnostics);

    let mut diagnostics = build_diagnostics;
    diagnostics.merge(&run_diagnostics);

    let dir = Path::new(output_dir);
    std::fs::create_dir_all(dir).map_err(to_value_error)?;
    let kpis_path = dir.join("kpis.parquet");
    let diagnostics_path = dir.join("diagnostics.parquet");
    let events_path = dir.join("events.parquet");
    let manifest_path = dir.join("manifest.json");

    write_kpis(&kpis_path, run_id, "default", 0, 0, &result).map_err(to_value_error)?;
    write_diagnostics(&diagnostics_path, run_id, &diagnostics).map_err(to_value_error)?;
    write_events(
        &events_path,
        run_id,
        &result.events,
        openmobisim_io_parquet::events::DEFAULT_SAMPLE_RATE,
    )
    .map_err(to_value_error)?;
    let manifest =
        Manifest::for_run(&travellers, &result, Second(window_s), default_weight, &description);
    write_manifest(&manifest_path, &manifest).map_err(to_value_error)?;
    let link_bins_path = match &result.link_bins {
        Some(bins) => {
            let path = dir.join("link_bins.parquet");
            write_link_bins(&path, run_id, bins, &network.inner, &description)
                .map_err(to_value_error)?;
            Some(path.to_string_lossy().into_owned())
        }
        None => None,
    };

    Ok(PyRunSummary {
        kpis_path: kpis_path.to_string_lossy().into_owned(),
        diagnostics_path: diagnostics_path.to_string_lossy().into_owned(),
        events_path: events_path.to_string_lossy().into_owned(),
        manifest_path: manifest_path.to_string_lossy().into_owned(),
        link_bins_path,
        master_seed,
        fingerprint: description.fingerprint_hex(),
        total_trips: result.completion.total_trips,
        completed: result.completion.completed,
        truncated: result.completion.truncated,
        no_vehicle_available: result.completion.no_vehicle_available,
        no_feasible_path: result.completion.no_feasible_path,
        total_travel_time_s: result.total_travel_time.get(),
        link_bins: result.link_bins.map(|inner| Py::new(py, PyLinkBins { inner })).transpose()?,
        route_sets: result
            .route_sets
            .map(|sets| Py::new(py, PyRouteSets::new(Arc::new(sets), network.inner.link_count())))
            .transpose()?,
    })
}
