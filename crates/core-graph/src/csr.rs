//! Compressed sparse row adjacency — the shape every structure in this crate
//! is built from.
//!
//! # Why CSR and not a vector of vectors
//!
//! A `Vec<Vec<LinkId>>` for node-to-link adjacency costs one allocation per
//! node, one pointer chase per lookup, and on a city-scale network it scatters
//! adjacency across the heap in whatever order the allocator felt like. CSR is
//! two flat arrays: neighbours of `i` are a contiguous slice, the whole
//! structure is two allocations, and it maps to a file without translation —
//! which is what makes the route store memory-mappable (S12).
//!
//! # Deterministic by construction
//!
//! Edges are sorted by `(source, target)` when the structure is built, never
//! left in input order and never in hash-map order. Two builds of the same
//! network therefore produce byte-identical adjacency, which is a precondition
//! for the artifact fingerprint meaning anything (S65) — and, less obviously,
//! for sweeps over adjacency to visit in the same order every run, which is
//! what the determinism guarantee rests on (S60).

use core::marker::PhantomData;

use openmobisim_core_types::ids::EntityId;

/// A typed compressed-sparse-row adjacency from `S` to `T`.
///
/// The type parameters carry no data — they exist so that a node-to-link
/// adjacency cannot be indexed with a link id by accident, which is a mistake
/// that otherwise compiles and produces plausible nonsense.
///
/// # Examples
///
/// ```
/// use openmobisim_core_graph::csr::Csr;
/// use openmobisim_core_types::ids::{EntityId, LinkId, NodeId};
///
/// // Three nodes; node 0 has two outgoing links, node 2 has one.
/// let csr: Csr<NodeId, LinkId> = Csr::from_pairs(
///     3,
///     [
///         (NodeId::new(0), LinkId::new(7)),
///         (NodeId::new(2), LinkId::new(1)),
///         (NodeId::new(0), LinkId::new(3)),
///     ],
/// );
///
/// // Targets come back sorted, whatever order they went in.
/// assert_eq!(csr.targets(NodeId::new(0)), &[LinkId::new(3), LinkId::new(7)]);
/// assert!(csr.targets(NodeId::new(1)).is_empty());
/// assert_eq!(csr.degree(NodeId::new(2)), 1);
/// assert_eq!(csr.len(), 3);
/// assert_eq!(csr.edge_count(), 3);
/// ```
#[derive(Clone, Debug)]
pub struct Csr<S, T> {
    /// `len() + 1` entries; `offsets[i]..offsets[i + 1]` brackets source `i`.
    offsets: Vec<u32>,
    /// Targets, grouped by source and sorted within each group.
    targets: Vec<T>,
    _source: PhantomData<fn() -> S>,
}

impl<S: EntityId, T: EntityId> Csr<S, T> {
    /// Build from `(source, target)` pairs, in any order.
    ///
    /// `source_count` fixes the source id space, so a source with no targets
    /// still has an entry — a network has dead-end nodes, and they must be
    /// addressable.
    ///
    /// # Panics
    ///
    /// Panics if any source id is null or beyond `source_count`, or if the
    /// edge count exceeds the `u32` offset space.
    #[must_use]
    pub fn from_pairs(source_count: u32, pairs: impl IntoIterator<Item = (S, T)>) -> Self {
        let mut edges: Vec<(u32, T)> = pairs
            .into_iter()
            .map(|(s, t)| {
                assert!(!s.is_null(), "CSR source id must not be the null sentinel");
                assert!(
                    s.raw() < source_count,
                    "CSR source id {} is beyond the declared source count {source_count}",
                    s.raw()
                );
                (s.raw(), t)
            })
            .collect();

        assert!(
            u32::try_from(edges.len()).is_ok(),
            "CSR edge count {} exceeds the u32 offset space",
            edges.len()
        );

        // Sorted by (source, target): the determinism guarantee of this module.
        edges.sort_unstable_by_key(|&(s, t)| (s, t.raw()));

        let mut offsets = Vec::with_capacity(source_count as usize + 1);
        let mut targets = Vec::with_capacity(edges.len());
        let mut cursor = 0usize;
        for source in 0..source_count {
            #[allow(
                clippy::cast_possible_truncation,
                reason = "the edge count was asserted to fit u32 above"
            )]
            offsets.push(cursor as u32);
            while cursor < edges.len() && edges[cursor].0 == source {
                targets.push(edges[cursor].1);
                cursor += 1;
            }
        }
        #[allow(
            clippy::cast_possible_truncation,
            reason = "the edge count was asserted to fit u32 above"
        )]
        offsets.push(targets.len() as u32);

        Self { offsets, targets, _source: PhantomData }
    }

    /// An adjacency with `source_count` sources and no edges.
    #[must_use]
    pub fn empty(source_count: u32) -> Self {
        Self {
            offsets: vec![0; source_count as usize + 1],
            targets: Vec::new(),
            _source: PhantomData,
        }
    }

    /// The targets of `source`, sorted.
    ///
    /// # Panics
    ///
    /// Panics if `source` is null or out of range.
    #[inline]
    #[must_use]
    pub fn targets(&self, source: S) -> &[T] {
        let i = source.index();
        let start = self.offsets[i] as usize;
        let end = self.offsets[i + 1] as usize;
        &self.targets[start..end]
    }

    /// How many targets `source` has.
    #[inline]
    #[must_use]
    pub fn degree(&self, source: S) -> usize {
        let i = source.index();
        (self.offsets[i + 1] - self.offsets[i]) as usize
    }

    /// How many sources the adjacency covers.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.offsets.len() - 1
    }

    /// Whether there are no sources at all.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// How many edges the adjacency holds.
    #[inline]
    #[must_use]
    pub fn edge_count(&self) -> usize {
        self.targets.len()
    }

    /// The largest degree of any source.
    ///
    /// Reported at build time: it sizes the node model's per-node scratch
    /// buffers, and an outlier usually means a data problem.
    #[must_use]
    pub fn max_degree(&self) -> usize {
        self.offsets.windows(2).map(|w| (w[1] - w[0]) as usize).max().unwrap_or(0)
    }

    /// Every `(source, target)` pair, in sorted order.
    pub fn iter(&self) -> impl Iterator<Item = (S, T)> + '_ {
        (0..self.len()).flat_map(move |i| {
            let source = S::from_index(i);
            self.targets(source).iter().map(move |&t| (source, t))
        })
    }

    /// Bytes held, for the memory figures in the manifest.
    #[must_use]
    pub fn bytes(&self) -> usize {
        self.offsets.len() * size_of::<u32>() + self.targets.len() * size_of::<T>()
    }
}
