//! The trips drawn from a network's streets (experimental), by hand and by property.
//!
//! The extract: way 100, a two-way residential street through nodes 1, 2, 3 (4.361°E to
//! 4.363°E on 52.0°N), and way 101, a two-way tertiary road on through nodes 3, 4, 5 (to
//! 4.365°E). By hand: homes are counted on the residential street alone and activities on the
//! tertiary road alone, so every trip starts on the first (longitude 4.361 to 4.363) and ends
//! on the second (4.363 to 4.365), on the parallel. No region cuts a motorway, so no trip
//! drives through. Properties: the ids count up; a departure lies in the hour after the start;
//! the classes come by their shares (within four standard deviations of a binomial draw);
//! without a timetable no trip may ride transit; the same seed gives the same trips, another
//! seed others. Bikes: the classes `car_and_bike` and `bike_only`, 60 % of the travellers, may
//! ride; the others may not.
//!
//! The pull of distance, by hand (`the_activity_cell_is_drawn_by_length_and_distance`): a
//! residential street, then a tertiary road T1 of two steps about 2 km east and a tertiary road
//! T2 of four steps about 5.5 km east, joined by footways (neither homes nor activities). The
//! share of the trips ending on T1 is L1·exp(−d1/4000) / (L1·exp(−d1/4000) + L2·exp(−d2/4000)),
//! L the roads' lengths and d the distance between the centres of the 500 m cells holding the
//! middles of the home street and of the road, computed in the test from the nodes' positions.
//!
//! Through traffic, by hand (`a_through_trip_enters_at_a_cut_end_and_leaves_at_the_other`): a
//! box with a street, and two motorways leaving it on two carriageways each, southwards (2
//! lanes) and northwards (4 lanes), cut at the buffer's edge and closed by a turnaround each
//! (`tests/region.rs`). A car enters where a turnaround ends (the start of the inbound
//! carriageway: node 13 south, node 33 north) and leaves where the other's begins (the end of
//! the outbound one: node 23 north, node 3 south), the near exit being 23 m away, under
//! GATES_APART_M. Entries by the capacity out of them: one third south, two thirds north (2
//! lanes to 4 of one class). 5 % of the trips drive through.

mod common;

use common::{at, read};
use openmobisim_graph::geometry::Position;
use openmobisim_graph::geometry::Xy;
use openmobisim_graph::network::Network;
use openmobisim_io_osm::trips::{
    CELL_M, CLASSES, DECAY_M, DEPARTURES_S, THROUGH, THROUGH_SHARE, draw,
};
use openmobisim_io_osm::{Osm, OsmNode, OsmWay, Region, network};
use openmobisim_types::ids::{EntityId, NodeId};
use openmobisim_types::mode::Mode;
use openmobisim_types::time::Second;

fn extract() -> Osm {
    let nodes: Vec<OsmNode> = (1..=5).map(|i| at(i, i)).collect();
    let ways = vec![
        OsmWay::new(100, &[1, 2, 3], &[("highway", "residential")]),
        OsmWay::new(101, &[3, 4, 5], &[("highway", "tertiary")]),
    ];
    Osm { nodes, ways }
}

fn lon_lat(p: Position) -> (f64, f64) {
    match p {
        Position::LonLat(at) => (at.lon, at.lat),
        Position::Metres(_) => panic!("a network read from an extract places trips in degrees"),
    }
}

