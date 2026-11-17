//! Each link's drawing: its shape is the positions of its way's nodes between its two ends, in
//! its own direction (the reverse link's reversed), and its road class its way's class (version
//! 1's names: `road` is `unclassified`, `path` a `footway`). Neither changes the length (the sum
//! of the geodesics, pyproj's value) or the fingerprint. For bikes: a cycleway keeps its shape
//! and its class as a street does.

mod common;

use common::{at, close, direction, read, street};
use openmobisim_graph::geometry::LonLat;
use openmobisim_graph::network::Network;
use openmobisim_io_osm::{Osm, OsmNode, OsmWay};
use openmobisim_types::ids::{EntityId, LinkId};
use openmobisim_types::mode::Mode;

/// The zigzag of `tests/geometry.rs` (way 100 through nodes 1 to 5, residential; pyproj's length
/// 542.861 119 846 913 m), way 200 back from 5 to 1 through node 9 tagged `highway`=`class`.
fn zigzag(class: &str) -> (Osm, [(f64, f64); 5]) {
    let points =
        [(4.800, 45.700), (4.801, 45.701), (4.802, 45.700), (4.803, 45.701), (4.804, 45.700)];
    let mut nodes: Vec<OsmNode> =
        (1..=5).zip(points).map(|(id, (lon, lat))| OsmNode::new(id, lon, lat)).collect();
    nodes.push(OsmNode::new(9, 4.802, 45.702));
    let ways = vec![
        OsmWay::new(100, &[1, 2, 3, 4, 5], &[("highway", "residential")]),
        OsmWay::new(200, &[5, 9, 1], &[("highway", class)]),
    ];
    (Osm { nodes, ways }, points)
}

fn only(net: &Network, mode: Mode, way: i64, tag: &str) -> LinkId {
    let d = direction(net, mode, way, tag).unwrap();
    assert_eq!(d.links.len(), 1);
    d.links[0]
}

#[test]
fn a_bent_way_gives_its_bends_in_each_direction_and_the_length_as_before() {
    let (osm, points) = zigzag("residential");
    let (net, _) = read(&osm).unwrap();
    let projection = net.projection().unwrap();
    let inner: Vec<_> =
        points[1..4].iter().map(|&(lon, lat)| projection.project(LonLat { lon, lat })).collect();
    let (f, b) = (only(&net, Mode::Car, 100, "f"), only(&net, Mode::Car, 100, "b"));
    assert_eq!(net.shape(f), &inner[..], "along the way: nodes 2, 3, 4");
    let back: Vec<_> = inner.iter().rev().copied().collect();
    assert_eq!(net.shape(b), &back[..], "against it: nodes 4, 3, 2");
    for l in [f, b] {
        assert!(close(net.length(l).get(), 542.861_119_846_913, 1e-6), "{} m", net.length(l).get());
    }
    let one_bend = only(&net, Mode::Car, 200, "f");
    assert_eq!(net.shape(one_bend), &[projection.project(LonLat { lon: 4.802, lat: 45.702 })]);
}

#[test]
fn a_link_is_classed_by_its_ways_row_and_the_class_changes_no_fingerprint() {
    let class = |highway: &str| {
        let (net, _) = read(&zigzag(highway).0).unwrap();
        let l = only(&net, Mode::Walk, 200, "f");
        (net.road_class(l).map(str::to_owned), net.fingerprint())
    };
    assert_eq!(class("residential").0.as_deref(), Some("residential"));
    assert_eq!(class("path").0.as_deref(), Some("footway"), "version 1's class of a path");
    // `road` is version 1's `unclassified`.
    let (road, road_print) = class("road");
    assert_eq!(road.as_deref(), Some("unclassified"));
    let (unclassified, unclassified_print) = class("unclassified");
    assert_eq!(unclassified.as_deref(), Some("unclassified"));
    assert_eq!(road_print, unclassified_print, "the same rows but for the class: one fingerprint");
    assert_eq!(class("cycleway").0.as_deref(), Some("cycleway"));
    let (net, _) = read(&zigzag("residential").0).unwrap();
    assert_eq!(net.road_class(only(&net, Mode::Car, 100, "b")), Some("residential"));
}

