//! The run pipeline, exposed to Python: a network, the demand as rows or a `trips.parquet`
//! path, one run, its output files.
//!
//! Deliberately low-level — a network handle, a function, a summary of
//! plain fields. `python/openmobisim`'s `Scenario`/`Run`/`Table` classes are
//! where this becomes the ergonomic API
//! (behaviour in Python, physics in Rust); this module's
//! job is only to move data across the boundary correctly.
//!
//! # The demand schema Python sees
//!
//! A trip row is the exact field order of `core_demand::RawTrip`:
//! `(traveller_id, trip_seq, origin_lon, origin_lat, destination_lon,
//! destination_lat, departure_time_s, user_class, weight, mode)`. There is one
//! shape here, not a "simple" one and a "general" one — a traveller whose
//! only available mode is a car, at a fixed departure time, is what this
//! same schema already expresses when `class_defaults` gives their class a
//! car and nothing else; it is not a different input format.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;

use numpy::{IntoPyArray, PyArray1, PyArrayMethods};

use openmobisim_core_demand::aggregate;
use openmobisim_core_demand::{
    ClassDefaults, Mode, Ownership, RawPerson, RawTrip, build_travellers, check_distinct_keys,
    read_persons_parquet, read_trips_parquet,
};
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::layers::{BikeCost, StaticLayer};
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_loading::{FidelityLevel, LinkBins};
use openmobisim_core_sim::services::hubs::RunHubs;
use openmobisim_core_sim::{
    ClassLimits, DemandValues, Disruptions, FlowMotor, LayerSetup, LinkValues, LoadingOptions,
    ModeDefaults, ParkingDefaults, ParkingSetup, Prices, ROUTE_ATTRIBUTES, RoadDisruption,
    Run as CoreRun, Skimmer, StaticLayers, Timings, TransitDisruption, TransitEffect, TransitSetup,
    TripTable, ValueLayer,
};
use openmobisim_core_transit::{TransitDefaults, WindowCut};

use crate::parking::{
    PyParkings, itinerary_choices, itinerary_journeys, parking_bins, parking_places,
    parking_summary, trip_modes, trip_results,
};
use crate::transit::{PyTransit, transit_calls, transit_summary};
use openmobisim_core_types::diagnostics::{Category, DiagKey, Diagnostics, Severity};
use openmobisim_core_types::ids::{EntityId, LinkId, TravellerId, TripId};
use openmobisim_core_types::time::Second;
use openmobisim_core_types::units::Duration;

use crate::choice::{PyRouteChoices, make_choice_model_with_classes};
use crate::network::PyNetwork;
use crate::routes::{PyRouteSets, to_options};
use openmobisim_core_routes::Registry;
use openmobisim_io_parquet::manifest::Manifest;
use openmobisim_io_parquet::{
    write_diagnostics, write_events, write_kpis, write_kpis_csv, write_layer_link_bins,
    write_link_bins, write_manifest, write_trip_results,
};

/// One `trips.parquet` row, as a plain tuple in `RawTrip`'s field order; the
/// last field is the optional `mode`, by name.
type TripRow = (String, u32, f64, f64, f64, f64, u32, String, Option<u32>, Option<String>);

fn trip_from_row(row: TripRow) -> PyResult<RawTrip> {
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
        mode,
    ) = row;
    Ok(RawTrip {
        traveller_id,
        trip_seq,
        origin: LonLat::new(origin_lon, origin_lat),
        destination: LonLat::new(destination_lon, destination_lat),
        departure_time: Second(departure_time_s),
        user_class,
        weight,
        mode: mode.map(|m| Mode::from_name(&m)).transpose().map_err(to_value_error)?,
    })
}

/// A `RawTrip` as the plain tuple [`trip_from_row`] reads.
fn row_from_trip(t: RawTrip) -> TripRow {
    (
        t.traveller_id,
        t.trip_seq,
        t.origin.lon,
        t.origin.lat,
        t.destination.lon,
        t.destination.lat,
        t.departure_time.get(),
        t.user_class,
        t.weight,
        t.mode.map(|m| m.as_str().to_string()),
    )
}

/// `demand_aggregate_space`'s core (S294): every trip end of `trips` moved to the nearest of
/// `zones` (lon, lat); the trips, how many ends moved and how many were at their zone's point.
///
/// # Errors
///
/// `ValueError` for a row with an unknown mode.
#[pyfunction]
pub fn demand_aggregate_space(
    trips: Vec<TripRow>,
    zones: Vec<(f64, f64)>,
) -> PyResult<(Vec<TripRow>, u64, u64)> {
    let mut raw: Vec<RawTrip> = trips.into_iter().map(trip_from_row).collect::<PyResult<_>>()?;
    let zones: Vec<LonLat> = zones.into_iter().map(|(lon, lat)| LonLat::new(lon, lat)).collect();
    let report = aggregate::aggregate_space(&mut raw, &zones);
    Ok((raw.into_iter().map(row_from_trip).collect(), report.moved, report.at_zone_point))
}

/// `demand_aggregate_time`'s core (S294): every departure of `trips` set to `start` (seconds
/// after midnight; the earliest departure rounded down to the quarter hour if `None`).
///
/// # Errors
///
/// `ValueError` for a row with an unknown mode.
#[pyfunction]
#[pyo3(signature = (trips, start=None))]
pub fn demand_aggregate_time(trips: Vec<TripRow>, start: Option<u32>) -> PyResult<Vec<TripRow>> {
    let mut raw: Vec<RawTrip> = trips.into_iter().map(trip_from_row).collect::<PyResult<_>>()?;
    let start = start.map_or_else(|| aggregate::default_start(&raw), Second);
    aggregate::aggregate_time(&mut raw, start);
    Ok(raw.into_iter().map(row_from_trip).collect())
}

/// One `persons.parquet` row, as a plain tuple in `RawPerson`'s field order.
type PersonRow = (String, Option<bool>, Option<bool>, Option<bool>, Option<String>);

fn person_from_row(row: PersonRow) -> RawPerson {
    let (traveller_id, owns_car, owns_bike, has_transit_pass, user_class) = row;
    RawPerson { traveller_id, owns_car, owns_bike, has_transit_pass, user_class }
}

/// The user's traveller and trip values by traveller and trip index: traveller values by
/// external id (an id not in the demand ignored, a traveller not given 0); trip values one per
/// row of the demand as given (`row_keys`, each row's traveller, sequence number and whether it
/// departs inside the window and is run), or keyed by
/// `(traveller id, trip sequence)` (a key not in the demand ignored, a trip not given 0). Names in
/// sorted order, so the attributes and the fingerprint do not depend on a dict's order.
fn demand_values(
    travellers: &openmobisim_core_demand::Travellers,
    trips: &openmobisim_core_demand::Trips,
    row_keys: &[(String, u32, bool)],
    persons: HashMap<String, Vec<(String, f64)>>,
    aligned: HashMap<String, Vec<f64>>,
    keyed: KeyedTripValues,
) -> PyResult<DemandValues> {
    let ids = travellers.external_ids();
    let mut out = DemandValues::new();
    let mut persons: Vec<(String, Vec<(String, f64)>)> = persons.into_iter().collect();
    persons.sort_by(|a, b| a.0.cmp(&b.0));
    for (name, pairs) in persons {
        let mut column = vec![0.0; ids.count() as usize];
        for (id, value) in pairs {
            if let Some(t) = ids.id_of(&id) {
                column[t as usize] = value;
            }
        }
        out = out.with_person(&name, column, &ROUTE_ATTRIBUTES).map_err(PyValueError::new_err)?;
    }
    // A trip's index from its traveller and sequence number: the demand keeps each traveller's
    // trips in sequence order, one per number (S128).
    let mut seqs: HashMap<&str, Vec<u32>> = HashMap::new();
    for (id, seq, _) in row_keys.iter().filter(|k| k.2) {
        seqs.entry(id.as_str()).or_default().push(*seq);
    }
    for list in seqs.values_mut() {
        list.sort_unstable();
        list.dedup();
    }
    let index_of = |id: &str, seq: u32| -> Option<usize> {
        let t = ids.typed_id_of::<TravellerId>(id)?;
        let k = seqs.get(id)?.binary_search(&seq).ok()?;
        travellers.trips_of(t).nth(k).map(|trip| trip.index())
    };
    let n = trips.len() as usize;
    let mut columns: Vec<(String, Vec<f64>)> = Vec::new();
    for (name, values) in aligned {
        if values.len() != row_keys.len() {
            return Err(PyValueError::new_err(format!(
                "trip value {name:?} has {} values; the demand has {} rows",
                values.len(),
                row_keys.len()
            )));
        }
        let mut column = vec![0.0; n];
        let mut set = vec![false; n];
        for ((id, seq, run), value) in row_keys.iter().zip(values) {
            // A repeated (traveller, sequence) row is dropped by the demand: its first counts;
            // a row outside the window is not run (S271).
            if let Some(i) = run.then(|| index_of(id, *seq)).flatten().filter(|&i| !set[i]) {
                column[i] = value;
                set[i] = true;
            }
        }
        columns.push((name, column));
    }
    for (name, entries) in keyed {
        if columns.iter().any(|(n, _)| *n == name) {
            return Err(PyValueError::new_err(format!("trip value {name:?} is given twice")));
        }
        let mut column = vec![0.0; n];
        for (id, seq, value) in entries {
            if let Some(i) = index_of(&id, seq) {
                column[i] = value;
            }
        }
        columns.push((name, column));
    }
    columns.sort_by(|a, b| a.0.cmp(&b.0));
    for (name, column) in columns {
        out = out.with_trip(&name, column, &ROUTE_ATTRIBUTES).map_err(PyValueError::new_err)?;
    }
    Ok(out)
}

/// Without a window given, a run lasts this long from the earliest departure, rounded down to
/// the quarter hour (*an assumption*: the starter mornings' departures span three hours,
/// 06:30–09:30, and a two-hour window would have cut or rescaled them).
pub(crate) const WINDOW_DEFAULT_S: u32 = 3 * 3600;

/// Trips departing on one side of the simulation window.
#[derive(Clone, Copy, Debug, Default)]
struct Outside {
    trips: u32,
    /// The people they stand for (traveller weights summed).
    people: u64,
    /// The earliest and latest of their departures, seconds after midnight.
    first_s: u32,
    last_s: u32,
}

impl Outside {
    fn add(&mut self, at: u32, people: u32) {
        if self.trips == 0 {
            (self.first_s, self.last_s) = (at, at);
        }
        self.first_s = self.first_s.min(at);
        self.last_s = self.last_s.max(at);
        self.trips += 1;
        self.people += u64::from(people);
    }
}

/// The run's simulation window and what in the demand departs outside it.
#[derive(Clone, Copy, Debug)]
struct Window {
    start_s: u32,
    end_s: u32,
    /// Whether the scenario gave the start, and the end.
    start_given: bool,
    end_given: bool,
    /// The demand's trips and people, all of them.
    trips: u32,
    people: u64,
    before: Outside,
    after: Outside,
}

impl Window {
    fn resolve(
        trips: &[RawTrip],
        start: Option<u32>,
        end: Option<u32>,
        default_weight: u32,
    ) -> PyResult<Self> {
        let earliest = trips.iter().map(|t| t.departure_time.get()).min().unwrap_or(0);
        let start_s = start.unwrap_or(earliest / 900 * 900);
        let end_s = end.unwrap_or_else(|| start_s.saturating_add(WINDOW_DEFAULT_S));
        if end_s <= start_s {
            return Err(PyValueError::new_err(format!(
                "the window ends at {end_s} s, not after its start at {start_s} s (seconds after midnight)"
            )));
        }
        let mut w = Self {
            start_s,
            end_s,
            start_given: start.is_some(),
            end_given: end.is_some(),
            trips: 0,
            people: 0,
            before: Outside::default(),
            after: Outside::default(),
        };
        for t in trips {
            let (at, people) = (t.departure_time.get(), t.weight.unwrap_or(default_weight));
            w.trips += 1;
            w.people += u64::from(people);
            if at < start_s {
                w.before.add(at, people);
            } else if at >= end_s {
                w.after.add(at, people);
            }
        }
        Ok(w)
    }

