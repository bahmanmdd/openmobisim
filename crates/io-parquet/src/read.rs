//! Reading a run's four tables back for its pages. Each file's columns are found by name (another
//! order, or another column beside them, reads the same); each reader reads only the columns the
//! pages need and returns, beside what it read, the warnings of what it found and set aside.
//!
//! A file is refused, naming it, only where the work cannot be done: it is missing or not a
//! readable Parquet file, a column it needs is missing or of a type it cannot read (a list of
//! text read whatever the name of its inner field, `item` as this crate writes it or `element`
//! as pyarrow does), or a row's own id (its trip in `trips.parquet` and `routes.parquet`, its link
//! in `links.parquet`) names nothing the inputs hold, so the row cannot be placed. Everything else
//! is a warning and the reading goes on with what agrees:
//! - `trips.parquet`: a trip of the demand without a row is read as not run (the run drops
//!   the trips its window cannot hold, and writes no row for them), warned only when their
//!   number is not the number the record says the run dropped; a trip's second row is set aside
//!   (the first is used); a `mode` that is not a mode is read as none.
//! - `links.parquet`: a row whose bin does not start on the window's grid, or starts outside the
//!   window, is set aside; a (link, bin) given twice, the first row is used. The rows are returned
//!   in link then bin order.
//! - `routes.parquet`: a trip whose legs are misnumbered (not 0 to n − 1, each once), of an
//!   unknown kind, without a value their kind needs, or naming a link, a run or a stop the inputs
//!   do not hold, is left out of the journeys; a `walk_gap` or `transfer_walk` without its seconds
//!   is read with its seconds unknown (`NaN`), the trip kept. [`read`] then leaves out each trip
//!   whose legs do not agree with its mode in `trips.parquet` (see [`agrees`]).
//! - `iterations.parquet`: a file without `would_change_share`, written by a run before that
//!   share was reported, is read with the share unknown (`NaN`).
//!
//! What the readers cannot see: a value changed in place (a time, a count, a stop), and a link
//! bin replaced by a divisor of it (every bin of 60 s also starts on a grid of 30 s).

use core::fmt;
use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};

use arrow_array::cast::AsArray;
use arrow_array::types::{Float64Type, UInt32Type, UInt64Type};
use arrow_array::{Array, RecordBatch};
use arrow_schema::{DataType, Schema};
use openmobisim_demand::demand::Demand;
use openmobisim_graph::network::Network;
use openmobisim_loop::assess::Iteration;
use openmobisim_transit::raptor::{Footpath, Ride};
use openmobisim_transit::timetable::Timetable;
use openmobisim_types::ids::{EntityId, LinkId, StopId, TransitRunId};
use openmobisim_types::mode::Mode;
use openmobisim_types::time::{Second, StepGrid};
use openmobisim_types::units::Duration;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

use crate::routes::RouteLeg;
use crate::{iterations, links, routes, trips};

/// Why a run's table was not read: the file and what is wrong with it; `io` when the file could
/// not be opened or read at all (missing, say).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputError {
    /// The file.
    pub file: PathBuf,
    /// What is wrong.
    pub why: String,
    /// Whether it could not be opened or read.
    pub io: bool,
}

impl fmt::Display for OutputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.file.display(), self.why)
    }
}

impl core::error::Error for OutputError {}

/// What the pages read of `trips.parquet`, per trip of the demand in its order: the mode it
/// took (none without an alternative), its realised door-to-door time (`NaN` where null), and
/// whether it finished. The other columns are not read.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TripRows {
    /// The mode taken.
    pub mode: Vec<Option<Mode>>,
    /// Door to door, seconds, as realised; `NaN` where the file holds null.
    pub travel_time: Vec<f64>,
    /// Whether it arrived before the window's end.
    pub finished: Vec<bool>,
    /// Whether it has a row: false for a trip the run did not run.
    pub present: Vec<bool>,
}

/// What the pages read of `links.parquet`: per (link, bin of entry) that someone entered, in
/// link then bin order. The exit times are not read.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Links {
    /// The row's link.
    pub link: Vec<LinkId>,
    /// Its bin of entry: step `i` of the window.
    pub bin: Vec<u32>,
    /// The cars that entered.
    pub cars: Vec<u32>,
    /// The sum of their crossing times, seconds (0 without a car).
    pub car_time: Vec<f64>,
    /// The bikes that entered.
    pub bikes: Vec<u32>,
    /// The walkers that entered.
    pub walkers: Vec<u32>,
}

