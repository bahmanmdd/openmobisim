//! Reading one service day of a feed into a [`Timetable`] (S199).
//!
//! 1. **Stops** (`stops.txt`): every stop or platform (`location_type` 0 or
//!    empty) with a position; the caller's `keep` decides which are in the study
//!    area.
//! 2. **The service day** (`trips.txt`, `calendar.txt`, `calendar_dates.txt`):
//!    the date given, or else the **busiest weekday** — the Monday to Friday with
//!    the most trips in service, the earliest on a tie, within the first year of
//!    the feed's dates (the interface's default, `date`).
//! 3. **Runs** (`stop_times.txt`, streamed): the calls of every trip in service
//!    that day, in `stop_sequence` order. A blank time is interpolated between
//!    the timed calls around it, by `shape_dist_traveled` where the feed gives
//!    it and by straight-line distance otherwise; a trip whose first or last call
//!    has no time is dropped. Then each run keeps only its calls at stops in the
//!    area; a run left with fewer than two is dropped. A run that leaves the area
//!    and comes back keeps the calls on both sides, joined.
//! 4. **Transfers** (`transfers.txt`): stop-to-stop minimum transfer times
//!    (`transfer_type` 2, and 1 — timed — as zero) between two stops that are
//!    kept. Rows for particular routes or trips, rows between stations rather
//!    than stops, and `transfer_type` 0 and 3 are skipped and counted.
//!
//! **Not read in this version** (counted, so the report says what is missing):
//! `frequencies.txt` (headway-based trips are not expanded), trips of the
//! previous service day that run past midnight into this one, shapes, fares.
//!
//! **Cost:** the stops and every trip's route and service in memory, and the
//! calls of the day's trips (about 24 bytes each: 55 MB for the whole
//! Netherlands' 2.3 million); `stop_times.txt` is streamed. The national Dutch
//! feed (1.18 GB of stop times) reads in a few seconds.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use openmobisim_core_graph::geometry::{LonLat, haversine_metres};
use openmobisim_core_transit::{
    ALIGHT, BOARD, CallSpec, RouteSpec, ServiceDate, StopSpec, Timetable, TimetableBuilder,
    TimetableReport,
};

use crate::GtfsError;
use crate::csv::Csv;
use crate::source::FeedSource;

const NONE: u32 = u32::MAX;
/// How far past the feed's first date the busiest weekday is looked for.
const SEARCH_DAYS: i32 = 366;

/// What a read found, kept and skipped.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GtfsReport {
    /// The service day read.
    pub date: Option<ServiceDate>,
    /// Whether it was chosen as the busiest weekday (not given).
    pub date_chosen: bool,
    /// Stops and platforms in the feed with a position.
    pub stops_in_feed: u32,
    /// Of those, in the study area.
    pub stops_in_area: u32,
    /// Of those, called at by a run kept: the timetable's stops.
    pub stops_kept: u32,
    /// Trips in the feed.
    pub trips_in_feed: u32,
    /// Trips in service on the day.
    pub trips_on_date: u32,
    /// Runs kept: trips of the day with two or more calls in the area.
    pub runs_kept: u32,
    /// Rows of `stop_times.txt`.
    pub stop_time_rows: u64,
    /// Calls kept.
    pub calls_kept: u64,
    /// Calls whose time was interpolated.
    pub times_interpolated: u64,
    /// Trips of the day dropped because their first or last call has no time.
    pub trips_untimed: u32,
    /// Rows that could not be read (a bad number, an unknown stop or trip), skipped.
    pub rows_skipped: u64,
    /// Transfers kept.
    pub transfers_kept: u32,
    /// Transfers skipped (see the [module docs](self)).
    pub transfers_skipped: u32,
    /// Rows of `frequencies.txt`: headway-based trips, not expanded in this version.
    pub frequencies_ignored: u32,
    /// What building the timetable dropped or repaired.
    pub timetable: TimetableReport,
}

