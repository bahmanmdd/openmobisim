//! Scenario edits of a road network: closures, lanes, capacity factors, signals ignored.

use openmobisim_core_graph::defaults::SignalDefaults;
use openmobisim_core_graph::examples::{manhattan_grid, node_name};
use openmobisim_core_graph::network::{LinkChanges, RoadNetwork};
use openmobisim_core_graph::toy_network;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_types::ids::{EntityId, LinkId, NodeId, TurnId};

#[test]
fn a_closed_link_keeps_its_index_and_data_but_no_node_reaches_it() {
    let (road, _) = toy_network();
    let a2 = LinkId::from_index(road.link_external_ids().id_of("a2").unwrap() as usize);
    let edited =
        road.with_changes(&LinkChanges { closed: vec![a2], ..LinkChanges::default() }).unwrap();
    assert_eq!(edited.link_count(), road.link_count());
    assert!(edited.is_closed(a2) && !edited.is_drivable(a2) && road.is_drivable(a2));
    assert_eq!(edited.link_from(a2), road.link_from(a2), "its data kept");
    assert!(!edited.out_links(road.link_from(a2)).contains(&a2));
    assert!(!edited.in_links(road.link_to(a2)).contains(&a2));
    assert!(road.out_links(road.link_from(a2)).contains(&a2), "the original untouched");
    let others = (0..road.link_count()).map(LinkId::new).filter(|&l| l != a2);
    for l in others {
        assert!(!edited.is_closed(l));
        assert!(edited.out_links(edited.link_from(l)).contains(&l));
    }
}

#[test]
fn lanes_scale_the_diagram_and_a_capacity_factor_keeps_it_a_triangle() {
    let (road, _) = toy_network();
    let id =
        |name: &str| LinkId::from_index(road.link_external_ids().id_of(name).unwrap() as usize);
    let (m1, a1) = (id("m1"), id("a1"));
    let changes = LinkChanges {
        lanes: vec![(m1, 1)],
        capacity_factor: vec![(a1, 0.5)],
        ..LinkChanges::default()
    };
    let edited = road.with_changes(&changes).unwrap();
    let (p, q) = (road.link_parameters(m1), edited.link_parameters(m1));
    assert!((q.capacity.get() - p.capacity.get() / 2.0).abs() < 1e-9);
    assert!((q.jam_density.get() - p.jam_density.get() / 2.0).abs() < 1e-12);
    assert_eq!(q.wave_speed, p.wave_speed, "both halved: the same wave speed");
    assert!((edited.storage(m1).get() - road.storage(m1).get() / 2.0).abs() < 1e-9);
    let (p, q) = (road.link_parameters(a1), edited.link_parameters(a1));
    assert!((q.capacity.get() - p.capacity.get() / 2.0).abs() < 1e-9);
    assert!(q.consistency_error() < 1e-9, "still a triangle");
    assert_eq!(edited.storage(a1), road.storage(a1), "a bottleneck keeps its storage");
    assert_eq!(edited.free_flow_time(a1), road.free_flow_time(a1));
}

#[test]
fn wrong_changes_are_refused() {
    let (road, _) = toy_network();
    let far = LinkId::new(road.link_count());
    let refused = |c: LinkChanges| road.with_changes(&c).unwrap_err();
    assert!(
        refused(LinkChanges { closed: vec![far], ..LinkChanges::default() }).contains("no link")
    );
    assert!(
        refused(LinkChanges { lanes: vec![(LinkId::new(0), 0)], ..LinkChanges::default() })
            .contains("0 lanes")
    );
    assert!(
        refused(LinkChanges {
            capacity_factor: vec![(LinkId::new(0), 0.0)],
            ..LinkChanges::default()
        })
        .contains("above 0")
    );
}

fn signalised_links(net: &RoadNetwork) -> Vec<LinkId> {
    (0..net.link_count()).map(LinkId::new).filter(|&l| net.is_link_signalised(l)).collect()
}