/// The three tables the pages draw a run from, read back and checked against one another.
#[derive(Clone, Debug)]
pub struct Tables {
    /// `trips.parquet`.
    pub trips: TripRows,
    /// `links.parquet`.
    pub links: Links,
    /// `routes.parquet`: per trip of the demand, in its order, the legs it took (none without an
    /// alternative); `None` for a trip left out of the journeys.
    pub routes: Routes,
    /// What was found and set aside, each naming its file: for the caller to show.
    pub warnings: Vec<String>,
}

/// Read `trips.parquet`, `links.parquet` and `routes.parquet` from `folder`, written for the run
/// of `demand` on `network` and `timetable` over `window` (its bins those of the links table),
/// which dropped `dropped` of the demand's trips (its record's count); then leave out of the
/// journeys each trip whose legs do not agree with its mode ([`agrees`]).
///
/// # Errors
///
/// [`OutputError`], naming the first file that cannot be read.
pub fn read(
    folder: &Path,
    demand: &Demand,
    network: &Network,
    timetable: Option<&Timetable>,
    window: StepGrid,
    dropped: u64,
) -> Result<Tables, OutputError> {
    let (trips, mut warnings) = read_trips(&folder.join("trips.parquet"), demand, dropped)?;
    let (links, more) = read_links(&folder.join("links.parquet"), network, window)?;
    warnings.extend(more);
    let file = folder.join("routes.parquet");
    let (mut routes, more) = read_routes(&file, demand, network, timetable)?;
    warnings.extend(more);
    let mut left_out = Vec::new();
    for (t, legs) in routes.iter_mut().enumerate() {
        if legs.as_ref().is_some_and(|l| !agrees(trips.mode[t], l)) {
            *legs = None;
            left_out.push(demand.trip_ids().external(u32::try_from(t).unwrap_or(u32::MAX)).into());
        }
    }
    if !left_out.is_empty() {
        let what = "trips left out of the journeys: their legs do not agree with their mode in \
                    trips.parquet";
        warnings.push(note(&file, what, &left_out));
    }
    Ok(Tables { trips, links, routes, warnings })
}

/// Whether `legs` agree with `mode`: without a mode, no leg; by car, bike or walk, exactly one
/// route of that mode and no ride; by transit, at least one ride and no route.
#[must_use]
pub fn agrees(mode: Option<Mode>, legs: &[RouteLeg]) -> bool {
    let routes: Vec<Mode> = legs
        .iter()
        .filter_map(|l| if let RouteLeg::Route { mode, .. } = l { Some(*mode) } else { None })
        .collect();
    let rides = legs.iter().filter(|l| matches!(l, RouteLeg::Ride(_))).count();
    match mode {
        None => legs.is_empty(),
        Some(Mode::Transit) => rides > 0 && routes.is_empty(),
        Some(m) => routes == [m] && rides == 0,
    }
}

/// A warning about `file`: `items.len()` `what`, the first five of them named.
fn note(file: &Path, what: &str, items: &[String]) -> String {
    let shown: Vec<&str> = items.iter().take(5).map(String::as_str).collect();
    let more =
        if items.len() > 5 { format!(" and {} more", items.len() - 5) } else { String::new() };
    format!("{}: {} {what}: {}{more}", file.display(), items.len(), shown.join(", "))
}

/// A file being read: its path, its batches, and where each column the reader needs is.
struct Reading {
    file: PathBuf,
    batches: Vec<RecordBatch>,
    at: HashMap<String, usize>,
}

/// Where the column `name` of the writer's `schema` is among the file's columns `found`, of the
/// writer's type (a list's inner field by its type alone): none when the file has no such
/// column, an error naming the two types when it has one of another type.
fn find(found: &Schema, schema: &Schema, name: &str) -> Result<Option<usize>, String> {
    let want = schema.field_with_name(name).expect("a column of the writer's");
    let Ok(i) = found.index_of(name) else { return Ok(None) };
    if same_type(found.field(i).data_type(), want.data_type()) {
        Ok(Some(i))
    } else {
        Err(format!(
            "column {name} is {}; it is read as {}",
            found.field(i).data_type(),
            want.data_type()
        ))
    }
}

