//! Fixed-order reductions: equal to their sequential definition, and identical at 1 and 8
//! threads and over two calls.
//!
//! A non-deterministic reduction fails neither on small inputs nor on one thread; it fails
//! when the work-stealing split differs. So the same data is reduced under pools of different
//! sizes, on values chosen so that the order of addition visibly matters.

use openmobisim_types::reduce::{
    REDUCTION_CHUNK, fixed_order_chunk_reduce, fixed_order_map, fixed_order_map_sum,
    fixed_order_sum,
};

/// Magnitudes 24 orders apart, so that floating-point addition is far from associative.
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

fn in_pool<R: Send>(threads: usize, f: impl FnOnce() -> R + Send) -> R {
    rayon::ThreadPoolBuilder::new().num_threads(threads).build().expect("a thread pool").install(f)
}

#[test]
fn the_sum_equals_its_chunked_sequential_definition() {
    let xs = adversarial_values(100_003);
    let definition = xs
        .chunks(REDUCTION_CHUNK)
        .map(|c| c.iter().fold(0.0, |a, b| a + b))
        .fold(0.0, |a: f64, b| a + b);
    assert_eq!(fixed_order_sum(&xs).to_bits(), definition.to_bits());
    // The order matters on these values: a different association gives a different sum, so
    // the equality above is not an accident of exact arithmetic.
    let reversed = xs.iter().rev().fold(0.0, |a, b| a + b);
    assert_ne!(reversed.to_bits(), definition.to_bits());
}

#[test]
fn the_sum_is_identical_at_one_and_eight_threads_and_over_two_calls() {
    let xs = adversarial_values(500_000);
    let one = in_pool(1, || fixed_order_sum(&xs));
    let eight = in_pool(8, || fixed_order_sum(&xs));
    let eight_again = in_pool(8, || fixed_order_sum(&xs));
    assert_eq!(one.to_bits(), eight.to_bits(), "{one} at 1 thread, {eight} at 8");
    assert_eq!(eight.to_bits(), eight_again.to_bits());
}

#[test]
fn the_map_sum_is_identical_at_one_and_eight_threads() {
    let xs = adversarial_values(300_000);
    let f = |&x: &f64| x * 1.000_001;
    let one = in_pool(1, || fixed_order_map_sum(&xs, f));
    let eight = in_pool(8, || fixed_order_map_sum(&xs, f));
    assert_eq!(one.to_bits(), eight.to_bits());
}

#[test]
fn chunk_results_combine_in_index_order() {
    // A non-commutative combine records the order the chunks were folded in.
    let xs: Vec<u32> = (0..10_000u32).collect();
    let order = in_pool(8, || {
        fixed_order_chunk_reduce(
            &xs,
            1_000,
            |c| vec![c[0]],
            Vec::new(),
            |mut a, b| {
                a.extend(b);
                a
            },
        )
    });
    assert_eq!(order, (0..10).map(|k| k * 1_000).collect::<Vec<u32>>());
}

#[test]
fn the_chunk_reduce_counts_and_sums_by_hand() {
    // 0 + 1 + … + 9 999 = 9 999 × 10 000 / 2 = 49 995 000, exact in f64.
    let xs: Vec<f64> = (0..10_000).map(f64::from).collect();
    let (n, total) = fixed_order_chunk_reduce(
        &xs,
        REDUCTION_CHUNK,
        |c| (c.len(), c.iter().fold(0.0, |a, b| a + b)),
        (0, 0.0),
        |a, b| (a.0 + b.0, a.1 + b.1),
    );
    assert_eq!(n, 10_000);
    assert_eq!(total.to_bits(), 49_995_000.0f64.to_bits());
}

#[test]
fn empty_and_single_inputs_give_the_identity_and_the_value() {
    assert_eq!(fixed_order_sum(&[]).to_bits(), 0.0f64.to_bits());
    assert_eq!(fixed_order_sum(&[1.5]).to_bits(), 1.5f64.to_bits());
    assert!(fixed_order_map::<f64, f64, _>(&[], |&x| x).is_empty());
}

#[test]
fn the_map_preserves_index_order_at_eight_threads() {
    let xs: Vec<u32> = (0..50_000u32).collect();
    let doubled = in_pool(8, || fixed_order_map(&xs, |&x| u64::from(x) * 2));
    assert!(doubled.iter().zip(0u64..).all(|(&v, i)| v == 2 * i));
}
