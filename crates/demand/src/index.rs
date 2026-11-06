//! The nearest node by straight line, without scanning them all: a uniform grid of cells over
//! the nodes a mode may use, searched in rings of cells around the point.
//!
//! # How
//!
//! [`NodeIndex::new`] keeps the nodes with an out-link or an in-link that allows the mode and
//! files them by cell: the cells are squares sized for about two nodes each over the nodes'
//! bounding box (at most about four million cells), the nodes of a cell in id order. A query
//! looks at the point's cell (the nearest cell of the grid when the point is outside it), then
//! at the ring of cells around those already seen, ring after ring, and stops as soon as the
//! best node found is strictly closer than any node outside the square seen so far can be: the
//! straight-line gap from the point to the nearest side of that square that has cells beyond
//! it. That gap is shrunk by a millionth of a cell so that the rounding of a node's cell cannot
//! end the search early; a stop that comes a ring later only looks at more cells.
//!
//! # Ties
//!
//! Two nodes at exactly the same distance: the lower node id. Because the search ends only when
//! the best is strictly closer than every node unseen, a node of lower id at the same distance
//! is never left unseen: the answer is the full scan's, whatever the grid.
//!
//! # Cost
//!
//! Building is one pass over the nodes and 4 + 16 bytes per node kept, plus 4 per cell; a
//! query looks at a few cells, whatever the network's size.

use openmobisim_graph::geometry::Xy;
use openmobisim_graph::network::Network;
use openmobisim_types::ids::{EntityId, LinkId, NodeId};
use openmobisim_types::mode::Mode;

/// The nodes a mode may use, filed by cell (see the module documentation).
#[derive(Clone, Debug)]
pub struct NodeIndex {
    /// The south-west corner of the grid, in metres.
    min: Xy,
    /// The side of a cell, in metres.
    cell: f64,
    cols: i64,
    rows: i64,
    /// `cols × rows + 1` offsets into `nodes`, cell by cell, row by row from the south.
    start: Vec<u32>,
    /// The nodes kept, grouped by cell, in id order within a cell.
    nodes: Vec<NodeId>,
    /// Their positions, parallel to `nodes`.
    xy: Vec<Xy>,
}

impl NodeIndex {
    /// The index of the nodes of `network` with an out-link or an in-link that allows `mode`.
    #[must_use]
    pub fn new(network: &Network, mode: Mode) -> Self {
        let allows = |links: &[LinkId]| links.iter().any(|&l| network.modes(l).contains(mode));
        let kept: Vec<NodeId> = NodeId::iter_space(network.node_count())
            .filter(|&n| allows(network.out_links(n)) || allows(network.in_links(n)))
            .collect();
        let xy: Vec<Xy> = kept.iter().map(|&n| network.node_position(n)).collect();
        Self::over(&kept, &xy)
    }