/// Read one service day of the feed at `path` (a zip or a folder), keeping the
/// stops for which `keep` is true. `date: None` reads the busiest weekday.
///
/// # Errors
///
/// [`GtfsError`]: the feed cannot be opened or read, a required file or column
/// is missing, or nothing runs on the day.
///
/// # Panics
///
/// As [`read_feed`].
pub fn read_gtfs(
    path: impl AsRef<Path>,
    date: Option<ServiceDate>,
    keep: &dyn Fn(LonLat) -> bool,
) -> Result<(Timetable, GtfsReport), GtfsError> {
    let mut source = FeedSource::open(path)?;
    read_feed(&mut source, date, keep)
}

/// [`read_gtfs`] from an open [`FeedSource`].
///
/// # Errors
///
/// As [`read_gtfs`].
///
/// # Panics
///
/// Panics if the feed has more than `u32::MAX` stops, routes, services or trips
/// in service on the day.
pub fn read_feed(
    source: &mut FeedSource,
    date: Option<ServiceDate>,
    keep: &dyn Fn(LonLat) -> bool,
) -> Result<(Timetable, GtfsReport), GtfsError> {
    let mut report = GtfsReport::default();
    let stops = read_stops(source, keep, &mut report)?;
    let routes = read_routes(source)?;

    // The trips' services, counted, so the busiest weekday can be found.
    let mut services = Services::default();
    {
        let mut csv = open(source, "trips.txt")?;
        let service = required(&csv, "trips.txt", "service_id")?;
        while csv.next()? {
            report.trips_in_feed += 1;
            let s = services.id(csv.field(service));
            services.trips[s as usize] += 1;
        }
    }
    services.read_calendars(source)?;
    let date = match date {
        Some(d) => d,
        None => {
            report.date_chosen = true;
            services.busiest_weekday().ok_or_else(|| {
                GtfsError::NoService("the feed has no service on any weekday".into())
            })?
        }
    };
    report.date = Some(date);
    let active = services.active_on(date);

    // The day's trips: their ids and routes.
    let mut trip_index: HashMap<String, u32> = HashMap::new();
    let mut trip_ids: Vec<String> = Vec::new();
    let mut trip_route: Vec<u32> = Vec::new();
    {
        let mut csv = open(source, "trips.txt")?;
        let (trip, route, service) = (
            required(&csv, "trips.txt", "trip_id")?,
            required(&csv, "trips.txt", "route_id")?,
            required(&csv, "trips.txt", "service_id")?,
        );
        while csv.next()? {
            let Some(s) = services.index.get(csv.field(service)) else { continue };
            if !active[*s as usize] {
                continue;
            }
            let Some(&r) = routes.index.get(csv.field(route)) else {
                report.rows_skipped += 1;
                continue;
            };
            let id = csv.field(trip).to_string();
            if trip_index.contains_key(&id) {
                report.rows_skipped += 1;
                continue;
            }
            trip_index.insert(id.clone(), u32::try_from(trip_ids.len()).expect("trips fit u32"));
            trip_ids.push(id);
            trip_route.push(r);
        }
    }
    report.trips_on_date = u32::try_from(trip_ids.len()).expect("trips fit u32");
    if trip_ids.is_empty() {
        return Err(GtfsError::NoService(format!("nothing runs on {date}")));
    }

    let rows = read_stop_times(source, &trip_index, &stops, &mut report)?;

    let mut builder = TimetableBuilder::new(date);
    let mut builder_stop = vec![NONE; stops.rows.len()];
    let mut builder_route = vec![NONE; routes.specs.len()];
    let mut calls: Vec<CallSpec> = Vec::new();
    let mut start = 0;
    while start < rows.len() {
        let trip = rows[start].trip;
        let mut end = start + 1;
        while end < rows.len() && rows[end].trip == trip {
            end += 1;
        }
        let mut run = rows[start..end].to_vec();
        start = end;
        if !fill_times(&mut run, &stops, &mut report) {
            report.trips_untimed += 1;
            continue;
        }
        run.retain(|row| stops.rows[row.stop as usize].in_area);
        if run.len() < 2 {
            continue;
        }
        calls.clear();
        for row in &run {
            // A stop joins the timetable when a run that is kept calls at it.
            let ix = &mut builder_stop[row.stop as usize];
            if *ix == NONE {
                *ix = builder.add_stop(stops.rows[row.stop as usize].spec.clone());
            }
            calls.push(CallSpec {
                stop: *ix,
                arrival: row.arrival,
                departure: row.departure,
                flags: row.flags,
            });
        }
        let r = trip_route[trip as usize] as usize;
        if builder_route[r] == NONE {
            builder_route[r] = builder.add_route(routes.specs[r].clone());
        }
        builder.add_run(trip_ids[trip as usize].clone(), builder_route[r], &calls);
        report.runs_kept += 1;
        report.calls_kept += calls.len() as u64;
    }
    report.stops_kept = u32::try_from(builder.stop_count()).expect("stops fit u32");

    read_transfers(source, &stops, &builder_stop, &mut builder, &mut report)?;
    if let Some(input) = source.file("frequencies.txt")? {
        let mut csv = Csv::new(input)?;
        while csv.next()? {
            report.frequencies_ignored += 1;
        }
    }

    let (timetable, built) = builder.build();
    report.timetable = built;
    Ok((timetable, report))
}

