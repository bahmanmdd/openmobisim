//! Footpaths on a hand network: a walking street of 1 000 m with nodes at 0, 300, 800 and
//! 1 000 m and stops A, B, C on the nodes at 0, 300 and 800 m, at 4.8 km/h (4/3 m/s). By hand:
//! A to B is 300 m, 225 s; B to C 500 m, 375 s; A to C 800 m, 600 s.

mod common;

use common::{CALENDAR, STOP_TIMES, east, footpaths, street, zip};
use openmobisim_io_gtfs::read;
use openmobisim_types::units::Metres;

fn files(transfers: &str) -> Vec<(&'static str, String)> {
    let stop = |id: &str, m: f64| format!("{id},{},{}\n", east(m).lat, east(m).lon);
    let stops = format!(
        "stop_id,stop_lat,stop_lon\n{}{}{}",
        stop("A", 0.0),
        stop("B", 300.0),
        stop("C", 800.0)
    );
    let mut out = vec![
        ("stops.txt", stops),
        ("trips.txt", "route_id,service_id,trip_id\nR,WK,T\n".to_owned()),
        ("calendar.txt", format!("{CALENDAR}WK,1,1,1,1,1,0,0,20261001,20261031\n")),
        (
            "stop_times.txt",
            format!("{STOP_TIMES}T,08:00:00,08:00:00,A,1\nT,08:10:00,08:10:00,C,2\n"),
        ),
    ];
    if !transfers.is_empty() {
        out.push(("transfers.txt", transfers.to_owned()));
    }
    out
}

fn footpaths_at(name: &str, transfers: &str, max: f64) -> Vec<Vec<(String, f64)>> {
    let files = files(transfers);
    let refs: Vec<(&str, &str)> = files.iter().map(|(n, t)| (*n, t.as_str())).collect();
    let path = zip(name, "", &refs);
    let net = street(&[0.0, 300.0, 800.0, 1_000.0]);
    let (t, report) = read(&path, &net, Metres(max)).unwrap();
    std::fs::remove_file(path).ok();
    let out: Vec<_> = ["A", "B", "C"].iter().map(|s| footpaths(&t, s)).collect();
    assert_eq!(report.footpaths, out.iter().map(Vec::len).sum::<usize>() as u64);
    out
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * b.max(1.0)
}

/// At `transfer_walk_max` 400 m: A and B, 300 m apart, are 225 s each way; B and C, 500 m
/// apart, are out. At 1 000 m every pair is in. At exactly 300 m, A to B is in; at 299 m, out.
/// Fixed bug: the walking limit bounds the footpath set as it is built (version 1 filtered
/// walks after the search); nothing beyond it is in the timetable.
#[test]
fn three_stops_on_a_street_at_4_8_km_h() {
    let at_400 = footpaths_at("fp400", "", 400.0);
    assert_eq!(at_400[0].len(), 1);
    assert_eq!(at_400[0][0].0, "B");
    assert!(close(at_400[0][0].1, 225.0), "{:?}", at_400[0]);
    assert_eq!(at_400[1].len(), 1, "B to A only: C is 500 m away");
    assert!(at_400[1][0].0 == "A" && close(at_400[1][0].1, 225.0));
    assert!(at_400[2].is_empty());

    let at_1000 = footpaths_at("fp1000", "", 1_000.0);
    let seconds: Vec<Vec<(&str, f64)>> =
        at_1000.iter().map(|l| l.iter().map(|(s, d)| (s.as_str(), *d)).collect()).collect();
    for (got, want) in seconds.iter().zip([
        [("B", 225.0), ("C", 600.0)],
        [("A", 225.0), ("C", 375.0)],
        [("A", 600.0), ("B", 375.0)],
    ]) {
        assert_eq!(got.len(), 2);
        for ((s, d), (ws, wd)) in got.iter().zip(want) {
            assert!(*s == ws && close(*d, wd), "{got:?}");
        }
    }
    assert_eq!(footpaths_at("fp300", "", 300.0)[0].len(), 1, "a walk of exactly the limit is in");
    assert!(footpaths_at("fp299", "", 299.0)[0].is_empty(), "300 m is out at 299 m");
}

