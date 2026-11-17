//! The network against hand derivations: free-flow times, capacities, lengths, the shape of a
//! grid, the order of ids, and the errors bad rows raise.
//!
//! The build values used throughout (`tests/common/mod.rs`): capacity_per_lane 1 800 veh/h,
//! bike_speed 15 km/h, walk_speed 4.8 km/h. They are test values, not defaults.

mod common;

use common::{ALL_LINK_MODES, VALUES, at, close, cycle_path, grid, node_name, row, two_nodes};
use openmobisim_graph::geometry::{LonLat, Position, UTM_K0};
use openmobisim_graph::network::{BuildError, BuildValues, LinkRow, Network, NetworkBuilder};
use openmobisim_types::ids::{EntityId, LinkId, NodeId};
use openmobisim_types::mode::{Mode, ModeSet};
use openmobisim_types::units::Flow;

fn only_link(net: &Network) -> LinkId {
    assert_eq!(net.link_count(), 1);
    LinkId::new(0)
}

fn time(net: &Network, mode: Mode) -> f64 {
    net.free_flow_time(only_link(net), mode).get()
}

#[test]
fn a_bike_rides_1500_m_at_15_km_h_in_360_s() {
    // 15 km/h = 15 000 m / 3 600 s = 4.1667 m/s; 1 500 m / 4.1667 m/s = 360 s.
    let net = two_nodes(LinkRow { length_m: Some(1_500.0), ..row(1, 50.0) }).build(VALUES).unwrap();
    assert!(close(time(&net, Mode::Bike), 360.0), "{}", time(&net, Mode::Bike));
}

#[test]
fn a_walker_covers_1000_m_at_4_8_km_h_in_750_s() {
    // 4.8 km/h = 4 800 m / 3 600 s = 1.3333 m/s; 1 000 m (the straight line) / 1.3333 m/s = 750 s.
    let net = two_nodes(row(1, 50.0)).build(VALUES).unwrap();
    assert!(close(time(&net, Mode::Walk), 750.0), "{}", time(&net, Mode::Walk));
}

#[test]
fn the_bike_and_walk_speeds_read_back_as_the_build_values_in_m_s() {
    // 36 km/h = 36 000 m / 3 600 s = 10 m/s; 7.2 km/h = 7 200 m / 3 600 s = 2 m/s; and on the
    // link, 1 000 m take 100 s by bike and 500 s on foot.
    let values = BuildValues { bike_speed: 36.0, walk_speed: 7.2, ..VALUES };
    let net = two_nodes(row(1, 50.0)).build(values).unwrap();
    assert!(close(net.bike_speed().get(), 10.0), "{}", net.bike_speed().get());
    assert!(close(net.walk_speed().get(), 2.0), "{}", net.walk_speed().get());
    assert!(close(time(&net, Mode::Bike), 100.0) && close(time(&net, Mode::Walk), 500.0));
}

#[test]
fn a_car_drives_2000_m_at_50_km_h_in_144_s() {
    // 50 km/h = 50 000 m / 3 600 s = 13.889 m/s; 2 000 m / 13.889 m/s = 144 s.
    let net = two_nodes(LinkRow { length_m: Some(2_000.0), ..row(1, 50.0) }).build(VALUES).unwrap();
    assert!(close(time(&net, Mode::Car), 144.0), "{}", time(&net, Mode::Car));
}

#[test]
fn a_signal_at_the_downstream_end_leaves_every_free_flow_time_unchanged() {
    // Free-flow time is length over speed, nothing else: with a signal at the downstream end (and
    // the node marked signalised), 1 500 m still takes 360 s by bike, 1 125 s on foot (1 500 m /
    // 1.3333 m/s) and 108 s by car (1 500 m / 13.889 m/s).
    let signalled =
        LinkRow { length_m: Some(1_500.0), signal_at_downstream_end: true, ..row(1, 50.0) };
    let mut b = two_nodes(signalled);
    b.mark_signalised("b");
    let net = b.build(VALUES).unwrap();
    let link = only_link(&net);
    assert!(net.has_signal_downstream(link));
    assert!(net.is_signalised(net.downstream(link)) && !net.is_signalised(net.upstream(link)));
    for (mode, seconds) in [(Mode::Bike, 360.0), (Mode::Walk, 1_125.0), (Mode::Car, 108.0)] {
        assert!(close(time(&net, mode), seconds), "{mode}: {}", time(&net, mode));
    }
}