fn open<'a>(
    source: &'a mut FeedSource,
    name: &'static str,
) -> Result<Csv<Box<dyn Read + 'a>>, GtfsError> {
    let input = source.file(name)?.ok_or(GtfsError::MissingFile(name))?;
    Ok(Csv::new(input)?)
}

fn required<R: Read>(
    csv: &Csv<R>,
    file: &'static str,
    column: &'static str,
) -> Result<usize, GtfsError> {
    csv.column(column).ok_or(GtfsError::MissingColumn { file, column })
}

struct StopRow {
    spec: StopSpec,
    in_area: bool,
}

struct Stops {
    index: HashMap<String, u32>,
    rows: Vec<StopRow>,
}

fn read_stops(
    source: &mut FeedSource,
    keep: &dyn Fn(LonLat) -> bool,
    report: &mut GtfsReport,
) -> Result<Stops, GtfsError> {
    let mut csv = open(source, "stops.txt")?;
    let id = required(&csv, "stops.txt", "stop_id")?;
    let (lat, lon) =
        (required(&csv, "stops.txt", "stop_lat")?, required(&csv, "stops.txt", "stop_lon")?);
    let (name, kind, parent) =
        (csv.column("stop_name"), csv.column("location_type"), csv.column("parent_station"));
    let mut stops = Stops { index: HashMap::new(), rows: Vec::new() };
    while csv.next()? {
        if !matches!(csv.get(kind), "" | "0") {
            continue;
        }
        let (Ok(y), Ok(x)) = (csv.field(lat).parse::<f64>(), csv.field(lon).parse::<f64>()) else {
            report.rows_skipped += 1;
            continue;
        };
        if !(x.is_finite()
            && y.is_finite()
            && (-180.0..=180.0).contains(&x)
            && (-90.0..=90.0).contains(&y))
        {
            report.rows_skipped += 1;
            continue;
        }
        let external = csv.field(id).to_string();
        if external.is_empty() || stops.index.contains_key(&external) {
            report.rows_skipped += 1;
            continue;
        }
        let position = LonLat::new(x, y);
        let in_area = keep(position);
        report.stops_in_feed += 1;
        report.stops_in_area += u32::from(in_area);
        let parent = csv.get(parent);
        stops
            .index
            .insert(external.clone(), u32::try_from(stops.rows.len()).expect("stops fit u32"));
        stops.rows.push(StopRow {
            spec: StopSpec {
                external_id: external,
                name: csv.get(name).to_string(),
                position,
                parent: (!parent.is_empty()).then(|| parent.to_string()),
            },
            in_area,
        });
    }
    Ok(stops)
}

struct Routes {
    index: HashMap<String, u32>,
    specs: Vec<RouteSpec>,
}

fn read_routes(source: &mut FeedSource) -> Result<Routes, GtfsError> {
    let mut csv = open(source, "routes.txt")?;
    let id = required(&csv, "routes.txt", "route_id")?;
    let kind = required(&csv, "routes.txt", "route_type")?;
    let (short, long) = (csv.column("route_short_name"), csv.column("route_long_name"));
    let mut routes = Routes { index: HashMap::new(), specs: Vec::new() };
    while csv.next()? {
        let external = csv.field(id).to_string();
        if routes.index.contains_key(&external) {
            continue;
        }
        let short_name = match csv.get(short) {
            "" => csv.get(long),
            s => s,
        };
        let route_type = csv.field(kind).parse::<u16>().unwrap_or(u16::MAX);
        routes
            .index
            .insert(external.clone(), u32::try_from(routes.specs.len()).expect("routes fit u32"));
        routes.specs.push(RouteSpec {
            external_id: external,
            short_name: short_name.to_string(),
            route_type,
        });
    }
    Ok(routes)
}