impl Reading {
    /// Open `file` and find the columns `needed` of the writer's `schema`, and those of
    /// `optional` it holds, by name ([`find`]). A missing column of `needed` is refused; a
    /// missing one of `optional` is left for the caller to read as unknown.
    fn open(
        file: &Path,
        schema: &Schema,
        needed: &[&str],
        optional: &[&str],
    ) -> Result<Self, OutputError> {
        let error = |why: String, io: bool| OutputError { file: file.to_path_buf(), why, io };
        let f = File::open(file).map_err(|e| error(e.to_string(), true))?;
        let builder = ParquetRecordBatchReaderBuilder::try_new(f)
            .map_err(|e| error(format!("not a readable Parquet file: {e}"), false))?;
        let found = builder.schema().clone();
        let mut at = HashMap::new();
        for (name, required) in
            needed.iter().map(|n| (n, true)).chain(optional.iter().map(|n| (n, false)))
        {
            match find(&found, schema, name).map_err(|why| error(why, false))? {
                Some(i) => {
                    at.insert((*name).to_owned(), i);
                }
                None if required => return Err(error(format!("it has no column {name}"), false)),
                None => {}
            }
        }
        let reader = builder.build().map_err(|e| error(e.to_string(), false))?;
        let batches =
            reader.collect::<Result<Vec<_>, _>>().map_err(|e| error(e.to_string(), false))?;
        Ok(Self { file: file.to_path_buf(), batches, at })
    }

    /// Whether the file holds the column `name` (one of those asked for).
    fn has(&self, name: &str) -> bool {
        self.at.contains_key(name)
    }

    fn refuse(&self, why: String) -> OutputError {
        OutputError { file: self.file.clone(), why, io: false }
    }

    fn rows(&self) -> usize {
        self.batches.iter().map(RecordBatch::num_rows).sum()
    }

    fn column<'a>(&self, b: &'a RecordBatch, name: &str) -> &'a dyn Array {
        b.column(self.at[name]).as_ref()
    }

    /// The column `name` as text, `None` for a null.
    fn texts(&self, name: &str) -> Vec<Option<&str>> {
        self.batches.iter().flat_map(|b| self.column(b, name).as_string::<i32>().iter()).collect()
    }

    fn u32s(&self, name: &str) -> Vec<Option<u32>> {
        let all = self
            .batches
            .iter()
            .flat_map(|b| self.column(b, name).as_primitive::<UInt32Type>().iter());
        all.collect()
    }

    /// The column `name` as counts, 0 for a null.
    fn u64s(&self, name: &str) -> Vec<u64> {
        let all = self
            .batches
            .iter()
            .flat_map(|b| self.column(b, name).as_primitive::<UInt64Type>().iter());
        all.map(Option::unwrap_or_default).collect()
    }

    /// The column `name` as floats, `NaN` for a null.
    fn floats(&self, name: &str) -> Vec<f64> {
        let all = self
            .batches
            .iter()
            .flat_map(|b| self.column(b, name).as_primitive::<Float64Type>().iter());
        all.map(|v| v.unwrap_or(f64::NAN)).collect()
    }

    /// The column `name` as flags, false for a null.
    fn bools(&self, name: &str) -> Vec<bool> {
        let all = self.batches.iter().flat_map(|b| self.column(b, name).as_boolean().iter());
        all.map(Option::unwrap_or_default).collect()
    }

    /// The list column `name`, each list's text read by `resolve` into an id: `None` for a null
    /// list, `Some(Err(()))` for a list holding an id `resolve` does not know.
    fn id_lists<'a, T>(
        &'a self,
        name: &str,
        mut resolve: impl FnMut(&'a str) -> Option<T>,
    ) -> Vec<Option<Result<Vec<T>, ()>>> {
        let mut out = Vec::with_capacity(self.rows());
        for b in &self.batches {
            let list = self.column(b, name).as_list::<i32>();
            let (values, offsets) = (list.values().as_string::<i32>(), list.value_offsets());
            for row in 0..list.len() {
                if list.is_null(row) {
                    out.push(None);
                    continue;
                }
                let at = |k: usize| usize::try_from(offsets[k]).unwrap_or_default();
                let span = at(row)..at(row + 1);
                let ids: Option<Vec<T>> = span.map(|k| resolve(values.value(k))).collect();
                out.push(Some(ids.ok_or(())));
            }
        }
        out
    }
}

/// Whether a column of type `found` reads as `want`: the same type, a list's inner field
/// compared by its type alone (its name, `item` or `element`, and its nullability aside).
fn same_type(found: &DataType, want: &DataType) -> bool {
    match (found, want) {
        (DataType::List(a), DataType::List(b)) => a.data_type() == b.data_type(),
        _ => found == want,
    }
}

