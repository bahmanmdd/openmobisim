//! Fixed-order parallel reductions: the same result whatever the number of threads.
//!
//! `xs.par_iter().sum::<f64>()` is not deterministic: rayon splits the work by how threads
//! happen to steal it, floating-point addition is not associative, and two runs on the same
//! input can differ in the last bits, silently. Here the slice is cut into chunks of a fixed
//! length ([`REDUCTION_CHUNK`]) that depends on neither the data, the thread count nor the
//! machine; each chunk is reduced sequentially, in parallel with the others; and the chunk
//! results are combined in index order. The result is a pure function of the input.
//!
//! ```
//! use openmobisim_types::reduce::fixed_order_sum;
//!
//! let xs: Vec<f64> = (0..100_000).map(|i| 1.0 / f64::from(i + 1)).collect();
//! assert_eq!(fixed_order_sum(&xs).to_bits(), fixed_order_sum(&xs).to_bits());
//! ```
//!
//! Never write a bare `par_iter().sum()`, `.reduce()` or `.fold()` over floating-point values;
//! if this module lacks the shape needed, it is added here.

use rayon::prelude::*;

/// The reduction chunk length, in elements.
///
/// Fixed, and independent of the data, the thread count and the machine. 4 096 `f64` values are
/// 32 KiB. Changing it changes results in the last bits: it is part of the numerical contract.
pub const REDUCTION_CHUNK: usize = 4096;

/// The sum of a slice of `f64`, in fixed order.
#[must_use]
pub fn fixed_order_sum(xs: &[f64]) -> f64 {
    fixed_order_map_sum(xs, |&x| x)
}

/// The sum of `f` over a slice, in fixed order, without building the mapped array.
///
/// # Examples
///
/// ```
/// use openmobisim_types::reduce::fixed_order_map_sum;
///
/// let pairs = [(1.0f64, 2.0f64), (3.0, 4.0)];
/// assert_eq!(fixed_order_map_sum(&pairs, |&(a, b)| a * b), 14.0);
/// ```
#[must_use]
pub fn fixed_order_map_sum<T, F>(xs: &[T], f: F) -> f64
where
    T: Sync,
    F: Fn(&T) -> f64 + Sync,
{
    fixed_order_chunk_reduce(
        xs,
        REDUCTION_CHUNK,
        |chunk| chunk.iter().map(&f).fold(0.0, |a, b| a + b),
        0.0,
        |a, b| a + b,
    )
}

/// Reduce a slice in chunks of `chunk_len`, combining the chunk results in index order.
///
/// The general form behind [`fixed_order_sum`], for an accumulator that is not one `f64` (a
/// histogram, a pair of moments).
///
/// Contract: `fold_chunk` is a pure function of its chunk; `empty` is an identity of `combine`;
/// `combine` is a pure function of its two arguments (it need not be commutative: it is applied
/// in index order only). Breaking any of these brings back the non-determinism this removes.
///
/// # Examples
///
/// ```
/// use openmobisim_types::reduce::{fixed_order_chunk_reduce, REDUCTION_CHUNK};
///
/// let xs: Vec<f64> = (0..10_000).map(f64::from).collect();
/// let (n, total) = fixed_order_chunk_reduce(
///     &xs,
///     REDUCTION_CHUNK,
///     |c| (c.len(), c.iter().fold(0.0, |a, b| a + b)),
///     (0, 0.0),
///     |a, b| (a.0 + b.0, a.1 + b.1),
/// );
/// assert_eq!(n, 10_000);
/// assert_eq!(total, 49_995_000.0); // n (n - 1) / 2
/// ```
///
/// # Panics
///
/// If `chunk_len` is zero.
#[must_use]
pub fn fixed_order_chunk_reduce<T, A, FF, FC>(
    xs: &[T],
    chunk_len: usize,
    fold_chunk: FF,
    empty: A,
    combine: FC,
) -> A
where
    T: Sync,
    A: Send,
    FF: Fn(&[T]) -> A + Sync,
    FC: Fn(A, A) -> A,
{
    assert!(chunk_len > 0, "the reduction chunk length must be non-zero");
    // The partials are computed in parallel and collected in index order; only where each is
    // computed depends on the threads.
    let partials: Vec<A> = xs.par_chunks(chunk_len).map(&fold_chunk).collect();
    partials.into_iter().fold(empty, &combine)
}

/// Map a slice into a new `Vec`, in parallel, in index order.
///
/// Order-preserving by construction, so safe with any mapping function.
///
/// # Examples
///
/// ```
/// use openmobisim_types::reduce::fixed_order_map;
///
/// assert_eq!(fixed_order_map(&[1.0f64, 2.0, 3.0], |x| x * 2.0), vec![2.0, 4.0, 6.0]);
/// ```
#[must_use]
pub fn fixed_order_map<T, A, F>(xs: &[T], f: F) -> Vec<A>
where
    T: Sync,
    A: Send,
    F: Fn(&T) -> A + Sync,
{
    xs.par_iter().map(&f).collect()
}
