//! A run's record, `run.toml`, read back: `run_file(record, *, out)` runs it again;
//! `Run.open(record, network=None, demand=None, transit=None)` opens the finished run beside it.
//!
//! The record is read with Python's standard `tomllib`. A key or table under `[parameters]`
//! the writer never writes is refused by name (a parameter mistyped would run on its default);
//! one elsewhere is ignored, all of them named in one `UserWarning` (`openmobisim_io_parquet::KEYS`;
//! a coefficient table's keys are the columns of the choice row; a `repairs` table's keys are
//! not checked); `stop_walk_max` or `transfer_walk_max` without `[transit]` is ignored, warned.
//! Read: `format` (only 1, when present),
//! `window_start_s` and `window_end_s` (required; `run_file` runs from `window_end_given_s`
//! where the record has it, the end given before the run moved it to a whole bin), `[parameters]` (an absent row is the default
//! of `PARAMETERS.md`; each class's coefficients by column name, all sixteen), the input files
//! (`[network] osm_file`, or `nodes_file` and `links_file`; `[demand] trips_file`; `[transit]
//! gtfs_file`), `[demand] classes`, and the three fingerprints; for `Run.open` also `[result]
//! stopped_by_rule` (absent: the run's end is not recorded, which the pages say) and
//! `[result.repairs]`. Known and not read: the version, the counts, the readers' reports. Each
//! file's path is joined to the folder of the record's canonical path (the folder it was written
//! in). Both build their inputs by one function, [`inputs`]: the network, the transit and the
//! demand by the same functions as `Network.from_osm` or `Network.from_tables`,
//! `Transit.from_gtfs` and `Demand.from_table`, each given the rows of `[parameters]` its entry
//! point reads; `run_file` then runs `run` with the rest, written into `out`, and `Run.open`
//! reads the run's tables. An input whose fingerprint differs from the record's, or that the
//! record has no fingerprint of, raises a `UserWarning` naming it (and both fingerprints), and
//! the work goes on: `run_file`'s new record carries the new fingerprint.
//!
//! For bikes: `bike_speed` is read from `[parameters]` and given to the network as at
//! `Network.from_tables`; the links' and trips' bike permissions come with their files.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use openmobisim_io_parquet::KEYS;
use openmobisim_loop::parameters::COLUMNS;
use pyo3::conversion::FromPyObjectOwned;
use pyo3::exceptions::{PyOSError, PyValueError};
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

