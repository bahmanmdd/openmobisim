//! Each layer's largest strongly connected component, by drawing: a link a mode cannot leave or
//! cannot reach on its own layer is dropped and counted, and the other layers keep the street if
//! they can use it. Bikes: the bike layer has its own component, joined by a cycleway the car
//! layer does not have.
//!
//! The drawing: a square of two-way residential streets A-B-C-D (nodes 1-4), a one-way spur
//! from C to E (node 5) with no way back, a separate two-way street F-G (6-7, an island for
//! cars), and a cycleway from G to A.

mod common;

use common::{direction, modes, read};
use openmobisim_graph::network::Network;
use openmobisim_io_osm::{Osm, OsmNode, OsmWay};
use openmobisim_types::ids::{EntityId, LinkId};
use openmobisim_types::mode::Mode;

fn drawing() -> Osm {
    let nodes = [
        (1, 0.0, 0.0),
        (2, 1.0, 0.0),
        (3, 1.0, 1.0),
        (4, 0.0, 1.0),
        (5, 2.0, 1.0),
        (6, -2.0, 0.0),
        (7, -1.0, 0.0),
    ];
    let r = [("highway", "residential")];
    Osm {
        nodes: nodes
            .iter()
            .map(|&(id, x, y)| OsmNode::new(id, 4.36 + x * 0.001, 52.0 + y * 0.001))
            .collect(),
        ways: vec![
            OsmWay::new(10, &[1, 2], &r),
            OsmWay::new(11, &[2, 3], &r),
            OsmWay::new(12, &[3, 4], &r),
            OsmWay::new(13, &[4, 1], &r),
            OsmWay::new(20, &[3, 5], &[("highway", "residential"), ("oneway", "yes")]),
            OsmWay::new(30, &[6, 7], &r),
            OsmWay::new(40, &[7, 1], &[("highway", "cycleway")]),
        ],
    }
}

/// The OSM node ids `mode`'s links touch.
fn touched(net: &Network, mode: Mode) -> Vec<String> {
    let mut ids: Vec<String> = LinkId::iter_space(net.link_count())
        .filter(|&l| net.modes(l).contains(mode))
        .flat_map(|l| [net.upstream(l), net.downstream(l)])
        .map(|n| net.node_ids().external(n.raw()).to_owned())
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

#[test]
fn each_layer_keeps_its_own_strong_component() {
    let (net, report) = read(&drawing()).unwrap();
    // Dropped: by car the spur C→E and the island's F→G and G→F (3); by bike the spur (1): it
    // is one-way for bikes too. Walkers walk every street both ways: nothing dropped.
    assert_eq!(report.links_unreachable, 4);
    // Cars keep the square (its corners B and D merged away, A and C kept from a loop, by hand:
    // node 1 is merged first, then node 3, leaving B and D).
    assert_eq!(touched(&net, Mode::Car), ["2", "4"]);
    // Bikes keep the square, the cycleway and the island; walkers also the spur both ways.
    assert_eq!(touched(&net, Mode::Bike), ["1", "3", "6", "7"]);
    assert_eq!(touched(&net, Mode::Walk), ["1", "3", "5", "6", "7"]);
    assert_eq!(modes(&net, 20, "f"), "w", "the spur: walkers alone");
    assert_eq!(modes(&net, 20, "b"), "w");
    for tag in ["f", "b"] {
        assert_eq!(modes(&net, 30, tag), "bw", "the island: bikes and walkers");
        assert_eq!(modes(&net, 40, tag), "bw", "the cycleway, unchanged");
        let island = direction(&net, Mode::Bike, 30, tag).unwrap();
        assert_eq!(net.road_class(island.links[0]), Some("residential"), "its class kept");
    }
}

#[test]
fn a_network_already_strongly_connected_loses_nothing() {
    let mut osm = drawing();
    osm.ways.retain(|w| w.id != 20 && w.id != 30);
    let (_, report) = read(&osm).unwrap();
    assert_eq!(report.links_unreachable, 0);
}

#[test]
fn a_footway_does_not_save_a_road_fragment_a_car_cannot_reach() {
    // Version 1's case: a square of roads (1-4) and a small road pair 5-6 off to the side,
    // joined to the square only by a footway 4-5. One component for walkers; two by car, of
    // which the smaller (5-6) loses its car links; the footway is a walk link only.
    let mut osm = drawing();
    osm.ways.retain(|w| w.id <= 13);
    osm.nodes.extend([OsmNode::new(8, 4.357, 52.001), OsmNode::new(9, 4.356, 52.001)]);
    osm.ways.push(OsmWay::new(50, &[8, 9], &[("highway", "residential")]));
    osm.ways.push(OsmWay::new(60, &[4, 8], &[("highway", "footway")]));
    let (net, report) = read(&osm).unwrap();
    assert_eq!(report.links_unreachable, 4, "the pair's 2 car and 2 bike links");
    assert_eq!(modes(&net, 50, "f"), "w");
    assert_eq!(modes(&net, 60, "f"), "w");
}
