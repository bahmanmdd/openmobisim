//! Hand-written feeds, each value derived from the GTFS reference (gtfs.org, "Reference",
//! revised 2024): the service day from `calendar.txt` and `calendar_dates.txt`, times past
//! midnight, interpolated times, frequencies, the night runs of the day before, what is counted
//! and skipped, and the refusals.

mod common;

use common::{CALENDAR, STOP_TIMES, STOPS, calls, hms, read, run, street, zip};
use openmobisim_graph::geometry::{Position, Xy};
use openmobisim_graph::network::{LinkRow, NetworkBuilder};
use openmobisim_io_gtfs::{GtfsError, Report};
use openmobisim_types::mode::{Mode, ModeSet};
use openmobisim_types::units::Metres;

/// Trips `ids` on service `service`, each A 08:00 to B 08:10.
fn trips_and_times(rows: &[(&str, &str)]) -> (String, String) {
    let mut trips = String::from("route_id,service_id,trip_id\n");
    let mut times = String::from(STOP_TIMES);
    for (service, trip) in rows {
        trips.push_str(&format!("R,{service},{trip}\n"));
        times.push_str(&format!("{trip},08:00:00,08:00:00,A,1\n{trip},08:10:00,08:10:00,B,2\n"));
    }
    (trips, times)
}

/// Weekday service WK in October 2026; EXTRA added on Wednesday the 7th; WK removed on Friday
/// the 9th; weekend service WE. By the reference, the 7th has WK's three trips and EXTRA's one
/// (four), every other weekday three, the 9th none: the busiest weekday is 2026-10-07.
#[test]
fn the_busiest_weekday_counts_the_calendar_and_its_exceptions() {
    let (trips, times) = trips_and_times(&[
        ("WK", "T1"),
        ("WK", "T2"),
        ("WK", "T3"),
        ("WE", "W1"),
        ("WE", "W2"),
        ("WE", "W3"),
        ("WE", "W4"),
        ("WE", "W5"),
        ("EXTRA", "X1"),
    ]);
    let calendar = format!(
        "{CALENDAR}WK,1,1,1,1,1,0,0,20261001,20261031\nWE,0,0,0,0,0,1,1,20261001,20261031\n"
    );
    let dates = "service_id,date,exception_type\nEXTRA,20261007,1\nWK,20261009,2\n";
    let files = [
        ("stops.txt", STOPS),
        ("trips.txt", trips.as_str()),
        ("calendar.txt", calendar.as_str()),
        ("calendar_dates.txt", dates),
        ("stop_times.txt", times.as_str()),
    ];
    let (t, report) = read("busiest", &files).unwrap();
    assert_eq!(report.date, 20_261_007);
    assert_eq!((report.trips, report.runs, report.calls), (4, 4, 8));
    for id in ["T1", "T2", "T3", "X1"] {
        assert!(run(&t, id).is_some(), "{id}");
    }
    assert!(run(&t, "W1").is_none(), "a weekend trip");
    assert_eq!(report.stops, 4);
    assert_eq!(t.stop_count(), 4, "every stop with a position, called at or not");

    // Without the extra service, every weekday of October has three: the earliest, Thursday
    // the 1st, is the busiest.
    let files = [files[0], files[1], files[2], files[4]];
    let (_, report) = read("tie", &files).unwrap();
    assert_eq!((report.date, report.trips), (20_261_001, 3));
}

/// Fixed bug: the busiest weekday was looked for only 366 days from the earliest start date.
/// OLD runs one trip every weekday from 2020 to 2030; NEW three more from Monday 2026-10-05.
/// By the reference, every weekday from 2026-10-05 has four trips and none before: the
/// busiest weekday is 2026-10-05.
#[test]
fn the_busiest_weekday_is_looked_for_over_the_whole_calendar() {
    let (trips, times) =
        trips_and_times(&[("OLD", "O1"), ("NEW", "N1"), ("NEW", "N2"), ("NEW", "N3")]);
    let calendar = format!(
        "{CALENDAR}OLD,1,1,1,1,1,0,0,20200101,20301231\nNEW,1,1,1,1,1,0,0,20261005,20301231\n"
    );
    let files = [
        ("stops.txt", STOPS),
        ("trips.txt", trips.as_str()),
        ("calendar.txt", calendar.as_str()),
        ("stop_times.txt", times.as_str()),
    ];
    let (_, report) = read("long", &files).unwrap();
    assert_eq!((report.date, report.trips), (20_261_005, 4));
}

