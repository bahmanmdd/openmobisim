//! Transit for Python (S199): reading a GTFS feed, the toy network's timetable,
//! and what a run's transit did.

use std::sync::Arc;

use numpy::IntoPyArray;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;

use openmobisim_core_graph::geometry::{LonLat, ground_distance_metres};
use openmobisim_core_graph::layers::StaticLayer;
use openmobisim_core_routes::NodeSnapper;
use openmobisim_core_sim::{BusReport, TransitResult};
use openmobisim_core_transit::{
    ServiceDate, ServiceKind, Timetable, TransitDefaults, UNKNOWN_TIME,
};
use openmobisim_core_types::ids::{EntityId, TransitRunId};
use openmobisim_io_gtfs::GtfsReport;

use crate::network::PyNetwork;

/// The timetable of one service day: stops, lines and every run that day.
///
/// Read one with ``openmobisim.transit_read_gtfs``; give it to a
/// ``Scenario`` (``transit=``) for its ``"transit"`` trips, and its buses ride the
/// roads among the cars.
#[pyclass(name = "Transit", module = "openmobisim._core", frozen)]
pub struct PyTransit {
    pub(crate) timetable: Arc<Timetable>,
    report: Option<GtfsReport>,
}

impl PyTransit {
    pub(crate) fn new(timetable: Timetable, report: Option<GtfsReport>) -> Self {
        Self { timetable: Arc::new(timetable), report }
    }
}

#[pymethods]
impl PyTransit {
    /// The service day, ``YYYY-MM-DD``.
    #[getter]
    fn date(&self) -> String {
        self.timetable.date().to_string()
    }

    /// How many stops (only those a run calls at).
    #[getter]
    fn stop_count(&self) -> u32 {
        self.timetable.stop_count()
    }

    /// How many lines (GTFS routes).
    #[getter]
    fn route_count(&self) -> u32 {
        self.timetable.route_count()
    }

    /// How many runs: one vehicle's trip on the day.
    #[getter]
    fn run_count(&self) -> u32 {
        self.timetable.run_count()
    }

    /// How many calls: a run at a stop.
    #[getter]
    fn call_count(&self) -> usize {
        self.timetable.call_count()
    }

    /// Runs by kind of service: ``{"bus": …, "tram": …, "metro": …, "rail": …,
    /// "ferry": …, "other": …}``, the kinds present.
    fn runs_by_kind<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let mut counts = [0u32; ServiceKind::ALL.len()];
        for r in 0..self.timetable.run_count() {
            counts[self.timetable.run_kind(TransitRunId::new(r)) as usize] += 1;
        }
        let d = PyDict::new(py);
        for kind in ServiceKind::ALL {
            if counts[kind as usize] > 0 {
                d.set_item(kind.as_str(), counts[kind as usize])?;
            }
        }
        Ok(d)
    }

    /// What reading the feed found, kept and skipped: counts by name (``None``
    /// for a timetable not read from a feed). ``date_chosen`` says whether the
    /// day was the feed's busiest weekday rather than one asked for.
    fn read_report<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyDict>>> {
        let Some(r) = &self.report else { return Ok(None) };
        let d = PyDict::new(py);
        d.set_item("date_chosen", r.date_chosen)?;
        d.set_item("stops_in_feed", r.stops_in_feed)?;
        d.set_item("stops_in_area", r.stops_in_area)?;
        d.set_item("stops_kept", r.stops_kept)?;
        d.set_item("trips_in_feed", r.trips_in_feed)?;
        d.set_item("trips_on_date", r.trips_on_date)?;
        d.set_item("runs_kept", r.runs_kept)?;
        d.set_item("stop_time_rows", r.stop_time_rows)?;
        d.set_item("calls_kept", r.calls_kept)?;
        d.set_item("times_interpolated", r.times_interpolated)?;
        d.set_item("trips_untimed", r.trips_untimed)?;
        d.set_item("rows_skipped", r.rows_skipped)?;
        d.set_item("transfers_kept", r.transfers_kept)?;
        d.set_item("transfers_skipped", r.transfers_skipped)?;
        d.set_item("frequency_rows", r.frequency_rows)?;
        d.set_item("runs_from_frequencies", r.runs_from_frequencies)?;
        Ok(Some(d))
    }

    /// Every line (GTFS route): ``{"route_id", "name", "kind", "runs", "first_s", "last_s"}``,
    /// lists by line in id order (S238): the ids and names ``transit_edit`` takes, the kind of
    /// service, how many runs it makes that day, and the first and last of their departures from
    /// their first stops, in seconds after midnight (``None`` for a line with no run).
    fn lines<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let t = &self.timetable;
        let n = t.route_count();
        let mut runs = vec![0u32; n as usize];
        let mut first: Vec<Option<u32>> = vec![None; n as usize];
        let mut last: Vec<Option<u32>> = vec![None; n as usize];
        for r in 0..t.run_count() {
            let run = TransitRunId::new(r);
            let line = t.run_route(run) as usize;
            let leaves = t.scheduled().departure[t.run_calls(run).start];
            runs[line] += 1;
            first[line] = Some(first[line].map_or(leaves, |f| f.min(leaves)));
            last[line] = Some(last[line].map_or(leaves, |l| l.max(leaves)));
        }
        let d = PyDict::new(py);
        d.set_item(
            "route_id",
            (0..n).map(|r| t.route_ids().external(r).to_string()).collect::<Vec<_>>(),
        )?;
        d.set_item("name", (0..n).map(|r| t.route_short_name(r).to_string()).collect::<Vec<_>>())?;
        d.set_item("kind", (0..n).map(|r| t.route_kind(r).as_str()).collect::<Vec<_>>())?;
        d.set_item("runs", runs)?;
        d.set_item("first_s", first)?;
        d.set_item("last_s", last)?;
        Ok(d)
    }

    /// Every stop: ``{"stop_id", "name", "lon", "lat"}``, lists and arrays by stop.
    fn stops<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let t = &self.timetable;
        let n = t.stop_count();
        let ids: Vec<String> = (0..n).map(|s| t.stop_ids().external(s).to_string()).collect();
        let names: Vec<String> =
            (0..n).map(|s| t.stop_name(EntityId::new(s)).to_string()).collect();
        let at: Vec<LonLat> = (0..n).map(|s| t.stop_position(EntityId::new(s))).collect();
        let d = PyDict::new(py);
        d.set_item("stop_id", ids)?;
        d.set_item("name", names)?;
        d.set_item("lon", at.iter().map(|p| p.lon).collect::<Vec<_>>().into_pyarray(py))?;
        d.set_item("lat", at.iter().map(|p| p.lat).collect::<Vec<_>>().into_pyarray(py))?;
        Ok(d)
    }

    fn __repr__(&self) -> String {
        format!(
            "Transit({}: {} runs, {} stops, {} lines)",
            self.timetable.date(),
            self.timetable.run_count(),
            self.timetable.stop_count(),
            self.timetable.route_count()
        )
    }
}

