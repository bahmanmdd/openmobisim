//! Turns: the counts, the U-turn rule, and the signal fraction.

mod common;

use common::{grid, node_name};
use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::network::{LinkSpec, RoadNetworkBuilder};
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::{EntityId, LinkId, NodeId, TurnId};

#[test]
fn a_grid_has_the_turns_a_grid_should_have() {
    let (net, _) = grid(4, false);
    let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);

    // Every node has in-degree == out-degree == d, and every approach has an
    // opposite exit, so each node contributes d² − d turns once U-turns are
    // excluded. A 4×4 grid: 4 corners (d = 2), 8 edges (d = 3), 4 interior
    // (d = 4).
    let expected = 4 * (2 * 2 - 2) + 8 * (3 * 3 - 3) + 4 * (4 * 4 - 4);
    assert_eq!(turns.len(), expected, "turn count");
    assert_eq!(turns.u_turn_count(), 0, "a grid has no dead ends, so no U-turns");
    assert_eq!(turns.max_turns_per_node(), 12);
}

#[test]
fn u_turns_are_excluded_where_there_is_another_way_out() {
    let (net, _) = grid(3, false);
    let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);
    for turn in TurnId::iter_space(u32::try_from(turns.len()).unwrap()) {
        assert!(
            !net.is_reverse_of(turns.incoming(turn), turns.outgoing(turn)),
            "a U-turn survived at a node with other exits"
        );
    }
}

#[test]
fn a_dead_end_keeps_its_u_turn() {
    // Otherwise a traveller who enters a cul-de-sac is stranded, and the route
    // search has to special-case it.
    let mut b = RoadNetworkBuilder::new();
    b.add_node("a", LonLat::new(4.800, 45.70));
    b.add_node("b", LonLat::new(4.801, 45.70));
    b.add_node("c", LonLat::new(4.802, 45.70)); // the dead end
    for (from, to) in [("a", "b"), ("b", "a"), ("b", "c"), ("c", "b")] {
        b.add_link(format!("l_{from}_{to}"), from, to, LinkSpec::new(RoadClass::Residential));
    }

    let mut diag = Diagnostics::new();
    let net = b.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut diag).unwrap();
    let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);

    // Both ends of this stub are dead ends, so both keep a U-turn.
    for end in ["a", "c"] {
        let node = net.node_external_ids().typed_id_of::<NodeId>(end).unwrap();
        let approach = net.in_links(node)[0];
        let movements = turns.turns_from(approach);
        assert_eq!(movements.len(), 1, "dead end {end} has exactly one movement");
        assert!(
            net.is_reverse_of(approach, turns.outgoing(movements[0])),
            "and at {end} it is the U-turn"
        );
    }
    assert_eq!(turns.u_turn_count(), 2);

    // The middle node has another way out, so its U-turns are gone: two
    // approaches, one through movement each.
    let b_node = net.node_external_ids().typed_id_of::<NodeId>("b").unwrap();
    assert_eq!(turns.turns_at(b_node).len(), 2);
    for &t in turns.turns_at(b_node) {
        assert!(!net.is_reverse_of(turns.incoming(t), turns.outgoing(t)));
    }
}

#[test]
fn signals_set_the_turn_capacity_fraction() {
    let (plain, _) = grid(5, false);
    let plain_turns = TurnTable::build(&plain, SignalDefaults::SHIPPED);
    let (signalled, _) = grid(5, true);
    let signalled_turns = TurnTable::build(&signalled, SignalDefaults::SHIPPED);

    // An unsignalised network lets everything through.
    for t in TurnId::iter_space(u32::try_from(plain_turns.len()).unwrap()) {
        assert!((plain_turns.capacity_fraction(t) - 1.0).abs() < 1e-6);
    }

    let interior = signalled.node_external_ids().typed_id_of::<NodeId>(&node_name(2, 2)).unwrap();
    let at_signal = signalled_turns.turns_at(interior);
    assert!(!at_signal.is_empty());
    for &t in at_signal {
        assert!(
            (f64::from(signalled_turns.capacity_fraction(t))
                - SignalDefaults::SHIPPED.green_fraction)
                .abs()
                < 1e-6,
            "a signalised turn must carry the green-time fraction"
        );
    }

    // A corner node is not signalised even in the signalised grid.
    let corner = signalled.node_external_ids().typed_id_of::<NodeId>(&node_name(0, 0)).unwrap();
    for &t in signalled_turns.turns_at(corner) {
        assert!((signalled_turns.capacity_fraction(t) - 1.0).abs() < 1e-6);
    }
}