/// A feed of exceptions alone (no `calendar.txt`): S1 runs on Friday 2026-10-09 and Saturday
/// the 10th, S2 on the 10th. Saturday has more trips but is no weekday: the 9th, one trip.
#[test]
fn a_feed_of_exceptions_alone() {
    let (trips, times) = trips_and_times(&[("S1", "A1"), ("S2", "B1"), ("S2", "B2")]);
    let dates = "service_id,date,exception_type\nS1,20261009,1\nS1,20261010,1\nS2,20261010,1\n";
    let files = [
        ("stops.txt", STOPS),
        ("trips.txt", trips.as_str()),
        ("calendar_dates.txt", dates),
        ("stop_times.txt", times.as_str()),
    ];
    let (t, report) = read("exceptions", &files).unwrap();
    assert_eq!((report.date, report.trips, report.runs), (20_261_009, 1, 1));
    assert!(run(&t, "A1").is_some());
}

const WEEKDAYS: &str = "service_id,monday,tuesday,wednesday,thursday,friday,saturday,sunday,start_date,end_date\n\
    WK,1,1,1,1,1,0,0,20261001,20261031\n";

/// Interpolation (reference, `stop_times.txt`: times between timepoints are interpolated):
/// by shape distance 0, 1 000, 3 000, 4 000 m over 08:00 to 08:10, the blank calls are at a
/// quarter and three quarters, 08:02:30 and 08:07:30; without shape distances, by straight
/// line between stops 0, 100, 400 and 1 000 m east, at a tenth and four tenths, 08:01:00 and
/// 08:04:00. A call past midnight, `25:10:00`, is 90 600 s, kept as such.
#[test]
fn blank_times_are_interpolated_and_times_past_midnight_kept() {
    let times = "trip_id,arrival_time,departure_time,stop_id,stop_sequence,shape_dist_traveled\n\
        S,08:00:00,08:00:00,A,1,0\nS,,,B,2,1000\nS,,,C,3,3000\nS,08:10:00,08:10:00,D,4,4000\n\
        L,08:00:00,08:00:00,A,1,\nL,,,B,2,\nL,,,C,3,\nL,08:10:00,08:10:00,D,4,\n\
        M,25:00:00,25:00:00,A,1,\nM,25:10:00,25:10:00,B,2,\n";
    let files = [
        ("stops.txt", STOPS),
        ("trips.txt", "route_id,service_id,trip_id\nR,WK,S\nR,WK,L\nR,WK,M\n"),
        ("calendar.txt", WEEKDAYS),
        ("stop_times.txt", times),
    ];
    let (t, report) = read("interpolation", &files).unwrap();
    let at = |id: &str| calls(&t, id).iter().map(|c| c.1).collect::<Vec<_>>();
    assert_eq!(at("S"), [hms(8, 0, 0), hms(8, 2, 30), hms(8, 7, 30), hms(8, 10, 0)]);
    assert_eq!(at("L"), [hms(8, 0, 0), hms(8, 1, 0), hms(8, 4, 0), hms(8, 10, 0)]);
    assert_eq!(calls(&t, "M")[1], ("B".to_owned(), 90_600, 90_600));
    assert_eq!(report.times_interpolated, 4);
}

