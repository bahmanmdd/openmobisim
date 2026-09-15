//! The importer: splitting, contraction, geometry, and what happens to data
//! that does not behave.
//!
//! Everything here runs against an in-memory source rather than a `.osm.pbf`
//! file. That is deliberate — see `source.rs` — and it is what lets each test
//! state the exact three-way junction or broken way it is about, instead of
//! shipping an opaque binary fixture and hoping it still contains what the test
//! name claims.

#![allow(
    clippy::cast_precision_loss,
    reason = "test fixtures build coordinates from small integer counts"
)]

use openmobisim_core_graph::geometry::{LonLat, ground_distance_metres};
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::{EntityId, LinkId, NodeId};
use openmobisim_io_osm::import::{ImportOptions, codes, import};
use openmobisim_io_osm::source::{MemorySource, OsmNode, OsmWay};

/// Roughly 100 m in latitude, and in longitude near 45°N.
const STEP: f64 = 0.001;

/// A node on a west-to-east line at 45.70°N, `i` steps from 4.80°E.
#[allow(clippy::cast_precision_loss, reason = "test fixtures use small counts")]
fn node(id: i64, i: f64) -> OsmNode {
    OsmNode::new(id, 4.80 + i * STEP, 45.70)
}

fn residential(id: i64, nodes: impl IntoIterator<Item = i64>) -> OsmWay {
    OsmWay::new(id, nodes, [("highway", "residential")])
}

fn run(source: &MemorySource) -> (openmobisim_core_graph::network::RoadNetwork, Diagnostics, u64) {
    let mut diagnostics = Diagnostics::new();
    let (network, report, _) =
        import(source, ImportOptions::default(), &mut diagnostics).expect("importable");
    let links = report.links_after_contraction;
    (network, diagnostics, links)
}

#[test]
fn a_two_way_street_becomes_two_links() {
    let source =
        MemorySource::new().nodes([node(1, 0.0), node(2, 1.0)]).way(residential(100, [1, 2]));

    let (net, diag, _) = run(&source);
    assert_eq!(net.node_count(), 2);
    assert_eq!(net.link_count(), 2, "one link per direction");
    assert!(diag.is_empty(), "clean data produces no diagnostics: {:?}", diag.rows());
}

#[test]
fn a_one_way_street_becomes_one_link() {
    let source = MemorySource::new().nodes([node(1, 0.0), node(2, 1.0)]).way(OsmWay::new(
        100,
        [1, 2],
        [("highway", "primary"), ("oneway", "yes")],
    ));

    let (net, _, _) = run(&source);
    assert_eq!(net.link_count(), 1);
    assert_eq!(net.link_from(LinkId::new(0)), NodeId::new(0));
}

#[test]
fn oneway_minus_one_reverses_the_link() {
    let source = MemorySource::new().nodes([node(1, 0.0), node(2, 1.0)]).way(OsmWay::new(
        100,
        [1, 2],
        [("highway", "primary"), ("oneway", "-1")],
    ));

    let (net, _, _) = run(&source);
    assert_eq!(net.link_count(), 1);
    // Node ids are assigned by sorted external id: "1" -> 0, "2" -> 1.
    let link = LinkId::new(0);
    assert_eq!(net.link_from(link), NodeId::new(1), "the link must run against the node order");
    assert_eq!(net.link_to(link), NodeId::new(0));
}

#[test]
fn intermediate_geometry_becomes_length_not_nodes() {
    // A way bending through three interior points is one link, and its length
    // is the polyline, not the straight line between its ends. Throwing the
    // geometry away would understate travel time on every curved street.
    let bend = MemorySource::new()
        .nodes([
            OsmNode::new(1, 4.800, 45.700),
            OsmNode::new(2, 4.801, 45.701),
            OsmNode::new(3, 4.802, 45.700),
            OsmNode::new(4, 4.803, 45.701),
        ])
        .way(residential(100, [1, 2, 3, 4]));

    let (net, _, _) = run(&bend);
    assert_eq!(net.node_count(), 2, "interior geometry points are not network nodes");
    assert_eq!(net.link_count(), 2);

    let straight = ground_distance_metres(LonLat::new(4.800, 45.700), LonLat::new(4.803, 45.701));
    let imported = net.link_length(LinkId::new(0)).get();
    assert!(
        imported > straight * 1.2,
        "a zigzag of {imported:.0} m should be well over the {straight:.0} m straight line"
    );
}

