//! The transit layer and the journeys on a timetable, by hand: hops loaded by the journeys
//! taken, boardings per stop, and each journey's rides, walks and waits.
//!
//! A line of nodes n0 (0, 0), n1 (1 000, 0), n2 (2 000, 0), n3 (3 000, 0) metres, footways each
//! way between neighbours (a walker covers 1 000 m in 750 s). Stops by sorted id: s1 at n0, s2 at
//! n1, s3 and s3b at n2, s4 at n3; a footpath s3→s3b of 30 s. Runs: r0 s1 100 s, s2 200 s, s3
//! 300 s; r1 s3b 400 s, s4 500 s. Transit-only trips at 0 s, a walk to a stop at most 1 500 m,
//! the boarding slack 60 s, time the only cost, the cheapest taken:
//! - `t1`, `t2` n0→n2: board r0 at s1 at 100 s (waiting 100 s), ride to s3 (200 s): 300 s. A walk
//!   to s2 (750 s) misses r0 there; leaving at s2 for a 1 000 m walk arrives at 950 s.
//! - `t3` n1→n2: board r0 at s2 at 200 s (waiting 200 s), ride to s3 (100 s): 300 s.
//! - `t4` n0→n3: r0 s1 to s3 (100 s waited, 200 s ridden), the footpath to s3b (30 s, there at
//!   330 s, in time for 400 s with the slack), 70 s waited, r1 to s4 (100 s): 500 s; one ride and
//!   a 1 000 m walk from s3 would arrive at 1 050 s.
//!
//! So the hops: s1→s2 ridden by t1, t2, t4 (3); s2→s3 by t1 to t4 (4); s3b→s4 by t4 (1); listed
//! the least ridden first. Boardings: s1 3, s2 1, s3 0, s3b 1, s4 0. Pairs by doors: n0→n2 (t1,
//! t2: pair 1), then n1→n2 (t3) before n0→n3 (t4), one traveller each, the lower trip id first.

#![allow(clippy::float_cmp, reason = "whole numbers and values a float holds exactly")]

mod common;

use std::path::PathBuf;

use openmobisim_demand::demand::Demand;
use openmobisim_demand::input::TripRow;
use openmobisim_graph::geometry::{Position, Xy};
use openmobisim_graph::network::{BuildValues, LinkRow, Network, NetworkBuilder};
use openmobisim_loop::parameters::{ChoiceModel, Parameters};
use openmobisim_loop::run::run;
use openmobisim_map::{MapError, Mapped, write};
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

fn demand(network: &Network, tt: &Timetable) -> Demand {
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
    Demand::build(&rows, &["all".to_string()], network, Some(tt), Metres(1_500.0))
        .expect("a demand")
}

fn parameters() -> Parameters {
    Parameters {
        link_bin: 60,
        iteration_cap: 1,
        choice: ChoiceModel::Deterministic,
        coefficients: vec![TIME_ONLY],
        ..Parameters::defaults(1)
    }
}

