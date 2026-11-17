//! Reading one GTFS feed into the timetable of one service day, the busiest weekday, with the
//! footpaths between its stops derived once on the walk network.
//!
//! | Module | What it holds |
//! |---|---|
//! | `source` | the feed: one `.zip` |
//! | `csv` | a streaming RFC 4180 reader over `csv-core` |
//! | `calendar` | the services, the dates they run on, the busiest weekday |
//! | `stop_times` | the stops, the runs of the day and the night runs of the day before |
//! | `footpaths` | the walks between stops within `transfer_walk_max`, and `transfers.txt` |
//! | `lines` | beside the timetable: the stops' names, the runs' lines and kinds (`lines`) |
//!
//! Nothing here is a bike: a run carries passengers on its timetable, and the footpaths are
//! walks. Units are metric: metres and seconds.

mod calendar;
mod csv;
mod footpaths;
mod lines;
mod source;
mod stop_times;

use core::fmt;
use std::io::Read;
use std::path::Path;

use openmobisim_graph::network::Network;
use openmobisim_transit::timetable::{Timetable, TimetableBuilder, TimetableError};
use openmobisim_types::repairs::Repairs;
use openmobisim_types::units::{Duration, Metres};

use crate::csv::Csv;
pub use crate::lines::{Kind, Line, Lines, lines};

/// What a read found, kept and skipped. Every count is over the whole feed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// The service day read, as GTFS writes a date: `YYYYMMDD`.
    pub date: u32,
    /// The trips in service that day (rows of `trips.txt`).
    pub trips: u32,
    /// The runs of the timetable: the day's trips, one per headway for a frequency template,
    /// and the night runs.
    pub runs: u32,
    /// Their calls.
    pub calls: u64,
    /// Of the runs, those of the day before running past 24:00:00 of it.
    pub night_runs: u32,
    /// Of the runs, the day's runs made from `frequencies.txt` templates.
    pub frequency_runs: u32,
    /// The stops of the timetable: every stop or platform of the feed with a position.
    pub stops: u32,
    /// Trips of the day or the day before dropped because their first or last call has no
    /// time.
    pub trips_untimed: u32,
    /// Trips of the day or the day before dropped because they have one call.
    pub trips_single_call: u32,
    /// Calls whose time was interpolated.
    pub times_interpolated: u64,
    /// Calls read with `pickup_type` or `drop_off_type` 1, kept as calls where anyone may
    /// board and alight: the transit crate's call has no such flag.
    pub pickup_drop_off_ignored: u64,
    /// Rows of `frequencies.txt` for a trip read that do not apply (a bad time or headway, an
    /// end not after its start), skipped.
    pub frequency_rows_skipped: u64,
    /// The footpaths of the timetable.
    pub footpaths: u64,
    /// Rows of `transfers.txt` that raised or kept a footpath's seconds.
    pub transfers_applied: u32,
    /// Rows of `transfers.txt` skipped.
    pub transfers_skipped: u32,
    /// What the reader set right instead of refusing, by kind (the crate's README lists them).
    pub repairs: Repairs,
}

/// Why a feed could not be read.
#[derive(Clone, Debug, PartialEq)]
pub enum GtfsError {
    /// `transfer_walk_max` is not a finite number at or above zero.
    TransferWalkMax {
        /// The value given, metres.
        metres: f64,
    },
    /// The network was built in metres: it has no projection for the stops' longitude and
    /// latitude.
    NetworkInMetres,
    /// Reading from disk failed.
    Io(String),
    /// The file is not a readable zip.
    Zip(String),
    /// A file every feed must have is missing.
    MissingFile(&'static str),
    /// A column a file must have is missing.
    MissingColumn {
        /// The file.
        file: &'static str,
        /// The column.
        column: &'static str,
    },
    /// No day of the feed's dates has a trip in service.
    NoService,
    /// The timetable's builder refused the runs or the footpaths.
    Timetable(TimetableError),
}

impl fmt::Display for GtfsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TransferWalkMax { metres } => {
                write!(
                    f,
                    "transfer_walk_max is {metres} m: a finite number at or above 0 is needed"
                )
            }
            Self::NetworkInMetres => {
                write!(f, "the network was built in metres: GTFS stops are longitude/latitude")
            }
            Self::Io(e) => write!(f, "reading the feed: {e}"),
            Self::Zip(e) => write!(f, "reading the feed's zip: {e}"),
            Self::MissingFile(name) => write!(f, "the feed has no {name}"),
            Self::MissingColumn { file, column } => write!(f, "{file} has no {column} column"),
            Self::NoService => write!(f, "no day of the feed has a trip in service"),
            Self::Timetable(e) => write!(f, "building the timetable: {e}"),
        }
    }
}

impl core::error::Error for GtfsError {}

/// The column `column` of `file`, which the reference makes required.
fn required<R: Read>(
    csv: &Csv<R>,
    file: &'static str,
    column: &'static str,
) -> Result<usize, GtfsError> {
    csv.column(column).ok_or(GtfsError::MissingColumn { file, column })
}

/// Read the GTFS feed at `path` (a `.zip`) into the timetable of its busiest weekday, the
/// stops projected into `network`'s plane and the footpaths between them derived on its walk
/// links within `transfer_walk_max` metres (`PARAMETERS.md`), and what was read.
///
/// # Errors
///
/// A [`GtfsError`]: `transfer_walk_max` not a finite number at or above zero; a network built
/// in metres; the feed cannot be opened or read; a required file or column is missing; no day
/// has a trip. Every row that cannot be read is repaired or skipped and noted in the report's
/// `repairs` (the crate's README lists them): a stop, a call, a time, a run out of order; a
/// feed with no weekday service is read on its busiest day of all seven, noted; a network with
/// no walk link gives no footpath, noted.
pub fn read(
    path: &Path,
    network: &Network,
    transfer_walk_max: Metres,
) -> Result<(Timetable, Report), GtfsError> {
    let max = transfer_walk_max.get();
    if !max.is_finite() || max < 0.0 {
        return Err(GtfsError::TransferWalkMax { metres: max });
    }
    let projection = network.projection().ok_or(GtfsError::NetworkInMetres)?;
    let mut feed = source::Feed::open(path)?;
    let mut report = Report::default();
    let stops = stop_times::read_stops(&mut feed, &projection, &mut report)?;
    let services = calendar::Services::read(&mut feed, &mut report.repairs)?;
    let day = match services.busiest_day(true) {
        Some(day) => day,
        None => {
            let day = services.busiest_day(false).ok_or(GtfsError::NoService)?;
            let what =
                "feeds with no Monday to Friday in service: their busiest day of all seven read";
            report.repairs.note("weekday_absent", what, calendar::date_of(day).to_string());
            day
        }
    };
    report.date = calendar::date_of(day);
    let runs = stop_times::read_runs(&mut feed, &services, day, &stops, &mut report)?;
    let footpaths = footpaths::derive(&mut feed, network, &stops, max, &mut report)?;

    let mut builder = TimetableBuilder::new();
    for (id, &xy) in stops.ids.iter().zip(&stops.xy) {
        builder.add_stop(id.clone(), (Metres(xy.x), Metres(xy.y)));
    }
    for (id, calls) in runs {
        builder.add_run(id, calls);
    }
    for (a, b, seconds) in footpaths {
        let name = |s: u32| stops.ids[s as usize].clone();
        builder.add_footpath(name(a), name(b), Duration(seconds));
    }
    let timetable = builder.build().map_err(GtfsError::Timetable)?;
    Ok((timetable, report))
}
