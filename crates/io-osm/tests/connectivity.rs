//! Cutting a study area, ways cut by the edge of an extract, and keeping the
//! strongly connected part (S172).
//!
//! Each test builds the smallest extract that has the situation in it, so the
//! right answer can be read off the drawing before the test is run.

#![allow(
    clippy::cast_precision_loss,
    reason = "test fixtures build coordinates from small integer counts"
)]

use openmobisim_core_graph::connectivity::analyse;
use openmobisim_core_graph::defaults::SignalDefaults;
use openmobisim_core_graph::geometry::{LonLat, ground_distance_metres};
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::{EntityId, LinkId};
use openmobisim_io_osm::import::{
    Connectivity, DropReason, ImportOptions, ImportOutput, codes, import_detailed,
};
use openmobisim_io_osm::region::{ClippedSource, Region};
use openmobisim_io_osm::source::{MemorySource, OsmNode, OsmSource, OsmWay};

/// About 100 m.
const STEP: f64 = 0.001;

/// A node on a grid: `x` steps east and `y` steps north of (4.80 E, 45.70 N).
fn at(id: i64, x: f64, y: f64) -> OsmNode {
    OsmNode::new(id, 4.80 + x * STEP, 45.70 + y * STEP)
}

fn road(id: i64, nodes: impl IntoIterator<Item = i64>) -> OsmWay {
    OsmWay::new(id, nodes, [("highway", "residential")])
}

fn one_way(id: i64, nodes: impl IntoIterator<Item = i64>) -> OsmWay {
    OsmWay::new(id, nodes, [("highway", "residential"), ("oneway", "yes")])
}

fn run(source: &dyn OsmSource, connectivity: Connectivity) -> (ImportOutput, Diagnostics) {
    run_with(source, connectivity, true)
}

/// Contraction off: every OSM junction stays a node, so a test can count them.
/// (With it on, the corners of a ring are bends and disappear, S120.)
fn run_raw(source: &dyn OsmSource, connectivity: Connectivity) -> (ImportOutput, Diagnostics) {
    run_with(source, connectivity, false)
}

fn run_with(
    source: &dyn OsmSource,
    connectivity: Connectivity,
    contract: bool,
) -> (ImportOutput, Diagnostics) {
    let mut diagnostics = Diagnostics::new();
    let options = ImportOptions { connectivity, contract, ..ImportOptions::default() };
    let out = import_detailed(source, options, &mut diagnostics).expect("importable");
    (out, diagnostics)
}

fn total_length(net: &RoadNetwork) -> f64 {
    LinkId::iter_space(net.link_count()).map(|l| net.link_length(l).get()).sum()
}

fn is_connected(net: &RoadNetwork) -> bool {
    let turns = TurnTable::build(net, SignalDefaults::SHIPPED);
    analyse(net, &turns).is_strongly_connected()
}

// --- regions --------------------------------------------------------------

#[test]
fn a_rectangle_contains_its_edges_and_nothing_beyond() {
    let r = Region::bbox(1.0, 2.0, 3.0, 4.0).unwrap();
    assert!(r.contains(1.0, 2.0) && r.contains(3.0, 4.0) && r.contains(2.0, 3.0));
    assert!(!r.contains(0.999, 3.0) && !r.contains(2.0, 4.001));
}

#[test]
fn bad_regions_are_refused_with_a_reason() {
    assert!(Region::bbox(3.0, 2.0, 1.0, 4.0).is_err(), "west must be west of east");
    assert!(Region::bbox(1.0, 2.0, f64::NAN, 4.0).is_err());
    assert!(Region::bbox(1.0, 2.0, 200.0, 4.0).is_err());
    assert!(Region::polygon(vec![(0.0, 0.0), (1.0, 1.0)]).is_err());
    assert!(Region::polygon(vec![(0.0, 0.0), (1.0, 1.0), (f64::INFINITY, 0.0)]).is_err());
}