    fn contains(&self, at: Second) -> bool {
        (self.start_s..self.end_s).contains(&at.get())
    }

    /// As a dict for Python, with what the timetable's cut took off, if there was one.
    fn to_dict<'py>(self, py: Python<'py>, cut: Option<WindowCut>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new(py);
        d.set_item("start_s", self.start_s)?;
        d.set_item("end_s", self.end_s)?;
        d.set_item("start_given", self.start_given)?;
        d.set_item("end_given", self.end_given)?;
        d.set_item("default_s", WINDOW_DEFAULT_S)?;
        d.set_item("trips", self.trips)?;
        d.set_item("people", self.people)?;
        for (side, o) in [("before", self.before), ("after", self.after)] {
            d.set_item(format!("{side}_trips"), o.trips)?;
            d.set_item(format!("{side}_people"), o.people)?;
            d.set_item(format!("{side}_first_s"), (o.trips > 0).then_some(o.first_s))?;
            d.set_item(format!("{side}_last_s"), (o.trips > 0).then_some(o.last_s))?;
        }
        d.set_item("timetable_runs_kept", cut.map(|c| c.runs_kept))?;
        d.set_item("timetable_runs_cut", cut.map(|c| c.runs_cut))?;
        d.set_item("timetable_runs_dropped", cut.map(|c| c.runs_dropped))?;
        d.set_item("timetable_calls_dropped", cut.map(|c| c.calls_dropped))?;
        Ok(d)
    }
}

/// Trip values keyed by trip, as Python gives them: per name, `(traveller id, trip sequence,
/// value)` each.
type KeyedTripValues = HashMap<String, Vec<(String, u32, f64)>>;

/// A road disruption as Python gives it: link indices, capacity factor, from and to (s).
type RoadDisruptionRow = (Vec<u32>, f64, f64, f64);

/// What one run produced, as plain fields. `python/openmobisim`'s `Run` class
/// wraps this into something with `.kpis()`/`.diagnostics()`/`.events()`/
/// `.manifest()` methods, each reading the corresponding path.
#[pyclass(name = "RunSummary", module = "openmobisim._core")]
pub struct PyRunSummary {
    /// What a skim needs of the run.
    skimmer: Arc<Skimmer>,
    /// The bike cost the run routed by, for a bike layer a skim prepares itself.
    bike_cost: BikeCost,
    /// Path to the written `kpis.parquet`.
    #[pyo3(get)]
    pub kpis_path: String,
    /// Path to the written `diagnostics.parquet`.
    #[pyo3(get)]
    pub diagnostics_path: String,
    /// Path to the written `events.parquet`.
    #[pyo3(get)]
    pub events_path: String,
    /// Path to the written `trip_results.parquet` (every trip and what became of it); `None`
    /// with the minimal outputs.
    #[pyo3(get)]
    pub trip_results_path: Option<String>,
    /// Path to the written `manifest.json`.
    #[pyo3(get)]
    pub manifest_path: String,
    /// Path to the written `link_bins.parquet`, if the run recorded per-link
    /// results.
    #[pyo3(get)]
    pub link_bins_path: Option<String>,
    /// The run's master seed.
    #[pyo3(get)]
    pub master_seed: u64,
    /// The choice model's name.
    #[pyo3(get)]
    pub choice_model: String,
    /// The equilibration strategy's name.
    #[pyo3(get)]
    pub equilibration: String,
    /// The route update's name: `"none"` if the route sets stayed as
    /// generated.
    #[pyo3(get)]
    pub route_update: String,
    /// What each iteration showed, as numpy arrays by name; see
    /// `Run.convergence()`.
    #[pyo3(get)]
    pub convergence: Py<PyDict>,
    /// Whether the strategy stopped early because it had converged.
    #[pyo3(get)]
    pub converged: bool,
    /// The run's fingerprint: 16 hex digits of a hash of every input that
    /// decides its results.
    #[pyo3(get)]
    pub fingerprint: String,
    /// Every trip in the demand.
    #[pyo3(get)]
    pub total_trips: u32,
    /// Arrived within the simulation window.
    #[pyo3(get)]
    pub completed: u32,
    /// Still in progress when the window ended.
    #[pyo3(get)]
    pub truncated: u32,
    /// No owned car was at the trip's origin.
    #[pyo3(get)]
    pub no_vehicle_available: u32,
    /// A car was available, but no path existed to the destination.
    #[pyo3(get)]
    pub no_feasible_path: u32,
    /// Total travel time, traveller-weight-scaled.
    #[pyo3(get)]
    pub total_travel_time_s: f64,
    /// The completed trips' time in their own vehicles (the drives and the bike rides),
    /// traveller-weight-scaled: the total travel time less the fixed times, walks, waits and
    /// rides.
    #[pyo3(get)]
    pub total_vehicle_time_s: f64,
    /// Per-link, per-time-bin results, if the run asked for them.
    #[pyo3(get)]
    pub link_bins: Option<Py<PyLinkBins>>,
    /// The bike layer's per-link results, if asked for and bikes ran;
    /// `pcu` there counts travellers.
    #[pyo3(get)]
    pub bike_link_bins: Option<Py<PyLinkBins>>,
    /// The walk layer's, likewise.
    #[pyo3(get)]
    pub walk_link_bins: Option<Py<PyLinkBins>>,
    /// Path to `link_bins_bike.parquet`, if written.
    #[pyo3(get)]
    pub link_bins_bike_path: Option<String>,
    /// Path to `link_bins_walk.parquet`, if written.
    #[pyo3(get)]
    pub link_bins_walk_path: Option<String>,
    /// Trips whose mode this run cannot simulate.
    #[pyo3(get)]
    pub mode_not_available: u32,
    /// In a run with a service, the trips whose taxi request was cancelled in its service day
    /// (the trip ends there); `None` in a run without one.
    #[pyo3(get)]
    pub cancelled: Option<u32>,
    /// The outcomes by mode, for each mode that has trips: a dict of the
    /// completion counts and `total_travel_time_s`.
    #[pyo3(get)]
    pub completion_by_mode: Py<PyDict>,
    /// The route sets the trips were routed from.
    #[pyo3(get)]
    pub route_sets: Option<Py<PyRouteSets>>,
    /// Which route each trip took, out of how many.
    #[pyo3(get)]
    pub route_choices: Option<Py<PyRouteChoices>>,
    /// The bike route sets (S298), with `bike_route_choice="doubly_stochastic"`; `None` otherwise.
    #[pyo3(get)]
    pub bike_route_sets: Option<Py<PyRouteSets>>,
    /// Each stated bike trip's route among its set (S298); `None` without bike route sets.
    #[pyo3(get)]
    pub bike_route_choices: Option<Py<PyRouteChoices>>,
    /// Every call of the run's timetable with its times and passengers, as
    /// columns; `None` without a timetable.
    #[pyo3(get)]
    pub transit_calls: Option<Py<PyDict>>,
    /// How transit went, as numbers by name; `None` without a timetable.
    #[pyo3(get)]
    pub transit_summary: Option<Py<PyDict>>,
    /// Path to the written `transit_calls.parquet`, if the run had a timetable.
    #[pyo3(get)]
    pub transit_calls_path: Option<String>,
    /// The simulation window as a dict: `start_s` and `end_s` (seconds after midnight),
    /// whether each was given, the demand's trips and people, those departing before and after
    /// it (left out: `before_trips`, `before_people`, `before_first_s`, `before_last_s`, and
    /// `after_*`), and what the timetable's cut took off (`timetable_*`, `None` without one).
    #[pyo3(get)]
    pub window: Py<PyDict>,
    /// The trips that had no alternative and travelled by car, as columns
    /// (`traveller_id`, `trip_seq`, `departure_s`, `weight`, `origin_lon`, `origin_lat`,
    /// `destination_lon`, `destination_lat`).
    #[pyo3(get)]
    pub fallback_car: Py<PyDict>,
    /// The trips the last choice left with no alternative, and why each mode gave them none,
    /// as columns (`crate::parking::no_alternative`).
    #[pyo3(get)]
    pub no_alternative: Py<PyDict>,
    /// How full each parking was, bin by bin, as columns; `None` without parkings.
    #[pyo3(get)]
    pub parking_bins: Option<Py<PyDict>>,
    /// Where each parking is and what it holds; `None` without parkings.
    #[pyo3(get)]
    pub parking_places: Option<Py<PyDict>>,
    /// What the parkings did, as numbers by name; `None` without parkings.
    #[pyo3(get)]
    pub parking_summary: Option<Py<PyDict>>,
    /// Path to the written `parking_bins.parquet`, if the run had parkings.
    #[pyo3(get)]
    pub parking_bins_path: Option<String>,
    /// The docked bike service's events of the last loading, as columns; `None` without one.
    #[pyo3(get)]
    pub bike_share_events: Option<Py<PyDict>>,
    /// Its trips, as columns; `None` without a service.
    #[pyo3(get)]
    pub bike_share_trips: Option<Py<PyDict>>,
    /// Its docks bin by bin, as columns; `None` without a service.
    #[pyo3(get)]
    pub bike_share_docks: Option<Py<PyDict>>,
    /// Its docks, where each is and its hub; `None` without a service.
    #[pyo3(get)]
    pub bike_share_dock_places: Option<Py<PyDict>>,
    /// Path to the written `bike_share_events.parquet`, if the run had a service.
    #[pyo3(get)]
    pub bike_share_events_path: Option<String>,
    /// Path to the written `bike_share_trips.parquet`, if the run had a service.
    #[pyo3(get)]
    pub bike_share_trips_path: Option<String>,
    /// Path to the written `bike_share_docks.parquet`, if the run had a service.
    #[pyo3(get)]
    pub bike_share_docks_path: Option<String>,
    /// The taxi fleet's events of the last loading, as columns; `None` without one (S302).
    #[pyo3(get)]
    pub taxi_events: Option<Py<PyDict>>,
    /// Its trips, as columns; `None` without a taxi fleet.
    #[pyo3(get)]
    pub taxi_trips: Option<Py<PyDict>>,
    /// Its vehicles bin by bin, as columns; `None` without a taxi fleet.
    #[pyo3(get)]
    pub taxi_vehicles: Option<Py<PyDict>>,
    /// Path to the written `taxi_events.parquet`, if the run had a taxi fleet.
    #[pyo3(get)]
    pub taxi_events_path: Option<String>,
    /// Path to the written `taxi_trips.parquet`, if the run had a taxi fleet.
    #[pyo3(get)]
    pub taxi_trips_path: Option<String>,
    /// Path to the written `taxi_vehicles.parquet`, if the run had a taxi fleet.
    #[pyo3(get)]
    pub taxi_vehicles_path: Option<String>,
    /// What travellers assumed about the services, by name (S303); `None` without a service.
    #[pyo3(get)]
    pub service_assumptions: Option<Py<PyDict>>,
    /// The operator of the service day, `{"name", "parameters"}` (S304); `None` without one.
    #[pyo3(get)]
    pub service_policy: Option<Py<PyDict>>,
    /// Its decisions as columns, as `FleetDay.decisions()` gives them (S304); `None` without an
    /// operator.
    #[pyo3(get)]
    pub service_decisions: Option<Py<PyDict>>,
    /// Path to the written `operator_tasks.parquet`, if the run had an operator.
    #[pyo3(get)]
    pub operator_tasks_path: Option<String>,
    /// Per service, by name: its kind and its service day's summary (`kpis.parquet`'s rows of
    /// the service, by metric), S303; `None` without a service.
    #[pyo3(get)]
    pub service_summaries: Option<Py<PyDict>>,
    /// Each transit, park-and-ride and bike-and-ride trip's choice, as columns;
    /// `None` without such trips.
    #[pyo3(get)]
    pub itinerary_choices: Option<Py<PyDict>>,
    /// Each itinerary trip's chosen journey, leg by leg (`Run.itinerary_journeys`); `None`
    /// without such trips.
    #[pyo3(get)]
    pub itinerary_journeys: Option<Py<PyDict>>,
    /// Every trip's departure and the mode it took, as columns.
    #[pyo3(get)]
    pub trip_modes: Py<PyDict>,
    /// Every trip and what became of it, as columns: `trip_results.parquet`'s, without `run_id`.
    #[pyo3(get)]
    pub trip_results: Py<PyDict>,
    /// What stood still when the last loading's window ended, as numbers by name and
    /// the loops as lists of link indices; `None` at level 0.
    #[pyo3(get)]
    pub gridlock: Option<Py<PyDict>>,
    /// Every reroute of the last loading, as columns.
    #[pyo3(get)]
    pub route_changes: Py<PyDict>,
    /// The full gap's per-trip numbers of the last loading (S293), as columns; `None` for a run of
    /// one loading.
    #[pyo3(get)]
    pub gap_trips: Option<Py<PyDict>>,
    /// What the full gaps are measured in: `"utility"` or `"time"` (S293).
    #[pyo3(get)]
    pub gap_currency: String,
    /// The realised routes of the trips that re-routed and arrived: `trip` and `links`.
    #[pyo3(get)]
    pub route_realised: Py<PyDict>,
    /// Itinerary trips whose chosen line could not be followed, and the mean
    /// difference between expected and realised arrival at the parking of the trips
    /// back, in seconds.
    #[pyo3(get)]
    pub itinerary_replanned: u32,
    /// See [`Self::itinerary_replanned`].
    #[pyo3(get)]
    pub return_mismatch_s: f64,
    /// Wall-clock time by stage: `(stage, iteration or None, seconds)`, in the order the
    /// stages ended; `network_read` first if the network recorded it, `total` last (the run
    /// alone, without the network's reading). Not part of any result: it differs between runs.
    #[pyo3(get)]
    pub timings: Vec<(String, Option<u32>, f64)>,
    /// Path to the written `timings.csv`: the same rows.
    #[pyo3(get)]
    pub timings_path: String,
    /// The links whose peak occupancy in the last loading exceeded their storage: where a
    /// point queue would have spilled back. `link`, `peak_pcu`, `storage_pcu`, worst first (by the
    /// PCU that did not fit); `None`
    /// at level 0.
    #[pyo3(get)]
    pub spillback: Option<Py<PyDict>>,
    /// The last loading's service day, kept to freeze it (S302): `None` without a service.
    share: Option<crate::frozen_day::ShareFreeze>,
}

