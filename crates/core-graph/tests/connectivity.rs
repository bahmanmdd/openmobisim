//! Strong connectivity: the components found are the ones a person can read off
//! a small drawing, at nodes and at links; the link graph splits where the turn
//! rule says it must; a connected verdict means every pair has a route; and a
//! mode's connectivity is asked of that mode's links alone.

use openmobisim_core_graph::connectivity::{
    analyse, analyse_by, link_components, node_components, strong_components,
};
use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::network::{LinkSpec, RoadNetwork, RoadNetworkBuilder};
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::{EntityId, LinkId, NodeId};

/// A network on a west-to-east line: nodes `n0..`, and the directed links
/// given as `(from, to)` pairs of node numbers.
fn line(node_count: u32, links: &[(u32, u32)]) -> RoadNetwork {
    let mut b = RoadNetworkBuilder::new();
    for i in 0..node_count {
        b.add_node(format!("n{i}"), LonLat::new(4.80 + f64::from(i) * 0.001, 45.70));
    }
    for (k, &(from, to)) in links.iter().enumerate() {
        b.add_link(
            format!("l{k:03}"),
            format!("n{from}"),
            format!("n{to}"),
            LinkSpec::new(RoadClass::Residential),
        );
    }
    let mut diag = Diagnostics::new();
    b.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut diag).unwrap()
}

fn node(net: &RoadNetwork, name: &str) -> usize {
    net.node_external_ids().typed_id_of::<NodeId>(name).unwrap().raw() as usize
}

#[test]
fn the_raw_algorithm_finds_the_components_of_a_small_graph() {
    // 0 → 1 → 2 → 0 is a cycle; 2 → 3; 3 → 4 → 3 is another cycle; 5 is alone.
    // Components: {0,1,2}, {3,4}, {5}.
    let edges: [&[u32]; 6] = [&[1], &[2], &[0, 3], &[4], &[3], &[]];
    let mut offsets = vec![0u32];
    let mut targets = Vec::new();
    for e in edges {
        targets.extend_from_slice(e);
        offsets.push(u32::try_from(targets.len()).unwrap());
    }
    let c = strong_components(&offsets, &targets);

    assert_eq!(c.count(), 3);
    assert_eq!(c.component_of(0), c.component_of(1));
    assert_eq!(c.component_of(1), c.component_of(2));
    assert_eq!(c.component_of(3), c.component_of(4));
    assert_ne!(c.component_of(0), c.component_of(3));
    assert_ne!(c.component_of(3), c.component_of(5));
    assert_eq!(c.sizes_descending(), vec![3, 2, 1]);
    assert_eq!(c.size_of(c.largest().unwrap()), 3);
}

#[test]
fn a_path_a_million_long_does_not_overflow_the_stack() {
    // The reason the search is iterative. A recursive one dies well before this.
    let n = 1_000_000u32;
    let offsets: Vec<u32> = (0..=n).collect();
    let targets: Vec<u32> = (1..n).chain([0]).collect(); // one big cycle
    let c = strong_components(&offsets, &targets);
    assert!(c.is_strongly_connected());
    assert_eq!(c.size_of(0), n);
}

#[test]
fn the_empty_graph_has_no_components() {
    let c = strong_components(&[0], &[]);
    assert_eq!(c.count(), 0);
    assert_eq!(c.largest(), None);
    assert!(!c.is_strongly_connected());
}

#[test]
fn a_two_way_street_is_strongly_connected() {
    let net = line(3, &[(0, 1), (1, 0), (1, 2), (2, 1)]);
    let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);
    let r = analyse(&net, &turns);
    assert!(r.is_strongly_connected(), "{r:?}");
    assert_eq!((r.node_largest, r.link_largest), (3, 4));
    assert_eq!((r.sources, r.sinks), (0, 0));
}

#[test]
fn a_one_way_spur_off_a_loop_is_a_sink_and_is_not_in_the_component() {
    // A two-way pair 0 ⇄ 1 with a one-way link 1 → 2 that leads nowhere.
    let net = line(3, &[(0, 1), (1, 0), (1, 2)]);
    let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);
    let nodes = node_components(&net);

    assert_eq!(nodes.count(), 2, "the pair, and the spur's end on its own");
    assert_ne!(nodes.component_of(node(&net, "n2")), nodes.component_of(node(&net, "n0")));
    assert_eq!(nodes.component_of(node(&net, "n0")), nodes.component_of(node(&net, "n1")));

    let r = analyse(&net, &turns);
    assert_eq!((r.sources, r.sinks), (0, 1));
    assert_eq!(r.node_largest, 2);
    assert_eq!(r.links_in_node_largest, 2, "only the two links of the pair");
    assert!(!r.is_strongly_connected());
}

