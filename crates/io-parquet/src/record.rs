//! `run.toml`: the record of a run, what was run, so that the folder alone tells which
//! parameters and inputs produced its three tables. TOML (version 1.0) written by hand: keys in
//! a fixed order, no time stamp, path, file name, machine or thread count, so a second write of
//! the same run is the same bytes.
//!
//! ```toml
//! # The record of one openmobisim run: ...
//! format = 1
//! openmobisim_version = "0.2.0-dev"
//! window_start_s = 25200
//! window_end_s = 28800
//!
//! [parameters]
//! capacity_per_lane = 1800.0  # veh/h per lane
//! ...                          (each row of PARAMETERS.md the run read, its unit as a comment)
//!
//! [parameters.coefficients."commute"]
//! car_drive_min = 1.0
//! ...                          (one table per class, a key per column of the choice row)
//!
//! [network]                    source, nodes, links, fingerprint
//! [network.osm_report]         the OSM reader's counts; absent for a network from tables
//! [demand]                     trips, classes, fingerprint
//! [transit]                    stops, runs, fingerprint; absent without transit
//! [transit.gtfs_report]        the GTFS reader's counts
//! [result]                     iterations, stopped_by_rule
//! ```
//!
//! TOML has no null: a value that does not exist is an absent key or table. `stop_walk_max` and
//! `transfer_walk_max` are absent without transit (nothing reads them there); the coefficients'
//! comment and tables are absent without a class; `osm_class_defaults` is never written (its one value is the OSM
//! reader's one table, `crates/io-osm/README.md`). A float is written by Rust's shortest
//! round-trip form with a point or an exponent (`1800.0`, `0.1`, `1e-7`), so TOML reads it as
//! a float of the same bits; an integer as an integer; a string and a quoted key as a basic
//! string with the quote, the backslash and every control character escaped. A float that is not
//! finite (TOML's `nan` and `inf` are never written), or an integer above TOML's largest
//! (2⁶³ − 1), is refused with an error naming its key, before anything is written.
//!
//! For bikes: `bike_speed`, the one speed of every bike, is a key of `[parameters]` with its
//! unit; a link's bike permission and a trip's bike are in the network's and the demand's
//! fingerprints, so a change to either changes the record.

use openmobisim_demand::demand::Demand;
use openmobisim_graph::network::Network;
use openmobisim_loop::parameters::{COLUMNS, ChoiceModel};
use openmobisim_loop::run::Run;

use crate::{Given, Source, WriteError};

/// The file's first line.
const HEAD: &str = "# The record of one openmobisim run: the parameters and inputs that produced \
                    trips.parquet, links.parquet and iterations.parquet in this folder.";

/// The rows of `PARAMETERS.md` written under `[parameters]`, in order, each with its unit as
/// that table's Unit column writes it (`coefficients` is written as tables after them;
/// `osm_class_defaults` is not written).
const UNITS: [(&str, &str); 14] = [
    ("capacity_per_lane", "veh/h per lane"),
    ("bike_speed", "km/h"),
    ("walk_speed", "km/h"),
    ("stop_walk_max", "m"),
    ("transfer_walk_max", "m"),
    ("boarding_slack", "s"),
    ("max_rides", "rides"),
    ("nearby_stops", "searches per trip end"),
    ("choice", "name"),
    ("nest_scale", "none"),
    ("iteration_cap", "iterations"),
    ("iteration_floor", "iterations"),
    ("link_bin", "s"),
    ("seed", "none"),
];

/// The comment above the coefficient tables.
const COEFFICIENTS: &str = "# coefficients: per class, one per column of the choice row: cost per \
                            minute of each time, per transfer, per euro, per unit of ln PS, and \
                            each mode's constant";

/// One value of the record.
enum Value<'a> {
    Int(u64),
    Float(f64),
    Str(&'a str),
    Flag(bool),
    Strs(Vec<&'a str>),
}

/// The text being written, and the table it is in (for the name of a refused key).
struct Toml {
    text: String,
    table: String,
}

impl Toml {
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
        "choice" => Value::Str(match p.choice {
            ChoiceModel::Logit => "logit",
            ChoiceModel::Deterministic => "deterministic",
        }),
        "nest_scale" => Value::Float(p.nest_scale),
        "iteration_cap" => Value::Int(u64::from(p.iteration_cap)),
        "iteration_floor" => Value::Int(u64::from(p.iteration_floor)),
        "link_bin" => Value::Int(u64::from(p.link_bin)),
        "seed" => Value::Int(p.seed),
        other => unreachable!("{other} is not a row of UNITS"),
    })
}

/// The record of `run` as TOML text.
pub(crate) fn text(
    run: &Run,
    network: &Network,
    demand: &Demand,
    given: &Given<'_>,
) -> Result<String, WriteError> {
    let mut t = Toml { text: format!("{HEAD}\n"), table: String::new() };
    let window = run.links.window();
    t.keys(&[
        ("format", Value::Int(1)),
        ("openmobisim_version", Value::Str(env!("CARGO_PKG_VERSION"))),
        ("window_start_s", Value::Int(u64::from(window.origin().get()))),
        ("window_end_s", Value::Int(u64::from(window.end().get()))),
    ])?;
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
    inputs(&mut t, network, demand, given)?;
    t.table("result");
    t.keys(&[
        ("iterations", Value::Int(run.iterations.len() as u64)),
        ("stopped_by_rule", Value::Flag(run.stopped_by_rule)),
    ])?;
    Ok(t.text)
}

/// The tables `[network]`, `[demand]` and `[transit]`, with the readers' counts.
fn inputs(
    t: &mut Toml,
    network: &Network,
    demand: &Demand,
    given: &Given<'_>,
) -> Result<(), WriteError> {
    t.table("network");
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
        t.table("network.osm_report");
        let n = |x: u32| Value::Int(u64::from(x));
        t.keys(&[
            ("nodes", n(r.nodes)),
            ("links", n(r.links)),
            ("links_unreachable", n(r.links_unreachable)),
            ("loops_dropped", n(r.loops_dropped)),
            ("ways_cut", n(r.ways_cut)),
        ])?;
    }
    t.table("demand");
    let fingerprint = hex(demand.fingerprint());
    t.keys(&[
        ("trips", Value::Int(u64::from(demand.trip_count()))),
        ("classes", Value::Strs(demand.class_ids().iter().collect())),
        ("fingerprint", Value::Str(&fingerprint)),
    ])?;
    let Some(transit) = &given.transit else { return Ok(()) };
    t.table("transit");
    let fingerprint = hex(transit.timetable.fingerprint());
    t.keys(&[
        ("stops", Value::Int(u64::from(transit.timetable.stop_count()))),
        ("runs", Value::Int(u64::from(transit.timetable.run_count()))),
        ("fingerprint", Value::Str(&fingerprint)),
    ])?;
    t.table("transit.gtfs_report");
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
    ])
}