/// Per-link, per-time-bin results, as numpy columns.
///
/// One row per (bin, link) that saw traffic, sorted by bin then link. A row
/// counts the traversals of the link that *finished* in the bin, including
/// those of trips still under way when the window ended.
#[pyclass(name = "LinkBins", module = "openmobisim._core")]
pub struct PyLinkBins {
    inner: LinkBins,
}

#[pymethods]
impl PyRunSummary {
    /// Seconds from each origin to each destination by `mode` (``"car"``, ``"bike"``,
    /// ``"walk"`` or ``"transit"``), leaving at `departure_s`, as a ``(origins, destinations)``
    /// float64 array: ``nan`` where nothing arrives within `max_s` (a skim; see
    /// ``Run.skim``). `network` is the run's, for a bike or walk layer the run did not prepare.
    #[allow(clippy::too_many_arguments, reason = "the points as columns, and the skim's own")]
    #[pyo3(signature = (network, mode, origin_lon, origin_lat, destination_lon, destination_lat, departure_s, max_s))]
    fn skim<'py>(
        &self,
        py: Python<'py>,
        network: PyRef<'_, PyNetwork>,
        mode: &str,
        origin_lon: Vec<f64>,
        origin_lat: Vec<f64>,
        destination_lon: Vec<f64>,
        destination_lat: Vec<f64>,
        departure_s: f64,
        max_s: f64,
    ) -> PyResult<Bound<'py, numpy::PyArray2<f64>>> {
        if origin_lon.len() != origin_lat.len() || destination_lon.len() != destination_lat.len() {
            return Err(PyValueError::new_err("a point is a longitude and a latitude"));
        }
        if !(departure_s.is_finite() && departure_s >= 0.0 && max_s > 0.0) {
            return Err(PyValueError::new_err(
                "departure_s is a second of the day (0 or more) and max_s above 0",
            ));
        }
        let mode = Mode::from_name(mode).map_err(to_value_error)?;
        let at = |lon: &[f64], lat: &[f64]| -> Vec<LonLat> {
            lon.iter().zip(lat).map(|(&x, &y)| LonLat::new(x, y)).collect()
        };
        let (origins, destinations) =
            (at(&origin_lon, &origin_lat), at(&destination_lon, &destination_lat));
        // A bike or walk layer the run did not prepare: made here, as the run would have.
        let own = match mode {
            Mode::Bike | Mode::Walk => {
                let layer = if mode == Mode::Bike { StaticLayer::Bike } else { StaticLayer::Walk };
                if self.skimmer.layer(layer).is_none() {
                    Some(LayerSetup::new(
                        network.static_layer(layer)?,
                        self.bike_cost,
                        network.inner.defaults().layers,
                    ))
                } else {
                    None
                }
            }
            _ => None,
        };
        let skimmer = self.skimmer.clone();
        let values = py
            .detach(|| {
                skimmer.skim(mode, &origins, &destinations, departure_s, max_s, own.as_ref())
            })
            .map_err(PyValueError::new_err)?;
        let rows = origins.len();
        numpy::PyArray1::from_vec(py, values)
            .reshape([rows, destinations.len()])
            .map_err(|e| PyValueError::new_err(e.to_string()))
    }

    /// What a frozen day's ``setup.json`` records of this run (``Run.freeze``); ``None`` for a
    /// run without a service.
    fn _freeze_setup<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyDict>>> {
        self.share.as_ref().map(|s| s.setup(py)).transpose()
    }

    /// Write the last loading's legs and road times into `folder`, which exists
    /// (``Run.freeze``).
    fn _freeze_write(&self, folder: &str) -> PyResult<()> {
        let share = self
            .share
            .as_ref()
            .ok_or_else(|| PyValueError::new_err("this run has no service: no day to freeze"))?;
        share.write(Path::new(folder))
    }

    /// The last loading's service day in ``fleet_day``'s tables, on the run's `network`: what a
    /// replay of the frozen day reproduces.
    fn _share_day<'py>(
        &self,
        py: Python<'py>,
        network: PyRef<'_, PyNetwork>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let share =
            self.share.as_ref().ok_or_else(|| PyValueError::new_err("this run has no service"))?;
        share.outcome(py, &network)
    }
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

pub(crate) fn to_value_error<E: std::fmt::Display>(e: E) -> PyErr {
    PyValueError::new_err(e.to_string())
}

/// The per-iteration numbers as a dict of numpy arrays, one entry per iteration.
fn convergence_arrays(
    py: Python<'_>,
    reports: &[openmobisim_core_sim::IterationReport],
    modes: &[Mode],
) -> PyResult<Py<PyDict>> {
    let dict = PyDict::new(py);
    let floats = |f: fn(&openmobisim_core_sim::IterationReport) -> f64| -> Vec<f64> {
        reports.iter().map(f).collect()
    };
    dict.set_item(
        "iteration",
        reports.iter().map(|r| r.iteration).collect::<Vec<_>>().into_pyarray(py),
    )?;
    dict.set_item("reselected_share", floats(|r| r.reselected_share).into_pyarray(py))?;
    dict.set_item("changed_share", floats(|r| r.changed_share).into_pyarray(py))?;
    dict.set_item("total_travel_time_s", floats(|r| r.total_travel_time_s).into_pyarray(py))?;
    dict.set_item(
        "completed",
        reports.iter().map(|r| r.completed).collect::<Vec<_>>().into_pyarray(py),
    )?;
    dict.set_item("completed_people", floats(|r| r.completed_people).into_pyarray(py))?;
    dict.set_item(
        "truncated",
        reports.iter().map(|r| r.truncated).collect::<Vec<_>>().into_pyarray(py),
    )?;
    dict.set_item("time_change", floats(|r| r.time_change).into_pyarray(py))?;
    dict.set_item("gap", floats(|r| r.gap).into_pyarray(py))?;
    dict.set_item("gap_network", floats(|r| r.gap_network).into_pyarray(py))?;
    dict.set_item("gap_expected", floats(|r| r.gap_expected).into_pyarray(py))?;
    dict.set_item("gap_excess", floats(|r| r.gap_excess).into_pyarray(py))?;
    dict.set_item("gap_network_excess", floats(|r| r.gap_network_excess).into_pyarray(py))?;
    dict.set_item("incomplete_share", floats(|r| r.incomplete_share).into_pyarray(py))?;
    dict.set_item("gap_flow", floats(|r| r.gap_flow).into_pyarray(py))?;
    dict.set_item("gap_flow_floor", floats(|r| r.gap_flow_floor).into_pyarray(py))?;
    dict.set_item("gap_flow_excess", floats(|r| r.gap_flow_excess).into_pyarray(py))?;
    // The modes the run lists alone (S302): a service's mode only in a run with a service.
    for mode in modes {
        let i = mode.index();
        dict.set_item(
            format!("gap_{}", mode.as_str()),
            reports.iter().map(|r| r.itinerary_gap[i]).collect::<Vec<_>>().into_pyarray(py),
        )?;
    }
    dict.set_item(
        "itinerary_recosted",
        reports.iter().map(|r| r.itinerary_recosted).collect::<Vec<_>>().into_pyarray(py),
    )?;
    dict.set_item("hub_mismatch_s", floats(|r| r.hub_mismatch_s).into_pyarray(py))?;
    // The planned-against-realised part of transit (S299).
    dict.set_item("gap_transit_plan_s", floats(|r| r.gap_transit_plan_s).into_pyarray(py))?;
    dict.set_item("gap_transit_plan_share", floats(|r| r.gap_transit_plan_share).into_pyarray(py))?;
    dict.set_item("mode_changed_share", floats(|r| r.mode_changed_share).into_pyarray(py))?;
    dict.set_item("mode_changed_floor", floats(|r| r.mode_changed_floor).into_pyarray(py))?;
    dict.set_item("itinerary_gap_excess", floats(|r| r.itinerary_gap_excess).into_pyarray(py))?;
    // What the stopping rule reads where the route cost is not time alone (S299).
    dict.set_item("gap_generalised_excess", floats(|r| r.gap_generalised_excess).into_pyarray(py))?;
    // The full gaps (S293); what they are measured in is the run's `gap_currency`.
    dict.set_item("gap_route", floats(|r| r.gap_route).into_pyarray(py))?;
    dict.set_item("gap_route_expected", floats(|r| r.gap_route_expected).into_pyarray(py))?;
    dict.set_item("gap_route_excess", floats(|r| r.gap_route_excess).into_pyarray(py))?;
    dict.set_item("gap_route_network", floats(|r| r.gap_route_network).into_pyarray(py))?;
    dict.set_item(
        "gap_route_network_excess",
        floats(|r| r.gap_route_network_excess).into_pyarray(py),
    )?;
    dict.set_item("gap_mode", floats(|r| r.gap_mode).into_pyarray(py))?;
    dict.set_item("gap_mode_expected", floats(|r| r.gap_mode_expected).into_pyarray(py))?;
    dict.set_item("gap_mode_excess", floats(|r| r.gap_mode_excess).into_pyarray(py))?;
    dict.set_item("excess_min_route", floats(|r| r.excess_min_route).into_pyarray(py))?;
    dict.set_item("excess_min_mode", floats(|r| r.excess_min_mode).into_pyarray(py))?;
    dict.set_item("unfinished_cost_share", floats(|r| r.unfinished_cost_share).into_pyarray(py))?;
    dict.set_item(
        "routes_added",
        reports.iter().map(|r| r.routes_added).collect::<Vec<_>>().into_pyarray(py),
    )?;
    dict.set_item(
        "route_searches",
        reports.iter().map(|r| r.route_searches).collect::<Vec<_>>().into_pyarray(py),
    )?;
    dict.set_item(
        "reroutes",
        reports.iter().map(|r| r.reroutes).collect::<Vec<_>>().into_pyarray(py),
    )?;
    dict.set_item(
        "reroute_searches",
        reports.iter().map(|r| r.reroute_searches).collect::<Vec<_>>().into_pyarray(py),
    )?;
    Ok(dict.unbind())
}

