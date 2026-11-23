//! The map of the hand run of `crates/io-parquet/tests/hand_run.rs`, read back from the written
//! file (the two script blocks found, the block decoded from base64 and inflated) and checked
//! against the hand numbers derived there and below. Nothing here is the simulator's own output.
//!
//! The network: nodes a (0, 0), b (1 500, 0), c (2 500, 0) metres, ids in that order; links by
//! sorted id: `ab` a→b, 1 500 m, bikes and walkers; `st1` a→b, 2 000 m, one lane, 50 km/h
//! (144 s), 1 800 veh/h; `st2` b→c, 1 000 m, one lane, 36 km/h (100 s), 900 veh/h. The window
//! 07:00 to 08:00 (25 200 s to 28 800 s of the clock), bins of 60 s: 60 bins. Seven trips, all
//! from a: `car1`, `car2`, `car3` to c at 0, 1, 2 s; `bike`, `walk` to b at 0 s; `walk-late` to b
//! at 3 000 s; `lost` to b, transit only and no timetable.
//!
//! By hand (`hand_run.rs` of io-parquet): the cars enter `st1` at 0, 1, 2 s, leave it at 144,
//! 146, 148 s (crossing times 144, 145, 146: sum 435 s, all in bin 0), enter `st2` then and leave
//! at 244, 248, 252 s (100, 102, 104: sum 306 s, in bin 2, from 120 s); door to door 244, 247,
//! 250 s, mean 247 s. The bike 360 s, the walker 1 125 s, both on `ab`; `walk-late` on `ab` and
//! still walking at the window's end; `lost` without an alternative.
//!
//! The rules (`src/rules.rs`): drawing class 4 for `ab` (no car), 2 for `st1` (1 800 ≥ 1 600),
//! 3 for `st2` (900, another car link). Places are the doors: a→b holds 4 trips, a→c 3, so pair 1 is a→b (4
//! travellers: the walk journey of 2 travellers, 1 finished at 1 125 s; the bike journey of 1 at
//! 360 s; 1 without an alternative) and pair 2 is a→c (the car journey `st1`, `st2` of 3, 247 s).
//! Every door is on its node: each journey's gap walks are 0 s.

#![allow(clippy::float_cmp, reason = "whole numbers and values a float holds exactly")]

mod common;

use std::path::PathBuf;

use openmobisim_demand::demand::Demand;
use openmobisim_demand::input::TripRow;
use openmobisim_graph::geometry::{Position, Xy};
use openmobisim_graph::network::{BuildValues, LinkRow, Network, NetworkBuilder};
use openmobisim_loop::parameters::{ChoiceModel, Parameters};
use openmobisim_loop::run::run;
use openmobisim_map::{Mapped, write};
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

/// The hand run's map, written to `name` in the test folder.
fn written(name: &str) -> PathBuf {
    let net = network();
    let dem = demand(&net);
    let parameters = Parameters {
        link_bin: 60,
        iteration_cap: 3,
        choice: ChoiceModel::Deterministic,
        coefficients: vec![TIME_ONLY; 2],
        ..Parameters::defaults(2)
    };
    let window = (Second(START), Second(START + 3_600));
    let r = run(&net, &dem, None, window, &parameters).expect("a run");
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    write(
        &Mapped {
            kept: &common::kept(&r, &net, &dem, None, &parameters, &format!("{name}.run")),
            network: &net,
            timetable: None,
            lines: None,
            record: "",
        },
        "hand run",
        &path,
    )
    .expect("written");
    path
}