#[test]
fn a_table_capacity_of_1200_veh_h_is_kept_as_a_third_of_a_vehicle_per_second() {
    // 1 200 veh/h / 3 600 s/h = 1/3 veh/s, whatever the lanes and capacity_per_lane say.
    let net =
        two_nodes(LinkRow { capacity_veh_h: Some(1_200.0), ..row(3, 50.0) }).build(VALUES).unwrap();
    assert!(close(net.capacity(only_link(&net)).get(), 1.0 / 3.0));
    assert!(close(net.capacity(only_link(&net)).as_veh_per_hour(), 1_200.0));
}

#[test]
fn a_car_link_of_unlimited_capacity_builds_with_its_lanes_and_free_flow_time_unchanged() {
    // A row may give an unlimited capacity as infinity: its downstream end passes any flow. The
    // lanes stay 2 and the car free-flow time stays 1 000 m / (50 000 / 3 600 m/s) = 72 s.
    let unlimited = LinkRow { capacity_veh_h: Some(f64::INFINITY), ..row(2, 50.0) };
    let net = two_nodes(unlimited).build(VALUES).unwrap();
    let link = only_link(&net);
    assert_eq!(net.capacity(link).get().to_bits(), Flow::INFINITE.get().to_bits());
    assert_eq!(net.lanes(link), 2);
    assert!(close(time(&net, Mode::Car), 72.0), "{}", time(&net, Mode::Car));
    // Negative infinity, NaN and 0 are replaced by lanes × capacity_per_lane, 2 × 1 800 =
    // 3 600 veh/h = 1 veh/s, and noted.
    for veh_h in [f64::NEG_INFINITY, f64::NAN, 0.0] {
        let net =
            two_nodes(LinkRow { capacity_veh_h: Some(veh_h), ..unlimited }).build(VALUES).unwrap();
        assert!(close(net.capacity(only_link(&net)).get(), 1.0), "{veh_h}");
        assert_eq!(noted(&net), [("capacities_defaulted", 1, vec!["ab".to_string()])]);
    }
}

#[test]
fn two_lanes_at_1800_veh_h_per_lane_pass_one_vehicle_per_second() {
    // No capacity in the row: 2 lanes × capacity_per_lane 1 800 veh/h = 3 600 veh/h = 1 veh/s.
    assert!(close(VALUES.capacity_per_lane, 1_800.0));
    let net = two_nodes(row(2, 50.0)).build(VALUES).unwrap();
    assert!(close(net.capacity(only_link(&net)).get(), 1.0));
    assert_eq!(net.lanes(only_link(&net)), 2);
}

#[test]
fn two_nodes_three_and_four_metres_apart_are_five_metres_apart() {
    // Pythagoras: √(3² + 4²) = 5. A bike at 15 km/h takes 5 / 4.1667 = 1.2 s.
    let mut b = NetworkBuilder::new();
    b.add_node("a", at(10.0, 20.0));
    b.add_node("b", at(13.0, 24.0));
    b.add_link("ab", "a", "b", row(1, 50.0));
    let net = b.build(VALUES).unwrap();
    assert!(close(net.length(only_link(&net)).get(), 5.0));
    assert!(close(time(&net, Mode::Bike), 1.2));
    assert!(net.projection().is_none(), "positions in metres are not projected");
}

#[test]
fn an_explicit_length_overrides_the_straight_line() {
    // The nodes are 1 000 m apart; the row says the street is 1 234.5 m long.
    let net = two_nodes(LinkRow { length_m: Some(1_234.5), ..row(1, 50.0) }).build(VALUES).unwrap();
    assert!(close(net.length(only_link(&net)).get(), 1_234.5));
}

