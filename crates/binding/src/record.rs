//! A run's two files, read: the settings, `run.toml`, which `run_file(folder=None, *, out)` runs;
//! and the record a run writes beside its four tables, `run_record.toml`, which
//! `Run.open(folder, network=None, demand=None, transit=None)` opens without running anything.
//!
//! Each file is found by its fixed name in the folder given; the path of a file of that name is
//! taken as its folder, and `run_file` given no folder reads the current one. Refused, saying what
//! to give instead: a file of any other name; and, given to `run_file`, a folder that holds a run's
//! record and no settings file. Both files are read with Python's standard `tomllib`.
//!
//! A settings file holds the keys of the record's part one (`openmobisim_io_parquet::KEYS`, the
//! tables before `result`): `format` (only 1, when present), `window_start_s` and `window_end_s`
//! (required; the end given, which the run moves up to a whole `link_bin` when it is not one),
//! `[parameters]` (an absent row is the default of `PARAMETERS.md`; each class's coefficients by
//! column name, all sixteen), the input files (`[network] osm_file` and `region`, or `nodes_file`
//! and `links_file`; `[demand] trips_file`; `[transit] gtfs_file`) and `[demand] classes`. A key
//! or table under `[parameters]` that is not one of these is refused by name (a parameter
//! mistyped would run on its default); one elsewhere is ignored, all of them named in one
//! `UserWarning` (a coefficient table's keys are the columns of the choice row); `stop_walk_max`
//! or `transfer_walk_max` without `[transit]` is ignored, warned. A settings file that holds
//! `[result]` is a run's record renamed: one `UserWarning` says to write a settings file instead,
//! and the run reads part one alone, but for the inputs' fingerprints, compared as below.
//!
//! A record is read whole, both parts (a `repairs` table's keys are not checked). Besides part
//! one, `Run.open` reads `[result] window_end_used_s` (required: the end of the window its tables
//! cover), `stopped_by_rule` (absent: the run's end is not recorded, which the pages say),
//! `[result.repairs]` and the inputs' fingerprints. Known and not read: the version, the counts,
//! the readers' reports.
//!
//! Each input file's path is joined to the folder of the file's canonical path (the folder it was
//! written in). Both build their inputs by one function, [`inputs`]: the network, the transit and
//! the demand by the same functions as `Network.from_osm` or `Network.from_tables`,
//! `Transit.from_gtfs` and `Demand.from_table`, each given the rows of `[parameters]` its entry
//! point reads; `run_file` then runs `run` with the rest, written into `out`, and `Run.open` reads
//! the run's tables. An input whose fingerprint differs from the one recorded (in a record, or a
//! record renamed), or that a record has no fingerprint of, raises a `UserWarning` naming it (and
//! both fingerprints), and the work goes on; a settings file holds no fingerprint, and nothing is
//! compared.
//!
//! For bikes: `bike_speed` is read from `[parameters]` and given to the network as at
//! `Network.from_tables`; the links' and trips' bike permissions come with their files.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use openmobisim_io_parquet::KEYS;
use openmobisim_loop::parameters::COLUMNS;
use pyo3::conversion::FromPyObjectOwned;
use pyo3::exceptions::{PyFileNotFoundError, PyOSError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyList};

use crate::keywords::{Entry, entry};
use crate::{Demand, Network, Run, Transit};

/// The settings a user writes, which `run_file` reads.
const SETTINGS: &str = "run.toml";
/// The record a run writes beside its tables, which `Run.open` reads.
pub(crate) const RECORD: &str = "run_record.toml";
/// What to do with a record to run it again.
const AGAIN: &str = "to run it again, write a run.toml (the record's lines before [result] are \
                     one), or copy the record to run.toml";

/// Which of the two files is read.
#[derive(Clone, Copy, PartialEq, Eq)]
enum File {
    Settings,
    Record,
}

impl File {
    /// Its fixed name.
    fn name(self) -> &'static str {
        match self {
            Self::Settings => SETTINGS,
            Self::Record => RECORD,
        }
    }

    /// What it is, in a message.
    fn what(self) -> &'static str {
        match self {
            Self::Settings => "settings file",
            Self::Record => "record",
        }
    }
}

/// A refusal of the file `f`: a `ValueError` beginning with its name.
fn refuse(f: File, why: &str) -> PyErr {
    PyValueError::new_err(format!("{}: {why}", f.name()))
}