    /// File `nodes`, at `xy`, by cell.
    fn over(nodes: &[NodeId], xy: &[Xy]) -> Self {
        let mut min = Xy::new(f64::INFINITY, f64::INFINITY);
        let mut max = Xy::new(f64::NEG_INFINITY, f64::NEG_INFINITY);
        for p in xy {
            (min.x, min.y, max.x, max.y) =
                (min.x.min(p.x), min.y.min(p.y), max.x.max(p.x), max.y.max(p.y));
        }
        if xy.is_empty() {
            (min, max) = (Xy::default(), Xy::default());
        }
        let (w, h) = ((max.x - min.x).max(1.0), (max.y - min.y).max(1.0));
        #[allow(clippy::cast_precision_loss, reason = "a node count is far below 2^52")]
        let per_two = w * h / (xy.len().max(1) as f64 / 2.0);
        let cell = per_two.sqrt().max((w * h / 4.0e6).sqrt());
        #[allow(
            clippy::cast_possible_truncation,
            reason = "an extent over a cell: the cells number about four million at most"
        )]
        let (cols, rows) = ((w / cell) as i64 + 1, (h / cell) as i64 + 1);
        let mut index =
            Self { min, cell, cols, rows, start: Vec::new(), nodes: Vec::new(), xy: Vec::new() };
        let of: Vec<usize> =
            xy.iter().map(|&p| index.cell_index(index.col(p.x), index.row(p.y))).collect();
        let mut start = vec![0_u32; index.cell_count() + 1];
        for &c in &of {
            start[c + 1] += 1;
        }
        for c in 0..index.cell_count() {
            start[c + 1] += start[c];
        }
        let mut fill = start.clone();
        index.nodes = vec![NodeId::NULL; nodes.len()];
        index.xy = vec![Xy::default(); nodes.len()];
        // The nodes come in id order, so each cell receives its own in id order.
        for (k, &c) in of.iter().enumerate() {
            let at = fill[c] as usize;
            fill[c] += 1;
            (index.nodes[at], index.xy[at]) = (nodes[k], xy[k]);
        }
        index.start = start;
        index
    }

    /// Whether no node is kept: the mode is allowed on no link.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// The kept node nearest to `p` by straight line, ties to the lower id, with its distance in
    /// metres; `None` when no node is kept.
    #[must_use]
    pub fn nearest(&self, p: Xy) -> Option<(NodeId, f64)> {
        if self.is_empty() {
            return None;
        }
        let (cx, cy) = (self.col(p.x), self.row(p.y));
        let mut best: Option<(f64, NodeId)> = None;
        for ring in 0.. {
            for y in (cy - ring).max(0)..=(cy + ring).min(self.rows - 1) {
                let edge = (y - cy).abs() == ring;
                let step = if edge { 1 } else { 2 * ring.max(1) };
                let mut x = cx - ring;
                while x <= cx + ring {
                    if (0..self.cols).contains(&x) {
                        self.look(self.cell_index(x, y), p, &mut best);
                    }
                    x += step;
                }
            }
            let outside = self.outside(p, (cx, cy), ring);
            if best.is_some_and(|(d, _)| d < outside) || outside.is_infinite() {
                break;
            }
        }
        best.map(|(d, n)| (n, d))
    }

    /// Offer every node of cell `c` as the nearest to `p`.
    fn look(&self, c: usize, p: Xy, best: &mut Option<(f64, NodeId)>) {
        for i in self.start[c] as usize..self.start[c + 1] as usize {
            let (d, n) = (p.distance_to(self.xy[i]), self.nodes[i]);
            if best.is_none_or(|(bd, bn)| d.total_cmp(&bd).then(n.cmp(&bn)).is_lt()) {
                *best = Some((d, n));
            }
        }
    }

    /// A lower bound on the distance from `p` to any node outside the square of cells within
    /// `ring` of `(cx, cy)`: the gap to the nearest of its sides with cells beyond, shrunk by a
    /// millionth of a cell; infinite when the square covers the grid.
    #[allow(clippy::cast_precision_loss, reason = "cell indices are far below 2^52")]
    fn outside(&self, p: Xy, (cx, cy): (i64, i64), ring: i64) -> f64 {
        let edge = |i: i64, min: f64| min + i as f64 * self.cell;
        let mut gap = f64::INFINITY;
        if cx - ring > 0 {
            gap = gap.min(p.x - edge(cx - ring, self.min.x));
        }
        if cx + ring < self.cols - 1 {
            gap = gap.min(edge(cx + ring + 1, self.min.x) - p.x);
        }
        if cy - ring > 0 {
            gap = gap.min(p.y - edge(cy - ring, self.min.y));
        }
        if cy + ring < self.rows - 1 {
            gap = gap.min(edge(cy + ring + 1, self.min.y) - p.y);
        }
        gap - self.cell * 1e-6
    }

    /// The column of `x`, clamped to the grid.
    fn col(&self, x: f64) -> i64 {
        Self::clamped((x - self.min.x) / self.cell, self.cols)
    }

    /// The row of `y`, clamped to the grid.
    fn row(&self, y: f64) -> i64 {
        Self::clamped((y - self.min.y) / self.cell, self.rows)
    }

    /// `v` floored and clamped to `0..n`.
    #[allow(clippy::cast_precision_loss, reason = "n is far below 2^52")]
    #[allow(clippy::cast_possible_truncation, reason = "clamped to 0..n first")]
    fn clamped(v: f64, n: i64) -> i64 {
        v.floor().clamp(0.0, (n - 1) as f64) as i64
    }

    /// The cell at column `x`, row `y`, both in the grid.
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation, reason = "in the grid")]
    fn cell_index(&self, x: i64, y: i64) -> usize {
        (y * self.cols + x) as usize
    }

    /// How many cells.
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation, reason = "positive")]
    fn cell_count(&self) -> usize {
        (self.cols * self.rows) as usize
    }
}