#[test]
fn trips_start_on_home_streets_and_end_on_activity_streets() {
    let (net, _) = read(&extract()).unwrap();
    let trips = draw(&net, 2_000, Second(27_000), false, 7);
    assert_eq!(trips.len(), 2_000);
    for (k, t) in trips.iter().enumerate() {
        assert_eq!(t.id, format!("t{k:07}"));
        let ((ox, oy), (dx, dy)) = (lon_lat(t.origin), lon_lat(t.destination));
        assert!((4.361 - 1e-7..=4.363 + 1e-7).contains(&ox), "{} starts at {ox}", t.id);
        assert!((4.363 - 1e-7..=4.365 + 1e-7).contains(&dx), "{} ends at {dx}", t.id);
        assert!((oy - 52.0).abs() < 1e-6 && (dy - 52.0).abs() < 1e-6);
        assert!((27_000..27_000 + DEPARTURES_S).contains(&t.departure.get()));
        let (_, _, open) = CLASSES.iter().find(|c| c.0 == t.class).expect("one of the classes");
        assert!(!t.modes.contains(Mode::Transit), "no timetable, no transit");
        for m in [Mode::Car, Mode::Bike, Mode::Walk] {
            assert_eq!(t.modes.contains(m), open.contains(&m), "{} of class {}", t.id, t.class);
        }
    }
    for (class, share, _) in CLASSES {
        #[allow(clippy::cast_precision_loss, reason = "small test counts")]
        let found = trips.iter().filter(|t| t.class == class).count() as f64 / 2_000.0;
        let deviation = (share * (1.0 - share) / 2_000.0).sqrt();
        assert!((found - share).abs() < 4.0 * deviation, "{class}: {found} for {share}");
    }
}

#[test]
fn the_same_seed_gives_the_same_trips_and_transit_is_open_with_a_timetable() {
    let (net, _) = read(&extract()).unwrap();
    let first = draw(&net, 300, Second(0), true, 1);
    assert_eq!(first, draw(&net, 300, Second(0), true, 1));
    assert_ne!(first, draw(&net, 300, Second(0), true, 2));
    assert!(first.iter().all(|t| t.modes.contains(Mode::Transit)));
    // A longer draw starts with the shorter one: a trip's draws are its own.
    assert_eq!(first[..], draw(&net, 400, Second(0), true, 1)[..300]);
}

#[test]
fn a_network_without_a_street_of_the_classes_gives_no_trips() {
    // A cycleway alone: neither a home street nor an activity street.
    let nodes: Vec<OsmNode> = (1..=3).map(|i| at(i, i)).collect();
    let ways = vec![OsmWay::new(100, &[1, 2, 3], &[("highway", "cycleway")])];
    let (net, _) = read(&Osm { nodes, ways }).unwrap();
    assert!(draw(&net, 10, Second(0), false, 0).is_empty());
}

/// Where the node of OSM id `id` lies on the network's plane.
fn place(net: &Network, id: i64) -> Xy {
    let node = NodeId::iter_space(net.node_count())
        .find(|n| net.node_ids().external(n.raw()) == id.to_string())
        .expect("the node is in the network");
    net.node_position(node)
}

/// The centre of the cell holding the middle of the street from node `a` to node `b`, and the
/// street's straight length.
fn cell_and_length(net: &Network, a: i64, b: i64) -> (Xy, f64) {
    let (p, q) = (place(net, a), place(net, b));
    let centre = |v: f64| ((v / CELL_M).floor() + 0.5) * CELL_M;
    (Xy::new(centre((p.x + q.x) / 2.0), centre((p.y + q.y) / 2.0)), (q.x - p.x).hypot(q.y - p.y))
}

#[test]
fn the_activity_cell_is_drawn_by_length_and_distance() {
    let ids = [0, 2, 30, 32, 80, 84];
    let nodes: Vec<OsmNode> = ids.iter().map(|&i| at(i + 1, i)).collect();
    let way = |id, a: i64, b: i64, class| OsmWay::new(id, &[a + 1, b + 1], &[("highway", class)]);
    let ways = vec![
        way(100, 0, 2, "residential"),
        way(101, 2, 30, "footway"),
        way(102, 30, 32, "tertiary"),
        way(103, 32, 80, "footway"),
        way(104, 80, 84, "tertiary"),
    ];
    let (net, _) = read(&Osm { nodes, ways }).unwrap();
    let (home, _) = cell_and_length(&net, 1, 3);
    let pull = |a, b| {
        let (cell, length) = cell_and_length(&net, a, b);
        length * (-(cell.x - home.x).hypot(cell.y - home.y) / DECAY_M).exp()
    };
    let (near, far) = (pull(31, 33), pull(81, 85));
    let share = near / (near + far);
    assert!((0.4..0.6).contains(&share), "the case tells the two roads apart: {share}");
    let trips = draw(&net, 4_000, Second(0), false, 3);
    let middle = (4.36 + 0.032 + 4.36 + 0.080) / 2.0;
    #[allow(clippy::cast_precision_loss, reason = "small test counts")]
    let found = trips.iter().filter(|t| lon_lat(t.destination).0 < middle).count() as f64 / 4_000.0;
    let deviation = (share * (1.0 - share) / 4_000.0).sqrt();
    assert!((found - share).abs() < 4.0 * deviation, "{found} end on the near road for {share}");
}