#[test]
fn the_network_and_its_traffic_are_the_hand_values() {
    let page = common::read(&written("map-hand-run.html"));
    let m = &page.meta;
    assert_eq!(m.get("title").text(), "hand run");
    assert_eq!(m.get("n_nodes").number(), 3.0);
    assert_eq!(m.get("n_links").number(), 3.0);
    assert_eq!(m.get("n_bins").number(), 60.0);
    assert_eq!(m.get("bin_seconds").number(), 60.0);
    assert_eq!(m.get("window_start").number(), 25_200.0);
    assert_eq!(m.get("trips").number(), 7.0);
    // The delay's floor the page bands by (the user's ruling): 2 s per car.
    assert_eq!(m.get("delay_floor_s").number(), 2.0);
    let origin: Vec<f64> = m.get("origin").list().iter().map(common::Json::number).collect();
    assert_eq!(origin, [0.0, 0.0]);
    // Nodes in decimetres from the origin: a 0, b 15 000, c 25 000.
    assert_eq!(page.i32("nx"), [0, 15_000, 25_000]);
    assert_eq!(page.i32("ny"), [0, 0, 0]);
    // Links ab, st1, st2: a→b, a→b, b→c.
    assert_eq!(page.u32("lu"), [0, 0, 1]);
    assert_eq!(page.u32("ld"), [1, 1, 2]);
    assert_eq!(page.u8("lm"), [2 + 4, 1, 1]);
    assert_eq!(page.u8("cls"), [4, 2, 3]);
    assert_eq!(page.f32("len"), [1_500.0, 2_000.0, 1_000.0]);
    assert_eq!(page.f32("cap"), [0.0, 1_800.0, 900.0]);
    assert_eq!(page.u8("lanes"), [0, 1, 1]);
    let (ff, spd) = (page.f32("ff"), page.f32("spd"));
    for (got, hand) in [(ff[0], 0.0), (ff[1], 144.0), (ff[2], 100.0), (spd[0], 0.0)] {
        assert!((got - hand).abs() < 1e-3, "{got} against {hand}");
    }
    assert!((spd[1] - 50.0).abs() < 1e-3 && (spd[2] - 36.0).abs() < 1e-3, "{spd:?}");
    // Cars per bin: st1 in bin 0 (3 cars, 435 s in all), st2 in bin 2 (3 cars, 306 s).
    let bs = page.u32("bs");
    assert_eq!(bs.len(), 61);
    assert_eq!(&bs[..4], [0, 1, 1, 2]);
    assert!(bs[4..].iter().all(|&b| b == 2), "{bs:?}");
    assert_eq!(page.u32("rl"), [1, 2]);
    assert_eq!(page.u32("rp"), [3, 3]);
    assert_eq!(page.f32("rs"), [435.0, 306.0]);
    // Over the window: one bike and two walkers on ab, none elsewhere.
    assert_eq!(page.u32("bv"), [1, 0, 0]);
    assert_eq!(page.u32("wv"), [2, 0, 0]);
    // No transit.
    assert!(page.u32("tl").is_empty() && page.i32("tsx").is_empty());
    assert!(m.get("stops").list().is_empty());
}

#[test]
fn the_pairs_their_journeys_and_each_links_pairs_are_the_hand_values() {
    let page = common::read(&written("map-hand-run-pairs.html"));
    assert_eq!(page.meta.get("n_pairs").number(), 2.0);
    assert_eq!(page.meta.get("pairs_total").number(), 2.0);
    // Pair 1 a→b, pair 2 a→c: doors in decimetres.
    assert_eq!(page.i32("qox"), [0, 0]);
    assert_eq!(page.i32("qoy"), [0, 0]);
    assert_eq!(page.i32("qdx"), [15_000, 25_000]);
    assert_eq!(page.i32("qdy"), [0, 0]);
    assert_eq!(page.u32("qn"), [4, 3]);
    assert_eq!(page.u32("qr"), [1, 0]);
    assert_eq!(page.u32("qs"), [0, 2, 3]);
    // The journeys: walk (2, 1 finished), bike (1); car (3). Modes car 0, bike 1, walk 2.
    assert_eq!(page.u8("om"), [2, 1, 0]);
    assert_eq!(page.u32("on"), [2, 1, 3]);
    assert_eq!(page.u32("of"), [1, 1, 3]);
    assert_eq!(page.f32("ot"), [1_125.0, 360.0, 247.0]);
    // Each a gap walk of 0 s, its route, a gap walk of 0 s: kinds walk 4, then drive 0, bike 1,
    // walk 2 on the network.
    assert_eq!(page.u32("ol"), [0, 3, 6, 9]);
    assert_eq!(page.u8("lk"), [4, 2, 4, 4, 1, 4, 4, 0, 4]);
    assert_eq!(page.f32("lsec"), [0.0, 1_125.0, 0.0, 0.0, 360.0, 0.0, 0.0, 247.0, 0.0]);
    let none = u32::MAX;
    assert_eq!(page.u32("la"), [none, 0, none, none, 1, none, none, 2, none]);
    assert_eq!(page.u32("lb"), [none, 1, none, none, 2, none, none, 4, none]);
    // The links: ab walked, ab ridden, st1 then st2 driven.
    assert_eq!(page.u32("pl"), [0, 0, 1, 2]);
    assert!(page.u32("pst").is_empty());
    // A link's pairs, by car, bike and walk together: ab, pair 1 with its 3 travellers on it (2
    // walkers, 1 bike); st1 and st2, pair 2 with 3.
    let users = page.link_users();
    assert_eq!(users.len(), 3);
    assert_eq!(users[&0], [(0, 3)]);
    assert_eq!(users[&1], [(1, 3)]);
    assert_eq!(users[&2], [(1, 3)]);
}