#[test]
fn a_three_by_three_grid_has_nine_nodes_twenty_four_links_and_the_degrees_of_a_grid() {
    // 3 rows × 2 east-west edges + 3 columns × 2 north-south edges = 12 streets, two links each.
    // Out-degree: corner 2, edge 3, centre 4; every street is two-way, so in-degree equals it.
    let net = grid(3);
    assert_eq!(net.node_count(), 9);
    assert_eq!(net.link_count(), 24);
    let id = |r, c| net.node_ids().typed_id_of::<NodeId>(&node_name(r, c)).unwrap();
    for (r, c, degree) in [(0, 0, 2), (0, 2, 2), (2, 0, 2), (2, 2, 2), (0, 1, 3), (1, 0, 3)] {
        assert_eq!(net.out_links(id(r, c)).len(), degree, "node ({r}, {c})");
    }
    assert_eq!(net.out_links(id(1, 2)).len(), 3);
    assert_eq!(net.out_links(id(2, 1)).len(), 3);
    assert_eq!(net.out_links(id(1, 1)).len(), 4);
    for node in NodeId::iter_space(net.node_count()) {
        assert_eq!(net.in_links(node).len(), net.out_links(node).len());
        assert!(net.out_links(node).iter().all(|&l| net.upstream(l) == node));
        assert!(net.in_links(node).iter().all(|&l| net.downstream(l) == node));
    }
    // Every grid link is 100 m: 7.2 s by car at 50 km/h, 24 s by bike, 75 s on foot.
    for link in LinkId::iter_space(net.link_count()) {
        assert!(close(net.length(link).get(), 100.0));
        assert!(close(net.free_flow_time(link, Mode::Car).get(), 7.2));
        assert!(close(net.free_flow_time(link, Mode::Bike).get(), 24.0));
        assert!(close(net.free_flow_time(link, Mode::Walk).get(), 75.0));
    }
}

fn shuffled_build(reverse: bool) -> Network {
    let mut nodes = vec![("c", 2.0), ("a", 0.0), ("d", 3.0), ("b", 1.0)];
    let mut links = vec![("l2", "b", "c"), ("l10", "c", "d"), ("l1", "a", "b")];
    if reverse {
        nodes.reverse();
        links.reverse();
    }
    let mut b = NetworkBuilder::new();
    for (name, x) in nodes {
        b.add_node(name, at(x * 100.0, 0.0));
    }
    for (name, up, down) in links {
        b.add_link(name, up, down, row(1, 50.0));
    }
    b.build(VALUES).unwrap()
}

#[test]
fn ids_come_out_in_sorted_external_order_whatever_the_input_order() {
    // The expected order is the standard library's sort of the external ids (bytewise:
    // "l1" < "l10" < "l2").
    let mut node_names = vec!["c", "a", "d", "b"];
    node_names.sort_unstable();
    let mut link_names = vec!["l2", "l10", "l1"];
    link_names.sort_unstable();
    for reverse in [false, true] {
        let net = shuffled_build(reverse);
        assert_eq!(net.node_ids().iter().collect::<Vec<_>>(), node_names);
        assert_eq!(net.link_ids().iter().collect::<Vec<_>>(), link_names);
        let ends = |l: u32| {
            let link = LinkId::new(l);
            let name = |n: NodeId| net.node_ids().external(n.raw()).to_owned();
            (name(net.upstream(link)), name(net.downstream(link)))
        };
        assert_eq!(ends(0), ("a".to_owned(), "b".to_owned())); // l1
        assert_eq!(ends(1), ("c".to_owned(), "d".to_owned())); // l10
        assert_eq!(ends(2), ("b".to_owned(), "c".to_owned())); // l2
        let c = net.node_ids().typed_id_of::<NodeId>("c").unwrap();
        assert!(close(net.node_position(c).x, 200.0));
    }
}

#[test]
fn longitude_latitude_is_projected_once_and_lengths_follow_the_meridian_arc() {
    // Two nodes 0.01° apart on the central meridian of UTM zone 31 (3° E), at 52° N. The
    // straight line in the projected plane is k0 × the meridian arc M(φ)·Δφ, M taken at the
    // midpoint (WGS84: a = 6 378 137 m, e² = 6.69438e-3).
    let (a, e2) = (6_378_137.0_f64, 6.694_379_990_141_32e-3_f64);
    let mid = 52.005_f64.to_radians();
    let w = 1.0 - e2 * mid.sin().powi(2);
    let arc = a * (1.0 - e2) / (w * w.sqrt()) * 0.01_f64.to_radians();

    let mut b = NetworkBuilder::new();
    b.add_node("s", Position::LonLat(LonLat::new(3.0, 52.0)));
    b.add_node("n", Position::LonLat(LonLat::new(3.0, 52.01)));
    b.add_link("sn", "s", "n", row(1, 50.0));
    let net = b.build(VALUES).unwrap();
    let zone = net.projection().expect("longitude/latitude is projected").zone();
    assert_eq!(zone.number(), 31);
    let ratio = net.length(only_link(&net)).get() / arc;
    assert!((ratio - UTM_K0).abs() < 1e-6, "projected / arc = {ratio}");
}