#[test]
fn a_through_trip_enters_at_a_cut_end_and_leaves_at_the_other() {
    // Steps of 0.001 degree from (4.80, 45.70), as tests/region.rs: the box over 0..2 x 0..2.
    let at = |id: i64, x: f64, y: f64| OsmNode::new(id, 4.80 + x * 0.001, 45.70 + y * 0.001);
    let mut nodes = Vec::new();
    for (k, (south, north)) in (0i64..).zip([(1.0, 1.0), (-5.0, 7.0), (-15.0, 17.0), (-30.0, 32.0)])
    {
        nodes.extend([at(1 + k, 1.0, south), at(11 + k, 1.3, south)]);
        nodes.extend([at(21 + k, 1.6, north), at(31 + k, 1.9, north)]);
    }
    let motorway = |id, nodes: &[i64], lanes| {
        OsmWay::new(id, nodes, &[("highway", "motorway"), ("ref", "A"), ("lanes", lanes)])
    };
    let street = |id, a, b| OsmWay::new(id, &[a, b], &[("highway", "residential")]);
    let ways = vec![
        motorway(100, &[1, 2, 3, 4], "2"),
        motorway(101, &[14, 13, 12, 11], "2"),
        motorway(200, &[21, 22, 23, 24], "4"),
        motorway(201, &[34, 33, 32, 31], "4"),
        street(102, 11, 1),
        street(202, 31, 21),
        street(103, 1, 21),
    ];
    let osm = Osm { nodes, ways };
    let region = Region::bbox(4.80, 45.70, 4.802, 45.702).unwrap();
    let (net, _) = network(&osm, common::VALUES, Some(&region)).unwrap();
    let trips = draw(&net, 20_000, Second(0), false, 5);
    let through: Vec<_> = trips.iter().filter(|t| t.class == THROUGH).collect();
    let node = |id: i64| osm.nodes.iter().find(|n| n.id == id).map(|n| (n.lon, n.lat)).unwrap();
    let is = |p: Position, id: i64| {
        let ((x, y), (lon, lat)) = (lon_lat(p), node(id));
        (x - lon).abs() < 1e-7 && (y - lat).abs() < 1e-7
    };
    let mut from_south = 0.0;
    for t in &through {
        assert!(t.modes.contains(Mode::Car) && !t.modes.contains(Mode::Walk));
        if is(t.origin, 13) {
            assert!(is(t.destination, 23), "{}: in at the south, out at the north", t.id);
            from_south += 1.0;
        } else {
            assert!(is(t.origin, 33) && is(t.destination, 3), "{}: in at the north", t.id);
        }
    }
    #[allow(clippy::cast_precision_loss, reason = "small test counts")]
    let (n, all) = (through.len() as f64, trips.len() as f64);
    assert!(
        (n / all - THROUGH_SHARE).abs()
            < 4.0 * (THROUGH_SHARE * (1.0 - THROUGH_SHARE) / all).sqrt()
    );
    let third = 1.0 / 3.0;
    assert!(
        (from_south / n - third).abs() < 4.0 * (third * (1.0 - third) / n).sqrt(),
        "{}",
        from_south / n
    );
}

#[test]
fn a_network_with_activity_streets_alone_draws_homes_on_them() {
    // A tertiary road alone, nodes 1 to 3: no home street, so its length serves for both ends.
    let nodes: Vec<OsmNode> = (1..=3).map(|i| at(i, i)).collect();
    let ways = vec![OsmWay::new(100, &[1, 2, 3], &[("highway", "tertiary")])];
    let (net, _) = read(&Osm { nodes, ways }).unwrap();
    let trips = draw(&net, 50, Second(0), false, 0);
    assert_eq!(trips.len(), 50);
    for t in &trips {
        for end in [t.origin, t.destination] {
            assert!((4.361 - 1e-7..=4.363 + 1e-7).contains(&lon_lat(end).0));
        }
    }
}
