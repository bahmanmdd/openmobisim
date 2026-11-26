//! The reader with `CarRoads::Main` (experimental), cars on the main roads alone, by hand: a motorway down to an unclassified road carries
//! cars; a residential street, a living street and a service road carry bikes and walkers only,
//! and a main road runs past their junctions as one car link. A roundabout tagged as one of
//! those streets is driven as the largest car road that meets it, so that it does not cut the
//! roads through it; one no car road meets is not driven. Bikes: a bike rides every street and
//! every roundabout as before, in the street's direction, and keeps the way's own class.

mod common;

use common::{VALUES, at, close, direction, modes};
use openmobisim_graph::network::Network;
use openmobisim_io_osm::geodesic::geodesic_m;
use openmobisim_io_osm::tags::minor_roundabout;
use openmobisim_io_osm::{CarRoads, Osm, OsmError, OsmNode, OsmWay, Report, network_for};
use openmobisim_types::ids::{EntityId, LinkId};
use openmobisim_types::mode::Mode;

fn read(osm: &Osm) -> Result<(Network, Report), OsmError> {
    network_for(osm, VALUES, None, CarRoads::Main)
}

fn of_mode(net: &Network, mode: Mode) -> Vec<LinkId> {
    LinkId::iter_space(net.link_count()).filter(|&l| net.modes(l).contains(mode)).collect()
}

fn tags(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
}

/// A ring of four nodes 10, 11, 12, 13 (a square of 0.001° about node 2 of the row) and the
/// row's nodes 1 to 3 west of it, node 4 east and node 5 south.
fn ring_nodes() -> Vec<OsmNode> {
    let mut nodes: Vec<OsmNode> = (1..=3).map(|i| at(i, i)).collect();
    nodes.extend([
        OsmNode::new(10, 4.370, 52.000),
        OsmNode::new(11, 4.371, 52.001),
        OsmNode::new(12, 4.372, 52.000),
        OsmNode::new(13, 4.371, 51.999),
        OsmNode::new(4, 4.380, 52.000),
        OsmNode::new(5, 4.371, 51.990),
    ]);
    nodes
}

#[test]
fn a_main_road_passes_a_side_street_as_one_car_link() {
    // Way 100, a tertiary road through nodes 1, 2, 3; way 101, a residential street from node 2
    // north to node 4. Cars: way 100 alone, node 2 no junction of theirs, so one link each way
    // (2 links). Bikes and walkers: both ways, split at node 2 (3 streets x 2 = 6 links each).
    let mut nodes: Vec<OsmNode> = (1..=3).map(|i| at(i, i)).collect();
    nodes.push(OsmNode::new(4, 4.362, 52.001));
    let ways = vec![
        OsmWay::new(100, &[1, 2, 3], &[("highway", "tertiary")]),
        OsmWay::new(101, &[2, 4], &[("highway", "residential")]),
    ];
    let (net, _) = read(&Osm { nodes, ways }).unwrap();
    assert_eq!(of_mode(&net, Mode::Car).len(), 2);
    assert_eq!((modes(&net, 100, "f"), modes(&net, 101, "f")), ("cbw".into(), "bw".into()));
    let car = direction(&net, Mode::Car, 100, "f").unwrap();
    let bike = direction(&net, Mode::Bike, 100, "f").unwrap();
    assert_eq!((car.links.len(), bike.links.len()), (1, 2));
    assert_eq!(car.length_m.to_bits(), bike.length_m.to_bits(), "the same road, the same length");
    assert_eq!((of_mode(&net, Mode::Bike).len(), of_mode(&net, Mode::Walk).len()), (6, 6));
}

#[test]
fn a_roundabout_tagged_as_a_street_is_driven_as_the_largest_road_that_meets_it() {
    // Way 500, a roundabout 10-11-12-13-10 tagged residential. A secondary road (way 501) from
    // node 3 to node 10, a tertiary road (way 502) from node 12 to node 4, another (way 503)
    // from node 13 to node 5, and a residential street (way 504) from node 11 to node 6. Cars
    // drive the ring one way as a secondary road (50 km/h, 1 lane, 900 veh/h): 10 to 12 past 11
    // (node 11 is no junction of theirs), 12 to 13, 13 to 10: 3 links, and 2 on each of the
    // three roads. Without the rule the three roads would not be joined.
    let ring = [("highway", "residential"), ("junction", "roundabout")];
    let mut nodes = ring_nodes();
    nodes.push(OsmNode::new(6, 4.371, 52.010));
    let ways = vec![
        OsmWay::new(500, &[10, 11, 12, 13, 10], &ring),
        OsmWay::new(501, &[3, 10], &[("highway", "secondary")]),
        OsmWay::new(502, &[12, 4], &[("highway", "tertiary")]),
        OsmWay::new(503, &[13, 5], &[("highway", "tertiary")]),
        OsmWay::new(504, &[11, 6], &[("highway", "residential")]),
    ];
    let (net, report) = read(&Osm { nodes, ways }).unwrap();
    assert_eq!(report.links_unreachable, 0);
    assert_eq!(of_mode(&net, Mode::Car).len(), 9);
    let on_ring = direction(&net, Mode::Car, 500, "f").unwrap();
    assert_eq!(on_ring.links.len(), 3);
    assert_eq!(
        (on_ring.lanes, on_ring.speed_km_h, on_ring.capacity_veh_h),
        (Some(1), Some(50.0), Some(900.0))
    );
    assert!(direction(&net, Mode::Car, 500, "b").is_none(), "a roundabout is one way");
    for &l in &on_ring.links {
        assert_eq!(net.road_class(l), Some("secondary"));
    }
    assert_eq!(modes(&net, 504, "f"), "bw", "the street off the ring carries no car");
    // A bike rides the ring in its direction, on links of the way's own class.
    let bike = direction(&net, Mode::Bike, 500, "f").unwrap();
    assert!(direction(&net, Mode::Bike, 500, "b").is_none());
    assert_eq!((bike.links.len(), net.road_class(bike.links[0])), (4, Some("residential")));
}