#[test]
fn a_cycle_path_needs_no_car_field_and_has_no_capacity_limit() {
    // A cycle path {bike, walk}, 1 500 m: by bike 1 500 m / (15 000 / 3 600 m/s) = 360 s, on foot
    // 1 500 m / (4 800 / 3 600 m/s) = 1 125 s; no cars, so 0 lanes and an infinite capacity.
    let path = LinkRow { length_m: Some(1_500.0), ..cycle_path() };
    let net = two_nodes(path).build(VALUES).unwrap();
    let link = only_link(&net);
    assert_eq!(net.modes(link), ModeSet::of(&[Mode::Bike, Mode::Walk]));
    assert!(close(time(&net, Mode::Bike), 360.0), "{}", time(&net, Mode::Bike));
    assert!(close(time(&net, Mode::Walk), 1_125.0), "{}", time(&net, Mode::Walk));
    assert!(close(net.free_flow_speed(link, Mode::Bike).get(), 15.0 / 3.6));
    let capacity = net.capacity(link).get();
    assert!(capacity.is_infinite() && capacity > 0.0, "{capacity}");
    assert_eq!(net.lanes(link), 0);
}

#[test]
#[should_panic(expected = "does not allow cars")]
fn a_cycle_path_has_no_car_free_flow_time() {
    let net = two_nodes(cycle_path()).build(VALUES).unwrap();
    let _ = net.free_flow_time(only_link(&net), Mode::Car);
}

/// The repairs a build noted: per kind its key, count and first ids.
fn noted(net: &Network) -> Vec<(&'static str, u64, Vec<String>)> {
    net.repairs().kinds().iter().map(|k| (k.key, k.count, k.first.clone())).collect()
}

#[test]
fn a_car_row_without_a_speed_or_lanes_loses_its_car_mode_and_keeps_the_others() {
    // Every link mode, no car speed (or none above zero), or no lanes, or 0 lanes: bike and walk
    // stay (1 000 m at 15 km/h: 240 s by bike); cars are not let on it, and no speed is guessed.
    for bad in [
        LinkRow { car_speed_km_h: None, ..row(1, 50.0) },
        row(1, 0.0),
        row(1, -30.0),
        row(1, f64::NAN),
        LinkRow { lanes: None, ..row(1, 50.0) },
        row(0, 50.0),
    ] {
        let net = two_nodes(bad).build(VALUES).unwrap();
        let ab = only_link(&net);
        assert_eq!(net.modes(ab), ModeSet::of(&[Mode::Bike, Mode::Walk]), "{bad:?}");
        assert!(close(time(&net, Mode::Bike), 240.0));
        assert_eq!((net.lanes(ab), net.capacity(ab).get()), (0, f64::INFINITY));
        assert_eq!(noted(&net), [("car_mode_removed", 1, vec!["ab".to_string()])]);
    }
    // A car-only link left with no mode is left out.
    let car_only =
        LinkRow { modes: ModeSet::of(&[Mode::Car]), car_speed_km_h: None, ..row(1, 50.0) };
    let net = two_nodes(car_only).build(VALUES).unwrap();
    assert_eq!(net.link_count(), 0);
    let keys: Vec<_> = noted(&net).into_iter().map(|(k, n, _)| (k, n)).collect();
    assert_eq!(keys, [("car_mode_removed", 1), ("links_without_mode", 1)]);
}

#[test]
fn a_row_without_cars_carrying_a_car_field_ignores_it() {
    let bikes = ModeSet::of(&[Mode::Bike]);
    for with in [
        LinkRow { modes: bikes, car_speed_km_h: Some(30.0), ..cycle_path() },
        LinkRow { modes: bikes, lanes: Some(1), ..cycle_path() },
        LinkRow { capacity_veh_h: Some(1_200.0), ..cycle_path() },
    ] {
        let net = two_nodes(with).build(VALUES).unwrap();
        let ab = only_link(&net);
        assert_eq!((net.lanes(ab), net.capacity(ab).get()), (0, f64::INFINITY));
        assert_eq!(noted(&net), [("car_fields_ignored", 1, vec!["ab".to_string()])]);
    }
}

#[test]
#[should_panic(expected = "transit is not on links")]
fn transit_has_no_free_flow_time_on_a_link() {
    let net = two_nodes(row(1, 50.0)).build(VALUES).unwrap();
    let _ = net.free_flow_time(only_link(&net), Mode::Transit);
}