fn parse_date(text: &str) -> PyResult<ServiceDate> {
    ServiceDate::parse(&text.replace('-', ""))
        .ok_or_else(|| PyValueError::new_err(format!("not a date (YYYY-MM-DD): {text:?}")))
}

/// Read one service day of a GTFS feed.
///
/// Args:
///     path: The feed: a ``.zip``, or a folder of its ``.txt`` files.
///     network: If given, only the stops a pedestrian can reach on the
///         network's walk layer are kept (within 300 m of its nearest node),
///         and every run keeps its calls at them: the feed of a country clipped
///         to a city while it is read.
///     date: The service day, ``"YYYY-MM-DD"``. ``None`` (the default) reads the
///         feed's busiest weekday.
///
/// Raises:
///     ValueError: If the feed cannot be read, lacks a file every feed has, or
///         runs nothing on the day.
#[pyfunction]
#[pyo3(signature = (path, network=None, date=None))]
pub fn transit_read_gtfs(
    path: &str,
    network: Option<PyRef<'_, PyNetwork>>,
    date: Option<&str>,
) -> PyResult<PyTransit> {
    let date = date.map(parse_date).transpose()?;
    let reach = TransitDefaults::SHIPPED.stop_walk_snap_m;
    let result = match network {
        Some(net) => {
            let walk = net.static_layer(StaticLayer::Walk)?;
            let graph = walk.network();
            let snapper = NodeSnapper::every_node(graph);
            // The layer's box, widened by the reach: a stop outside it is out of reach,
            // and a nearest-node search from far outside the grid is slow.
            let (mut w, mut s, mut e, mut n) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
            for raw in 0..graph.node_count() {
                let at = graph.node_lonlat(EntityId::new(raw));
                (w, s, e, n) = (w.min(at.lon), s.min(at.lat), e.max(at.lon), n.max(at.lat));
            }
            let dlat = reach / 110_000.0;
            let dlon = reach / (110_000.0 * ((s + n) / 2.0).to_radians().cos().max(0.01));
            let keep = |p: LonLat| {
                if p.lon < w - dlon || p.lon > e + dlon || p.lat < s - dlat || p.lat > n + dlat {
                    return false;
                }
                let node = snapper.nearest(graph, p);
                ground_distance_metres(p, graph.node_lonlat(node)) <= reach
            };
            openmobisim_io_gtfs::read_gtfs(path, date, &keep)
        }
        None => openmobisim_io_gtfs::read_gtfs(path, date, &|_| true),
    };
    let (timetable, report) = result.map_err(|e| PyValueError::new_err(e.to_string()))?;
    Ok(PyTransit::new(timetable, Some(report)))
}

/// The toy network's timetable: the tram ``T1`` (stop ``N1`` to stop ``H``
/// at ``D2`` and back, every 10 minutes, 300 s) and the bus ``B1`` (``W``,
/// ``M``, ``D1`` on the arterial, every 10 minutes, scheduled 120 s and 240 s
/// after ``W``), for three hours from 0 s: the fixture of the hand-derived
/// transit cases.
#[pyfunction]
pub fn toy_network_transit() -> PyTransit {
    PyTransit::new(openmobisim_core_transit::examples::toy_transit(), None)
}

