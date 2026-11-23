//! The display rules by hand, each on a small run read back from its written map: walks between
//! a door and the network, the drawing classes at their thresholds, the road classes and a
//! link's shape, the cyclists' and walkers' widths, the pairs beyond those the page holds, and a
//! journey's time as a mean.
//!
//! The network: nodes a (0, 0), b (1 500, 0), c (2 500, 0) metres; `ab` a→b, 1 500 m, bikes and
//! walkers (a bike rides it in 360 s at 15 km/h, a walker walks it in 1 125 s at 4.8 km/h, a metre
//! in 0.75 s); `st1` a→b, 2 000 m, one lane at 50 km/h (144 s), 1 800 veh/h (a car every 2 s);
//! `st2` b→c, 1 000 m at 36 km/h (100 s), 900 veh/h (a car every 4 s). The window from 07:00
//! (25 200 s) for an hour, bins of 60 s, the least time taken.

#![allow(clippy::float_cmp, reason = "whole numbers and values a float holds exactly")]

mod common;

use std::path::PathBuf;

use common::Page;
use openmobisim_demand::demand::Demand;
use openmobisim_demand::input::TripRow;
use openmobisim_graph::geometry::{Position, Xy};
use openmobisim_graph::network::{BuildValues, LinkDrawing, LinkRow, Network, NetworkBuilder};
use openmobisim_loop::parameters::{ChoiceModel, Parameters};
use openmobisim_loop::run::run;
use openmobisim_map::rules::{FLOW_FULL, flow_share, road_rank};
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

fn path() -> LinkRow {
    LinkRow {
        modes: ModeSet::of(&[Mode::Bike, Mode::Walk]),
        lanes: None,
        car_speed_km_h: None,
        length_m: Some(1_500.0),
        capacity_veh_h: None,
        signal_at_downstream_end: false,
    }
}

/// The network of the header, with `more` links a→b besides.
fn network(more: &[(&str, LinkRow)]) -> Network {
    drawn(more, &[])
}

/// The network of the header, with `more` links a→b besides and `drawings`.
fn drawn(more: &[(&str, LinkRow)], drawings: &[(&str, LinkDrawing)]) -> Network {
    let mut b = NetworkBuilder::new();
    for (id, x) in [("a", 0.0), ("b", 1_500.0), ("c", 2_500.0)] {
        b.add_node(id, Position::Metres(Xy::new(x, 0.0)));
    }
    b.add_link("ab", "a", "b", path());
    b.add_link("st1", "a", "b", street(2_000.0, 50.0, 1_800.0));
    b.add_link("st2", "b", "c", street(1_000.0, 36.0, 900.0));
    for (id, row) in more {
        b.add_link(*id, "a", "b", *row);
    }
    for (id, d) in drawings {
        b.add_drawing(*id, d.clone());
    }
    b.build(VALUES).expect("a network")
}

/// A trip `id` from `from` to `to` (metres), leaving `after` seconds after 07:00 by `mode`.
fn trip(id: &str, from: (f64, f64), to: (f64, f64), after: u32, mode: Mode) -> TripRow {
    TripRow {
        id: id.to_string(),
        origin: Position::Metres(Xy::new(from.0, from.1)),
        destination: Position::Metres(Xy::new(to.0, to.1)),
        departure: Second(START + after),
        class: "all".to_string(),
        modes: ModeSet::of(&[mode]),
    }
}

/// The run of `rows` on `net`, its map written to `name` and read back.
fn map(net: &Network, rows: &[TripRow], name: &str) -> Page {
    let classes = ["all".to_string()];
    let dem = Demand::build(rows, &classes, net, None, Metres(400.0)).expect("a demand");
    let parameters = Parameters {
        link_bin: 60,
        iteration_cap: 3,
        choice: ChoiceModel::Deterministic,
        coefficients: vec![TIME_ONLY],
        ..Parameters::defaults(1)
    };
    let window = (Second(START), Second(START + 3_600));
    let r = run(net, &dem, None, window, &parameters).expect("a run");
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    write(
        &Mapped {
            kept: &common::kept(&r, net, &dem, None, &parameters, &format!("{name}.run")),
            network: net,
            timetable: None,
            lines: None,
            record: "",
        },
        name,
        &path,
    )
    .expect("written");
    common::read(&path)
}