/// The full gap's per-trip numbers of the last loading (S293), as columns: one row per trip
/// measured, in trip order (`trip`, its index in the demand), its generalised minutes in the
/// alternative taken, the best car route offered, the best over the network, what the model
/// expects (over every mode for a trip choosing its mode, over its car routes for the others) and
/// the best over every mode (`NaN` for a trip given its mode), and whether it was unfinished.
fn gap_trip_columns(py: Python<'_>, gap: &openmobisim_core_sim::GapTrips) -> PyResult<Py<PyDict>> {
    let rows: Vec<usize> = (0..gap.len()).filter(|&t| !gap.g_chosen[t].is_nan()).collect();
    let column = |v: &[f64]| rows.iter().map(|&t| v[t]).collect::<Vec<f64>>();
    let expected: Vec<f64> =
        rows.iter()
            .map(|&t| {
                if gap.g_best_all[t].is_nan() {
                    gap.g_expected_set[t]
                } else {
                    gap.g_expected_all[t]
                }
            })
            .collect();
    let d = PyDict::new(py);
    d.set_item(
        "trip",
        rows.iter()
            .map(|&t| u32::try_from(t).expect("trip ids fit in 32 bits"))
            .collect::<Vec<u32>>()
            .into_pyarray(py),
    )?;
    d.set_item("g_chosen_min", column(&gap.g_chosen).into_pyarray(py))?;
    d.set_item("g_best_set_min", column(&gap.g_best_set).into_pyarray(py))?;
    d.set_item("g_best_network_min", column(&gap.g_best_network).into_pyarray(py))?;
    d.set_item("g_expected_min", expected.into_pyarray(py))?;
    d.set_item("g_best_all_min", column(&gap.g_best_all).into_pyarray(py))?;
    d.set_item(
        "unfinished",
        rows.iter().map(|&t| gap.unfinished[t]).collect::<Vec<bool>>().into_pyarray(py),
    )?;
    Ok(d.unbind())
}

