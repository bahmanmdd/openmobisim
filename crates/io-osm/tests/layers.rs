//! The bike and walk layers an import builds (S195): each its own graph from
//! the same ways, the road network unchanged by them.

#![allow(clippy::float_cmp, reason = "speeds are set exactly from the defaults table")]

use openmobisim_core_graph::layers::{BikeInfrastructure, StaticLayerDefaults};
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::{EntityId, LinkId};
use openmobisim_io_osm::import::{
    Connectivity, ImportOptions, ImportOutput, LayerOptions, import_detailed,
};
use openmobisim_io_osm::source::{MemorySource, OsmNode, OsmWay};

const STEP: f64 = 0.001;

#[allow(clippy::cast_precision_loss, reason = "test fixtures use small counts")]
fn node(id: i64, x: f64, y: f64) -> OsmNode {
    OsmNode::new(id, 4.80 + x * STEP, 45.70 + y * STEP)
}

fn import(source: &MemorySource, options: ImportOptions) -> (ImportOutput, Diagnostics) {
    let mut diagnostics = Diagnostics::new();
    let out = import_detailed(source, options, &mut diagnostics).expect("importable");
    (out, diagnostics)
}

fn with_layers() -> ImportOptions {
    ImportOptions { layers: LayerOptions::BOTH, ..ImportOptions::default() }
}

/// With layers and without contraction, so a test can name every link by its
/// OSM nodes.
fn uncontracted() -> ImportOptions {
    ImportOptions { contract: false, ..with_layers() }
}

/// A small neighbourhood: a two-way avenue with a cycle track, a one-way
/// street open to contraflow cycling, a one-way street that is not, a cycleway,
/// a footway, a motorway and a bike-only service road behind `access=no`.
fn neighbourhood() -> MemorySource {
    MemorySource::new()
        .nodes([
            node(1, 0.0, 0.0),
            node(2, 1.0, 0.0),
            node(3, 2.0, 0.0),
            node(4, 0.0, 1.0),
            node(5, 1.0, 1.0),
            node(6, 2.0, 1.0),
            node(7, 0.0, -1.0),
            node(8, 2.0, -1.0),
            node(9, 1.0, 2.0),
        ])
        .way(OsmWay::new(100, [1, 2, 3], [("highway", "secondary"), ("cycleway", "track")]))
        .way(OsmWay::new(
            101,
            [4, 5, 6],
            [("highway", "residential"), ("oneway", "yes"), ("oneway:bicycle", "no")],
        ))
        .way(OsmWay::new(102, [1, 4], [("highway", "residential"), ("oneway", "yes")]))
        .way(OsmWay::new(103, [3, 6], [("highway", "cycleway")]))
        .way(OsmWay::new(104, [2, 5], [("highway", "footway")]))
        .way(OsmWay::new(105, [7, 8], [("highway", "motorway")]))
        .way(OsmWay::new(
            106,
            [5, 9],
            [("highway", "service"), ("access", "no"), ("bicycle", "designated")],
        ))
}

fn same_network(a: &RoadNetwork, b: &RoadNetwork) {
    assert_eq!(a.node_count(), b.node_count());
    assert_eq!(a.link_count(), b.link_count());
    for link in LinkId::iter_space(a.link_count()) {
        assert_eq!(a.link_from(link), b.link_from(link));
        assert_eq!(a.link_to(link), b.link_to(link));
        assert_eq!(a.link_length(link), b.link_length(link));
        assert_eq!(a.link_class(link), b.link_class(link));
        assert_eq!(
            a.link_external_ids().external(link.raw()),
            b.link_external_ids().external(link.raw())
        );
    }
}

/// The directed links of a graph as (from, to) OSM node pairs, sorted.
fn pairs(network: &RoadNetwork) -> Vec<(String, String)> {
    let nodes = network.node_external_ids();
    let mut v: Vec<(String, String)> = LinkId::iter_space(network.link_count())
        .map(|l| {
            (
                nodes.external(network.link_from(l).raw()).to_owned(),
                nodes.external(network.link_to(l).raw()).to_owned(),
            )
        })
        .collect();
    v.sort();
    v
}

fn has(network: &RoadNetwork, from: &str, to: &str) -> bool {
    pairs(network).contains(&(from.to_owned(), to.to_owned()))
}

#[test]
fn layers_are_off_by_default_and_do_not_change_the_road_network() {
    let source = neighbourhood();
    let (plain, plain_diag) = import(&source, ImportOptions::default());
    assert!(plain.bike.is_none() && plain.walk.is_none());

    for connectivity in [Connectivity::Keep, Connectivity::Strong] {
        let (a, _) = import(&source, ImportOptions { connectivity, ..ImportOptions::default() });
        let (b, b_diag) = import(&source, ImportOptions { connectivity, ..with_layers() });
        same_network(&a.network, &b.network);
        assert_eq!(a.report, b.report, "the road report does not see the layers");
        if connectivity == Connectivity::Keep {
            assert_eq!(plain_diag.rows(), b_diag.rows(), "nor do the road diagnostics");
        }
    }
}

#[test]
fn the_bike_layer_has_contraflow_bike_only_roads_and_no_motorway() {
    let (out, _) = import(&neighbourhood(), uncontracted());
    let bike = out.bike.expect("asked for").network;
    let g = bike.network();

    // Contraflow on 101 (both ways for bikes; split at 5, where the bike-only
    // road joins), none on 102.
    for (a, b) in [("4", "5"), ("5", "6")] {
        assert!(has(g, a, b) && has(g, b, a), "{a} ↔ {b}: {:?}", pairs(g));
        assert!(has(&out.network, a, b) && !has(&out.network, b, a), "cars still one way");
    }
    assert!(has(g, "1", "4") && !has(g, "4", "1"));
    // The bike-only road is a bike link and not a road link.
    assert!(has(g, "5", "9") && has(g, "9", "5"));
    assert!(!has(&out.network, "5", "9"));
    // No motorway, no footway.
    assert!(!has(g, "7", "8") && !has(g, "2", "5"));
}