#[test]
fn a_polygon_can_be_concave_and_either_winding() {
    // An L: the square [0,3]² without the square [1,3]×[1,3].
    let l = vec![(0.0, 0.0), (3.0, 0.0), (3.0, 1.0), (1.0, 1.0), (1.0, 3.0), (0.0, 3.0)];
    let mut reversed = l.clone();
    reversed.reverse();
    for vertices in [l, reversed] {
        let r = Region::polygon(vertices).unwrap();
        assert!(r.contains(0.5, 0.5) && r.contains(2.5, 0.5) && r.contains(0.5, 2.5));
        assert!(!r.contains(2.0, 2.0), "the notch is outside");
        assert!(!r.contains(-0.5, 0.5) && !r.contains(3.5, 0.5));
    }
    // A repeated closing vertex is accepted.
    let closed = Region::polygon(vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 0.0)]).unwrap();
    assert!(closed.contains(0.9, 0.2));
}

/// A street of six nodes going east, 1..=6, at y = 0.
fn street() -> MemorySource {
    MemorySource::new().nodes((1..=6).map(|i| at(i, (i - 1) as f64, 0.0))).way(road(100, 1..=6))
}

#[test]
fn a_clip_keeps_the_nodes_inside_and_cuts_the_road_there() {
    // Nodes 1..=6 are 0..5 steps east. A box over steps 1.5..4.5 holds nodes 3, 4, 5
    // (steps 2, 3, 4).
    let src = street();
    let region = Region::bbox(4.80 + 1.5 * STEP, 45.69, 4.80 + 4.5 * STEP, 45.71).unwrap();
    let clipped = ClippedSource::new(&src, &region);

    let mut ways = Vec::new();
    clipped.for_each_way(&mut |w| ways.push(w)).unwrap();
    assert_eq!(ways.len(), 1);
    assert_eq!(ways[0].node_ids, vec![3, 4, 5]);
    assert_eq!(ways[0].tag("highway"), Some("residential"), "tags are kept");
    let mut nodes = Vec::new();
    clipped.for_each_node(&mut |n| nodes.push(n.id)).unwrap();
    assert_eq!(nodes, vec![3, 4, 5]);

    let (out, _) = run(&clipped, Connectivity::Keep);
    assert_eq!(out.network.link_count(), 2, "one two-way street");
    // Nodes 3 to 5 are two steps apart; the length is the ground distance, both ways.
    let (a, b) = (src.node_map()[&3], src.node_map()[&5]);
    let expected =
        2.0 * ground_distance_metres(LonLat::new(a.lon, a.lat), LonLat::new(b.lon, b.lat));
    assert!((total_length(&out.network) - expected).abs() < 0.5, "{expected}");
}

#[test]
fn a_road_that_leaves_and_comes_back_is_two_roads_not_one_with_a_bridge() {
    // A road along y = 0 through nodes 1, 2, 3, 4, 5, and a region that excludes
    // the middle node 3 (a notch cut out of a rectangle by a polygon).
    let src = MemorySource::new()
        .nodes((1..=5).map(|i| at(i, (i - 1) as f64, 0.0)))
        .way(road(100, 1..=5));
    let x = |k: f64| 4.80 + k * STEP;
    let (lo, hi) = (45.699, 45.701);
    // A "U" opening upwards around node 3 at x = 2 steps: the region misses a
    // small box around it.
    let region = Region::polygon(vec![
        (x(-0.5), lo),
        (x(4.5), lo),
        (x(4.5), hi),
        (x(2.5), hi),
        (x(2.5), 45.70 - 0.0002),
        (x(1.5), 45.70 - 0.0002),
        (x(1.5), hi),
        (x(-0.5), hi),
    ])
    .unwrap();
    assert!(!region.contains(x(2.0), 45.70), "node 3 is outside");
    assert!(region.contains(x(1.0), 45.70) && region.contains(x(3.0), 45.70));

    let clipped = ClippedSource::new(&src, &region);
    let mut ways = Vec::new();
    clipped.for_each_way(&mut |w| ways.push(w.node_ids)).unwrap();
    assert_eq!(ways, vec![vec![1, 2], vec![4, 5]]);

    let (out, _) = run(&clipped, Connectivity::Keep);
    assert_eq!(out.network.link_count(), 4, "two streets of one segment, two ways each");
    // Two segments of ~70 m: nothing spans the gap. A bridge would add a link
    // of about 140 m between nodes 2 and 4.
    let longest = LinkId::iter_space(out.network.link_count())
        .map(|l| out.network.link_length(l).get())
        .fold(0.0_f64, f64::max);
    assert!(longest < 100.0, "no link crosses the gap: {longest}");
}

