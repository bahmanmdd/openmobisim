//! `manhattan_grid` — the public, shared fixture (S105), promoted out of
//! this crate's own private test helper (`tests/common/mod.rs`) in Phase 1
//! step 8.

use openmobisim_core_graph::examples::{link_count, manhattan_grid, node_name};
use openmobisim_core_types::ids::NodeId;

#[test]
fn a_grid_has_the_counts_a_grid_should_have() {
    let (net, diag) = manhattan_grid(5, 200.0, false);
    assert_eq!(net.node_count(), 25);
    assert_eq!(net.link_count(), link_count(5));
    assert!(diag.is_empty(), "a clean synthetic grid must produce no diagnostics");

    let id = |r, c| net.node_external_ids().typed_id_of::<NodeId>(&node_name(r, c)).unwrap();
    assert_eq!(net.out_links(id(0, 0)).len(), 2, "a corner has two exits");
    assert_eq!(net.out_links(id(2, 2)).len(), 4, "an interior node has four exits");
}

#[test]
fn signals_mark_interior_nodes_only() {
    let (net, _) = manhattan_grid(4, 150.0, true);
    let interior = net.node_external_ids().typed_id_of::<NodeId>(&node_name(1, 1)).unwrap();
    let corner = net.node_external_ids().typed_id_of::<NodeId>(&node_name(0, 0)).unwrap();
    assert!(net.is_signalised(interior));
    assert!(!net.is_signalised(corner));
}

#[test]
fn block_metres_scales_link_length() {
    let (small, _) = manhattan_grid(3, 100.0, false);
    let (large, _) = manhattan_grid(3, 400.0, false);
    let link = |net: &openmobisim_core_graph::network::RoadNetwork| {
        let a = net.node_external_ids().typed_id_of::<NodeId>(&node_name(0, 0)).unwrap();
        net.out_links(a)[0]
    };
    let ratio = large.link_length(link(&large)).get() / small.link_length(link(&small)).get();
    assert!(
        (ratio - 4.0).abs() < 1e-4,
        "quadrupling block_metres must quadruple link length (ratio was {ratio})"
    );
}

#[test]
#[should_panic(expected = "at least 2×2")]
fn a_grid_needs_at_least_two_by_two_nodes() {
    let _ = manhattan_grid(1, 100.0, false);
}