/// The file `f` that `path` names: `path` itself when its name is `f`'s, else `f` in the folder
/// `path` (none: the current folder). Refused, saying what to give instead: a path that is
/// neither (a `ValueError` for a file of another name, a `FileNotFoundError` for nothing there);
/// and, for the settings, a folder that holds a run's record and no settings file.
fn find(path: Option<&Path>, f: File) -> PyResult<PathBuf> {
    let path = path.unwrap_or_else(|| Path::new("."));
    let named = |name: &str| path.file_name().is_some_and(|n| n == name);
    let file = if named(f.name()) {
        path.to_path_buf()
    } else if path.is_dir() {
        path.join(f.name())
    } else {
        let what = match f {
            File::Settings if named(RECORD) => format!("it is the record a run wrote; {AGAIN}"),
            File::Settings => "run_file reads a settings file named run.toml, or the folder \
                               holding it: rename the file run.toml, or give its folder"
                .to_owned(),
            File::Record if named(SETTINGS) => "it is a settings file; Run.open opens a finished \
                                                run: the folder holding the record the run \
                                                wrote, run_record.toml, or that file"
                .to_owned(),
            File::Record => "Run.open opens a finished run: the folder holding the record the \
                             run wrote, run_record.toml, or that file"
                .to_owned(),
        };
        let message = format!("{}: {what}", path.display());
        return Err(if path.exists() {
            PyValueError::new_err(message)
        } else {
            PyFileNotFoundError::new_err(message)
        });
    };
    if f == File::Settings && !file.exists() && file.with_file_name(RECORD).exists() {
        let folder = file.parent().unwrap_or(path);
        return Err(PyValueError::new_err(format!(
            "{}: the folder holds a run's record, run_record.toml, and no settings file, \
             run.toml; {AGAIN}",
            folder.display()
        )));
    }
    Ok(file)
}

/// The table at the dotted name `name` of `toml` (`""`: the top), if the file `f` holds it; a
/// value that is not a table is refused.
fn table<'py>(
    f: File,
    toml: &Bound<'py, PyDict>,
    name: &str,
) -> PyResult<Option<Bound<'py, PyDict>>> {
    let mut at = toml.clone();
    for part in name.split('.').filter(|p| !p.is_empty()) {
        let Some(next) = at.get_item(part)? else { return Ok(None) };
        at =
            next.cast_into::<PyDict>().map_err(|_| refuse(f, &format!("{name} is not a table")))?;
    }
    Ok(Some(at))
}

/// The keys and tables of `toml` that the file `f` does not hold (a settings file: part one's
/// keys and `result`; a record: both parts'): under `[parameters]` refused by name (in each
/// class's coefficient table, a column that is not one of the choice row's); elsewhere ignored,
/// all named in one `UserWarning`. A settings file holding `[result]`, a record renamed, is told
/// so in one `UserWarning`, and its part two is not checked. A `repairs` table's keys are kinds of
/// repair, not checked.
fn known(f: File, toml: &Bound<'_, PyDict>) -> PyResult<()> {
    let renamed = f == File::Settings && toml.contains("result")?;
    if renamed {
        let message = "run.toml: it holds [result]: it is a run's record renamed, not a settings \
                       file; the run reads it down to [result], the settings that run ran with, \
                       and compares the inputs' fingerprints. Write a settings file instead (the \
                       record's lines before [result] are one)";
        crate::warn(toml.py(), &[message.to_owned()])?;
    }
    let mut ignored = Vec::new();
    for (name, keys) in KEYS {
        if f == File::Settings && name.starts_with("result") {
            continue;
        }
        let Some(t) = table(f, toml, name)? else { continue };
        for key in t.keys() {
            let key: String = key.extract()?;
            if keys.contains(&key.as_str()) {
                continue;
            }
            let at = if name.is_empty() { String::new() } else { format!(" [{name}]") };
            if name == "parameters" {
                return Err(refuse(f, &format!("{key} is not a key of the {}{at}", f.what())));
            }
            ignored.push(format!("{key}{at}"));
        }
    }
    if !ignored.is_empty() {
        let message = format!(
            "{}: {} keys a {} never holds, ignored: {}",
            f.name(),
            ignored.len(),
            f.what(),
            ignored.join(", ")
        );
        crate::warn(toml.py(), &[message])?;
    }
    let Some(classes) = table(f, toml, "parameters.coefficients")? else { return Ok(()) };
    for (class, row) in classes.iter() {
        let class: String = class.extract()?;
        let row = row
            .cast_into::<PyDict>()
            .map_err(|_| refuse(f, &format!("parameters.coefficients.{class:?} is not a table")))?;
        for column in row.keys() {
            let column: String = column.extract()?;
            if !COLUMNS.contains(&column.as_str()) {
                return Err(refuse(
                    f,
                    &format!(
                        "{column} is not a column of the choice row \
                         [parameters.coefficients.{class:?}]"
                    ),
                ));
            }
        }
    }
    Ok(())
}