#[test]
fn a_way_with_nodes_missing_from_the_extract_is_not_bridged_either() {
    // The same thing without a region: node 3 is simply not in the extract.
    let src = MemorySource::new()
        .nodes([at(1, 0.0, 0.0), at(2, 1.0, 0.0), at(4, 3.0, 0.0), at(5, 4.0, 0.0)])
        .way(road(100, 1..=5));
    let (out, diag) = run(&src, Connectivity::Keep);
    assert_eq!(out.network.link_count(), 4, "two pieces, both ways");
    assert_eq!(diag.count_of(codes::MISSING_NODE), 1);
    assert_eq!(diag.count_of(codes::WAY_CUT_INTO_PIECES), 1);
    assert_eq!(diag.count_of(codes::WAY_LOST_TO_MISSING_NODES), 0);
}

#[test]
fn the_clip_is_a_subset_and_changes_nothing_it_keeps() {
    // Clipping a small grid to its left two columns keeps exactly the links
    // inside, at their full length.
    let mut src = MemorySource::new();
    let mut id = 1;
    let mut ways = Vec::new();
    let mut grid = [[0i64; 4]; 3];
    for (row, line) in grid.iter_mut().enumerate() {
        for (col, cell) in line.iter_mut().enumerate() {
            src = src.node(at(id, col as f64, row as f64));
            *cell = id;
            id += 1;
        }
    }
    let mut way_id = 1000;
    for line in &grid {
        ways.push(road(way_id, line.iter().copied()));
        way_id += 1;
    }
    for col in [0usize, 1, 2, 3] {
        ways.push(road(way_id, grid.iter().map(|line| line[col])));
        way_id += 1;
    }
    let src = src.ways(ways);

    let (whole, _) = run_raw(&src, Connectivity::Keep);
    let region =
        Region::bbox(4.80 - 0.1 * STEP, 45.70 - 0.1 * STEP, 4.80 + 1.1 * STEP, 45.70 + 2.1 * STEP)
            .unwrap();
    let clipped = ClippedSource::new(&src, &region);
    let (part, _) = run_raw(&clipped, Connectivity::Keep);

    // 3 rows × 2 columns of nodes: 3 horizontal and 2·2 vertical street pieces.
    assert_eq!(part.network.node_count(), 6);
    assert_eq!(part.network.link_count(), 2 * (3 + 4));
    assert!(part.network.link_count() < whole.network.link_count());
    assert!(total_length(&part.network) < total_length(&whole.network));
    assert!(is_connected(&part.network));
}

// --- closed loops -----------------------------------------------------------

#[test]
fn a_closed_loop_with_no_junction_is_dropped_counted_and_returned() {
    // A street 1—2 and a small loop road that leaves node 2 and returns to it:
    // 2 → 3 → 4 → 2 with nothing else touching it.
    let src = MemorySource::new()
        .nodes([at(1, 0.0, 0.0), at(2, 1.0, 0.0), at(3, 1.0, 1.0), at(4, 2.0, 1.0)])
        .ways([road(100, [1, 2]), road(101, [2, 3, 4, 2])]);
    let (out, diag) = run(&src, Connectivity::Keep);
    assert_eq!(out.network.link_count(), 2, "only the street");
    assert_eq!(out.report.closed_loops_dropped, 1);
    assert_eq!(diag.count_of(codes::CLOSED_LOOP_DROPPED), 1);
    assert_eq!(out.dropped.len(), 1);
    assert_eq!(out.dropped[0].reason, DropReason::ClosedLoop);
    assert_eq!(out.dropped[0].way_id, 101);
    assert_eq!(out.dropped[0].geometry.len(), 4, "the loop's own shape");
}