/// Frequencies (reference, `frequencies.txt`): a template running 07:00 to 08:00 every
/// 1 200 s, then 08:00 to 08:30 every 900 s, starts at 07:00, 07:20, 07:40, 08:00 and 08:15:
/// five runs named `F@<seconds>`, each its template's times from its start; the template
/// itself is not a run. A row ending before it starts does not apply: skipped and counted.
#[test]
fn a_frequency_template_runs_once_per_headway() {
    let files = [
        ("stops.txt", STOPS),
        ("trips.txt", "route_id,service_id,trip_id\nR,WK,F\n"),
        ("calendar.txt", WEEKDAYS),
        (
            "stop_times.txt",
            "trip_id,arrival_time,departure_time,stop_id,stop_sequence\n\
            F,00:00:00,00:01:00,A,1\nF,00:10:00,00:10:00,B,2\n",
        ),
        (
            "frequencies.txt",
            "trip_id,start_time,end_time,headway_secs\n\
            F,07:00:00,08:00:00,1200\nF,08:00:00,08:30:00,900\nF,09:00:00,08:45:00,600\n",
        ),
    ];
    let (t, report) = read("frequencies", &files).unwrap();
    let starts = [hms(7, 0, 0), hms(7, 20, 0), hms(7, 40, 0), hms(8, 0, 0), hms(8, 15, 0)];
    assert_eq!((report.runs, report.frequency_runs, report.trips), (5, 5, 1));
    assert_eq!(report.frequency_rows_skipped, 1, "09:00 to 08:45 ends before it starts");
    assert!(run(&t, "F").is_none(), "the template is not a run");
    for s in starts {
        let got = calls(&t, &format!("F@{s}"));
        assert_eq!((got[0].1, got[0].2), (s - 60, s), "a minute's dwell at A before {s}");
        assert_eq!((got[1].1, got[1].2), (s + 540, s + 540), "nine minutes on to B");
    }
    assert!(run(&t, &format!("F@{}", hms(8, 30, 0))).is_none(), "08:30 is the end");
}

/// Fixed bug: the night runs of the day before were neither read nor counted. The busiest
/// weekday is Tuesday 2026-10-06 (WK's three trips and TUE's two; Monday has WK's three and
/// MON's one). On Monday, WK's trip N runs A 23:50, B 26:00, C 26:10 and MON's trip Y runs
/// A 24:30, B 24:40: on Tuesday they are night runs, `N@previous-day` B 7 200 s, C 7 800 s
/// (A, before midnight, is not of the day) and `Y@previous-day` A 1 800 s, B 2 400 s. N runs
/// on Tuesday too, its own times kept past midnight.
#[test]
fn the_night_runs_of_the_day_before_are_read_and_counted() {
    let trips = "route_id,service_id,trip_id\n\
        R,WK,N\nR,WK,D1\nR,WK,D2\nR,MON,Y\nR,TUE,Z1\nR,TUE,Z2\n";
    let times = "trip_id,arrival_time,departure_time,stop_id,stop_sequence,pickup_type\n\
        N,23:50:00,23:50:00,A,1,0\nN,26:00:00,26:00:00,B,2,0\nN,26:10:00,26:10:00,C,3,0\n\
        D1,08:00:00,08:00:00,A,1,0\nD1,08:10:00,08:10:00,B,2,1\n\
        D2,08:00:00,08:00:00,A,1,0\nD2,08:10:00,08:10:00,B,2,0\n\
        Y,24:30:00,24:30:00,A,1,0\nY,24:40:00,24:40:00,B,2,0\n\
        Z1,09:00:00,09:00:00,A,1,0\nZ1,09:10:00,09:10:00,B,2,0\n\
        Z2,09:00:00,09:00:00,B,1,0\nZ2,09:10:00,09:10:00,C,2,0\n";
    let calendar = format!("{CALENDAR}WK,1,1,1,1,1,0,0,20261005,20261009\n");
    let dates = "service_id,date,exception_type\nMON,20261005,1\nTUE,20261006,1\n";
    let files = [
        ("stops.txt", STOPS),
        ("trips.txt", trips),
        ("calendar.txt", calendar.as_str()),
        ("calendar_dates.txt", dates),
        ("stop_times.txt", times),
    ];
    let (t, report) = read("night", &files).unwrap();
    assert_eq!(report.date, 20_261_006);
    assert_eq!((report.trips, report.runs, report.night_runs), (5, 7, 2));
    assert_eq!(
        calls(&t, "N@previous-day"),
        [("B".into(), 7_200, 7_200), ("C".into(), 7_800, 7_800)]
    );
    assert_eq!(
        calls(&t, "Y@previous-day"),
        [("A".into(), 1_800, 1_800), ("B".into(), 2_400, 2_400)]
    );
    assert_eq!(calls(&t, "N")[2], ("C".into(), hms(26, 10, 0), hms(26, 10, 0)));
    assert!(run(&t, "Y").is_none() && run(&t, "D1@previous-day").is_none());
    // D1's pick-up flag is counted, and its call kept for boarding and alighting alike.
    assert_eq!(report.pickup_drop_off_ignored, 1);
    assert_eq!(calls(&t, "D1").len(), 2);
}

