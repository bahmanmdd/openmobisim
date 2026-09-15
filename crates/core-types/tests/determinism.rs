//! The determinism guarantee, tested the way it can actually fail.
//!
//! A reduction that is non-deterministic does not fail on small inputs, does
//! not fail on one thread, and does not fail reproducibly. It fails when the
//! work-stealing split happens to differ between two runs. So these tests
//! attack exactly that: the same data reduced under thread pools of different
//! sizes, and reduced many times over.

// Exact float comparison is the assertion here, not an oversight: bit-identical
// results are precisely the property under test.
#![allow(clippy::float_cmp, reason = "bit-exact results are the property under test")]

use openmobisim_core_types::reduce::{
    REDUCTION_CHUNK, fixed_order_chunk_reduce, fixed_order_map, fixed_order_map_sum,
    fixed_order_sum,
};

/// Values chosen so that association order visibly matters: mixing magnitudes
/// several orders apart is what makes floating-point addition non-associative
/// in practice, not just in principle.
fn adversarial_values(n: u32) -> Vec<f64> {
    (0..n)
        .map(|i| {
            let x = f64::from(i);
            match i % 4 {
                0 => 1e12 + x,
                1 => 1e-12 * (x + 1.0),
                2 => -1e12 + x,
                _ => 1.0 / (x + 1.0),
            }
        })
        .collect()
}

#[test]
fn the_sum_is_stable_across_repeated_calls() {
    let xs = adversarial_values(1_000_000);
    let first = fixed_order_sum(&xs);
    for _ in 0..20 {
        assert_eq!(fixed_order_sum(&xs).to_bits(), first.to_bits());
    }
}

#[test]
fn the_sum_equals_its_defining_chunked_sequential_form() {
    // The contract is not "some deterministic number" but *this* number: the
    // chunked, in-order sum. Pinning it means a future optimisation cannot
    // quietly change results while still passing a self-consistency test.
    let xs = adversarial_values(100_000);
    let expected = xs
        .chunks(REDUCTION_CHUNK)
        .map(|c| c.iter().fold(0.0, |a, b| a + b))
        .fold(0.0, |a, b| a + b);
    assert_eq!(fixed_order_sum(&xs).to_bits(), expected.to_bits());
}

#[test]
#[cfg(feature = "parallel")]
fn the_sum_does_not_depend_on_the_thread_count() {
    // This is the test that would have caught `par_iter().sum()`.
    let xs = adversarial_values(500_000);
    let mut results = Vec::new();
    for threads in [1usize, 2, 3, 8] {
        let pool =
            rayon::ThreadPoolBuilder::new().num_threads(threads).build().expect("thread pool");
        results.push((threads, pool.install(|| fixed_order_sum(&xs))));
    }
    let (_, first) = results[0];
    for (threads, value) in &results {
        assert_eq!(
            value.to_bits(),
            first.to_bits(),
            "sum changed with {threads} threads: {value} vs {first}"
        );
    }
}

#[test]
#[cfg(feature = "parallel")]
fn map_sum_does_not_depend_on_the_thread_count() {
    let xs = adversarial_values(300_000);
    let mut results = Vec::new();
    for threads in [1usize, 4, 7] {
        let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
        results.push(pool.install(|| fixed_order_map_sum(&xs, |&x| x * 1.000_001)));
    }
    assert!(results.windows(2).all(|w| w[0].to_bits() == w[1].to_bits()));
}

#[test]
fn chunk_reduce_combines_in_index_order() {
    // A deliberately non-commutative combine: the result records the order the
    // chunks were folded in. Anything other than index order shows up here.
    let xs: Vec<u32> = (0..10_000u32).collect();
    let order = fixed_order_chunk_reduce(
        &xs,
        1_000,
        |chunk| vec![chunk[0]],
        Vec::new(),
        |mut a, b| {
            a.extend(b);
            a
        },
    );
    assert_eq!(order, vec![0, 1_000, 2_000, 3_000, 4_000, 5_000, 6_000, 7_000, 8_000, 9_000]);
}

#[test]
fn empty_and_short_inputs_behave() {
    assert_eq!(fixed_order_sum(&[]), 0.0);
    assert_eq!(fixed_order_sum(&[1.5]), 1.5);
    assert_eq!(fixed_order_map_sum::<f64, _>(&[], |&x| x), 0.0);
    assert!(fixed_order_map::<f64, f64, _>(&[], |&x| x).is_empty());
}

#[test]
fn map_preserves_index_order() {
    let xs: Vec<u32> = (0..50_000u32).collect();
    let doubled = fixed_order_map(&xs, |&x| x * 2);
    assert!(doubled.iter().enumerate().all(|(i, &v)| v as usize == i * 2));
}