// --- connectivity -----------------------------------------------------------

/// A two-way square 1—2—3—4—1, one-way spurs into and out of it, and an island.
fn messy() -> MemorySource {
    MemorySource::new()
        .nodes([
            at(1, 0.0, 0.0),
            at(2, 2.0, 0.0),
            at(3, 2.0, 2.0),
            at(4, 0.0, 2.0),
            at(5, 4.0, 0.0),  // end of a one-way spur out of the square: a sink
            at(6, -2.0, 0.0), // start of a one-way spur into the square: a source
            at(7, 10.0, 10.0),
            at(8, 12.0, 10.0),
        ])
        .ways([
            road(100, [1, 2]),
            road(101, [2, 3]),
            road(102, [3, 4]),
            road(103, [4, 1]),
            one_way(104, [2, 5]), // out
            one_way(105, [6, 1]), // in
            road(106, [7, 8]),    // an island: connected inside, alone
        ])
}

#[test]
fn keeping_everything_is_the_default_and_leaves_the_network_disconnected() {
    let (out, _) = run(&messy(), Connectivity::Keep);
    assert!(!is_connected(&out.network));
    assert_eq!(out.dropped.len(), 0);
    assert_eq!(out.report.links_disconnected, 0);
    assert_eq!(out.report.components_before, 0, "not measured when not asked for");
}

#[test]
fn the_strong_option_keeps_the_square_and_drops_spurs_and_island() {
    let (keep, _) = run_raw(&messy(), Connectivity::Keep);
    let (out, _) = run_raw(&messy(), Connectivity::Strong);
    let net = &out.network;

    assert!(is_connected(net));
    assert_eq!(net.node_count(), 4);
    assert_eq!(net.link_count(), 8, "four streets, both ways");

    // What went: two one-way spurs (1 link each) and the island (2 links).
    let r = &out.report;
    assert_eq!(r.links_disconnected, 4);
    assert_eq!(r.nodes_disconnected, 4, "the sink, the source and the island's two nodes");
    assert_eq!(out.dropped.len(), 4);
    assert!(out.dropped.iter().all(|d| d.reason == DropReason::NotStronglyConnected));
    let mut ways: Vec<i64> = out.dropped.iter().map(|d| d.way_id).collect();
    ways.sort_unstable();
    assert_eq!(ways, vec![104, 105, 106, 106]);
    // The square, the sink, the source, and the island: four components with
    // more than one node... the square (4), the island (2), and two singletons.
    assert_eq!(r.components_before, 4);

    // The kept part is exactly the square: nothing was added or altered, and
    // length is conserved between what was kept, what was dropped, and the whole.
    let dropped_length = f64::from(u32::try_from(r.length_disconnected_m).unwrap());
    let whole = total_length(&keep.network);
    let kept = total_length(net);
    assert!((whole - kept - dropped_length).abs() < 4.0, "{whole} = {kept} + {dropped_length}");
}

#[test]
fn keeping_the_strong_part_is_idempotent() {
    // Running the filter on what it produced removes nothing more.
    let (first, _) = run(&messy(), Connectivity::Strong);
    assert!(first.report.links_disconnected > 0);

    // Rebuild an extract holding exactly the square and import it again.
    let square = MemorySource::new()
        .nodes([at(1, 0.0, 0.0), at(2, 2.0, 0.0), at(3, 2.0, 2.0), at(4, 0.0, 2.0)])
        .ways([road(100, [1, 2]), road(101, [2, 3]), road(102, [3, 4]), road(103, [4, 1])]);
    let (again, _) = run(&square, Connectivity::Strong);
    assert_eq!(again.report.links_disconnected, 0);
    assert_eq!(again.network.link_count(), first.network.link_count());
    assert_eq!(again.report.components_before, 1);
}

