//! Stops and runs: the feed's stops in the network's plane, and the runs of the service day
//! and the night runs of the day before.
//!
//! - **Stops** (`stops.txt`): every row with `location_type` 0 or blank, projected with the
//!   network's projection; rows of `location_type` 1 to 4 (stations, entrances, nodes, boarding
//!   areas) are not stops.
//! - **Refusals.** A row of `stops.txt`, `trips.txt` or `stop_times.txt` that cannot be read is
//!   an error naming the file, the row's stop or trip and the value: a position, time,
//!   sequence, distance or `location_type` that is not one, an empty or repeated stop id, a
//!   repeated trip id, a call of a trip that `trips.txt` does not hold, a call at a stop that is
//!   not a stop with a position. Every row of
//!   `stop_times.txt` is read, whichever day its trip runs.
//! - **Trips** (`trips.txt`): those whose service runs on the day, and, for the night runs,
//!   those whose service runs on the day before.
//! - **Calls** (`stop_times.txt`, streamed): each trip's rows in `stop_sequence` order; times
//!   `H:MM:SS` as seconds after the day's midnight, hours past 24 kept as such (`25:10:00` is
//!   90 600 s). A blank arrival or departure is the other one; a call with neither is
//!   interpolated between the timed calls around it by `shape_dist_traveled` when every call
//!   between them has one, else by straight-line distance in the plane, rounded to the second.
//!   A trip whose first or last call has no time is dropped and counted; so is a trip of one
//!   call. `pickup_type` and `drop_off_type` 1 are counted and ignored: the transit crate's
//!   call has no such flag.
//! - **Frequencies** (`frequencies.txt`): a trip listed there is a template, its times read
//!   from its first departure; it runs from each row's `start_time` every `headway_secs` while
//!   before its `end_time`, one run each, named `<trip_id>@<seconds>`; the template is not a
//!   run.
//! - **Night runs**: each run of the day before keeps its calls that depart at or after
//!   24:00:00 of that day, shifted by −86 400 s (an arrival before 24:00:00 becomes 0); with two
//!   calls or more it is a run of the day, named `<its name>@previous-day`.

use std::collections::{HashMap, HashSet};

use openmobisim_graph::geometry::{LonLat, Projection, Xy};
use openmobisim_transit::timetable::Call;
use openmobisim_types::time::Second;

use crate::calendar::Services;
use crate::source::Feed;
use crate::{GtfsError, Report, required};

/// A blank time.
const UNSET: u32 = u32::MAX;
/// The seconds of a day.
const DAY: u32 = 86_400;

/// The feed's stops: external ids and positions in the network's plane, by index.
pub(crate) struct Stops {
    /// Stop id to index.
    pub index: HashMap<String, u32>,
    /// The external ids.
    pub ids: Vec<String>,
    /// The positions, metres.
    pub xy: Vec<Xy>,
}

/// Read every stop or platform with a position (see the module documentation).
pub(crate) fn read_stops(
    feed: &mut Feed,
    projection: &Projection,
    report: &mut Report,
) -> Result<Stops, GtfsError> {
    let f = "stops.txt";
    let mut csv = feed.csv(f)?;
    let id = required(&csv, f, "stop_id")?;
    let (lat, lon) = (required(&csv, f, "stop_lat")?, required(&csv, f, "stop_lon")?);
    let kind = csv.column("location_type");
    let mut stops = Stops { index: HashMap::new(), ids: Vec::new(), xy: Vec::new() };
    while csv.next()? {
        let external = csv.field(id);
        let bad = |column, value: &str| GtfsError::BadValue {
            file: f,
            id: external.to_owned(),
            column,
            value: value.to_owned(),
        };
        match csv.get(kind) {
            "" | "0" => {}
            "1" | "2" | "3" | "4" => continue,
            v => return Err(bad("location_type", v)),
        }
        if external.is_empty() {
            return Err(bad("stop_id", external));
        }
        // Within the projection's range: longitude ±180°, latitude ±84° (the UTM system).
        let degrees = |column, text: &str, limit: f64| {
            let v = text.parse::<f64>().ok().filter(|v| v.is_finite() && v.abs() <= limit);
            v.ok_or_else(|| bad(column, text))
        };
        let x = degrees("stop_lon", csv.field(lon), 180.0)?;
        let y = degrees("stop_lat", csv.field(lat), 84.0)?;
        let position = LonLat::new(x, y);
        if stops.index.contains_key(external) {
            return Err(GtfsError::RepeatedId { file: f, id: external.to_owned() });
        }
        let s = u32::try_from(stops.ids.len()).expect("stops fit u32");
        stops.index.insert(external.to_owned(), s);
        stops.ids.push(external.to_owned());
        stops.xy.push(projection.project(position));
    }
    report.stops = u32::try_from(stops.ids.len()).expect("stops fit u32");
    Ok(stops)
}

