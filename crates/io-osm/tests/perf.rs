//! Performance floors for the import path.
//!
//! Import is build-time work, but it is also the first thing a new user waits
//! for, and the zero-data promise is "a working run within minutes" for the
//! whole pipeline. Contraction in particular is an iterative pass over a
//! hash-indexed graph, which is exactly the shape of thing that turns
//! accidentally quadratic without anyone noticing — a small test network would
//! never show it.
//!
//! Enforced in release only, for the reasons in `core-types`' `tests/perf.rs`.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    reason = "fixtures build coordinates and counts from small integers"
)]

use std::hint::black_box;
use std::time::{Duration, Instant};

use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_io_osm::import::{ImportOptions, import};
use openmobisim_io_osm::source::{MemorySource, OsmNode, OsmWay};

fn check(name: &str, iterations: u64, release_millis: u64, body: impl FnOnce()) {
    let start = Instant::now();
    body();
    let elapsed = start.elapsed();
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

/// A grid of streets, each edge mapped as its own OSM way with two interior
/// geometry points — which is roughly what real OSM looks like, and what makes
/// contraction have work to do.
fn grid_source(n: i64) -> MemorySource {
    let mut source = MemorySource::new();
    let id = |r: i64, c: i64| r * 10_000 + c;
    let mid = |r: i64, c: i64, k: i64| 50_000_000 + (r * 10_000 + c) * 10 + k;

    for r in 0..n {
        for c in 0..n {
            source = source.node(OsmNode::new(
                id(r, c),
                4.8 + c as f64 * 0.001,
                45.7 + r as f64 * 0.001,
            ));
        }
    }

    let mut way_id = 1i64;
    for r in 0..n {
        for c in 0..n {
            for (dr, dc, k) in [(0i64, 1i64, 0i64), (1, 0, 1)] {
                let (r2, c2) = (r + dr, c + dc);
                if r2 >= n || c2 >= n {
                    continue;
                }
                // Two interior points, so each edge exercises the polyline
                // length path as well as the split.
                let (a, b) = (mid(r, c, k * 2), mid(r, c, k * 2 + 1));
                let lon0 = 4.8 + c as f64 * 0.001;
                let lat0 = 45.7 + r as f64 * 0.001;
                let lon1 = 4.8 + c2 as f64 * 0.001;
                let lat1 = 45.7 + r2 as f64 * 0.001;
                source = source
                    .node(OsmNode::new(a, lon0 + (lon1 - lon0) / 3.0, lat0 + (lat1 - lat0) / 3.0))
                    .node(OsmNode::new(
                        b,
                        lon0 + 2.0 * (lon1 - lon0) / 3.0,
                        lat0 + 2.0 * (lat1 - lat0) / 3.0,
                    ));
                source = source.way(OsmWay::new(
                    way_id,
                    [id(r, c), a, b, id(r2, c2)],
                    [("highway", "secondary"), ("lanes", "2")],
                ));
                way_id += 1;
            }
        }
    }
    source
}

#[test]
fn import_throughput() {
    let source = grid_source(70);
    let ways = source.way_count() as u64;
    check("import (grid, contracted)", ways, 4_000, || {
        let mut diagnostics = Diagnostics::new();
        let (network, report, _) =
            import(&source, ImportOptions::default(), &mut diagnostics).expect("importable");
        assert!(diagnostics.is_empty(), "{:?}", diagnostics.rows());
        black_box((network.link_count(), report.links_after_contraction));
    });
}

#[test]
fn contraction_is_not_accidentally_quadratic() {
    // The pass repeats until nothing more merges. On a chain it converges
    // logarithmically; the risk is a shape where it does not. Doubling the work
    // must not much more than double the time.
    let small = grid_source(40);
    let large = grid_source(80);

    let time = |s: &MemorySource| {
        let start = Instant::now();
        let mut d = Diagnostics::new();
        let (_, report, _) = import(s, ImportOptions::default(), &mut d).expect("importable");
        (start.elapsed(), report.links_before_contraction)
    };

    let (t_small, n_small) = time(&small);
    let (t_large, n_large) = time(&large);

    let work_ratio = n_large as f64 / n_small as f64;
    let time_ratio = t_large.as_secs_f64() / t_small.as_secs_f64().max(1e-9);
    println!(
        "contraction scaling: {n_small} -> {n_large} links ({work_ratio:.1}x), \
         {t_small:?} -> {t_large:?} ({time_ratio:.1}x)"
    );

    if cfg!(debug_assertions) {
        return;
    }
    assert!(
        time_ratio < work_ratio * 3.0,
        "time grew {time_ratio:.1}x for {work_ratio:.1}x the work — that is superlinear enough to look quadratic"
    );
}

#[test]
fn contraction_removes_what_it_should_on_a_realistic_shape() {
    // Each grid edge is one way with two interior points. Splitting gives one
    // link per edge per direction already, so contraction's work here is the
    // interior geometry — and the node count is what proves it.
    let source = grid_source(30);
    let mut diagnostics = Diagnostics::new();
    let (network, report, _) =
        import(&source, ImportOptions::default(), &mut diagnostics).expect("importable");

    // 30x30 junctions, minus the four corners: a corner has degree two, so it
    // is a bend in a street rather than a junction, and contracting it is
    // correct. Interior geometry points never become network nodes — they
    // are kept instead in the S125 geometry artifact (`tests/geometry.rs`).
    assert_eq!(network.node_count(), 896);
    assert_eq!(report.nodes_kept, source.node_count() as u64);
    println!(
        "grid 30: {} nodes kept -> {} network nodes, {} links",
        report.nodes_kept,
        network.node_count(),
        network.link_count()
    );
}
