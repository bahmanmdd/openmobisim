//! Minimal performance floors.
//!
//! These are **not** benchmarks. They are regression tripwires for the handful
//! of operations that sit inside per-link, per-step or per-traveller loops,
//! where an accidental allocation, a hidden `HashMap` probe or a lost `#[inline]`
//! turns a 2 ns operation into a 200 ns one and nothing else in the suite
//! notices. Death by a thousand small costs is a named risk for this project;
//! this file is the cheapest possible defence against it.
//!
//! The budgets are deliberately loose — roughly an order of magnitude above
//! what the operations actually cost — so that a shared CI runner having a bad
//! day does not turn into a red build. A failure here means something changed
//! by a factor of ten, not by twenty percent. Measure properly with a
//! benchmark before drawing any conclusion from the numbers printed below.

use std::hint::black_box;
use std::time::{Duration, Instant};

use openmobisim_core_types::diagnostics::{
    Category, DiagKey, Diagnostics, ElementRef, Severity, codes,
};
use openmobisim_core_types::ids::{EntityId, ExternalIdTableBuilder, LinkId};
use openmobisim_core_types::reduce::fixed_order_sum;
use openmobisim_core_types::rng::{DrawAddress, RngKey, Stream, StreamRng};
use openmobisim_core_types::time::{Second, StepGrid};

/// Budgets are enforced in release builds only.
///
/// A debug build has no inlining, a bounds check on every index and a
/// debug-assert in every id constructor, and the ratio to release is neither
/// constant nor small — ChaCha8 alone is two orders of magnitude slower. A
/// budget wide enough not to flake in debug would be too wide to catch
/// anything in release. So debug runs still execute every path and print every
/// number, and CI enforces the floors with `cargo test --release`.
fn budget(release_millis: u64) -> Duration {
    Duration::from_millis(release_millis)
}

/// Time `body`, print the rate, and — in a release build — fail if it took
/// longer than `budget`.
fn check(name: &str, iterations: u64, budget: Duration, body: impl FnOnce()) {
    let start = Instant::now();
    body();
    let elapsed = start.elapsed();

    #[allow(clippy::cast_precision_loss, reason = "iteration counts are far below 2^53")]
    let per_op_nanos = elapsed.as_secs_f64() * 1e9 / iterations as f64;

    if cfg!(debug_assertions) {
        println!(
            "{name}: {iterations} ops in {elapsed:?} ({per_op_nanos:.1} ns/op, debug — not enforced)"
        );
        return;
    }

    println!(
        "{name}: {iterations} ops in {elapsed:?} ({per_op_nanos:.1} ns/op, budget {budget:?})"
    );
    assert!(
        elapsed <= budget,
        "{name} took {elapsed:?}, over its {budget:?} budget \
         ({per_op_nanos:.1} ns/op) — something on this path got much more expensive"
    );
}

#[test]
fn reduction_throughput() {
    let n = 4_000_000u32;
    let xs: Vec<f64> = (0..n).map(|i| f64::from(i) * 0.5).collect();
    check("fixed_order_sum", u64::from(n), budget(200), || {
        black_box(fixed_order_sum(black_box(&xs)));
    });
}

#[test]
fn step_lookup_throughput() {
    // `step_of` runs for every vehicle, every event and every curve read. It
    // is the reason `FastDivU32` exists, so it is the one worth pinning.
    let grid = StepGrid::new(Second::ZERO, 300, 4 * 3600).unwrap();
    let n = 10_000_000u32;
    check("StepGrid::step_of", u64::from(n), budget(100), || {
        let mut acc = 0u64;
        for t in 0..n {
            acc += u64::from(black_box(grid).step_of(Second(t)).get());
        }
        black_box(acc);
    });
}

#[test]
fn rng_block_throughput() {
    // One ChaCha8 block per call, so this is the cost of a traveller's set of
    // choice draws. If it ever becomes the bottleneck, the fix is to pull more
    // values from each block — not to weaken the generator.
    let rng = StreamRng::new(RngKey::from_seed(1), Stream::Choice);
    let n = 500_000u32;
    // Budget widened 120ms -> 200ms (240 -> 400 ns/op) after a CI flake on a
    // busy macos-latest runner (344 ns/op measured, against the real cost of
    // 36-44 ns/op from S110). 400 ns/op is an order of magnitude above that
    // measured cost, matching this file's own stated margin rather than the
    // ~6x the old budget gave this one test.
    check("StreamRng::block", u64::from(n), budget(200), || {
        let mut acc = 0f64;
        for i in 0..n {
            acc += rng.block(DrawAddress::from_quad(i, 0, 3, 5)).unit(0);
        }
        black_box(acc);
    });
}

#[test]
fn external_id_lookup_throughput() {
    // Build-time only, but it runs once per OSM way and once per GTFS stop, so
    // a linear scan hiding in here would show up as a slow import.
    let n = 100_000u32;
    let mut builder = ExternalIdTableBuilder::with_capacity(n as usize);
    for i in 0..n {
        builder.insert(format!("way/{i:08}"));
    }
    let table = builder.build();

    let probes: Vec<String> = (0..n).step_by(7).map(|i| format!("way/{i:08}")).collect();
    check("ExternalIdTable::id_of", probes.len() as u64, budget(60), || {
        let mut found = 0u32;
        for p in &probes {
            found += u32::from(table.id_of(p).is_some());
        }
        assert_eq!(found as usize, probes.len());
    });
}

#[test]
fn diagnostics_recording_is_cheap_on_the_exception_path() {
    let n = 200_000u32;
    check("Diagnostics::record", u64::from(n), budget(120), || {
        let mut diag = Diagnostics::new();
        for i in 0..n {
            diag.record(DiagKey::new(
                Category::Modelling,
                codes::NO_FEASIBLE_PATH,
                Severity::Warning,
                ElementRef::of(LinkId::new(i % 2_000)),
            ));
        }
        assert_eq!(diag.total(), u64::from(n));
    });
}

#[test]
fn an_unused_diagnostics_report_allocates_nothing() {
    // "A feature that is off must cost nothing, not merely little." A report
    // that recorded nothing must not have touched the allocator; `HashMap::new`
    // is the reason this holds, and this test is the reason it keeps holding.
    let diag = Diagnostics::new();
    assert_eq!(size_of_val(&diag), size_of::<Diagnostics>());
    assert!(diag.is_empty());
    assert_eq!(diag.rows().capacity(), 0, "an empty report must not allocate its rows either");
}

#[test]
fn memory_footprints_match_the_design_estimates() {
    // Foundations §5 states per-run memory in formulas. These assertions pin
    // the per-entry sizes those formulas are built on, so that an estimate in
    // the design document and the code cannot drift apart silently.
    let links = 50_000usize;
    let steps = 48usize; // four hours at 300 s

    // Cumulative curves: links x steps x 2 (up and down) x 8 bytes.
    let curve_bytes = links * steps * 2 * size_of::<f64>();
    assert_eq!(curve_bytes, 38_400_000, "the ~40 MB curve figure in Foundations §5");

    // A vehicle in flight: id, link, path position — four u32s at most.
    assert!(size_of::<LinkId>() * 4 <= 16, "a vehicle record must stay within 16 bytes");
}
