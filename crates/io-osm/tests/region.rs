//! A study area cut out of an extract (version 1's region tests, through the reader's one
//! behaviour: stubs, a 2 km buffer for motorways, trunk roads and their slip roads, and
//! turnarounds): a box and a polygon by hand, the distance to them, the nodes kept, a road that
//! leaves and comes back, the buffer, the turnaround, and a one-way avenue cut short. Bikes: a
//! cut means the same on the bike layer; the avenue cut short stays for walkers alone.

mod common;

use common::{VALUES, direction};
use openmobisim_graph::network::Network;
use openmobisim_io_osm::geodesic::geodesic_m;
use openmobisim_io_osm::region::{MAIN_ROAD_BUFFER_M, TURNAROUND_FIRST_ID};
use openmobisim_io_osm::{Osm, OsmNode, OsmWay, Region, network};
use openmobisim_types::ids::{EntityId, LinkId, NodeId};
use openmobisim_types::mode::Mode;

/// A thousandth of a degree: about 111 m north, 78 m east at 45.7°N.
const STEP: f64 = 0.001;

/// A node `x` steps east and `y` steps north of (4.80°E, 45.70°N).
fn at(id: i64, x: f64, y: f64) -> OsmNode {
    OsmNode::new(id, 4.80 + x * STEP, 45.70 + y * STEP)
}

/// Nodes 1..=n on the parallel 45.70°N, 0, 1, …, n − 1 steps east.
fn line(n: i64) -> Vec<OsmNode> {
    let mut x = 0.0;
    (1..=n)
        .map(|id| {
            x += 1.0;
            at(id, x - 1.0, 0.0)
        })
        .collect()
}

fn road(id: i64, nodes: &[i64]) -> OsmWay {
    OsmWay::new(id, nodes, &[("highway", "residential")])
}

fn position(osm: &Osm, id: i64) -> (f64, f64) {
    let n = osm.nodes.iter().find(|n| n.id == id).expect("a node");
    (n.lon, n.lat)
}

/// The sum of the geodesics along `ids`.
fn along(osm: &Osm, ids: &[i64]) -> f64 {
    ids.windows(2).map(|p| geodesic_m(position(osm, p[0]), position(osm, p[1])).unwrap()).sum()
}

/// The links of `mode`, as (upstream, downstream) OSM node ids and length.
fn links(net: &Network, mode: Mode) -> Vec<(String, String, f64)> {
    let id = |n: NodeId| net.node_ids().external(n.raw()).to_owned();
    let mut out: Vec<_> = LinkId::iter_space(net.link_count())
        .filter(|&l| net.modes(l).contains(mode))
        .map(|l| (id(net.upstream(l)), id(net.downstream(l)), net.length(l).0))
        .collect();
    out.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    out
}

#[test]
fn a_box_contains_its_edges_and_a_polygon_can_be_concave_and_either_winding() {
    let r = Region::bbox(1.0, 2.0, 3.0, 4.0).unwrap();
    assert!(r.contains(1.0, 2.0) && r.contains(3.0, 4.0) && r.contains(2.0, 3.0));
    assert!(!r.contains(0.999, 3.0) && !r.contains(2.0, 4.001));
    // An L: the square [0,3]² without [1,3]×[1,3].
    let l = vec![(0.0, 0.0), (3.0, 0.0), (3.0, 1.0), (1.0, 1.0), (1.0, 3.0), (0.0, 3.0)];
    let mut reversed = l.clone();
    reversed.reverse();
    for vertices in [l, reversed] {
        let r = Region::polygon(vertices).unwrap();
        assert!(r.contains(0.5, 0.5) && r.contains(2.5, 0.5) && r.contains(0.5, 2.5));
        assert!(!r.contains(2.0, 2.0), "the notch is outside");
        assert!(!r.contains(-0.5, 0.5) && !r.contains(3.5, 0.5));
    }
    let closed = Region::polygon(vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 0.0)]).unwrap();
    assert!(closed.contains(0.9, 0.2), "a repeated closing vertex is dropped");
}