#[test]
fn bike_speeds_follow_the_infrastructure() {
    let (out, _) = import(&neighbourhood(), uncontracted());
    let bike = out.bike.expect("asked for").network;
    let g = bike.network();
    let d = StaticLayerDefaults::SHIPPED;
    let ids = g.node_external_ids();
    for link in LinkId::iter_space(g.link_count()) {
        let from = ids.external(g.link_from(link).raw());
        let to = ids.external(g.link_to(link).raw());
        let (infrastructure, km_h) = match (from, to) {
            // The avenue with its track, and the cycleway.
            ("1" | "2" | "3", "1" | "2" | "3") | ("3", "6") | ("6", "3") => {
                (BikeInfrastructure::Separated, d.bike_dedicated_km_h)
            }
            _ => (BikeInfrastructure::Mixed, d.bike_mixed_km_h),
        };
        assert_eq!(bike.infrastructure(link), infrastructure, "{from} → {to}");
        assert!((bike.speed(link) * 3.6 - km_h).abs() < 1e-9, "{from} → {to}");
    }
}

#[test]
fn the_walk_layer_walks_every_street_both_ways_but_not_the_motorway_or_cycleway() {
    let (out, _) = import(&neighbourhood(), uncontracted());
    let walk = out.walk.expect("asked for").network;
    let g = walk.network();
    for (a, b) in [("1", "4"), ("2", "5"), ("4", "5"), ("1", "2")] {
        assert!(has(g, a, b) && has(g, b, a), "{a} ↔ {b}: {:?}", pairs(g));
    }
    assert!(!has(g, "7", "8"), "no motorway");
    assert!(!has(g, "3", "6"), "no cycleway");
    assert!(!has(g, "5", "9"), "the bike-only road is closed to walkers");
    let speed = StaticLayerDefaults::SHIPPED.walk_km_h;
    assert!(LinkId::iter_space(g.link_count()).all(|l| (walk.speed(l) * 3.6 - speed).abs() < 1e-9));
}

#[test]
fn every_layer_shares_the_road_network_s_projection_and_node_positions() {
    let (out, _) = import(&neighbourhood(), with_layers());
    let road = &out.network;
    for layer in [out.bike.as_ref().unwrap(), out.walk.as_ref().unwrap()] {
        let g = layer.network.network();
        assert_eq!(g.projection(), road.projection());
        for i in 0..g.node_count() {
            let external = g.node_external_ids().external(i);
            let Some(r) = road.node_external_ids().id_of(external) else { continue };
            assert_eq!(
                g.node_position(EntityId::new(i)),
                road.node_position(EntityId::new(r)),
                "node {external}"
            );
        }
        assert!(layer.geometry.matches(g));
    }
}

#[test]
fn the_layer_report_counts_its_ways_and_its_dedicated_length() {
    let (out, _) = import(&neighbourhood(), with_layers());
    let bike = out.bike.unwrap().report;
    // 100, 101, 102, 103, 106: not the footway, not the motorway.
    assert_eq!(bike.ways_kept, 5);
    assert!(bike.dedicated_length_m > 0 && bike.dedicated_length_m < bike.length_m);
    let walk = out.walk.unwrap().report;
    // 100, 101, 102, 104: not the cycleway, the motorway or the bike-only road.
    assert_eq!(walk.ways_kept, 4);
    assert_eq!(walk.dedicated_length_m, 0);
}

#[test]
fn a_layer_is_trimmed_to_its_own_largest_strong_component() {
    // A cycleway island far from the rest is dropped from the bike layer under
    // `Strong`, and the road network does not notice it.
    let source = neighbourhood()
        .nodes([node(20, 10.0, 10.0), node(21, 11.0, 10.0)])
        .way(OsmWay::new(200, [20, 21], [("highway", "cycleway")]));
    let (out, _) =
        import(&source, ImportOptions { connectivity: Connectivity::Strong, ..with_layers() });
    let bike = out.bike.unwrap();
    assert!(!has(bike.network.network(), "20", "21"));
    assert_eq!(bike.report.links_disconnected, 2);
    assert!(bike.report.components_before >= 2);
}

#[test]
fn a_layer_is_contracted_but_not_across_a_change_of_infrastructure() {
    // One street in two ways: the first half with a lane, the second without.
    let source = MemorySource::new()
        .nodes([node(1, 0.0, 0.0), node(2, 1.0, 0.0), node(3, 2.0, 0.0), node(4, 3.0, 0.0)])
        .way(OsmWay::new(1, [1, 2], [("highway", "residential"), ("cycleway", "lane")]))
        .way(OsmWay::new(2, [2, 3], [("highway", "residential"), ("cycleway", "lane")]))
        .way(OsmWay::new(3, [3, 4], [("highway", "residential")]));
    let (out, _) = import(&source, with_layers());
    let bike = out.bike.unwrap().network;
    // 1–2–3 merge (same lane), 3–4 does not merge into them: two links each way.
    assert_eq!(bike.network().link_count(), 4, "{:?}", pairs(bike.network()));
    // The road network merges all three (the lane is not a car attribute).
    assert_eq!(out.network.link_count(), 2);
}