/// Refuse a TOML boolean for the key `dotted`: no key read from either file is a boolean (but
/// `stopped_by_rule`, read apart), and Python would read `true` as the number 1.
fn not_boolean(f: File, v: &Bound<'_, PyAny>, dotted: &str) -> PyResult<()> {
    if v.is_instance_of::<PyBool>() {
        return Err(refuse(f, &format!("{dotted}: a boolean ({v}) is not a value of this key")));
    }
    Ok(())
}

/// The value of `key` in the table `name` read as a `T`, if present; a value that is not a `T`,
/// or a boolean, is refused naming the key.
fn value<'py, T: FromPyObjectOwned<'py>>(
    f: File,
    t: &Bound<'py, PyDict>,
    name: &str,
    key: &str,
) -> PyResult<Option<T>> {
    let Some(v) = t.get_item(key)? else { return Ok(None) };
    let dotted = if name.is_empty() { key.to_string() } else { format!("{name}.{key}") };
    not_boolean(f, &v, &dotted)?;
    v.extract::<T>().map(Some).map_err(|e| {
        let e: PyErr = e.into();
        refuse(f, &format!("{dotted}: {e}"))
    })
}

/// The value of the required `key` in the table `name`; its absence is refused naming it.
fn required<'py, T: FromPyObjectOwned<'py>>(
    f: File,
    t: &Bound<'py, PyDict>,
    name: &str,
    key: &str,
) -> PyResult<T> {
    let dotted = if name.is_empty() { key.to_string() } else { format!("{name}.{key}") };
    value(f, t, name, key)?
        .ok_or_else(|| refuse(f, &format!("{dotted} is missing; it is required")))
}