/// Run the whole pipeline — demand, route sets, route choice, loading and,
/// under an equilibration, iterations of choice and loading — writing all four
/// output artifacts to `output_dir`. `openmobisim.Scenario` is the public way
/// in; its defaults differ from this function's (`flow_level` 4, `"logit"`,
/// `"msa"`).
///
/// Exactly one of `trips`/`trips_path` must be given, and at most one of
/// `persons`/`persons_path` — the in-memory and file forms of the same
/// schema, never both at once for the same input.
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
    class_defaults=None, class_modes=None, class_options=None, class_limits=None,
    link_values=None, person_values=None, trip_values=None, trip_values_keyed=None,
    road_disruptions=None, transit_disruptions=None, disruptions_known=false, zones=None,
    default_weight=1, window_start_s=None, window_end_s=None,
    flow_level=0, flow_step_s=300, link_bin_s=None,
    route_method=None, route_options=None, master_seed=0,
    choice_model=None, choice_options=None,
    equilibration="free_flow", equilibration_options=None,
    route_update=None, route_update_options=None,
    choice_detour_limit=None, choice_max_routes=None, max_cost_vectors=2, route_cache=false, bike_cost="utility",
    bike_route_choice="shortest", bike_route_options=None, transit=None,
    parkings=None, parking_options=None, transit_options=None, modes=None, mode_options=None,
    loading_options=None, price_options=None, threads=None, fingerprint_only=false,
    design_id="default", replication=0, outputs="all", hub_join_m=150.0,
    services=None, class_no_bike_share=None, class_no_taxi=None, service_assumptions=None,
    policy=None,
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
    class_modes: Option<HashMap<String, Vec<String>>>,
    class_options: Option<HashMap<String, HashMap<String, f64>>>,
    class_limits: Option<HashMap<String, HashMap<String, f64>>>,
    link_values: Option<HashMap<String, HashMap<String, Vec<f64>>>>,
    person_values: Option<HashMap<String, Vec<(String, f64)>>>,
    trip_values: Option<HashMap<String, Vec<f64>>>,
    trip_values_keyed: Option<KeyedTripValues>,
    road_disruptions: Option<Vec<RoadDisruptionRow>>,
    transit_disruptions: Option<Vec<(u32, i64, u32, u32)>>,
    disruptions_known: bool,
    zones: Option<Vec<(f64, f64)>>,
    default_weight: u32,
    window_start_s: Option<u32>,
    window_end_s: Option<u32>,
    flow_level: u32,
    flow_step_s: u32,
    link_bin_s: Option<u32>,
    route_method: Option<&str>,
    route_options: Option<HashMap<String, f64>>,
    master_seed: u64,
    choice_model: Option<&Bound<'_, PyAny>>,
    choice_options: Option<HashMap<String, f64>>,
    equilibration: &str,
    equilibration_options: Option<HashMap<String, f64>>,
    route_update: Option<&str>,
    route_update_options: Option<HashMap<String, f64>>,
    choice_detour_limit: Option<f64>,
    choice_max_routes: Option<u32>,
    max_cost_vectors: u32,
    route_cache: bool,
    bike_cost: &str,
    bike_route_choice: &str,
    bike_route_options: Option<HashMap<String, f64>>,
    transit: Option<PyRef<'_, PyTransit>>,
    parkings: Option<PyRef<'_, PyParkings>>,
    parking_options: Option<HashMap<String, f64>>,
    transit_options: Option<HashMap<String, f64>>,
    modes: Option<Vec<String>>,
    mode_options: Option<HashMap<String, f64>>,
    loading_options: Option<HashMap<String, f64>>,
    price_options: Option<HashMap<String, f64>>,
    threads: Option<usize>,
    fingerprint_only: bool,
    design_id: &str,
    replication: u32,
    outputs: &str,
    hub_join_m: f64,
    services: Option<Vec<crate::services::ServiceTuple<'_>>>,
    class_no_bike_share: Option<Vec<String>>,
    class_no_taxi: Option<Vec<String>>,
    service_assumptions: Option<std::collections::BTreeMap<String, f64>>,
    policy: Option<crate::policy::PolicyTuple<'_>>,
) -> PyResult<Py<PyAny>> {
    // What travellers assume about the services (S303): the shipped values, those given by
    // name replacing them.
    let assumptions = match &service_assumptions {
        None => openmobisim_core_sim::services::ServiceAssumptions::SHIPPED,
        Some(values) => openmobisim_core_sim::services::ServiceAssumptions::SHIPPED
            .with_options(values)
            .map_err(PyValueError::new_err)?,
    };
    // Which files the run writes (C2): "all", or "minimal" (kpis.csv and manifest.json) for mass
    // runs; the results in memory are the same either way.
    let minimal = match outputs {
        "all" => false,
        "minimal" => true,
        other => {
            return Err(PyValueError::new_err(format!(
                "outputs must be \"all\" or \"minimal\", got {other:?}"
            )));
        }
    };
    if !(hub_join_m.is_finite() && hub_join_m >= 0.0) {
        return Err(PyValueError::new_err(format!(
            "hub_join_m must be 0 or more metres, got {hub_join_m}"
        )));
    }
    // The run's thread count (S277): `None` for every logical core; never 0.
    let threads = match threads {
        Some(0) => return Err(PyValueError::new_err("threads must be at least 1, or None")),
        n => n.and_then(std::num::NonZeroUsize::new),
    };
    let started = Instant::now();
    let mut clock = started;
    let mut timings = Timings::new();
    let bike_cost = BikeCost::from_name(bike_cost).ok_or_else(|| {
        PyValueError::new_err(format!(
            "bike_cost must be \"utility\", \"dedicated\" or \"time\", not {bike_cost:?}"
        ))
    })?;
    if let Some(limit) = choice_detour_limit {
        if !(limit.is_finite() && limit >= 0.0) {
            return Err(PyValueError::new_err(format!(
                "choice_detour_limit must be 0 (offer every route) or more, got {limit}"
            )));
        }
    }
    // Refuse a bad method, model or option before doing any work.
    let class_options: BTreeMap<String, BTreeMap<String, f64>> = class_options
        .unwrap_or_default()
        .into_iter()
        .map(|(class, o)| (class, o.into_iter().collect()))
        .collect();
    let by_name: Vec<(String, BTreeMap<String, f64>)> =
        class_options.iter().map(|(c, o)| (c.clone(), o.clone())).collect();
    make_choice_model_with_classes(choice_model, choice_options.clone(), &by_name)?;
    let class_modes: HashMap<String, [bool; Mode::COUNT]> = class_modes
        .unwrap_or_default()
        .into_iter()
        .map(|(class, names)| {
            let mut allowed = [false; Mode::COUNT];
            for name in &names {
                allowed[Mode::from_name(name).map_err(to_value_error)?.index()] = true;
            }
            Ok((class, allowed))
        })
        .collect::<PyResult<_>>()?;
    // Each class's own choice-set limits (S235), refused by name before any work.
    let class_limits: HashMap<String, ClassLimits> = class_limits
        .unwrap_or_default()
        .into_iter()
        .map(|(class, options)| {
            let limits = ClassLimits::from_options(&options.into_iter().collect())
                .map_err(|e| PyValueError::new_err(format!("class {class:?}: {e}")))?;
            Ok((class, limits))
        })
        .collect::<PyResult<_>>()?;
    // The user's link values (S236), by layer and name in sorted order, so the attributes and
    // the fingerprint do not depend on a dict's order.
    let link_values = {
        let mut out = LinkValues::new();
        let mut layers: Vec<(String, HashMap<String, Vec<f64>>)> =
            link_values.unwrap_or_default().into_iter().collect();
        layers.sort_by(|a, b| a.0.cmp(&b.0));
        for (layer, columns) in layers {
            let value_layer = ValueLayer::from_name(&layer).ok_or_else(|| {
                PyValueError::new_err(format!(
                    "link_values has no layer {layer:?}; the layers are road, bike and walk"
                ))
            })?;
            let mut columns: Vec<(String, Vec<f64>)> = columns.into_iter().collect();
            columns.sort_by(|a, b| a.0.cmp(&b.0));
            for (name, values) in columns {
                out = out
                    .with_column(value_layer, &name, values, &ROUTE_ATTRIBUTES)
                    .map_err(PyValueError::new_err)?;
            }
        }
        out
    };
    let transit_defaults =
        TransitDefaults::from_options(&to_options(transit_options)).map_err(to_value_error)?;
    let parking_defaults =
        ParkingDefaults::from_options(&to_options(parking_options)).map_err(to_value_error)?;
    let mode_defaults =
        ModeDefaults::from_options(&to_options(mode_options)).map_err(PyValueError::new_err)?;
    let loading = LoadingOptions::from_options(&to_options(loading_options))
        .map_err(PyValueError::new_err)?;
    let prices = Prices::from_options(&to_options(price_options)).map_err(PyValueError::new_err)?;
    // The modes a trip without a stated mode chooses among (M5); none: no choice.
    let modes: Vec<Mode> = modes
        .unwrap_or_default()
        .iter()
        .map(|name| Mode::from_name(name))
        .collect::<Result<_, _>>()
        .map_err(to_value_error)?;
    if parkings.is_some() && transit.is_none() {
        return Err(PyValueError::new_err(
            "parkings serve park-and-ride and bike-and-ride, which need a timetable: give transit= too",
        ));
    }
    let strategy = openmobisim_core_sim::equilibration::strategy(
        equilibration,
        &to_options(equilibration_options),
    )
    .map_err(to_value_error)?;
    // The static assignment's solver at level 1 alone, and no self-regulated averaging there
    // (S294).
    if equilibration == "frank_wolfe" && flow_level != 1 {
        return Err(PyValueError::new_err(format!(
            "equilibration \"frank_wolfe\" is the static assignment's solver: give flow_level 1, \
             or another strategy at flow_level {flow_level}"
        )));
    }
    if equilibration == "sra" && flow_level == 1 {
        return Err(PyValueError::new_err(
            "equilibration \"sra\" does not apply at flow_level 1: the static assignment is \
             solved by \"frank_wolfe\" or \"msa\" (the method of successive averages)",
        ));
    }
    // The route method and the route update unless named (S179, S223; here since S280, C1 —
    // one default table): a run that loads more than once (iterations, or the free-flow
    // loading's groups at a level with queues) starts from one route per pair and lets the
    // update find the rest; a run that loads once has no later loading to choose among new
    // routes, so it gets the penalty method's set and no update. Options given for a method
    // mean the default method of a single loading.
    let iterates = equilibration != "free_flow" && strategy.max_iterations() > 1;
    let reloads = iterates || (flow_level >= 2 && strategy.increments() > 1);
    let route_method = route_method.unwrap_or(if reloads && route_options.is_none() {
        "shortest"
    } else {
        "penalty"
    });
    let route_update = route_update.unwrap_or(if reloads || route_update_options.is_some() {
        "best_response"
    } else {
        "none"
    });
    let generator = Registry::builtin()
        .create(route_method, &to_options(route_options))
        .map_err(to_value_error)?;
    let update =
        openmobisim_core_sim::route_update::update(route_update, &to_options(route_update_options))
            .map_err(to_value_error)?;
    let mut raw_trips: Vec<RawTrip> = match (trips, trips_path) {
        (Some(rows), None) => rows.into_iter().map(trip_from_row).collect::<PyResult<_>>()?,
        (None, Some(path)) => read_trips_parquet(path).map_err(to_value_error)?,
        _ => return Err(PyValueError::new_err("give exactly one of trips or trips_path")),
    };
    // A repeated (traveller_id, trip_seq) is refused with the row named (X-68), on the rows as
    // given, before the window leaves any out, so the row number is the user's.
    check_distinct_keys(&raw_trips).map_err(to_value_error)?;
    let raw_persons = match (persons, persons_path) {
        (Some(rows), None) => rows.into_iter().map(person_from_row).collect(),
        (None, Some(path)) => read_persons_parquet(path).map_err(to_value_error)?,
        (None, None) => Vec::new(),
        (Some(_), Some(_)) => {
            return Err(PyValueError::new_err("give at most one of persons or persons_path"));
        }
    };

    // A class the scenario does not list may use every mode, so it owns what they need (S243):
    // a car and a bike; transit needs no pass, and a pass is held only where said (U-42).
    let mut class_defaults_built = ClassDefaults::new().with_fallback(Ownership::VEHICLES);
    if let Some(map) = class_defaults {
        for (class, (car, bike, transit_pass)) in map {
            class_defaults_built =
                class_defaults_built.with_default(class, Ownership { car, bike, transit_pass });
        }
    }

    // Each row's traveller and place in their day, for trip values given row by row (S249).
    // The simulation window (S271, W-2): the scenario's, or from the earliest departure rounded
    // down to the quarter hour, for `WINDOW_DEFAULT_S`. A trip departing outside it is left out
    // and reported (U-34 (b)): a warning, not an error.
    let window = Window::resolve(&raw_trips, window_start_s, window_end_s, default_weight)?;
    let row_keys: Vec<(String, u32, bool)> = if trip_values.as_ref().is_some_and(|v| !v.is_empty())
        || trip_values_keyed.as_ref().is_some_and(|v| !v.is_empty())
    {
        raw_trips
            .iter()
            .map(|t| (t.traveller_id.clone(), t.trip_seq, window.contains(t.departure_time)))
            .collect()
    } else {
        Vec::new()
    };
    raw_trips.retain(|t| window.contains(t.departure_time));
    let mut build_diagnostics = Diagnostics::new();
    // Level 1 (S294): the period the hourly flows are of, read from the departures before
    // they move (unless given); then every departure at the window's start and every trip end
    // at its zone's point.
    let mut static_period_s = 0;
    if flow_level == 1 {
        static_period_s = if loading.period_s > 0 {
            loading.period_s
        } else {
            aggregate::departure_period_s(&raw_trips).max(aggregate::QUARTER_HOUR_S)
        };
        let moved = aggregate::aggregate_time(&mut raw_trips, Second(window.start_s));
        let zone_points: Vec<LonLat> =
            zones.unwrap_or_default().into_iter().map(|(lon, lat)| LonLat::new(lon, lat)).collect();
        let space = aggregate::aggregate_space(&mut raw_trips, &zone_points);
        for (code, n) in [
            (aggregate::codes::DEPARTURE_MOVED_TO_WINDOW_START, moved),
            (aggregate::codes::TRIP_END_MOVED_TO_ZONE_POINT, space.moved),
        ] {
            if n > 0 {
                build_diagnostics
                    .record_n(DiagKey::run_level(Category::Modelling, code, Severity::Info), n);
            }
        }
    }
    let (travellers, trips_table) = build_travellers(
        raw_trips,
        raw_persons,
        &class_defaults_built,
        default_weight,
        &mut build_diagnostics,
    )
    .map_err(to_value_error)?;
    // Traveller classes (S231), by their index in the demand: the modes each may use (a class
    // not given any: all the run offers) and its own coefficients (none: the model's).
    let class_names: Vec<String> = {
        let ids = travellers.class_external_ids();
        (0..ids.count()).map(|c| ids.external(c).to_string()).collect()
    };
    // The service modes (S302) are governed by `has_bike_share` and `has_taxi` alone, not by a
    // class's `modes` list (which names the ordinary modes): with a service every class is
    // offered its modes (a shared bike alone or to transit; a taxi) unless the column is 0.
    let no_share = class_no_bike_share.unwrap_or_default();
    let no_taxi = class_no_taxi.unwrap_or_default();
    let class_allowed: Vec<[bool; Mode::COUNT]> = class_names
        .iter()
        .map(|c| {
            let mut allowed = class_modes.get(c).copied().unwrap_or([true; Mode::COUNT]);
            let share = !no_share.contains(c);
            allowed[Mode::BikeShare.index()] = share;
            allowed[Mode::BikeShareTransit.index()] = share;
            allowed[Mode::Taxi.index()] = !no_taxi.contains(c);
            allowed
        })
        .collect();
    let class_limits: Vec<ClassLimits> =
        class_names.iter().map(|c| class_limits.get(c).copied().unwrap_or_default()).collect();
    // The walk to a stop is searched as far as the longest any class in the demand takes.
    let access_search_s =
        class_limits.iter().filter_map(|l| l.access_walk_max_s).fold(0.0, f64::max);
    let by_index: Vec<(String, BTreeMap<String, f64>)> = class_names
        .iter()
        .map(|c| (c.clone(), class_options.get(c).cloned().unwrap_or_default()))
        .collect();
    let choice = make_choice_model_with_classes(choice_model, choice_options, &by_index)?;
    let demand_values = demand_values(
        &travellers,
        &trips_table,
        &row_keys,
        person_values.unwrap_or_default(),
        trip_values.unwrap_or_default(),
        trip_values_keyed.unwrap_or_default(),
    )?;
    clock = timings.lap("demand_read", None, clock);
    let travellers = Arc::new(travellers);
    let trips_table = Arc::new(trips_table);
    let trips_used = trips_table.clone();

    let mut run =
        CoreRun::new(network.inner.clone(), travellers.clone(), trips_table, Second(window.end_s))
            .with_window_start(Second(window.start_s));
    let level = match flow_level {
        0 | 1 => None,
        2 => Some(FidelityLevel::PointQueue),
        3 => Some(FidelityLevel::SpatialQueue),
        4 => Some(FidelityLevel::Full),
        other => {
            return Err(PyValueError::new_err(format!(
                "flow_level must be 0 (free flow), 1 (static), 2, 3 or 4, got {other}"
            )));
        }
    };
    let mut run_turns: Option<Arc<TurnTable>> = None;
    if flow_level == 1 {
        let turns = Arc::new(TurnTable::build(&network.inner, network.inner.signals()));
        run_turns = Some(turns.clone());
        run = run.with_flow_motor(FlowMotor::Static { turns, period_s: static_period_s });
    }
    if let Some(level) = level {
        if flow_step_s == 0 {
            return Err(PyValueError::new_err("flow_step_s must be positive"));
        }
        let turns = Arc::new(TurnTable::build(&network.inner, network.inner.signals()));
        run_turns = Some(turns.clone());
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
    run = run
        .with_master_seed(master_seed)
        .with_choice_model(choice)
        .with_equilibration(Arc::from(strategy))
        .with_route_update(Arc::from(update));
    if let Some(limit) = choice_detour_limit {
        run = run.with_choice_detour_limit(limit);
    }
    if let Some(n) = choice_max_routes {
        run = run.with_choice_max_routes(n);
    }
    if max_cost_vectors == 0 {
        return Err(PyValueError::new_err("max_cost_vectors must be 1 or more"));
    }
    run = run.with_max_cost_vectors(max_cost_vectors);
    // Bike route sets (S298): an option; "shortest" is every run's one route per pair.
    // "sampled", the old name of "doubly_stochastic" (S299), selects the same.
    match bike_route_choice {
        "shortest" => {
            if bike_route_options.is_some() {
                return Err(PyValueError::new_err(
                    "bike_route_options are the doubly stochastic method's: give \
                     bike_route_choice=\"doubly_stochastic\"",
                ));
            }
        }
        "doubly_stochastic" | "sampled" => {
            let generator =
                openmobisim_core_routes::Sampled::from_options(&to_options(bike_route_options))
                    .map_err(to_value_error)?;
            run = run.with_bike_sampled(generator);
        }
        other => {
            return Err(PyValueError::new_err(format!(
                "bike_route_choice must be \"shortest\" or \"doubly_stochastic\", got {other:?}"
            )));
        }
    }
    if route_cache {
        run = run.with_route_cache(route_cache_handle().clone());
    }
    // The bike and walk layers, prepared only for a run whose trips use them, or may choose
    // them.
    let unstated = (0..trips_used.len()).any(|i| !trips_used.mode_given(TripId::new(i)));
    let uses = |mode: Mode| {
        (0..trips_used.len()).any(|i| trips_used.mode(TripId::new(i)) == mode)
            || unstated && modes.contains(&mode)
    };
    let setup = |layer: StaticLayer| -> PyResult<LayerSetup> {
        Ok(LayerSetup::new(
            network.static_layer(layer)?,
            bike_cost,
            network.inner.defaults().layers,
        ))
    };
    // A timetable walks its passengers on the walk layer (S199), so the run needs it; a docked
    // service's docks are snapped to both layers, and a taxi request that fails walks on (S302).
    let services = services.unwrap_or_default();
    let docked = services.iter().any(|s| s.0 == "bike_docked");
    let layers = StaticLayers {
        bike: if uses(Mode::Bike) || uses(Mode::BikeTransit) || docked {
            Some(setup(StaticLayer::Bike)?)
        } else {
            None
        },
        walk: if uses(Mode::Walk) || transit.is_some() || !services.is_empty() {
            Some(setup(StaticLayer::Walk)?)
        } else {
            None
        },
    };
    // Stops with walk and bike access points, and buses on this network's roads if asked.
    let mut window_cut = None;
    let transit_setup = match &transit {
        Some(t) => {
            let walk = layers.walk.as_ref().expect("made above for a timetable");
            let bike = setup(StaticLayer::Bike)?;
            // The window's part of the day and the planning horizon past its end (S271, S279):
            // each run's calls from the window's start to its end plus the longest journey a
            // trip may take, so a trip leaving late in the window still sees the journeys that
            // end after it, as it sees its car and bike alternatives that would. The loading
            // stops at the end, and the outputs keep the window's calls alone.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "seconds")]
            let horizon = window.end_s.saturating_add(transit_defaults.journey_max_s.ceil() as u32);
            let (cut, report) = t.timetable.within(window.start_s, horizon);
            window_cut = Some(report);
            let built = TransitSetup::new(Arc::new(cut), walk, Some(&bike), transit_defaults)
                .with_access_search_s(access_search_s);
            // Buses ride the roads only when asked (S299); else every bus keeps to its timetable.
            let built = if transit_defaults.buses_on_roads {
                built.with_roads(network.inner.clone())
            } else {
                built
            };
            Some(Arc::new(built))
        }
        None => None,
    };
    // Parkings as hubs on the layers, with the stops around them (M4).
    let parking_setup = match (&parkings, &transit_setup) {
        (Some(p), Some(t)) => {
            let bike = setup(StaticLayer::Bike)?;
            let built =
                ParkingSetup::new(&p.rows, network.inner.clone(), Some(&bike), t, parking_defaults)
                    .map_err(to_value_error)?;
            Some(Arc::new(built))
        }
        _ => None,
    };
    // The services (S302), snapped to the run's walk and bike layers.
    let service_setups = crate::services::setups_on(&services, &layers, &network.inner)?;
    // Their docks as resources of the run's one hub set, in service and dock order: a dock
    // within `hub_join_m` of a stop joins its hub (none without a service).
    let service_rows: Vec<openmobisim_core_sim::services::hubs::ResourceRow> =
        service_setups.iter().flat_map(|s| s.resource_rows()).collect();
    let layers = Arc::new(layers);
    // The bike layer's link count, for its route sets' Python view (S298).
    let bike_link_count =
        layers.get(StaticLayer::Bike).map_or(0, |s| s.network().network().link_count());
    run = run.with_layers(layers.clone());
    if let Some(t) = &transit_setup {
        run = run.with_transit(t.clone());
    }
    if let Some(p) = &parking_setup {
        run = run.with_parking(p.clone());
    }
    let service_setups = (!service_setups.is_empty()).then(|| Arc::new(service_setups));
    if let Some(setups) = &service_setups {
        run = run
            .with_services(Arc::clone(setups))
            .with_hub_join_m(hub_join_m)
            .with_service_assumptions(assumptions);
    }
    // The operator of the service day (S304), as `fleet_day` takes it; the ids it names
    // decisions by are the services' and the network's.
    let raised = match (&policy, &service_setups) {
        (Some(_), None) => {
            return Err(PyValueError::new_err(
                "policy steers a run's services: give services too, or no policy",
            ));
        }
        (_, Some(setups)) => {
            let ids = Arc::new(crate::policy::Ids::new(setups.as_slice(), network.inner.clone()));
            let (chosen, raised) = crate::policy::policy_from_py(policy, Arc::clone(&ids))?;
            if let Some(p) = chosen {
                run = run.with_service_policy(p);
            }
            raised
        }
        (None, None) => Arc::new(std::sync::Mutex::new(None)),
    };
    run = run
        .with_mode_choice(&modes)
        .with_class_modes(class_allowed)
        .with_class_limits(class_limits)
        .with_link_values(link_values)
        .with_demand_values(demand_values)
        .with_disruptions(Disruptions {
            road: road_disruptions
                .unwrap_or_default()
                .into_iter()
                .map(|(links, factor, from_s, to_s)| RoadDisruption {
                    links: links.into_iter().map(LinkId::new).collect(),
                    factor,
                    from_s,
                    to_s,
                })
                .collect(),
            transit: transit_disruptions
                .unwrap_or_default()
                .into_iter()
                .map(|(route, delay, from_s, to_s)| TransitDisruption {
                    route,
                    // A negative delay cancels.
                    effect: u32::try_from(delay)
                        .map_or(TransitEffect::Cancel, TransitEffect::Delay),
                    from_s,
                    to_s,
                })
                .collect(),
            known: disruptions_known,
        })
        .with_mode_defaults(mode_defaults)
        .with_loading_options(loading)
        .with_prices(prices)
        .with_threads(threads);
    // What went in, taken before it runs (S168).
    let description = run.description();
    // Only the fingerprint (S277): what the run would be, without running it.
    if fingerprint_only {
        return Ok(description.fingerprint_hex().into_pyobject(py)?.into_any().unbind());
    }
    timings.lap("setup", None, clock);
    let mut run_diagnostics = Diagnostics::new();
    // Python's lock released (S277): the run works on a pool of its own, and a choice model in
    // Python is called from that pool's thread, which takes the lock when it needs it.
    let result = py.detach(|| run.try_execute(&mut run_diagnostics)).map_err(|e| {
        // A Python operator's own exception is raised as it was (S304).
        match e {
            openmobisim_core_sim::run::RunError::Operator(_) => raised
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
                .unwrap_or_else(|| to_value_error(e)),
            e => to_value_error(e),
        }
    })?;
    let core_stages = run.timings().to_vec();
    // A run with a service keeps its service day, to freeze it (S302, S303); one without keeps
    // nothing.
    let share = service_setups.as_ref().map(|setups| {
        crate::frozen_day::ShareFreeze::new(
            Arc::clone(setups),
            (
                result.services.as_ref().and_then(|s| s.last.clone()),
                result.taxis.as_ref().and_then(|t| t.last.clone()),
            ),
            &trips_used,
            &travellers,
            result.service_road_times.clone(),
            &network.inner,
            (f64::from(window.start_s), f64::from(window.end_s)),
            master_seed,
            description.fingerprint_hex(),
        )
    });
    // What a skim needs after the run (S238), kept with the summary.
    let skimmer = Arc::new(Skimmer::new(
        network.inner.clone(),
        run_turns,
        run.final_link_times(),
        layers,
        transit_setup.clone().map(|t| (t, result.transit.as_ref().map(|r| r.times.clone()))),
    ));
    let mut clock = Instant::now();

    let mut diagnostics = build_diagnostics;
    diagnostics.merge(&run_diagnostics);

    // The run's one hub set; each stop's and parking's hub id is the `hub_id` the outputs write.
    // Its stops and parkings come before the services' docks, so their hubs are the same with
    // services or without; `hub_join_m` reaches the fingerprint only with a service, whose docks
    // it joins to stops.
    let run_hubs = RunHubs::build(
        transit_setup.as_deref(),
        &[],
        parking_setup.as_deref(),
        &service_rows,
        hub_join_m,
    );
    let (stop_hubs, parking_hubs) = (run_hubs.stop_hub_ids(), run_hubs.parking_hub_ids());
    let dir = Path::new(output_dir);
    std::fs::create_dir_all(dir).map_err(to_value_error)?;
    let kpis_path = dir.join(if minimal { "kpis.csv" } else { "kpis.parquet" });
    let diagnostics_path = dir.join("diagnostics.parquet");
    let events_path = dir.join("events.parquet");
    let manifest_path = dir.join("manifest.json");
    // Every trip and what became of it (S283): `trip_results.parquet` and `Run.trip_results()`. A trip
    // without a stated mode counts as the run's one mode, if it has one.
    let mut distinct = modes.clone();
    distinct.sort_unstable();
    distinct.dedup();
    let one_mode = if let [one] = distinct.as_slice() { Some(*one) } else { None };
    let trip_table = TripTable::new(&result, &trips_used, &travellers, one_mode);
    let mut trip_results_path = None;

    // C2: the design and replication a batch names ("default" and 0 for a single run); like the
    // run's id, labels and not inputs, so not in the fingerprint.
    if minimal {
        write_kpis_csv(&kpis_path, run_id, design_id, replication, 0, &result)
            .map_err(to_value_error)?;
    } else {
        write_kpis(&kpis_path, run_id, design_id, replication, 0, &result)
            .map_err(to_value_error)?;
        write_diagnostics(&diagnostics_path, run_id, &diagnostics).map_err(to_value_error)?;
        write_events(
            &events_path,
            run_id,
            &result.events,
            openmobisim_io_parquet::events::DEFAULT_SAMPLE_RATE,
        )
        .map_err(to_value_error)?;
        let path = dir.join("trip_results.parquet");
        write_trip_results(&path, run_id, &trip_table, &description).map_err(to_value_error)?;
        trip_results_path = Some(path.to_string_lossy().into_owned());
    }
    let mut manifest =
        Manifest::for_run(&travellers, &result, Second(window.end_s), default_weight, &description);
    manifest.window_start_seconds = window.start_s;
    manifest.trips_outside_window = window.before.trips + window.after.trips;
    manifest.people_outside_window = window.before.people + window.after.people;
    manifest.outputs = outputs.to_string();
    write_manifest(&manifest_path, &manifest).map_err(to_value_error)?;
    let link_bins_path = match &result.link_bins {
        Some(_) if minimal => None,
        Some(bins) => {
            let path = dir.join("link_bins.parquet");
            write_link_bins(&path, run_id, bins, &network.inner, &description)
                .map_err(to_value_error)?;
            Some(path.to_string_lossy().into_owned())
        }
        None => None,
    };
    let mut layer_paths: [Option<String>; 2] = [None, None];
    for (i, (layer, bins)) in
        [(StaticLayer::Bike, &result.bike_link_bins), (StaticLayer::Walk, &result.walk_link_bins)]
            .into_iter()
            .enumerate()
    {
        if let Some(bins) = bins.as_ref().filter(|_| !minimal) {
            let path = dir.join(format!("link_bins_{}.parquet", layer.as_str()));
            let graph = network.static_layer(layer)?;
            write_layer_link_bins(&path, run_id, layer, bins, graph.network(), &description)
                .map_err(to_value_error)?;
            layer_paths[i] = Some(path.to_string_lossy().into_owned());
        }
    }
    let [link_bins_bike_path, link_bins_walk_path] = layer_paths;
    // Every call of the timetable, with its times and passengers (S199, S200).
    let transit_calls_path = match (&result.transit, &transit_setup) {
        (Some(r), Some(t)) if !minimal => {
            let path = dir.join("transit_calls.parquet");
            openmobisim_io_parquet::write_transit_calls(
                &path,
                run_id,
                t.timetable(),
                r,
                &description,
                window.end_s,
                &stop_hubs,
            )
            .map_err(to_value_error)?;
            Some(path.to_string_lossy().into_owned())
        }
        _ => None,
    };
    // How full every parking was, bin by bin (M4, D12).
    let parking_bins_path = match (&result.parking, &parking_setup) {
        (Some(r), Some(p)) if !minimal => {
            let path = dir.join("parking_bins.parquet");
            openmobisim_io_parquet::write_parking_bins(
                &path,
                run_id,
                p,
                r,
                &description,
                &parking_hubs,
            )
            .map_err(to_value_error)?;
            Some(path.to_string_lossy().into_owned())
        }
        _ => None,
    };
    // A run's docked service (S302): its shared-bike legs and its docks bin by bin; nothing
    // without one.
    let service_out = match &service_setups {
        Some(setups) if result.services.is_some() => Some(crate::service_results::service_results(
            py,
            setups,
            &run_hubs,
            result.services.as_ref(),
            (&trips_used, &travellers, &trip_table),
            (f64::from(window.start_s), f64::from(window.end_s)),
            (dir, run_id, &description, minimal),
        )?),
        _ => None,
    };
    // A run's taxi fleet (S302): its trips, events and vehicles bin by bin; nothing without.
    let taxi_setup = service_setups.as_deref().and_then(|s| {
        s.iter().find(|s| s.kind() == openmobisim_core_sim::services::ServiceKind::Taxi)
    });
    let taxi_out = match (taxi_setup, &result.taxis) {
        (Some(setup), Some(taxis)) => Some(crate::service_results::taxi_results(
            py,
            setup,
            taxis,
            (&trips_used, &travellers),
            (f64::from(window.start_s), f64::from(window.end_s)),
            (dir, run_id, &description, minimal),
        )?),
        _ => None,
    };
    // The operator's decisions (S304) as a table with ids, written to `operator_tasks.parquet`
    // (S305); nothing without an operator.
    let operator_tasks = match (&result.service_decisions, &service_setups) {
        (Some((_, records)), Some(setups)) => {
            Some(openmobisim_core_sim::services::decisions::OperatorTaskTable::new(
                records,
                setups,
                &network.inner,
            ))
        }
        _ => None,
    };
    let operator_tasks_path = match &operator_tasks {
        Some(t) if !minimal => {
            let path = dir.join("operator_tasks.parquet");
            openmobisim_io_parquet::write_operator_tasks(&path, run_id, t, &description)
                .map_err(to_value_error)?;
            Some(path.to_string_lossy().into_owned())
        }
        _ => None,
    };
    // Their own stage, so the time the service tables and files take is seen (none without).
    if service_out.is_some() || taxi_out.is_some() {
        clock = timings.lap("services_written", None, clock);
    }
    let (
        taxi_events,
        taxi_trips,
        taxi_vehicles,
        taxi_events_path,
        taxi_trips_path,
        taxi_vehicles_path,
    ) = match taxi_out {
        Some(o) => (
            Some(o.events),
            Some(o.trips),
            Some(o.vehicles),
            o.events_path,
            o.trips_path,
            o.vehicles_path,
        ),
        None => (None, None, None, None, None, None),
    };
    let (
        bike_share_events,
        bike_share_trips,
        bike_share_docks,
        bike_share_dock_places,
        bike_share_events_path,
        bike_share_trips_path,
        bike_share_docks_path,
    ) = match service_out {
        Some(o) => (
            Some(o.events),
            Some(o.trips),
            Some(o.docks),
            Some(o.places),
            o.events_path,
            o.trips_path,
            o.docks_path,
        ),
        None => (None, None, None, None, None, None, None),
    };
    let (parking_bins_table, parking_places_table, parking_summary_table) =
        match (&result.parking, &parking_setup) {
            (Some(r), Some(p)) => (
                Some(parking_bins(py, p, r, &parking_hubs)?.unbind()),
                Some(parking_places(py, p, &parking_hubs)?.unbind()),
                Some(parking_summary(py, p, r)?.unbind()),
            ),
            _ => (None, None, None),
        };
    let itinerary_table = match &result.itineraries {
        Some(it) => Some(
            itinerary_choices(py, parking_setup.as_deref(), it, &trips_used, &travellers)?.unbind(),
        ),
        None => None,
    };
    let journeys_table = match &result.itineraries {
        Some(it) => Some(itinerary_journeys(py, it)?.unbind()),
        None => None,
    };
    let trip_modes_table = trip_modes(py, &trip_table)?.unbind();
    // What the services were expected to give and what their day gave (S303).
    let service_assumptions_out = description
        .service_assumptions
        .as_ref()
        .map(|e| -> PyResult<Py<PyDict>> {
            let d = PyDict::new(py);
            for (n, v) in e {
                d.set_item(n, v)?;
            }
            Ok(d.unbind())
        })
        .transpose()?;
    // The operator's decisions (S304).
    let (service_policy_out, service_decisions_out) =
        match (&result.service_decisions, &operator_tasks) {
            (Some((described, _)), Some(tasks)) => (
                crate::policy::description_to_py(py, Some(described))?.map(Bound::unbind),
                Some(crate::policy::decision_columns(py, tasks)?.unbind()),
            ),
            _ => (None, None),
        };
    let service_summaries = (result.services.is_some() || result.taxis.is_some())
        .then(|| -> PyResult<Py<PyDict>> {
            use openmobisim_core_sim::services::in_run::{DockSummary, TaxiSummary};
            let all = PyDict::new(py);
            let one = |kind: &str, rows: Vec<(&str, f64)>| -> PyResult<Bound<'_, PyDict>> {
                let d = PyDict::new(py);
                d.set_item("kind", kind)?;
                for (n, v) in rows {
                    d.set_item(n, v)?;
                }
                Ok(d)
            };
            if let Some(s) = &result.services {
                let summary = s.last.as_ref().map_or_else(DockSummary::idle, |d| d.summary);
                all.set_item(
                    &s.service,
                    one("bike_docked", summary.rows(s.assumed_extra_walk_s))?,
                )?;
            }
            if let Some(t) = &result.taxis {
                let summary = t.last.as_ref().map_or_else(TaxiSummary::idle, |d| d.summary);
                all.set_item(&t.service, one("taxi", summary.rows(t.assumed.wait_s))?)?;
            }
            Ok(all.unbind())
        })
        .transpose()?;
    let trip_results_table = trip_results(py, &trip_table)?.unbind();
    let gap_trips = result.gap_trips.as_ref().map(|g| gap_trip_columns(py, g)).transpose()?;
    let route_choices_py = match (&result.route_choices, &result.route_sets) {
        (Some(choices), Some(sets)) => Some(PyRouteChoices::new(py, choices, sets)?),
        _ => None,
    };
    let bike_route_choices_py = match (&result.bike_route_choices, &result.bike_route_sets) {
        (Some(choices), Some(sets)) => Some(PyRouteChoices::new(py, choices, sets)?),
        _ => None,
    };

    let (itinerary_replanned, return_mismatch_s) = result
        .itineraries
        .as_ref()
        .map_or((0, f64::NAN), |it| (it.replanned, it.return_mismatch_s));
    let completion_by_mode = PyDict::new(py);
    // A run with a service counts its cancelled taxi requests (S306).
    let service = result.services.is_some() || result.taxis.is_some();
    // The modes, and "none": the trips that chose and had no mode to choose (S269).
    let rows = Mode::ALL
        .iter()
        .map(|mode| (mode.as_str(), &result.by_mode[mode.index()]))
        .chain(std::iter::once(("none", &result.no_mode)));
    for (name, m) in rows {
        if m.completion.total_trips == 0 {
            continue;
        }
        let row = PyDict::new(py);
        row.set_item("total_trips", m.completion.total_trips)?;
        row.set_item("completed", m.completion.completed)?;
        row.set_item("truncated", m.completion.truncated)?;
        if service {
            row.set_item("cancelled", m.completion.cancelled)?;
        }
        row.set_item("no_vehicle_available", m.completion.no_vehicle_available)?;
        row.set_item("no_feasible_path", m.completion.no_feasible_path)?;
        row.set_item("mode_not_available", m.completion.mode_not_available)?;
        row.set_item("total_travel_time_s", m.total_travel_time.get())?;
        row.set_item("people", m.weighted_trips)?;
        row.set_item("completed_people", m.weighted_completed)?;
        row.set_item("truncated_people", m.weighted_truncated)?;
        if service {
            row.set_item("cancelled_people", m.weighted_cancelled)?;
        }
        row.set_item("not_run_people", m.weighted_not_run)?;
        completion_by_mode.set_item(name, row)?;
    }

    let (transit_calls_table, transit_summary_table) = match (&result.transit, &transit_setup) {
        (Some(r), Some(t)) => (
            Some(transit_calls(py, t.timetable(), r, window.end_s, &stop_hubs)?.unbind()),
            Some(
                transit_summary(
                    py,
                    r,
                    t.bus_report(),
                    result.itineraries.as_ref().map(|i| i.timetable_searches),
                )?
                .unbind(),
            ),
        ),
        _ => (None, None),
    };
    let gridlock = match &result.gridlock {
        Some(lock) => {
            let d = PyDict::new(py);
            d.set_item("vehicles_on_network", lock.on_network)?;
            d.set_item("vehicles_outside", lock.outside)?;
            d.set_item("links_waiting", lock.waiting_links)?;
            let loops: Vec<Vec<u32>> =
                lock.loops.iter().map(|c| c.iter().map(|l| l.raw()).collect()).collect();
            d.set_item("loop_count", loops.len())?;
            d.set_item("loop_links", loops.iter().map(Vec::len).sum::<usize>())?;
            d.set_item("loops", loops)?;
            d.set_item("room_waits_with_room", lock.room_waits_with_room)?;
            Some(d.unbind())
        }
        None => None,
    };
    let route_changes = {
        let (mut trip, mut who, mut seq, mut second, mut link, mut planned, mut taken) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let mut reason: Vec<&str> = Vec::new();
        for r in &result.reroutes {
            let t = TripId::new(r.vehicle.raw());
            let traveller = trips_used.traveller(t);
            trip.push(t.raw());
            who.push(travellers.external_ids().external(traveller.raw()).to_string());
            seq.push(t.raw() - travellers.first_trip(traveller).raw());
            second.push(r.second);
            link.push(r.link.raw());
            planned.push(r.planned_next.raw());
            taken.push(r.new_next.raw());
            reason.push(r.reason.as_str());
        }
        let d = PyDict::new(py);
        d.set_item("trip", trip.into_pyarray(py))?;
        d.set_item("traveller_id", who)?;
        d.set_item("trip_seq", seq.into_pyarray(py))?;
        d.set_item("second", second.into_pyarray(py))?;
        d.set_item("link", link.into_pyarray(py))?;
        d.set_item("next_planned", planned.into_pyarray(py))?;
        d.set_item("next_taken", taken.into_pyarray(py))?;
        d.set_item("reason", reason)?;
        d.unbind()
    };
    let route_realised = {
        let d = PyDict::new(py);
        let trips: Vec<u32> = result.routes_realised.iter().map(|(t, _)| t.raw()).collect();
        let links: Vec<Bound<'_, PyArray1<u32>>> = result
            .routes_realised
            .iter()
            .map(|(_, l)| l.iter().map(|x| x.raw()).collect::<Vec<u32>>().into_pyarray(py))
            .collect();
        d.set_item("trip", trips.into_pyarray(py))?;
        d.set_item("links", links)?;
        d.unbind()
    };
    // Wall-clock time by stage (S223): the network's reading if it was recorded, this
    // function's own stages around the core's, and the whole of it.
    timings.lap("results_written", None, clock);
    let mut timing_rows: Vec<(String, Option<u32>, f64)> = Vec::new();
    if let Some(read) = network.read_s {
        timing_rows.push(("network_read".to_string(), None, read));
    }
    let (before, after) = timings.stages().split_at(2);
    for stage in before.iter().chain(&core_stages).chain(after) {
        timing_rows.push((stage.name.to_string(), stage.iteration, stage.seconds));
    }
    timing_rows.push(("total".to_string(), None, started.elapsed().as_secs_f64()));
    let timings_path = dir.join("timings.csv");
    if !minimal {
        write_timings(&timings_path, &timing_rows).map_err(to_value_error)?;
    }

    // Where a queue outgrew its link (S229): peak occupancy against storage, worst first.
    let spillback = match &result.link_peak_pcu {
        Some(peaks) => {
            let mut over: Vec<(u32, f64, f64)> = peaks
                .iter()
                .enumerate()
                .filter_map(|(i, &peak)| {
                    let link = LinkId::from_index(i);
                    let storage = network.inner.storage(link).get();
                    (peak > storage).then(|| (link.raw(), peak, storage))
                })
                .collect();
            // Worst first by the PCU that did not fit: a few cars over a 5-m link matter less than
            // a queue a hundred cars past its road.
            over.sort_by(|a, b| (b.1 - b.2).total_cmp(&(a.1 - a.2)).then(a.0.cmp(&b.0)));
            let d = PyDict::new(py);
            d.set_item("link", over.iter().map(|o| o.0).collect::<Vec<_>>().into_pyarray(py))?;
            d.set_item("peak_pcu", over.iter().map(|o| o.1).collect::<Vec<_>>().into_pyarray(py))?;
            d.set_item(
                "storage_pcu",
                over.iter().map(|o| o.2).collect::<Vec<_>>().into_pyarray(py),
            )?;
            Some(d.unbind())
        }
        None => None,
    };

    let summary = PyRunSummary {
        skimmer,
        bike_cost,
        spillback,
        timings: timing_rows,
        timings_path: timings_path.to_string_lossy().into_owned(),
        gridlock,
        route_changes,
        gap_trips,
        gap_currency: result.iterations.last().map_or("time", |r| r.gap_currency).to_string(),
        route_realised,
        parking_bins: parking_bins_table,
        parking_places: parking_places_table,
        parking_summary: parking_summary_table,
        parking_bins_path,
        bike_share_events,
        bike_share_trips,
        bike_share_docks,
        bike_share_dock_places,
        bike_share_events_path,
        bike_share_trips_path,
        bike_share_docks_path,
        taxi_events,
        taxi_trips,
        taxi_vehicles,
        taxi_events_path,
        taxi_trips_path,
        taxi_vehicles_path,
        service_assumptions: service_assumptions_out,
        service_policy: service_policy_out,
        service_decisions: service_decisions_out,
        operator_tasks_path,
        service_summaries,
        itinerary_choices: itinerary_table,
        itinerary_journeys: journeys_table,
        trip_modes: trip_modes_table,
        trip_results: trip_results_table,
        itinerary_replanned,
        return_mismatch_s,
        transit_calls_path,
        window: window.to_dict(py, window_cut)?.unbind(),
        fallback_car: crate::parking::fallback_car(
            py,
            &result.fallback_car,
            &trips_used,
            &travellers,
        )?
        .unbind(),
        no_alternative: crate::parking::no_alternative(
            py,
            &result.no_alternative,
            &trips_used,
            &travellers,
            &result.modes_listed(),
        )?
        .unbind(),
        transit_calls: transit_calls_table,
        transit_summary: transit_summary_table,
        kpis_path: kpis_path.to_string_lossy().into_owned(),
        diagnostics_path: diagnostics_path.to_string_lossy().into_owned(),
        events_path: events_path.to_string_lossy().into_owned(),
        trip_results_path,
        manifest_path: manifest_path.to_string_lossy().into_owned(),
        link_bins_path,
        master_seed,
        choice_model: description.choice_model.clone(),
        equilibration: description.equilibration.clone(),
        route_update: description.route_update.clone(),
        convergence: convergence_arrays(py, &result.iterations, &result.modes_listed())?,
        converged: result.converged,
        fingerprint: description.fingerprint_hex(),
        total_trips: result.completion.total_trips,
        completed: result.completion.completed,
        truncated: result.completion.truncated,
        no_vehicle_available: result.completion.no_vehicle_available,
        no_feasible_path: result.completion.no_feasible_path,
        total_travel_time_s: result.total_travel_time.get(),
        total_vehicle_time_s: result.total_vehicle_time.get(),
        link_bins: result.link_bins.map(|inner| Py::new(py, PyLinkBins { inner })).transpose()?,
        bike_link_bins: result
            .bike_link_bins
            .map(|inner| Py::new(py, PyLinkBins { inner }))
            .transpose()?,
        walk_link_bins: result
            .walk_link_bins
            .map(|inner| Py::new(py, PyLinkBins { inner }))
            .transpose()?,
        link_bins_bike_path,
        link_bins_walk_path,
        mode_not_available: result.completion.mode_not_available,
        cancelled: service.then_some(result.completion.cancelled),
        completion_by_mode: completion_by_mode.unbind(),
        route_choices: route_choices_py,
        route_sets: result
            .route_sets
            .map(|sets| Py::new(py, PyRouteSets::new(Arc::new(sets), network.inner.link_count())))
            .transpose()?,
        bike_route_choices: bike_route_choices_py,
        bike_route_sets: result
            .bike_route_sets
            .map(|sets| Py::new(py, PyRouteSets::new(Arc::new(sets), bike_link_count)))
            .transpose()?,
        share,
    };
    Ok(Py::new(py, summary)?.into_any())
}

