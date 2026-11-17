//! The names beside the timetable, from a hand feed: each stop's name, each run's line (short
//! name, else long name, else route id) and kind (from `route_type`, as release 0.1 read it),
//! the run's name read back to its trip for a frequency run and a night run.

mod common;

use common::zip;
use openmobisim_io_gtfs::{Kind, lines};

#[test]
fn stops_are_named_and_runs_have_their_lines_and_kinds() {
    let routes = "route_id,route_short_name,route_long_name,route_type\n\
                  r1,1,Tram one,0\nr19,,Night bus nineteen,3\nrX,,,2\nr6,,,715\nr7,F,,1200\nr8,C,,6\nr9,M,,x\n";
    let trips = "route_id,service_id,trip_id\nr1,s,t1\nr19,s,t19\nrX,s,tX\nr6,s,t6\nr7,s,t7\nr8,s,t8\nr9,s,t9\nnone,s,t0\n";
    let stops = "stop_id,stop_name,stop_lat,stop_lon\na,Delft Station,52,4.36\nb,,52,4.37\n";
    let path =
        zip("lines", "", &[("routes.txt", routes), ("trips.txt", trips), ("stops.txt", stops)]);
    let l = lines(&path).expect("read");
    std::fs::remove_file(&path).ok();
    assert_eq!(l.stop_name("a"), Some("Delft Station"));
    assert_eq!(l.stop_name("b"), None, "a blank name is no name");
    let named = |run: &str| l.line_of_run(run).map(|line| (line.name.as_str(), line.kind));
    assert_eq!(named("t1"), Some(("1", Kind::Tram)));
    assert_eq!(named("t19"), Some(("Night bus nineteen", Kind::Bus)), "the long name");
    assert_eq!(named("tX"), Some(("rX", Kind::Rail)), "the route id");
    assert_eq!(named("t6"), Some(("r6", Kind::Bus)), "extended type 715");
    assert_eq!(named("t7"), Some(("F", Kind::Ferry)), "extended type 1200");
    assert_eq!(named("t8"), Some(("C", Kind::Other)), "gondola, 6");
    assert_eq!(named("t9"), Some(("M", Kind::Other)), "a type that is not a number");
    assert_eq!(named("t0"), None, "a trip of an unknown route");
    assert_eq!(named("t1@28800"), Some(("1", Kind::Tram)), "a frequency run");
    assert_eq!(named("t19@previous-day"), Some(("Night bus nineteen", Kind::Bus)), "a night run");
    assert_eq!(named("t1@28800@previous-day"), Some(("1", Kind::Tram)));
    assert_eq!(named("t1@x"), None);
    let kinds: Vec<&str> = Kind::ALL.iter().map(|k| k.as_str()).collect();
    assert_eq!(kinds, ["rail", "metro", "tram", "bus", "ferry", "other"]);
}

#[test]
fn a_feed_without_routes_names_its_stops_and_no_line() {
    let stops = "stop_id,stop_name,stop_lat,stop_lon\na,A,52,4.36\n";
    let path = zip(
        "lines-none",
        "",
        &[("stops.txt", stops), ("trips.txt", "route_id,service_id,trip_id\nr,s,t\n")],
    );
    let l = lines(&path).expect("read");
    std::fs::remove_file(&path).ok();
    assert_eq!(l.stop_name("a"), Some("A"));
    assert!(l.trip_lines.is_empty());
}

/// The other basic and extended route types, by GTFS's reference and release 0.1's reading.
#[test]
fn every_route_type_has_its_kind() {
    let cases = [
        (1, Kind::Metro),
        (4, Kind::Ferry),
        (5, Kind::Tram),
        (11, Kind::Bus),
        (12, Kind::Metro),
        (100, Kind::Rail),
        (109, Kind::Rail),
        (199, Kind::Rail),
        (400, Kind::Metro),
        (401, Kind::Metro),
        (699, Kind::Metro),
        (900, Kind::Tram),
        (906, Kind::Tram),
        (999, Kind::Tram),
        (1100, Kind::Other),
    ];
    for (route_type, kind) in cases {
        assert_eq!(Kind::from_route_type(route_type), kind, "route_type {route_type}");
    }
}