/// The keywords of each entry point from `[parameters]`: the rows each reads, `coefficients` as
/// the dict from class to its sixteen in the order of the choice row. A boolean is refused;
/// `stop_walk_max` and `transfer_walk_max` without transit are ignored, warned (a record holds
/// them only with transit, and nothing reads them without). Each other value is checked where
/// its entry point reads it.
fn keywords<'py>(
    f: File,
    py: Python<'py>,
    toml: &Bound<'py, PyDict>,
    with_transit: bool,
) -> PyResult<[Bound<'py, PyDict>; 4]> {
    let [network, transit, demand, run] = [(); 4].map(|()| PyDict::new(py));
    let Some(p) = table(f, toml, "parameters")? else {
        return Ok([network, transit, demand, run]);
    };
    for (name, v) in p.iter() {
        let name: String = name.extract()?;
        if name == "coefficients" {
            run.set_item("coefficients", coefficients(f, py, toml)?)?;
            continue;
        }
        not_boolean(f, &v, &format!("parameters.{name}"))?;
        let at = match entry(&name) {
            Some(Entry::Network) => &network,
            Some(Entry::Transit | Entry::Demand) if !with_transit => {
                let message = format!(
                    "{}: parameters.{name} is ignored: it is read only with a transit, \
                     [transit] gtfs_file, and the {} has none",
                    f.name(),
                    f.what()
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
fn coefficients<'py>(
    f: File,
    py: Python<'py>,
    toml: &Bound<'py, PyDict>,
) -> PyResult<Bound<'py, PyDict>> {
    let rows = PyDict::new(py);
    let Some(classes) = table(f, toml, "parameters.coefficients")? else { return Ok(rows) };
    for (class, row) in classes.iter() {
        let class: String = class.extract()?;
        let name = format!("parameters.coefficients.{class:?}");
        let row =
            row.cast_into::<PyDict>().map_err(|_| refuse(f, &format!("{name} is not a table")))?;
        let values: Vec<f64> =
            COLUMNS.iter().map(|c| required(f, &row, &name, c)).collect::<PyResult<_>>()?;
        rows.set_item(class, PyList::new(py, values)?)?;
    }
    Ok(rows)
}

/// The file `f` at `path` read with `tomllib`, every key known ([`known`]) and `format`, if
/// given, 1; and the folder its canonical path is in, to which its file paths are joined.
fn load<'py>(py: Python<'py>, f: File, path: &Path) -> PyResult<(Bound<'py, PyDict>, PathBuf)> {
    let os_error = |e: std::io::Error| PyOSError::new_err(format!("{}: {e}", path.display()));
    let canonical = std::fs::canonicalize(path).map_err(os_error)?;
    let text = std::fs::read_to_string(&canonical).map_err(os_error)?;
    let toml = py.import("tomllib")?.call_method1("loads", (text,))?.cast_into::<PyDict>()?;
    known(f, &toml)?;
    match value::<i64>(f, &toml, "", "format")? {
        Some(format) if format != 1 => {
            Err(refuse(f, &format!("format {format} is not read; the format read is 1")))
        }
        _ => Ok((toml, canonical.parent().map(Path::to_path_buf).unwrap_or_default())),
    }
}

/// Where the network is read from.
enum NetworkFiles {
    /// The extract, and the region it was cut to (`[network] region`, as `tomllib` reads it).
    Osm(PathBuf, Option<Py<PyAny>>),
    Tables(PathBuf, PathBuf),
}

/// `[network]`'s files joined to `folder`: `osm_file`, or `nodes_file` and `links_file`.
fn network_files(f: File, net: &Bound<'_, PyDict>, folder: &Path) -> PyResult<NetworkFiles> {
    let file = |key: &str| -> PyResult<Option<PathBuf>> {
        Ok(value::<String>(f, net, "network", key)?.map(|p| folder.join(p)))
    };
    match (file("osm_file")?, file("nodes_file")?, file("links_file")?) {
        (Some(osm), None, None) => {
            Ok(NetworkFiles::Osm(osm, net.get_item("region")?.map(Bound::unbind)))
        }
        (Some(_), _, _) => Err(refuse(
            f,
            "network.osm_file and network.nodes_file or links_file are two networks; a run reads \
             one",
        )),
        (None, nodes, links) => Ok(NetworkFiles::Tables(
            nodes.ok_or_else(|| refuse(f, "network.nodes_file is missing (or osm_file)"))?,
            links.ok_or_else(|| refuse(f, "network.links_file is missing (or osm_file)"))?,
        )),
    }
}

/// What an input the file names no file of means: for `run_file`, a key the settings lack,
/// refused naming it; for `Run.open`, an input the caller must pass, refused naming the keyword
/// when it is not passed.
#[derive(Clone, Copy, PartialEq, Eq)]
enum NoFile {
    MissingKey,
    PassIt,
}

/// The run's inputs as its file names them (rebuilt from its files, or passed), and what else
/// `run` reads from it.
struct Rebuilt<'py> {
    network: Bound<'py, Network>,
    demand: Bound<'py, Demand>,
    transit: Option<Bound<'py, Transit>>,
    run_keywords: Bound<'py, PyDict>,
    /// The window given, `window_start_s` and `window_end_s`.
    window: (u32, u32),
}

/// The inputs passed to `Run.open` (none to `run_file`): network, demand, transit.
pub(crate) type Passed<'a, 'py> = (
    Option<&'a Bound<'py, Network>>,
    Option<&'a Bound<'py, Demand>>,
    Option<&'a Bound<'py, Transit>>,
);

/// An input recorded in the file `f` compared with the one built or passed: a `UserWarning`
/// naming it and both fingerprints when they differ, or saying it cannot be compared when the
/// record has no fingerprint of it; the work goes on in both.
fn compare(
    py: Python<'_>,
    f: File,
    input: &str,
    recorded: Option<String>,
    built: u64,
) -> PyResult<()> {
    let built = format!("{built:016x}");
    let message = match recorded {
        Some(recorded) if recorded == built => return Ok(()),
        Some(recorded) => format!(
            "{}: the {input} changed since the record was written: its fingerprint is \
             {built}, the record's {recorded}; the work goes on with the {input} as it is",
            f.name()
        ),
        None => format!(
            "{}: the record has no fingerprint of the {input}: the {input} given or \
             rebuilt ({built}) cannot be compared with the run's; the work goes on with it",
            f.name()
        ),
    };
    crate::warn(py, &[message])
}

/// The inputs' fingerprints `toml` records in part two (`[result.network]`, `[result.demand]`,
/// `[result.transit]`, this one only `with_transit`), each `None` where it has none; `None` for
/// a file without `[result]`, a settings file, of which nothing is compared.
fn recorded(
    f: File,
    toml: &Bound<'_, PyDict>,
    with_transit: bool,
) -> PyResult<Option<[Option<String>; 3]>> {
    if !toml.contains("result")? {
        return Ok(None);
    }
    let print = |input: &str| -> PyResult<Option<String>> {
        let name = format!("result.{input}");
        match table(f, toml, &name)? {
            Some(t) => value::<String>(f, &t, &name, "fingerprint"),
            None => Ok(None),
        }
    };
    let transit = if with_transit { print("transit")? } else { None };
    Ok(Some([print("network")?, print("demand")?, transit]))
}

/// The inputs the file `f`, read as `toml` (its files joined to `folder`), names: each one
/// `passed` taken as it is, each other built from its file exactly as `Network.from_osm` or
/// `Network.from_tables`, `Transit.from_gtfs` and `Demand.from_table` build it, with the rows of
/// `[parameters]` its entry point reads. An input the file has no file for is met as `no_file`
/// says; a transit passed to a run that had none is set aside with a warning; passed inputs must
/// be built on one another (a demand on the network and transit used, a transit on the network).
/// Then each fingerprint is compared with the one recorded, if any ([`recorded`], [`compare`]).
fn inputs<'py>(
    py: Python<'py>,
    f: File,
    toml: &Bound<'py, PyDict>,
    folder: &Path,
    passed: Passed<'_, 'py>,
    no_file: NoFile,
) -> PyResult<Rebuilt<'py>> {
    let window: (u32, u32) =
        (required(f, toml, "", "window_start_s")?, required(f, toml, "", "window_end_s")?);
    let missing = |name: &str| refuse(f, &format!("[{name}] is missing; it is required"));
    let net = table(f, toml, "network")?.ok_or_else(|| missing("network"))?;
    let dem = table(f, toml, "demand")?.ok_or_else(|| missing("demand"))?;
    let tra = table(f, toml, "transit")?;
    let [network_kw, transit_kw, demand_kw, run_keywords] = keywords(f, py, toml, tra.is_some())?;
    let pass_it = |input: &str| {
        refuse(
            f,
            &format!(
                "the {input} was given in memory when the run was made (the record names no \
                 file of it); pass it: {input}="
            ),
        )
    };
    let unfiled =
        ["osm_file", "nodes_file", "links_file"].iter().all(|k| !net.contains(k).unwrap_or(false));
    if passed.0.is_none() && unfiled && no_file == NoFile::PassIt {
        return Err(pass_it("network"));
    }
    let network_files =
        passed.0.map_or_else(|| network_files(f, &net, folder).map(Some), |_| Ok(None))?;
    let passed_transit = match (&tra, passed.2) {
        (None, Some(_)) => {
            crate::warn(py, &["the transit passed is set aside: the run had none".to_owned()])?;
            None
        }
        (_, t) => t,
    };
    let feed = match (&tra, passed_transit) {
        (Some(t), None) => Some(required::<String>(f, t, "transit", "gtfs_file")?),
        _ => None,
    };
    let trips = match passed.1 {
        Some(_) => None,
        None if no_file == NoFile::PassIt && !dem.contains("trips_file")? => {
            return Err(pass_it("demand"));
        }
        None => Some(folder.join(required::<String>(f, &dem, "demand", "trips_file")?)),
    };
    let classes: Vec<String> = required(f, &dem, "demand", "classes")?;
    let recorded = recorded(f, toml, tra.is_some())?;

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
    agree(py, f, (&network, &demand, transit.as_ref()), recorded)?;
    Ok(Rebuilt { network, demand, transit, run_keywords, window })
}

