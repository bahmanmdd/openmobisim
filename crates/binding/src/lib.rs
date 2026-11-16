//! The Python binding: the module `openmobisim`, with `Network`, `Transit`, `Demand`, `run`,
//! `run_file` and `Run`, and nothing else.
//!
//! ```python
//! import openmobisim as oms
//! network = oms.Network.from_osm(path, **network_parameters)
//! network = oms.Network.from_tables(nodes, links, **network_parameters)
//! transit = oms.Transit.from_gtfs(path, network, **transit_parameters)
//! demand  = oms.Demand.from_table(trips, classes, network, transit=None, **demand_parameters)
//! result  = oms.run(network, demand, transit=None, window=(start_s, end_s), out=folder, **parameters)
//! result  = oms.run_file(record, out=folder)
//! ```
//!
//! Every keyword is a row of `PARAMETERS.md`, read at the one entry point whose crate reads it
//! (`keywords.rs`); the tables, in memory or Parquet files, are read into the crates' rows
//! (`tables.rs`); a run's record runs again through the same entry points (`record.rs`). Each
//! input read from a file keeps the file's canonical path, which the record writes. The interpreter's
//! lock is released while a file is read, a network, timetable or demand is built, and a run
//! runs and is written. A crate's error is a `ValueError` carrying its message unchanged (an
//! input or output failure an `OSError`); a panic is pyo3's `PanicException`, never an abort.