/// What is dropped is counted: a trip whose first call has no time, a trip of one call. A
/// station (`location_type` 1) is not a stop. The feed may sit in a folder inside its zip.
#[test]
fn the_dropped_trips_are_counted() {
    let stops =
        format!("{STOPS}P,Station,52.0,4.36,1\n").replace("stop_lon\n", "stop_lon,location_type\n");
    let times = format!(
        "{STOP_TIMES}U,,,A,1\nU,08:10:00,08:10:00,B,2\nO,08:00:00,08:00:00,A,1\n\
         K,08:00:00,08:00:00,A,1\nK,08:10:00,08:10:00,B,2\n"
    );
    let files = [
        ("stops.txt", stops.as_str()),
        ("trips.txt", "route_id,service_id,trip_id\nR,WK,U\nR,WK,O\nR,WK,K\n"),
        ("calendar.txt", WEEKDAYS),
        ("stop_times.txt", times.as_str()),
    ];
    let path = zip("dropped", "feed/", &files);
    let (t, report) =
        openmobisim_io_gtfs::read(&path, &street(&[0.0, 1_000.0]), Metres(400.0)).unwrap();
    std::fs::remove_file(path).ok();
    assert_eq!((report.trips_untimed, report.trips_single_call), (1, 1));
    assert_eq!(report.stops, 4, "A to D: the station is not a stop");
    assert_eq!(calls(&t, "K").len(), 2);
    assert_eq!((report.runs, report.trips), (1, 3));
}

const GOOD_TIMES: &str = "trip_id,arrival_time,departure_time,stop_id,stop_sequence\n\
    T1,08:00:00,08:00:00,A,1\nT1,08:10:00,08:10:00,B,2\n";

/// A good feed of one trip with `file`'s text replaced (or added) by `text`.
fn feed(file: &'static str, text: String) -> Vec<(&'static str, String)> {
    let mut files: Vec<(&'static str, String)> = vec![
        ("stops.txt", STOPS.to_owned()),
        ("trips.txt", "route_id,service_id,trip_id\nR,WK,T1\n".to_owned()),
        ("calendar.txt", WEEKDAYS.to_owned()),
        ("stop_times.txt", GOOD_TIMES.to_owned()),
    ];
    match files.iter_mut().find(|f| f.0 == file) {
        Some(f) => f.1 = text,
        None => files.push((file, text)),
    }
    files
}

fn times(rows: &str) -> String {
    format!("{GOOD_TIMES}{rows}")
}