/// A mode by its name, `car`, `bike`, `walk` or `transit`.
fn mode(name: &str) -> Option<Mode> {
    Mode::ALL.into_iter().find(|m| m.to_string() == name)
}

/// Read the columns the pages need of `trips.parquet`, written for `demand` by a run that
/// dropped `dropped` of its trips, and its warnings: the trips without a row are warned of only
/// when they are not `dropped` in number.
///
/// # Errors
///
/// [`OutputError`]: the file cannot be read, a column it needs (`trip_id`, `mode`,
/// `travel_time_s`, `finished`) is missing or of another type, or a row's trip is not the
/// demand's.
pub fn read_trips(
    path: &Path,
    demand: &Demand,
    dropped: u64,
) -> Result<(TripRows, Vec<String>), OutputError> {
    let r = Reading::open(
        path,
        &trips::schema(),
        &["trip_id", "mode", "travel_time_s", "finished"],
        &[],
    )?;
    let n = demand.trip_count() as usize;
    let mut out = TripRows {
        mode: vec![None; n],
        travel_time: vec![f64::NAN; n],
        finished: vec![false; n],
        present: vec![false; n],
    };
    let (ids, modes, times, finished) =
        (r.texts("trip_id"), r.texts("mode"), r.floats("travel_time_s"), r.bools("finished"));
    let (mut seen, mut twice, mut unknown) = (vec![false; n], Vec::new(), Vec::new());
    for i in 0..r.rows() {
        let id = ids[i].unwrap_or("");
        let t =
            demand.trip_ids().id_of(id).ok_or_else(|| {
                r.refuse(format!("row {i}: trip {id:?} is not of the demand given"))
            })? as usize;
        if std::mem::replace(&mut seen[t], true) {
            twice.push(id.to_owned());
            continue;
        }
        out.mode[t] = modes[i].and_then(|m| {
            let m2 = mode(m);
            if m2.is_none() {
                unknown.push(id.to_owned());
            }
            m2
        });
        out.travel_time[t] = times[i];
        out.finished[t] = finished[i];
        out.present[t] = true;
    }
    let mut missing: Vec<String> = (0..n)
        .filter(|&t| !seen[t])
        .map(|t| demand.trip_ids().external(u32::try_from(t).unwrap_or(u32::MAX)).to_owned())
        .collect();
    if missing.len() as u64 == dropped {
        missing.clear();
    }
    let mut warnings = Vec::new();
    let not_run = format!(
        "trips of the demand without a row, read as not run (the record says the run dropped \
         {dropped})"
    );
    for (items, what) in [
        (missing, not_run.as_str()),
        (twice, "trips given twice, their first row used"),
        (unknown, "trips whose mode is not car, bike, walk or transit, read as none"),
    ] {
        if !items.is_empty() {
            warnings.push(note(path, what, &items));
        }
    }
    Ok((out, warnings))
}

/// Read the columns the pages need of `links.parquet`, written for `network` over `window`, and
/// its warnings.
///
/// # Errors
///
/// [`OutputError`]: the file cannot be read, a column it needs (`link_id`, `bin_start_s`,
/// `cars`, `car_time_s`, `bikes`, `walkers`) is missing or of another type, or a row's link is
/// not the network's.
pub fn read_links(
    path: &Path,
    network: &Network,
    window: StepGrid,
) -> Result<(Links, Vec<String>), OutputError> {
    let needed = ["link_id", "bin_start_s", "cars", "car_time_s", "bikes", "walkers"];
    let r = Reading::open(path, &links::schema(), &needed, &[])?;
    let (ids, starts) = (r.texts("link_id"), r.u32s("bin_start_s"));
    let (cars, time, bikes, walkers) =
        (r.u32s("cars"), r.floats("car_time_s"), r.u32s("bikes"), r.u32s("walkers"));
    let (origin, step) = (window.origin().get(), window.step_seconds());
    let mut rows: Vec<(LinkId, u32, usize)> = Vec::with_capacity(r.rows());
    let mut off = Vec::new();
    for i in 0..r.rows() {
        let id = ids[i].unwrap_or("");
        let link = network
            .link_ids()
            .id_of(id)
            .ok_or_else(|| r.refuse(format!("row {i}: link {id:?} is not of the network given")))?;
        let bin = starts[i]
            .and_then(|s| s.checked_sub(origin))
            .filter(|d| d % step == 0)
            .map(|d| d / step)
            .filter(|&b| b < window.n_steps());
        match bin {
            Some(b) => rows.push((LinkId::new(link), b, i)),
            None => {
                off.push(format!("{id} at {}", starts[i].map_or("null".into(), |s| s.to_string())))
            }
        }
    }
    rows.sort_by_key(|&(l, b, i)| (l, b, i));
    let mut twice = Vec::new();
    rows.dedup_by(|later, first| {
        let same = (later.0, later.1) == (first.0, first.1);
        if same {
            twice.push(format!("{} at bin {}", ids[later.2].unwrap_or(""), later.1));
        }
        same
    });
    let mut out = Links::default();
    for &(link, bin, i) in &rows {
        out.link.push(link);
        out.bin.push(bin);
        out.cars.push(cars[i].unwrap_or_default());
        out.car_time.push(if time[i].is_nan() { 0.0 } else { time[i] });
        out.bikes.push(bikes[i].unwrap_or_default());
        out.walkers.push(walkers[i].unwrap_or_default());
    }
    let mut warnings = Vec::new();
    for (items, what) in [
        (off, "rows set aside: their bin does not start on the record's grid inside its window"),
        (twice, "(link, bin) rows given twice, the first used"),
    ] {
        if !items.is_empty() {
            warnings.push(note(path, what, &items));
        }
    }
    Ok((out, warnings))
}