/// Way 100 through nodes 1 to 7 in a row, crossed at node 4 by way 500: two pieces, 1-4 and
/// 4-7. Each piece's shape is its own inner nodes (2, 3 and 5, 6), in its own direction:
/// reversed on the `b` links.
#[test]
fn a_way_of_two_pieces_gives_each_piece_its_own_bends_in_each_direction() {
    let mut nodes: Vec<OsmNode> = (1..=7).map(|i| at(i, i)).collect();
    nodes.push(OsmNode::new(998, 4.364, 52.001));
    let r = [("highway", "residential")];
    let ways = vec![OsmWay::new(100, &[1, 2, 3, 4, 5, 6, 7], &r), OsmWay::new(500, &[4, 998], &r)];
    let (net, _) = read(&Osm { nodes, ways }).unwrap();
    let projection = net.projection().unwrap();
    let node = |i: i64| projection.project(LonLat { lon: at(i, i).lon, lat: at(i, i).lat });
    let f = direction(&net, Mode::Car, 100, "f").unwrap().links;
    let b = direction(&net, Mode::Car, 100, "b").unwrap().links;
    assert_eq!((f.len(), b.len()), (2, 2));
    let shape = |l| net.shape(l).to_vec();
    let id = |l: LinkId| net.link_ids().external(l.raw()).to_owned();
    let piece = |links: &[LinkId], name: &str| *links.iter().find(|&&l| id(l) == name).unwrap();
    assert_eq!(shape(piece(&f, "car:100:0:f")), [node(2), node(3)]);
    assert_eq!(shape(piece(&f, "car:100:1:f")), [node(5), node(6)]);
    assert_eq!(shape(piece(&b, "car:100:0:b")), [node(3), node(2)]);
    assert_eq!(shape(piece(&b, "car:100:1:b")), [node(6), node(5)]);
}

/// Way 300, a residential street between nodes 8 and 9 joined to the rest by a footway alone:
/// outside the car and bike layers' strong components, it is dropped there; the walk layer keeps
/// it, with its class.
#[test]
fn a_link_kept_on_one_layer_keeps_its_class() {
    let mut osm = street(3, &[("highway", "residential")]);
    osm.nodes.extend([OsmNode::new(8, 4.37, 52.002), OsmNode::new(9, 4.371, 52.002)]);
    osm.ways.push(OsmWay::new(300, &[8, 9], &[("highway", "residential")]));
    osm.ways.push(OsmWay::new(400, &[3, 8], &[("highway", "footway")]));
    let (net, report) = read(&osm).unwrap();
    assert_eq!(report.links_unreachable, 4);
    for tag in ["f", "b"] {
        assert!(direction(&net, Mode::Car, 300, tag).is_none());
        let walk = direction(&net, Mode::Walk, 300, tag).unwrap();
        assert_eq!(net.road_class(walk.links[0]), Some("residential"));
    }
}

/// Way 100 through nodes 1 to 7, node 4 not in the extract: two runs, 1-3 and 5-7, each with its
/// own inner node (2, then 6) as its shape; nothing bridges the gap.
#[test]
fn a_way_cut_by_the_extract_edge_keeps_each_runs_bends() {
    let mut osm = street(7, &[("highway", "cycleway")]);
    osm.nodes.retain(|n| n.id != 4);
    let (net, report) = read(&osm).unwrap();
    assert_eq!(report.ways_cut, 1);
    let projection = net.projection().unwrap();
    let node = |i: i64| projection.project(LonLat { lon: at(i, i).lon, lat: at(i, i).lat });
    let f = direction(&net, Mode::Bike, 100, "f").unwrap().links;
    let shapes: Vec<Vec<_>> = f.iter().map(|&l| net.shape(l).to_vec()).collect();
    assert_eq!(shapes, [vec![node(2)], vec![node(6)]]);
    for &l in &f {
        assert_eq!(net.road_class(l), Some("cycleway"));
    }
}