#[derive(Clone, Copy)]
struct Calendar {
    days: [bool; 7],
    start: i32,
    end: i32,
}

#[derive(Default)]
struct Services {
    index: HashMap<String, u32>,
    trips: Vec<u64>,
    calendar: Vec<Option<Calendar>>,
    /// `(service, day) -> added (true) or removed (false)`; the last row wins.
    exceptions: HashMap<(u32, i32), bool>,
}

impl Services {
    fn id(&mut self, name: &str) -> u32 {
        if let Some(&s) = self.index.get(name) {
            return s;
        }
        let s = u32::try_from(self.trips.len()).expect("services fit u32");
        self.index.insert(name.to_string(), s);
        self.trips.push(0);
        self.calendar.push(None);
        s
    }

    fn read_calendars(&mut self, source: &mut FeedSource) -> Result<(), GtfsError> {
        if let Some(input) = source.file("calendar.txt")? {
            let mut csv = Csv::new(input)?;
            let service = required(&csv, "calendar.txt", "service_id")?;
            let (start, end) = (
                required(&csv, "calendar.txt", "start_date")?,
                required(&csv, "calendar.txt", "end_date")?,
            );
            let days: Vec<Option<usize>> = openmobisim_core_transit::Weekday::ALL
                .iter()
                .map(|d| csv.column(d.as_str()))
                .collect();
            while csv.next()? {
                let (Some(a), Some(b)) =
                    (ServiceDate::parse(csv.field(start)), ServiceDate::parse(csv.field(end)))
                else {
                    continue;
                };
                let mut flags = [false; 7];
                for (flag, column) in flags.iter_mut().zip(&days) {
                    *flag = csv.get(*column) == "1";
                }
                let s = self.id(csv.field(service));
                self.calendar[s as usize] =
                    Some(Calendar { days: flags, start: a.day_number(), end: b.day_number() });
            }
        }
        if let Some(input) = source.file("calendar_dates.txt")? {
            let mut csv = Csv::new(input)?;
            let service = required(&csv, "calendar_dates.txt", "service_id")?;
            let (date, kind) = (
                required(&csv, "calendar_dates.txt", "date")?,
                required(&csv, "calendar_dates.txt", "exception_type")?,
            );
            while csv.next()? {
                let Some(d) = ServiceDate::parse(csv.field(date)) else { continue };
                let added = match csv.field(kind) {
                    "1" => true,
                    "2" => false,
                    _ => continue,
                };
                let s = self.id(csv.field(service));
                self.exceptions.insert((s, d.day_number()), added);
            }
        }
        Ok(())
    }

    fn by_calendar(&self, s: usize, day: i32) -> bool {
        self.calendar[s].is_some_and(|c| {
            c.start <= day
                && day <= c.end
                && c.days[ServiceDate::from_day_number(day).weekday() as usize]
        })
    }

    fn active_on(&self, date: ServiceDate) -> Vec<bool> {
        let day = date.day_number();
        (0..self.trips.len())
            .map(|s| {
                let s32 = u32::try_from(s).expect("services fit u32");
                self.exceptions
                    .get(&(s32, day))
                    .copied()
                    .unwrap_or_else(|| self.by_calendar(s, day))
            })
            .collect()
    }