#[test]
fn a_region_that_is_no_area_is_refused_with_a_reason() {
    assert!(Region::bbox(3.0, 2.0, 1.0, 4.0).is_err(), "west must be west of east");
    assert!(Region::bbox(1.0, 2.0, f64::NAN, 4.0).is_err());
    assert!(Region::bbox(1.0, 2.0, 200.0, 4.0).is_err());
    assert!(Region::polygon(vec![(0.0, 0.0), (1.0, 1.0)]).is_err());
    assert!(Region::polygon(vec![(0.0, 0.0), (1.0, 1.0), (f64::INFINITY, 0.0)]).is_err());
}

#[test]
fn a_way_the_region_does_not_keep_has_no_repair_counted() {
    // Way 100 through nodes 1..=4 (0..3 steps east), inside a box over steps -0.5..3.5, and way
    // 200 through nodes 10, 11 (100 and 101 steps east), outside it with no node inside: no stub.
    // Each tags `maxspeed=fast`, not a speed. Cut to the box, way 100 alone is read: 1 repair,
    // way 100's; the whole extract reads both: 2.
    let mut nodes = line(4);
    nodes.extend([at(10, 100.0, 0.0), at(11, 101.0, 0.0)]);
    let fast = [("highway", "residential"), ("maxspeed", "fast")];
    let ways = vec![OsmWay::new(100, &[1, 2, 3, 4], &fast), OsmWay::new(200, &[10, 11], &fast)];
    let osm = Osm { nodes, ways };
    let region = Region::bbox(4.80 - 0.5 * STEP, 45.69, 4.80 + 3.5 * STEP, 45.71).unwrap();
    let speeds = |region: Option<&Region>| {
        let (_, report) = network(&osm, VALUES, region).unwrap();
        let k = report.repairs.kinds().iter().find(|k| k.key == "speeds_defaulted").cloned();
        k.map(|k| (k.count, k.first))
    };
    assert_eq!(speeds(Some(&region)), Some((1, vec!["way 100".to_owned()])));
    assert_eq!(speeds(None).map(|k| k.0), Some(2));
}

#[test]
fn a_region_measures_how_far_a_point_is_from_it() {
    let r = Region::bbox(4.80, 45.70, 4.81, 45.71).unwrap();
    assert!(r.distance_m(4.805, 45.705).abs() < f64::EPSILON, "inside");
    // A thousandth of a degree north: 111.32 m on the mean meridian degree.
    let north = r.distance_m(4.805, 45.711);
    assert!((north - 111.32).abs() < 0.5, "{north}");
    let east = r.distance_m(4.811, 45.705);
    assert!((east - 111.32 * 45.705_f64.to_radians().cos()).abs() < 0.5, "{east}");
    let square =
        Region::polygon(vec![(4.80, 45.70), (4.81, 45.70), (4.81, 45.71), (4.80, 45.71)]).unwrap();
    assert!((square.distance_m(4.805, 45.711) - north).abs() < 1e-6, "the same as a polygon");
}

#[test]
fn a_clip_keeps_the_nodes_inside_and_a_stub_each_side_and_changes_nothing_it_keeps() {
    // A street of nodes 1..=6, 0..5 steps east. A box over steps 1.5..4.5 holds 3, 4, 5; the
    // first node outside at either end, 2 and 6, stays as a stub: the street from 2 to 6.
    let osm = Osm { nodes: line(6), ways: vec![road(100, &[1, 2, 3, 4, 5, 6])] };
    let region = Region::bbox(4.80 + 1.5 * STEP, 45.69, 4.80 + 4.5 * STEP, 45.71).unwrap();
    let (net, report) = network(&osm, VALUES, Some(&region)).unwrap();
    assert_eq!(report.ways_cut, 1);
    let length = along(&osm, &[2, 3, 4, 5, 6]);
    for mode in [Mode::Car, Mode::Bike, Mode::Walk] {
        let got = links(&net, mode);
        assert_eq!(got.len(), 2, "{mode}");
        assert_eq!((got[0].0.as_str(), got[0].1.as_str()), ("2", "6"));
        assert!((got[0].2 - length).abs() < 1e-9, "{mode}: {} m, by hand {length}", got[0].2);
    }
    // Read without a region, the same street from 1 to 6.
    let (whole, _) = network(&osm, VALUES, None).unwrap();
    let got = links(&whole, Mode::Car);
    assert_eq!((got[0].0.as_str(), got[0].1.as_str()), ("1", "6"));
}