/// Every call of the run's timetable as columns, with its times in the run and
/// its passengers: what `Run.transit_calls` returns.
pub(crate) fn transit_calls<'py>(
    py: Python<'py>,
    timetable: &Timetable,
    result: &TransitResult,
) -> PyResult<Bound<'py, PyDict>> {
    let n = timetable.call_count();
    let mut run = Vec::with_capacity(n);
    let mut line = Vec::with_capacity(n);
    let mut kind = Vec::with_capacity(n);
    let mut stop = Vec::with_capacity(n);
    let mut sequence = Vec::with_capacity(n);
    for r in 0..timetable.run_count() {
        let id = TransitRunId::new(r);
        let route = timetable.run_route(id);
        for (k, c) in timetable.run_calls(id).enumerate() {
            run.push(timetable.run_ids().external(r).to_string());
            line.push(timetable.route_short_name(route).to_string());
            kind.push(timetable.route_kind(route).as_str());
            stop.push(timetable.stop_ids().external(timetable.call_stop(c).raw()).to_string());
            sequence.push(u32::try_from(k).expect("calls of a run fit u32"));
        }
    }
    let seconds = |v: &[u32]| -> Vec<f64> {
        v.iter().map(|&t| if t == UNKNOWN_TIME { f64::NAN } else { f64::from(t) }).collect()
    };
    let d = PyDict::new(py);
    d.set_item("run", run)?;
    d.set_item("line", line)?;
    d.set_item("kind", kind)?;
    d.set_item("stop_id", stop)?;
    d.set_item("sequence", sequence.into_pyarray(py))?;
    let s = timetable.scheduled();
    d.set_item("scheduled_arrival_s", seconds(&s.arrival).into_pyarray(py))?;
    d.set_item("scheduled_departure_s", seconds(&s.departure).into_pyarray(py))?;
    d.set_item("arrival_s", seconds(&result.times.arrival).into_pyarray(py))?;
    d.set_item("departure_s", seconds(&result.times.departure).into_pyarray(py))?;
    d.set_item("boardings", result.boardings.clone().into_pyarray(py))?;
    d.set_item("alightings", result.alightings.clone().into_pyarray(py))?;
    d.set_item("on_board", result.on_board(timetable).into_pyarray(py))?;
    Ok(d)
}

/// How transit went in a run, as numbers by name: what `Run.transit_summary` is.
pub(crate) fn transit_summary<'py>(
    py: Python<'py>,
    result: &TransitResult,
    buses: Option<BusReport>,
) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item("boardings", result.boardings.iter().sum::<f64>())?;
    if let Some(b) = buses {
        d.set_item("bus_groups_on_roads", b.groups_on_roads)?;
        d.set_item("bus_groups_by_schedule_off_road", b.by_schedule_off_road)?;
        d.set_item("bus_groups_by_schedule_no_route", b.by_schedule_no_route)?;
        d.set_item("bus_groups_by_schedule_implausible", b.by_schedule_implausible)?;
    }
    if let Some(b) = result.buses {
        d.set_item("bus_runs_on_roads", b.runs_on_roads)?;
        d.set_item("bus_runs_arrived", b.runs_arrived)?;
        d.set_item("bus_delay_mean_s", b.delay_mean_s)?;
    }
    Ok(d)
}

/// The same day with a scenario's changes made (S238): lines (by index) cancelled, and lines
/// run at a new headway in a window, `(line, headway_s, from_s, to_s)`; with what was done.
/// `openmobisim.transit_edit` is the Python face.
///
/// # Errors
///
/// `ValueError` for a line out of range, a headway of 0 or a window that ends before it
/// starts.
#[pyfunction]
#[pyo3(signature = (transit, cancelled=Vec::new(), headways=Vec::new()))]
pub fn transit_edit<'py>(
    py: Python<'py>,
    transit: PyRef<'_, PyTransit>,
    cancelled: Vec<u32>,
    headways: Vec<(u32, u32, u32, u32)>,
) -> PyResult<(PyTransit, Bound<'py, PyDict>)> {
    use openmobisim_core_transit::{Headway, ServiceChanges};
    let changes = ServiceChanges {
        cancelled,
        headways: headways
            .into_iter()
            .map(|(route, headway_s, from_s, to_s)| Headway { route, headway_s, from_s, to_s })
            .collect(),
    };
    let (timetable, report) =
        transit.timetable.with_changes(&changes).map_err(PyValueError::new_err)?;
    let d = PyDict::new(py);
    d.set_item("runs_cancelled", report.runs_cancelled)?;
    d.set_item("runs_replaced", report.runs_replaced)?;
    d.set_item("runs_added", report.runs_added)?;
    Ok((PyTransit::new(timetable, transit.report.clone()), d))
}