    fn busiest_weekday(&self) -> Option<ServiceDate> {
        let first = self
            .calendar
            .iter()
            .flatten()
            .map(|c| c.start)
            .chain(self.exceptions.keys().map(|k| k.1))
            .min()?;
        let last = self
            .calendar
            .iter()
            .flatten()
            .map(|c| c.end)
            .chain(self.exceptions.keys().map(|k| k.1))
            .max()?
            .min(first + SEARCH_DAYS);
        if last < first {
            return None;
        }
        let span = usize::try_from(last - first + 1).expect("a year of days");
        let mut count = vec![0i64; span];
        for (s, c) in self.calendar.iter().enumerate() {
            let Some(c) = c else { continue };
            let n = i64::try_from(self.trips[s]).expect("trip counts fit i64");
            for day in c.start.max(first)..=c.end.min(last) {
                if c.days[ServiceDate::from_day_number(day).weekday() as usize] {
                    count[usize::try_from(day - first).expect("in the span")] += n;
                }
            }
        }
        for (&(s, day), &added) in &self.exceptions {
            if day < first || day > last {
                continue;
            }
            let n = i64::try_from(self.trips[s as usize]).expect("trip counts fit i64");
            let base = self.by_calendar(s as usize, day);
            let slot = &mut count[usize::try_from(day - first).expect("in the span")];
            match (added, base) {
                (true, false) => *slot += n,
                (false, true) => *slot -= n,
                _ => {}
            }
        }
        (0..span)
            .map(|i| ServiceDate::from_day_number(first + i32::try_from(i).expect("in the span")))
            .filter(|d| d.weekday().is_weekday())
            .map(|d| (count[usize::try_from(d.day_number() - first).expect("in the span")], d))
            .filter(|&(n, _)| n > 0)
            // The most trips; on a tie, the earliest date.
            .max_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)))
            .map(|(_, d)| d)
    }
}

/// One row of `stop_times.txt` of a trip in service.
#[derive(Clone, Copy)]
struct Row {
    trip: u32,
    sequence: u32,
    stop: u32,
    arrival: u32,
    departure: u32,
    distance: f32,
    flags: u8,
}

const UNSET: u32 = u32::MAX;

fn read_stop_times(
    source: &mut FeedSource,
    trips: &HashMap<String, u32>,
    stops: &Stops,
    report: &mut GtfsReport,
) -> Result<Vec<Row>, GtfsError> {
    let mut csv = open(source, "stop_times.txt")?;
    let f = "stop_times.txt";
    let (trip, stop, sequence) = (
        required(&csv, f, "trip_id")?,
        required(&csv, f, "stop_id")?,
        required(&csv, f, "stop_sequence")?,
    );
    let (arrival, departure) =
        (required(&csv, f, "arrival_time")?, required(&csv, f, "departure_time")?);
    let (pickup, drop_off, distance) =
        (csv.column("pickup_type"), csv.column("drop_off_type"), csv.column("shape_dist_traveled"));
    let mut rows = Vec::new();
    while csv.next()? {
        report.stop_time_rows += 1;
        let Some(&t) = trips.get(csv.field(trip)) else { continue };
        let (Some(&s), Ok(seq)) =
            (stops.index.get(csv.field(stop)), csv.field(sequence).parse::<u32>())
        else {
            report.rows_skipped += 1;
            continue;
        };
        let (Some(a), Some(d)) = (parse_time(csv.field(arrival)), parse_time(csv.field(departure)))
        else {
            report.rows_skipped += 1;
            continue;
        };
        let mut flags = BOARD | ALIGHT;
        if csv.get(pickup) == "1" {
            flags &= !BOARD;
        }
        if csv.get(drop_off) == "1" {
            flags &= !ALIGHT;
        }
        #[allow(
            clippy::cast_possible_truncation,
            reason = "a distance along a route, far inside f32"
        )]
        let dist = csv.get(distance).parse::<f64>().map_or(f32::NAN, |v| v as f32);
        rows.push(Row {
            trip: t,
            sequence: seq,
            stop: s,
            arrival: a,
            departure: d,
            distance: dist,
            flags,
        });
    }
    rows.sort_unstable_by_key(|r| (r.trip, r.sequence));
    Ok(rows)
}

/// `H:MM:SS`, hours past 24 allowed: `Some(UNSET)` for a blank, `None` if not a time.
fn parse_time(text: &str) -> Option<u32> {
    if text.is_empty() {
        return Some(UNSET);
    }
    let mut parts = text.split(':');
    let (h, m, s) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() || m.len() != 2 || s.len() != 2 {
        return None;
    }
    let (h, m, s): (u32, u32, u32) = (h.trim().parse().ok()?, m.parse().ok()?, s.parse().ok()?);
    (m < 60 && s < 60).then(|| h * 3600 + m * 60 + s)
}

