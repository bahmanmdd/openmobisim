//! The transit numbers of the summary on a hand timetable (the case of
//! `crates/map/tests/transit.rs`): a line of nodes n0 (0, 0), n1 (1 000, 0), n2 (2 000, 0), n3
//! (3 000, 0) m with footways; stops s1, s2, s3 and s3b, s4 at n0 to n3 (s3 and s3b both at n2);
//! runs r0 s1 100 s, s2 200 s, s3 300 s and r1 s3b 400 s, s4 500 s; a footpath s3→s3b of 30 s.
//! Transit-only trips at 0 s, time the only cost: `t1`, `t2` n0→n2 ride r0 (one ride each), `t3`
//! n1→n2 rides r0 (one), `t4` n0→n3 rides r0 then r1 (two). Boardings 1 + 1 + 1 + 2 = 5; rides
//! per transit trip 5 / 4 = 1.25; one trip of four changes vehicle (25.0 %). 5 stops, 2 runs. No
//! walk alternative, so no kilometres walked on links: the walks to and from stops are off them.

mod common;

use std::path::PathBuf;

use common::{has, value};
use openmobisim_demand::demand::Demand;
use openmobisim_demand::input::TripRow;
use openmobisim_graph::geometry::{Position, Xy};
use openmobisim_graph::network::{BuildValues, LinkRow, Network, NetworkBuilder};
use openmobisim_loop::parameters::{ChoiceModel, Parameters};
use openmobisim_loop::run::run;
use openmobisim_summary::{Summarised, write};
use openmobisim_transit::timetable::{Call, Timetable, TimetableBuilder};
use openmobisim_types::mode::{Mode, ModeSet};
use openmobisim_types::time::Second;
use openmobisim_types::units::{Duration, Metres};

const VALUES: BuildValues =
    BuildValues { capacity_per_lane: 1_800.0, bike_speed: 15.0, walk_speed: 4.8 };
const TIME_ONLY: [f64; 16] =
    [1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];

fn network() -> Network {
    let footway = LinkRow {
        modes: ModeSet::of(&[Mode::Walk]),
        lanes: None,
        car_speed_km_h: None,
        length_m: Some(1_000.0),
        capacity_veh_h: None,
        signal_at_downstream_end: false,
    };
    let mut b = NetworkBuilder::new();
    for i in 0..4 {
        b.add_node(format!("n{i}"), Position::Metres(Xy::new(1_000.0 * f64::from(i), 0.0)));
    }
    for i in 0..3 {
        b.add_link(format!("f{i}"), format!("n{i}"), format!("n{}", i + 1), footway);
        b.add_link(format!("b{i}"), format!("n{}", i + 1), format!("n{i}"), footway);
    }
    b.build(VALUES).expect("a network")
}

fn timetable() -> Timetable {
    let mut b = TimetableBuilder::new();
    for (id, x) in
        [("s1", 0.0), ("s2", 1_000.0), ("s3", 2_000.0), ("s3b", 2_000.0), ("s4", 3_000.0)]
    {
        b.add_stop(id, (Metres(x), Metres(0.0)));
    }
    let call = |stop: &str, s: u32| Call {
        stop: stop.to_string(),
        arrival: Second(s),
        departure: Second(s),
    };
    b.add_run("r0", vec![call("s1", 100), call("s2", 200), call("s3", 300)]);
    b.add_run("r1", vec![call("s3b", 400), call("s4", 500)]);
    b.add_footpath("s3", "s3b", Duration(30.0));
    b.build().expect("a timetable")
}

/// The run of the header, its summary written with `lines` and `record`; the page's text.
fn summary_with(lines: Option<&openmobisim_io_gtfs::Lines>, record: &str, name: &str) -> String {
    let (net, tt) = (network(), timetable());
    let row = |id: &str, from: f64, to: f64| TripRow {
        id: id.to_string(),
        origin: Position::Metres(Xy::new(from, 0.0)),
        destination: Position::Metres(Xy::new(to, 0.0)),
        departure: Second(0),
        class: "all".to_string(),
        modes: ModeSet::of(&[Mode::Transit]),
    };
    let rows = [
        row("t1", 0.0, 2_000.0),
        row("t2", 0.0, 2_000.0),
        row("t3", 1_000.0, 2_000.0),
        row("t4", 0.0, 3_000.0),
    ];
    let dem = Demand::build(&rows, &["all".to_string()], &net, Some(&tt), Metres(1_500.0))
        .expect("a demand");
    let parameters = Parameters {
        link_bin: 60,
        iteration_cap: 1,
        choice: ChoiceModel::Deterministic,
        coefficients: vec![TIME_ONLY],
        ..Parameters::defaults(1)
    };
    let r = run(&net, &dem, Some(&tt), (Second(0), Second(3_600)), &parameters).expect("a run");
    let kept = common::kept(&r, &net, &dem, Some(&tt), &parameters, &format!("{name}.run"));
    let summarised = Summarised {
        kept: &kept,
        network: &net,
        timetable: Some(&tt),
        lines,
        iterations: &r.iterations,
        stopped_by_rule: Some(r.stopped_by_rule),
        record,
    };
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    write(&summarised, "transit", &path).expect("written");
    std::fs::read_to_string(&path).expect("the summary")
}

