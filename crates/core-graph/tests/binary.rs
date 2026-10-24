//! The binary form of a built network (a network cache file's content): what is written comes
//! back equal, field for field and byte for byte, for every kind of network the crate builds,
//! and a file of another network, version or a damaged one is refused.

use std::collections::{BTreeMap, HashMap};

use openmobisim_core_graph::binary::{FormatError, Header, Reader, Writer};
use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
use openmobisim_core_graph::geometry::{LonLat, polyline_length_metres};
use openmobisim_core_graph::layers::StaticNetwork;
use openmobisim_core_graph::link_geometry::{LinkGeometry, NetworkFingerprint};
use openmobisim_core_graph::network::{LinkChanges, LinkSpec, RoadNetwork, RoadNetworkBuilder};
use openmobisim_core_graph::network_options::NetworkDefaults;
use openmobisim_core_graph::{manhattan_grid, toy_network, toy_network_layers};
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::{EntityId, LinkId};

fn bytes_of(network: &RoadNetwork) -> Vec<u8> {
    let mut w = Writer::new();
    network.write_binary(&mut w);
    w.finish()
}

/// Write, read back, and check the two are equal and write the same bytes (which also tells a
/// NaN from another and `-0.0` from `0.0`, which `==` does not).
fn round_trip(network: &RoadNetwork) -> RoadNetwork {
    let bytes = bytes_of(network);
    let mut r = Reader::new(&bytes).unwrap();
    let back = RoadNetwork::read_binary(&mut r).unwrap();
    r.finish().unwrap();
    assert!(back == *network, "the network read back differs from the one written");
    assert_eq!(bytes_of(&back), bytes, "the network read back writes other bytes");
    assert_eq!(NetworkFingerprint::of(&back), NetworkFingerprint::of(network));
    back
}

#[test]
fn the_toy_network_and_a_signalised_grid_come_back_equal() {
    let (toy, _) = toy_network();
    let back = round_trip(&toy);
    // Spot checks through the public face, in case equality itself were wrong.
    for raw in 0..toy.link_count() {
        let l = LinkId::new(raw);
        assert_eq!(back.link_parameters(l), toy.link_parameters(l));
        assert_eq!(back.free_flow_time(l).get().to_bits(), toy.free_flow_time(l).get().to_bits());
        assert_eq!(back.storage(l).get().to_bits(), toy.storage(l).get().to_bits());
    }
    for raw in 0..toy.node_count() {
        let n = EntityId::new(raw);
        assert_eq!(back.out_links(n), toy.out_links(n));
        assert_eq!(back.in_links(n), toy.in_links(n));
        assert_eq!(back.node_position(n), toy.node_position(n));
    }
    assert_eq!(back.projection(), toy.projection());
    let (grid, _) = manhattan_grid(6, 150.0, true);
    round_trip(&grid);
}

#[test]
fn an_edited_network_keeps_its_closures_lanes_and_capacities() {
    let (toy, _) = toy_network();
    let id = |name: &str| LinkId::new(toy.link_external_ids().id_of(name).unwrap());
    let edited = toy
        .with_changes(&LinkChanges {
            closed: vec![id("a2")],
            lanes: vec![(id("a1"), 3)],
            capacity_factor: vec![(id("s1"), 0.5)],
        })
        .unwrap();
    let back = round_trip(&edited);
    assert!(
        back.is_closed(id("a2")) && !back.out_links(back.link_from(id("a2"))).contains(&id("a2"))
    );
    assert!(back != toy, "the edit is read back, not the original");
}

#[test]
fn a_network_built_with_other_parameters_keeps_them() {
    let options = BTreeMap::from([
        ("capacity_factor".to_string(), 0.9),
        ("residential.free_flow_km_h".to_string(), 25.0),
        ("primary.lanes".to_string(), 3.0),
        ("walk_km_h".to_string(), 4.5),
        ("street_50.saturation_flow_veh_h_lane".to_string(), 1100.0),
    ]);
    let defaults = NetworkDefaults::from_options(&options).unwrap();
    let mut b = RoadNetworkBuilder::new();
    b.add_node("a", LonLat::new(4.35, 52.0));
    b.add_node("b", LonLat::new(4.36, 52.0));
    b.add_node("c", LonLat::new(4.36, 52.01));
    b.mark_signalised("b");
    b.mark_signal_location("c");
    b.add_link("ab", "a", "b", LinkSpec::new(RoadClass::Residential));
    b.add_link("bc", "b", "c", LinkSpec { lanes: Some(2), ..LinkSpec::new(RoadClass::Primary) });
    b.add_link(
        "ca",
        "c",
        "a",
        LinkSpec {
            capacity_veh_h: Some(900.0),
            maxspeed_km_h: Some(30.0),
            roundabout: true,
            length_m: Some(2000.0),
            ..LinkSpec::new(RoadClass::Tertiary)
        },
    );
    let network = b.build_with(&defaults, &mut Diagnostics::new()).unwrap();
    let back = round_trip(&network);
    assert_eq!(*back.defaults(), defaults);
    assert_eq!(back.defaults().descriptor(), network.defaults().descriptor());
}