#[test]
fn hops_boardings_and_journeys_on_a_timetable_are_the_hand_values() {
    let (net, tt) = (network(), timetable());
    let dem = demand(&net, &tt);
    let r = run(&net, &dem, Some(&tt), (Second(0), Second(3_600)), &parameters()).expect("a run");
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("map-transit.html");
    write(
        &Mapped {
            kept: &common::kept(&r, &net, &dem, Some(&tt), &parameters(), "map-transit.run"),
            network: &net,
            timetable: Some(&tt),
            lines: None,
            record: "",
        },
        "transit",
        &path,
    )
    .expect("written");
    let page = common::read(&path);
    let stops: Vec<&str> = page.meta.get("stops").list().iter().map(common::Json::text).collect();
    assert_eq!(stops, ["s1", "s2", "s3", "s3b", "s4"]);
    assert_eq!(page.i32("tsx"), [0, 10_000, 20_000, 20_000, 30_000]);
    assert_eq!(page.i32("tsy"), [0, 0, 0, 0, 0]);
    assert_eq!(page.u32("tsb"), [3, 1, 0, 1, 0]);
    assert_eq!(page.u32("ta"), [3, 0, 1]);
    assert_eq!(page.u32("tb"), [4, 1, 2]);
    assert_eq!(page.u32("tl"), [1, 3, 4]);
    // The pairs: n0→n2 (2), n1→n2 (1), n0→n3 (1); one journey each, all finished.
    assert_eq!(page.u32("qn"), [2, 1, 1]);
    assert_eq!(page.u32("qr"), [0, 0, 0]);
    assert_eq!(page.u32("qs"), [0, 1, 2, 3]);
    assert_eq!(page.u8("om"), [3, 3, 3]);
    assert_eq!(page.u32("on"), [2, 1, 1]);
    assert_eq!(page.u32("of"), [2, 1, 1]);
    assert_eq!(page.f32("ot"), [300.0, 300.0, 500.0]);
    // Legs: walk 4, wait 5, ride 3; the walk to the first stop and from the last are 0 s.
    assert_eq!(page.u32("ol"), [0, 4, 8, 15]);
    assert_eq!(page.u8("lk"), [4, 5, 3, 4, 4, 5, 3, 4, 4, 5, 3, 4, 5, 3, 4]);
    let seconds = [0.0, 100.0, 200.0, 0.0, 0.0, 200.0, 100.0, 0.0, 0.0, 100.0, 200.0, 30.0, 70.0];
    assert_eq!(page.f32("lsec"), [seconds.as_slice(), &[100.0, 0.0]].concat());
    // Where each leg is drawn: rides through ranges of the stops, the walk between rides from s3
    // (2) to s3b (3), the rest nowhere.
    let n = u32::MAX;
    assert_eq!(page.u32("la"), [n, n, 0, n, n, n, 3, n, n, n, 5, 2, n, 8, n]);
    assert_eq!(page.u32("lb"), [n, n, 3, n, n, n, 5, n, n, n, 8, 3, n, 10, n]);
    assert_eq!(page.u32("pst"), [0, 1, 2, 1, 2, 0, 1, 2, 3, 4]);
    assert!(page.u32("pl").is_empty());
}

#[test]
fn a_journey_without_its_timetable_is_refused() {
    let (net, tt) = (network(), timetable());
    let dem = demand(&net, &tt);
    let r = run(&net, &dem, Some(&tt), (Second(0), Second(3_600)), &parameters()).expect("a run");
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("map-transit-refused.html");
    let _ = std::fs::remove_file(&path);
    assert_eq!(
        write(
            &Mapped {
                kept: &common::kept(
                    &r,
                    &net,
                    &dem,
                    Some(&tt),
                    &parameters(),
                    "map-no-timetable.run"
                ),
                network: &net,
                timetable: None,
                lines: None,
                record: ""
            },
            "refused",
            &path
        ),
        Err(MapError::NoTimetable)
    );
    assert!(!path.exists(), "nothing written");
}

/// A timetable of `stops` (`(id, x)`, metres on the line) and `runs` (`(id, [(stop, second)])`).
fn timetable_of(stops: &[(&str, f64)], runs: &[(&str, &[(&str, u32)])]) -> Timetable {
    let mut b = TimetableBuilder::new();
    for &(id, x) in stops {
        b.add_stop(id, (Metres(x), Metres(0.0)));
    }
    for &(id, calls) in runs {
        let calls = calls
            .iter()
            .map(|&(stop, s)| Call {
                stop: stop.to_string(),
                arrival: Second(s),
                departure: Second(s),
            })
            .collect();
        b.add_run(id, calls);
    }
    b.build().expect("a timetable")
}