mod keywords;
mod record;
mod tables;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use openmobisim_demand::demand::Demand as RustDemand;
use openmobisim_graph::network::{BuildValues, Network as RustNetwork};
use openmobisim_io_gtfs::{GtfsError, Report as GtfsReport};
use openmobisim_io_osm::{OsmError, Report as OsmReport};
use openmobisim_io_parquet::{Files, Given as RunGiven, Source, Transit as RunTransit, WriteError};
use openmobisim_loop::parameters::{ChoiceModel, Parameters};
use openmobisim_transit::timetable::Timetable;
use openmobisim_types::time::Second;
use openmobisim_types::units::Metres;
use pyo3::exceptions::{PyOSError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyDict;

use keywords::{Entry, Given};

/// The defaults of `PARAMETERS.md` that no crate holds, since the graph, the GTFS reader and the
/// demand crate take these values from their caller: veh/h per lane, km/h, km/h, m, m.
const CAPACITY_PER_LANE: f64 = 1_800.0;
const BIKE_SPEED: f64 = 15.0;
const WALK_SPEED: f64 = 4.8;
const TRANSFER_WALK_MAX: f64 = 400.0;
const STOP_WALK_MAX: f64 = 1_500.0;

fn value_error(e: &impl ToString) -> PyErr {
    PyValueError::new_err(e.to_string())
}

/// The canonical path of a file just read (absolute, symbolic links resolved), for the record;
/// an `OSError` naming it if it cannot be resolved.
fn canonical(path: &Path) -> PyResult<PathBuf> {
    std::fs::canonicalize(path).map_err(|e| PyOSError::new_err(format!("{}: {e}", path.display())))
}

/// The graph's three build values: the keywords given, else the defaults above.
fn build_values(given: &Given<'_>) -> PyResult<BuildValues> {
    Ok(BuildValues {
        capacity_per_lane: given.get("capacity_per_lane", CAPACITY_PER_LANE)?,
        bike_speed: given.get("bike_speed", BIKE_SPEED)?,
        walk_speed: given.get("walk_speed", WALK_SPEED)?,
    })
}

/// A network: built once, from an OpenStreetMap extract or from two tables, then immutable.
#[pyclass(frozen, module = "openmobisim")]
struct Network {
    network: Arc<RustNetwork>,
    /// The build values it was built with, kept for the run's record.
    values: BuildValues,
    /// The OSM reader's counts; none for a network from tables.
    report: Option<OsmReport>,
    /// The files it was read from, canonical: the extract, or the nodes and the links tables
    /// each when given as a file.
    osm_file: Option<PathBuf>,
    nodes_file: Option<PathBuf>,
    links_file: Option<PathBuf>,
}

#[pymethods]
impl Network {
    /// Read one `.osm.pbf` extract. Keywords: `capacity_per_lane` (veh/h per lane),
    /// `bike_speed`, `walk_speed` (km/h), `osm_class_defaults` (only `None`: the reader's one
    /// table, `crates/io-osm/README.md`).
    #[staticmethod]
    #[pyo3(signature = (path, **network_parameters))]
    #[allow(clippy::needless_pass_by_value, reason = "pyo3 hands the arguments over by value")]
    fn from_osm(
        py: Python<'_>,
        path: PathBuf,
        network_parameters: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Self> {
        let given = Given::read(Entry::Network, "Network.from_osm", network_parameters)?;
        let values = build_values(&given)?;
        if given.raw("osm_class_defaults").is_some_and(|v| !v.is_none()) {
            return Err(PyValueError::new_err(
                "osm_class_defaults: the OSM reader reads one class table, the one in \
                 crates/io-osm/README.md; it takes no other, so the only value is None",
            ));
        }
        let read = py.detach(|| openmobisim_io_osm::read(&path, values));
        let (network, report) = read.map_err(|e| match e {
            OsmError::Io { .. } => PyOSError::new_err(e.to_string()),
            _ => value_error(&e),
        })?;
        Ok(Self {
            network: Arc::new(network),
            values,
            report: Some(report),
            osm_file: Some(canonical(&path)?),
            nodes_file: None,
            links_file: None,
        })
    }

    /// Build from a nodes table (`node_id`, then `lon_deg`, `lat_deg` or `x_m`, `y_m`) and a
    /// links table (`link_id`, `upstream`, `downstream`, `modes`, `length_m`, `lanes`,
    /// `car_speed_km_h`, `capacity_veh_h`), each in memory or the path of a Parquet file.
    /// Keywords: `capacity_per_lane` (veh/h per lane), `bike_speed`, `walk_speed` (km/h).
    #[staticmethod]
    #[pyo3(signature = (nodes, links, **network_parameters))]
    fn from_tables(
        py: Python<'_>,
        nodes: &Bound<'_, PyAny>,
        links: &Bound<'_, PyAny>,
        network_parameters: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Self> {
        let given = Given::read(Entry::Network, "Network.from_tables", network_parameters)?;
        if given.raw("osm_class_defaults").is_some() {
            return Err(PyTypeError::new_err(
                "Network.from_tables: osm_class_defaults is read by Network.from_osm, not here: \
                 a network from tables has no OSM classes",
            ));
        }
        let values = build_values(&given)?;
        let builder = tables::network(nodes, links)?;
        let network = py.detach(|| builder.build(values)).map_err(|e| value_error(&e))?;
        Ok(Self {
            network: Arc::new(network),
            values,
            report: None,
            osm_file: None,
            nodes_file: tables::path(nodes).map(|p| canonical(&p)).transpose()?,
            links_file: tables::path(links).map(|p| canonical(&p)).transpose()?,
        })
    }
}

/// The timetable of one service day, read from a GTFS feed on a network.
#[pyclass(frozen, module = "openmobisim")]
struct Transit {
    timetable: Arc<Timetable>,
    /// The network its footpaths were derived on.
    network: Arc<RustNetwork>,
    report: GtfsReport,
    /// The `transfer_walk_max` it was read with, metres, kept for the run's record.
    transfer_walk_max: f64,
    /// The feed it was read from, canonical.
    gtfs_file: PathBuf,
}

#[pymethods]
impl Transit {
    /// Read one GTFS feed (`.zip`) on `network`, built from longitude/latitude. Keyword:
    /// `transfer_walk_max` (m).
    #[staticmethod]
    #[pyo3(signature = (path, network, **transit_parameters))]
    #[allow(clippy::needless_pass_by_value, reason = "pyo3 hands the arguments over by value")]
    fn from_gtfs(
        py: Python<'_>,
        path: PathBuf,
        network: &Bound<'_, Network>,
        transit_parameters: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Self> {
        let given = Given::read(Entry::Transit, "Transit.from_gtfs", transit_parameters)?;
        let transfer_walk_max = given.get("transfer_walk_max", TRANSFER_WALK_MAX)?;
        let network = Arc::clone(&network.get().network);
        let read =
            py.detach(|| openmobisim_io_gtfs::read(&path, &network, Metres(transfer_walk_max)));
        let (timetable, report) = read.map_err(|e| match e {
            GtfsError::Io(_) => PyOSError::new_err(e.to_string()),
            _ => value_error(&e),
        })?;
        let gtfs_file = canonical(&path)?;
        Ok(Self { timetable: Arc::new(timetable), network, report, transfer_walk_max, gtfs_file })
    }
}

/// The trips of a run, linked to the network per mode, with their stops when a timetable is
/// given.
#[pyclass(frozen, module = "openmobisim")]
struct Demand {
    demand: RustDemand,
    network: Arc<RustNetwork>,
    transit: Option<Arc<Timetable>>,
    /// The `stop_walk_max` it was built with, metres, kept for the run's record.
    stop_walk_max: f64,
    /// The trips table's file, canonical, when it was given as one.
    trips_file: Option<PathBuf>,
}

#[pymethods]
impl Demand {
    /// Build from a trips table (`trip_id`, `origin_lon_deg`, `origin_lat_deg` or
    /// `origin_x_m`, `origin_y_m`, the same for `destination_`, `departure_s`, `class_id`,
    /// `modes`), in memory or the path of a Parquet file, and the list of class ids, on
    /// `network` and, if given, `transit` (read on the same network). Keyword: `stop_walk_max`
    /// (m).
    #[staticmethod]
    #[pyo3(signature = (trips, classes, network, transit=None, **demand_parameters))]
    #[allow(clippy::needless_pass_by_value, reason = "pyo3 hands the arguments over by value")]
    fn from_table(
        py: Python<'_>,
        trips: &Bound<'_, PyAny>,
        classes: Vec<String>,
        network: &Bound<'_, Network>,
        transit: Option<&Bound<'_, Transit>>,
        demand_parameters: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Self> {
        let given = Given::read(Entry::Demand, "Demand.from_table", demand_parameters)?;
        let stop_walk_max = given.get("stop_walk_max", STOP_WALK_MAX)?;
        let network = Arc::clone(&network.get().network);
        let transit = transit.map(|t| t.get());
        if transit.is_some_and(|t| !Arc::ptr_eq(&t.network, &network)) {
            return Err(PyValueError::new_err("the transit was read on another network"));
        }
        let transit = transit.map(|t| Arc::clone(&t.timetable));
        let rows = tables::trips(trips)?;
        let max = Metres(stop_walk_max);
        let build = || RustDemand::build(&rows, &classes, &network, transit.as_deref(), max);
        let demand = py.detach(build).map_err(|e| value_error(&e))?;
        let trips_file = tables::path(trips).map(|p| canonical(&p)).transpose()?;
        Ok(Self { demand, network, transit, stop_walk_max, trips_file })
    }
}

/// What a run returns: the paths of its three Parquet files and of its record, `run.toml`,
/// whether the stopping rule (else the iteration cap) ended it, and the readers' counts (`None`
/// for a network from tables or a run without transit).
#[pyclass(frozen, get_all, module = "openmobisim")]
struct Run {
    /// `trips.parquet`: one row per trip.
    trips: PathBuf,
    /// `links.parquet`: one row per link and bin of entry that someone entered.
    links: PathBuf,
    /// `iterations.parquet`: one row per iteration.
    iterations: PathBuf,
    /// `run.toml`: the record of what was run.
    record: PathBuf,
    /// Whether the stopping rule ended the run (else the iteration cap).
    stopped_by_rule: bool,
    /// The OSM reader's counts, as a dict; `None` for a network from tables.
    osm_report: Option<Py<PyDict>>,
    /// The GTFS reader's counts, as a dict; `None` without transit.
    gtfs_report: Option<Py<PyDict>>,
}

/// The loop's parameters: its defaults for the demand's classes, and the keywords given.
fn parameters(given: &Given<'_>, demand: &RustDemand) -> PyResult<Parameters> {
    let d = Parameters::defaults(demand.class_count());
    let choice = match given.raw("choice") {
        None => d.choice,
        Some(_) => match given.get("choice", String::new())?.as_str() {
            "logit" => ChoiceModel::Logit,
            "deterministic" => ChoiceModel::Deterministic,
            other => {
                return Err(PyValueError::new_err(format!(
                    "choice: {other:?} is not a choice model; the models are \"logit\" and \
                     \"deterministic\""
                )));
            }
        },
    };
    let mut coefficients = d.coefficients.clone();
    if given.raw("coefficients").is_some() {
        let rows: BTreeMap<String, [f64; 16]> = given.get("coefficients", BTreeMap::new())?;
        if let Some(other) = rows.keys().find(|c| demand.class_ids().id_of(c).is_none()) {
            return Err(PyValueError::new_err(format!(
                "coefficients: {other} is not a class of the demand"
            )));
        }
        for (i, class) in demand.class_ids().iter().enumerate() {
            coefficients[i] = *rows.get(class).ok_or_else(|| {
                PyValueError::new_err(format!("coefficients: class {class} has no row"))
            })?;
        }
    }
    let seed = given.get("seed", d.seed)?;
    if i64::try_from(seed).is_err() {
        return Err(PyValueError::new_err(format!(
            "seed: {seed} is outside 0 to 9223372036854775807 (2^63 - 1), the integers the \
             run's record holds"
        )));
    }
    Ok(Parameters {
        link_bin: given.get("link_bin", d.link_bin)?,
        iteration_cap: given.get("iteration_cap", d.iteration_cap)?,
        iteration_floor: given.get("iteration_floor", d.iteration_floor)?,
        seed,
        boarding_slack: given.get("boarding_slack", d.boarding_slack)?,
        max_rides: given.get("max_rides", d.max_rides)?,
        nearby_stops: given.get("nearby_stops", d.nearby_stops)?,
        choice,
        nest_scale: given.get("nest_scale", d.nest_scale)?,
        coefficients,
    })
}

/// Run the loop on `network`, `demand` (built on that network and that transit) and `transit`
/// over `window`, two seconds of the clock, and write `trips.parquet`, `links.parquet`,
/// `iterations.parquet` and `run.toml` into `out`. Keywords: the loop's rows of `PARAMETERS.md`.
#[pyfunction]
#[pyo3(signature = (network, demand, transit=None, *, window, out, **parameters))]
#[allow(clippy::needless_pass_by_value, reason = "pyo3 hands the arguments over by value")]
fn run(
    py: Python<'_>,
    network: &Bound<'_, Network>,
    demand: &Bound<'_, Demand>,
    transit: Option<&Bound<'_, Transit>>,
    window: (u32, u32),
    out: PathBuf,
    parameters: Option<&Bound<'_, PyDict>>,
) -> PyResult<Run> {
    let given = Given::read(Entry::Run, "run", parameters)?;
    let (network, demand, transit) = (network.get(), demand.get(), transit.map(|t| t.get()));
    let p = self::parameters(&given, &demand.demand)?;
    if !Arc::ptr_eq(&network.network, &demand.network) {
        return Err(PyValueError::new_err("the demand was built on another network"));
    }
    let same = match (transit, &demand.transit) {
        (None, None) => true,
        (Some(t), Some(d)) => Arc::ptr_eq(&t.timetable, d),
        _ => false,
    };
    if !same {
        return Err(PyValueError::new_err("the demand was built on another transit"));
    }
    let timetable = transit.map(|t| &*t.timetable);
    let window = (Second(window.0), Second(window.1));
    let (net, dem) = (&*network.network, &demand.demand);
    let given = RunGiven {
        source: network.report.as_ref().map_or(Source::Tables, Source::Osm),
        files: Files {
            osm: network.osm_file.as_deref(),
            nodes: network.nodes_file.as_deref(),
            links: network.links_file.as_deref(),
            trips: demand.trips_file.as_deref(),
            gtfs: transit.map(|t| t.gtfs_file.as_path()),
        },
        build_values: network.values,
        stop_walk_max: demand.stop_walk_max,
        transit: transit.map(|t| RunTransit {
            timetable: &t.timetable,
            report: &t.report,
            transfer_walk_max: t.transfer_walk_max,
        }),
        parameters: &p,
    };
    // An `out` that would replace an input file is refused before the loop runs: the paths
    // alone tell it, and `write` would refuse it after the run.
    openmobisim_io_parquet::replaces_no_input(&given.files, &out).map_err(|e| value_error(&e))?;
    let written = py.detach(|| {
        let r = openmobisim_loop::run::run(net, dem, timetable, window, &p)
            .map_err(|e| value_error(&e))?;
        let write = openmobisim_io_parquet::write(&r, net, dem, &given, &out);
        let written = write.map_err(|e| match e {
            WriteError::Io(_) => PyOSError::new_err(e.to_string()),
            _ => value_error(&e),
        })?;
        Ok::<_, PyErr>((written, r.stopped_by_rule))
    });
    let (written, stopped_by_rule) = written?;
    Ok(Run {
        trips: written.trips.path,
        links: written.links.path,
        iterations: written.iterations.path,
        record: written.record,
        stopped_by_rule,
        osm_report: network.report.as_ref().map(|r| osm_report(py, r)).transpose()?,
        gtfs_report: transit.map(|t| gtfs_report(py, &t.report)).transpose()?,
    })
}

/// The OSM reader's counts as a dict.
fn osm_report(py: Python<'_>, r: &OsmReport) -> PyResult<Py<PyDict>> {
    let d = PyDict::new(py);
    d.set_item("nodes", r.nodes)?;
    d.set_item("links", r.links)?;
    d.set_item("links_unreachable", r.links_unreachable)?;
    d.set_item("loops_dropped", r.loops_dropped)?;
    d.set_item("ways_cut", r.ways_cut)?;
    Ok(d.unbind())
}

/// The GTFS reader's counts as a dict; `date` as GTFS writes it, `YYYYMMDD`.
fn gtfs_report(py: Python<'_>, r: &GtfsReport) -> PyResult<Py<PyDict>> {
    let d = PyDict::new(py);
    d.set_item("date", r.date)?;
    d.set_item("trips", r.trips)?;
    d.set_item("runs", r.runs)?;
    d.set_item("calls", r.calls)?;
    d.set_item("night_runs", r.night_runs)?;
    d.set_item("frequency_runs", r.frequency_runs)?;
    d.set_item("stops", r.stops)?;
    d.set_item("trips_untimed", r.trips_untimed)?;
    d.set_item("trips_single_call", r.trips_single_call)?;
    d.set_item("times_interpolated", r.times_interpolated)?;
    d.set_item("pickup_drop_off_ignored", r.pickup_drop_off_ignored)?;
    d.set_item("frequency_rows_skipped", r.frequency_rows_skipped)?;
    d.set_item("footpaths", r.footpaths)?;
    d.set_item("transfers_applied", r.transfers_applied)?;
    d.set_item("transfers_skipped", r.transfers_skipped)?;
    Ok(d.unbind())
}

/// Multimodal dynamic traffic assignment: cars on a point-queue network, bikes and walkers at
/// free flow, transit on its timetable, choice and loading iterated to a stable state.
#[pymodule]
fn openmobisim(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Network>()?;
    m.add_class::<Transit>()?;
    m.add_class::<Demand>()?;
    m.add_class::<Run>()?;
    m.add_function(wrap_pyfunction!(run, m)?)?;
    m.add_function(wrap_pyfunction!(record::run_file, m)?)?;
    Ok(())
}