#[test]
fn a_road_that_leaves_and_comes_back_is_two_roads_not_one_with_a_bridge() {
    // A road through nodes 1..=7, 0..6 steps east, and a region with a notch around nodes 3, 4
    // and 5: inside 1, 2 and 6, 7; stubs 3 and 5; node 4 is not kept, so nothing spans 3-5. A
    // tertiary road below the notch (1, 8, 9, 7) joins the two pieces, so both are in the strong
    // component.
    let mut nodes = line(7);
    nodes.extend([at(8, 0.0, -0.6), at(9, 6.0, -0.6)]);
    let below = OsmWay::new(200, &[1, 8, 9, 7], &[("highway", "tertiary")]);
    let osm = Osm { nodes, ways: vec![road(100, &[1, 2, 3, 4, 5, 6, 7]), below] };
    let x = |k: f64| 4.80 + k * STEP;
    let (lo, hi, notch) = (45.699, 45.701, 45.70 - 0.0002);
    let region = Region::polygon(vec![
        (x(-0.5), lo),
        (x(6.5), lo),
        (x(6.5), hi),
        (x(4.5), hi),
        (x(4.5), notch),
        (x(1.5), notch),
        (x(1.5), hi),
        (x(-0.5), hi),
    ])
    .unwrap();
    let (net, report) = network(&osm, VALUES, Some(&region)).unwrap();
    assert_eq!(report.ways_cut, 1);
    let id = |n: NodeId| net.node_ids().external(n.raw()).to_owned();
    for (tag, want) in [("f", [("1", "3"), ("5", "7")]), ("b", [("3", "1"), ("7", "5")])] {
        let d = direction(&net, Mode::Car, 100, tag).unwrap();
        let mut got: Vec<(String, String)> =
            d.links.iter().map(|&l| (id(net.upstream(l)), id(net.downstream(l)))).collect();
        got.sort();
        let want: Vec<(String, String)> = want.iter().map(|&(a, b)| (a.into(), b.into())).collect();
        assert_eq!(got, want, "two roads, each with its stub; no link crosses the gap");
        assert!((d.length_m - along(&osm, &[1, 2, 3]) - along(&osm, &[5, 6, 7])).abs() < 1e-9);
    }
}

#[test]
fn a_buffer_keeps_main_roads_near_the_region_and_nothing_else() {
    // A box over steps 0..2, 0..2; a street 1-2 inside it. From node 2 two roads go north: a
    // two-way motorway through 20 (3 steps, about 110 m outside), 21 (10 steps, 890 m) and 22 (30
    // steps, 3.1 km); a residential street through 40 (3 steps) and 41 (4 steps). The buffer
    // (2 km) keeps the motorway to 21; the stub keeps the street to 40.
    assert!((MAIN_ROAD_BUFFER_M - 2_000.0).abs() < f64::EPSILON);
    let osm = Osm {
        nodes: vec![
            at(1, 0.5, 0.5),
            at(2, 1.5, 0.5),
            at(20, 1.5, 3.0),
            at(21, 1.5, 10.0),
            at(22, 1.5, 30.0),
            at(40, 1.0, 3.0),
            at(41, 1.0, 4.0),
        ],
        ways: vec![
            road(10, &[1, 2]),
            OsmWay::new(30, &[2, 20, 21, 22], &[("highway", "motorway"), ("oneway", "no")]),
            road(50, &[2, 40, 41]),
        ],
    };
    let region = Region::bbox(4.80, 45.70, 4.80 + 2.0 * STEP, 45.70 + 2.0 * STEP).unwrap();
    let (net, _) = network(&osm, VALUES, Some(&region)).unwrap();
    let ends: Vec<(String, String)> =
        links(&net, Mode::Car).into_iter().map(|l| (l.0, l.1)).collect();
    for (a, b) in [("2", "21"), ("2", "40"), ("1", "2")] {
        assert!(ends.contains(&(a.into(), b.into())), "{a}-{b} in {ends:?}");
    }
    assert!(!ends.iter().any(|(a, b)| a == "22" || b == "22" || a == "41" || b == "41"));
    let motorway = direction(&net, Mode::Car, 30, "f").unwrap();
    assert!((motorway.length_m - along(&osm, &[2, 20, 21])).abs() < 1e-9);
}