#[test]
fn ways_split_where_they_meet() {
    // A T-junction: the crossing way's middle node is used twice, so the
    // through way must be cut there.
    let source = MemorySource::new()
        .nodes([node(1, 0.0), node(2, 1.0), node(3, 2.0), OsmNode::new(4, 4.801, 45.701)])
        .ways([residential(100, [1, 2, 3]), residential(101, [2, 4])]);

    let (net, _, _) = run(&source);
    assert_eq!(net.node_count(), 4);
    // Three streets, each two-way: 1-2, 2-3, 2-4.
    assert_eq!(net.link_count(), 6);

    let junction = net.node_external_ids().typed_id_of::<NodeId>("2").unwrap();
    assert_eq!(net.out_links(junction).len(), 3, "the T-junction has three exits");
}

#[test]
fn a_tagged_node_splits_a_way_even_when_nothing_else_uses_it() {
    // Signals change what happens at a node, which is the whole reason the node
    // has to survive: the control delay attaches to the approach.
    let source = MemorySource::new()
        .nodes([
            node(1, 0.0),
            OsmNode::new(2, 4.801, 45.700).with_tags([("highway", "traffic_signals")]),
            node(3, 2.0),
        ])
        .way(residential(100, [1, 2, 3]));

    let (net, _, _) = run(&source);
    assert_eq!(net.node_count(), 3, "the signal must remain a node");
    assert_eq!(net.link_count(), 4);

    let signal = net.node_external_ids().typed_id_of::<NodeId>("2").unwrap();
    assert!(net.is_signalised(signal));
    // And its approaches carry a control delay.
    for &approach in net.in_links(signal) {
        assert!(net.link_parameters(approach).control_delay.get() > 10.0);
    }
}

// --- contraction ----------------------------------------------------------

#[test]
fn two_ways_that_are_one_street_are_merged() {
    // OSM splits a street whenever a name or a limit changes. The node between
    // them is not a junction and should not survive.
    let source = MemorySource::new()
        .nodes([node(1, 0.0), node(2, 1.0), node(3, 2.0)])
        .ways([residential(100, [1, 2]), residential(101, [2, 3])]);

    let (net, _, _) = run(&source);
    assert_eq!(net.node_count(), 2, "the joint between the two ways is not a junction");
    assert_eq!(net.link_count(), 2);

    // And the merged link is as long as the two it replaced.
    let whole = ground_distance_metres(LonLat::new(4.800, 45.70), LonLat::new(4.802, 45.70));
    assert!((net.link_length(LinkId::new(0)).get() - whole).abs() < 0.5);
}

#[test]
fn contraction_stops_where_the_street_actually_changes() {
    // A speed limit change is a change in the fundamental diagram. Merging
    // across it would silently pick one of the two.
    let source = MemorySource::new().nodes([node(1, 0.0), node(2, 1.0), node(3, 2.0)]).ways([
        OsmWay::new(100, [1, 2], [("highway", "residential"), ("maxspeed", "30")]),
        OsmWay::new(101, [2, 3], [("highway", "residential"), ("maxspeed", "50")]),
    ]);

    let (net, _, _) = run(&source);
    assert_eq!(net.node_count(), 3);
    assert_eq!(net.link_count(), 4);
}

#[test]
fn contraction_stops_at_a_signal() {
    let source = MemorySource::new()
        .nodes([
            node(1, 0.0),
            OsmNode::new(2, 4.801, 45.700).with_tags([("highway", "traffic_signals")]),
            node(3, 2.0),
        ])
        .ways([residential(100, [1, 2]), residential(101, [2, 3])]);

    let (net, _, _) = run(&source);
    assert_eq!(net.node_count(), 3);
}

#[test]
fn a_cul_de_sac_is_not_contracted_into_a_loop() {
    // Contracting the far end of a stub would turn A -> X -> A into A -> A,
    // which is not a street. This is the condition that protects it.
    let source = MemorySource::new()
        .nodes([node(1, 0.0), node(2, 1.0), node(3, 2.0)])
        .ways([residential(100, [1, 2]), residential(101, [2, 3])]);

    let (net, _, _) = run(&source);
    for link in LinkId::iter_space(net.link_count()) {
        assert_ne!(net.link_from(link), net.link_to(link), "contraction produced a self-loop");
    }
    assert_eq!(net.link_count(), 2);
}

