//! The toy network fixture itself: what it is, so cases built on it can rely on it.

use openmobisim_core_graph::defaults::SignalDefaults;
use openmobisim_core_graph::examples::toy_network;
use openmobisim_core_graph::geometry::haversine_metres;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_types::ids::{EntityId, LinkId, NodeId};

#[test]
fn the_toy_network_has_the_shape_its_documentation_promises() {
    let (net, diagnostics) = toy_network();
    assert!(diagnostics.is_empty(), "the fixture builds cleanly");
    assert_eq!((net.node_count(), net.link_count()), (16, 16));

    let node = |n: &str| net.node_external_ids().typed_id_of::<NodeId>(n).expect("a toy node");
    let link = |l: &str| net.link_external_ids().typed_id_of::<LinkId>(l).expect("a toy link");

    // One signalised node; one ring, four links, all flagged; nothing else flagged.
    let signalised: Vec<u32> = NodeId::iter_space(net.node_count())
        .filter(|&n| net.is_signalised(n))
        .map(|n| n.raw())
        .collect();
    assert_eq!(signalised, vec![node("S").raw()]);
    let ring: Vec<LinkId> =
        LinkId::iter_space(net.link_count()).filter(|&l| net.is_roundabout(l)).collect();
    let mut expected_ring = vec![link("r1"), link("r2"), link("r3"), link("r4")];
    expected_ring.sort();
    assert_eq!(ring, expected_ring);

    // The sub-vehicle-length chain: each piece holds less than one car.
    for piece in ["c1", "c2", "c3"] {
        assert!(net.storage(link(piece)).get() < 1.0, "{piece} is shorter than a car");
    }
    // Everything else holds at least a car and a half.
    for name in ["a1", "s1", "a2", "m1", "a3", "a4", "a5", "r1", "r2", "r3", "r4", "n3", "e2"] {
        assert!(net.storage(link(name)).get() > 1.5, "{name} is an ordinary link");
    }
}

#[test]
fn the_toy_networks_geometry_agrees_with_its_stated_lengths() {
    // Lengths are set exactly; the node positions are only for maps, and must
    // not contradict them.
    let (net, _) = toy_network();
    for link in LinkId::iter_space(net.link_count()) {
        let (from, to) = (net.node_lonlat(net.link_from(link)), net.node_lonlat(net.link_to(link)));
        let straight = haversine_metres(from, to);
        let stated = net.link_length(link).get();
        assert!(
            (straight - stated).abs() <= 0.02 * stated + 0.05,
            "{link:?}: nodes are {straight:.2} m apart, the link says {stated:.2} m"
        );
    }
}

#[test]
fn every_toy_link_can_be_driven_on_and_the_turn_table_builds() {
    let (net, _) = toy_network();
    let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);
    assert!(!turns.is_empty());
    for link in LinkId::iter_space(net.link_count()) {
        assert!(net.link_parameters(link).capacity.get() > 0.0);
    }
}