/// The trips read: those of the day and those of the day before, by index; and every trip id
/// of the feed.
struct Trips {
    all: HashSet<String>,
    index: HashMap<String, u32>,
    ids: Vec<String>,
    today: Vec<bool>,
    yesterday: Vec<bool>,
}

fn read_trips(
    feed: &mut Feed,
    services: &Services,
    day: i64,
    report: &mut Report,
) -> Result<Trips, GtfsError> {
    let (on_day, on_eve) = (services.active_on(day), services.active_on(day - 1));
    let f = "trips.txt";
    let mut csv = feed.csv(f)?;
    let (trip, service) = (required(&csv, f, "trip_id")?, required(&csv, f, "service_id")?);
    let mut trips = Trips {
        all: HashSet::new(),
        index: HashMap::new(),
        ids: Vec::new(),
        today: Vec::new(),
        yesterday: Vec::new(),
    };
    while csv.next()? {
        let id = csv.field(trip);
        if !trips.all.insert(id.to_owned()) {
            return Err(GtfsError::RepeatedId { file: f, id: id.to_owned() });
        }
        let s = services.index[csv.field(service)] as usize;
        if !on_day[s] && !on_eve[s] {
            continue;
        }
        trips.index.insert(id.to_owned(), u32::try_from(trips.ids.len()).expect("trips fit u32"));
        trips.ids.push(id.to_owned());
        trips.today.push(on_day[s]);
        trips.yesterday.push(on_eve[s]);
        report.trips += u32::from(on_day[s]);
    }
    Ok(trips)
}

/// Per trip, its frequency periods: `(start, end, headway)` in seconds, by start.
type Periods = HashMap<u32, Vec<(u32, u32, u32)>>;

/// `frequencies.txt`'s rows for the trips read. A row with a bad time, a headway not above 0
/// or an end not after its start does not apply: skipped and counted.
fn read_frequencies(
    feed: &mut Feed,
    trips: &Trips,
    report: &mut Report,
) -> Result<Periods, GtfsError> {
    let mut out: Periods = HashMap::new();
    let f = "frequencies.txt";
    let Some(mut csv) = feed.optional_csv(f)? else { return Ok(out) };
    let (trip, start) = (required(&csv, f, "trip_id")?, required(&csv, f, "start_time")?);
    let (end, headway) = (required(&csv, f, "end_time")?, required(&csv, f, "headway_secs")?);
    while csv.next()? {
        let Some(&t) = trips.index.get(csv.field(trip)) else { continue };
        let times = (parse_time(csv.field(start)), parse_time(csv.field(end)));
        match (times, csv.field(headway).parse::<u32>()) {
            ((Some(a), Some(b)), Ok(h)) if a != UNSET && b != UNSET && b > a && h > 0 => {
                out.entry(t).or_default().push((a, b, h));
            }
            _ => report.frequency_rows_skipped += 1,
        }
    }
    for periods in out.values_mut() {
        periods.sort_unstable();
    }
    Ok(out)
}