/// Write the run's time by stage as `stage,iteration,seconds`; an empty iteration is a
/// stage outside the iterations.
fn write_timings(path: &Path, rows: &[(String, Option<u32>, f64)]) -> std::io::Result<()> {
    use std::fmt::Write as _;
    let mut text = String::from("stage,iteration,seconds\n");
    for (stage, iteration, seconds) in rows {
        let iteration = iteration.map_or_else(String::new, |i| i.to_string());
        let _ = writeln!(text, "{stage},{iteration},{seconds:.6}");
    }
    std::fs::write(path, text)
}

/// The route sets kept between runs of this process, when a run asks for the cache.
fn route_cache_handle() -> &'static Arc<openmobisim_core_sim::RouteSetCache> {
    static CACHE: OnceLock<Arc<openmobisim_core_sim::RouteSetCache>> = OnceLock::new();
    CACHE.get_or_init(|| Arc::new(openmobisim_core_sim::RouteSetCache::default()))
}

/// Forget every route set the cache holds. The counters of `route_cache_info` stay.
#[pyfunction]
pub fn route_cache_clear() {
    route_cache_handle().clear();
}

/// What the route-set cache has done: ``(hits, misses, sets held)``.
#[pyfunction]
pub fn route_cache_info() -> (u64, u64, usize) {
    let s = route_cache_handle().stats();
    (s.hits, s.misses, s.held)
}

