//! The Python binding: the module `openmobisim`, with `Network`, `Transit`, `Demand`, `run`,
//! `run_file`, `Run` and its methods `Run.map`, `Run.summary` and `Run.open`, and nothing else.
//!
//! ```python
//! import openmobisim as oms
//! network = oms.Network.from_osm(path, region=None, **network_parameters)
//! network = oms.Network.from_tables(nodes, links, **network_parameters)
//! transit = oms.Transit.from_gtfs(path, network, **transit_parameters)
//! demand  = oms.Demand.from_table(trips, classes, network, transit=None, **demand_parameters)
//! result  = oms.run(network, demand, transit=None, window=(start_s, end_s), out=folder, **parameters)
//! result  = oms.run_file(record, out=folder)
//! result  = oms.Run.open(record, network=None, demand=None, transit=None)
//! page    = result.map(path)
//! page    = result.summary(path)
//! ```
//!
//! Every keyword is a row of `PARAMETERS.md`, read at the one entry point whose crate reads it
//! (`keywords.rs`); the tables, in memory or Parquet files, are read into the crates' rows
//! (`tables.rs`); a run's record runs again through the same entry points (`record.rs`). Each
//! input read from a file keeps the file's canonical path, which the record writes. The interpreter's
//! lock is released while a file is read, a network, timetable or demand is built, and a run
//! runs and is written. A crate's error is a `ValueError` carrying its message unchanged (an
//! input or output failure an `OSError`); a panic is pyo3's `PanicException`, never an abort.
//!
//! What a reader or the run set right instead of refusing (`openmobisim_types::repairs`) is
//! shown (`reports.rs`) as one `UserWarning` per kind and call, naming the entry point, the count, what was
//! done, the key and the first five ids; and kept as a dict, key to count, on the object
//! (`Network.repairs`, `Transit.repairs`, `Demand.repairs`) and on the run (`Run.repairs`).

