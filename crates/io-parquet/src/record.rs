//! `run_record.toml`: the record of a run, written by the run beside its four tables, so that the
//! folder alone tells which settings and inputs produced them and what the run found and did.
//! TOML (version 1.0) written by hand: keys in a fixed order, no time stamp, machine or thread
//! count, and an input file's path relative to the record's folder, so a second write of the same
//! run is the same bytes. The record has two parts. Part one, every line before `[result]`, is
//! the settings the run ran with, every default filled in: it is a settings file, `run.toml`,
//! as the binding's `run_file` reads one. Part two, `[result]` and the tables under it, is what
//! the run found and did: the version, the end of the window it used, how it ended, the inputs'
//! counts, fingerprints and readers' reports, and every repair.
//!
//! ```toml
//! # The record of one openmobisim run, written by the run: ...
//! format = 1
//! window_start_s = 25200
//! window_end_s = 28800         the end given
//!
//! [parameters]
//! capacity_per_lane = 1800.0  # veh/h per lane
//! ...                          (each row of PARAMETERS.md the run read, its unit as a comment)
//!
//! [parameters.coefficients."commute"]
//! car_drive_min = 1.0
//! ...                          (one table per class, a key per column of the choice row)
//!
//! [network]                    osm_file, region (the region the extract was cut to, absent
//!                              for the whole extract) and car_roads = "main" (absent for
//!                              cars on every car road), or nodes_file and links_file; each
//!                              absent for a table given in memory, so the table may be empty
//! [demand]                     trips_file (absent likewise), classes
//! [transit]                    gtfs_file; absent without transit
//!
//! # Part two: what the run found and did.
//! [result]                     openmobisim_version, window_end_used_s, iterations, stopped_by_rule
//! [result.repairs]             what the run set right or dropped (the window), by kind
//! [result.network]             source, nodes, links, fingerprint
//! [result.network.osm_report]  the OSM reader's counts; absent for a network from tables
//! [result.network.repairs]     what the network's readers set right, by kind
//! [result.demand]              trips, trips_given (only when it differs from trips), fingerprint
//! [result.demand.repairs]      what the demand's readers set right or dropped, by kind
//! [result.transit]             stops, runs, fingerprint; absent without transit
//! [result.transit.gtfs_report] the GTFS reader's counts
//! [result.transit.repairs]     what the GTFS reader set right, by kind
//! ```
//!
//! `window_end_s` is the end given; `window_end_used_s`, always written, is the end the run used:
//! the end given, or, when the window was not a whole number of `link_bin`, the end moved up to
//! the next whole bin (counted as `window_end_moved` in `[result.repairs]`). A `repairs` table
//! holds one key per kind of repair made, its count (each crate's README names its kinds), and is
//! absent when none was made. `trips_given` counts the rows of the trips table and is written
//! only when the demand dropped some, so that it differs from `trips`, the trips the demand holds
//! (the difference is in `[result.demand.repairs]`); of those, the trips the run did not run are
//! counted in `[result.repairs]`.
//!
//! TOML has no null: a value that does not exist is an absent key or table. `stop_walk_max` and
//! `transfer_walk_max` are absent without transit (nothing reads them there); the coefficients'
//! comment and tables are absent without a class; `osm_class_defaults` is never written (its one value is the OSM
//! reader's one table, `crates/io-osm/README.md`). A float is written by Rust's shortest
//! round-trip form with a point or an exponent (`1800.0`, `0.1`, `1e-7`), so TOML reads it as
//! a float of the same bits; an integer as an integer; a string and a quoted key as a basic
//! string with the quote, the backslash and every control character escaped. A float that is not
//! finite (TOML's `nan` and `inf` are never written), or an integer above TOML's largest
//! (2⁶³ − 1), is refused with an error naming its key, before anything is written. An input
//! file's path is written relative to the folder the record is in, from the canonical paths of
//! both (absolute, symbolic links resolved), with `/` between components (`../inputs/trips.parquet`);
//! absolute only when no relative path exists (another drive); a path that is not UTF-8, which
//! a TOML string cannot hold, is refused naming its key.
//!
//! For bikes: `bike_speed`, the one speed of every bike, is a key of `[parameters]` with its
//! unit; a link's bike permission and a trip's bike are in the network's and the demand's
//! fingerprints, so a change to either changes the record.