fn close(got: &[f32], hand: &[f64]) {
    assert_eq!(got.len(), hand.len(), "{got:?} against {hand:?}");
    for (g, h) in got.iter().zip(hand) {
        assert!((f64::from(*g) - h).abs() < 1e-3, "{got:?} against {hand:?}");
    }
}

/// Doors 300 m from a and 400 m from b: each journey walks 300 m (225 s) to its first node and
/// 400 m (300 s) from its last. The car enters `st1` at 225 s, leaves it at 369 s, at its door at
/// 669 s; the bike rides 360 s, at its door at 885 s; the walker walks 1 125 s, at its door at
/// 1 650 s. One pair of three journeys of one traveller each, in trip id order (bike, car, walk).
#[test]
fn walks_between_the_doors_and_the_network_are_legs_of_their_journeys() {
    let (from, to) = ((0.0, 300.0), (1_500.0, 400.0));
    let rows = [
        trip("g-bike", from, to, 0, Mode::Bike),
        trip("g-car", from, to, 0, Mode::Car),
        trip("g-walk", from, to, 0, Mode::Walk),
    ];
    let page = map(&network(&[]), &rows, "map-gaps.html");
    assert_eq!(page.u32("qn"), [3]);
    assert_eq!(page.i32("qoy"), [3_000]);
    assert_eq!(page.i32("qdy"), [4_000]);
    assert_eq!(page.u8("om"), [1, 0, 2]);
    assert_eq!(page.u32("of"), [1, 1, 1]);
    close(&page.f32("ot"), &[885.0, 669.0, 1_650.0]);
    assert_eq!(page.u8("lk"), [4, 1, 4, 4, 0, 4, 4, 2, 4]);
    close(&page.f32("lsec"), &[225.0, 360.0, 300.0, 225.0, 144.0, 300.0, 225.0, 1_125.0, 300.0]);
    assert_eq!(page.u32("pl"), [0, 1, 0]);
}

/// Car links a→b of 3 600, 3 599, 2 400, 2 399, 1 600 and 1 599 veh/h and of no limit: classes
/// 0, 1, 1, 2, 2, 3 and 0 (no limit reaches every threshold); `ab` without cars 4. Links by
/// sorted id: `ab`, `k1599`, `k1600`, `k2399`, `k2400`, `k3599`, `k3600`, `kinf`, `st1`, `st2`.
#[test]
fn the_drawing_classes_are_cut_at_3600_2400_and_1600_veh_h() {
    let more: Vec<(&str, LinkRow)> = [
        ("k3600", 3_600.0),
        ("k3599", 3_599.0),
        ("k2400", 2_400.0),
        ("k2399", 2_399.0),
        ("k1600", 1_600.0),
        ("k1599", 1_599.0),
        ("kinf", f64::INFINITY),
    ]
    .into_iter()
    .map(|(id, c)| (id, street(2_000.0, 50.0, c)))
    .collect();
    let rows = [trip("w", (0.0, 0.0), (1_500.0, 0.0), 0, Mode::Walk)];
    let page = map(&network(&more), &rows, "map-classes.html");
    assert_eq!(page.u8("cls"), [4, 3, 2, 2, 1, 1, 0, 0, 2, 3]);
    assert_eq!(
        page.f32("cap"),
        [0.0, 1_599.0, 1_600.0, 2_399.0, 2_400.0, 3_599.0, 3_600.0, 0.0, 1_800.0, 900.0]
    );
}