#[test]
fn a_roundabout_drawn_as_two_ways_is_raised_whole_and_one_no_car_road_meets_is_not() {
    // The ring as two ways: 600 through 10, 11, 12 and 601 through 12, 13, 10. A tertiary road
    // (way 602) from node 3 meets way 600 alone, at node 11: way 600 is raised to tertiary, and
    // way 601, which meets it, with it. Were way 601 left out, the half ring would lead nowhere
    // and be dropped. Cars then drive the road and once round the ring: as the ring has one
    // road only, the contraction leaves one link out and one back, together twice the road and
    // once the ring (by the geodesics). A second ring, way 700 (20-21-22-20), meets two
    // residential streets alone (ways 701 and 703): no car drives it, bikes and walkers do.
    let ring = [("highway", "service"), ("junction", "roundabout")];
    let mut nodes = ring_nodes();
    nodes.extend([
        OsmNode::new(20, 4.390, 52.000),
        OsmNode::new(21, 4.391, 52.001),
        OsmNode::new(22, 4.392, 52.000),
        OsmNode::new(23, 4.399, 52.000),
    ]);
    let m = |a: (f64, f64), b: (f64, f64)| geodesic_m(a, b).expect("a short geodesic");
    let (n3, n10, n11) = ((4.363, 52.0), (4.370, 52.000), (4.371, 52.001));
    let (n12, n13) = ((4.372, 52.000), (4.371, 51.999));
    let round = m(n10, n11) + m(n11, n12) + m(n12, n13) + m(n13, n10);
    let ways = vec![
        OsmWay::new(600, &[10, 11, 12], &ring),
        OsmWay::new(601, &[12, 13, 10], &ring),
        OsmWay::new(602, &[3, 11], &[("highway", "tertiary")]),
        OsmWay::new(700, &[20, 21, 22, 20], &ring),
        OsmWay::new(701, &[4, 20], &[("highway", "residential")]),
        OsmWay::new(702, &[12, 4], &[("highway", "residential")]),
        OsmWay::new(703, &[22, 23], &[("highway", "residential")]),
    ];
    let (net, _) = read(&Osm { nodes, ways }).unwrap();
    let cars = of_mode(&net, Mode::Car);
    assert_eq!(cars.len(), 2);
    let driven: f64 = cars.iter().map(|&l| net.length(l).0).sum();
    assert!(close(driven, 2.0 * m(n3, n11) + round, 1e-6), "{driven} m");
    for &l in &cars {
        assert_eq!(net.road_class(l), Some("tertiary"));
    }
    assert_eq!((modes(&net, 700, "f"), modes(&net, 703, "f")), ("bw".into(), "bw".into()));
}

#[test]
fn a_ring_of_two_ways_takes_one_class_whichever_way_meets_the_larger_road() {
    // The ring as two ways, 600 through 10, 11, 12 and 601 through 12, 13, 10. A tertiary road
    // meets way 600 at node 11, a primary road way 601 at node 13, a secondary road the ring at
    // node 10: the whole ring is a primary road's (60 km/h, 2 lanes, 2 160 veh/h), not half
    // tertiary and half primary; and the same with the two ways' ids exchanged.
    for (first, second) in [(600, 601), (601, 600)] {
        let ring = [("highway", "residential"), ("junction", "roundabout")];
        let mut nodes = ring_nodes();
        nodes.push(OsmNode::new(6, 4.371, 52.010));
        let ways = vec![
            OsmWay::new(first, &[10, 11, 12], &ring),
            OsmWay::new(second, &[12, 13, 10], &ring),
            OsmWay::new(602, &[11, 6], &[("highway", "tertiary")]),
            OsmWay::new(603, &[13, 5], &[("highway", "primary")]),
            OsmWay::new(604, &[3, 10], &[("highway", "secondary")]),
        ];
        let (net, _) = read(&Osm { nodes, ways }).unwrap();
        for way in [600, 601] {
            let d = direction(&net, Mode::Car, way, "f").unwrap_or_else(|| panic!("way {way}"));
            assert_eq!(
                (d.lanes, d.speed_km_h, d.capacity_veh_h),
                (Some(2), Some(60.0), Some(2160.0)),
                "way {way}, the ring's first way {first}"
            );
            assert_eq!(net.road_class(d.links[0]), Some("primary"));
        }
    }
}

#[test]
fn only_an_open_roundabout_of_a_street_class_is_raised() {
    let is = |pairs: &[(&str, &str)]| minor_roundabout(&tags(pairs), 4);
    for highway in ["residential", "living_street", "service"] {
        assert!(is(&[("highway", highway), ("junction", "roundabout")]), "{highway}");
        assert!(is(&[("highway", highway), ("junction", "circular")]), "{highway}");
        assert!(!is(&[("highway", highway)]), "{highway}: no roundabout");
    }
    // A roundabout of a car class needs no raising; one closed to cars is not raised.
    assert!(!is(&[("highway", "tertiary"), ("junction", "roundabout")]));
    assert!(!is(&[("highway", "footway"), ("junction", "roundabout")]));
    let ring = [("highway", "residential"), ("junction", "roundabout")];
    for closed in [("access", "private"), ("motor_vehicle", "no"), ("vehicle", "private")] {
        assert!(!is(&[ring[0], ring[1], closed]), "{closed:?}");
    }
    let aisle = [("highway", "service"), ("service", "parking_aisle"), ("junction", "roundabout")];
    assert!(!is(&aisle));
}