#[test]
fn the_file_names_no_address_and_a_second_write_is_the_same_bytes() {
    let (a, b) = (written("map-hand-run-a.html"), written("map-hand-run-b.html"));
    let (a, b) = (std::fs::read(a).expect("a"), std::fs::read(b).expect("b"));
    assert!(a == b, "two writes of the same run differ");
    let text = String::from_utf8(a).expect("UTF-8");
    for address in ["http:", "https:", "ftp:", "@import", "url("] {
        assert!(!text.contains(address), "{address} in the file");
    }
    // Every element is in the file itself: no source or link to another file.
    for attribute in [" src=", " href="] {
        assert!(!text.contains(attribute), "{attribute} in the file");
    }
}

/// The names the page shows: each held pair's two places by release 0.1's rule (no stop: where
/// the place lies from the centre, the mean of the links' ends: a, b, a, b, b, c, so x = 7 000 / 6
/// = 1 166.7 m: a is 1.2 km W, b 0.3 km E, c 1.3 km E of it); the links' and nodes' external ids;
/// and, from a hand record, the provenance line and the source line.
#[test]
fn places_ids_and_the_records_lines_are_the_hand_values() {
    let net = network();
    let dem = demand(&net);
    let parameters = Parameters {
        link_bin: 60,
        iteration_cap: 3,
        choice: ChoiceModel::Deterministic,
        coefficients: vec![TIME_ONLY; 2],
        ..Parameters::defaults(2)
    };
    let r =
        run(&net, &dem, None, (Second(START), Second(START + 3_600)), &parameters).expect("a run");
    let record = "# a hand record\nformat = 1\nwindow_start_s = 25200\nwindow_end_s = 28800\n\n\
                  [parameters]\nchoice = \"deterministic\"  # name\nlink_bin = 60  # s\n\
                  seed = 0  # none\n\n[network]\n\n[result]\nopenmobisim_version = \"0.2.0-dev\"\n\
                  window_end_used_s = 28800\niterations = 3\nstopped_by_rule = false\n\n\
                  [result.network]\nsource = \"osm\"\nfingerprint = \"0123456789abcdef\"\n";
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("map-hand-run-names.html");
    let kept = common::kept(&r, &net, &dem, None, &parameters, "map-hand-run-names.run");
    let mapped = Mapped { kept: &kept, network: &net, timetable: None, lines: None, record };
    write(&mapped, "hand run", &path).expect("written");
    let page = common::read(&path);
    assert_eq!(
        page.places(),
        [
            ["1.2 km W of the centre", "0.3 km E of the centre"],
            ["1.2 km W of the centre", "1.3 km E of the centre"]
        ]
    );
    let distinct: Vec<&str> =
        page.meta.get("place_names").list().iter().map(common::Json::text).collect();
    assert_eq!(
        distinct,
        ["1.2 km W of the centre", "0.3 km E of the centre", "1.3 km E of the centre"]
    );
    assert_eq!(page.texts("lid"), ["ab", "st1", "st2"]);
    assert_eq!(page.texts("nid"), ["a", "b", "c"]);
    assert_eq!(
        page.meta.get("provenance").text(),
        "0.2.0-dev · window 07:00–08:00 · bin 60 s · choice deterministic · 3 it, ended by the cap · seed 0 · fingerprint 01234567"
    );
    assert_eq!(page.meta.get("source").text(), "© OpenStreetMap contributors (ODbL)");
    // The size shown is the whole file's, in MB of 10^6 bytes, to the tenth shown.
    let bytes = std::fs::metadata(&path).expect("the file").len();
    #[allow(clippy::cast_precision_loss, reason = "a file's length, far below 2^53")]
    let shown = (bytes as f64 / 1e5).round() / 10.0;
    assert_eq!(page.meta.get("megabytes").number(), shown, "{bytes} bytes");
}
