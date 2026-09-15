//! The road network: identity, structure, and what happens to bad data.

mod common;

use common::{grid, grid_link_count, node_name};
use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::network::{LinkSpec, MIN_LINK_LENGTH_M, RoadNetworkBuilder, codes};
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::{EntityId, LinkId, NodeId};

#[test]
fn a_grid_has_the_counts_a_grid_should_have() {
    let (net, diag) = grid(5, false);
    assert_eq!(net.node_count(), 25);
    assert_eq!(net.link_count(), grid_link_count(5));
    assert!(diag.is_empty(), "a clean grid must produce no diagnostics: {:?}", diag.rows());

    // Corner nodes have two exits, edge nodes three, interior nodes four.
    let id = |r, c| net.node_external_ids().typed_id_of::<NodeId>(&node_name(r, c)).unwrap();
    assert_eq!(net.out_links(id(0, 0)).len(), 2);
    assert_eq!(net.out_links(id(0, 2)).len(), 3);
    assert_eq!(net.out_links(id(2, 2)).len(), 4);
    // A grid is symmetric, so in-degree matches out-degree everywhere.
    for node in NodeId::iter_space(net.node_count()) {
        assert_eq!(net.in_links(node).len(), net.out_links(node).len());
    }
}

#[test]
fn ids_are_assigned_by_sorted_external_id_whatever_order_they_arrive_in() {
    // The property that makes a cached network reusable and a seeded run
    // comparable across builds.
    let build = |reverse: bool| {
        let mut b = RoadNetworkBuilder::new();
        let names = ["c", "a", "d", "b"];
        let order: Vec<&str> =
            if reverse { names.iter().rev().copied().collect() } else { names.to_vec() };
        for (i, name) in order.iter().enumerate() {
            #[allow(clippy::cast_precision_loss, reason = "four nodes")]
            let offset = i as f64 * 0.001;
            b.add_node(*name, LonLat::new(4.8 + offset, 45.7));
        }
        b.add_link("link_2", "a", "b", LinkSpec::new(RoadClass::Primary));
        b.add_link("link_1", "b", "c", LinkSpec::new(RoadClass::Primary));
        let mut d = Diagnostics::new();
        b.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut d).unwrap()
    };

    let forwards = build(false);
    let backwards = build(true);

    for (a, b) in [(&forwards, &backwards)] {
        assert_eq!(a.node_count(), b.node_count());
        assert_eq!(a.link_count(), b.link_count());
        for node in NodeId::iter_space(a.node_count()) {
            assert_eq!(
                a.node_external_ids().external(node.raw()),
                b.node_external_ids().external(node.raw())
            );
        }
        for link in LinkId::iter_space(a.link_count()) {
            assert_eq!(a.link_from(link), b.link_from(link));
            assert_eq!(a.link_to(link), b.link_to(link));
            assert_eq!(
                a.link_external_ids().external(link.raw()),
                b.link_external_ids().external(link.raw())
            );
        }
    }

    // And the assignment really is sorted, not merely stable.
    assert_eq!(forwards.node_external_ids().external(0), "a");
    assert_eq!(forwards.node_external_ids().external(3), "d");
    assert_eq!(forwards.link_external_ids().external(0), "link_1");
}

#[test]
fn link_geometry_comes_out_in_metres() {
    let (net, _) = grid(3, false);
    // 0.001° of latitude is about 111 m; the projection shrinks it by k0.
    for link in LinkId::iter_space(net.link_count()) {
        let m = net.link_length(link).get();
        assert!((70.0..130.0).contains(&m), "link length {m} m is implausible for a 0.001° grid");
    }
}

#[test]
fn free_flow_time_includes_the_control_delay_at_signals() {
    let (plain, _) = grid(5, false);
    let (signalled, _) = grid(5, true);

    // Find a link whose downstream node is an interior, signalised one.
    let interior = signalled.node_external_ids().typed_id_of::<NodeId>(&node_name(2, 2)).unwrap();
    let approach = signalled.in_links(interior)[0];

    assert!(signalled.is_signalised(interior));
    assert!(
        signalled.free_flow_time(approach).get() > plain.free_flow_time(approach).get() + 10.0,
        "a signalised approach must carry a control delay of roughly 20 s"
    );
    // The two free-flow times are computed independently (`length / speed +
    // control_delay` vs `length / speed`), so their difference need not land
    // on the exact same bit pattern as `control_delay` on every platform —
    // observed to differ in the last bit between Linux and macOS. Compare
    // with a tolerance far tighter than anything that matters physically.
    let difference = (signalled.free_flow_time(approach) - plain.free_flow_time(approach)).get();
    let expected = signalled.link_parameters(approach).control_delay.get();
    assert!(
        (difference - expected).abs() < 1e-9,
        "control delay should reproduce the free-flow time difference: {expected} vs {difference}"
    );
}

#[test]
fn storage_is_jam_density_times_length() {
    let (net, _) = grid(3, false);
    for link in LinkId::iter_space(net.link_count()) {
        let p = net.link_parameters(link);
        let expected = p.jam_density.get() * net.link_length(link).get();
        assert!((net.storage(link).get() - expected).abs() < 1e-9);
        // A secondary road at 130 veh/km/lane over ~110 m holds about 14 cars.
        assert!((10.0..20.0).contains(&net.storage(link).get()));
    }
}