/// Version 1's order of road classes: `steps` drawn as a footway (15), `primary` 4, a name of
/// neither as `unclassified` (10); each link's name among the distinct names in sorted order
/// (`mystery`, `primary`, `steps`). `st1`'s shape, one point at (750, 300) m: 7 500 and 3 000
/// decimetres from the origin (a), the other links straight.
#[test]
fn a_road_class_on_every_link_draws_by_version_1s_order_and_the_shape_is_held() {
    let class = |c: &str, shape: Vec<Position>| LinkDrawing { shape, road_class: Some(c.into()) };
    let bend = vec![Position::Metres(Xy::new(750.0, 300.0))];
    let drawings = [
        ("ab", class("steps", vec![])),
        ("st1", class("primary", bend)),
        ("st2", class("mystery", vec![])),
    ];
    let rows = [trip("w", (0.0, 0.0), (1_500.0, 0.0), 0, Mode::Walk)];
    let page = map(&drawn(&[], &drawings), &rows, "map-road-classes.html");
    assert_eq!(page.u8("cls"), [15, 4, 10]);
    assert_eq!(page.u32("rc"), [2, 1, 0]);
    let names: Vec<&str> =
        page.meta.get("road_classes").list().iter().map(common::Json::text).collect();
    assert_eq!(names, ["mystery", "primary", "steps"]);
    assert_eq!(page.u32("vs"), [0, 0, 1, 1]);
    assert_eq!((page.i32("dx"), page.i32("dy")), (vec![7_500], vec![3_000]));
    assert_eq!([road_rank("motorway"), road_rank("road"), road_rank("cycleway")], [0, 10, 16]);
}

/// One link without a class: the page draws by capacity (the classes of the header's network,
/// 4, 2 and 3) and holds no road class.
#[test]
fn a_network_with_a_link_unclassed_draws_by_capacity() {
    let classed = LinkDrawing { shape: vec![], road_class: Some("primary".into()) };
    let drawings = [("st1", classed.clone()), ("st2", classed)];
    let rows = [trip("w", (0.0, 0.0), (1_500.0, 0.0), 0, Mode::Walk)];
    let page = map(&drawn(&[], &drawings), &rows, "map-part-classed.html");
    assert_eq!(page.u8("cls"), [4, 2, 3]);
    assert!(page.meta.get("road_classes") == &common::Json::Null);
    assert!(page.u32("vs").iter().all(|&v| v == 0));
}

/// The cyclists' and walkers' width by hand: the share √(min(n, 3 000) / 3 000) of 0, 750, 3 000
/// and 6 000 travellers is 0, 0.5, 1 and 1: on the map 1, 4.5, 8 and 8 px (1 + 7 × share), on the
/// summary's maps 1.5, 5.5, 9.5 and 9.5 units (1.5 + 8 × share). The page reads the same number.
#[test]
fn the_cyclists_and_walkers_width_is_full_at_3000_for_both() {
    assert_eq!(FLOW_FULL, 3_000);
    let shares: Vec<f64> = [0, 750, 3_000, 6_000].into_iter().map(flow_share).collect();
    assert_eq!(shares, [0.0, 0.5, 1.0, 1.0]);
    let px: Vec<f64> = shares.iter().map(|s| 1.0 + 7.0 * s).collect();
    assert_eq!(px, [1.0, 4.5, 8.0, 8.0]);
    let rows = [trip("w", (0.0, 0.0), (1_500.0, 0.0), 0, Mode::Walk)];
    let page = map(&network(&[]), &rows, "map-flow-full.html");
    assert_eq!(page.meta.get("flow_full").number(), 3_000.0);
}

/// 2 006 walkers to b: three pairs of two (`a0`, `a1` from (0, 10); `b0`, `b1` from (0, 20);
/// `c0`, `c1` from (0, 30)) and 2 000 of one, `s0000` to `s1999`, from (0, −1 − i / 4). 2 003
/// pairs; the page holds 2 000: the three of two, then the singles of the lowest ids, `s0000`
/// to `s1996` (from (0, −500) the last), so it holds 6 + 1 997 = 2 003 of the 2 006 trips.
#[test]
fn the_page_holds_the_2000_busiest_pairs_and_says_of_how_many() {
    let to = (1_500.0, 0.0);
    let mut rows = Vec::new();
    for (k, name) in ["a", "b", "c"].into_iter().enumerate() {
        let y = 10.0 * f64::from(u8::try_from(k).expect("small") + 1);
        rows.push(trip(&format!("{name}0"), (0.0, y), to, 0, Mode::Walk));
        rows.push(trip(&format!("{name}1"), (0.0, y), to, 0, Mode::Walk));
    }
    for i in 0..2_000_u32 {
        let y = -1.0 - f64::from(i) / 4.0;
        rows.push(trip(&format!("s{i:04}"), (0.0, y), to, 0, Mode::Walk));
    }
    let page = map(&network(&[]), &rows, "map-pairs-held.html");
    assert_eq!(page.meta.get("trips").number(), 2_006.0);
    assert_eq!(page.meta.get("n_pairs").number(), 2_000.0);
    assert_eq!(page.meta.get("pairs_total").number(), 2_003.0);
    let (qn, qoy) = (page.u32("qn"), page.i32("qoy"));
    assert_eq!(qn.len(), 2_000);
    assert_eq!(qn.iter().sum::<u32>(), 2_003);
    assert_eq!(&qn[..4], [2, 2, 2, 1]);
    assert_eq!(&qoy[..4], [100, 200, 300, -10]);
    assert_eq!(qoy[1_999], -5_000);
}