#[test]
fn a_one_way_link_into_the_network_is_a_source() {
    let net = line(3, &[(0, 1), (1, 0), (2, 1)]);
    let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);
    let r = analyse(&net, &turns);
    assert_eq!((r.sources, r.sinks), (1, 0));
    assert_eq!(r.node_largest, 2);
}

#[test]
fn two_islands_are_two_components_and_the_larger_is_chosen() {
    // Islands: a triangle of two-way streets (0,1,2), and a single street (3,4).
    let net = line(5, &[(0, 1), (1, 0), (1, 2), (2, 1), (2, 0), (0, 2), (3, 4), (4, 3)]);
    let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);
    let r = analyse(&net, &turns);
    assert_eq!(r.node_components, 2);
    assert_eq!(r.node_component_sizes_top, vec![3, 2]);
    assert_eq!(r.node_largest, 3);
    assert_eq!(r.links_in_node_largest, 6);
    assert!(!r.is_strongly_connected());
}

#[test]
fn a_two_way_triangle_is_two_link_components_but_still_connects_every_node() {
    // Without a U-turn, a vehicle going round clockwise stays clockwise. The
    // links are therefore two components of three, and requiring one would
    // reject a network in which every trip has a route.
    let net = line(3, &[(0, 1), (1, 0), (1, 2), (2, 1), (2, 0), (0, 2)]);
    let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);
    let r = analyse(&net, &turns);
    assert_eq!((r.link_components, r.link_largest), (2, 3));
    assert!(r.is_strongly_connected());
    assert!(all_pairs_reachable(&net, &turns));
}

#[test]
fn a_link_that_can_only_be_started_on_is_outside_the_core_and_harmless() {
    // 0 ⇄ 1, with a one-way road 1 → 2 → 0 back round. At node 1 the only
    // approach is from 0, and it may not turn back: the link 1 → 0 can only be
    // entered by a trip that starts at node 1. It is its own link component,
    // and every node still reaches every node.
    let ring = line(3, &[(0, 1), (1, 0), (1, 2), (2, 0)]);
    let turns = TurnTable::build(&ring, SignalDefaults::SHIPPED);
    let r = analyse(&ring, &turns);
    assert_eq!((r.link_components, r.link_largest), (2, 3));
    assert!(r.is_strongly_connected(), "{r:?}");
    assert!(all_pairs_reachable(&ring, &turns));
}

#[test]
fn a_link_that_cannot_turn_back_is_in_a_different_link_component() {
    // A two-way street 0 ⇄ 1 whose node 1 has another exit, 1 → 2, into a
    // sink. At the node level 0 and 1 are mutually reachable. At the link
    // level, a vehicle that arrived at 1 from 0 may not turn back (there is
    // another exit), and the other exit is a trap: the link 0 → 1 cannot reach
    // 1 → 0, so they are different link components.
    let trap = line(3, &[(0, 1), (1, 0), (1, 2)]);
    let turns = TurnTable::build(&trap, SignalDefaults::SHIPPED);
    let nodes = node_components(&trap);
    assert_eq!(nodes.component_of(node(&trap, "n0")), nodes.component_of(node(&trap, "n1")));
    let links = link_components(&trap, &turns);
    let find = |from: &str, to: &str| -> usize {
        LinkId::iter_space(trap.link_count())
            .find(|&l| {
                trap.link_from(l).raw() as usize == node(&trap, from)
                    && trap.link_to(l).raw() as usize == node(&trap, to)
            })
            .unwrap()
            .raw() as usize
    };
    assert_ne!(
        links.component_of(find("n0", "n1")),
        links.component_of(find("n1", "n0")),
        "no legal way from the approach to node 1 back to node 0"
    );
    assert!(!analyse(&trap, &turns).is_strongly_connected());
}

/// Whether every node can reach every other node, by trips that start on any
/// link leaving the origin and end on any link entering the destination:
/// a breadth-first search over the link graph from each node, the slow and
/// obviously right way.
fn all_pairs_reachable(net: &RoadNetwork, turns: &TurnTable) -> bool {
    let n = net.node_count();
    for origin in NodeId::iter_space(n) {
        let mut seen = vec![false; net.link_count() as usize];
        let mut queue: Vec<LinkId> = net.out_links(origin).to_vec();
        for &l in &queue {
            seen[l.raw() as usize] = true;
        }
        let mut reached = vec![false; n as usize];
        reached[origin.raw() as usize] = true;
        while let Some(l) = queue.pop() {
            reached[net.link_to(l).raw() as usize] = true;
            for &t in turns.turns_from(l) {
                let next = turns.outgoing(t);
                if !seen[next.raw() as usize] {
                    seen[next.raw() as usize] = true;
                    queue.push(next);
                }
            }
        }
        if reached.iter().any(|&r| !r) {
            return false;
        }
    }
    true
}