#[test]
fn a_link_to_an_undeclared_node_is_dropped_and_recorded() {
    // The simulation never stops for bad data — it resolves it and says so.
    let mut b = RoadNetworkBuilder::new();
    b.add_node("a", LonLat::new(4.80, 45.70));
    b.add_node("b", LonLat::new(4.81, 45.70));
    b.add_link("good", "a", "b", LinkSpec::new(RoadClass::Primary));
    b.add_link("dangling", "a", "nowhere", LinkSpec::new(RoadClass::Primary));

    let mut diag = Diagnostics::new();
    let net = b.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut diag).unwrap();

    assert_eq!(net.link_count(), 1, "the dangling link must be dropped, not kept");
    assert_eq!(diag.count_of(codes::LINK_REFERENCES_UNKNOWN_NODE), 1);
}

#[test]
fn a_zero_length_link_is_given_a_minimum_and_recorded() {
    // Duplicate nodes are common in OSM. A zero-length link would divide by
    // zero in the loading and loop in the route search.
    let mut b = RoadNetworkBuilder::new();
    b.add_node("a", LonLat::new(4.80, 45.70));
    b.add_node("b", LonLat::new(4.80, 45.70)); // same place
    b.add_link("degenerate", "a", "b", LinkSpec::new(RoadClass::Residential));

    let mut diag = Diagnostics::new();
    let net = b.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut diag).unwrap();

    let link = LinkId::new(0);
    assert!((net.link_length(link).get() - MIN_LINK_LENGTH_M).abs() < 1e-9);
    assert!(net.free_flow_time(link).get() > 0.0);
    assert!(net.storage(link).get() > 0.0);
    assert_eq!(diag.count_of(codes::DEGENERATE_LINK_LENGTH), 1);
}

#[test]
fn an_explicit_length_overrides_the_straight_line() {
    // A link that has not been split at every geometry point is longer than the
    // straight line between its ends, and the importer knows by how much.
    let mut b = RoadNetworkBuilder::new();
    b.add_node("a", LonLat::new(4.80, 45.70));
    b.add_node("b", LonLat::new(4.81, 45.70));
    b.add_link(
        "winding",
        "a",
        "b",
        LinkSpec { length_m: Some(2_000.0), ..LinkSpec::new(RoadClass::Primary) },
    );

    let mut diag = Diagnostics::new();
    let net = b.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut diag).unwrap();
    assert!((net.link_length(LinkId::new(0)).get() - 2_000.0).abs() < 1e-9);
    assert!(diag.is_empty());
}

#[test]
fn an_empty_or_unprojectable_network_is_refused_at_build_time() {
    let mut diag = Diagnostics::new();
    let empty = RoadNetworkBuilder::new();
    assert!(
        empty.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut diag).is_err(),
        "an empty network is a scenario error, not a data condition"
    );

    let mut polar = RoadNetworkBuilder::new();
    polar.add_node("north", LonLat::new(0.0, 88.0));
    assert!(polar.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut diag).is_err());
}

#[test]
fn reverse_links_recognise_each_other() {
    let (net, _) = grid(3, false);
    let a = net.node_external_ids().typed_id_of::<NodeId>(&node_name(1, 1)).unwrap();
    let out = net.out_links(a)[0];
    let back = net
        .out_links(net.link_to(out))
        .iter()
        .copied()
        .find(|&l| net.link_to(l) == a)
        .expect("a grid is bidirectional");
    assert!(net.is_reverse_of(out, back));
    assert!(net.is_reverse_of(back, out));
    assert!(!net.is_reverse_of(out, out));
}

#[test]
fn the_projection_is_chosen_once_and_reported() {
    let (net, _) = grid(3, false);
    // Lyon sits in zone 31N.
    assert_eq!(net.projection().zone().number(), 31);
    assert_eq!(net.projection().zone().epsg(), 32_631);
}

#[test]
fn whole_array_accessors_agree_with_per_element_ones() {
    // The structure-of-arrays accessors are what the loading will use; they
    // must not drift from the per-element ones the tests use.
    let (net, _) = grid(4, true);
    let lengths = net.link_lengths();
    let times = net.free_flow_times();
    let storages = net.storages();
    assert_eq!(lengths.len(), net.link_count() as usize);
    for link in LinkId::iter_space(net.link_count()) {
        assert_eq!(lengths[link.index()], net.link_length(link));
        assert_eq!(times[link.index()], net.free_flow_time(link));
        assert_eq!(storages[link.index()], net.storage(link));
    }
}

#[test]
fn an_unknown_highway_tag_is_reported_rather_than_guessed() {
    assert_eq!(RoadClass::from_osm_highway("bridleway"), None);
    // The code exists so the importer has one place to record the fallback.
    assert_eq!(codes::UNKNOWN_HIGHWAY_CLASS.as_str(), "unknown_highway_class");
}