/// `transfers.txt` (reference: `transfer_type` 2 with `min_transfer_time`, 1 a timed transfer):
/// A to B at 300 s raises the walk's 225 s; B to A at 100 s leaves it at 225 s; A to B timed
/// (type 1, zero) leaves it; A to C has no footpath at 400 m, a trip's row and type 0 and 3
/// rows are skipped, as is a stop to itself.
#[test]
fn transfers_raise_a_footpath_to_their_minimum_time() {
    let transfers = "from_stop_id,to_stop_id,transfer_type,min_transfer_time,from_trip_id\n\
        A,B,2,300,\nB,A,2,100,\nA,B,1,,\nA,C,2,60,\nB,A,2,900,T\nA,B,0,,\nB,A,3,,\nA,A,2,120,\n";
    let files = files(transfers);
    let refs: Vec<(&str, &str)> = files.iter().map(|(n, t)| (*n, t.as_str())).collect();
    let path = zip("transfers", "", &refs);
    let (t, report) = read(&path, &street(&[0.0, 300.0, 800.0, 1_000.0]), Metres(400.0)).unwrap();
    std::fs::remove_file(path).ok();
    assert!(close(footpaths(&t, "A")[0].1, 300.0));
    assert!(close(footpaths(&t, "B")[0].1, 225.0));
    assert_eq!((report.transfers_applied, report.transfers_skipped), (3, 5));
}

/// A `transfers.txt` without its `transfer_type` column is not read (`files_unread`): A to B
/// stays 225 s by its walk, though the file says 300 s.
#[test]
fn a_transfers_file_without_one_of_its_columns_is_not_read() {
    let files = files("from_stop_id,to_stop_id,min_transfer_time\nA,B,300\n");
    let refs: Vec<(&str, &str)> = files.iter().map(|(n, t)| (*n, t.as_str())).collect();
    let path = zip("transfers-no-type", "", &refs);
    let (t, report) = read(&path, &street(&[0.0, 300.0, 800.0, 1_000.0]), Metres(400.0)).unwrap();
    std::fs::remove_file(path).ok();
    assert!(close(footpaths(&t, "A")[0].1, 225.0));
    let kinds: Vec<_> =
        report.repairs.kinds().iter().map(|k| (k.key, k.count, k.first.clone())).collect();
    assert_eq!(kinds, [("files_unread", 1, vec!["transfers.txt".to_owned()])]);
}

/// On a network no walker may use (one car link from 0 to 1 000 m), no stop can be walked to:
/// no footpath (`footpaths_none`), the run read all the same.
#[test]
fn a_network_without_a_walk_link_gives_no_footpath() {
    use openmobisim_graph::geometry::Position;
    use openmobisim_graph::network::{LinkRow, NetworkBuilder};
    use openmobisim_types::mode::{Mode, ModeSet};
    let mut b = NetworkBuilder::new();
    b.add_node("a", Position::LonLat(east(0.0)));
    b.add_node("b", Position::LonLat(east(1_000.0)));
    let row = LinkRow {
        modes: ModeSet::of(&[Mode::Car]),
        lanes: Some(1),
        car_speed_km_h: Some(50.0),
        length_m: None,
        capacity_veh_h: None,
        signal_at_downstream_end: false,
    };
    b.add_link("ab", "a", "b", row);
    let net = b.build(common::VALUES).unwrap();
    let files = files("");
    let refs: Vec<(&str, &str)> = files.iter().map(|(n, t)| (*n, t.as_str())).collect();
    let path = zip("no-walk", "", &refs);
    let (t, report) = read(&path, &net, Metres(400.0)).unwrap();
    std::fs::remove_file(path).ok();
    assert_eq!((report.footpaths, report.runs), (0, 1));
    assert!(footpaths(&t, "A").is_empty());
    assert_eq!(report.repairs.count("footpaths_none"), 1);
}