/// The network read from its files, with the keywords of `[parameters]` it reads.
fn build_network(
    py: Python<'_>,
    files: NetworkFiles,
    keywords: &Bound<'_, PyDict>,
) -> PyResult<Network> {
    match files {
        NetworkFiles::Osm(osm, region) => {
            Network::from_osm(py, osm, region.as_ref().map(|r| r.bind(py)), Some(keywords))
        }
        NetworkFiles::Tables(nodes, links) => {
            let (nodes, links) = (nodes.into_pyobject(py)?, links.into_pyobject(py)?);
            Network::from_tables(py, &nodes, &links, Some(keywords))
        }
    }
}

/// Refuse a demand not built on the network and transit used (`built_on`); then compare each
/// fingerprint with the one recorded (`recorded`: network, demand, transit), if any.
fn agree(
    py: Python<'_>,
    f: File,
    (network, demand, transit): crate::RunInputs<'_, '_>,
    recorded: Option<[Option<String>; 3]>,
) -> PyResult<()> {
    let d = demand.get();
    crate::built_on(network.get(), d, transit.map(Bound::get))?;
    let Some([network_print, demand_print, transit_print]) = recorded else { return Ok(()) };
    compare(py, f, "network", network_print, network.get().network.fingerprint())?;
    compare(py, f, "demand", demand_print, d.demand.fingerprint())?;
    if let Some(t) = transit {
        compare(py, f, "transit", transit_print, t.get().timetable.fingerprint())?;
    }
    Ok(())
}