#[test]
fn the_two_indexes_agree_with_each_other() {
    // `turns_from` and `turns_at` are two views of one set; a turn must appear
    // in exactly one of each, and the flat arrays must agree with both.
    let (net, _) = grid(4, true);
    let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);

    let mut seen_via_links = 0usize;
    for link in LinkId::iter_space(net.link_count()) {
        for &t in turns.turns_from(link) {
            assert_eq!(turns.incoming(t), link);
            assert_eq!(turns.node(t), net.link_to(link));
            seen_via_links += 1;
        }
    }
    assert_eq!(seen_via_links, turns.len());

    let mut seen_via_nodes = 0usize;
    for node in NodeId::iter_space(net.node_count()) {
        for &t in turns.turns_at(node) {
            assert_eq!(turns.node(t), node);
            seen_via_nodes += 1;
        }
    }
    assert_eq!(seen_via_nodes, turns.len());
}

#[test]
fn every_turn_connects_links_that_actually_meet() {
    let (net, _) = grid(4, false);
    let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);
    for t in TurnId::iter_space(u32::try_from(turns.len()).unwrap()) {
        let (i, o) = (turns.incoming(t), turns.outgoing(t));
        assert_eq!(net.link_to(i), turns.node(t));
        assert_eq!(net.link_from(o), turns.node(t));
    }
}

#[test]
fn a_turn_can_be_found_from_the_pair_of_links_it_joins() {
    let (net, _) = grid(3, false);
    let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);
    let centre = net.node_external_ids().typed_id_of::<NodeId>(&node_name(1, 1)).unwrap();

    let approach = net.in_links(centre)[0];
    for &exit in net.out_links(centre) {
        let found = turns.find(approach, exit);
        if net.is_reverse_of(approach, exit) {
            assert!(found.is_none(), "the U-turn must not be findable at a through node");
        } else {
            let t = found.expect("a through movement must exist");
            assert_eq!(turns.incoming(t), approach);
            assert_eq!(turns.outgoing(t), exit);
        }
    }
}

#[test]
fn a_network_with_no_movements_produces_no_turns() {
    // Two nodes joined one way: the downstream node has no exit, the upstream
    // none entering. Neither contributes a turn.
    let mut b = RoadNetworkBuilder::new();
    b.add_node("a", LonLat::new(4.800, 45.70));
    b.add_node("b", LonLat::new(4.801, 45.70));
    b.add_link("l", "a", "b", LinkSpec::new(RoadClass::Residential));

    let mut diag = Diagnostics::new();
    let net = b.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut diag).unwrap();
    let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);
    assert!(turns.is_empty());
    assert_eq!(turns.max_turns_per_node(), 0);
}

#[test]
fn turn_ids_are_a_pure_function_of_the_network() {
    // Turn ids reach diagnostics and the observation API, so building the same
    // network twice must give the same turn ids.
    let (net, _) = grid(4, true);
    let a = TurnTable::build(&net, SignalDefaults::SHIPPED);
    let b = TurnTable::build(&net, SignalDefaults::SHIPPED);
    assert_eq!(a.len(), b.len());
    for t in TurnId::iter_space(u32::try_from(a.len()).unwrap()) {
        assert_eq!(a.incoming(t), b.incoming(t));
        assert_eq!(a.outgoing(t), b.outgoing(t));
        assert_eq!(a.node(t), b.node(t));
    }
}