/// The route updates that can be selected by name, the default first.
#[pyfunction]
pub fn route_update_methods() -> Vec<String> {
    let mut names: Vec<String> = openmobisim_core_sim::route_update::Registry::builtin()
        .names()
        .into_iter()
        .map(|n| openmobisim_core_sim::route_update::public_name(n).to_string())
        .collect();
    names.sort_by_key(|n| (n != openmobisim_core_sim::route_update::DEFAULT_UPDATE, n.clone()));
    names
}

/// The equilibration strategies that can be selected by name, the default first.
#[pyfunction]
pub fn equilibration_strategies() -> Vec<String> {
    let mut names: Vec<String> = openmobisim_core_sim::equilibration::Registry::builtin()
        .names()
        .into_iter()
        .map(str::to_string)
        .collect();
    names.sort_by_key(|n| (n != openmobisim_core_sim::equilibration::DEFAULT_STRATEGY, n.clone()));
    names
}

/// Every parameter of every group at its shipped value: `(group, argument,
/// name, value)`, where `argument` is the `Scenario` (or reader) argument that overrides it by
/// name. The registries' groups (equilibration strategies, route methods, route updates, choice
/// models) come from each built-in's canonical descriptor with every option at its default.
#[pyfunction]
pub fn parameter_rows() -> PyResult<Vec<(String, String, String, f64)>> {
    let mut rows: Vec<(String, String, String, f64)> = Vec::new();
    let mut push = |group: &str, argument: &str, values: Vec<(String, f64)>| {
        for (name, value) in values {
            rows.push((group.to_string(), argument.to_string(), name, value));
        }
    };
    let owned =
        |v: Vec<(&'static str, f64)>| v.into_iter().map(|(n, x)| (n.to_string(), x)).collect();
    push(
        "network",
        "network_options",
        openmobisim_core_graph::network_options::NetworkDefaults::shipped().values(),
    );
    push("loading", "loading_options", LoadingOptions::SHIPPED.values());
    push("modes", "mode_options", owned(ModeDefaults::SHIPPED.values()));
    push("transit", "transit_options", owned(TransitDefaults::SHIPPED.values()));
    push("parking", "parking_options", owned(ParkingDefaults::SHIPPED.values()));
    push("prices", "price_options", owned(Prices::SHIPPED.values()));
    let none = BTreeMap::new();
    for name in equilibration_strategies() {
        let s =
            openmobisim_core_sim::equilibration::strategy(&name, &none).map_err(to_value_error)?;
        // `free_flow` writes its bare name at its defaults (S225): its two options by hand.
        let values = if name == "free_flow" {
            let d = openmobisim_core_sim::FreeFlow::default();
            vec![
                ("increments".to_string(), f64::from(d.increments)),
                ("warmup".to_string(), f64::from(d.warmup)),
            ]
        } else {
            descriptor_values(&s.descriptor())
        };
        push(&format!("equilibration.{name}"), "equilibration_options", values);
    }
    for name in crate::routes::route_methods() {
        let g = Registry::builtin().create(&name, &none).map_err(to_value_error)?;
        push(&format!("route_method.{name}"), "route_options", descriptor_values(&g.descriptor()));
    }
    for name in route_update_methods() {
        let u = openmobisim_core_sim::route_update::update(&name, &none).map_err(to_value_error)?;
        let mut values = descriptor_values(&u.descriptor());
        // The descriptor writes the search trees only when on (so older fingerprints stay), and
        // the table lists every option at its default (S282).
        if name == "least_cost" && !values.iter().any(|(n, _)| n == "tree_step_s") {
            values.push((
                "tree_step_s".to_string(),
                f64::from(openmobisim_core_sim::route_update::DEFAULT_TREE_STEP_S),
            ));
        }
        // The same for the iteration the sets stop growing after (S306): 0, every iteration.
        if name == "least_cost" && !values.iter().any(|(n, _)| n == "until_iteration") {
            values.push(("until_iteration".to_string(), 0.0));
        }
        push(&format!("route_update.{name}"), "route_update_options", values);
    }
    for name in crate::choice::choice_models() {
        let m = openmobisim_core_choice::model(&name, &none).map_err(to_value_error)?;
        push(&format!("choice_model.{name}"), "choice_options", descriptor_values(&m.descriptor()));
    }
    // The bike route sets of the doubly stochastic method (S298), `bike_route_options`.
    let bike = openmobisim_core_routes::Sampled::from_options(&none).map_err(to_value_error)?;
    push(
        "bike_route.doubly_stochastic",
        "bike_route_options",
        descriptor_values(&openmobisim_core_routes::RouteSetGenerator::descriptor(&bike)),
    );
    // The services (S302, S303): their options, what travellers assume, the built-in policies'
    // parameters (S304).
    use openmobisim_core_sim::services::{DockDefaults, ServiceAssumptions, TaxiDefaults};
    push("service.bike_docked", "options", owned(DockDefaults::SHIPPED.values()));
    push("service.taxi", "options", owned(TaxiDefaults::SHIPPED.values()));
    push("service_assumptions", "service_assumptions", owned(ServiceAssumptions::SHIPPED.values()));
    push("service_prices", "price_options", owned(Prices::SHIPPED.service_values(true, true)));
    for name in ["reset", "threshold", "charge_rule"] {
        let p = openmobisim_core_sim::services::policy::policy_builtin(name, &none)
            .map_err(PyValueError::new_err)?;
        push(&format!("policy.{name}"), "policy_options", p.describe().parameters);
    }
    Ok(rows)
}

/// The numeric `key=value` pairs of a canonical descriptor (`name;key=value;…`). The Monte Carlo
/// method writes its demand bias as `bias=demand|none`; its option is `biased` (1 or 0).
fn descriptor_values(descriptor: &str) -> Vec<(String, f64)> {
    descriptor
        .split(';')
        .skip(1)
        .filter_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            match (key, value) {
                ("bias", "demand") => Some(("biased".to_string(), 1.0)),
                ("bias", "none") => Some(("biased".to_string(), 0.0)),
                _ => Some((key.to_string(), value.parse().ok()?)),
            }
        })
        .collect()
}
