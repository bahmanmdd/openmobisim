//! Ways into links, by hand: where ways are split, what a two-way street becomes, the signal
//! flag, ways cut by the extract's edge, closed ways, contraction (version 1's tests: two ways
//! that are one street merged, not across a change, a signal or into a loop; a chain collapsed;
//! the length conserved; a one-way street meeting a two-way one kept), and the same network
//! whatever the order of the input. Bikes: a split, a cut, a signal and a merge mean the same
//! on the bike layer as on the car layer.

mod common;

use common::{at, direction, read, street};
use openmobisim_graph::network::Network;
use openmobisim_io_osm::geodesic::geodesic_m;
use openmobisim_io_osm::{Osm, OsmNode, OsmWay};
use openmobisim_types::ids::{EntityId, LinkId, NodeId};
use openmobisim_types::mode::Mode;

fn residential(id: i64, nodes: &[i64]) -> OsmWay {
    OsmWay::new(id, nodes, &[("highway", "residential")])
}

fn node_of(net: &Network, osm_id: &str) -> NodeId {
    net.node_ids().typed_id_of::<NodeId>(osm_id).expect("a network node")
}

/// The links of `mode`.
fn of_mode(net: &Network, mode: Mode) -> Vec<LinkId> {
    LinkId::iter_space(net.link_count()).filter(|&l| net.modes(l).contains(mode)).collect()
}

/// Nodes 1..=n in a row (about 69 m apart), with the ways given.
fn row(n: i64, ways: Vec<OsmWay>) -> Osm {
    Osm { nodes: (1..=n).map(|i| at(i, i)).collect(), ways }
}

#[test]
fn a_two_way_street_is_two_links_per_mode_and_its_inner_nodes_are_not_network_nodes() {
    // Way 100 through nodes 1..=5 and the return street 5-999-1: nodes 2, 3, 4 and 999 are
    // used once and are no junctions; the network nodes are the ends, 1 and 5.
    let (net, report) = read(&street(5, &[("highway", "residential")])).unwrap();
    assert_eq!((net.node_count(), report.nodes), (2, 2));
    for mode in [Mode::Car, Mode::Bike, Mode::Walk] {
        assert_eq!(of_mode(&net, mode).len(), 4, "{mode}: two links each for ways 100 and 900");
        let f = direction(&net, mode, 100, "f").unwrap();
        let b = direction(&net, mode, 100, "b").unwrap();
        assert_eq!((f.links.len(), b.links.len()), (1, 1));
        assert_eq!(net.upstream(f.links[0]), node_of(&net, "1"));
        assert_eq!(net.downstream(f.links[0]), node_of(&net, "5"));
        assert_eq!(net.upstream(b.links[0]), node_of(&net, "5"));
        assert_eq!(f.length_m.to_bits(), b.length_m.to_bits(), "both ways: the way's length");
    }
    assert_eq!(net.link_count(), 12, "one mode per link: 4 links on each of three layers");
}

#[test]
fn ways_split_where_they_share_a_node() {
    // A T: way 100 through 1, 2, 3 and way 101 from 2 north to 4. Node 2 is used twice, so way
    // 100 is two pieces (1-2, 2-3); per mode, 3 streets × 2 = 6 links.
    let mut osm = row(3, vec![residential(100, &[1, 2, 3]), residential(101, &[2, 4])]);
    osm.nodes.push(OsmNode::new(4, 4.362, 52.001));
    let (net, _) = read(&osm).unwrap();
    assert_eq!(of_mode(&net, Mode::Car).len(), 6);
    assert_eq!(net.out_links(node_of(&net, "2")).len(), 9, "three exits on each of three layers");
    let f = direction(&net, Mode::Car, 100, "f").unwrap();
    let ids: Vec<&str> = f.links.iter().map(|&l| net.link_ids().external(l.raw())).collect();
    assert_eq!(ids, ["car:100:0:f", "car:100:1:f"], "pieces numbered along the way");
}

