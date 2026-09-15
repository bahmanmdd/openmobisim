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

use openmobisim_core_demand::{
    ClassDefaults, Ownership, RawPerson, RawTrip, build_travellers, read_persons_parquet,
    read_trips_parquet,
};
use openmobisim_core_graph::examples::manhattan_grid as build_manhattan_grid;
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_sim::Run as CoreRun;
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::time::Second;
use openmobisim_io_parquet::manifest::Manifest;
use openmobisim_io_parquet::{write_diagnostics, write_events, write_kpis, write_manifest};

/// A road network, opaque from Python — built by [`manhattan_grid`] and
/// passed straight back into [`run_pipeline`].
#[pyclass(name = "Network", module = "openmobisim._core")]
pub struct PyNetwork {
    pub(crate) inner: Arc<RoadNetwork>,
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
    Ok(PyNetwork { inner: Arc::new(network) })
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
))]
#[allow(
    clippy::too_many_arguments,
    reason = "the Phase 1 pipeline boundary; python/openmobisim.Scenario is where this becomes ergonomic"
)]
pub fn run_pipeline(
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
) -> PyResult<PyRunSummary> {
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
    let manifest = Manifest::for_run(&travellers, &result, Second(window_s), default_weight);
    write_manifest(&manifest_path, &manifest).map_err(to_value_error)?;

    Ok(PyRunSummary {
        kpis_path: kpis_path.to_string_lossy().into_owned(),
        diagnostics_path: diagnostics_path.to_string_lossy().into_owned(),
        events_path: events_path.to_string_lossy().into_owned(),
        manifest_path: manifest_path.to_string_lossy().into_owned(),
        total_trips: result.completion.total_trips,
        completed: result.completion.completed,
        truncated: result.completion.truncated,
        no_vehicle_available: result.completion.no_vehicle_available,
        no_feasible_path: result.completion.no_feasible_path,
        total_travel_time_s: result.total_travel_time.get(),
    })
}