/// Complete a run's times: a missing arrival or departure is the other; a call
/// with neither is interpolated between the timed calls around it. `false` if
/// the first or last call has no time.
fn fill_times(run: &mut [Row], stops: &Stops, report: &mut GtfsReport) -> bool {
    for r in run.iter_mut() {
        match (r.arrival, r.departure) {
            (UNSET, d) if d != UNSET => r.arrival = d,
            (a, UNSET) if a != UNSET => r.departure = a,
            _ => {}
        }
    }
    let (first, last) = (run[0], run[run.len() - 1]);
    if first.arrival == UNSET || last.arrival == UNSET {
        return false;
    }
    let mut i = 0;
    while i < run.len() {
        if run[i].arrival != UNSET {
            i += 1;
            continue;
        }
        let a = i - 1;
        let mut b = i;
        while run[b].arrival == UNSET {
            b += 1;
        }
        // Distance along the run from call `a`: shape distances if every call has one.
        let by_shape = run[a..=b].iter().all(|r| r.distance.is_finite());
        let mut along = vec![0.0f64; b - a + 1];
        for k in a + 1..=b {
            let step = if by_shape {
                f64::from(run[k].distance - run[k - 1].distance).max(0.0)
            } else {
                let (p, q) = (
                    stops.rows[run[k - 1].stop as usize].spec.position,
                    stops.rows[run[k].stop as usize].spec.position,
                );
                haversine_metres(p, q)
            };
            along[k - a] = along[k - a - 1] + step;
        }
        let (t0, t1) = (f64::from(run[a].departure), f64::from(run[b].arrival));
        let total = along[b - a];
        for k in a + 1..b {
            #[allow(clippy::cast_precision_loss, reason = "a handful of calls")]
            let share =
                if total > 0.0 { along[k - a] / total } else { (k - a) as f64 / (b - a) as f64 };
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "between two u32 times, so a u32 time"
            )]
            let t = (t0 + (t1 - t0).max(0.0) * share).round() as u32;
            run[k].arrival = t;
            run[k].departure = t;
            report.times_interpolated += 1;
        }
        i = b + 1;
    }
    true
}

fn read_transfers(
    source: &mut FeedSource,
    stops: &Stops,
    builder_stop: &[u32],
    builder: &mut TimetableBuilder,
    report: &mut GtfsReport,
) -> Result<(), GtfsError> {
    let Some(input) = source.file("transfers.txt")? else { return Ok(()) };
    let mut csv = Csv::new(input)?;
    let f = "transfers.txt";
    let (from, to, kind) = (
        required(&csv, f, "from_stop_id")?,
        required(&csv, f, "to_stop_id")?,
        required(&csv, f, "transfer_type")?,
    );
    let seconds = csv.column("min_transfer_time");
    let specific: Vec<Option<usize>> =
        ["from_route_id", "to_route_id", "from_trip_id", "to_trip_id"]
            .iter()
            .map(|c| csv.column(c))
            .collect();
    while csv.next()? {
        let min = match csv.field(kind) {
            "2" => csv.get(seconds).parse::<u32>().ok(),
            "1" => Some(0),
            _ => None,
        };
        let general = specific.iter().all(|c| csv.get(*c).is_empty());
        let stop = |name: &str| {
            stops.index.get(name).map(|&s| builder_stop[s as usize]).filter(|&b| b != NONE)
        };
        match (min, general, stop(csv.field(from)), stop(csv.field(to))) {
            (Some(min), true, Some(a), Some(b)) => {
                builder.add_transfer(a, b, min);
                report.transfers_kept += 1;
            }
            _ => report.transfers_skipped += 1,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times() {
        assert_eq!(parse_time("08:05:09"), Some(8 * 3600 + 5 * 60 + 9));
        assert_eq!(parse_time("8:05:09"), Some(8 * 3600 + 5 * 60 + 9));
        assert_eq!(parse_time("25:00:00"), Some(25 * 3600), "past midnight");
        assert_eq!(parse_time(""), Some(UNSET));
        for bad in ["8:5:09", "08:60:00", "x", "08:00", "08:00:00:00"] {
            assert_eq!(parse_time(bad), None, "{bad:?}");
        }
    }
}