#[test]
fn a_long_chain_collapses_to_one_link_each_way() {
    // Ten OSM ways describing one straight street.
    let mut source = MemorySource::new();
    for i in 0..=10 {
        source = source.node(node(i + 1, i as f64));
    }
    for i in 0..10 {
        source = source.way(residential(100 + i, [i + 1, i + 2]));
    }

    let mut diagnostics = Diagnostics::new();
    let (net, report, _) =
        import(&source, ImportOptions::default(), &mut diagnostics).expect("importable");

    assert_eq!(report.links_before_contraction, 20);
    assert_eq!(report.links_after_contraction, 2);
    assert_eq!(report.nodes_contracted, 9);
    assert!(report.contraction_ratio() > 0.85);
    assert_eq!(net.node_count(), 2);

    let whole = ground_distance_metres(LonLat::new(4.800, 45.70), LonLat::new(4.810, 45.70));
    assert!((net.link_length(LinkId::new(0)).get() - whole).abs() < 1.0);
}

#[test]
fn contraction_can_be_switched_off() {
    let mut source = MemorySource::new();
    for i in 0..=4 {
        source = source.node(node(i + 1, i as f64));
    }
    for i in 0..4 {
        source = source.way(residential(100 + i, [i + 1, i + 2]));
    }

    let mut diagnostics = Diagnostics::new();
    let (net, report, _) = import(
        &source,
        ImportOptions { contract: false, ..ImportOptions::default() },
        &mut diagnostics,
    )
    .expect("importable");

    assert_eq!(net.node_count(), 5);
    assert_eq!(report.links_after_contraction, report.links_before_contraction);
    assert_eq!(report.nodes_contracted, 0);
}

// --- data that does not behave -------------------------------------------

#[test]
fn a_way_cut_by_the_extract_boundary_keeps_what_survives() {
    // A bounding box cuts ways. The part inside is still a street.
    let source = MemorySource::new()
        .nodes([node(1, 0.0), node(2, 1.0)]) // node 3 is outside the extract
        .way(residential(100, [1, 2, 3]));

    let (net, diag, _) = run(&source);
    assert_eq!(net.link_count(), 2, "the surviving part is still imported");
    assert_eq!(diag.count_of(codes::MISSING_NODE), 1);
}

#[test]
fn a_way_with_nothing_left_is_dropped_and_recorded() {
    let source = MemorySource::new()
        .nodes([node(1, 0.0), node(2, 1.0)])
        .ways([residential(100, [1, 2]), residential(101, [7, 8, 9])]);

    let (net, diag, _) = run(&source);
    assert_eq!(net.link_count(), 2);
    assert_eq!(diag.count_of(codes::WAY_LOST_TO_MISSING_NODES), 1);
    assert_eq!(diag.count_of(codes::MISSING_NODE), 3);
}

#[test]
fn non_roads_are_ignored_quietly_and_odd_roads_are_recorded() {
    let source = MemorySource::new().nodes([node(1, 0.0), node(2, 1.0), node(3, 2.0)]).ways([
        residential(100, [1, 2]),
        OsmWay::new(101, [1, 2, 3], [("building", "yes")]),
        OsmWay::new(102, [2, 3], [("highway", "proposed")]),
        OsmWay::new(103, [2, 3], [("highway", "service"), ("access", "private")]),
    ]);

    let (net, diag, _) = run(&source);
    assert_eq!(net.link_count(), 2, "only the residential way is a road");

    // A building is the overwhelming majority of an extract; counting it would
    // drown every other line of the report.
    assert_eq!(diag.count_of(openmobisim_core_graph::network::codes::UNKNOWN_HIGHWAY_CLASS), 0);
    assert_eq!(diag.count_of(codes::UNKNOWN_HIGHWAY_CLASS), 1);
    assert_eq!(diag.count_of(codes::ACCESS_DENIED), 1);
}

#[test]
fn an_unparseable_speed_limit_falls_back_and_is_recorded() {
    let source = MemorySource::new().nodes([node(1, 0.0), node(2, 1.0)]).way(OsmWay::new(
        100,
        [1, 2],
        [("highway", "primary"), ("maxspeed", "DE:urban")],
    ));

    let (net, diag, _) = run(&source);
    assert_eq!(diag.count_of(codes::MAXSPEED_UNPARSED), 1);
    // The class default applies: primary is 60 km/h.
    let v = net.link_parameters(LinkId::new(0)).free_flow_speed.as_km_per_hour();
    assert!((v - 60.0).abs() < 1e-9, "expected the class default, got {v}");
}

#[test]
fn an_empty_extract_is_an_error_not_a_silent_empty_network() {
    let mut diagnostics = Diagnostics::new();
    let empty = MemorySource::new();
    assert!(import(&empty, ImportOptions::default(), &mut diagnostics).is_err());
}

// --- determinism ----------------------------------------------------------

