//! Deterministic parallel reductions (Foundations §3).
//!
//! # The hazard this module exists to remove
//!
//! `xs.par_iter().sum::<f64>()` is **not deterministic**. Rayon splits work
//! according to how many threads happen to steal it, floating-point addition
//! is not associative, and so two runs of the same binary on the same machine
//! with the same input can produce different sums in the last bits.
//!
//! That would be tolerable if it announced itself. It does not. It silently
//! destroys common random numbers across designs, makes the convergence gap
//! wander below its own sampling floor, and makes two evaluations of the same
//! design differ — all without a single error message. It is the single most
//! likely thing to be quietly wrong in a first implementation, which is why
//! the run-twice bit-identity check is a CI gate from Phase 1.
//!
//! # The fix
//!
//! Partition the slice into **fixed-size chunks that do not depend on the
//! data, the thread count or the machine**; reduce each chunk sequentially;
//! combine the chunk results **in index order**. The result is then a pure
//! function of the input, and identical whether or not the `parallel` feature
//! is enabled — which is exactly what `tests/determinism.rs` asserts.
//!
//! ```
//! use openmobisim_core_types::reduce::fixed_order_sum;
//!
//! let xs: Vec<f64> = (0..100_000).map(|i| 1.0 / f64::from(i + 1)).collect();
//! assert_eq!(fixed_order_sum(&xs), fixed_order_sum(&xs)); // and across runs
//! ```
//!
//! # The rule for contributors
//!
//! **Never write a bare `par_iter().sum()`, `par_iter().reduce()` or
//! `par_iter().fold()` over floating-point state.** If this module does not
//! have the shape you need, add it here rather than working around it. This is
//! one of the two rules `CONTRIBUTING.md` states outright; the other is that
//! sweeps are Jacobi.

#[cfg(feature = "parallel")]
use rayon::prelude::*;

/// The reduction chunk length.
///
/// Fixed, and deliberately independent of the data, the thread count and the
/// machine: those are precisely the things that must not influence the result.
/// 4096 `f64`s is 32 KiB, which sits inside L1 on every target we build for,
/// and is large enough that per-chunk overhead disappears.
///
/// **Changing this value changes results in the last bits.** It is part of the
/// numerical contract, not a tuning knob.
pub const REDUCTION_CHUNK: usize = 4096;

/// Sum a slice of `f64`, deterministically.
///
/// Use this wherever a total is taken over a large array: link flows, travel
/// times, gap terms.
#[must_use]
pub fn fixed_order_sum(xs: &[f64]) -> f64 {
    fixed_order_map_sum(xs, |&x| x)
}

/// Map each element to an `f64` and sum, deterministically.
///
/// The one-pass form of `map(...).sum()`, without materialising the mapped
/// array.
///
/// # Examples
///
/// ```
/// use openmobisim_core_types::reduce::fixed_order_map_sum;
///
/// let pairs = [(1.0f64, 2.0f64), (3.0, 4.0)];
/// let total = fixed_order_map_sum(&pairs, |&(a, b)| a * b);
/// assert_eq!(total, 14.0);
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

/// Reduce a slice in fixed-size chunks, combining the chunk results in index
/// order.
///
/// The general form behind [`fixed_order_sum`]. Use it when the per-chunk
/// accumulator is not a single `f64` — a histogram, a pair of moments, a
/// bounding box.
///
/// # Contract
///
/// * `fold_chunk` must be a pure function of the chunk it is given.
/// * `empty` must be an identity for `combine`: `combine(empty, x) == x`.
/// * `combine` need not be commutative — it is only ever applied in index
///   order — but it must be a pure function of its two arguments.
///
/// Breaking any of these silently reintroduces exactly the non-determinism
/// this module exists to prevent.
///
/// # Examples
///
/// An online mean and count, reduced deterministically:
///
/// ```
/// use openmobisim_core_types::reduce::{fixed_order_chunk_reduce, REDUCTION_CHUNK};
///
/// let xs: Vec<f64> = (0..10_000).map(f64::from).collect();
/// let (n, total) = fixed_order_chunk_reduce(
///     &xs,
///     REDUCTION_CHUNK,
///     |c| (c.len() as u64, c.iter().fold(0.0, |a, b| a + b)),
///     (0, 0.0),
///     |a, b| (a.0 + b.0, a.1 + b.1),
/// );
/// assert_eq!(n, 10_000);
/// assert_eq!(total, 49_995_000.0);
/// ```
///
/// # Panics
///
/// Panics if `chunk_len` is zero.
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
    assert!(chunk_len > 0, "reduction chunk length must be non-zero");

    if xs.is_empty() {
        return empty;
    }

    // Both branches produce the *same partials in the same order*; only where
    // each partial is computed differs. That is the whole trick.
    #[cfg(feature = "parallel")]
    let partials: Vec<A> = xs.par_chunks(chunk_len).map(&fold_chunk).collect();

    #[cfg(not(feature = "parallel"))]
    let partials: Vec<A> = xs.chunks(chunk_len).map(&fold_chunk).collect();

    partials.into_iter().fold(empty, &combine)
}

/// Map a slice into a new `Vec` in index order, in parallel.
///
/// Order-preserving by construction, so unlike a reduction this is safe with
/// any mapping function. It is here rather than at each call site so that
/// `rayon` stays an implementation detail of this crate.
///
/// # Examples
///
/// ```
/// use openmobisim_core_types::reduce::fixed_order_map;
///
/// let xs = [1.0f64, 2.0, 3.0];
/// assert_eq!(fixed_order_map(&xs, |x| x * 2.0), vec![2.0, 4.0, 6.0]);
/// ```
#[must_use]
pub fn fixed_order_map<T, A, F>(xs: &[T], f: F) -> Vec<A>
where
    T: Sync,
    A: Send,
    F: Fn(&T) -> A + Sync,
{
    #[cfg(feature = "parallel")]
    {
        xs.par_iter().map(&f).collect()
    }
    #[cfg(not(feature = "parallel"))]
    {
        xs.iter().map(&f).collect()
    }
}

/// Whether this build reduces in parallel.
///
/// Reported in `manifest.json`. It must not change any result — that is the
/// point — but a reader comparing two manifests deserves to know.
#[must_use]
pub const fn is_parallel_build() -> bool {
    cfg!(feature = "parallel")
}