/// Three cars a→c leaving at 0, 0 and 5 s (`m1` before `m2` at the same instant, the lower id):
/// on `st1` they leave at 144, max(144, 144 + 2) = 146 and max(149, 146 + 2) = 149 s; on `st2` at
/// 244, max(246, 244 + 4) = 248 and max(249, 248 + 4) = 252 s. Door to door 244, 248 and 247 s:
/// the mean 739 / 3 = 246.33 s, where the middle value is 247 s.
#[test]
fn a_journeys_time_is_the_mean_of_its_finished_travellers() {
    let to = (2_500.0, 0.0);
    let rows = [
        trip("m1", (0.0, 0.0), to, 0, Mode::Car),
        trip("m2", (0.0, 0.0), to, 0, Mode::Car),
        trip("m3", (0.0, 0.0), to, 5, Mode::Car),
    ];
    let page = map(&network(&[]), &rows, "map-mean.html");
    assert_eq!(page.u32("on"), [3]);
    assert_eq!(page.u32("of"), [3]);
    close(&page.f32("ot"), &[739.0 / 3.0]);
    close(&page.f32("lsec"), &[0.0, 739.0 / 3.0, 0.0]);
    assert_eq!(page.u32("rp"), [3, 3]);
    assert_eq!(page.f32("rs"), [144.0 + 146.0 + 144.0, 100.0 + 102.0 + 103.0]);
}

/// The delay rule by hand: 145 s against 144 s free flow is 1 / 144; a car faster than free flow
/// is 0, never below; a free-flow time of 0 gives 0.
#[test]
fn the_delay_is_the_mean_time_over_free_flow_less_one_never_below_0() {
    use openmobisim_map::rules::delay;
    assert!((delay(145.0, 144.0) - 1.0 / 144.0).abs() < 1e-15);
    assert_eq!(delay(288.0, 144.0), 1.0);
    assert_eq!(delay(100.0, 144.0), 0.0);
    assert_eq!(delay(5.0, 0.0), 0.0);
}

/// The delay's band by hand, the floor a fixed display rule of 2 s per car: a link crossed in
/// 0.433 s against 0.045 s (862 %, 0.39 s per car) and one of 10 s free flow crossed in 11.9 s
/// (19 %, 1.9 s) are in the first band; 12 s there (20 %, 2 s: at the floor) is in 15–50 %, 16 s
/// (60 %) in 50–100 %, 25 s (150 %) in 100 %+; 145 s against 144 s (0.7 %, 1 s) in the first, 150 s
/// (4 %, 6 s: past the floor, under 15 %) in the first, 180 s (25 %, 36 s) in 15–50 %, 100 s (faster
/// than free flow) in the first; a free-flow time of 0 in the first.
#[test]
fn a_link_whose_cars_lose_under_2_s_each_is_in_the_first_band() {
    use openmobisim_map::rules::{DELAY_FLOOR_S, delay_band};
    assert_eq!(DELAY_FLOOR_S, 2.0);
    let bands: Vec<usize> = [
        (0.433, 0.045),
        (11.9, 10.0),
        (12.0, 10.0),
        (16.0, 10.0),
        (25.0, 10.0),
        (145.0, 144.0),
        (150.0, 144.0),
        (180.0, 144.0),
        (100.0, 144.0),
        (5.0, 0.0),
    ]
    .iter()
    .map(|&(mean, free)| delay_band(mean, free))
    .collect();
    assert_eq!(bands, [0, 0, 1, 2, 3, 0, 0, 1, 0, 0]);
}