#[test]
fn every_signal_ignored_gives_the_network_built_without_signals() {
    // A 5 × 5 grid with signals at its 9 interior nodes, every signal then ignored, must be the
    // same network, field by field and bit by bit, as the grid built without signals: the same
    // free-flow times (length over speed, no control delay), flags and turn capacities.
    let (with, _) = manhattan_grid(5, 200.0, true);
    let (without, _) = manhattan_grid(5, 200.0, false);
    let off = signalised_links(&with);
    assert_eq!(off.len(), 9 * 4, "four approaches at each of the 9 interior nodes");
    assert!(with != without, "the signals make a difference to begin with");
    let edited = with
        .with_changes(&LinkChanges { signal_off: off.clone(), ..LinkChanges::default() })
        .unwrap();
    assert!(edited == without, "every signal ignored is the grid without signals");
    for &l in &off {
        let delay = with.link_parameters(l).control_delay.get();
        assert!(delay > 20.0 && delay < 21.0, "Webster's delay at the shipped values, {delay}");
        let saved = with.free_flow_time(l).get() - edited.free_flow_time(l).get();
        assert!((saved - delay).abs() < 1e-9, "the free-flow time loses exactly the delay");
    }
    let (a, b) = (
        TurnTable::build(&edited, edited.signals()),
        TurnTable::build(&without, without.signals()),
    );
    assert_eq!(a.len(), b.len());
    for t in (0..a.len()).map(TurnId::from_index) {
        let full = a.capacity_fraction(t).to_bits();
        assert_eq!(full, 1.0f32.to_bits(), "the full saturation flow through every turn");
        assert_eq!(full, b.capacity_fraction(t).to_bits());
    }
    let shipped = SignalDefaults::SHIPPED.turn_capacity_fraction();
    let before = TurnTable::build(&with, with.signals());
    assert!(
        (0..before.len())
            .map(TurnId::from_index)
            .any(|t| (f64::from(before.capacity_fraction(t)) - shipped).abs() < 1e-6),
        "the signals' green share before the edit"
    );
}

#[test]
fn one_approach_ignored_changes_that_link_only_and_the_node_keeps_its_signal() {
    let (net, _) = manhattan_grid(4, 150.0, true);
    let node = net.node_external_ids().typed_id_of::<NodeId>(&node_name(1, 1)).unwrap();
    let into: Vec<LinkId> = net.in_links(node).to_vec();
    assert_eq!(into.len(), 4);
    assert!(into.iter().all(|&l| net.is_link_signalised(l)));
    let one = net
        .with_changes(&LinkChanges { signal_off: vec![into[0]], ..LinkChanges::default() })
        .unwrap();
    assert!(!one.is_link_signalised(into[0]));
    assert_eq!(one.link_parameters(into[0]).control_delay.get().to_bits(), 0.0f64.to_bits());
    let bare = one.link_length(into[0]).get() / one.link_parameters(into[0]).free_flow_speed.get();
    assert_eq!(one.free_flow_time(into[0]).get().to_bits(), bare.to_bits(), "length over speed");
    assert_eq!(one.link_parameters(into[0]).capacity, net.link_parameters(into[0]).capacity);
    assert!(one.is_signalised(node), "three approaches still have the signal");
    for l in (0..net.link_count()).map(LinkId::new).filter(|&l| l != into[0]) {
        assert_eq!(one.link_parameters(l), net.link_parameters(l), "link {} untouched", l.index());
        assert_eq!(one.free_flow_time(l), net.free_flow_time(l));
        assert_eq!(one.is_link_signalised(l), net.is_link_signalised(l));
    }
    let all = net
        .with_changes(&LinkChanges { signal_off: into.clone(), ..LinkChanges::default() })
        .unwrap();
    assert!(!all.is_signalised(node), "no approach has the signal: the node has none");
    let other = net.node_external_ids().typed_id_of::<NodeId>(&node_name(1, 2)).unwrap();
    assert!(all.is_signalised(other), "another junction keeps its signal");
}

#[test]
fn a_link_without_a_signal_is_left_as_it_is_and_an_unknown_link_is_refused() {
    let (net, _) = manhattan_grid(4, 150.0, true);
    let plain: Vec<LinkId> =
        (0..net.link_count()).map(LinkId::new).filter(|&l| !net.is_link_signalised(l)).collect();
    assert!(!plain.is_empty());
    let edited =
        net.with_changes(&LinkChanges { signal_off: plain, ..LinkChanges::default() }).unwrap();
    assert!(edited == net);
    let far = LinkId::new(net.link_count());
    let err = net
        .with_changes(&LinkChanges { signal_off: vec![far], ..LinkChanges::default() })
        .unwrap_err();
    assert!(err.contains("no link"), "{err}");
}