#[test]
fn dropping_a_spur_lets_contraction_merge_what_it_had_pinned() {
    // A straight two-way street 1—2—3 with a one-way spur off node 2. With the
    // spur, node 2 is a junction and the street is two links each way; without
    // it, node 2 is just a point on the street and contraction merges them —
    // the same street, the same length, in fewer links.
    let src = MemorySource::new()
        .nodes([at(1, 0.0, 0.0), at(2, 1.0, 0.0), at(3, 2.0, 0.0), at(4, 1.0, 1.0)])
        .ways([road(100, [1, 2]), road(101, [2, 3]), one_way(102, [2, 4])]);
    let (keep, _) = run(&src, Connectivity::Keep);
    let (strong, _) = run(&src, Connectivity::Strong);
    assert_eq!(keep.network.link_count(), 5);
    assert_eq!(strong.network.link_count(), 2, "one street, both ways");
    assert_eq!(strong.network.node_count(), 2);
    let spur = total_length(&keep.network) - total_length(&strong.network);
    assert!((spur - out_length(&strong, &keep)).abs() < 1e-6);
}

/// The length the strong import dropped, as the report gives it in metres,
/// compared exactly to what the two networks differ by is not possible (it is
/// rounded), so this recomputes it from the dropped geometry.
fn out_length(strong: &ImportOutput, _keep: &ImportOutput) -> f64 {
    strong
        .dropped
        .iter()
        .map(|d| openmobisim_core_graph::geometry::polyline_length_metres(&d.geometry))
        .sum()
}

#[test]
fn a_network_that_is_already_connected_loses_nothing_and_costs_no_change() {
    let square = MemorySource::new()
        .nodes([at(1, 0.0, 0.0), at(2, 2.0, 0.0), at(3, 2.0, 2.0), at(4, 0.0, 2.0)])
        .ways([road(100, [1, 2]), road(101, [2, 3]), road(102, [3, 4]), road(103, [4, 1])]);
    let (keep, _) = run(&square, Connectivity::Keep);
    let (strong, _) = run(&square, Connectivity::Strong);
    assert_eq!(strong.network.link_count(), keep.network.link_count());
    assert!((total_length(&strong.network) - total_length(&keep.network)).abs() < 1e-9);
    assert!(strong.dropped.is_empty());
}

#[test]
fn clipping_a_one_way_avenue_leaves_a_sink_and_a_source_that_the_filter_removes() {
    // A two-way cross street (node 3 — 4) and a one-way avenue running north
    // through nodes 1, 2, 5, 6 that a region cuts short at both ends. The avenue
    // is a one-way road alone: no cycle uses it, so it goes, and the cross street
    // stays.
    let src = MemorySource::new()
        .nodes([
            at(1, 0.0, -3.0),
            at(2, 0.0, 0.0),
            at(5, 0.0, 3.0),
            at(3, -2.0, 0.0),
            at(4, 2.0, 0.0),
        ])
        .ways([one_way(100, [1, 2, 5]), road(101, [3, 2, 4])]);
    let region =
        Region::bbox(4.80 - 3.0 * STEP, 45.70 - 1.0 * STEP, 4.80 + 3.0 * STEP, 45.70 + 1.0 * STEP)
            .unwrap();
    let clipped = ClippedSource::new(&src, &region);
    let (out, _) = run(&clipped, Connectivity::Strong);
    // Only the cross street survives inside the box (the avenue's nodes 1 and 5
    // are outside, so it is a single node): two links of node 3—2 and 2—4, both
    // ways, contracted into one street.
    assert!(is_connected(&out.network));
    assert_eq!(out.network.link_count(), 2);
}

// --- by mode ------------------------------------------------------------------

fn footway(id: i64, nodes: impl IntoIterator<Item = i64>) -> OsmWay {
    OsmWay::new(id, nodes, [("highway", "footway")])
}