/// Run the settings file `run.toml` in `folder` (none: the current folder; the path of a file of
/// that name: its folder), writing the run into `out`, and return its `Run`. The inputs are the
/// files it names, joined to its folder ([`inputs`]); the parameters are its `[parameters]`, a
/// row absent there taking the default of `PARAMETERS.md`; the window is its `window_start_s`
/// and `window_end_s`. The keys, the window, the files, the classes and the booleans are checked
/// before an input is built; each parameter's value is checked by the entry point that reads
/// it, so a value the run refuses is refused after the inputs before it are built.
#[pyfunction]
#[pyo3(signature = (folder=None, *, out))]
#[allow(clippy::needless_pass_by_value, reason = "pyo3 hands the arguments over by value")]
pub(crate) fn run_file(py: Python<'_>, folder: Option<PathBuf>, out: PathBuf) -> PyResult<Run> {
    let f = File::Settings;
    let file = find(folder.as_deref(), f)?;
    let (toml, folder) = load(py, f, &file)?;
    let i = inputs(py, f, &toml, &folder, (None, None, None), NoFile::MissingKey)?;
    let (network, demand, transit) = (&i.network, &i.demand, i.transit.as_ref());
    crate::run(py, network, demand, transit, i.window, out, Some(&i.run_keywords))
}

/// Open the finished run in `folder` (the path of its `run_record.toml`: that file's folder), its
/// four tables beside its record, without running anything. The inputs are `passed`, or rebuilt
/// from the files the record names ([`inputs`]; one the record names no file of must be passed);
/// an input whose fingerprint differs from the record's, or that it has no fingerprint of, is
/// shown in a warning and the opening goes on. The window (`window_start_s` and `[result]
/// window_end_used_s`), `link_bin` (absent: its default), `[result] stopped_by_rule` (a boolean;
/// absent, the run's end is not recorded, which the pages say) and `[result.repairs]` (each an
/// integer) come from the record. The three tables the pages read and the iterations are read
/// once here: a table that cannot be read is refused naming it, and what the readers set aside
/// is shown in warnings.
pub(crate) fn open(py: Python<'_>, folder: &Path, passed: Passed<'_, '_>) -> PyResult<Run> {
    let f = File::Record;
    let record = find(Some(folder), f)?;
    let (toml, folder) = load(py, f, &record)?;
    let i = inputs(py, f, &toml, &folder, passed, NoFile::PassIt)?;
    let bin = match i.run_keywords.get_item("link_bin")? {
        Some(b) => {
            b.extract::<u32>().map_err(|e| refuse(f, &format!("parameters.link_bin: {e}")))?
        }
        None => openmobisim_loop::parameters::Parameters::defaults(0).link_bin,
    };
    let result = table(f, &toml, "result")?.ok_or_else(|| {
        refuse(f, "[result] is missing; it is required (the end of the window the run used)")
    })?;
    let end: u32 = required(f, &result, "result", "window_end_used_s")?;
    let stopped_by_rule = result
        .get_item("stopped_by_rule")?
        .map(|v| {
            let flag = v.cast_into::<PyBool>().map_err(|e| {
                refuse(f, &format!("result.stopped_by_rule is {}, not a boolean", e.into_inner()))
            })?;
            Ok::<_, PyErr>(flag.is_true())
        })
        .transpose()?;
    let mut repairs = Vec::new();
    if let Some(t) = table(f, &toml, "result.repairs")? {
        for key in t.keys() {
            let key: String = key.extract()?;
            repairs.push((key.clone(), required::<u64>(f, &t, "result.repairs", &key)?));
        }
    }
    let window = crate::Window { start: i.window.0, end, bin };
    let inputs = (&i.network, &i.demand, i.transit.as_ref());
    let ended = crate::Ended { stopped_by_rule, repairs };
    let opened = Run::of(&record, window, ended, inputs)?;
    let read = py.detach(|| opened.read(None))?;
    crate::warn(py, &read.warnings)?;
    Ok(opened)
}
