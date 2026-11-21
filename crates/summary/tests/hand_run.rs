//! The summary of io-parquet's hand run (`crates/io-parquet/tests/hand_run.rs`, `read_back.py`),
//! every value a test reads checked against the hand numbers derived here.
//!
//! The run: nodes a (0, 0), b (1 500, 0), c (2 500, 0) m; `ab` a→b 1 500 m for bikes and walkers;
//! `st1` a→b 2 000 m at 50 km/h (144 s), 1 800 veh/h; `st2` b→c 1 000 m at 36 km/h (100 s),
//! 900 veh/h; the window 07:00–08:00 in bins of 60 s, three iterations (the cap: the stopping rule
//! reads five), time the only cost. Classes `commute` (the three cars) and `leisure` (the rest).
//! - Trips 7: cars at 244, 247 and 250 s, the bike at 360 s, `walk` at 1 125 s: 5 finished
//!   (71.43 %); `walk-late` still walking at 08:00 and `lost` without an alternative (1 each, 2 in
//!   all: 28.57 %). The mean of the finished, 2 226 / 5 = 445.2 s = 7.4 min; 2 226 s = 0.6 h.
//! - Cars: 3 on `st1` (6 km) and 3 on `st2` (3 km): 9.0 vehicle-km; their crossing times 144 +
//!   145 + 146 = 435 s and 100 + 102 + 104 = 306 s, 741 s = 0.20583 h: 9 / 0.20583 = 43.7 km/h.
//!   Delays 145 / 144 − 1 = 0.7 % and 102 / 100 − 1 = 2.0 %: all 9.0 km in the band 0–15 %.
//!   Volumes 3 cars over the hour: 3 veh/h each; `st1` first, the lower id at equal volume.
//! - Modes: car 3 (42.9 %, all finished, 247 / 60 = 4.1 min, 741 s = 0.2 h); bike 1 (14.3 %, 6.0
//!   min, 0.1 h); walk 2 (28.6 %, 50.0 % finished, 18.75 min shown 18.8, 0.3 h); no alternative 1
//!   (14.3 %). By class: `commute` 3 trips, none without; `leisure` 4, 1 without (25.00 %).
//! - Departures by quarter hour: 07:00 the 3 cars, the bike, `walk` and `lost` (6); 07:45
//!   `walk-late` (at 07:50); 07:15 and 07:30 none.
//! - Kilometres cycled 1 × 1.5 = 1.5; walked 2 × 1.5 = 3.0.
//! - Iterations (`read_back.py`): mean 445.2 s = 7.42 min each; at iteration 0 everyone offered
//!   and found (100 %), nobody switched; 1 unfinished; gap 0. Under the least cost the share that
//!   would change is not reported: "—" at every iteration. Under the logit (the last test), each
//!   set holding one alternative, nobody would change (0.00 %) and the gap is not reported: "—".
//! - The network: car 2 links, 3 nodes, 3.0 km; bike and walk each 1 link, 2 nodes, 1.5 km.

mod common;

use std::path::PathBuf;

use common::{has, value};
use openmobisim_demand::demand::Demand;
use openmobisim_demand::input::TripRow;
use openmobisim_graph::geometry::{Position, Xy};
use openmobisim_graph::network::{BuildValues, LinkRow, Network, NetworkBuilder};
use openmobisim_loop::assess::Iteration;
use openmobisim_loop::parameters::{ChoiceModel, Parameters};
use openmobisim_loop::run::run;
use openmobisim_summary::{Summarised, write};
use openmobisim_types::mode::{Mode, ModeSet};
use openmobisim_types::time::Second;
use openmobisim_types::units::Metres;

const VALUES: BuildValues =
    BuildValues { capacity_per_lane: 1_800.0, bike_speed: 15.0, walk_speed: 4.8 };
