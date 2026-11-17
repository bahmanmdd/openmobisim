//! What the readers counted and set right, for Python: one `UserWarning` per kind of repair, and
//! the readers' reports and repairs as dicts.

use openmobisim_io_gtfs::Report as GtfsReport;
use openmobisim_io_osm::Report as OsmReport;
use openmobisim_types::repairs::Repairs;
use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::value_error;

/// Show each of `warnings` as a Python `UserWarning`.
pub(crate) fn warn(py: Python<'_>, warnings: &[String]) -> PyResult<()> {
    for w in warnings {
        let message = std::ffi::CString::new(w.as_str()).map_err(|e| value_error(&e))?;
        PyErr::warn(py, &py.get_type::<pyo3::exceptions::PyUserWarning>(), &message, 1)?;
    }
    Ok(())
}

/// Show each kind of `repairs` as one Python `UserWarning`, naming `method`, the count, what was
/// done, the kind's key and its first ids.
pub(crate) fn warn_repairs(py: Python<'_>, method: &str, repairs: &Repairs) -> PyResult<()> {
    let lines: Vec<String> = repairs
        .kinds()
        .iter()
        .map(|k| {
            let more = if k.count > k.first.len() as u64 { ", …" } else { "" };
            format!(
                "{method}: {} {} ({}; first: {}{more})",
                k.count,
                k.what,
                k.key,
                k.first.join(", ")
            )
        })
        .collect();
    warn(py, &lines)
}

/// The kinds of repair that drop a trip: the trips table's reader's, the demand's, the run's.
const DROPPING: [&str; 7] = [
    "trips_unread",
    "trips_repeated",
    "trips_of_unknown_class",
    "trips_unplaced",
    "trips_without_mode",
    "trips_outside_window",
    "trips_after_window",
];

/// One `UserWarning` saying that nothing is left to run: "<method>: no trip is left: N given,
/// all dropped (by reason: …)", the reasons each dropping kind of `repairs` with its count.
pub(crate) fn warn_no_trip(
    py: Python<'_>,
    method: &str,
    given: u64,
    repairs: &[&Repairs],
) -> PyResult<()> {
    let reasons: Vec<String> = repairs
        .iter()
        .flat_map(|r| r.kinds())
        .filter(|k| DROPPING.contains(&k.key))
        .map(|k| format!("{} {}", k.count, k.key))
        .collect();
    let why = if reasons.is_empty() {
        String::new()
    } else {
        format!(" (by reason: {})", reasons.join(", "))
    };
    warn(py, &[format!("{method}: no trip is left: {given} given, all dropped{why}")])
}

/// `repairs` as a dict, each kind's key to its count, in the order first met.
pub(crate) fn repairs_dict<'a>(
    py: Python<'_>,
    repairs: impl IntoIterator<Item = (&'a str, u64)>,
) -> PyResult<Py<PyDict>> {
    let d = PyDict::new(py);
    for (key, count) in repairs {
        d.set_item(key, count)?;
    }
    Ok(d.unbind())
}

/// A reader's repairs as `(key, count)` pairs.
pub(crate) fn counts(repairs: &Repairs) -> impl Iterator<Item = (&str, u64)> {
    repairs.kinds().iter().map(|k| (k.key, k.count))
}

/// The OSM reader's counts as a dict.
pub(crate) fn osm_report(py: Python<'_>, r: &OsmReport) -> PyResult<Py<PyDict>> {
    let d = PyDict::new(py);
    d.set_item("nodes", r.nodes)?;
    d.set_item("links", r.links)?;
    d.set_item("links_unreachable", r.links_unreachable)?;
    d.set_item("loops_dropped", r.loops_dropped)?;
    d.set_item("ways_cut", r.ways_cut)?;
    Ok(d.unbind())
}

/// The GTFS reader's counts as a dict; `date` as GTFS writes it, `YYYYMMDD`.
pub(crate) fn gtfs_report(py: Python<'_>, r: &GtfsReport) -> PyResult<Py<PyDict>> {
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