/// A large two-way square of roads (1–4), a small road pair (5 ⇄ 6) off to the
/// side, and a footway from the square to the small pair. Every link of the
/// network together is one component; the roads alone are two.
fn roads_joined_by_a_footway() -> MemorySource {
    MemorySource::new()
        .nodes([
            at(1, 0.0, 0.0),
            at(2, 2.0, 0.0),
            at(3, 2.0, 2.0),
            at(4, 0.0, 2.0),
            at(5, 5.0, 0.0),
            at(6, 7.0, 0.0),
        ])
        .ways([
            road(100, [1, 2]),
            road(101, [2, 3]),
            road(102, [3, 4]),
            road(103, [4, 1]),
            road(104, [5, 6]),
            footway(105, [2, 5]),
        ])
}

#[test]
fn a_footway_does_not_save_a_road_fragment_a_car_cannot_reach() {
    let (out, _) = run_raw(&roads_joined_by_a_footway(), Connectivity::Strong);
    // The small road pair goes: two links. The footway stays: it is not a road.
    assert_eq!(out.report.links_disconnected, 2);
    let ways: Vec<i64> = out.dropped.iter().map(|d| d.way_id).collect();
    assert_eq!(ways, vec![104, 104]);
    assert!(out.dropped.iter().all(|d| d.reason == DropReason::NotStronglyConnected));
    // 8 square links, and the footway's 2.
    assert_eq!(out.network.link_count(), 10);
    let footways = LinkId::iter_space(out.network.link_count())
        .filter(|&l| !out.network.link_class(l).carries_motor_traffic())
        .count();
    assert_eq!(footways, 2, "the footway, both ways, untouched");
}

#[test]
fn footways_are_never_removed_even_when_they_lead_nowhere() {
    // A footway on its own, far from any road, and a two-way road.
    let src = MemorySource::new()
        .nodes([at(1, 0.0, 0.0), at(2, 2.0, 0.0), at(7, 9.0, 9.0), at(8, 11.0, 9.0)])
        .ways([road(100, [1, 2]), footway(101, [7, 8])]);
    let (out, _) = run_raw(&src, Connectivity::Strong);
    assert_eq!(out.report.links_disconnected, 0);
    assert_eq!(out.network.link_count(), 4);
}

#[test]
fn a_network_with_no_roads_at_all_is_left_alone() {
    let src =
        MemorySource::new().nodes([at(1, 0.0, 0.0), at(2, 2.0, 0.0)]).way(footway(100, [1, 2]));
    let (out, _) = run_raw(&src, Connectivity::Strong);
    assert_eq!(out.network.link_count(), 2);
    assert_eq!(out.report.components_before, 0);
}

// --- contracting the car layer past pedestrian crossings (S172) ---------------

fn run_drivable(source: &dyn OsmSource) -> (ImportOutput, Diagnostics) {
    let mut diagnostics = Diagnostics::new();
    let options = ImportOptions { contract_drivable: true, ..ImportOptions::default() };
    let out = import_detailed(source, options, &mut diagnostics).expect("importable");
    (out, diagnostics)
}

/// A two-way street 1—2—3—4 along y = 0, and a footway across it at node 2 (from
/// 5 on the north side through 2 to 6 on the south side): a pedestrian crossing.
fn street_with_a_crossing() -> MemorySource {
    MemorySource::new()
        .nodes([
            at(1, 0.0, 0.0),
            at(2, 1.0, 0.0),
            at(3, 2.0, 0.0),
            at(4, 3.0, 0.0),
            at(5, 1.0, 0.3),
            at(6, 1.0, -0.3),
        ])
        .ways([road(100, [1, 2, 3, 4]), footway(101, [5, 2, 6])])
}

#[test]
fn a_crossing_footway_splits_the_street_by_default() {
    // Today's behaviour, kept: node 2 is a junction, so the street is two links
    // each way, and three short ones once the crossing is counted.
    let (out, _) = run(&street_with_a_crossing(), Connectivity::Keep);
    let drivable = LinkId::iter_space(out.network.link_count())
        .filter(|&l| out.network.link_class(l).carries_motor_traffic())
        .count();
    assert_eq!(drivable, 4, "1—2 and 2—4, both ways");
}