use std::path::{Component, Path, Prefix};

use openmobisim_demand::demand::Demand;
use openmobisim_graph::network::Network;
use openmobisim_io_osm::{CarRoads, Region, Report};
use openmobisim_loop::parameters::{COLUMNS, ChoiceModel};
use openmobisim_loop::run::Run;
use openmobisim_types::repairs::Repairs;
use openmobisim_types::time::Second;

use crate::{Files, Given, Source, WriteError};

/// The file's first line.
const HEAD: &str = "# The record of one openmobisim run, written by the run: part one, every line \
                    before [result], is the settings it ran with, every default filled in (a \
                    settings file, run.toml, as it stands); part two, [result], is what it found \
                    and did. It describes trips.parquet, links.parquet, iterations.parquet and \
                    routes.parquet in this folder.";

/// The comment above part two.
const PART_TWO: &str = "# Part two: what the run found and did.";

/// The comment of `window_end_used_s`.
const END_USED: &str = "s: window_end_s, moved up to a whole link_bin when it was not one";

/// The rows of `PARAMETERS.md` written under `[parameters]`, in order, each with its unit as
/// that table's Unit column writes it (`coefficients` is written as tables after them;
/// `osm_class_defaults` is not written).
const UNITS: [(&str, &str); 17] = [
    ("capacity_per_lane", "veh/h per lane"),
    ("bike_speed", "km/h"),
    ("walk_speed", "km/h"),
    ("stop_walk_max", "m"),
    ("transfer_walk_max", "m"),
    ("boarding_slack", "s"),
    ("max_rides", "rides"),
    ("nearby_stops", "searches per trip end"),
    ("max_car_routes", "routes per origin-destination pair"),
    ("max_transit_journeys", "journeys per trip"),
    ("choice", "name"),
    ("nest_scale", "none"),
    ("step_exponent", "none"),
    ("iteration_cap", "iterations"),
    ("iteration_floor", "iterations"),
    ("link_bin", "s"),
    ("seed", "none"),
];

/// Every table of the record, by its dotted name (`""` for the top), and the keys the writer may
/// write in it, so that a reader can name any other: part one's tables, then part two's,
/// `result` and those under it. The tables of the coefficients, one per class under
/// `parameters.coefficients`, hold the columns of the choice row
/// (`openmobisim_loop::parameters::COLUMNS`); a `repairs` table, the kinds of repair its
/// readers name.
pub const KEYS: [(&str, &[&str]); 11] = [
    (
        "",
        &[
            "format",
            "window_start_s",
            "window_end_s",
            "parameters",
            "network",
            "demand",
            "transit",
            "result",
        ],
    ),
    (
        "parameters",
        &[
            "capacity_per_lane",
            "bike_speed",
            "walk_speed",
            "stop_walk_max",
            "transfer_walk_max",
            "boarding_slack",
            "max_rides",
            "nearby_stops",
            "max_car_routes",
            "max_transit_journeys",
            "choice",
            "nest_scale",
            "step_exponent",
            "iteration_cap",
            "iteration_floor",
            "link_bin",
            "seed",
            "coefficients",
        ],
    ),
    ("network", &["osm_file", "region", "car_roads", "nodes_file", "links_file"]),
    ("demand", &["trips_file", "classes"]),
    ("transit", &["gtfs_file"]),
    (
        "result",
        &[
            "openmobisim_version",
            "window_end_used_s",
            "iterations",
            "stopped_by_rule",
            "repairs",
            "network",
            "demand",
            "transit",
        ],
    ),
    ("result.network", &["source", "nodes", "links", "fingerprint", "osm_report", "repairs"]),
    (
        "result.network.osm_report",
        &["nodes", "links", "links_unreachable", "loops_dropped", "ways_cut"],
    ),
    ("result.demand", &["trips", "trips_given", "fingerprint", "repairs"]),
    ("result.transit", &["stops", "runs", "fingerprint", "gtfs_report", "repairs"]),
    (
        "result.transit.gtfs_report",
        &[
            "date",
            "trips",
            "runs",
            "calls",
            "night_runs",
            "frequency_runs",
            "stops",
            "trips_untimed",
            "trips_single_call",
            "times_interpolated",
            "pickup_drop_off_ignored",
            "frequency_rows_skipped",
            "footpaths",
            "transfers_applied",
            "transfers_skipped",
        ],
    ),
];

