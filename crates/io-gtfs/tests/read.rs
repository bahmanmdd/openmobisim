//! Reading a small hand-written feed, as a folder and as a zip: the busiest
//! weekday, service exceptions, clipping to an area, interpolated times, the
//! pick-up and drop-off rules, transfers, and what is counted as skipped.

use std::io::Write;
use std::path::PathBuf;

use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_transit::{ALIGHT, BOARD, ServiceDate, ServiceKind, Timetable};
use openmobisim_core_types::ids::{EntityId, TransitRunId};
use openmobisim_io_gtfs::{GtfsError, GtfsReport, read_gtfs};

const FILES: &[(&str, &str)] = &[
    (
        "stops.txt",
        "stop_id,stop_name,stop_lat,stop_lon,location_type,parent_station\n\
         P,Station,52.37,4.91,1,\n\
         A,Alpha,52.37,4.90,0,\n\
         B,Bravo,52.37,4.91,,P\n\
         C,Charlie,52.37,4.93,0,\n\
         D,Delta (outside),52.37,5.50,0,\n\
         E,Echo,52.37,4.95,0,\n\
         X,Broken,north,4.9,0,\n",
    ),
    (
        "routes.txt",
        "route_id,route_short_name,route_long_name,route_type\n\
         R1,1,Bus one,3\n\
         R2,,Tram two,0\n",
    ),
    (
        "trips.txt",
        "route_id,service_id,trip_id\n\
         R1,WK,T1\n\
         R1,WE,T2\n\
         R2,WK,T3\n\
         R1,EXTRA,T4\n\
         R1,WK,T5\n\
         R1,WK,T6\n\
         R1,WK,T7\n",
    ),
    (
        "calendar.txt",
        "service_id,monday,tuesday,wednesday,thursday,friday,saturday,sunday,start_date,end_date\n\
         WK,1,1,1,1,1,0,0,20261001,20261031\n\
         WE,0,0,0,0,0,1,1,20261001,20261031\n",
    ),
    (
        "calendar_dates.txt",
        "service_id,date,exception_type\n\
         EXTRA,20261007,1\n\
         WK,20261009,2\n",
    ),
    (
        "stop_times.txt",
        "trip_id,arrival_time,departure_time,stop_id,stop_sequence,pickup_type,drop_off_type\n\
         T1,08:30:00,08:30:00,C,4,0,0\n\
         T1,08:00:00,08:00:00,A,1,0,1\n\
         T1,08:10:00,08:11:00,B,2,0,0\n\
         T1,08:20:00,08:20:00,D,3,0,0\n\
         T2,08:00:00,08:00:00,A,1,0,0\n\
         T2,08:10:00,08:10:00,B,2,0,0\n\
         T3,09:00:00,09:00:00,A,1,0,0\n\
         T3,09:20:00,09:20:00,C,2,1,0\n\
         T4,07:00:00,07:00:00,A,1,0,0\n\
         T4,07:05:00,07:05:00,B,2,0,0\n\
         T5,10:00:00,10:00:00,A,1,0,0\n\
         T5,,,B,2,0,0\n\
         T5,10:20:00,10:20:00,C,3,0,0\n\
         T6,,,A,1,0,0\n\
         T6,11:10:00,11:10:00,B,2,0,0\n\
         T5,99:99:99,10:30:00,C,4,0,0\n\
         T7,12:00:00,12:00:00,E,1,0,0\n\
         T7,12:30:00,12:30:00,D,2,0,0\n",
    ),
    (
        "transfers.txt",
        "from_stop_id,to_stop_id,transfer_type,min_transfer_time,from_trip_id\n\
         B,C,2,120,\n\
         A,B,3,,\n\
         C,B,2,60,T1\n\
         P,A,2,30,\n",
    ),
    ("frequencies.txt", "trip_id,start_time,end_time,headway_secs\nT1,06:00:00,09:00:00,600\n"),
];

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("openmobisim-gtfs-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (file, text) in FILES {
        std::fs::write(dir.join(file), text).unwrap();
    }
    dir
}

