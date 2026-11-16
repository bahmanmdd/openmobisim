//! `run_file(record, *, out)`: a run's record, `run.toml`, run again.
//!
//! The record is read with Python's standard `tomllib`. Every key and table it holds must be one
//! the writer writes (`openmobisim_io_parquet::KEYS`; a coefficient table's keys are the columns
//! of the choice row): any other is refused by name. Read: `format` (only 1, when present),
//! `window_start_s` and `window_end_s` (required), `[parameters]` (an absent row is the default
//! of `PARAMETERS.md`; each class's coefficients by column name, all sixteen), the input files
//! (`[network] osm_file`, or `nodes_file` and `links_file`; `[demand] trips_file`; `[transit]
//! gtfs_file`), `[demand] classes`, and the three fingerprints. Known and not read: the version,
//! the counts, the readers' reports, `[result]`. Each file's path is joined to the folder of the
//! record's canonical path (the folder it was written in). The network, the transit and the
//! demand are built by the same functions as `Network.from_osm` or `Network.from_tables`,
//! `Transit.from_gtfs` and `Demand.from_table`, each given the rows of `[parameters]` its entry
//! point reads, and the run by `run` with the rest, written into `out`. An input whose
//! fingerprint differs from the record's raises a `UserWarning` naming it and both fingerprints,
//! and the run goes on: its new record carries the new fingerprint.
//!
//! For bikes: `bike_speed` is read from `[parameters]` and given to the network as at
//! `Network.from_tables`; the links' and trips' bike permissions come with their files.

use std::ffi::CString;
use std::path::{Path, PathBuf};

use openmobisim_io_parquet::KEYS;
use openmobisim_loop::parameters::COLUMNS;
use pyo3::conversion::FromPyObjectOwned;
use pyo3::exceptions::{PyOSError, PyUserWarning, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyList};

use crate::keywords::{Entry, entry};
use crate::{Demand, Network, Run, Transit};

/// A refusal of the record: a `ValueError` beginning `run.toml:`.
fn refuse(why: &str) -> PyErr {
    PyValueError::new_err(format!("run.toml: {why}"))
}

/// The table at the dotted name `name` of `toml` (`""`: the top), if the record holds it; a
/// value that is not a table is refused.
fn table<'py>(toml: &Bound<'py, PyDict>, name: &str) -> PyResult<Option<Bound<'py, PyDict>>> {
    let mut at = toml.clone();
    for part in name.split('.').filter(|p| !p.is_empty()) {
        let Some(next) = at.get_item(part)? else { return Ok(None) };
        at = next.cast_into::<PyDict>().map_err(|_| refuse(&format!("{name} is not a table")))?;
    }
    Ok(Some(at))
}

/// Refuse any key or table of `toml` the writer never writes: the tables and keys of `KEYS`,
/// and in each class's coefficient table the columns of the choice row.
fn known(toml: &Bound<'_, PyDict>) -> PyResult<()> {
    for (name, keys) in KEYS {
        let Some(t) = table(toml, name)? else { continue };
        for key in t.keys() {
            let key: String = key.extract()?;
            if !keys.contains(&key.as_str()) {
                let at = if name.is_empty() { String::new() } else { format!(" [{name}]") };
                return Err(refuse(&format!("{key} is not a key of the record{at}")));
            }
        }
    }
    let Some(classes) = table(toml, "parameters.coefficients")? else { return Ok(()) };
    for (class, row) in classes.iter() {
        let class: String = class.extract()?;
        let row = row
            .cast_into::<PyDict>()
            .map_err(|_| refuse(&format!("parameters.coefficients.{class:?} is not a table")))?;
        for column in row.keys() {
            let column: String = column.extract()?;
            if !COLUMNS.contains(&column.as_str()) {
                return Err(refuse(&format!(
                    "{column} is not a column of the choice row [parameters.coefficients.{class:?}]"
                )));
            }
        }
    }
    Ok(())
}

/// Refuse a TOML boolean for the key `dotted`: no key read from the record is a boolean, and
/// Python would read `true` as the number 1.
fn not_boolean(v: &Bound<'_, PyAny>, dotted: &str) -> PyResult<()> {
    if v.is_instance_of::<PyBool>() {
        return Err(refuse(&format!("{dotted}: a boolean ({v}) is not a value of this key")));
    }
    Ok(())
}