/// The transit-only trips `(id, from x, to x)` at 0 s on the line, run on `tt` and mapped as
/// `title` to `name`.
fn mapped(tt: &Timetable, trips: &[(&str, f64, f64)], title: &str, name: &str) -> common::Page {
    let net = network();
    let rows: Vec<TripRow> = trips
        .iter()
        .map(|&(id, from, to)| TripRow {
            id: id.to_string(),
            origin: Position::Metres(Xy::new(from, 0.0)),
            destination: Position::Metres(Xy::new(to, 0.0)),
            departure: Second(0),
            class: "all".to_string(),
            modes: ModeSet::of(&[Mode::Transit]),
        })
        .collect();
    let dem = Demand::build(&rows, &["all".to_string()], &net, Some(tt), Metres(1_500.0))
        .expect("a demand");
    let r = run(&net, &dem, Some(tt), (Second(0), Second(3_600)), &parameters()).expect("a run");
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    write(
        &Mapped {
            kept: &common::kept(&r, &net, &dem, Some(tt), &parameters(), &format!("{name}.run")),
            network: &net,
            timetable: Some(tt),
            lines: None,
            record: "",
        },
        title,
        &path,
    )
    .expect("written");
    common::read(&path)
}

/// Stops s1 at n0, s2 at n1, s3 at n2; run `rA` s1 300 s, s2 400 s, s3 500 s and run `rB` s1
/// 100 s, s2 200 s: two patterns (their stops differ) sharing the hop s1→s2. `tA` from (300, 0),
/// 300 m from n0, to n2: a walk of 225 s to s1 (300 m at 0.75 s/m), there at 225 s, too late for
/// `rB` (100 s) and in time for `rA` with the slack (285 ≤ 300): 75 s waited, 200 s ridden, at
/// its door at 500 s. `tB` n0→n1: `rB` at 100 s (100 s waited, 100 s ridden): 200 s. The hop
/// s1→s2 carries both (2), one hop for both patterns; s2→s3 carries `tA` (1). Boardings: s1 2.
#[test]
fn an_access_walk_is_a_leg_and_two_patterns_share_a_hop() {
    let tt = timetable_of(
        &[("s1", 0.0), ("s2", 1_000.0), ("s3", 2_000.0)],
        &[("rA", &[("s1", 300), ("s2", 400), ("s3", 500)]), ("rB", &[("s1", 100), ("s2", 200)])],
    );
    let page =
        mapped(&tt, &[("tA", 300.0, 2_000.0), ("tB", 0.0, 1_000.0)], "share", "map-share.html");
    assert_eq!(page.u32("ta"), [1, 0]);
    assert_eq!(page.u32("tb"), [2, 1]);
    assert_eq!(page.u32("tl"), [1, 2]);
    assert_eq!(page.u32("tsb"), [2, 0, 0]);
    assert_eq!(page.i32("qox"), [3_000, 0]);
    assert_eq!(page.f32("ot"), [500.0, 200.0]);
    assert_eq!(page.u8("lk"), [4, 5, 3, 4, 4, 5, 3, 4]);
    assert_eq!(page.f32("lsec"), [225.0, 75.0, 200.0, 0.0, 0.0, 100.0, 100.0, 0.0]);
    assert_eq!(page.u32("pst"), [0, 1, 2, 0, 1]);
}