#[test]
fn the_car_layer_can_be_contracted_past_a_crossing_and_the_footway_is_untouched() {
    let (default, _) = run(&street_with_a_crossing(), Connectivity::Keep);
    let (out, _) = run_drivable(&street_with_a_crossing());
    let net = &out.network;

    let is_car = |l: LinkId| net.link_class(l).carries_motor_traffic();
    let cars: Vec<LinkId> = LinkId::iter_space(net.link_count()).filter(|&l| is_car(l)).collect();
    assert_eq!(cars.len(), 2, "one street, both ways: the crossing no longer splits it");
    let footways = net.link_count() as usize - cars.len();
    assert_eq!(footways, 4, "the footway, in two halves and both ways, exactly as before");

    // Nothing was lost: the street is as long as it was, and node 2 is still there
    // for the pedestrians (node 1 and 4, the sides 5 and 6, and 2).
    let car_length = |n: &RoadNetwork| {
        LinkId::iter_space(n.link_count())
            .filter(|&l| n.link_class(l).carries_motor_traffic())
            .map(|l| n.link_length(l).get())
            .sum::<f64>()
    };
    assert!((car_length(net) - car_length(&default.network)).abs() < 1e-6);
    assert_eq!(net.node_count(), default.network.node_count(), "a node stays for the footway");
}

#[test]
fn a_signal_at_the_crossing_still_splits_the_street() {
    // A signalised crossing delays cars, so the node is where something happens.
    let src = MemorySource::new()
        .nodes([
            at(1, 0.0, 0.0),
            OsmNode::new(2, 4.80 + STEP, 45.70).with_tags([("highway", "traffic_signals")]),
            at(3, 2.0, 0.0),
            at(4, 3.0, 0.0),
            at(5, 1.0, 0.3),
            at(6, 1.0, -0.3),
        ])
        .ways([road(100, [1, 2, 3, 4]), footway(101, [5, 2, 6])]);
    let (out, _) = run_drivable(&src);
    let cars = LinkId::iter_space(out.network.link_count())
        .filter(|&l| out.network.link_class(l).carries_motor_traffic())
        .count();
    assert_eq!(cars, 4, "the signal is a place where something happens; the street stays split");
}

#[test]
fn a_one_way_street_is_contracted_past_a_crossing_too_and_a_junction_is_not() {
    // A one-way street 1 → 2 → 3 with a crossing at 2, and a side street that
    // really joins at node 4 (a car can turn there).
    let src = MemorySource::new()
        .nodes([
            at(1, 0.0, 0.0),
            at(2, 1.0, 0.0),
            at(3, 2.0, 0.0),
            at(4, 3.0, 0.0),
            at(5, 1.0, 0.3),
            at(6, 1.0, -0.3),
            at(7, 3.0, 1.0),
        ])
        .ways([one_way(100, [1, 2, 3, 4]), footway(101, [5, 2, 6]), road(102, [4, 7])]);
    let (out, _) = run_drivable(&src);
    let cars: Vec<LinkId> = LinkId::iter_space(out.network.link_count())
        .filter(|&l| out.network.link_class(l).carries_motor_traffic())
        .collect();
    // 1 → 4 (merged past 2 and 3, which nothing joins) and the side street both ways.
    assert_eq!(cars.len(), 3);
    let longest = cars.iter().map(|&l| out.network.link_length(l).get()).fold(0.0, f64::max);
    assert!(longest > 200.0, "the one-way street is a single link of three blocks: {longest}");
}

#[test]
fn contracting_the_car_layer_leaves_a_network_without_footways_as_it_was() {
    // With no footway anywhere, the two ways of contracting are the same network.
    let src = street();
    let (a, _) = run(&src, Connectivity::Keep);
    let (b, _) = run_drivable(&src);
    assert_eq!(a.network.link_count(), b.network.link_count());
    assert_eq!(a.network.node_count(), b.network.node_count());
    let total = |n: &RoadNetwork| {
        LinkId::iter_space(n.link_count()).map(|l| n.link_length(l).get()).sum::<f64>()
    };
    assert!((total(&a.network) - total(&b.network)).abs() < 1e-9);
}