/// The column of `iterations.parquet` a run written before it was reported lacks.
const WOULD_CHANGE: &str = "would_change_share";

/// Read `iterations.parquet`, its rows in the file's order, and its warnings. A file without
/// `would_change_share` (a run written before that share was reported) is read with the share
/// unknown (`NaN`), warned.
///
/// # Errors
///
/// [`OutputError`]: the file cannot be read, or a column other than `would_change_share` is
/// missing, or a column is of another type.
pub fn read_iterations(path: &Path) -> Result<(Vec<Iteration>, Vec<String>), OutputError> {
    let schema = iterations::schema();
    let names: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
    let needed: Vec<&str> = names.iter().copied().filter(|&n| n != WOULD_CHANGE).collect();
    let r = Reading::open(path, &schema, &needed, &[WOULD_CHANGE])?;
    let number = r.u32s("iteration");
    let counts =
        ["finished", "unfinished", "without_alternative", "without_transit", "without_stop"]
            .map(|n| r.u64s(n));
    let share = |name: &str| r.floats(name);
    let (mean, offered, switch, found, gap) = (
        share("mean_trip_time_s"),
        share("offered_share"),
        share("switch_share"),
        share("found_share"),
        share("gap"),
    );
    let mut warnings = Vec::new();
    let would = if r.has(WOULD_CHANGE) {
        share(WOULD_CHANGE)
    } else {
        warnings.push(format!(
            "{}: it has no column {WOULD_CHANGE} (a run written before that share was reported): \
             the share of trips that would change is read as unknown",
            path.display()
        ));
        vec![f64::NAN; r.rows()]
    };
    let stages = iterations::STAGES.map(|n| r.floats(n));
    let rows = (0..r.rows())
        .map(|k| Iteration {
            iteration: number[k].unwrap_or_default(),
            mean_trip_time: mean[k],
            finished: counts[0][k],
            unfinished: counts[1][k],
            without_alternative: counts[2][k],
            without_transit: counts[3][k],
            without_stop: counts[4][k],
            offered_share: offered[k],
            switch_share: switch[k],
            found_share: found[k],
            would_change_share: would[k],
            gap: gap[k],
            stage_seconds: [0, 1, 2, 3, 4].map(|s| stages[s][k]),
        })
        .collect();
    Ok((rows, warnings))
}

/// Per trip of the demand, in its order, the legs it took, in order: none without an
/// alternative, `None` for a trip left out of the journeys.
pub type Routes = Vec<Option<Vec<RouteLeg>>>;

/// The columns of `routes.parquet` the legs are read from, row by row.
struct RouteColumns<'a> {
    kind: Vec<Option<&'a str>>,
    seconds: Vec<f64>,
    links: Vec<Option<Result<Vec<LinkId>, ()>>>,
    run: Vec<Option<&'a str>>,
    board: Vec<Option<&'a str>>,
    alight: Vec<Option<&'a str>>,
    board_s: Vec<Option<u32>>,
    alight_s: Vec<Option<u32>>,
    from: Vec<Option<&'a str>>,
    to: Vec<Option<&'a str>>,
}