/// A small random directed graph, from a seed: a linear congruential
/// generator, so the test needs no dependency and every failure reproduces.
fn random_links(seed: u64, nodes: u32) -> Vec<(u32, u32)> {
    let mut state = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    let mut next = move || {
        state =
            state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        (state >> 33) as u32
    };
    let mut links = Vec::new();
    for a in 0..nodes {
        for b in 0..nodes {
            if a == b {
                continue;
            }
            // Roughly a third of the possible links; a coin decides whether a
            // link is two-way (both directions) or one-way.
            if next() % 100 < 22 {
                links.push((a, b));
                if next() % 2 == 0 {
                    links.push((b, a));
                }
            }
        }
    }
    links.sort_unstable();
    links.dedup();
    links
}

#[test]
fn the_criterion_is_sound_a_connected_verdict_means_every_pair_has_a_route() {
    // The claim in the module docs — a core with an entry and an exit at every
    // node is enough — checked against brute force on many small graphs, of
    // which some are connected and some are not, and several are the awkward
    // kind (two-way streets that cannot turn round).
    let (mut connected, mut not) = (0, 0);
    for seed in 0..400u64 {
        let nodes = 4 + u32::try_from(seed % 5).unwrap();
        let links = random_links(seed, nodes);
        if links.is_empty() {
            continue;
        }
        let net = line(nodes, &links);
        let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);
        let verdict = analyse(&net, &turns).is_strongly_connected();
        let truth = all_pairs_reachable(&net, &turns);
        if verdict {
            assert!(truth, "seed {seed}: called connected, but a pair has no route: {links:?}");
            connected += 1;
        } else {
            not += 1;
        }
    }
    assert!(connected > 20 && not > 20, "the sample must contain both kinds: {connected}/{not}");
}

#[test]
fn a_dead_end_keeps_the_network_connected_because_it_keeps_its_u_turn() {
    // Two-way 0 ⇄ 1 ⇄ 2: both ends are dead ends, each with a U-turn.
    let net = line(3, &[(0, 1), (1, 0), (1, 2), (2, 1)]);
    let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);
    assert_eq!(turns.u_turn_count(), 2);
    assert!(link_components(&net, &turns).is_strongly_connected());
}

#[test]
fn a_grid_is_one_component_at_both_levels() {
    let (net, _) = openmobisim_core_graph::examples::manhattan_grid(6, 100.0, false);
    let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);
    let r = analyse(&net, &turns);
    assert!(r.is_strongly_connected());
    assert_eq!(r.node_largest, 36);
    assert_eq!(r.link_largest, net.link_count());
}

#[test]
fn a_footway_does_not_connect_what_a_car_cannot_cross() {
    // Two separate two-way roads, 0 ⇄ 1 and 2 ⇄ 3, and a footway 1 ⇄ 2 between
    // them. Counting every link the network is one component; for cars it is two.
    let mut b = RoadNetworkBuilder::new();
    for i in 0..4 {
        b.add_node(format!("n{i}"), LonLat::new(4.80 + f64::from(i) * 0.001, 45.70));
    }
    let links = [
        (0, 1, RoadClass::Residential),
        (1, 0, RoadClass::Residential),
        (1, 2, RoadClass::Footway),
        (2, 1, RoadClass::Footway),
        (2, 3, RoadClass::Residential),
        (3, 2, RoadClass::Residential),
    ];
    for (k, (from, to, class)) in links.into_iter().enumerate() {
        b.add_link(format!("l{k}"), format!("n{from}"), format!("n{to}"), LinkSpec::new(class));
    }
    let mut diag = Diagnostics::new();
    let net = b.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut diag).unwrap();
    let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);

    assert!(analyse(&net, &turns).is_strongly_connected(), "on foot it is one network");
    let cars = analyse_by(&net, &turns, |l| net.link_class(l).carries_motor_traffic());
    assert!(!cars.is_strongly_connected());
    assert_eq!((cars.nodes, cars.links, cars.node_components), (4, 4, 2));
    assert_eq!(cars.node_component_sizes_top, vec![2, 2]);
}

#[test]
fn a_mode_with_no_links_has_an_empty_report() {
    let net = line(3, &[(0, 1), (1, 0)]);
    let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);
    let r = analyse_by(&net, &turns, |_| false);
    assert_eq!((r.nodes, r.links, r.node_components), (0, 0, 0));
    assert!(!r.is_strongly_connected());
}

#[test]
fn a_node_no_link_touches_is_a_component_of_its_own() {
    // Three nodes, one street between two of them: the third is stranded.
    let net = line(3, &[(0, 1), (1, 0)]);
    let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);
    let r = analyse(&net, &turns);
    assert_eq!((r.nodes, r.node_components), (3, 2));
    assert_eq!(r.node_component_sizes_top, vec![2, 1]);
    assert!(!r.is_strongly_connected());
}