fn zipped(name: &str, prefix: &str) -> PathBuf {
    let path =
        std::env::temp_dir().join(format!("openmobisim-gtfs-{name}-{}.zip", std::process::id()));
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (file, text) in FILES {
        zip.start_file(format!("{prefix}{file}"), options).unwrap();
        zip.write_all(text.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
    path
}

fn west_of_five(p: LonLat) -> bool {
    p.lon < 5.0
}

fn run(t: &Timetable, id: &str) -> Option<TransitRunId> {
    t.run_ids().id_of(id).map(TransitRunId::new)
}

fn calls(t: &Timetable, id: &str) -> Vec<(String, u32, u32, u8)> {
    let r = run(t, id).expect("the run is there");
    t.run_calls(r)
        .map(|c| {
            (
                t.stop_ids().external(t.call_stop(c).raw()).to_string(),
                t.scheduled().arrival[c],
                t.scheduled().departure[c],
                t.call_flags(c),
            )
        })
        .collect()
}

fn check(t: &Timetable, report: &GtfsReport) {
    // Wednesday 7 October has the weekday trips and the extra one: the busiest.
    assert_eq!(report.date, ServiceDate::parse("20261007"));
    assert!(report.date_chosen);
    assert_eq!(report.stops_in_feed, 5, "A to E: the station and the broken row are not stops");
    assert_eq!(report.stops_in_area, 4);
    assert_eq!(report.trips_in_feed, 7);
    assert_eq!(report.trips_on_date, 6, "T1, T3, T4, T5, T6, T7");
    assert_eq!(report.trips_untimed, 1, "T6 has no first time");
    assert_eq!(report.runs_kept, 4);
    assert_eq!(report.times_interpolated, 1);
    assert_eq!(report.rows_skipped, 2, "the broken stop, the bad time");
    assert_eq!((report.transfers_kept, report.transfers_skipped), (1, 3));
    assert_eq!(report.frequencies_ignored, 1);

    // T1 is clipped (D is outside) and sorted by sequence; A lets nobody off.
    assert_eq!(
        calls(t, "T1"),
        [
            ("A".into(), 8 * 3600, 8 * 3600, BOARD),
            ("B".into(), 8 * 3600 + 600, 8 * 3600 + 660, BOARD | ALIGHT),
            ("C".into(), 8 * 3600 + 1800, 8 * 3600 + 1800, BOARD | ALIGHT),
        ]
    );
    assert!(run(t, "T2").is_none(), "a weekend trip");
    assert!(run(t, "T6").is_none());
    // T5's B is interpolated by distance: A→B is a third of A→C.
    let t5 = calls(t, "T5");
    assert_eq!(t5.len(), 3);
    assert!((i64::from(t5[1].1) - i64::from(10 * 3600 + 400)).abs() <= 1, "{t5:?}");
    // The tram's C takes nobody on.
    assert_eq!(calls(t, "T3")[1].3, ALIGHT);
    let t3 = run(t, "T3").unwrap();
    assert_eq!(t.route_kind(t.run_route(t3)), ServiceKind::Tram);
    assert_eq!(
        t.route_short_name(t.run_route(t3)),
        "Tram two",
        "the long name when there is no short one"
    );
    assert_eq!(t.stop_parent(t.stop_ids().typed_id_of("B").unwrap()), Some("P"));
    assert_eq!(t.transfers().len(), 1);
    assert_eq!(t.transfers()[0].seconds, 120);
    // E's only run, T7, has one call in the area: dropped, and E with it.
    assert!(run(t, "T7").is_none());
    assert_eq!(report.stops_kept, 3);
    assert_eq!(t.stop_count(), 3, "only the stops a kept run calls at");
}

#[test]
fn a_folder_feed() {
    let dir = folder("folder");
    let (t, report) = read_gtfs(&dir, None, &west_of_five).unwrap();
    check(&t, &report);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_zipped_feed_at_the_root_or_in_a_folder() {
    for (name, prefix) in [("zip-root", ""), ("zip-nested", "feed/")] {
        let path = zipped(name, prefix);
        let (t, report) = read_gtfs(&path, None, &west_of_five).unwrap();
        check(&t, &report);
        std::fs::remove_file(path).ok();
    }
}

#[test]
fn a_given_date_and_a_date_without_service() {
    let dir = folder("dates");
    // Friday 9 October: the weekday service is removed; nothing else runs.
    let err = read_gtfs(&dir, ServiceDate::parse("20261009"), &west_of_five).unwrap_err();
    assert!(matches!(err, GtfsError::NoService(_)), "{err:?}");
    // A Saturday: only the weekend trip.
    let (t, report) = read_gtfs(&dir, ServiceDate::parse("20261010"), &west_of_five).unwrap();
    assert!(!report.date_chosen);
    assert_eq!(t.run_count(), 1);
    assert!(run(&t, "T2").is_some());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn an_area_no_run_calls_in_twice_is_an_error() {
    let dir = folder("empty-area");
    let err = read_gtfs(&dir, None, &|p: LonLat| p.lon > 5.0).unwrap_err();
    assert!(
        matches!(&err, GtfsError::NoService(m) if m.contains("1 of the feed's 5 stops")),
        "{err:?}"
    );
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_missing_file_is_an_error() {
    let dir = folder("missing");
    std::fs::remove_file(dir.join("stop_times.txt")).unwrap();
    let err = read_gtfs(&dir, None, &west_of_five).unwrap_err();
    assert_eq!(err, GtfsError::MissingFile("stop_times.txt"));
    std::fs::remove_dir_all(dir).ok();
}