const TIME_ONLY: [f64; 16] =
    [1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
const START: u32 = 25_200;

fn street(length_m: f64, speed_km_h: f64, capacity_veh_h: f64) -> LinkRow {
    LinkRow {
        modes: ModeSet::of(&[Mode::Car]),
        lanes: Some(1),
        car_speed_km_h: Some(speed_km_h),
        length_m: Some(length_m),
        capacity_veh_h: Some(capacity_veh_h),
        signal_at_downstream_end: false,
    }
}

fn network() -> Network {
    let mut b = NetworkBuilder::new();
    for (id, x) in [("a", 0.0), ("b", 1_500.0), ("c", 2_500.0)] {
        b.add_node(id, Position::Metres(Xy::new(x, 0.0)));
    }
    let path = LinkRow {
        modes: ModeSet::of(&[Mode::Bike, Mode::Walk]),
        lanes: None,
        car_speed_km_h: None,
        length_m: Some(1_500.0),
        capacity_veh_h: None,
        signal_at_downstream_end: false,
    };
    b.add_link("ab", "a", "b", path);
    b.add_link("st1", "a", "b", street(2_000.0, 50.0, 1_800.0));
    b.add_link("st2", "b", "c", street(1_000.0, 36.0, 900.0));
    b.build(VALUES).expect("a network")
}

fn demand(network: &Network) -> Demand {
    let row = |id: &str, to: f64, departure: u32, class: &str, mode: Mode| TripRow {
        id: id.to_string(),
        origin: Position::Metres(Xy::new(0.0, 0.0)),
        destination: Position::Metres(Xy::new(to, 0.0)),
        departure: Second(departure),
        class: class.to_string(),
        modes: ModeSet::of(&[mode]),
    };
    let rows = [
        row("car1", 2_500.0, START, "commute", Mode::Car),
        row("car2", 2_500.0, START + 1, "commute", Mode::Car),
        row("car3", 2_500.0, START + 2, "commute", Mode::Car),
        row("bike", 1_500.0, START, "leisure", Mode::Bike),
        row("walk", 1_500.0, START, "leisure", Mode::Walk),
        row("walk-late", 1_500.0, START + 3_000, "leisure", Mode::Walk),
        row("lost", 1_500.0, START, "leisure", Mode::Transit),
    ];
    let classes = ["commute".to_string(), "leisure".to_string()];
    Demand::build(&rows, &classes, network, None, Metres(400.0)).expect("a demand")
}

/// The hand run's summary, written once to each of `names` in the test folder; their texts.
fn written_to(names: &[&str]) -> Vec<String> {
    written_with(ChoiceModel::Deterministic, |_| {}, names)
}

/// The hand run under the choice model `choice`, its iterations table changed by `edit` (to show
/// a value the hand run does not give), its summary written once to each of `names` in the test
/// folder; their texts.
fn written_with(choice: ChoiceModel, edit: fn(&mut [Iteration]), names: &[&str]) -> Vec<String> {
    let net = network();
    let dem = demand(&net);
    let parameters = Parameters {
        link_bin: 60,
        iteration_cap: 3,
        choice,
        coefficients: vec![TIME_ONLY; 2],
        ..Parameters::defaults(2)
    };
    let window = (Second(START), Second(START + 3_600));
    let r = run(&net, &dem, None, window, &parameters).expect("a run");
    let mut iterations = r.iterations.clone();
    edit(&mut iterations);
    let kept = common::kept(
        &r,
        &net,
        &dem,
        None,
        &parameters,
        &format!("summary-{}.run", names.join("-")),
    );
    let record = "# the hand run's record\nformat = 1\nnote = \"<b> & </script>\"\n";
    let run = Summarised {
        kept: &kept,
        network: &net,
        timetable: None,
        lines: None,
        iterations: &iterations,
        stopped_by_rule: Some(r.stopped_by_rule),
        record,
    };
    names
        .iter()
        .map(|name| {
            let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
            write(&run, "hand run", &path).expect("written");
            std::fs::read_to_string(&path).expect("the summary")
        })
        .collect()
}

/// The hand run's summary, written to `name`; its text.
fn written(name: &str) -> String {
    written_to(&[name]).remove(0)
}

fn check(html: &str, expected: &[(&str, &str)]) {
    for (key, hand) in expected {
        assert_eq!(value(html, key), *hand, "{key}");
    }
}

#[test]
fn the_tiles_modes_and_departures_are_the_hand_values() {
    let html = written("summary-hand-run.html");
    check(
        &html,
        &[
            ("trips", "7"),
            ("completed", "71.43"),
            ("finished", "5"),
            ("not-completed", "28.57"),
            ("still-travelling", "1"),
            ("without-alternative", "1"),
            ("mean-trip", "7.4"),
            ("travel-time", "0.6"),
            ("vehicle-km", "9.0"),
            ("mean-speed", "43.7"),
        ],
    );
    check(
        &html,
        &[
            ("mode.car.trips", "3"),
            ("mode.car.share", "42.9"),
            ("mode.car.completed", "100.0"),
            ("mode.car.mean", "4.1"),
            ("mode.car.hours", "0.2"),
            ("mode.bike.trips", "1"),
            ("mode.bike.share", "14.3"),
            ("mode.bike.mean", "6.0"),
            ("mode.bike.hours", "0.1"),
            ("mode.walk.trips", "2"),
            ("mode.walk.share", "28.6"),
            ("mode.walk.completed", "50.0"),
            ("mode.walk.mean", "18.8"),
            ("mode.walk.hours", "0.3"),
            ("mode.none.trips", "1"),
            ("mode.none.share", "14.3"),
            ("mode.none.mean", "—"),
            ("class.0.trips", "3"),
            ("class.0.without", "0"),
            ("class.0.share", "0.00"),
            ("class.1.trips", "4"),
            ("class.1.without", "1"),
            ("class.1.share", "25.00"),
        ],
    );
    assert!(!has(&html, "mode.transit.trips"));
    check(
        &html,
        &[
            ("dep.07:00.car", "3"),
            ("dep.07:00.bike", "1"),
            ("dep.07:00.walk", "1"),
            ("dep.07:00.none", "1"),
            ("dep.07:00.all", "6"),
            ("dep.07:15.all", "0"),
            ("dep.07:30.all", "0"),
            ("dep.07:45.walk", "1"),
            ("dep.07:45.all", "1"),
            ("dep.all.car", "3"),
            ("dep.all.walk", "2"),
            ("dep.all.all", "7"),
        ],
    );
    assert!(!has(&html, "dep.08:00.all"));
}

#[test]
fn the_roads_bikes_walking_iterations_and_network_are_the_hand_values() {
    let html = written("summary-hand-run-roads.html");
    check(
        &html,
        &[
            ("band.0.km", "9.0"),
            ("band.0.share", "100.0"),
            ("band.1.km", "0.0"),
            ("band.3.share", "0.0"),
            ("busy.0.link", "st1"),
            ("busy.0.length", "2,000"),
            ("busy.0.volume", "3"),
            ("busy.0.delay", "0.7"),
            ("busy.1.link", "st2"),
            ("busy.1.length", "1,000"),
            ("busy.1.volume", "3"),
            ("busy.1.delay", "2.0"),
            ("km-cycled", "1.5"),
            ("km-walked", "3.0"),
        ],
    );
    assert!(!has(&html, "busy.2.link") && !has(&html, "boardings"));
    check(
        &html,
        &[
            ("it.0.mean", "7.42"),
            ("it.0.offered", "100.00"),
            ("it.0.found", "100.00"),
            ("it.0.would", "—"),
            ("it.0.switched", "0.00"),
            ("it.0.unfinished", "1"),
            ("it.0.gap", "0.00"),
            ("it.2.mean", "7.42"),
            ("it.2.found", "0.00"),
            ("it.2.would", "—"),
            ("it.2.switched", "0.00"),
            ("stopped-by", "The iteration cap"),
        ],
    );
    assert_eq!((0..10).filter(|k| has(&html, &format!("it.{k}.mean"))).count(), 3);
    check(
        &html,
        &[
            ("net.car.links", "2"),
            ("net.car.nodes", "3"),
            ("net.car.km", "3.0"),
            ("net.bike.links", "1"),
            ("net.bike.nodes", "2"),
            ("net.bike.km", "1.5"),
            ("net.walk.links", "1"),
            ("net.walk.nodes", "2"),
            ("net.walk.km", "1.5"),
        ],
    );
    assert!(has(&html, "map.car") && has(&html, "map.bike") && has(&html, "map.walk"));
    assert!(!has(&html, "map.transit") && !has(&html, "net.stops"));
}

#[test]
fn the_page_names_no_address_shows_the_record_whole_and_is_the_same_bytes_twice() {
    let mut texts = written_to(&["summary-a.html", "summary-b.html"]);
    let (a, b) = (texts.remove(0), texts.remove(0));
    assert!(a == b, "two writes of the same run differ");
    for address in ["http:", "https:", "ftp:", "@import", "url(", " src="] {
        assert!(!a.contains(address), "{address} in the file");
    }
    assert!(a.contains("note = &quot;&lt;b&gt; &amp; &lt;/script&gt;&quot;"));
    assert_eq!(a.matches("</script>").count(), 1);
    assert!(a.contains("<title>hand run</title>"));
}

/// Each chart's grid lines at round values, each label its line's value: the departures' highest
/// quarter hour holds 6 trips, so the axis reaches 10 (5 at half way); the cars entering links,
/// 3 a bin at most (bin 0 on st1, bin 2 on st2), reach 5 (2.5); the mean trip, 7.42 min, 10 (5).
/// A line's value is read back from its height on the chart: the plot runs from y = 12 (the top)
/// to y = 150 (0), charts 190 high with margins of 12 and 40.
#[test]
fn the_charts_grid_lines_are_round_and_labelled_with_their_values() {
    let html = written("summary-grid.html");
    let mut charts = Vec::new();
    for chart in html.split("<svg viewBox=\"0 0 520 190\"").skip(1) {
        let mut lines = Vec::new();
        for piece in chart.split("class=\"grid\"/>").collect::<Vec<_>>().windows(2) {
            let y: f64 = piece[0]
                .rsplit("y1=\"")
                .next()
                .and_then(|t| t.split('"').next())
                .expect("y1")
                .parse()
                .expect("y");
            let label =
                piece[1].split('>').nth(1).and_then(|t| t.split('<').next()).expect("label");
            let value = label.replace(',', "").parse::<f64>().expect("a number");
            lines.push((y, value, label.to_owned()));
        }
        charts.push(lines);
    }
    let labels: Vec<Vec<String>> =
        charts.iter().map(|c| c.iter().map(|l| l.2.clone()).collect()).collect();
    assert_eq!(labels, [["10", "5", "0"], ["5", "2.5", "0"], ["10", "5", "0"]]);
    for lines in &charts {
        let top = lines[0].1;
        for &(y, value, _) in lines {
            assert!((value - top * (150.0 - y) / 138.0).abs() < 1e-9, "{value} at y {y}");
        }
    }
}

/// The static maps on the hand run, by hand: the frame 2 000 units wide with a margin of 20, the
/// network 2 500 m wide, so 0.784 units a metre, x = 20 + 0.784 x; y = 20 on the line (the
/// network has no height: the frame 80 high). The base network once: a–b and b–c. Car traffic:
/// st1 and st2 in the band 0–15 % (0.7 % and 2.0 %); widths 2 + 6 √(capacity / 1 800 veh/h, the
/// 99th percentile of the two): 8 for st1 (1 800), 6.24, kept to the half unit, 6 for st2 (900);
/// each on the right of its travel, east, that is below the line on the frame, by a quarter of
/// its width in half units plus one: 5 and 4 units. The path ab: its one bike 1.5 wide
/// (1.5 + 8 √(1 / 3 000) = 1.65, kept to the half unit) on the line (no way back for bikes),
/// dashed 3.6 and 2.25; its two walkers 1.5 wide (1.5 + 8 √(2 / 3 000) = 1.71), dotted 0.1 and 3.
#[test]
fn the_static_maps_coordinates_sides_and_widths_are_the_hand_values() {
    let html = written("summary-maps.html");
    assert!(html.contains("<path id=\"net\" d=\"M20 20L1196 20M1196 20L1980 20\"/>"));
    let car = &html[html.find("data-k=\"map.car\"").expect("the car map")..];
    assert!(car.starts_with("data-k=\"map.car\" role=\"img\""));
    assert!(html.contains("<svg viewBox=\"0 0 2000 80\" class=\"map\" data-k=\"map.car\""));
    let car = &car[..car.find("</svg>").expect("its end")];
    assert!(
        car.contains("<path class=\"k-b0\" stroke-width=\"6\" d=\"M1196 24L1980 24\"/>"),
        "{car}"
    );
    assert!(
        car.contains("<path class=\"k-b0\" stroke-width=\"8\" d=\"M20 25L1196 25\"/>"),
        "{car}"
    );
    let bike = &html[html.find("data-k=\"map.bike\"").expect("the bike map")..];
    assert!(bike.contains(
        "<path class=\"k-bike\" stroke-width=\"1.5\" stroke-dasharray=\"3.6 2.25\" d=\"M20 20L1196 20\"/>"
    ));
    let walk = &html[html.find("data-k=\"map.walk\"").expect("the walk map")..];
    assert!(walk.contains(
        "<path class=\"k-walk\" stroke-width=\"1.5\" stroke-dasharray=\"0.1 3\" d=\"M20 20L1196 20\"/>"
    ));
}

/// Each model reports the one measure that can reach 0, and the other shows as missing ("—",
/// never "NaN"). Under the least cost (the test of the tiles above) the share that would change
/// is "—" and the gap 0.00. The hand run under the logit, the other parameters the same: each set
/// holds one alternative, which the logit chooses at any cost, so nobody would change, 0.00 % at
/// each of the three iterations, and the gap is "—".
#[test]
fn under_the_logit_the_gap_is_missing_and_nobody_would_change() {
    let html = written_with(ChoiceModel::Logit, |_| {}, &["summary-logit.html"]).remove(0);
    for k in 0..3 {
        assert_eq!(value(&html, &format!("it.{k}.would")), "0.00", "iteration {k}");
        assert_eq!(value(&html, &format!("it.{k}.gap")), "—", "iteration {k}");
    }
    let least_cost = written("summary-least-cost.html");
    for k in 0..3 {
        assert_eq!(value(&least_cost, &format!("it.{k}.would")), "—", "iteration {k}");
        assert_eq!(value(&least_cost, &format!("it.{k}.gap")), "0.00", "iteration {k}");
    }
    assert!(!html.contains("NaN") && !least_cost.contains("NaN"), "a NaN on a page");
}

/// The share is shown in per cent with two decimals: the logit run's iterations table with the
/// share of iteration 1 set to 0.25 by the test shows 25.00 there, and 0.00 at iterations 0 and 2.
#[test]
fn a_share_that_would_change_of_a_quarter_is_shown_as_25_per_cent() {
    let html = written_with(
        ChoiceModel::Logit,
        |rows| rows[1].would_change_share = 0.25,
        &["summary-quarter.html"],
    )
    .remove(0);
    let shown: Vec<String> = (0..3).map(|k| value(&html, &format!("it.{k}.would"))).collect();
    assert_eq!(shown, ["0.00", "25.00", "0.00"]);
}
