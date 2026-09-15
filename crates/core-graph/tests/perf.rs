//! Performance floors for the build path.
//!
//! Nothing here runs per simulation step — this is all build-time work — but a
//! build that takes minutes is a build a user stops doing, and the zero-data
//! start promises "a working run within minutes" for the *whole* pipeline, of
//! which this is one part. The budgets are loose, and enforced in release only,
//! for the reasons given in `core-types`' `tests/perf.rs`.

mod common;

use std::hint::black_box;
use std::time::{Duration, Instant};

use common::grid;
use openmobisim_core_graph::defaults::SignalDefaults;
use openmobisim_core_graph::geometry::{Hemisphere, LonLat, Projection, UtmZone};
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_types::ids::{EntityId, NodeId};

fn check(name: &str, iterations: u64, release_millis: u64, body: impl FnOnce()) {
    let start = Instant::now();
    body();
    let elapsed = start.elapsed();

    #[allow(clippy::cast_precision_loss, reason = "iteration counts are small")]
    let per_op_nanos = elapsed.as_secs_f64() * 1e9 / iterations as f64;

    if cfg!(debug_assertions) {
        println!(
            "{name}: {iterations} ops in {elapsed:?} ({per_op_nanos:.0} ns/op, debug — not enforced)"
        );
        return;
    }
    let budget = Duration::from_millis(release_millis);
    println!(
        "{name}: {iterations} ops in {elapsed:?} ({per_op_nanos:.0} ns/op, budget {budget:?})"
    );
    assert!(elapsed <= budget, "{name} took {elapsed:?}, over its {budget:?} budget");
}

#[test]
fn projection_throughput() {
    // Runs once per OSM node: a city extract is a few million of them.
    let proj = Projection::wgs84(UtmZone::new(31, Hemisphere::North));
    let n = 1_000_000u32;
    check("Projection::project", u64::from(n), 400, || {
        let mut acc = 0.0f64;
        for i in 0..n {
            let p = LonLat::new(4.8 + f64::from(i % 1000) * 1e-4, 45.7 + f64::from(i % 997) * 1e-4);
            acc += proj.project(black_box(p)).x;
        }
        black_box(acc);
    });
}

#[test]
fn network_build_throughput() {
    // A 150×150 grid is 22 500 nodes and 89 400 links — the order of a
    // medium city's contracted road network.
    let n = 150u32;
    check("RoadNetworkBuilder::build", u64::from(n * n), 3_000, || {
        let (net, diag) = grid(n, true);
        assert_eq!(net.node_count(), n * n);
        assert!(diag.is_empty());
        black_box(net.link_count());
    });
}

#[test]
fn turn_build_throughput() {
    let (net, _) = grid(150, true);
    check("TurnTable::build", u64::from(net.link_count()), 500, || {
        let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);
        black_box(turns.len());
    });
}

#[test]
fn adjacency_lookup_is_a_slice_not_a_search() {
    let (net, _) = grid(150, false);
    let nodes = net.node_count();
    check("RoadNetwork::out_links", u64::from(nodes) * 10, 100, || {
        let mut acc = 0usize;
        for _ in 0..10 {
            for node in NodeId::iter_space(nodes) {
                acc += net.out_links(node).len();
            }
        }
        black_box(acc);
    });
}

#[test]
fn a_city_scale_network_fits_the_memory_the_design_expects() {
    // Foundations §5 budgets the immutable inputs as shared and small next to
    // the per-run state. This pins the network's share of that claim.
    let (net, _) = grid(150, true);
    let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);

    let bytes = net.bytes() + turns.bytes();
    let per_link = bytes / net.link_count() as usize;
    println!(
        "network: {} nodes, {} links, {} turns, {:.1} MB ({per_link} B/link)",
        net.node_count(),
        net.link_count(),
        turns.len(),
        {
            #[allow(clippy::cast_precision_loss, reason = "byte counts are far below 2^52")]
            let mb = bytes as f64 / 1e6;
            mb
        }
    );
    assert!(
        per_link < 600,
        "the network costs {per_link} bytes per link, which is more than the design budgets"
    );
}