/// One row of `stop_times.txt` of a trip read.
#[derive(Clone, Copy)]
struct Row {
    trip: u32,
    sequence: u32,
    stop: u32,
    arrival: u32,
    departure: u32,
    /// `shape_dist_traveled`, or NaN.
    distance: f64,
}

fn read_stop_times(
    feed: &mut Feed,
    trips: &Trips,
    stops: &Stops,
    report: &mut Report,
) -> Result<Vec<Row>, GtfsError> {
    let f = "stop_times.txt";
    let mut csv = feed.csv(f)?;
    let (trip, stop) = (required(&csv, f, "trip_id")?, required(&csv, f, "stop_id")?);
    let sequence = required(&csv, f, "stop_sequence")?;
    let (arrival, departure) =
        (required(&csv, f, "arrival_time")?, required(&csv, f, "departure_time")?);
    let (pickup, drop_off) = (csv.column("pickup_type"), csv.column("drop_off_type"));
    let distance = csv.column("shape_dist_traveled");
    let mut rows = Vec::new();
    while csv.next()? {
        // Every row is read, whichever day its trip runs: a bad row is an error, not a skip.
        let id = csv.field(trip);
        let bad = |column, value: &str| GtfsError::BadValue {
            file: f,
            id: id.to_owned(),
            column,
            value: value.to_owned(),
        };
        if !trips.all.contains(id) {
            let (trip, stop_sequence) = (id.to_owned(), csv.field(sequence).to_owned());
            return Err(GtfsError::UnknownTrip { trip, stop_sequence });
        }
        let Some(&s) = stops.index.get(csv.field(stop)) else {
            return Err(GtfsError::UnknownStop {
                trip: id.to_owned(),
                stop: csv.field(stop).to_owned(),
            });
        };
        let seq = csv.field(sequence);
        let seq = seq.parse::<u32>().map_err(|_| bad("stop_sequence", seq))?;
        let time = |column, c| parse_time(csv.field(c)).ok_or_else(|| bad(column, csv.field(c)));
        let (a, d) = (time("arrival_time", arrival)?, time("departure_time", departure)?);
        let distance = match csv.get(distance) {
            "" => f64::NAN,
            v => v
                .parse::<f64>()
                .ok()
                .filter(|v| v.is_finite())
                .ok_or_else(|| bad("shape_dist_traveled", v))?,
        };
        let Some(&t) = trips.index.get(id) else { continue };
        if csv.get(pickup) == "1" || csv.get(drop_off) == "1" {
            report.pickup_drop_off_ignored += 1;
        }
        rows.push(Row { trip: t, sequence: seq, stop: s, arrival: a, departure: d, distance });
    }
    rows.sort_by_key(|r| (r.trip, r.sequence));
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
    let (h, m, s): (u32, u32, u32) = (h.parse().ok()?, m.parse().ok()?, s.parse().ok()?);
    if m >= 60 || s >= 60 {
        return None;
    }
    // An hour too large for the seconds to fit (or to tell from a blank) is not a time.
    let seconds = h.checked_mul(3600)?.checked_add(m * 60 + s)?;
    (seconds != UNSET).then_some(seconds)
}