#[test]
fn a_signal_splits_the_way_and_flags_the_links_that_end_there() {
    // Node 2 tagged highway=traffic_signals: a network node, signalised, never merged away;
    // the links into it (1→2 and 3→2, on each layer) carry the downstream-end flag, the links
    // out of it do not. Stored only.
    let mut osm = street(3, &[("highway", "residential")]);
    osm.nodes[1] = at(2, 2).with_tags(&[("highway", "traffic_signals")]);
    let (net, _) = read(&osm).unwrap();
    let signal = node_of(&net, "2");
    assert!(net.is_signalised(signal));
    for l in LinkId::iter_space(net.link_count()) {
        assert_eq!(net.has_signal_downstream(l), net.downstream(l) == signal, "{l}");
    }
    assert_eq!(net.in_links(signal).len(), 6, "two on each layer");
}

#[test]
fn a_way_cut_by_the_extract_edge_keeps_what_it_holds_and_bridges_nothing() {
    // Way 100 through 1..=5, node 3 missing: two runs, 1-2 and 4-5, no link across 2-4.
    let mut osm = street(5, &[("highway", "cycleway")]);
    osm.nodes.retain(|n| n.id != 3);
    let (net, report) = read(&osm).unwrap();
    assert_eq!(report.ways_cut, 1);
    let f = direction(&net, Mode::Bike, 100, "f").unwrap();
    let ends: Vec<(NodeId, NodeId)> =
        f.links.iter().map(|&l| (net.upstream(l), net.downstream(l))).collect();
    assert_eq!(
        ends,
        [(node_of(&net, "1"), node_of(&net, "2")), (node_of(&net, "4"), node_of(&net, "5"))]
    );
}

#[test]
fn a_closed_way_touching_nothing_is_dropped_and_counted() {
    // A footway ring 1-2-3-1 with no other way: its one piece starts and ends at node 1.
    let osm = Osm {
        nodes: vec![at(1, 0), at(2, 1), OsmNode::new(3, 4.3605, 52.001), at(4, 5), at(5, 6)],
        ways: vec![
            OsmWay::new(100, &[1, 2, 3, 1], &[("highway", "footway")]),
            residential(200, &[4, 5]),
        ],
    };
    let (net, report) = read(&osm).unwrap();
    assert_eq!(report.loops_dropped, 1, "on the walk layer");
    assert!(direction(&net, Mode::Walk, 100, "f").is_none());
    // A ring that a street touches at a second node is two pieces and kept: merging them again
    // would make a link from node 1 to itself.
    let mut touched = osm.clone();
    touched.ways.push(residential(300, &[2, 5]));
    let (net, report) = read(&touched).unwrap();
    assert_eq!(report.loops_dropped, 0);
    assert_eq!(direction(&net, Mode::Walk, 100, "f").unwrap().links.len(), 2);
}

#[test]
fn two_ways_that_are_one_street_are_merged_and_keep_their_length() {
    // OpenStreetMap splits a street where a name changes: the node between is no junction.
    let osm = row(3, vec![residential(100, &[1, 2]), residential(101, &[2, 3])]);
    let (net, _) = read(&osm).unwrap();
    assert_eq!(net.node_count(), 2, "the joint between the two ways is gone");
    let whole = geodesic_m((4.361, 52.0), (4.362, 52.0)).unwrap()
        + geodesic_m((4.362, 52.0), (4.363, 52.0)).unwrap();
    for mode in [Mode::Car, Mode::Bike, Mode::Walk] {
        let links = of_mode(&net, mode);
        assert_eq!(links.len(), 2, "{mode}");
        for l in links {
            assert!((net.length(l).0 - whole).abs() < 1e-9, "{mode}: {} m", net.length(l).0);
            assert_eq!(net.shape(l).len(), 1, "the joint is the merged link's bend");
        }
    }
}