/// The keys and tables of `toml` the writer never writes: under `[parameters]` refused by name
/// (in each class's coefficient table, a column that is not one of the choice row's); elsewhere
/// ignored, all named in one `UserWarning`. A `repairs` table's keys are kinds of repair, not
/// checked.
fn known(toml: &Bound<'_, PyDict>) -> PyResult<()> {
    let mut ignored = Vec::new();
    for (name, keys) in KEYS {
        let Some(t) = table(toml, name)? else { continue };
        for key in t.keys() {
            let key: String = key.extract()?;
            if keys.contains(&key.as_str()) {
                continue;
            }
            let at = if name.is_empty() { String::new() } else { format!(" [{name}]") };
            if name == "parameters" {
                return Err(refuse(&format!("{key} is not a key of the record{at}")));
            }
            ignored.push(format!("{key}{at}"));
        }
    }
    if !ignored.is_empty() {
        let message = format!(
            "run.toml: {} keys the writer never writes, ignored: {}",
            ignored.len(),
            ignored.join(", ")
        );
        crate::warn(toml.py(), &[message])?;
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
/// `stop_walk_max` and `transfer_walk_max` without transit are ignored, warned (the writer
/// writes them only with transit, and nothing reads them without). Each other value is checked
/// where its entry point reads it.
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
                let message = format!(
                    "run.toml: parameters.{name} is ignored: it is read only with a transit, \
                     [transit] gtfs_file, and the record has none"
                );
                crate::warn(py, &[message])?;
                continue;
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

/// What an input the record names no file of means: for `run_file`, a key the record lacks,
/// refused naming it; for `Run.open`, an input the caller must pass, refused naming the keyword
/// when it is not passed.
#[derive(Clone, Copy, PartialEq, Eq)]
enum NoFile {
    MissingKey,
    PassIt,
}

/// The run's inputs as its record names them (rebuilt from its files, or passed), and what else
/// `run` reads from it.
struct Rebuilt<'py> {
    network: Bound<'py, Network>,
    demand: Bound<'py, Demand>,
    transit: Option<Bound<'py, Transit>>,
    run_keywords: Bound<'py, PyDict>,
    window: (u32, u32),
    /// The window as given to the run: `window_end_given_s` where the record has it.
    window_given: (u32, u32),
}

/// The inputs passed to `Run.open` (none to `run_file`): network, demand, transit.
pub(crate) type Passed<'a, 'py> = (
    Option<&'a Bound<'py, Network>>,
    Option<&'a Bound<'py, Demand>>,
    Option<&'a Bound<'py, Transit>>,
);

/// An input of the record compared with the one built or passed: a `UserWarning` naming it and
/// both fingerprints when they differ, or saying it cannot be compared when the record has no
/// fingerprint of it; the work goes on in both.
fn compare(py: Python<'_>, input: &str, recorded: Option<String>, built: u64) -> PyResult<()> {
    let built = format!("{built:016x}");
    let message = match recorded {
        Some(recorded) if recorded == built => return Ok(()),
        Some(recorded) => format!(
            "run.toml: the {input} changed since the record was written: its fingerprint is \
             {built}, the record's {recorded}; the work goes on with the {input} as it is"
        ),
        None => format!(
            "run.toml: the record has no fingerprint of the {input}: the {input} given or \
             rebuilt ({built}) cannot be compared with the run's; the work goes on with it"
        ),
    };
    crate::warn(py, &[message])
}

/// The inputs the record `toml` (its files joined to `folder`) names: each one `passed` taken as
/// it is, each other built from the record's file exactly as `Network.from_osm` or
/// `Network.from_tables`, `Transit.from_gtfs` and `Demand.from_table` build it, with the rows of
/// `[parameters]` its entry point reads. An input the record has no file for is met as `no_file`
/// says; a transit passed to a run that had none is set aside with a warning; passed inputs
/// must be built on one another (a demand on the network and transit used, a transit on the
/// network). Then each fingerprint is compared with the record's ([`compare`]).
fn inputs<'py>(
    py: Python<'py>,
    toml: &Bound<'py, PyDict>,
    folder: &Path,
    passed: Passed<'_, 'py>,
    no_file: NoFile,
) -> PyResult<Rebuilt<'py>> {
    let window: (u32, u32) =
        (required(toml, "", "window_start_s")?, required(toml, "", "window_end_s")?);
    let given_end: Option<u32> = value(toml, "", "window_end_given_s")?;
    let window_given = (window.0, given_end.unwrap_or(window.1));
    let missing = |name: &str| refuse(&format!("[{name}] is missing; it is required"));
    let net = table(toml, "network")?.ok_or_else(|| missing("network"))?;
    let dem = table(toml, "demand")?.ok_or_else(|| missing("demand"))?;
    let tra = table(toml, "transit")?;
    let [network_kw, transit_kw, demand_kw, run_keywords] = keywords(py, toml, tra.is_some())?;
    let pass_it = |input: &str| {
        refuse(&format!(
            "the {input} was given in memory when the run was made (the record names no file of \
             it); pass it: {input}="
        ))
    };
    let unfiled =
        ["osm_file", "nodes_file", "links_file"].iter().all(|k| !net.contains(k).unwrap_or(false));
    if passed.0.is_none() && unfiled && no_file == NoFile::PassIt {
        return Err(pass_it("network"));
    }
    let network_files =
        passed.0.map_or_else(|| network_files(&net, folder).map(Some), |_| Ok(None))?;
    let passed_transit = match (&tra, passed.2) {
        (None, Some(_)) => {
            crate::warn(py, &["the transit passed is set aside: the run had none".to_owned()])?;
            None
        }
        (_, t) => t,
    };
    let feed = match (&tra, passed_transit) {
        (Some(t), None) => Some(required::<String>(t, "transit", "gtfs_file")?),
        _ => None,
    };
    let trips = match passed.1 {
        Some(_) => None,
        None if no_file == NoFile::PassIt && !dem.contains("trips_file")? => {
            return Err(pass_it("demand"));
        }
        None => Some(folder.join(required::<String>(&dem, "demand", "trips_file")?)),
    };
    let classes: Vec<String> = required(&dem, "demand", "classes")?;
    let recorded = [
        value::<String>(&net, "network", "fingerprint")?,
        value::<String>(&dem, "demand", "fingerprint")?,
        tra.as_ref().map(|t| value::<String>(t, "transit", "fingerprint")).transpose()?.flatten(),
    ];

    let network = match (passed.0, network_files) {
        (Some(n), _) => n.clone(),
        (None, Some(files)) => Bound::new(py, build_network(py, files, &network_kw)?)?,
        (None, None) => unreachable!("a network passed or its files"),
    };
    let transit = match (passed_transit, feed) {
        (Some(t), _) => Some(t.clone()),
        (None, Some(feed)) => Some(Bound::new(
            py,
            Transit::from_gtfs(py, folder.join(feed), &network, Some(&transit_kw))?,
        )?),
        (None, None) => None,
    };
    if transit.as_ref().is_some_and(|t| !Arc::ptr_eq(&t.get().network, &network.get().network)) {
        return Err(PyValueError::new_err("the transit was read on another network"));
    }
    let demand = match (passed.1, trips) {
        (Some(d), _) => d.clone(),
        (None, Some(trips)) => {
            let trips = trips.into_pyobject(py)?;
            let built = Demand::from_table(
                py,
                &trips,
                classes,
                &network,
                transit.as_ref(),
                Some(&demand_kw),
            )?;
            Bound::new(py, built)?
        }
        (None, None) => unreachable!("a demand passed or its file"),
    };
    agree(py, (&network, &demand, transit.as_ref()), recorded)?;
    Ok(Rebuilt { network, demand, transit, run_keywords, window, window_given })
}