/// Complete a trip's times: a missing arrival or departure is the other; a call with neither
/// is interpolated between the timed calls around it. `false` if the first or last call has
/// no time.
fn fill_times(run: &mut [Row], xy: &[Xy], report: &mut Report) -> bool {
    for r in run.iter_mut() {
        match (r.arrival, r.departure) {
            (UNSET, d) if d != UNSET => r.arrival = d,
            (a, UNSET) if a != UNSET => r.departure = a,
            _ => {}
        }
    }
    if run[0].arrival == UNSET || run[run.len() - 1].arrival == UNSET {
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
        let mut along = vec![0.0_f64; b - a + 1];
        for k in a + 1..=b {
            let step = if by_shape {
                (run[k].distance - run[k - 1].distance).max(0.0)
            } else {
                xy[run[k - 1].stop as usize].distance_to(xy[run[k].stop as usize])
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

/// A run being made: its name and its calls (stop index, arrival, departure).
type Draft = (String, Vec<(u32, u32, u32)>);

/// The runs a trip makes on its own service day: itself, or one per headway of its periods.
fn expand(name: &str, run: &[Row], periods: Option<&Vec<(u32, u32, u32)>>) -> Vec<Draft> {
    let calls: Vec<(u32, u32, u32)> =
        run.iter().map(|r| (r.stop, r.arrival, r.departure)).collect();
    let Some(periods) = periods else { return vec![(name.to_owned(), calls)] };
    let first = calls[0].2;
    let mut out = Vec::new();
    for &(start, end, headway) in periods {
        let mut t = start;
        while t < end {
            // Add before subtracting: a first stop's arrival can be before the first departure.
            let at = |s: u32| s.saturating_add(t).saturating_sub(first);
            out.push((
                format!("{name}@{t}"),
                calls.iter().map(|&(s, a, d)| (s, at(a), at(d))).collect(),
            ));
            t = t.saturating_add(headway);
        }
    }
    out
}

/// A run of the day before as a night run of the day: its calls departing at or after
/// 24:00:00, shifted by a day; `None` with fewer than two.
fn night((name, calls): Draft) -> Option<Draft> {
    let kept: Vec<_> = calls
        .into_iter()
        .filter(|c| c.2 >= DAY)
        .map(|(s, a, d)| (s, a.saturating_sub(DAY), d - DAY))
        .collect();
    (kept.len() >= 2).then(|| (format!("{name}@previous-day"), kept))
}

/// The runs of the busiest weekday `day` and the night runs of the day before, each as its
/// external id and calls, in the feed's trip order.
pub(crate) fn read_runs(
    feed: &mut Feed,
    services: &Services,
    day: i64,
    stops: &Stops,
    report: &mut Report,
) -> Result<Vec<(String, Vec<Call>)>, GtfsError> {
    let trips = read_trips(feed, services, day, report)?;
    let frequencies = read_frequencies(feed, &trips, report)?;
    let mut rows = read_stop_times(feed, &trips, stops, report)?;
    let mut out = Vec::new();
    let mut push = |(name, calls): Draft, report: &mut Report| {
        report.runs += 1;
        report.calls += calls.len() as u64;
        let to_call = |&(s, a, d): &(u32, u32, u32)| Call {
            stop: stops.ids[s as usize].clone(),
            arrival: Second(a),
            departure: Second(d),
        };
        out.push((name, calls.iter().map(to_call).collect()));
    };
    for run in rows.chunk_by_mut(|a, b| a.trip == b.trip) {
        let t = run[0].trip as usize;
        if !fill_times(run, &stops.xy, report) {
            report.trips_untimed += 1;
            continue;
        }
        if run.len() < 2 {
            report.trips_single_call += 1;
            continue;
        }
        let periods = frequencies.get(&run[0].trip);
        let drafts = expand(&trips.ids[t], run, periods);
        if trips.yesterday[t] {
            for draft in drafts.iter().cloned().filter_map(night) {
                report.night_runs += 1;
                push(draft, report);
            }
        }
        if trips.today[t] {
            for draft in drafts {
                report.frequency_runs += u32::from(periods.is_some());
                push(draft, report);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times() {
        assert_eq!(parse_time("08:05:09"), Some(8 * 3600 + 5 * 60 + 9));
        assert_eq!(parse_time("8:05:09"), Some(8 * 3600 + 5 * 60 + 9));
        assert_eq!(parse_time("25:10:00"), Some(90_600), "past midnight, kept as such");
        assert_eq!(parse_time(""), Some(UNSET));
        for bad in ["8:5:09", "08:60:00", "x", "08:00", "08:00:00:00", "1193047:00:00"] {
            assert_eq!(parse_time(bad), None, "{bad:?}");
        }
        assert_eq!(parse_time("1193046:28:14"), Some(UNSET - 1), "the largest time");
        assert_eq!(parse_time("1193046:28:15"), None, "the blank's own value is not a time");
    }
}
