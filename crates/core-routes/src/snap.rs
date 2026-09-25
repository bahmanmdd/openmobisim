//! Snapping a point to the nearest node a car can use, without scanning them all.
//!
//! The placeholder this replaces scanned every node for every trip, twice: on a
//! 150 000-node extract with 19 000 trips that is thousands of millions of
//! distance computations, and most of a run's time. A [`NodeSnapper`] is a
//! uniform grid over the drivable nodes, built once; a query looks at rings of
//! cells around the point and stops as soon as no unseen cell can be closer.
//!
//! **Cost:** build is one pass over the nodes (12 bytes per drivable node plus
//! the grid); a query looks at a handful of cells, independent of network size.
//! **Determinism:** ties in distance are broken by node id.

use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_types::ids::{EntityId, LinkId, NodeId};

/// A grid index over the network's drivable nodes (S153: a node with at least
/// one link that carries motor traffic; if there is none, every node).
#[derive(Clone, Debug)]
pub struct NodeSnapper {
    min_x: f64,
    min_y: f64,
    cell: f64,
    cols: usize,
    rows: usize,
    /// `cols * rows + 1` offsets into `nodes`.
    start: Vec<u32>,
    /// Node ids, grouped by cell in id order.
    nodes: Vec<u32>,
    /// Every indexed node's position, parallel to `nodes`.
    positions: Vec<(f64, f64)>,
}

fn drivable(network: &RoadNetwork, link: LinkId) -> bool {
    network.link_class(link).carries_motor_traffic()
}

impl NodeSnapper {
    /// Index `network`'s nodes.
    #[must_use]
    pub fn new(network: &RoadNetwork) -> Self {
        let mut ids: Vec<u32> = (0..network.node_count())
            .filter(|&n| {
                let node = NodeId::new(n);
                network.out_links(node).iter().any(|&l| drivable(network, l))
                    || network.in_links(node).iter().any(|&l| drivable(network, l))
            })
            .collect();
        if ids.is_empty() {
            ids = (0..network.node_count()).collect();
        }
        Self::over(network, &ids)
    }

    /// Index every node of `network` that has a link: a bike or walk layer's
    /// graph (S195), whose every link is usable by its mode.
    #[must_use]
    pub fn every_node(network: &RoadNetwork) -> Self {
        let ids: Vec<u32> = (0..network.node_count())
            .filter(|&n| {
                let node = NodeId::new(n);
                !network.out_links(node).is_empty() || !network.in_links(node).is_empty()
            })
            .collect();
        Self::over(network, &ids)
    }

    fn over(network: &RoadNetwork, ids: &[u32]) -> Self {
        let xy: Vec<(f64, f64)> = ids
            .iter()
            .map(|&n| {
                let p = network.node_position(NodeId::new(n));
                (p.x, p.y)
            })
            .collect();
        let (mut min_x, mut min_y) = (f64::INFINITY, f64::INFINITY);
        let (mut max_x, mut max_y) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
        for &(x, y) in &xy {
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
        let (w, h) = ((max_x - min_x).max(1.0), (max_y - min_y).max(1.0));
        // About two nodes per cell, but never a cell under 50 m or a grid of more than ~4 M cells.
        #[allow(clippy::cast_precision_loss, reason = "a node count is far below 2^52")]
        let per_cell = (w * h / (xy.len().max(1) as f64 / 2.0)).sqrt();
        let cell = per_cell.clamp(50.0, 5_000.0).max((w * h / 4.0e6).sqrt());
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a non-negative extent over a positive cell size, bounded by ~4 million cells"
        )]
        let (cols, rows) = (((w / cell) as usize + 1), ((h / cell) as usize + 1));