#[test]
fn contraction_stops_where_the_car_fields_change_at_a_signal_and_before_a_loop() {
    // A speed limit change: the car links stay apart; the bike and walk links, which carry no
    // car field, merge.
    let osm = row(
        3,
        vec![
            OsmWay::new(100, &[1, 2], &[("highway", "residential"), ("maxspeed", "30")]),
            OsmWay::new(101, &[2, 3], &[("highway", "residential"), ("maxspeed", "50")]),
        ],
    );
    let (net, _) = read(&osm).unwrap();
    assert_eq!(net.node_count(), 3);
    let counts = [Mode::Car, Mode::Bike, Mode::Walk].map(|m| of_mode(&net, m).len());
    assert_eq!(counts, [4, 2, 2]);
    // A class change: every layer stays apart (the class is each link's drawing).
    let osm = row(
        3,
        vec![
            residential(100, &[1, 2]),
            OsmWay::new(101, &[2, 3], &[("highway", "tertiary"), ("maxspeed", "30")]),
        ],
    );
    let (net, _) = read(&osm).unwrap();
    assert_eq!([Mode::Car, Mode::Bike, Mode::Walk].map(|m| of_mode(&net, m).len()), [4, 4, 4]);
    // A signal between them.
    let mut osm = row(3, vec![residential(100, &[1, 2]), residential(101, &[2, 3])]);
    osm.nodes[1] = at(2, 2).with_tags(&[("highway", "traffic_signals")]);
    assert_eq!(read(&osm).unwrap().0.node_count(), 3);
    // No link from a node to itself: a cul-de-sac's far end stays.
    let (net, _) =
        read(&row(3, vec![residential(100, &[1, 2]), residential(101, &[2, 3])])).unwrap();
    for l in LinkId::iter_space(net.link_count()) {
        assert_ne!(net.upstream(l), net.downstream(l), "a self-loop");
    }
}

#[test]
fn a_long_chain_collapses_to_one_link_each_way_and_the_length_is_conserved() {
    // Ten ways describing one straight street, and a stub off node 6.
    let mut ways: Vec<OsmWay> = (0..10).map(|i| residential(100 + i, &[i + 1, i + 2])).collect();
    ways.push(residential(300, &[6, 50]));
    let mut osm = row(11, ways);
    osm.nodes.push(OsmNode::new(50, 4.366, 52.001));
    let (net, _) = read(&osm).unwrap();
    assert_eq!(net.node_count(), 4, "the two ends, the stub's junction and its end");
    let segments: f64 = (1..=10)
        .map(|i| {
            geodesic_m((4.36 + 0.001 * i as f64, 52.0), (4.361 + 0.001 * i as f64, 52.0)).unwrap()
        })
        .sum::<f64>()
        + geodesic_m((4.366, 52.0), (4.366, 52.001)).unwrap();
    for mode in [Mode::Car, Mode::Bike, Mode::Walk] {
        assert_eq!(of_mode(&net, mode).len(), 6, "{mode}: 1-6, 6-11 and the stub, each way");
        let total: f64 = of_mode(&net, mode).iter().map(|&l| net.length(l).0).sum();
        assert!(
            (total - 2.0 * segments).abs() < 1e-6,
            "{mode}: {total} m, by hand {}",
            2.0 * segments
        );
    }
}

#[test]
fn a_corner_is_a_bend_not_a_junction() {
    // Two streets meeting at a right angle with nothing else there: one link each way, as long
    // as both legs.
    let osm = Osm {
        nodes: vec![at(1, 0), at(2, 2), OsmNode::new(3, 4.362, 52.002)],
        ways: vec![residential(100, &[1, 2]), residential(101, &[2, 3])],
    };
    let (net, _) = read(&osm).unwrap();
    assert_eq!(net.node_count(), 2);
    let legs = geodesic_m((4.36, 52.0), (4.362, 52.0)).unwrap()
        + geodesic_m((4.362, 52.0), (4.362, 52.002)).unwrap();
    let l = of_mode(&net, Mode::Car)[0];
    assert!((net.length(l).0 - legs).abs() < 1e-9);
}