/// The value of `key` in the table `name` read as a `T`, if present; a value that is not a `T`,
/// or a boolean, is refused naming the key.
fn value<'py, T: FromPyObjectOwned<'py>>(
    t: &Bound<'py, PyDict>,
    name: &str,
    key: &str,
) -> PyResult<Option<T>> {
    let Some(v) = t.get_item(key)? else { return Ok(None) };
    let dotted = if name.is_empty() { key.to_string() } else { format!("{name}.{key}") };
    not_boolean(&v, &dotted)?;
    v.extract::<T>().map(Some).map_err(|e| {
        let e: PyErr = e.into();
        refuse(&format!("{dotted}: {e}"))
    })
}

/// The value of the required `key` in the table `name`; its absence is refused naming it.
fn required<'py, T: FromPyObjectOwned<'py>>(
    t: &Bound<'py, PyDict>,
    name: &str,
    key: &str,
) -> PyResult<T> {
    let dotted = if name.is_empty() { key.to_string() } else { format!("{name}.{key}") };
    value(t, name, key)?.ok_or_else(|| refuse(&format!("{dotted} is missing; it is required")))
}

/// The keywords of each entry point from `[parameters]`: the rows each reads, `coefficients` as
/// the dict from class to its sixteen in the order of the choice row. A boolean is refused;
/// `stop_walk_max` and `transfer_walk_max` without transit are refused (the writer writes them
/// only with transit). Each other value is checked where its entry point reads it.
fn keywords<'py>(
    py: Python<'py>,
    toml: &Bound<'py, PyDict>,
    with_transit: bool,
) -> PyResult<[Bound<'py, PyDict>; 4]> {
    let [network, transit, demand, run] = [(); 4].map(|()| PyDict::new(py));
    let Some(p) = table(toml, "parameters")? else { return Ok([network, transit, demand, run]) };
    for (name, v) in p.iter() {
        let name: String = name.extract()?;
        if name == "coefficients" {
            run.set_item("coefficients", coefficients(py, toml)?)?;
            continue;
        }
        not_boolean(&v, &format!("parameters.{name}"))?;
        let at = match entry(&name) {
            Some(Entry::Network) => &network,
            Some(Entry::Transit | Entry::Demand) if !with_transit => {
                return Err(refuse(&format!(
                    "parameters.{name} is read only with a transit, [transit] gtfs_file"
                )));
            }
            Some(Entry::Transit) => &transit,
            Some(Entry::Demand) => &demand,
            _ => &run,
        };
        at.set_item(name, v)?;
    }
    Ok([network, transit, demand, run])
}

/// `[parameters.coefficients]` as the dict `run` reads: each class's sixteen coefficients in the
/// order of the choice row, every column required.
fn coefficients<'py>(py: Python<'py>, toml: &Bound<'py, PyDict>) -> PyResult<Bound<'py, PyDict>> {
    let rows = PyDict::new(py);
    let Some(classes) = table(toml, "parameters.coefficients")? else { return Ok(rows) };
    for (class, row) in classes.iter() {
        let class: String = class.extract()?;
        let name = format!("parameters.coefficients.{class:?}");
        let row =
            row.cast_into::<PyDict>().map_err(|_| refuse(&format!("{name} is not a table")))?;
        let values: Vec<f64> =
            COLUMNS.iter().map(|c| required(&row, &name, c)).collect::<PyResult<_>>()?;
        rows.set_item(class, PyList::new(py, values)?)?;
    }
    Ok(rows)
}

/// Warn when an input's fingerprint differs from the one recorded (none recorded: no warning).
fn compare(py: Python<'_>, input: &str, recorded: Option<String>, built: u64) -> PyResult<()> {
    let built = format!("{built:016x}");
    match recorded {
        Some(recorded) if recorded != built => {
            let message = format!(
                "run.toml: the {input} changed since the record was written: its fingerprint is \
                 {built}, the record's {recorded}; the run goes on with the {input} as it is"
            );
            let message = CString::new(message).map_err(|e| refuse(&e.to_string()))?;
            PyErr::warn(py, &py.get_type::<PyUserWarning>(), &message, 1)
        }
        _ => Ok(()),
    }
}

/// The record at `record` read with `tomllib`, every key known and `format`, if given, 1; and
/// the folder its canonical path is in, to which its file paths are joined.
fn load<'py>(py: Python<'py>, record: &Path) -> PyResult<(Bound<'py, PyDict>, PathBuf)> {
    let os_error = |e: std::io::Error| PyOSError::new_err(format!("{}: {e}", record.display()));
    let path = std::fs::canonicalize(record).map_err(os_error)?;
    let text = std::fs::read_to_string(&path).map_err(os_error)?;
    let toml = py.import("tomllib")?.call_method1("loads", (text,))?.cast_into::<PyDict>()?;
    known(&toml)?;
    match value::<i64>(&toml, "", "format")? {
        Some(format) if format != 1 => {
            Err(refuse(&format!("format {format} is not read; the format read is 1")))
        }
        _ => Ok((toml, path.parent().map(Path::to_path_buf).unwrap_or_default())),
    }
}

