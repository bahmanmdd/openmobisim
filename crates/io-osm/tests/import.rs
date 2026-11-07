//! Ways into links, by hand: where ways are split, what a two-way street becomes, the signal
//! flag, ways cut by the extract's edge, closed ways, and the same network whatever the order
//! of the input. Bikes: a split, a cut and a signal mean the same for a bike link as for a car.

mod common;

use common::{at, direction, read, street};
use openmobisim_io_osm::{Osm, OsmNode, OsmWay};
use openmobisim_types::ids::{EntityId, LinkId, NodeId};

fn residential(id: i64, nodes: &[i64]) -> OsmWay {
    OsmWay::new(id, nodes, &[("highway", "residential")])
}

fn node_of(net: &openmobisim_graph::network::Network, osm_id: &str) -> NodeId {
    net.node_ids().typed_id_of::<NodeId>(osm_id).expect("a network node")
}

#[test]
fn a_two_way_street_is_two_links_and_its_inner_nodes_are_not_network_nodes() {
    // Way 100 through nodes 1..=5 and the return street 5-999-1: nodes 2, 3, 4 and 999 are
    // used once and are no junctions; the network nodes are the ends, 1 and 5.
    let (net, report) = read(&street(5, &[("highway", "residential")])).unwrap();
    assert_eq!(net.node_count(), 2);
    assert_eq!(report.nodes, 2);
    assert_eq!(net.link_count(), 4, "two links each for ways 100 and 900");
    let f = direction(&net, 100, "f").unwrap();
    let b = direction(&net, 100, "b").unwrap();
    assert_eq!((f.links.len(), b.links.len()), (1, 1));
    assert_eq!(net.upstream(f.links[0]), node_of(&net, "1"));
    assert_eq!(net.downstream(f.links[0]), node_of(&net, "5"));
    assert_eq!(net.upstream(b.links[0]), node_of(&net, "5"));
    assert_eq!(f.length_m.to_bits(), b.length_m.to_bits(), "both directions: the way's one length");
}

#[test]
fn ways_split_where_they_share_a_node() {
    // A T: way 100 through 1, 2, 3 and way 101 from 2 north to 4. Node 2 is used twice, so
    // way 100 is two pieces (1-2, 2-3); each street two-way: 3 streets x 2 = 6 links, plus the
    // return street 3-999-1 (2 links).
    let mut osm = street(3, &[("highway", "residential")]);
    osm.nodes.push(OsmNode::new(4, 4.362, 52.001));
    osm.ways.push(residential(101, &[2, 4]));
    let (net, _) = read(&osm).unwrap();
    assert_eq!(net.link_count(), 8);
    assert_eq!(net.out_links(node_of(&net, "2")).len(), 3, "the T's junction has three exits");
    let f = direction(&net, 100, "f").unwrap();
    let ids: Vec<&str> = f.links.iter().map(|&l| net.link_ids().external(l.raw())).collect();
    assert_eq!(ids, ["100:0:f", "100:1:f"], "pieces numbered along the way");
}

#[test]
fn a_signal_splits_the_way_and_flags_the_links_that_end_there() {
    // Node 2 tagged highway=traffic_signals: a network node, signalised; the links into it
    // (1→2 and 3→2) carry the downstream-end flag, the links out of it do not. Stored only.
    let mut osm = street(3, &[("highway", "residential")]);
    osm.nodes[1] = at(2, 2).with_tags(&[("highway", "traffic_signals")]);
    let (net, _) = read(&osm).unwrap();
    let signal = node_of(&net, "2");
    assert!(net.is_signalised(signal));
    for l in LinkId::iter_space(net.link_count()) {
        assert_eq!(net.has_signal_downstream(l), net.downstream(l) == signal, "{l}");
    }
    assert_eq!(net.in_links(signal).len(), 2);
}

#[test]
fn a_way_cut_by_the_extract_edge_keeps_what_it_holds_and_bridges_nothing() {
    // Way 100 through 1..=5, node 3 missing: two runs, 1-2 and 4-5, no link across 2-4.
    let mut osm = street(5, &[("highway", "cycleway")]);
    osm.nodes.retain(|n| n.id != 3);
    let (net, report) = read(&osm).unwrap();
    assert_eq!(report.ways_cut, 1);
    let f = direction(&net, 100, "f").unwrap();
    let ends: Vec<(NodeId, NodeId)> =
        f.links.iter().map(|&l| (net.upstream(l), net.downstream(l))).collect();
    assert_eq!(
        ends,
        [(node_of(&net, "1"), node_of(&net, "2")), (node_of(&net, "4"), node_of(&net, "5"))]
    );
}

#[test]
fn a_closed_way_touching_nothing_is_dropped_and_counted() {
    // A ring 1-2-3-1 with no other street: its one piece starts and ends at node 1.
    let osm = Osm {
        nodes: vec![at(1, 0), at(2, 1), OsmNode::new(3, 4.3605, 52.001), at(4, 5), at(5, 6)],
        ways: vec![
            OsmWay::new(100, &[1, 2, 3, 1], &[("highway", "footway")]),
            residential(200, &[4, 5]),
        ],
    };
    let (net, report) = read(&osm).unwrap();
    assert_eq!(report.loops_dropped, 1);
    assert!(direction(&net, 100, "f").is_none());
    // A ring that a street touches at a second node is two pieces and kept.
    let mut touched = osm.clone();
    touched.ways.push(residential(300, &[2, 5]));
    let (net, report) = read(&touched).unwrap();
    assert_eq!(report.loops_dropped, 0);
    assert_eq!(direction(&net, 100, "f").unwrap().links.len(), 2);
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
    assert!(direction(&net, 100, "f").is_none(), "a square is not a street");
}
