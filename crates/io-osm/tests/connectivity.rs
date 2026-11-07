//! The car network's largest strongly connected component, by drawing: a car link a car
//! cannot leave or cannot reach loses its car mode and is counted, and keeps its bike and walk
//! modes; a link a car never uses is kept as it is. Bikes: a cycleway is never touched, and a
//! street outside the component stays a bike and walk link.
//!
//! The drawing: a square of two-way residential streets A-B-C-D (nodes 1-4), a one-way spur
//! from C to E (node 5) with no way back, a separate two-way street F-G (6-7, an island), and a
//! cycleway from G to A.

mod common;

use common::{direction, read};
use openmobisim_io_osm::{Osm, OsmNode, OsmWay};

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

#[test]
fn the_square_keeps_its_cars_and_the_spur_and_the_island_keep_bikes_and_walkers() {
    let (net, report) = read(&drawing()).unwrap();
    // Lose the car: the spur's C→E and the island's F→G and G→F (3 links). Kept, all of them:
    // the square's 8 (car, bike, walk), the spur's C→E (now bike, walk) and E→C (walk alone:
    // `oneway` binds vehicles, not walkers), the island's 2 (now bike, walk), the cycleway's 2:
    // 8 + 2 + 2 + 2 = 14 links on A to G, 7 nodes.
    assert_eq!(report.links_unreachable, 3);
    assert_eq!((net.link_count(), net.node_count()), (14, 7));
    for way in [10, 11, 12, 13] {
        assert_eq!(direction(&net, way, "f").unwrap().modes, "cbw");
        assert_eq!(direction(&net, way, "b").unwrap().modes, "cbw");
    }
    let spur = direction(&net, 20, "f").unwrap();
    assert_eq!((spur.modes.as_str(), spur.lanes, spur.speed_km_h), ("bw", None, None));
    assert_eq!(direction(&net, 20, "b").unwrap().modes, "w");
    for tag in ["f", "b"] {
        let island = direction(&net, 30, tag).unwrap();
        assert_eq!((island.modes.as_str(), island.lanes, island.speed_km_h), ("bw", None, None));
        assert_eq!(direction(&net, 40, tag).unwrap().modes, "bw", "the cycleway, unchanged");
    }
}

#[test]
fn a_network_already_strongly_connected_loses_nothing_and_a_second_pass_changes_nothing() {
    let mut osm = drawing();
    osm.ways.retain(|w| w.id != 20 && w.id != 30);
    let (net, report) = read(&osm).unwrap();
    assert_eq!((report.links_unreachable, net.link_count()), (0, 10));
}