#[test]
fn a_southern_network_keeps_its_hemisphere() {
    let mut b = RoadNetworkBuilder::new();
    b.add_node("a", LonLat::new(151.20, -33.87));
    b.add_node("b", LonLat::new(151.21, -33.87));
    b.add_link("ab", "a", "b", LinkSpec::new(RoadClass::Primary));
    b.add_link("ba", "b", "a", LinkSpec::new(RoadClass::Primary));
    let network = b
        .build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
        .unwrap();
    assert_eq!(round_trip(&network).projection(), network.projection());
}

#[test]
fn the_bike_and_walk_layers_come_back_with_their_speeds_and_facilities() {
    let (bike, walk) = toy_network_layers();
    for layer in [bike, walk] {
        let mut w = Writer::new();
        layer.write_binary(&mut w);
        let bytes = w.finish();
        let mut r = Reader::new(&bytes).unwrap();
        let back = StaticNetwork::read_binary(&mut r).unwrap();
        r.finish().unwrap();
        assert!(back == layer);
        assert_eq!(back.layer(), layer.layer());
        assert_eq!(back.link_seconds(), layer.link_seconds());
        let mut again = Writer::new();
        back.write_binary(&mut again);
        assert_eq!(again.finish(), bytes);
    }
}

#[test]
fn street_geometry_comes_back_and_is_refused_for_another_network() {
    // Each link's length measured along its two points, as an importer measures it; a two-way
    // street shares one point run.
    let at = HashMap::from([
        ("a", LonLat::new(4.35, 52.0)),
        ("b", LonLat::new(4.36, 52.0)),
        ("c", LonLat::new(4.36, 52.01)),
    ]);
    let mut b = RoadNetworkBuilder::new();
    for (name, &p) in &at {
        b.add_node(*name, p);
    }
    for (id, from, to) in [("ab", "a", "b"), ("ba", "b", "a"), ("bc", "b", "c")] {
        let length_m = Some(polyline_length_metres(&[at[from], at[to]]));
        b.add_link(id, from, to, LinkSpec { length_m, ..LinkSpec::new(RoadClass::Residential) });
    }
    let toy = b
        .build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
        .unwrap();
    let mut points: HashMap<String, Vec<LonLat>> = HashMap::new();
    for raw in 0..toy.link_count() {
        let l = LinkId::new(raw);
        let (a, b) = (toy.node_lonlat(toy.link_from(l)), toy.node_lonlat(toy.link_to(l)));
        points.insert(toy.link_external_ids().external(raw).to_string(), vec![a, b]);
    }
    let geometry = LinkGeometry::build(&toy, &points);
    assert_eq!(geometry.street_count(), 2, "the two-way street shares its run");
    let mut w = Writer::new();
    geometry.write_binary(&mut w);
    let bytes = w.finish();
    let back = LinkGeometry::read_binary(&mut Reader::new(&bytes).unwrap(), &toy).unwrap();
    assert!(back == geometry && back.matches(&toy));
    let (grid, _) = manhattan_grid(3, 100.0, false);
    let err = LinkGeometry::read_binary(&mut Reader::new(&bytes).unwrap(), &grid).unwrap_err();
    assert!(matches!(err, FormatError::Invalid(m) if m.contains("another network")));
}

#[test]
fn a_file_cut_short_or_damaged_is_refused_never_read() {
    let (toy, _) = toy_network();
    let bytes = bytes_of(&toy);
    let mut damaged = bytes.clone();
    let middle = damaged.len() / 2;
    damaged[middle] ^= 1;
    assert_eq!(Reader::new(&damaged).unwrap_err(), FormatError::Damaged);
    // Content cut short but checksummed again (a writer of another layout): read, and refused.
    let mut w = Writer::new();
    toy.write_binary(&mut w);
    let full = w.finish();
    let mut short = Writer::new();
    for &b in &full[8..full.len() / 2] {
        short.u8(b);
    }
    let short = short.finish();
    let err = RoadNetwork::read_binary(&mut Reader::new(&short).unwrap()).unwrap_err();
    assert!(matches!(err, FormatError::Invalid(_)), "{err}");
}

#[test]
fn the_header_comes_first_and_the_network_after_it() {
    let (toy, _) = toy_network();
    let header = Header::current("build", "source-checksum", 0.25);
    let mut w = Writer::new();
    header.write(&mut w);
    toy.write_binary(&mut w);
    let bytes = w.finish();
    let mut r = Reader::new(&bytes).unwrap();
    assert_eq!(Header::read(&mut r).unwrap(), header);
    assert!(RoadNetwork::read_binary(&mut r).unwrap() == toy);
    r.finish().unwrap();
}