#[test]
fn importing_the_same_extract_twice_gives_the_same_network() {
    // Contraction walks hash maps; if its candidate order leaked into the
    // result, ids would differ between builds and every cached artifact would
    // stop being comparable.
    let mut source = MemorySource::new();
    for i in 0..=8 {
        source = source.node(node(i + 1, i as f64));
    }
    source = source.node(OsmNode::new(20, 4.804, 45.701));
    for i in 0..8 {
        source = source.way(residential(100 + i, [i + 1, i + 2]));
    }
    source = source.way(residential(200, [5, 20]));

    let (a, _, _) = run(&source);
    let (b, _, _) = run(&source);

    assert_eq!(a.node_count(), b.node_count());
    assert_eq!(a.link_count(), b.link_count());
    for link in LinkId::iter_space(a.link_count()) {
        assert_eq!(a.link_from(link), b.link_from(link));
        assert_eq!(a.link_to(link), b.link_to(link));
        assert_eq!(a.link_length(link), b.link_length(link));
        assert_eq!(
            a.link_external_ids().external(link.raw()),
            b.link_external_ids().external(link.raw())
        );
    }
}

#[test]
fn the_report_adds_up() {
    let source = MemorySource::new().nodes([node(1, 0.0), node(2, 1.0), node(3, 2.0)]).ways([
        residential(100, [1, 2]),
        residential(101, [2, 3]),
        OsmWay::new(102, [1, 3], [("building", "yes")]),
    ]);

    let mut diagnostics = Diagnostics::new();
    let (net, report, _) =
        import(&source, ImportOptions::default(), &mut diagnostics).expect("importable");

    assert_eq!(report.ways_seen, 3);
    assert_eq!(report.ways_kept, 2);
    assert_eq!(report.nodes_seen, 3);
    assert_eq!(report.nodes_kept, 3);
    assert_eq!(report.links_before_contraction, 4);
    assert_eq!(report.links_after_contraction, 2);
    assert_eq!(u64::from(net.link_count()), report.links_after_contraction);
}

// --- conservation ---------------------------------------------------------

#[test]
fn contraction_conserves_total_network_length() {
    // The property that makes contraction safe: it changes how the network is
    // described, never how long it is. If this fails, every travel time in
    // every run is wrong by whatever it lost.
    let mut source = MemorySource::new();
    for i in 0..=12 {
        source = source.node(node(i + 1, i as f64));
    }
    source = source.node(OsmNode::new(50, 4.805, 45.701)); // a stub off node 6
    for i in 0..12 {
        source = source.way(residential(100 + i, [i + 1, i + 2]));
    }
    source = source.way(residential(300, [6, 50]));

    let mut diagnostics = Diagnostics::new();
    let uncontracted = import(
        &source,
        ImportOptions { contract: false, ..ImportOptions::default() },
        &mut diagnostics,
    )
    .expect("importable")
    .0;
    let contracted =
        import(&source, ImportOptions::default(), &mut diagnostics).expect("importable").0;

    let total = |net: &openmobisim_core_graph::network::RoadNetwork| -> f64 {
        LinkId::iter_space(net.link_count()).map(|l| net.link_length(l).get()).sum()
    };

    let before = total(&uncontracted);
    let after = total(&contracted);
    assert!(
        (before - after).abs() < 1e-6,
        "contraction changed total network length from {before:.3} m to {after:.3} m"
    );
    assert!(contracted.link_count() < uncontracted.link_count(), "contraction did nothing");
}

#[test]
fn a_corner_is_contracted_because_it_is_a_bend_not_a_junction() {
    // Two streets meeting at a right angle with nothing else at the node: a
    // traveller has no choice there, so it is not a junction. The merged link
    // keeps the full length of both legs.
    let source = MemorySource::new()
        .nodes([
            OsmNode::new(1, 4.800, 45.700),
            OsmNode::new(2, 4.802, 45.700),
            OsmNode::new(3, 4.802, 45.702),
        ])
        .ways([residential(100, [1, 2]), residential(101, [2, 3])]);

    let (net, _, _) = run(&source);
    assert_eq!(net.node_count(), 2, "the corner is a bend, not a junction");

    let legs = ground_distance_metres(LonLat::new(4.800, 45.700), LonLat::new(4.802, 45.700))
        + ground_distance_metres(LonLat::new(4.802, 45.700), LonLat::new(4.802, 45.702));
    assert!(
        (net.link_length(LinkId::new(0)).get() - legs).abs() < 0.5,
        "the merged link must keep the length of both legs"
    );
}
