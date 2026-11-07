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
//!
//! Nothing here is a bike: a run carries passengers on its timetable, and the footpaths are
//! walks. Units are metric: metres and seconds.

mod calendar;
mod csv;
mod footpaths;
mod source;
mod stop_times;

use core::fmt;
use std::io::Read;
use std::path::Path;

use openmobisim_graph::network::Network;
use openmobisim_transit::timetable::{Timetable, TimetableBuilder, TimetableError};
use openmobisim_types::units::{Duration, Metres};

use crate::csv::Csv;

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
    /// A value of a row cannot be read: a bad number, time, date or position.
    BadValue {
        /// The file.
        file: &'static str,
        /// The row's id: its trip, stop or service.
        id: String,
        /// The column.
        column: &'static str,
        /// The value as written.
        value: String,
    },
    /// An id is given twice: a stop, a trip, a service's calendar, or a service's exception
    /// on one date.
    RepeatedId {
        /// The file.
        file: &'static str,
        /// The id (for an exception, the service and the date).
        id: String,
    },
    /// A row of `stop_times.txt` names a trip that `trips.txt` does not hold (the reference
    /// requires every stop time's trip to exist).
    UnknownTrip {
        /// The trip named.
        trip: String,
        /// The row's `stop_sequence`, as written.
        stop_sequence: String,
    },
    /// A row of `stop_times.txt` names a stop that `stops.txt` does not hold as a stop or
    /// platform with a position.
    UnknownStop {
        /// The trip.
        trip: String,
        /// The stop named.
        stop: String,
    },
    /// No Monday to Friday of the feed's dates has a trip in service.
    NoWeekdayService,
    /// No link of the network allows walking: no stop can be linked to it.
    NoWalkLinks,
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
            Self::BadValue { file, id, column, value } => {
                write!(f, "{file}: row {id}: {column} {value:?} cannot be read")
            }
            Self::RepeatedId { file, id } => write!(f, "{file}: {id} is given twice"),
            Self::UnknownTrip { trip, stop_sequence } => write!(
                f,
                "stop_times.txt: the row of stop_sequence {stop_sequence:?} names trip {trip:?}, \
                 which trips.txt does not hold"
            ),
            Self::UnknownStop { trip, stop } => {
                write!(f, "stop_times.txt: trip {trip} calls at unknown stop {stop:?}")
            }
            Self::NoWeekdayService => write!(f, "no Monday to Friday of the feed has a trip"),
            Self::NoWalkLinks => write!(f, "no link of the network allows walking"),
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
/// in metres; the feed cannot be opened or read; a required file or column is missing; a row
/// of `stops.txt`, `trips.txt`, `calendar.txt`, `calendar_dates.txt` or `stop_times.txt`
/// cannot be read (a bad value, an id given twice, an unknown trip or stop), naming its file, id and
/// value; no weekday has a trip; no link allows walking; the timetable's builder refuses a run (a call
/// out of order, naming the run).
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
    let services = calendar::Services::read(&mut feed)?;
    let day = services.busiest_weekday().ok_or(GtfsError::NoWeekdayService)?;
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