impl RouteColumns<'_> {
    /// Row `i`'s leg, `None` when its kind is unknown, it lacks a value its kind needs, or it
    /// names a link, run or stop the inputs do not hold. A walk's seconds may be `NaN` (null).
    fn leg(&self, i: usize, timetable: Option<&Timetable>) -> Option<RouteLeg> {
        let stop = |s: Option<&str>| Some(StopId::new(timetable?.stop_ids().id_of(s?)?));
        match self.kind[i]? {
            "walk_gap" => Some(RouteLeg::WalkGap(Duration(self.seconds[i]))),
            name @ ("car" | "bike" | "walk") => {
                let links = self.links[i].as_ref()?.as_ref().ok()?.clone();
                Some(RouteLeg::Route { mode: mode(name)?, links })
            }
            "ride" => Some(RouteLeg::Ride(Ride {
                run: TransitRunId::new(timetable?.run_ids().id_of(self.run[i]?)?),
                board: (stop(self.board[i])?, Second(self.board_s[i]?)),
                alight: (stop(self.alight[i])?, Second(self.alight_s[i]?)),
            })),
            "transfer_walk" => Some(RouteLeg::TransferWalk(Footpath {
                from: stop(self.from[i])?,
                to: stop(self.to[i])?,
                seconds: Duration(self.seconds[i]),
            })),
            _ => None,
        }
    }
}

/// Read `routes.parquet`, written for `demand` on `network` and `timetable`: per trip of the
/// demand, in its order, its legs in order ([`Routes`]; a trip left out, see the module), and
/// its warnings.
///
/// # Errors
///
/// [`OutputError`]: the file cannot be read, a column is missing or of another type, or a
/// row's trip is not the demand's.
pub fn read_routes(
    path: &Path,
    demand: &Demand,
    network: &Network,
    timetable: Option<&Timetable>,
) -> Result<(Routes, Vec<String>), OutputError> {
    let schema = routes::schema();
    let names: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
    let r = Reading::open(path, &schema, &names, &[])?;
    // Each distinct link id is looked up once.
    let mut known: HashMap<&str, Option<LinkId>> = HashMap::new();
    let lookup =
        |id| *known.entry(id).or_insert_with(|| network.link_ids().id_of(id).map(LinkId::new));
    let c = RouteColumns {
        kind: r.texts("kind"),
        seconds: r.floats("seconds"),
        links: r.id_lists("links", lookup),
        run: r.texts("run_id"),
        board: r.texts("board_stop"),
        alight: r.texts("alight_stop"),
        board_s: r.u32s("board_s"),
        alight_s: r.u32s("alight_s"),
        from: r.texts("from_stop"),
        to: r.texts("to_stop"),
    };
    let (trip, leg) = (r.texts("trip_id"), r.u32s("leg"));
    let n = demand.trip_count() as usize;
    let mut rows: Vec<Vec<(Option<u32>, Option<RouteLeg>)>> = vec![Vec::new(); n];
    let mut unknown_seconds = Vec::new();
    for i in 0..r.rows() {
        let id = trip[i].unwrap_or("");
        let t =
            demand.trip_ids().id_of(id).ok_or_else(|| {
                r.refuse(format!("row {i}: trip {id:?} is not of the demand given"))
            })? as usize;
        if matches!(c.kind[i], Some("walk_gap" | "transfer_walk")) && c.seconds[i].is_nan() {
            unknown_seconds.push(id.to_owned());
        }
        rows[t].push((leg[i], c.leg(i, timetable)));
    }
    let name =
        |t: usize| demand.trip_ids().external(u32::try_from(t).unwrap_or(u32::MAX)).to_owned();
    let mut left_out = Vec::new();
    let mut out = Vec::with_capacity(n);
    for (t, mut legs) in rows.into_iter().enumerate() {
        legs.sort_by_key(|(k, _)| *k);
        let numbered = legs.iter().enumerate().all(|(k, (at, _))| *at == u32::try_from(k).ok());
        let read: Option<Vec<RouteLeg>> = legs.into_iter().map(|(_, l)| l).collect();
        let kept = read.filter(|_| numbered);
        if kept.is_none() {
            left_out.push(name(t));
        }
        out.push(kept);
    }
    unknown_seconds.dedup();
    let mut warnings = Vec::new();
    for (items, what) in [
        (
            left_out,
            "trips left out of the journeys: their legs are misnumbered, of an unknown kind, \
             without a value their kind needs, or name a link, run or stop the inputs do not hold",
        ),
        (
            unknown_seconds,
            "trips with a walk whose seconds are missing: their legs' seconds are unknown",
        ),
    ] {
        if !items.is_empty() {
            warnings.push(note(path, what, &items));
        }
    }
    Ok((out, warnings))
}