/// Repairs as (key, count, first ids).
type Noted = Vec<(&'static str, u64, Vec<String>)>;

/// The feed `files` read, its report's repairs as (key, count, first ids), and its runs' calls.
fn repaired(name: &str, files: &[(&'static str, String)]) -> (Report, Noted) {
    let refs: Vec<(&str, &str)> = files.iter().map(|(n, t)| (*n, t.as_str())).collect();
    let (_, report) = read(name, &refs).expect("the feed is read");
    let kinds = report.repairs.kinds().iter().map(|k| (k.key, k.count, k.first.clone())).collect();
    (report, kinds)
}

fn one(key: &'static str, id: &str) -> Noted {
    vec![(key, 1, vec![id.to_owned()])]
}

/// Set right, noted, the read going on: in `stops.txt`, a position that is not a number or
/// outside longitude ±180° and latitude ±84°, an empty stop id, an undefined `location_type`
/// (the stop skipped); a repeated stop id (the first kept); in `trips.txt`, a repeated trip id
/// (the first row kept). The four stops A to D, one trip T1 from A to B.
#[test]
fn a_bad_row_of_stops_and_trips_is_set_right_and_noted() {
    for (i, (text, id)) in [
        (format!("{STOPS}X,Broken,north,4.36\n"), "X"),
        (format!("{STOPS}X,Far,52.0,190\n"), "X"),
        (format!("{STOPS}X,Pole,89.0,4.36\n"), "X"),
        (format!("{STOPS},Nameless,52.0,4.36\n"), ""),
    ]
    .into_iter()
    .enumerate()
    {
        let (report, kinds) = repaired(&format!("stops-{i}"), &feed("stops.txt", text));
        assert_eq!((report.stops, report.runs), (4, 1), "case {i}");
        assert_eq!(kinds, one("stops_skipped", id), "case {i}");
    }
    // A of an undefined location_type is skipped: T1's call there is dropped, and T1, left with
    // one call, is dropped as a trip of one call.
    let typed = STOPS
        .replace("stop_lon\n", "stop_lon,location_type\n")
        .replace("Alpha,52.0,4.36", "Alpha,52.0,4.36,x");
    let (report, kinds) = repaired("location", &feed("stops.txt", typed));
    assert_eq!((report.stops, report.runs, report.trips_single_call), (3, 0, 1));
    let keys: Vec<_> = kinds.iter().map(|k| (k.0, k.1)).collect();
    assert_eq!(keys, [("stops_skipped", 1), ("calls_dropped", 1)]);
    let (report, kinds) =
        repaired("stop-again", &feed("stops.txt", format!("{STOPS}A,Again,52.0,4.36\n")));
    assert_eq!((report.stops, kinds), (4, one("stops_repeated", "A")));
    let twice = "route_id,service_id,trip_id\nR,WK,T1\nR,WK,T1\n".into();
    let (report, kinds) = repaired("trip-again", &feed("trips.txt", twice));
    assert_eq!((report.trips, report.runs, kinds), (1, 1, one("trips_repeated", "T1")));
}

/// Set right, noted: in `calendar.txt`, a date that is not one (the row skipped), a weekday flag
/// other than 0 or 1 (read as 0), a service given twice (the first kept); in
/// `calendar_dates.txt`, an exception type other than 1 or 2 or a date that is not one (the row
/// skipped), a service's date given twice (the first kept). The day read stays the good feed's.
#[test]
fn a_bad_row_of_calendar_is_set_right_and_noted() {
    let (good, _) = repaired("calendar-good", &feed("calendar.txt", WEEKDAYS.to_owned()));
    for (i, (file, text, id)) in [
        (
            "calendar_dates.txt",
            "service_id,date,exception_type\nWK,20261009,2\nWK,20261009,1\n".to_owned(),
            "WK on 20261009",
        ),
        ("calendar.txt", format!("{WEEKDAYS}WE,0,0,0,0,0,1,1,20261001,20261331\n"), "WE"),
        ("calendar.txt", format!("{WEEKDAYS}WE,0,0,0,0,0,yes,1,20261001,20261031\n"), "WE"),
        ("calendar.txt", format!("{WEEKDAYS}WK,0,0,0,0,0,1,1,20261001,20261031\n"), "WK"),
        (
            "calendar_dates.txt",
            "service_id,date,exception_type\nWK,20261009,3\n".to_owned(),
            "WK on 20261009",
        ),
        (
            "calendar_dates.txt",
            "service_id,date,exception_type\nWK,2026-10-09,2\n".to_owned(),
            "WK on 2026-10-09",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let (report, kinds) = repaired(&format!("calendar-{i}"), &feed(file, text));
        assert_eq!(kinds, one("calendar_rows_skipped", id), "case {i}");
        assert_eq!((report.date, report.runs), (good.date, 1), "case {i}");
    }
}

/// Set right, noted: in `stop_times.txt`, a time that is not `H:MM:SS` (read as blank: the
/// other time taken), a sequence that is not a number or a stop not held (the call dropped,
/// whichever day the trip runs: T9 runs on no day), a trip `trips.txt` does not hold (the row
/// skipped), a shape distance that is not a number (read as blank). T1 calls at A (08:00), B
/// (08:10) and a third call C.
#[test]
fn a_bad_row_of_stop_times_is_set_right_and_noted() {
    let case = |i: usize, rows: &str| {
        repaired(&format!("stop-times-{i}"), &feed("stop_times.txt", times(rows)))
    };
    for (i, rows) in
        ["T1,99:99:99,08:20:00,C,3\n", "T1,08:20:00,8:2:00,C,3\n"].into_iter().enumerate()
    {
        let (report, kinds) = case(i, rows);
        assert_eq!((report.calls, kinds), (3, one("times_unread", "T1")), "{rows}");
    }
    for (i, rows) in
        ["T1,08:20:00,08:20:00,C,third\n", "T1,08:20:00,08:20:00,Q,3\n"].into_iter().enumerate()
    {
        let (report, kinds) = case(i + 2, rows);
        assert_eq!((report.calls, kinds), (2, one("calls_dropped", "T1")), "{rows}");
    }
    let mut files = feed("stop_times.txt", times("T9,08:20:00,08:20:00,Q,1\n"));
    files[1].1 = "route_id,service_id,trip_id\nR,WK,T1\nR,NONE,T9\n".into();
    assert_eq!(repaired("stop-times-4", &files).1, one("calls_dropped", "T9"));
    let (report, kinds) = case(5, "T7,08:20:00,08:20:00,C,5\n");
    assert_eq!((report.runs, kinds), (1, one("stop_times_skipped", "T7")));
    let shaped = "trip_id,arrival_time,departure_time,stop_id,stop_sequence,shape_dist_traveled\n\
        T1,08:00:00,08:00:00,A,1,0\nT1,08:10:00,08:10:00,B,2,1km\n";
    let (report, kinds) = repaired("stop-times-6", &feed("stop_times.txt", shaped.into()));
    assert_eq!((report.runs, kinds), (1, one("distances_unread", "T1")));
    // No arrival_time column: read as blank, the departures taken.
    let departures =
        "trip_id,departure_time,stop_id,stop_sequence\nT1,08:00:00,A,1\nT1,08:10:00,B,2\n";
    let (report, kinds) = repaired("stop-times-7", &feed("stop_times.txt", departures.into()));
    assert_eq!((report.runs, kinds), (1, one("columns_absent", "arrival_time")));
}

/// A trip whose every call is at a stop not held (Q twice) has no call left: no run, counted
/// once (`trips_without_call`) beside its two calls dropped.
#[test]
fn a_trip_left_with_no_call_is_counted() {
    let rows = "trip_id,arrival_time,departure_time,stop_id,stop_sequence\n\
        T1,08:00:00,08:00:00,Q,1\nT1,08:10:00,08:10:00,Q,2\n";
    let (report, kinds) = repaired("no-call", &feed("stop_times.txt", rows.into()));
    assert_eq!((report.runs, report.trips_single_call), (0, 0));
    let t1 = vec!["T1".to_owned(), "T1".to_owned()];
    assert_eq!(kinds, [("calls_dropped", 2, t1), ("trips_without_call", 1, vec!["T1".to_owned()])]);
}

/// Two overlapping frequency periods of T1 (08:00 to 09:00 and 08:30 to 09:30, every 10 min):
/// the second gives again the runs at 08:30, 08:40 and 08:50 (30 600, 31 200, 31 800 s), whose
/// names are taken; those 3 are left out, noted: 6 + 3 = 9 runs. A `frequencies.txt` without
/// its `headway_secs` column is not read: one run, T1 itself.
#[test]
fn overlapping_frequencies_keep_the_first_run_of_each_name() {
    let rows = "trip_id,start_time,end_time,headway_secs\nT1,08:00:00,09:00:00,600\nT1,08:30:00,09:30:00,600\n";
    let (report, kinds) = repaired("overlap", &feed("frequencies.txt", rows.into()));
    assert_eq!((report.runs, report.frequency_runs), (9, 9));
    let first = ["T1@30600", "T1@31200", "T1@31800"].map(str::to_owned).to_vec();
    assert_eq!(kinds, [("runs_repeated", 3, first)]);
    let no_headway = "trip_id,start_time,end_time\nT1,08:00:00,09:00:00\n";
    let (report, kinds) = repaired("no-headway", &feed("frequencies.txt", no_headway.into()));
    assert_eq!((report.runs, kinds), (1, one("files_unread", "frequencies.txt")));
}

/// The refusals, each naming what it refuses, and the two repairs of a whole feed.
#[test]
fn the_refusals() {
    let (trips, times) = trips_and_times(&[("WK", "T1")]);
    let files = [
        ("stops.txt", STOPS),
        ("trips.txt", trips.as_str()),
        ("calendar.txt", WEEKDAYS),
        ("stop_times.txt", times.as_str()),
    ];
    let net = street(&[0.0, 1_000.0]);
    let path = zip("refusals", "", &files);
    for bad in [-1.0, f64::NAN, f64::INFINITY] {
        let err = openmobisim_io_gtfs::read(&path, &net, Metres(bad)).unwrap_err();
        assert!(matches!(err, GtfsError::TransferWalkMax { .. }), "{err:?}");
    }
    let mut b = NetworkBuilder::new();
    b.add_node("a", Position::Metres(Xy::new(0.0, 0.0)));
    b.add_node("b", Position::Metres(Xy::new(100.0, 0.0)));
    let row = LinkRow {
        modes: ModeSet::of(&[Mode::Walk]),
        lanes: None,
        car_speed_km_h: None,
        length_m: None,
        capacity_veh_h: None,
        signal_at_downstream_end: false,
    };
    b.add_link("l", "a", "b", row);
    let in_metres = b.build(common::VALUES).unwrap();
    let err = openmobisim_io_gtfs::read(&path, &in_metres, Metres(400.0)).unwrap_err();
    assert_eq!(err, GtfsError::NetworkInMetres);
    std::fs::remove_file(path).ok();

    let err = read("missing", &files[..3]).unwrap_err();
    assert_eq!(err, GtfsError::MissingFile("stop_times.txt"));
    let no_lat = [("stops.txt", "stop_id,stop_lon\nA,4.36\n"), files[1], files[2], files[3]];
    let err = read("column", &no_lat).unwrap_err();
    assert_eq!(err, GtfsError::MissingColumn { file: "stops.txt", column: "stop_lat" });
    let weekend = "service_id,monday,tuesday,wednesday,thursday,friday,saturday,sunday,start_date,end_date\n\
        WK,0,0,0,0,0,1,1,20261001,20261031\n";
    // A weekend-only feed: its busiest day of all seven, the first Saturday (2026-10-03), noted.
    let (_, report) =
        read("weekend", &[files[0], files[1], ("calendar.txt", weekend), files[3]]).unwrap();
    assert_eq!((report.date, report.runs), (20_261_003, 1));
    assert_eq!(report.repairs.kinds()[0].key, "weekday_absent");
    assert_eq!(report.repairs.kinds()[0].first, ["20261003"]);
    let never = "service_id,monday,tuesday,wednesday,thursday,friday,saturday,sunday,start_date,end_date\n\
        WK,0,0,0,0,0,0,0,20261001,20261031\n";
    let err = read("never", &[files[0], files[1], ("calendar.txt", never), files[3]]).unwrap_err();
    assert_eq!(err, GtfsError::NoService);
    // A run whose second call arrives before the first departs: dropped, noted.
    let backwards = format!("{STOP_TIMES}T1,08:10:00,08:10:00,A,1\nT1,08:00:00,08:00:00,B,2\n");
    let (_, report) =
        read("backwards", &[files[0], files[1], files[2], ("stop_times.txt", &backwards)]).unwrap();
    assert_eq!(report.runs, 0);
    assert_eq!(report.repairs.kinds()[0].key, "trips_out_of_order");
    let not_zip =
        std::env::temp_dir().join(format!("openmobisim-gtfs-text-{}", std::process::id()));
    std::fs::write(&not_zip, "not a zip").unwrap();
    let err = openmobisim_io_gtfs::read(&not_zip, &net, Metres(400.0)).unwrap_err();
    assert!(matches!(err, GtfsError::Zip(_)), "{err:?}");
    std::fs::remove_file(not_zip).ok();
}
