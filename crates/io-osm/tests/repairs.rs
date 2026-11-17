//! One extract with one way of each repaired fault, read whole, with the report's counts by
//! hand. Nodes 1 to 8 on a line (about 69 m apart, node 8 at node 7's position), a two-way
//! residential way between each two (ways 101 to 107), and a residential street back from 8 to
//! 1 through node 999 (way 900), so cars can always return.
//!
//! - way 101 `access=private`: closed to every mode; given twice: `ways_repeated` 1.
//! - way 102 `maxspeed=none`: the class's 30 km/h, `speeds_defaulted` 1.
//! - way 103 `lanes=2;4`: read as absent, `tags_unread` 1.
//! - way 104 `lanes=4`, `lanes:forward=2`, `lanes:backward=1`: the sides as tagged,
//!   `lanes_repaired` 1.
//! - way 105 `oneway=no;yes`: read as absent (both ways), `tags_unread` 1 more.
//! - way 107 from node 7 to node 8 at one position: both its links are 0 m on each of the three
//!   layers, left out, `links_of_no_length` 6.
//! - way 108 from node 8 to node 50 at latitude 89°: node 50 is not placeable, the way is cut
//!   there (`ways_cut` 1) and holds no link.
//!
//! For bikes: ways 102 to 105 keep their bikes both ways; way 101 is closed to them too; way 107's
//! bike links are left out as its car links are.

mod common;

use common::{at, direction, modes, read};
use openmobisim_io_osm::{Osm, OsmNode, OsmWay};
use openmobisim_types::ids::EntityId;
use openmobisim_types::mode::Mode;

#[test]
fn one_way_of_each_fault_builds_with_the_counts_by_hand() {
    let mut nodes: Vec<OsmNode> = (1..=7).map(|i| at(i, i)).collect();
    nodes.push(at(8, 7));
    nodes.push(OsmNode::new(999, 4.36, 52.001));
    nodes.push(OsmNode::new(50, 4.36, 89.0));
    let residential = |id: i64, a: i64, b: i64, extra: &[(&str, &str)]| {
        let mut tags = vec![("highway", "residential")];
        tags.extend_from_slice(extra);
        OsmWay::new(id, &[a, b], &tags)
    };
    let ways = vec![
        residential(101, 1, 2, &[("access", "private")]),
        residential(101, 1, 2, &[]),
        residential(102, 2, 3, &[("maxspeed", "none")]),
        residential(103, 3, 4, &[("lanes", "2;4")]),
        residential(104, 4, 5, &[("lanes", "4"), ("lanes:forward", "2"), ("lanes:backward", "1")]),
        residential(105, 5, 6, &[("oneway", "no;yes")]),
        residential(106, 6, 7, &[]),
        residential(107, 7, 8, &[]),
        residential(108, 8, 50, &[]),
        OsmWay::new(900, &[8, 999, 1], &[("highway", "residential")]),
    ];
    let (net, report) = read(&Osm { nodes, ways }).expect("the extract builds");
    let counts: Vec<(&str, u64)> =
        report.repairs.kinds().iter().map(|k| (k.key, k.count)).collect();
    assert_eq!(
        counts,
        [
            ("ways_repeated", 1),
            ("speeds_defaulted", 1),
            ("tags_unread", 2),
            ("lanes_repaired", 1),
            ("links_of_no_length", 6),
        ]
    );
    assert_eq!(report.ways_cut, 1);
    assert_eq!((modes(&net, 101, "f"), modes(&net, 107, "f")), ("-".into(), "-".into()));
    // Way 102 merged with way 103 (the same street once both are read): the class's 30 km/h.
    let none = direction(&net, Mode::Car, 102, "f").unwrap();
    assert_eq!((modes(&net, 102, "f").as_str(), none.speed_km_h), ("cbw", Some(30.0)));
    let car = |way, tag| direction(&net, Mode::Car, way, tag).unwrap().lanes;
    assert_eq!((car(104, "f"), car(104, "b")), (Some(2), Some(1)));
    // `oneway=no;yes` read as absent: cars drive way 105 both ways (backwards merged with 106).
    let back = direction(&net, Mode::Car, 106, "b").unwrap().links[0];
    let id = |n: openmobisim_types::ids::NodeId| net.node_ids().external(n.raw()).to_owned();
    assert_eq!((id(net.upstream(back)), id(net.downstream(back))), ("7".into(), "5".into()));
    let first: Vec<&str> = report.repairs.kinds()[2].first.iter().map(String::as_str).collect();
    assert_eq!(first, ["way 103", "way 105"]);
}

/// The reviewer's case: a ring 1-2-3-7-1 (way 1) and an island 4-5-6 (ways 2, 4-5-6, and 3,
/// 6-4: as one closed way 4-5-6-4 the island would be one piece from 4 back to 4, a loop,
/// dropped), node 4 at node 3's position, joined only by way 100 from 3 to 4, all two-way
/// residential. Way 100's two links are 0 m on each layer: left out before the strong components
/// are taken (`links_of_no_length` 6), so the island is outside each: its 4 links per layer (2
/// pieces, both ways) are dropped (`links_unreachable` 12).
#[test]
fn a_piece_of_no_length_joins_nothing_for_the_car_network() {
    let nodes = vec![at(1, 0), at(2, 1), at(3, 2), at(7, 1), at(4, 2), at(5, 3), at(6, 4)];
    let mut nodes = nodes;
    nodes[3] = openmobisim_io_osm::OsmNode::new(7, 4.361, 51.999);
    nodes[5] = openmobisim_io_osm::OsmNode::new(5, 4.363, 52.001);
    let residential = [("highway", "residential")];
    let ways = vec![
        OsmWay::new(1, &[1, 2, 3, 7, 1], &residential),
        OsmWay::new(2, &[4, 5, 6], &residential),
        OsmWay::new(3, &[6, 4], &residential),
        OsmWay::new(100, &[3, 4], &residential),
    ];
    let (net, report) = read(&Osm { nodes, ways }).expect("the extract builds");
    assert_eq!(report.repairs.count("links_of_no_length"), 6);
    assert_eq!(report.links_unreachable, 12);
    for (way, tag) in [(2, "f"), (2, "b"), (3, "f"), (3, "b"), (100, "f")] {
        assert_eq!(modes(&net, way, tag), "-", "way {way} {tag}");
    }
    assert_eq!(modes(&net, 1, "f"), "cbw");
}

/// A segment whose geodesic does not converge (Vincenty's method between two nearly antipodal
/// points, 0°, 0° and 179.5°E, 0.5°N, a case it is known not to solve) cuts the way at its far
/// node, which is read as one the extract does not hold: way 100, nodes 1, 2, 3, keeps runs of
/// one node each, so no link (`ways_cut` 1); way 200 from node 3 to node 4 near it is read: 2
/// nodes, 2 links on each of three layers.
#[test]
fn a_segment_whose_geodesic_does_not_converge_cuts_the_way() {
    let node = openmobisim_io_osm::OsmNode::new;
    let nodes =
        vec![node(1, 0.0, 0.0), node(2, 179.5, 0.5), node(3, 179.501, 0.5), node(4, 179.502, 0.5)];
    let residential = [("highway", "residential")];
    let ways =
        vec![OsmWay::new(100, &[1, 2, 3], &residential), OsmWay::new(200, &[3, 4], &residential)];
    let (net, report) = read(&Osm { nodes, ways }).expect("the extract builds");
    assert_eq!(report.ways_cut, 1);
    assert_eq!((net.node_count(), net.link_count()), (2, 6));
    assert_eq!(modes(&net, 100, "f"), "-");
}