/// The comment above the coefficient tables.
const COEFFICIENTS: &str = "# coefficients: per class, one per column of the choice row: cost per \
                            minute of each time, per transfer, per euro; on ln PS, minus the \
                            exponent of the path size (read times nest_scale under the logit, as \
                            0 under the least cost); and each mode's constant";

/// One value of the record.
enum Value<'a> {
    Int(u64),
    Float(f64),
    Str(&'a str),
    Flag(bool),
    Strs(Vec<&'a str>),
    /// A region: `[west, south, east, north]` for a box, `[[lon, lat], …]` for a polygon.
    Region(&'a Region),
}

/// The text being written, and the table it is in (for the name of a refused key).
struct Toml {
    text: String,
    table: String,
}

impl Toml {
    /// The text from its first line `head` to the window's keys.
    fn start(head: &str, (start, end): (Second, Second)) -> Result<Self, WriteError> {
        let mut t = Self { text: format!("{head}\n"), table: String::new() };
        t.keys(&[
            ("format", Value::Int(1)),
            ("window_start_s", Value::Int(u64::from(start.get()))),
            ("window_end_s", Value::Int(u64::from(end.get()))),
        ])?;
        Ok(t)
    }

    /// Start a table: a blank line, then its header.
    fn table(&mut self, header: &str) {
        self.text.push_str(&format!("\n[{header}]\n"));
        self.table = header.to_string();
    }

    /// Write `key = value`, then `  # comment` when there is one.
    fn key(
        &mut self,
        key: &str,
        value: &Value<'_>,
        comment: Option<&str>,
    ) -> Result<(), WriteError> {
        let refuse = |value: String| {
            let key = if self.table.is_empty() {
                key.to_string()
            } else {
                format!("{}.{key}", self.table)
            };
            WriteError::NotWritable { key, value }
        };
        let text = match value {
            Value::Int(n) if i64::try_from(*n).is_err() => return Err(refuse(n.to_string())),
            Value::Int(n) => n.to_string(),
            Value::Float(x) if !x.is_finite() => return Err(refuse(x.to_string())),
            Value::Float(x) => format!("{x:?}"),
            Value::Str(s) => string(s),
            Value::Flag(b) => b.to_string(),
            Value::Strs(v) => {
                format!("[{}]", v.iter().map(|s| string(s)).collect::<Vec<_>>().join(", "))
            }
            Value::Region(Region::BBox { west, south, east, north }) => {
                format!("[{west:?}, {south:?}, {east:?}, {north:?}]")
            }
            Value::Region(Region::Polygon(points)) => {
                let points: Vec<String> =
                    points.iter().map(|(lon, lat)| format!("[{lon:?}, {lat:?}]")).collect();
                format!("[{}]", points.join(", "))
            }
        };
        self.text.push_str(&format!("{key} = {text}"));
        if let Some(c) = comment {
            self.text.push_str(&format!("  # {c}"));
        }
        self.text.push('\n');
        Ok(())
    }

    /// Write every `(key, value)` of `entries`, no comments.
    fn keys(&mut self, entries: &[(&str, Value<'_>)]) -> Result<(), WriteError> {
        entries.iter().try_for_each(|(k, v)| self.key(k, v, None))
    }
}

/// `s` as a TOML basic string: in quotes, the quote and the backslash escaped by a backslash,
/// every control character (U+0000 to U+001F and U+007F) as `\uXXXX`.
fn string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c < ' ' || c == '\u{7f}' => out.push_str(&format!("\\u{:04X}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A fingerprint as 16 lower-case hexadecimal digits.
fn hex(fingerprint: u64) -> String {
    format!("{fingerprint:016x}")
}

/// The value of the parameter `name` (a row of [`UNITS`]) the run used; `None` for
/// `stop_walk_max` and `transfer_walk_max` without transit, where nothing reads them.
fn parameter<'a>(name: &str, given: &Given<'a>) -> Option<Value<'a>> {
    let p = given.parameters;
    Some(match name {
        "capacity_per_lane" => Value::Float(given.build_values.capacity_per_lane),
        "bike_speed" => Value::Float(given.build_values.bike_speed),
        "walk_speed" => Value::Float(given.build_values.walk_speed),
        "stop_walk_max" => Value::Float(given.transit.and(Some(given.stop_walk_max))?),
        "transfer_walk_max" => Value::Float(given.transit.as_ref()?.transfer_walk_max),
        "boarding_slack" => Value::Int(u64::from(p.boarding_slack)),
        "max_rides" => Value::Int(u64::from(p.max_rides)),
        "nearby_stops" => Value::Int(u64::from(p.nearby_stops)),
        "max_car_routes" => Value::Int(u64::from(p.max_car_routes)),
        "max_transit_journeys" => Value::Int(u64::from(p.max_transit_journeys)),
        "choice" => Value::Str(match p.choice {
            ChoiceModel::Logit => "logit",
            ChoiceModel::Deterministic => "deterministic",
        }),
        "nest_scale" => Value::Float(p.nest_scale),
        "step_exponent" => Value::Float(p.step_exponent),
        "iteration_cap" => Value::Int(u64::from(p.iteration_cap)),
        "iteration_floor" => Value::Int(u64::from(p.iteration_floor)),
        "link_bin" => Value::Int(u64::from(p.link_bin)),
        "seed" => Value::Int(p.seed),
        other => unreachable!("{other} is not a row of UNITS"),
    })
}

/// The record of `run` as TOML text, each input file's path relative to `folder`, the canonical
/// path of the folder it is written in. Without a folder (the build that checks every value
/// before the folder exists, its text discarded) each path is written whole. Part one, the
/// settings as used, then part two, `[result]`.
pub(crate) fn text(
    run: &Run,
    network: &Network,
    demand: &Demand,
    given: &Given<'_>,
    folder: Option<&Path>,
) -> Result<String, WriteError> {
    let mut t = Toml::start(HEAD, run.window_given)?;
    t.table("parameters");
    for (name, unit) in UNITS {
        if let Some(value) = parameter(name, given) {
            t.key(name, &value, Some(unit))?;
        }
    }
    if demand.class_count() > 0 {
        t.text.push_str(&format!("\n{COEFFICIENTS}\n"));
    }
    for (class, row) in demand.class_ids().iter().zip(&given.parameters.coefficients) {
        t.table(&format!("parameters.coefficients.{}", string(class)));
        for (column, &b) in COLUMNS.iter().zip(row) {
            t.key(column, &Value::Float(b), None)?;
        }
    }
    let classes = demand.class_ids().iter().collect();
    settings_inputs(&mut t, &given.source, &given.files, classes, folder)?;
    t.text.push_str(&format!("\n{PART_TWO}"));
    t.table("result");
    t.key("openmobisim_version", &Value::Str(env!("CARGO_PKG_VERSION")), None)?;
    let used = Value::Int(u64::from(run.links.window().end().get()));
    t.key("window_end_used_s", &used, Some(END_USED))?;
    t.keys(&[
        ("iterations", Value::Int(run.iterations.len() as u64)),
        ("stopped_by_rule", Value::Flag(run.stopped_by_rule)),
    ])?;
    repairs(&mut t, "result", &run.repairs)?;
    found(&mut t, network, demand, given)?;
    Ok(t.text)
}

/// The table `[<name>.repairs]`: each kind of `repairs` with its count, in the order first met;
/// nothing when no repair was made.
fn repairs(t: &mut Toml, name: &str, repairs: &Repairs) -> Result<(), WriteError> {
    if repairs.is_empty() {
        return Ok(());
    }
    t.table(&format!("{name}.repairs"));
    repairs.kinds().iter().try_for_each(|k| t.key(k.key, &Value::Int(k.count), None))
}

/// `file` relative to `folder`, both canonical paths, with `/` between components: a `..` for
/// each component of the folder past their common start, then the rest of the file's. The file's
/// whole path when they have no common start (another drive) or there is no folder; on Windows a
/// canonical path on a drive with a letter, `\\?\D:\…`, is written without its verbatim prefix
/// `\\?\` (`D:/…`, the same file). `None` when a component is not UTF-8.
fn relative(file: &Path, folder: Option<&Path>) -> Option<String> {
    let f: Vec<Component<'_>> = file.components().collect();
    let d: Vec<Component<'_>> = folder.map_or_else(Vec::new, |d| d.components().collect());
    let common = f.iter().zip(&d).take_while(|(a, b)| a == b).count();
    if common == 0 {
        let whole = file.to_str()?;
        let whole = match f.first() {
            Some(Component::Prefix(p)) if matches!(p.kind(), Prefix::VerbatimDisk(_)) => {
                whole.strip_prefix(r"\\?\").unwrap_or(whole)
            }
            _ => whole,
        };
        return Some(whole.replace(std::path::MAIN_SEPARATOR, "/"));
    }
    let mut parts = vec![".."; d.len() - common];
    for c in &f[common..] {
        parts.push(c.as_os_str().to_str()?);
    }
    Some(parts.join("/"))
}

/// Write each `(key, file)` whose file is given, its path relative to `folder` ([`relative`]).
fn files(
    t: &mut Toml,
    files: &[(&str, Option<&Path>)],
    folder: Option<&Path>,
) -> Result<(), WriteError> {
    for &(key, file) in files {
        let Some(file) = file else { continue };
        let text = relative(file, folder).ok_or_else(|| WriteError::NotWritable {
            key: format!("{}.{key}", t.table),
            value: file.display().to_string(),
        })?;
        t.key(key, &Value::Str(&text), None)?;
    }
    Ok(())
}

/// Part one's tables `[network]`, `[demand]` and `[transit]` (this one only with transit): the
/// input files' paths, the region the extract was cut to, and the classes.
fn settings_inputs(
    t: &mut Toml,
    source: &Source<'_>,
    f: &Files<'_>,
    classes: Vec<&str>,
    folder: Option<&Path>,
) -> Result<(), WriteError> {
    t.table("network");
    files(t, &[("osm_file", f.osm)], folder)?;
    if let Source::Osm(Report { region: Some(region), .. }) = source {
        t.key("region", &Value::Region(region), None)?;
    }
    if let Source::Osm(Report { car_roads: CarRoads::Main, .. }) = source {
        t.key("car_roads", &Value::Str("main"), None)?;
    }
    files(t, &[("nodes_file", f.nodes), ("links_file", f.links)], folder)?;
    t.table("demand");
    files(t, &[("trips_file", f.trips)], folder)?;
    t.key("classes", &Value::Strs(classes), None)?;
    if f.gtfs.is_some() {
        t.table("transit");
        files(t, &[("gtfs_file", f.gtfs)], folder)?;
    }
    Ok(())
}

/// The settings file of a case made from an extract: the window, then the tables `[network]`,
/// `[demand]` and `[transit]` as a record writes them (every parameter left to its default),
/// each file's path relative to `folder`.
pub(crate) fn case_text(
    window: (Second, Second),
    report: &Report,
    files: &Files<'_>,
    classes: Vec<&str>,
    folder: &Path,
) -> Result<String, WriteError> {
    let head = "# A case made by openmobisim.make_case: openmobisim.run_file(this folder, \
                out=...) runs it.";
    let mut t = Toml::start(head, window)?;
    settings_inputs(&mut t, &Source::Osm(report), files, classes, Some(folder))?;
    Ok(t.text)
}

/// Part two's tables of the inputs, `[result.network]`, `[result.demand]` and
/// `[result.transit]`: their counts, fingerprints, readers' reports and repairs.
fn found(
    t: &mut Toml,
    network: &Network,
    demand: &Demand,
    given: &Given<'_>,
) -> Result<(), WriteError> {
    t.table("result.network");
    let source = match given.source {
        Source::Osm(_) => "osm",
        Source::Tables => "tables",
    };
    let fingerprint = hex(network.fingerprint());
    t.keys(&[
        ("source", Value::Str(source)),
        ("nodes", Value::Int(u64::from(network.node_count()))),
        ("links", Value::Int(u64::from(network.link_count()))),
        ("fingerprint", Value::Str(&fingerprint)),
    ])?;
    if let Source::Osm(r) = given.source {
        t.table("result.network.osm_report");
        let n = |x: u32| Value::Int(u64::from(x));
        t.keys(&[
            ("nodes", n(r.nodes)),
            ("links", n(r.links)),
            ("links_unreachable", n(r.links_unreachable)),
            ("loops_dropped", n(r.loops_dropped)),
            ("ways_cut", n(r.ways_cut)),
        ])?;
    }
    repairs(t, "result.network", given.network_repairs)?;
    t.table("result.demand");
    let fingerprint = hex(demand.fingerprint());
    t.key("trips", &Value::Int(u64::from(demand.trip_count())), None)?;
    if given.trips_given != u64::from(demand.trip_count()) {
        t.key("trips_given", &Value::Int(given.trips_given), None)?;
    }
    t.key("fingerprint", &Value::Str(&fingerprint), None)?;
    repairs(t, "result.demand", given.demand_repairs)?;
    let Some(transit) = &given.transit else { return Ok(()) };
    t.table("result.transit");
    let fingerprint = hex(transit.timetable.fingerprint());
    t.keys(&[
        ("stops", Value::Int(u64::from(transit.timetable.stop_count()))),
        ("runs", Value::Int(u64::from(transit.timetable.run_count()))),
        ("fingerprint", Value::Str(&fingerprint)),
    ])?;
    t.table("result.transit.gtfs_report");
    let (r, n) = (transit.report, |x: u32| Value::Int(u64::from(x)));
    t.keys(&[
        ("date", n(r.date)),
        ("trips", n(r.trips)),
        ("runs", n(r.runs)),
        ("calls", Value::Int(r.calls)),
        ("night_runs", n(r.night_runs)),
        ("frequency_runs", n(r.frequency_runs)),
        ("stops", n(r.stops)),
        ("trips_untimed", n(r.trips_untimed)),
        ("trips_single_call", n(r.trips_single_call)),
        ("times_interpolated", Value::Int(r.times_interpolated)),
        ("pickup_drop_off_ignored", Value::Int(r.pickup_drop_off_ignored)),
        ("frequency_rows_skipped", Value::Int(r.frequency_rows_skipped)),
        ("footpaths", Value::Int(r.footpaths)),
        ("transfers_applied", n(r.transfers_applied)),
        ("transfers_skipped", n(r.transfers_skipped)),
    ])?;
    repairs(t, "result.transit", &r.repairs)
}