#[test]
fn a_footway_crossing_a_street_splits_the_walk_layer_not_the_car_layer() {
    // Street 1-2-3; a footway 5-2-6 across it at node 2 (a crossing). Version 1 contracted the
    // car links as if the footway were not there: the cars see one street.
    let mut osm = row(3, vec![residential(100, &[1, 2, 3])]);
    osm.nodes.extend([OsmNode::new(5, 4.362, 52.001), OsmNode::new(6, 4.362, 51.999)]);
    osm.ways.push(OsmWay::new(200, &[5, 2, 6], &[("highway", "footway")]));
    let (net, _) = read(&osm).unwrap();
    assert_eq!(of_mode(&net, Mode::Car).len(), 2);
    assert_eq!(of_mode(&net, Mode::Bike).len(), 2);
    assert_eq!(
        of_mode(&net, Mode::Walk).len(),
        8,
        "the street's 2 pieces and the footway's 2, each way"
    );
}

#[test]
fn a_one_way_street_meeting_a_two_way_one_keeps_the_node_and_every_movement() {
    // One-way street 100 from 1 through 2 to 3; a two-way street 101 from 2 to 4; a two-way
    // street 102 back from 3 to 1. At node 2 two car links come in (from 1, from 4) and two go
    // out (to 3, to 4): no street's two directions. Version 1 could pair 1→2 with 2→4 and
    // 4→2 with 2→3 and merge the node away, losing 1→2→3; here node 2 stays.
    let mut osm = row(
        4,
        vec![
            OsmWay::new(100, &[1, 2, 3], &[("highway", "residential"), ("oneway", "yes")]),
            residential(101, &[2, 4]),
            residential(102, &[3, 1]),
        ],
    );
    osm.nodes[3] = OsmNode::new(4, 4.362, 52.001);
    let (net, _) = read(&osm).unwrap();
    let two = node_of(&net, "2");
    let cars = |links: &[LinkId]| -> Vec<String> {
        links
            .iter()
            .filter(|&&l| net.modes(l).contains(Mode::Car))
            .map(|&l| net.node_ids().external(net.downstream(l).raw()).to_owned())
            .collect()
    };
    let mut exits = cars(net.out_links(two));
    exits.sort();
    assert_eq!(exits, ["3", "4"], "a car from 1 goes on to 3, or turns to 4");
}

#[test]
fn the_same_extract_in_any_order_gives_the_same_network() {
    let mut osm = street(4, &[("highway", "residential")]);
    osm.nodes.push(OsmNode::new(5, 4.362, 52.001));
    osm.ways.push(OsmWay::new(101, &[2, 5], &[("highway", "cycleway")]));
    let mut reversed = osm.clone();
    reversed.nodes.reverse();
    reversed.ways.reverse();
    let (a, ra) = read(&osm).unwrap();
    let (b, rb) = read(&reversed).unwrap();
    assert_eq!(ra, rb);
    assert_eq!(a.link_count(), b.link_count());
    assert_eq!(a.fingerprint(), b.fingerprint());
    for l in LinkId::iter_space(a.link_count()) {
        assert_eq!(a.link_ids().external(l.raw()), b.link_ids().external(l.raw()));
        assert_eq!((a.upstream(l), a.downstream(l)), (b.upstream(l), b.downstream(l)));
        assert_eq!(a.length(l).0.to_bits(), b.length(l).0.to_bits());
        assert_eq!(a.modes(l), b.modes(l));
    }
}

#[test]
fn an_extract_with_no_street_is_an_error() {
    let osm = Osm {
        nodes: vec![at(1, 0), at(2, 1)],
        ways: vec![OsmWay::new(1, &[1, 2], &[("building", "yes")])],
    };
    assert!(matches!(read(&osm), Err(openmobisim_io_osm::OsmError::NoStreets)));
    let area = street(2, &[("highway", "pedestrian"), ("area", "yes")]);
    let (net, _) = read(&area).unwrap();
    assert!(direction(&net, Mode::Walk, 100, "f").is_none(), "a square is not a street");
}