/// A title and a stop id holding a quote, a tag and the end of a script element: the file's
/// three script elements still end where they should (three `</script>` in the file), its data
/// parses, and both come back as given; the title element shows the title escaped.
#[test]
fn a_title_and_a_stop_id_with_quotes_tags_and_script_ends_come_back_literally() {
    let (title, stop) = (r#"map "<i>" </script> & end"#, r#"s1 "<b>" </script>"#);
    let tt = timetable_of(&[(stop, 0.0), ("s2", 1_000.0)], &[("r0", &[(stop, 100), ("s2", 200)])]);
    let page = mapped(&tt, &[("t", 0.0, 1_000.0)], title, "map-escape.html");
    assert_eq!(page.html.matches("</script>").count(), 3);
    assert_eq!(page.meta.get("title").text(), title);
    let stops: Vec<&str> = page.meta.get("stops").list().iter().map(common::Json::text).collect();
    assert_eq!(stops, [stop, "s2"]);
    assert!(
        page.html.contains("<title>map &quot;&lt;i&gt;&quot; &lt;/script&gt; &amp; end</title>")
    );
    assert_eq!(page.u32("tl"), [1]);
}

/// The names beside the timetable, as the approved page shows them: the case of
/// `an_access_walk_is_a_leg_and_two_patterns_share_a_hop` with `rA` a tram of line "1" and `rB` a
/// bus of line "19", and the stops named. The hop s1→s2 is ridden by a tram and by a bus: two hops,
/// kept apart by kind (tram 2, bus 3), each of one traveller; s2→s3 by the tram. Listed by riders,
/// then stops, then kind. The lines held in the order the pairs' rides meet them: "1" (pair 1,
/// `tA`), "19" (pair 2, `tB`). Places: `tA`'s origin (300, 0) is 300 m from s1, beyond 60 m:
/// "near Delft Station"; its destination is s3 itself: "Campus"; `tB`'s doors are s1 and s2.
#[test]
fn kinds_lines_and_names_are_as_the_approved_page_shows_them() {
    use openmobisim_io_gtfs::{Kind, Line, Lines};
    let tt = timetable_of(
        &[("s1", 0.0), ("s2", 1_000.0), ("s3", 2_000.0)],
        &[("rA", &[("s1", 300), ("s2", 400), ("s3", 500)]), ("rB", &[("s1", 100), ("s2", 200)])],
    );
    let mut lines = Lines::default();
    for (stop, name) in [("s1", "Delft Station"), ("s2", "Markt"), ("s3", "Campus")] {
        lines.stop_names.insert(stop.to_owned(), name.to_owned());
    }
    lines.trip_lines.insert("rA".to_owned(), Line { name: "1".to_owned(), kind: Kind::Tram });
    lines.trip_lines.insert("rB".to_owned(), Line { name: "19".to_owned(), kind: Kind::Bus });
    let net = network();
    let rows: Vec<TripRow> = [("tA", 300.0, 2_000.0), ("tB", 0.0, 1_000.0)]
        .iter()
        .map(|&(id, from, to)| TripRow {
            id: id.to_string(),
            origin: Position::Metres(Xy::new(from, 0.0)),
            destination: Position::Metres(Xy::new(to, 0.0)),
            departure: Second(0),
            class: "all".to_string(),
            modes: ModeSet::of(&[Mode::Transit]),
        })
        .collect();
    let dem = Demand::build(&rows, &["all".to_string()], &net, Some(&tt), Metres(1_500.0))
        .expect("a demand");
    let r = run(&net, &dem, Some(&tt), (Second(0), Second(3_600)), &parameters()).expect("a run");
    let kept = common::kept(&r, &net, &dem, Some(&tt), &parameters(), "map-kinds.run");
    let mapped = Mapped {
        kept: &kept,
        network: &net,
        timetable: Some(&tt),
        lines: Some(&lines),
        record: "",
    };
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("map-transit-names.html");
    write(&mapped, "names", &path).expect("written");
    let page = common::read(&path);
    let texts = |key: &str| -> Vec<String> {
        page.meta.get(key).list().iter().map(|j| j.text().to_owned()).collect()
    };
    assert_eq!(texts("stops"), ["Delft Station", "Markt", "Campus"]);
    assert_eq!(texts("kinds"), ["rail", "metro", "tram", "bus", "ferry", "other"]);
    assert_eq!(page.u32("ta"), [0, 0, 1]);
    assert_eq!(page.u32("tb"), [1, 1, 2]);
    assert_eq!(page.u8("tk"), [2, 3, 2]);
    assert_eq!(page.u32("tl"), [1, 1, 1]);
    assert_eq!(texts("lines"), ["1", "19"]);
    let kinds: Vec<f64> =
        page.meta.get("line_kinds").list().iter().map(common::Json::number).collect();
    assert_eq!(kinds, [2.0, 3.0]);
    let n = u32::MAX;
    assert_eq!(page.u32("lline"), [n, n, 0, n, n, n, 1, n]);
    assert_eq!(page.places(), [["near Delft Station", "Campus"], ["Delft Station", "Markt"]]);
}
