//! The names beside the timetable: each stop's name, and each run's line (its route's short name,
//! else its long name, else its route id) and kind of service (from the route's `route_type`).
//! The timetable does not hold them: they name what it schedules, for the run's pages.
//!
//! - **Stops** (`stops.txt`): `stop_name` by `stop_id`; a blank name is no name.
//! - **Routes** (`routes.txt`, optional here): `route_short_name`, falling back to
//!   `route_long_name`, then to the `route_id`; `route_type` as a kind: 0, 5 and 900–999 tram;
//!   1, 12 and 400–699 metro; 2, 100–199 and 300–399 rail; 3, 11, 200–299 and 700–899 bus; 4,
//!   1000–1099 and 1200–1299 ferry; anything else, or a type that is not a number, other (the
//!   basic and the extended route types, as release 0.1 read them).
//! - **Trips** (`trips.txt`): each trip's `route_id`. A run is named by its trip (the reader's
//!   rule, `stop_times`): the trip's id, `<trip>@<seconds>` for a run of a frequency template,
//!   and either with `@previous-day` for a night run; [`Lines::line_of_run`] reads it back.
//!
//! Nothing here refuses a feed: a file or a column missing, a trip of an unknown route, leave
//! the names they would give unknown.

use std::collections::BTreeMap;
use std::path::Path;

use crate::GtfsError;
use crate::source::Feed;

/// A kind of transit service, as GTFS `route_type` gives it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Kind {
    /// Trains: intercity, regional, suburban.
    Rail = 0,
    /// Metro, underground, light rail on its own track, monorail.
    Metro = 1,
    /// Tram.
    Tram = 2,
    /// Bus, coach and trolleybus.
    Bus = 3,
    /// Ferry and other water transport.
    Ferry = 4,
    /// Cable cars, funiculars and anything else scheduled.
    Other = 5,
}

impl Kind {
    /// Every kind, in id order.
    pub const ALL: [Kind; 6] =
        [Kind::Rail, Kind::Metro, Kind::Tram, Kind::Bus, Kind::Ferry, Kind::Other];

    /// The kind of a GTFS `route_type`: the basic types 0–12 and the extended types 100–1799.
    #[must_use]
    pub const fn from_route_type(route_type: u16) -> Self {
        match route_type {
            0 | 5 | 900..=999 => Kind::Tram,
            1 | 12 | 400..=699 => Kind::Metro,
            2 | 100..=199 | 300..=399 => Kind::Rail,
            3 | 11 | 200..=299 | 700..=899 => Kind::Bus,
            4 | 1000..=1099 | 1200..=1299 => Kind::Ferry,
            _ => Kind::Other,
        }
    }

    /// Its name: `rail`, `metro`, `tram`, `bus`, `ferry`, `other`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Kind::Rail => "rail",
            Kind::Metro => "metro",
            Kind::Tram => "tram",
            Kind::Bus => "bus",
            Kind::Ferry => "ferry",
            Kind::Other => "other",
        }
    }
}

/// A line: its name and its kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    /// The route's short name, else its long name, else its id.
    pub name: String,
    /// Its kind of service.
    pub kind: Kind,
}

/// The names beside a timetable (see the module documentation).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lines {
    /// Each named stop's name, by stop id.
    pub stop_names: BTreeMap<String, String>,
    /// Each trip's line, by trip id (a trip of an unknown route has none).
    pub trip_lines: BTreeMap<String, Line>,
}

impl Lines {
    /// The name of stop `stop` (its id), if it has one.
    #[must_use]
    pub fn stop_name(&self, stop: &str) -> Option<&str> {
        self.stop_names.get(stop).map(String::as_str)
    }

    /// The line of the run named `run` by the reader: its trip's, the trip being the run's name
    /// less a `@previous-day` and a `@<seconds>` ending.
    #[must_use]
    pub fn line_of_run(&self, run: &str) -> Option<&Line> {
        let trip = run.strip_suffix("@previous-day").unwrap_or(run);
        if let Some(line) = self.trip_lines.get(trip) {
            return Some(line);
        }
        let (template, seconds) = trip.rsplit_once('@')?;
        seconds.bytes().all(|b| b.is_ascii_digit()).then(|| self.trip_lines.get(template)).flatten()
    }
}

/// Read the names beside the timetable of the GTFS feed at `path` (see the module
/// documentation).
///
/// # Errors
///
/// [`GtfsError::Io`] or [`GtfsError::Zip`]: the feed cannot be opened or read.
pub fn lines(path: &Path) -> Result<Lines, GtfsError> {
    let mut feed = Feed::open(path)?;
    let mut out = Lines::default();
    if let Some(mut csv) = feed.optional_csv("stops.txt")? {
        if let (Some(id), Some(name)) = (csv.column("stop_id"), csv.column("stop_name")) {
            while csv.next()? {
                if !csv.field(name).is_empty() {
                    out.stop_names.insert(csv.field(id).to_owned(), csv.field(name).to_owned());
                }
            }
        }
    }
    let mut routes: BTreeMap<String, Line> = BTreeMap::new();
    if let Some(mut csv) = feed.optional_csv("routes.txt")? {
        let Some(id) = csv.column("route_id") else { return Ok(out) };
        let (short, long, kind) = (
            csv.column("route_short_name"),
            csv.column("route_long_name"),
            csv.column("route_type"),
        );
        while csv.next()? {
            let route = csv.field(id).to_owned();
            let name = [csv.get(short), csv.get(long)].into_iter().find(|n| !n.is_empty());
            let name = name.map_or_else(|| route.clone(), str::to_owned);
            let kind = csv.get(kind).parse::<u16>().map_or(Kind::Other, Kind::from_route_type);
            routes.entry(route).or_insert(Line { name, kind });
        }
    }
    if let Some(mut csv) = feed.optional_csv("trips.txt")? {
        if let (Some(trip), Some(route)) = (csv.column("trip_id"), csv.column("route_id")) {
            while csv.next()? {
                if let Some(line) = routes.get(csv.field(route)) {
                    out.trip_lines.insert(csv.field(trip).to_owned(), line.clone());
                }
            }
        }
    }
    Ok(out)
}
