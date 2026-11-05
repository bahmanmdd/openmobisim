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

#[test]
fn a_car_row_without_a_speed_or_lanes_is_refused_naming_the_link() {
    let no_speed = LinkRow { car_speed_km_h: None, ..row(1, 50.0) };
    let err = error_of(two_nodes(no_speed));
    assert_eq!(err, BuildError::MissingCarField { link: "ab".into(), field: "car_speed_km_h" });
    assert!(err.to_string().contains("link ab"), "{err}");
    let no_lanes = LinkRow { lanes: None, ..row(1, 50.0) };
    assert_eq!(
        error_of(two_nodes(no_lanes)),
        BuildError::MissingCarField { link: "ab".into(), field: "lanes" }
    );
}

#[test]
fn a_row_without_cars_carrying_a_car_field_is_refused_naming_the_link() {
    let bikes = ModeSet::of(&[Mode::Bike]);
    let with_speed = LinkRow { modes: bikes, car_speed_km_h: Some(30.0), ..cycle_path() };
    let err = error_of(two_nodes(with_speed));
    assert_eq!(err, BuildError::CarFieldWithoutCars { link: "ab".into(), field: "car_speed_km_h" });
    assert!(err.to_string().contains("link ab"), "{err}");
    let with_lanes = LinkRow { modes: bikes, lanes: Some(1), ..cycle_path() };
    assert_eq!(
        error_of(two_nodes(with_lanes)),
        BuildError::CarFieldWithoutCars { link: "ab".into(), field: "lanes" }
    );
    let with_capacity = LinkRow { capacity_veh_h: Some(1_200.0), ..cycle_path() };
    assert_eq!(
        error_of(two_nodes(with_capacity)),
        BuildError::CarFieldWithoutCars { link: "ab".into(), field: "capacity_veh_h" }
    );
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
fn a_link_to_an_undeclared_node_is_refused_naming_the_link() {
    let mut b = two_nodes(row(1, 50.0));
    b.add_link("dangling", "a", "nowhere", row(1, 50.0));
    let err = error_of(b);
    assert_eq!(err, BuildError::UnknownNode { link: "dangling".into(), node: "nowhere".into() });
    assert!(err.to_string().contains("dangling") && err.to_string().contains("nowhere"));
}

#[test]
fn a_zero_negative_or_nan_length_is_refused_naming_the_link() {
    for metres in [0.0, -5.0, f64::NAN, f64::INFINITY] {
        let err = error_of(two_nodes(LinkRow { length_m: Some(metres), ..row(1, 50.0) }));
        assert!(matches!(&err, BuildError::BadLength { link, .. } if link == "ab"), "{err}");
        assert!(err.to_string().contains("link ab"), "{err}");
    }
    // Two nodes at the same place give a straight line of 0 m.
    let mut b = NetworkBuilder::new();
    b.add_node("a", at(5.0, 5.0));
    b.add_node("b", at(5.0, 5.0));
    b.add_link("ab", "a", "b", row(1, 50.0));
    assert_eq!(error_of(b), BuildError::BadLength { link: "ab".into(), metres: 0.0 });
}

#[test]
fn a_zero_speed_is_refused() {
    for km_h in [0.0, -30.0, f64::NAN] {
        let err = error_of(two_nodes(row(1, km_h)));
        assert!(matches!(&err, BuildError::BadSpeed { link, .. } if link == "ab"), "{err}");
    }
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
fn every_other_bad_row_is_refused_naming_what_it_refuses() {
    let unprojectable = {
        let mut b = NetworkBuilder::new();
        b.add_node("pole", Position::LonLat(LonLat::new(0.0, 88.0)));
        error_of(b)
    };
    assert!(matches!(&unprojectable, BuildError::NotProjectable { node, .. } if node == "pole"));
    assert!(unprojectable.to_string().contains("±84°"));

    let mut b = two_nodes(row(1, 50.0));
    b.add_node("a", at(1.0, 1.0));
    assert_eq!(error_of(b), BuildError::DuplicateNode("a".into()));

    let mut b = two_nodes(row(1, 50.0));
    b.add_link("ab", "b", "a", row(1, 50.0));
    assert_eq!(error_of(b), BuildError::DuplicateLink("ab".into()));

    let mut b = two_nodes(row(1, 50.0));
    b.mark_signalised("z");
    assert_eq!(error_of(b), BuildError::UnknownSignalNode("z".into()));

    for modes in [ModeSet::EMPTY, ALL_LINK_MODES.with(Mode::Transit)] {
        let err = error_of(two_nodes(LinkRow { modes, ..row(1, 50.0) }));
        assert_eq!(err, BuildError::BadModes { link: "ab".into(), modes });
    }
    assert_eq!(error_of(two_nodes(row(0, 50.0))), BuildError::NoLanes("ab".into()));
    let err = error_of(two_nodes(LinkRow { capacity_veh_h: Some(0.0), ..row(1, 50.0) }));
    assert_eq!(err, BuildError::BadCapacity { link: "ab".into(), veh_h: 0.0 });
}

#[test]
fn the_error_reported_does_not_depend_on_the_input_order() {
    // Two bad links; the one first in sorted order ("ab" < "zz") is named, either way round.
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
        assert!(matches!(error_of(b), BuildError::BadSpeed { link, .. } if link == "ab"));
    }
}