fn error_of(b: NetworkBuilder) -> BuildError {
    b.build(VALUES).expect_err("the build must be refused")
}

#[test]
fn a_link_to_an_undeclared_node_is_left_out_and_noted() {
    let mut b = two_nodes(row(1, 50.0));
    b.add_link("dangling", "a", "nowhere", row(1, 50.0));
    let net = b.build(VALUES).unwrap();
    assert_eq!(net.link_count(), 1);
    assert_eq!(noted(&net), [("links_to_no_node", 1, vec!["dangling".to_string()])]);
}

#[test]
fn a_bad_length_is_the_straight_line_and_a_link_of_no_length_is_left_out() {
    // a and b 1 000 m apart: a length of 0, −5, NaN or ∞ is replaced by 1 000 m, noted.
    for metres in [0.0, -5.0, f64::NAN, f64::INFINITY] {
        let net =
            two_nodes(LinkRow { length_m: Some(metres), ..row(1, 50.0) }).build(VALUES).unwrap();
        assert_eq!(net.length(only_link(&net)).get(), 1_000.0, "{metres}");
        assert_eq!(noted(&net), [("lengths_defaulted", 1, vec!["ab".to_string()])]);
    }
    // Two nodes at one place, or a node to itself, without a length: 0 m, the link left out.
    let mut b = NetworkBuilder::new();
    b.add_node("a", at(5.0, 5.0));
    b.add_node("b", at(5.0, 5.0));
    b.add_link("ab", "a", "b", row(1, 50.0));
    b.add_link("aa", "a", "a", row(1, 50.0));
    let net = b.build(VALUES).unwrap();
    assert_eq!(net.link_count(), 0);
    assert_eq!(noted(&net), [("links_of_no_length", 2, vec!["aa".to_string(), "ab".to_string()])]);
}

#[test]
fn a_build_value_not_above_zero_is_refused() {
    let values = |bike_speed, walk_speed, capacity_per_lane| BuildValues {
        capacity_per_lane,
        bike_speed,
        walk_speed,
    };
    let refused = |v: BuildValues| two_nodes(row(1, 50.0)).build(v).unwrap_err();
    assert_eq!(
        refused(values(0.0, 4.8, 1_800.0)),
        BuildError::BadValue { name: "bike_speed", value: 0.0 }
    );
    assert_eq!(
        refused(values(15.0, -1.0, 1_800.0)),
        BuildError::BadValue { name: "walk_speed", value: -1.0 }
    );
    assert!(matches!(
        refused(values(15.0, 4.8, f64::NAN)),
        BuildError::BadValue { name: "capacity_per_lane", .. }
    ));
}

#[test]
fn an_empty_network_is_refused() {
    assert_eq!(NetworkBuilder::new().build(VALUES).unwrap_err(), BuildError::NoNodes);
}

#[test]
fn a_mix_of_longitude_latitude_and_metres_is_refused() {
    let mut b = NetworkBuilder::new();
    b.add_node("m", at(0.0, 0.0));
    b.add_node("g", Position::LonLat(LonLat::new(4.8, 45.7)));
    assert_eq!(error_of(b), BuildError::MixedPositions { lon_lat: "g".into(), metres: "m".into() });
}