        let cell_of = |x: f64, y: f64| -> usize {
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "inside the extent, so non-negative and below cols/rows"
            )]
            let (cx, cy) = (
                (((x - min_x) / cell) as usize).min(cols - 1),
                (((y - min_y) / cell) as usize).min(rows - 1),
            );
            cy * cols + cx
        };
        let mut count = vec![0u32; cols * rows + 1];
        for &(x, y) in &xy {
            count[cell_of(x, y) + 1] += 1;
        }
        for i in 1..count.len() {
            count[i] += count[i - 1];
        }
        let start = count.clone();
        let mut fill = count;
        let mut nodes = vec![0u32; ids.len()];
        let mut positions = vec![(0.0, 0.0); ids.len()];
        for (k, &(x, y)) in xy.iter().enumerate() {
            let c = cell_of(x, y);
            let at = fill[c] as usize;
            fill[c] += 1;
            nodes[at] = ids[k];
            positions[at] = (x, y);
        }
        Self { min_x, min_y, cell, cols, rows, start, nodes, positions }
    }

    /// The indexed node closest to `point`, ties broken by node id.
    ///
    /// # Panics
    ///
    /// Panics if the network had no nodes, which a built network never has.
    #[must_use]
    pub fn nearest(&self, network: &RoadNetwork, point: LonLat) -> NodeId {
        let p = network.projection().project(point);
        self.nearest_xy(p.x, p.y)
    }

    /// [`Self::nearest`] for a point already in the network's projection (metres).
    ///
    /// # Panics
    ///
    /// Panics if the network had no nodes.
    #[must_use]
    pub fn nearest_xy(&self, x: f64, y: f64) -> NodeId {
        let best = self.nearest_in_rings(x, y).unwrap_or_else(|| self.nearest_scan(x, y));
        NodeId::new(best)
    }

    fn nearest_scan(&self, x: f64, y: f64) -> u32 {
        (0..self.nodes.len())
            .min_by(|&a, &b| self.better(x, y, a, b))
            .map(|i| self.nodes[i])
            .expect("a network has at least one node")
    }

    fn better(&self, x: f64, y: f64, a: usize, b: usize) -> core::cmp::Ordering {
        let d = |i: usize| (self.positions[i].0 - x).hypot(self.positions[i].1 - y);
        d(a).total_cmp(&d(b)).then_with(|| self.nodes[a].cmp(&self.nodes[b]))
    }

    fn nearest_in_rings(&self, x: f64, y: f64) -> Option<u32> {
        // Outside the grid the ring bound below does not hold; scan instead.
        #[allow(clippy::cast_precision_loss, reason = "a grid dimension is far below 2^52")]
        let (gx, gy) =
            (self.min_x + self.cell * self.cols as f64, self.min_y + self.cell * self.rows as f64);
        if x < self.min_x || y < self.min_y || x >= gx || y >= gy {
            return None;
        }
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_possible_wrap,
            reason = "inside the grid, so a small non-negative cell index"
        )]
        let (cx, cy) =
            (((x - self.min_x) / self.cell) as i64, ((y - self.min_y) / self.cell) as i64);
        let (cols, rows) = (self.cols as i64, self.rows as i64);
        let mut best: Option<usize> = None;
        let max_ring = cols.max(rows);
        for ring in 0..=max_ring {
            for gy_ in (cy - ring).max(0)..=(cy + ring).min(rows - 1) {
                for gx_ in (cx - ring).max(0)..=(cx + ring).min(cols - 1) {
                    // Only the ring's own cells: the border of the square.
                    if (gx_ - cx).abs().max((gy_ - cy).abs()) != ring {
                        continue;
                    }
                    #[allow(
                        clippy::cast_sign_loss,
                        clippy::cast_possible_truncation,
                        reason = "clamped to the grid above, so a small non-negative index"
                    )]
                    let c = (gy_ * cols + gx_) as usize;
                    for i in self.start[c] as usize..self.start[c + 1] as usize {
                        if best.is_none_or(|b| self.better(x, y, i, b).is_lt()) {
                            best = Some(i);
                        }
                    }
                }
            }
            if let Some(b) = best {
                let d = (self.positions[b].0 - x).hypot(self.positions[b].1 - y);
                #[allow(clippy::cast_precision_loss, reason = "a small ring count")]
                if d <= ring as f64 * self.cell {
                    break;
                }
            }
        }
        best.map(|i| self.nodes[i])
    }

    /// How many nodes are indexed.
    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether nothing is indexed (never true for a built network).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}
