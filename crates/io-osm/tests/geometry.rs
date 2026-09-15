//! The S125 link-geometry artifact: what it stores and the invariants that
//! make it safe to trust.
//!
//! `crates/core-graph/src/link_geometry.rs` checks the central invariant —
//! that stored geometry remeasures to the stored length — with a
//! `debug_assert!` on every import, which is compiled out in release. These
//! tests assert the same property (and the others S125 commits to) as real,
//! always-enforced tests, on fixtures built the same way
//! `tests/import.rs` builds them.

#![allow(
    clippy::cast_precision_loss,
    reason = "test fixtures build coordinates from small integer counts"
)]

use openmobisim_core_graph::geometry::{LonLat, polyline_length_metres};
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::{EntityId, LinkId};
use openmobisim_io_osm::import::{ImportOptions, import};
use openmobisim_io_osm::source::{MemorySource, OsmNode, OsmWay};

const STEP: f64 = 0.001;

fn node(id: i64, i: f64) -> OsmNode {
    OsmNode::new(id, 4.80 + i * STEP, 45.70)
}

fn residential(id: i64, nodes: impl IntoIterator<Item = i64>) -> OsmWay {
    OsmWay::new(id, nodes, [("highway", "residential")])
}

#[test]
fn every_link_s_geometry_remeasures_to_its_stored_length() {
    // A bend, so the polyline actually differs from the straight line between
    // endpoints — the case that would expose a concatenation bug in
    // `contract` that a straight fixture could not.
    let source = MemorySource::new()
        .nodes([
            OsmNode::new(1, 4.800, 45.700),
            OsmNode::new(2, 4.801, 45.701),
            OsmNode::new(3, 4.802, 45.700),
            OsmNode::new(4, 4.803, 45.701),
            OsmNode::new(5, 4.804, 45.700),
        ])
        .way(residential(100, [1, 2, 3, 4, 5]));

    let mut diagnostics = Diagnostics::new();
    let (network, _, geometry) =
        import(&source, ImportOptions::default(), &mut diagnostics).expect("importable");

    assert!(network.link_count() > 0);
    for link in LinkId::iter_space(network.link_count()) {
        let points: Vec<LonLat> =
            geometry.points(link).expect("every link here has geometry").collect();
        let remeasured = polyline_length_metres(&points);
        let stored = network.link_length(link).get();
        assert!(
            (remeasured - stored).abs() < 1e-6,
            "link {link:?}: geometry remeasures to {remeasured} m, stored length is {stored} m"
        );
    }
}

#[test]
fn contraction_still_remeasures_correctly() {
    // Ten ways describing one bending street: the same case as
    // `tests/import.rs`'s `a_long_chain_collapses_to_one_link_each_way`, here
    // checking that the geometry concatenated across nine contractions still
    // agrees with the summed length, not just that the length itself is
    // conserved (S120 already tests that).
    let mut source = MemorySource::new();
    for i in 0..=10 {
        source = source.node(node(i + 1, i as f64));
    }
    for i in 0..10 {
        source = source.way(residential(100 + i, [i + 1, i + 2]));
    }

    let mut diagnostics = Diagnostics::new();
    let (network, report, geometry) =
        import(&source, ImportOptions::default(), &mut diagnostics).expect("importable");
    assert_eq!(report.links_after_contraction, 2, "one merged link per direction");

    for link in LinkId::iter_space(network.link_count()) {
        let points: Vec<LonLat> =
            geometry.points(link).expect("every link here has geometry").collect();
        assert_eq!(points.len(), 11, "ten segments merged keep all eleven original points");
        let remeasured = polyline_length_metres(&points);
        assert!((remeasured - network.link_length(link).get()).abs() < 1e-6);
    }
}

#[test]
fn a_two_way_street_shares_one_point_run() {
    let source = MemorySource::new()
        .nodes([node(1, 0.0), node(2, 1.0), node(3, 2.0)])
        .ways([residential(100, [1, 2]), residential(101, [2, 3])]);

    let mut diagnostics = Diagnostics::new();
    let (network, _, geometry) =
        import(&source, ImportOptions::default(), &mut diagnostics).expect("importable");

    // Contraction merges this into one two-way street: two directed links,
    // one stored point run (S125).
    assert_eq!(network.link_count(), 2);
    assert_eq!(geometry.street_count(), 1, "a two-way street stores geometry once, not twice");

    let a = LinkId::new(0);
    let b = LinkId::new(1);
    assert!(network.is_reverse_of(a, b));

    let forward: Vec<LonLat> = geometry.points(a).unwrap().collect();
    let backward: Vec<LonLat> = geometry.points(b).unwrap().collect();
    assert_eq!(forward.len(), backward.len());
    assert!(
        forward.iter().zip(backward.iter().rev()).all(|(&f, &r)| f == r),
        "the two directions must read the same points back to front"
    );
}

#[test]
fn a_oneway_street_gets_its_own_street_and_nothing_else_does() {
    let source = MemorySource::new().nodes([node(1, 0.0), node(2, 1.0)]).way(OsmWay::new(
        100,
        [1, 2],
        [("highway", "primary"), ("oneway", "yes")],
    ));

    let mut diagnostics = Diagnostics::new();
    let (network, _, geometry) =
        import(&source, ImportOptions::default(), &mut diagnostics).expect("importable");

    assert_eq!(network.link_count(), 1);
    assert_eq!(geometry.street_count(), 1);
    assert!(geometry.has_geometry(LinkId::new(0)));
}

#[test]
fn the_artifact_matches_its_own_network_and_no_other() {
    let source_a =
        MemorySource::new().nodes([node(1, 0.0), node(2, 1.0)]).way(residential(100, [1, 2]));
    let source_b = MemorySource::new()
        .nodes([node(1, 0.0), node(2, 1.0), node(3, 2.0)])
        .ways([residential(100, [1, 2]), residential(101, [2, 3])]);

    let mut diagnostics = Diagnostics::new();
    let (network_a, _, geometry_a) =
        import(&source_a, ImportOptions::default(), &mut diagnostics).expect("importable");
    let (network_b, _, _) =
        import(&source_b, ImportOptions::default(), &mut diagnostics).expect("importable");

    assert!(geometry_a.matches(&network_a));
    assert!(
        !geometry_a.matches(&network_b),
        "an artifact must not appear to match a network it was not built from"
    );
}