#[test]
fn a_buffer_joins_a_cut_dual_carriageway_by_a_turnaround() {
    // A box over steps 0..2, 0..2, and a motorway leaving it southwards on two carriageways 0.3
    // steps (23 m) apart: south from (1, 1) and north back to (1.3, 1), each 30 steps (3.3 km)
    // out, past the buffer; a street joins them inside. Cut at the buffer's edge each would end
    // in a sink or start at a source, and the strong component would take both back to the box
    // (Delft's A13 and A4, in version 1 before the turnaround). The turnaround joins the
    // southbound cut end (node 3, 15 steps south) to the northbound one (node 13): a one-way
    // motorway_link at 30 km/h, and both carriageways stay to the buffer's edge.
    let ys = [1.0, -5.0, -15.0, -30.0];
    let mut nodes = Vec::new();
    for (k, &y) in (0i64..).zip(&ys) {
        nodes.extend([at(1 + k, 1.0, y), at(11 + k, 1.3, y)]);
    }
    let motorway = [("highway", "motorway"), ("ref", "A13")];
    let osm = Osm {
        nodes,
        ways: vec![
            OsmWay::new(100, &[1, 2, 3, 4], &motorway),
            OsmWay::new(101, &[14, 13, 12, 11], &motorway),
            road(102, &[11, 1]),
        ],
    };
    let region = Region::bbox(4.80, 45.70, 4.80 + 2.0 * STEP, 45.70 + 2.0 * STEP).unwrap();
    let (net, _) = network(&osm, VALUES, Some(&region)).unwrap();
    let turn = direction(&net, Mode::Car, TURNAROUND_FIRST_ID, "f").expect("a turnaround");
    let l = turn.links[0];
    let id = |n: NodeId| net.node_ids().external(n.raw()).to_owned();
    assert_eq!((id(net.upstream(l)), id(net.downstream(l))), ("3".into(), "13".into()));
    assert_eq!(net.road_class(l), Some("motorway_link"));
    assert_eq!(
        (turn.lanes, turn.speed_km_h, turn.capacity_veh_h),
        (Some(1), Some(30.0), Some(1550.0))
    );
    let kept = |way| direction(&net, Mode::Car, way, "f").map_or(0.0, |d| d.length_m);
    assert!((kept(100) - along(&osm, &[1, 2, 3])).abs() < 1e-6, "southbound to the buffer's edge");
    assert!((kept(101) - along(&osm, &[13, 12, 11])).abs() < 1e-6, "northbound from it");
    // Without a region, the whole motorway: no turnaround.
    let (whole, _) = network(&osm, VALUES, None).unwrap();
    assert!(direction(&whole, Mode::Car, TURNAROUND_FIRST_ID, "f").is_none());
}

#[test]
fn clipping_a_one_way_avenue_leaves_it_to_walkers_and_the_strong_component_removes_it() {
    // A two-way cross street 3-2-4 and a one-way avenue north through 1, 2, 5 that a region cuts
    // short at both ends (1 and 5 are kept as stubs). No cycle uses the avenue by car or by
    // bike: both layers drop it and keep the cross street, contracted to one street; walkers
    // walk the avenue both ways.
    let osm = Osm {
        nodes: vec![
            at(1, 0.0, -3.0),
            at(2, 0.0, 0.0),
            at(5, 0.0, 3.0),
            at(3, -2.0, 0.0),
            at(4, 2.0, 0.0),
        ],
        ways: vec![
            OsmWay::new(100, &[1, 2, 5], &[("highway", "residential"), ("oneway", "yes")]),
            road(101, &[3, 2, 4]),
        ],
    };
    let region =
        Region::bbox(4.80 - 3.0 * STEP, 45.70 - 1.0 * STEP, 4.80 + 3.0 * STEP, 45.70 + 1.0 * STEP)
            .unwrap();
    let (net, report) = network(&osm, VALUES, Some(&region)).unwrap();
    assert_eq!(report.links_unreachable, 4, "the avenue's two pieces, by car and by bike");
    assert_eq!(links(&net, Mode::Car).len(), 2);
    assert_eq!(links(&net, Mode::Bike).len(), 2);
    assert_eq!(links(&net, Mode::Walk).len(), 8, "both streets, two pieces each, both ways");
}