/// Where the record's network is read from.
enum NetworkFiles {
    Osm(PathBuf),
    Tables(PathBuf, PathBuf),
}

/// `[network]`'s files joined to `folder`: `osm_file`, or `nodes_file` and `links_file`.
fn network_files(net: &Bound<'_, PyDict>, folder: &Path) -> PyResult<NetworkFiles> {
    let file = |key: &str| -> PyResult<Option<PathBuf>> {
        Ok(value::<String>(net, "network", key)?.map(|f| folder.join(f)))
    };
    match (file("osm_file")?, file("nodes_file")?, file("links_file")?) {
        (Some(osm), None, None) => Ok(NetworkFiles::Osm(osm)),
        (Some(_), _, _) => Err(refuse(
            "network.osm_file and network.nodes_file or links_file are two networks; a record \
             names one",
        )),
        (None, nodes, links) => Ok(NetworkFiles::Tables(
            nodes.ok_or_else(|| refuse("network.nodes_file is missing (or osm_file)"))?,
            links.ok_or_else(|| refuse("network.links_file is missing (or osm_file)"))?,
        )),
    }
}

/// Run again from the record at `record`, a `run.toml`, writing the run into `out`, and return
/// its `Run`. The inputs are the files the record names, joined to its folder; the parameters
/// are its `[parameters]`, a row absent there taking the default of `PARAMETERS.md`. The keys,
/// the window, the files, the classes, the fingerprints and the booleans are checked before an
/// input is built; each parameter's value is checked by the entry point that reads it, so a
/// value the run refuses is refused after the inputs before it are built.
#[pyfunction]
#[pyo3(signature = (record, *, out))]
#[allow(clippy::needless_pass_by_value, reason = "pyo3 hands the arguments over by value")]
pub(crate) fn run_file(py: Python<'_>, record: PathBuf, out: PathBuf) -> PyResult<Run> {
    let (toml, folder) = load(py, &record)?;
    let window: (u32, u32) =
        (required(&toml, "", "window_start_s")?, required(&toml, "", "window_end_s")?);
    let missing = |name: &str| refuse(&format!("[{name}] is missing; it is required"));
    let net = table(&toml, "network")?.ok_or_else(|| missing("network"))?;
    let dem = table(&toml, "demand")?.ok_or_else(|| missing("demand"))?;
    let tra = table(&toml, "transit")?;
    let [network_kw, transit_kw, demand_kw, run_kw] = keywords(py, &toml, tra.is_some())?;
    let network_files = network_files(&net, &folder)?;
    let feed = tra.as_ref().map(|t| required::<String>(t, "transit", "gtfs_file")).transpose()?;
    let trips = folder.join(required::<String>(&dem, "demand", "trips_file")?);
    let classes: Vec<String> = required(&dem, "demand", "classes")?;
    let recorded = [
        value::<String>(&net, "network", "fingerprint")?,
        value::<String>(&dem, "demand", "fingerprint")?,
        tra.as_ref().map(|t| value::<String>(t, "transit", "fingerprint")).transpose()?.flatten(),
    ];

    let network = match network_files {
        NetworkFiles::Osm(osm) => Network::from_osm(py, osm, Some(&network_kw))?,
        NetworkFiles::Tables(nodes, links) => {
            let (nodes, links) = (nodes.into_pyobject(py)?, links.into_pyobject(py)?);
            Network::from_tables(py, &nodes, &links, Some(&network_kw))?
        }
    };
    let network = Bound::new(py, network)?;
    let transit = match feed {
        None => None,
        Some(feed) => {
            let transit = Transit::from_gtfs(py, folder.join(feed), &network, Some(&transit_kw))?;
            Some(Bound::new(py, transit)?)
        }
    };
    let trips = trips.into_pyobject(py)?;
    let demand =
        Demand::from_table(py, &trips, classes, &network, transit.as_ref(), Some(&demand_kw))?;
    let demand = Bound::new(py, demand)?;
    let [network_print, demand_print, transit_print] = recorded;
    compare(py, "network", network_print, network.get().network.fingerprint())?;
    compare(py, "demand", demand_print, demand.get().demand.fingerprint())?;
    if let Some(t) = &transit {
        compare(py, "transit", transit_print, t.get().timetable.fingerprint())?;
    }
    crate::run(py, &network, &demand, transit.as_ref(), window, out, Some(&run_kw))
}