mod keywords;
mod record;
mod reports;
mod tables;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use openmobisim_demand::demand::Demand as RustDemand;
use openmobisim_graph::network::{BuildValues, Network as RustNetwork};
use openmobisim_io_gtfs::{GtfsError, Lines, Report as GtfsReport};
use openmobisim_io_osm::{OsmError, Report as OsmReport};
use openmobisim_io_parquet::{
    Files, Given as RunGiven, OutputError, Source, Transit as RunTransit, WriteError,
};
use openmobisim_loop::assess::Iteration;
use openmobisim_loop::parameters::{ChoiceModel, Parameters};
use openmobisim_map::{Kept, MapError, Mapped};
use openmobisim_summary::{Summarised, SummaryError};
use openmobisim_transit::timetable::Timetable;
use openmobisim_types::repairs::Repairs;
use openmobisim_types::time::{Second, StepGrid};
use openmobisim_types::units::Metres;
use pyo3::exceptions::{PyOSError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyDict;

use keywords::{Entry, Given};
use reports::{counts, gtfs_report, osm_report, repairs_dict, warn, warn_no_trip, warn_repairs};

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

/// Refuse a demand not built on `network` and `transit`, naming which.
fn built_on(network: &Network, demand: &Demand, transit: Option<&Transit>) -> PyResult<()> {
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
    Ok(())
}

/// A run's network, demand and transit, as the caller holds them.
type RunInputs<'a, 'py> =
    (&'a Bound<'py, Network>, &'a Bound<'py, Demand>, Option<&'a Bound<'py, Transit>>);

/// The four tables a run writes, beside its record.
const TABLES: [&str; 4] =
    ["trips.parquet", "links.parquet", "iterations.parquet", "routes.parquet"];

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

/// One `UserWarning` when `network` holds no link: no trip can travel on it.
fn no_link(py: Python<'_>, method: &str, network: &RustNetwork) -> PyResult<()> {
    if network.link_count() > 0 {
        return Ok(());
    }
    let message = format!(
        "{method}: no link is left: the network holds {} nodes and no link (each left out is \
         counted above), so no trip can travel on it",
        network.node_count()
    );
    warn(py, &[message])
}

/// A network: built once, from an OpenStreetMap extract or from two tables, then immutable.
#[pyclass(frozen, module = "openmobisim")]
struct Network {
    network: Arc<RustNetwork>,
    /// The build values it was built with, kept for the run's record.
    values: BuildValues,
    /// The OSM reader's counts; none for a network from tables.
    report: Option<OsmReport>,
    /// What its readers set right: the OSM reader's (the build's among them), or the tables'
    /// reader's and the build's.
    repairs: Repairs,
    /// The files it was read from, canonical: the extract, or the nodes and the links tables
    /// each when given as a file.
    osm_file: Option<PathBuf>,
    nodes_file: Option<PathBuf>,
    links_file: Option<PathBuf>,
}

#[pymethods]
impl Network {
    /// Read one `.osm.pbf` extract, cut to `region` if given: a `(west, south, east, north)`
    /// box or a sequence of `(lon, lat)` points of a polygon, in WGS84 degrees, lists and tuples
    /// alike. Keywords:
    /// `capacity_per_lane` (veh/h per lane), `bike_speed`, `walk_speed` (km/h),
    /// `osm_class_defaults` (only `None`: the reader's one table, `crates/io-osm/README.md`).
    #[staticmethod]
    #[pyo3(signature = (path, region=None, **network_parameters))]
    #[allow(clippy::needless_pass_by_value, reason = "pyo3 hands the arguments over by value")]
    fn from_osm(
        py: Python<'_>,
        path: PathBuf,
        region: Option<&Bound<'_, PyAny>>,
        network_parameters: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Self> {
        let given = Given::read(Entry::Network, "Network.from_osm", network_parameters)?;
        let values = build_values(&given)?;
        let region = keywords::region(region)?;
        if given.raw("osm_class_defaults").is_some_and(|v| !v.is_none()) {
            return Err(PyValueError::new_err(
                "osm_class_defaults: the OSM reader reads one class table, the one in \
                 crates/io-osm/README.md; it takes no other, so the only value is None",
            ));
        }
        let read = py.detach(|| openmobisim_io_osm::read(&path, values, region.as_ref()));
        let (network, report) = read.map_err(|e| match e {
            OsmError::Io { .. } => PyOSError::new_err(e.to_string()),
            _ => value_error(&e),
        })?;
        warn_repairs(py, "Network.from_osm", &report.repairs)?;
        no_link(py, "Network.from_osm", &network)?;
        Ok(Self {
            network: Arc::new(network),
            values,
            repairs: report.repairs.clone(),
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
        let (builder, mut repairs) = tables::network(nodes, links)?;
        let network = py.detach(|| builder.build(values)).map_err(|e| value_error(&e))?;
        repairs.extend(network.repairs());
        warn_repairs(py, "Network.from_tables", &repairs)?;
        no_link(py, "Network.from_tables", &network)?;
        Ok(Self {
            network: Arc::new(network),
            values,
            repairs,
            report: None,
            osm_file: None,
            nodes_file: tables::path(nodes).map(|p| canonical(&p)).transpose()?,
            links_file: tables::path(links).map(|p| canonical(&p)).transpose()?,
        })
    }

    /// What its readers set right, as a dict: each kind's key to its count (the keys in
    /// `crates/io-osm/README.md`, `crates/graph/README.md` and this crate's README).
    #[getter]
    fn repairs(&self, py: Python<'_>) -> PyResult<Py<PyDict>> {
        repairs_dict(py, counts(&self.repairs))
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
    /// Beside the timetable: the stops' names, the runs' lines and kinds, for the run's pages.
    lines: Arc<Lines>,
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
        let read = py.detach(|| {
            let read = openmobisim_io_gtfs::read(&path, &network, Metres(transfer_walk_max))?;
            Ok::<_, GtfsError>((read, openmobisim_io_gtfs::lines(&path)?))
        });
        let ((timetable, report), lines) = read.map_err(|e| match e {
            GtfsError::Io(_) => PyOSError::new_err(e.to_string()),
            _ => value_error(&e),
        })?;
        warn_repairs(py, "Transit.from_gtfs", &report.repairs)?;
        if report.runs == 0 {
            let message = "Transit.from_gtfs: no run is left: the timetable holds no run of its \
                           day, so transit carries nobody";
            warn(py, &[message.to_owned()])?;
        }
        let gtfs_file = canonical(&path)?;
        let (timetable, lines) = (Arc::new(timetable), Arc::new(lines));
        Ok(Self { timetable, network, report, transfer_walk_max, gtfs_file, lines })
    }

    /// What the GTFS reader set right, as a dict: each kind's key to its count
    /// (`crates/io-gtfs/README.md`).
    #[getter]
    fn repairs(&self, py: Python<'_>) -> PyResult<Py<PyDict>> {
        repairs_dict(py, counts(&self.report.repairs))
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
    /// What its readers set right or dropped: the trips table's reader's and the build's.
    repairs: Repairs,
    /// How many rows the trips table held, the dropped ones with them.
    #[pyo3(get)]
    trips_given: u64,
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
        let (rows, mut repairs, trips_given) = tables::trips(trips)?;
        let max = Metres(stop_walk_max);
        let build = || RustDemand::build(&rows, &classes, &network, transit.as_deref(), max);
        let demand = py.detach(build).map_err(|e| value_error(&e))?;
        repairs.extend(demand.repairs());
        warn_repairs(py, "Demand.from_table", &repairs)?;
        if demand.trip_count() == 0 {
            warn_no_trip(py, "Demand.from_table", trips_given, &[&repairs])?;
        }
        let carless = repairs.count("trips_without_car_network");
        if carless > 0 {
            warn(
                py,
                &[format!(
                    "Demand.from_table: the network has no car link: {carless} trips that ask for a car \
                 cannot drive (trips_without_car_network)"
                )],
            )?;
        }
        let trips_file = tables::path(trips).map(|p| canonical(&p)).transpose()?;
        Ok(Self { demand, network, transit, stop_walk_max, trips_file, repairs, trips_given })
    }

    /// What its readers set right or dropped, as a dict: each kind's key to its count
    /// (`crates/demand/README.md` and this crate's README).
    #[getter]
    fn repairs(&self, py: Python<'_>) -> PyResult<Py<PyDict>> {
        repairs_dict(py, counts(&self.repairs))
    }
}

/// How a run ended, from the run or its record: whether the stopping rule (else the cap) ended
/// it, `None` when the record does not say; and what the run set right or dropped, by kind,
/// `(key, count)` in the order first met.
#[derive(Clone, Debug)]
pub(crate) struct Ended {
    pub(crate) stopped_by_rule: Option<bool>,
    pub(crate) repairs: Vec<(String, u64)>,
}

impl Ended {
    /// How many trips the run dropped: `trips_outside_window` and `trips_after_window`.
    fn dropped(&self) -> u64 {
        let dropping = ["trips_outside_window", "trips_after_window"];
        self.repairs.iter().filter(|(k, _)| dropping.contains(&k.as_str())).map(|r| r.1).sum()
    }
}

/// A run's window: the two seconds of the clock it ran between, and the bin of its links table.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Window {
    /// The window's first second, a second of the clock.
    pub(crate) start: u32,
    /// The second it ends at, a second of the clock.
    pub(crate) end: u32,
    /// The bin of the links table, s (`link_bin`).
    pub(crate) bin: u32,
}

/// The two pages of a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Page {
    /// `Run.map`, the interactive map.
    Map,
    /// `Run.summary`, the page of its numbers.
    Summary,
}

impl Page {
    /// The method's name, for a message.
    fn method(self) -> &'static str {
        match self {
            Self::Map => "Run.map",
            Self::Summary => "Run.summary",
        }
    }
}

/// What a run returns: the paths of its four Parquet files and of its record, `run.toml`,
/// whether the stopping rule (else the iteration cap) ended it (`None` when an opened record
/// does not say), the readers' counts (`None` for a network from tables or a run without
/// transit) and every repair; and, hidden, the files of the run, its window and its inputs, by
/// reference. Nothing of the run itself is kept: its map and its
/// summary read its files.
#[pyclass(frozen, module = "openmobisim")]
pub(crate) struct Run {
    /// `trips.parquet`: one row per trip.
    #[pyo3(get)]
    trips: PathBuf,
    /// `links.parquet`: one row per link and bin of entry that someone entered.
    #[pyo3(get)]
    links: PathBuf,
    /// `iterations.parquet`: one row per iteration.
    #[pyo3(get)]
    iterations: PathBuf,
    /// `routes.parquet`: one row per trip and leg of what each trip took.
    #[pyo3(get)]
    routes: PathBuf,
    /// `run.toml`: the record of what was run.
    #[pyo3(get)]
    record: PathBuf,
    /// How it ended, and what it set right or dropped.
    ended: Ended,
    /// The run's input and output files (canonical), which a page never replaces.
    files: Vec<PathBuf>,
    /// Its window and bin.
    window: Window,
    /// The network it ran on.
    network: Py<Network>,
    /// The demand it ran.
    demand: Py<Demand>,
    /// Its transit, if any.
    transit: Option<Py<Transit>>,
}

#[pymethods]
impl Run {
    /// The OSM reader's counts (its network's), as a dict; `None` for a network from tables.
    #[getter]
    fn osm_report(&self, py: Python<'_>) -> PyResult<Option<Py<PyDict>>> {
        self.network.get().report.as_ref().map(|r| osm_report(py, r)).transpose()
    }

    /// The GTFS reader's counts (its transit's), as a dict; `None` without transit.
    #[getter]
    fn gtfs_report(&self, py: Python<'_>) -> PyResult<Option<Py<PyDict>>> {
        self.transit.as_ref().map(|t| gtfs_report(py, &t.get().report)).transpose()
    }

    /// Whether the stopping rule ended the run (else the iteration cap); `None` when the record
    /// of an opened run does not say.
    #[getter]
    fn stopped_by_rule(&self) -> Option<bool> {
        self.ended.stopped_by_rule
    }

    /// Every repair of the run and its inputs, as a dict of dicts, each kind's key to its count:
    /// `network`, `demand`, `transit` (absent without transit) and `run` (the window's). The
    /// trips given are `Demand.trips_given`.
    #[getter]
    fn repairs(&self, py: Python<'_>) -> PyResult<Py<PyDict>> {
        let d = PyDict::new(py);
        d.set_item("network", self.network.get().repairs(py)?)?;
        d.set_item("demand", self.demand.get().repairs(py)?)?;
        if let Some(t) = &self.transit {
            d.set_item("transit", t.get().repairs(py)?)?;
        }
        let run = self.ended.repairs.iter().map(|(k, n)| (k.as_str(), *n));
        d.set_item("run", repairs_dict(py, run)?)?;
        Ok(d.unbind())
    }

    /// Write the run's interactive map, one self-contained HTML file, to `path` (replaced if
    /// there), titled `title`, else with the file's stem; return `path`. A `path` that is one of
    /// the run's files is refused ([`Run::page_path`]). The run is read from its files.
    #[pyo3(signature = (path, title=None))]
    fn map(&self, py: Python<'_>, path: PathBuf, title: Option<String>) -> PyResult<PathBuf> {
        self.page(py, Page::Map, path, title)
    }

    /// Write the run's summary, one self-contained HTML page of its numbers, to `path` (replaced
    /// if there), titled `title`, else with the file's stem; return `path`. A `path` that is one
    /// of the run's files is refused ([`Run::page_path`]). The run is read from its files.
    #[pyo3(signature = (path, title=None))]
    fn summary(&self, py: Python<'_>, path: PathBuf, title: Option<String>) -> PyResult<PathBuf> {
        self.page(py, Page::Summary, path, title)
    }

    /// Open the finished run whose record is `record` (a `run.toml`), its four tables beside it,
    /// without running it (`record::open`).
    #[staticmethod]
    #[pyo3(signature = (record, network=None, demand=None, transit=None))]
    #[allow(clippy::needless_pass_by_value, reason = "pyo3 hands the arguments over by value")]
    fn open(
        py: Python<'_>,
        record: PathBuf,
        network: Option<&Bound<'_, Network>>,
        demand: Option<&Bound<'_, Demand>>,
        transit: Option<&Bound<'_, Transit>>,
    ) -> PyResult<Self> {
        record::open(py, &record, (network, demand, transit))
    }
}

/// What the pages read of a run: what they keep of it, its iterations (the summary's alone),
/// its record's text, and the warnings of what the readers set aside.
pub(crate) struct Read {
    kept: Kept,
    iterations: Vec<Iteration>,
    record: String,
    warnings: Vec<String>,
}

impl Run {
    /// The run of `inputs` whose record is `record`, its four tables beside it, over `window`.
    /// Its files: its inputs' (canonical, as each input keeps them) and its own.
    pub(crate) fn of(
        record: &Path,
        window: Window,
        ended: Ended,
        (network, demand, transit): RunInputs<'_, '_>,
    ) -> PyResult<Self> {
        let beside = |name: &str| record.with_file_name(name);
        let (n, d, t) = (network.get(), demand.get(), transit.map(Bound::get));
        let inputs = [&n.osm_file, &n.nodes_file, &n.links_file, &d.trips_file];
        let mut files: Vec<PathBuf> = inputs.into_iter().flatten().cloned().collect();
        files.extend(t.map(|t| t.gtfs_file.clone()));
        for path in TABLES.map(beside).iter().chain([&record.to_path_buf()]) {
            files.push(canonical(path)?);
        }
        Ok(Self {
            trips: beside(TABLES[0]),
            links: beside(TABLES[1]),
            iterations: beside(TABLES[2]),
            routes: beside(TABLES[3]),
            record: record.to_path_buf(),
            ended,
            files,
            window,
            network: network.clone().unbind(),
            demand: demand.clone().unbind(),
            transit: transit.map(|t| t.clone().unbind()),
        })
    }

    /// What `page` reads of the run, beside its record, on its inputs: the trips, links and
    /// routes tables (`openmobisim_io_parquet::read`), the iterations for the summary alone, and
    /// the record's text. A file that cannot be read is refused naming it: an `OSError` when it
    /// cannot be opened, else a `ValueError` (a column missing, say); what the readers set aside
    /// comes back as warnings, for the caller to show.
    pub(crate) fn read(&self, page: Option<Page>) -> PyResult<Read> {
        let (network, demand) = (&self.network.get().network, &self.demand.get().demand);
        let timetable = self.transit.as_ref().map(|t| &*t.get().timetable);
        let Window { start, end, bin } = self.window;
        let window = StepGrid::new(Second(start), bin, end.saturating_sub(start))
            .map_err(|e| value_error(&e))?;
        let refused =
            |e: OutputError| if e.io { PyOSError::new_err(e.to_string()) } else { value_error(&e) };
        let folder = self.record.parent().unwrap_or(Path::new("."));
        let dropped = self.ended.dropped();
        let t = openmobisim_io_parquet::read(folder, demand, network, timetable, window, dropped)
            .map_err(refused)?;
        let mut warnings = t.warnings;
        let iterations = match page {
            Some(Page::Map) => Vec::new(),
            _ => {
                let (rows, more) =
                    openmobisim_io_parquet::read_iterations(&self.iterations).map_err(refused)?;
                warnings.extend(more);
                rows
            }
        };
        let record = std::fs::read_to_string(&self.record)
            .map_err(|e| PyOSError::new_err(format!("{}: {e}", self.record.display())))?;
        let kept = Kept::of(demand, &t.trips, t.routes, t.links, window);
        Ok(Read { kept, iterations, record, warnings })
    }

    /// Write the run's `page` to `path`, read from its files; the readers' warnings are shown.
    fn page(
        &self,
        py: Python<'_>,
        page: Page,
        path: PathBuf,
        title: Option<String>,
    ) -> PyResult<PathBuf> {
        let stem = self.page_path(page.method(), &path)?;
        let title = title.unwrap_or(stem);
        let transit = self.transit.as_ref().map(Py::get);
        let written = py.detach(|| {
            let read = self.read(Some(page))?;
            let run = Summarised {
                kept: &read.kept,
                network: &self.network.get().network,
                timetable: transit.map(|t| &*t.timetable),
                lines: transit.map(|t| &*t.lines),
                iterations: &read.iterations,
                stopped_by_rule: self.ended.stopped_by_rule,
                record: &read.record,
            };
            let written = match page {
                Page::Map => {
                    let (network, timetable, lines) = (run.network, run.timetable, run.lines);
                    let mapped =
                        Mapped { kept: run.kept, network, timetable, lines, record: run.record };
                    openmobisim_map::write(&mapped, &title, &path).map_err(|e| match e {
                        MapError::Io(_) => PyOSError::new_err(e.to_string()),
                        _ => value_error(&e),
                    })
                }
                Page::Summary => {
                    openmobisim_summary::write(&run, &title, &path).map_err(|e| match e {
                        SummaryError::Io(_) => PyOSError::new_err(e.to_string()),
                        SummaryError::Map(_) => value_error(&e),
                    })
                }
            };
            written.map(|()| read.warnings)
        });
        warn(py, &written?)?;
        Ok(path)
    }

    /// A page's title, the stem of `path`, once `path` is known not to be one of the run's five
    /// files (its four tables and its record) or one of its input files (canonical paths compared): those are refused, a
    /// `ValueError` naming `method` and the file, before anything is written.
    fn page_path(&self, method: &str, path: &Path) -> PyResult<String> {
        if let Ok(target) = std::fs::canonicalize(path)
            && self.files.contains(&target)
        {
            return Err(PyValueError::new_err(format!(
                "{method}: {} is a file of the run (one of its inputs or outputs); the page \
                 would replace it",
                target.display()
            )));
        }
        Ok(path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default())
    }
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

/// `run`'s `window`, two seconds of the clock; a boolean in it is refused.
fn window_pair(v: &Bound<'_, PyAny>) -> PyResult<(u32, u32)> {
    let pair = v.extract()?;
    keywords::not_boolean("window", v).map(|()| pair)
}

/// Run the loop on `network`, `demand` (built on that network and that transit) and `transit`
/// over `window`, two seconds of the clock, and write `trips.parquet`, `links.parquet`,
/// `iterations.parquet`, `routes.parquet` and `run.toml` into `out`. Keywords: the loop's rows of `PARAMETERS.md`.
#[pyfunction]
#[pyo3(signature = (network, demand, transit=None, *, window, out, **parameters))]
#[allow(clippy::needless_pass_by_value, reason = "pyo3 hands the arguments over by value")]
fn run(
    py: Python<'_>,
    network: &Bound<'_, Network>,
    demand: &Bound<'_, Demand>,
    transit: Option<&Bound<'_, Transit>>,
    #[pyo3(from_py_with = window_pair)] window: (u32, u32),
    out: PathBuf,
    parameters: Option<&Bound<'_, PyDict>>,
) -> PyResult<Run> {
    let given = Given::read(Entry::Run, "run", parameters)?;
    let (network_bound, demand_bound, transit_bound) = (network, demand, transit);
    let (network, demand, transit) = (network.get(), demand.get(), transit.map(|t| t.get()));
    let p = self::parameters(&given, &demand.demand)?;
    built_on(network, demand, transit)?;
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
        network_repairs: &network.repairs,
        trips_given: demand.trips_given,
        demand_repairs: &demand.repairs,
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
        Ok::<_, PyErr>((written, r))
    });
    let (written, r) = written?;
    warn_repairs(py, "run", &r.repairs)?;
    if r.trips.kept.iter().all(|&k| !k) {
        warn_no_trip(py, "run", demand.trips_given, &[&demand.repairs, &r.repairs])?;
    }
    let bounds = (network_bound, demand_bound, transit_bound);
    let window = Window { start: r.window.0.get(), end: r.window.1.get(), bin: p.link_bin };
    let repairs = r.repairs.kinds().iter().map(|k| (k.key.to_owned(), k.count)).collect();
    let ended = Ended { stopped_by_rule: Some(r.stopped_by_rule), repairs };
    Run::of(&written.record, window, ended, bounds)
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
