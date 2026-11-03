//! Work split over threads in fixed chunks, so that no result depends on the thread count.
//!
//! Each function splits its items into chunks of a fixed number of items, in item order,
//! hands the chunks to the threads of the calling thread's pool (rayon's global pool unless
//! the caller installed one), and gives back the results in item order. Each item's result is
//! a pure function of the item and of what the closure reads, never of which thread computed
//! it or of what that thread computed before, so a run on one thread and a run on twelve give
//! the same results, to the bit. Sums over items are never taken here: the caller adds the
//! per-item results one after the other in item order, so the order of the additions (and
//! with it the rounding) is that of a loop on one thread.
//!
//! Work that fits one chunk runs on the calling thread: handing it to a pool costs more than it
//! saves. Without the `parallel` feature everything runs on the calling thread, with the same
//! results.

#[cfg(feature = "parallel")]
use rayon::prelude::*;

/// Run `work` on every item with a scratch made by `init` (at least once per chunk a thread
/// takes, so a large scratch wants large chunks), in chunks of `chunk` items; the results in
/// item order.
pub(crate) fn map<T, R, S, I, F>(items: &[T], chunk: usize, init: I, work: F) -> Vec<R>
where
    T: Sync,
    R: Send,
    I: Fn() -> S + Sync + Send,
    F: Fn(&mut S, &T) -> R + Sync + Send,
{
    #[cfg(not(feature = "parallel"))]
    let _ = chunk;
    #[cfg(feature = "parallel")]
    if items.len() > chunk {
        let chunks: Vec<Vec<R>> = items
            .par_chunks(chunk.max(1))
            .map_init(&init, |scratch, chunk| chunk.iter().map(|i| work(scratch, i)).collect())
            .collect();
        return chunks.into_iter().flatten().collect();
    }
    let mut scratch = init();
    items.iter().map(|i| work(&mut scratch, i)).collect()
}

/// Run `work` on every item, changing it in place, in chunks of `chunk` items.
pub(crate) fn for_each_mut<T, F>(items: &mut [T], chunk: usize, work: F)
where
    T: Send,
    F: Fn(&mut T) + Sync + Send,
{
    #[cfg(not(feature = "parallel"))]
    let _ = chunk;
    #[cfg(feature = "parallel")]
    if items.len() > chunk {
        items.par_chunks_mut(chunk.max(1)).for_each(|chunk| chunk.iter_mut().for_each(&work));
        return;
    }
    items.iter_mut().for_each(work);
}