/// The transit map in the map's colours by kind: `r0` a tram, `r1` a bus, so the hops s1→s2 and
/// s2→s3 are drawn as trams (class `k-k2`) and s3b→s4 as a bus (`k-k3`), each kind in the legend
/// under the map; a network read from OpenStreetMap credits it in the footer, one from tables does
/// not.
#[test]
fn the_transit_map_colours_hops_by_kind_and_the_footer_credits_openstreetmap() {
    use openmobisim_io_gtfs::{Kind, Line, Lines};
    let mut lines = Lines::default();
    lines.trip_lines.insert("r0".to_owned(), Line { name: "1".to_owned(), kind: Kind::Tram });
    lines.trip_lines.insert("r1".to_owned(), Line { name: "19".to_owned(), kind: Kind::Bus });
    let html =
        summary_with(Some(&lines), "[result.network]\nsource = \"osm\"\n", "summary-kinds.html");
    let map = &html[html.find("data-k=\"map.transit\"").expect("the transit map")..];
    assert!(map.contains("class=\"k-k2\"") && map.contains("class=\"k-k3\""));
    assert!(map.contains("k-k2\"></span>tram") && map.contains("k-k3\"></span>bus"));
    assert!(html.contains("<br>© OpenStreetMap contributors (ODbL)</footer>"));
    let plain =
        summary_with(None, "[result.network]\nsource = \"tables\"\n", "summary-tables.html");
    assert!(!plain.contains("OpenStreetMap") && plain.contains("class=\"k-k5\""));
}

#[test]
fn boardings_rides_per_trip_and_changes_are_the_hand_values() {
    let (net, tt) = (network(), timetable());
    let row = |id: &str, from: f64, to: f64| TripRow {
        id: id.to_string(),
        origin: Position::Metres(Xy::new(from, 0.0)),
        destination: Position::Metres(Xy::new(to, 0.0)),
        departure: Second(0),
        class: "all".to_string(),
        modes: ModeSet::of(&[Mode::Transit]),
    };
    let rows = [
        row("t1", 0.0, 2_000.0),
        row("t2", 0.0, 2_000.0),
        row("t3", 1_000.0, 2_000.0),
        row("t4", 0.0, 3_000.0),
    ];
    let dem = Demand::build(&rows, &["all".to_string()], &net, Some(&tt), Metres(1_500.0))
        .expect("a demand");
    let parameters = Parameters {
        link_bin: 60,
        iteration_cap: 1,
        choice: ChoiceModel::Deterministic,
        coefficients: vec![TIME_ONLY],
        ..Parameters::defaults(1)
    };
    let r = run(&net, &dem, Some(&tt), (Second(0), Second(3_600)), &parameters).expect("a run");
    let kept = common::kept(&r, &net, &dem, Some(&tt), &parameters, "summary-boardings.run");
    let summarised = Summarised {
        kept: &kept,
        network: &net,
        timetable: Some(&tt),
        lines: None,
        iterations: &r.iterations,
        stopped_by_rule: Some(r.stopped_by_rule),
        record: "",
    };
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("summary-transit.html");
    write(&summarised, "transit", &path).expect("written");
    let html = std::fs::read_to_string(&path).expect("the summary");
    for (key, hand) in [
        ("boardings", "5"),
        ("rides-per-trip", "1.25"),
        ("changing", "25.0"),
        ("mode.transit.trips", "4"),
        ("mode.transit.share", "100.0"),
        ("net.stops", "5"),
        ("net.runs", "2"),
        ("net.walk.links", "6"),
        ("net.car.links", "0"),
    ] {
        assert_eq!(value(&html, key), hand, "{key}");
    }
    assert!(has(&html, "map.transit"));
    assert!(!has(&html, "km-walked") && !has(&html, "map.walk") && !has(&html, "map.car"));
}