/// The network read from its record's files, with the keywords of `[parameters]` it reads.
fn build_network(
    py: Python<'_>,
    files: NetworkFiles,
    keywords: &Bound<'_, PyDict>,
) -> PyResult<Network> {
    match files {
        NetworkFiles::Osm(osm) => Network::from_osm(py, osm, Some(keywords)),
        NetworkFiles::Tables(nodes, links) => {
            let (nodes, links) = (nodes.into_pyobject(py)?, links.into_pyobject(py)?);
            Network::from_tables(py, &nodes, &links, Some(keywords))
        }
    }
}

/// Refuse a demand not built on the network and transit used (`built_on`); then compare each
/// fingerprint with the record's (`recorded`: network, demand, transit).
fn agree(
    py: Python<'_>,
    (network, demand, transit): crate::RunInputs<'_, '_>,
    recorded: [Option<String>; 3],
) -> PyResult<()> {
    let d = demand.get();
    crate::built_on(network.get(), d, transit.map(Bound::get))?;
    let [network_print, demand_print, transit_print] = recorded;
    compare(py, "network", network_print, network.get().network.fingerprint())?;
    compare(py, "demand", demand_print, d.demand.fingerprint())?;
    if let Some(t) = transit {
        compare(py, "transit", transit_print, t.get().timetable.fingerprint())?;
    }
    Ok(())
}

/// Run again from the record at `record`, a `run.toml`, writing the run into `out`, and return
/// its `Run`. The inputs are the files the record names, joined to its folder ([`inputs`]); the
/// parameters are its `[parameters]`, a row absent there taking the default of `PARAMETERS.md`.
/// The keys, the window, the files, the classes, the fingerprints and the booleans are checked
/// before an input is built; each parameter's value is checked by the entry point that reads it,
/// so a value the run refuses is refused after the inputs before it are built. An input whose
/// fingerprint differs from the record's raises a `UserWarning`, and the run goes on.
#[pyfunction]
#[pyo3(signature = (record, *, out))]
#[allow(clippy::needless_pass_by_value, reason = "pyo3 hands the arguments over by value")]
pub(crate) fn run_file(py: Python<'_>, record: PathBuf, out: PathBuf) -> PyResult<Run> {
    let (toml, folder) = load(py, &record)?;
    let i = inputs(py, &toml, &folder, (None, None, None), NoFile::MissingKey)?;
    let (network, demand, transit) = (&i.network, &i.demand, i.transit.as_ref());
    crate::run(py, network, demand, transit, i.window_given, out, Some(&i.run_keywords))
}

/// Open the finished run whose record is `record`, a `run.toml`, its four tables beside it,
/// without running anything. The inputs are `passed`, or rebuilt from the record's files
/// ([`inputs`]; one the record names no file of must be passed); an input whose fingerprint
/// differs from the record's, or that it has no fingerprint of, is shown in a warning and the
/// opening goes on. The window, `link_bin` (absent there: its default), `[result]
/// stopped_by_rule` (a boolean; absent, the run's end is not recorded, which the pages say) and
/// `[result.repairs]` (each an integer) come from the record. The three tables the pages read
/// and the iterations are read once here: a table that cannot be read is refused naming it, and
/// what the readers set aside is shown in warnings.
pub(crate) fn open(py: Python<'_>, record: &Path, passed: Passed<'_, '_>) -> PyResult<Run> {
    let (toml, folder) = load(py, record)?;
    let i = inputs(py, &toml, &folder, passed, NoFile::PassIt)?;
    let bin = match i.run_keywords.get_item("link_bin")? {
        Some(b) => b.extract::<u32>().map_err(|e| refuse(&format!("parameters.link_bin: {e}")))?,
        None => openmobisim_loop::parameters::Parameters::defaults(0).link_bin,
    };
    let stopped = match table(&toml, "result")? {
        Some(result) => result.get_item("stopped_by_rule")?,
        None => None,
    };
    let stopped_by_rule = stopped
        .map(|v| {
            let flag = v.cast_into::<PyBool>().map_err(|e| {
                refuse(&format!("result.stopped_by_rule is {}, not a boolean", e.into_inner()))
            })?;
            Ok::<_, PyErr>(flag.is_true())
        })
        .transpose()?;
    let mut repairs = Vec::new();
    if let Some(t) = table(&toml, "result.repairs")? {
        for key in t.keys() {
            let key: String = key.extract()?;
            repairs.push((key.clone(), required::<u64>(&t, "result.repairs", &key)?));
        }
    }
    let window = crate::Window { start: i.window.0, end: i.window.1, bin };
    let inputs = (&i.network, &i.demand, i.transit.as_ref());
    let ended = crate::Ended { stopped_by_rule, repairs };
    let opened = Run::of(record, window, ended, inputs)?;
    let read = py.detach(|| opened.read(None))?;
    crate::warn(py, &read.warnings)?;
    Ok(opened)
}