#[test]
fn every_other_bad_row_is_repaired_and_noted() {
    // A node beyond ±84°: left out, and the link naming it; a node in metres not finite alike.
    let mut b = NetworkBuilder::new();
    b.add_node("a", Position::LonLat(LonLat::new(4.36, 52.0)));
    b.add_node("b", Position::LonLat(LonLat::new(4.37, 52.0)));
    b.add_node("pole", Position::LonLat(LonLat::new(0.0, 88.0)));
    b.add_link("ab", "a", "b", row(1, 50.0));
    b.add_link("to-pole", "a", "pole", row(1, 50.0));
    let net = b.build(VALUES).unwrap();
    assert_eq!((net.node_count(), net.link_count()), (2, 1));
    let keys = noted(&net);
    assert_eq!(
        keys,
        [
            ("nodes_unplaced", 1, vec!["pole".to_string()]),
            ("links_to_no_node", 1, vec!["to-pole".to_string()])
        ]
    );
    let mut b = two_nodes(row(1, 50.0));
    b.add_node("nan", at(f64::NAN, 0.0));
    assert_eq!(noted(&b.build(VALUES).unwrap()), [("nodes_unplaced", 1, vec!["nan".to_string()])]);

    // A node or a link id given twice: the first row given kept (a stays at 0, ab runs a→b).
    let mut b = two_nodes(row(1, 50.0));
    b.add_node("a", at(1.0, 1.0));
    b.add_link("ab", "b", "a", row(2, 30.0));
    let net = b.build(VALUES).unwrap();
    assert_eq!(net.node_position(NodeId::new(0)).x, 0.0);
    assert_eq!((net.upstream(only_link(&net)), net.lanes(only_link(&net))), (NodeId::new(0), 1));
    let keys: Vec<_> = noted(&net).into_iter().map(|(k, n, _)| (k, n)).collect();
    assert_eq!(keys, [("nodes_repeated", 1), ("links_repeated", 1)]);

    // A signal on a node not held: ignored.
    let mut b = two_nodes(row(1, 50.0));
    b.mark_signalised("z");
    assert_eq!(noted(&b.build(VALUES).unwrap()), [("signals_unplaced", 1, vec!["z".to_string()])]);

    // A link allowing transit: transit taken out; a link allowing nothing: left out.
    let net = two_nodes(LinkRow { modes: ALL_LINK_MODES.with(Mode::Transit), ..row(1, 50.0) })
        .build(VALUES)
        .unwrap();
    assert_eq!(net.modes(only_link(&net)), ALL_LINK_MODES);
    assert_eq!(noted(&net), [("links_transit_removed", 1, vec!["ab".to_string()])]);
    let net = two_nodes(LinkRow { modes: ModeSet::EMPTY, ..cycle_path() }).build(VALUES).unwrap();
    assert_eq!(net.link_count(), 0);
    assert_eq!(noted(&net), [("links_without_mode", 1, vec!["ab".to_string()])]);
}

/// Of a repeated id the first usable row is kept: a first row that is itself left out (a node
/// not finite, a link to a node not held) does not take the good second row with it, and each
/// fault is noted once. a at 0 m and b at 1 000 m; the second `ab` row runs a→b with 2 lanes.
#[test]
fn a_repeated_id_keeps_its_first_usable_row() {
    let mut b = NetworkBuilder::new();
    b.add_node("a", at(f64::NAN, 0.0));
    b.add_node("a", at(0.0, 0.0));
    b.add_node("b", at(1_000.0, 0.0));
    b.add_link("ab", "a", "nowhere", row(1, 50.0));
    b.add_link("ab", "a", "b", row(2, 50.0));
    let net = b.build(VALUES).unwrap();
    assert_eq!((net.node_count(), net.link_count()), (2, 1));
    assert_eq!((net.lanes(only_link(&net)), net.length(only_link(&net)).get()), (2, 1_000.0));
    let one = |key| (key, 1, vec!["a".to_string()]);
    assert_eq!(noted(&net), [one("nodes_repeated"), ("links_repeated", 1, vec!["ab".to_string()])]);
    // Every row of an id left out: the first row's fault, the other rows as repeated.
    let mut b = two_nodes(row(1, 50.0));
    b.add_link("ac", "a", "nowhere", row(1, 50.0));
    b.add_link("ac", "a", "elsewhere", row(1, 50.0));
    let net = b.build(VALUES).unwrap();
    assert_eq!(net.link_count(), 1);
    let keys: Vec<_> = noted(&net).into_iter().map(|(k, n, _)| (k, n)).collect();
    assert_eq!(keys, [("links_repeated", 1), ("links_to_no_node", 1)]);
}

#[test]
fn the_repairs_do_not_depend_on_the_input_order() {
    // Two bad links ("zz" without lanes, "ab" without a car speed), either way round: both lose
    // their car mode, noted in sorted order.
    for reverse in [false, true] {
        let mut b = NetworkBuilder::new();
        b.add_node("a", at(0.0, 0.0));
        b.add_node("b", at(1.0, 0.0));
        let mut bad = vec![("zz", row(0, 50.0)), ("ab", row(1, 0.0))];
        if reverse {
            bad.reverse();
        }
        for (name, r) in bad {
            b.add_link(name, "a", "b", r);
        }
        let net = b.build(VALUES).unwrap();
        assert_eq!(
            noted(&net),
            [("car_mode_removed", 2, vec!["ab".to_string(), "zz".to_string()])]
        );
    }
}
