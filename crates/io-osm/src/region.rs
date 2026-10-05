//! Cutting a study area out of a larger extract.
//!
//! A city's extract is usually bigger than the network a study wants — a
//! congestion-charge cordon, a district, a bounding box around a corridor. A
//! [`Region`] is that area, as a rectangle or a polygon, and a
//! [`ClippedSource`] presents an [`OsmSource`] as if the extract held only what
//! lies inside it.
//!
//! # What "inside" does to a road
//!
//! A node is kept if it lies inside the region, and dropped otherwise. A way is
//! cut into **maximal runs of consecutive kept nodes**, and each run of two or
//! more becomes a way of its own, with the original's tags. Consequences worth
//! knowing:
//!
//! * A road crossing the boundary is cut at its last node inside. It ends
//!   there as a dead end, which is what a cordon's edge is.
//! * A road that leaves and comes back in is **two roads**, not one with a
//!   straight link across the gap. Bridging the gap would invent a street
//!   through whatever is outside.
//! * Nothing is added: no node is placed on the boundary, and no road is
//!   moved. The network is a subset of the extract's.
//!
//! Where a one-way road crosses the boundary its cut end becomes a source or a
//! sink, which is what [`Connectivity::Strong`](crate::import::Connectivity)
//! then removes, and reports.
//!
//! # Keeping what can be kept (S230, roadmap I-ac)
//!
//! [`ClipOptions`] can keep more than the nodes inside:
//!
//! * **Stubs:** a way crossing the boundary keeps its first node outside, so the segment that
//!   crosses the edge stays (a two-way road ends just past the edge instead of at its last node
//!   inside, and a road that leaves and comes back within one segment stays connected).
//! * **A buffer for main roads:** motorway and trunk ways and their slip roads keep their nodes
//!   within [`ClipOptions::main_road_buffer_m`] of the region, so a ring road just outside a
//!   study area (Amsterdam's A10, Paris' Périphérique) and the main approaches stay, as people
//!   use them for trips inside the area. Nothing else outside is kept.

use std::cell::RefCell;
use std::collections::HashSet;
use std::fmt;

use crate::source::{OsmError, OsmNode, OsmSource, OsmWay};

/// A study area.
#[derive(Clone, Debug, PartialEq)]
pub enum Region {
    /// A rectangle in degrees; the edges are inside.
    BBox {
        /// The western edge.
        west: f64,
        /// The southern edge.
        south: f64,
        /// The eastern edge.
        east: f64,
        /// The northern edge.
        north: f64,
    },
    /// A simple polygon of `(lon, lat)` vertices, in either winding, closed
    /// implicitly. Membership is the even–odd rule on the plane of degrees,
    /// which is exact enough at the scale of a city.
    Polygon(Vec<(f64, f64)>),
}

/// Why a region could not be made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegionError(String);

impl fmt::Display for RegionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for RegionError {}

/// What a [`ClippedSource`] keeps besides the nodes inside its region (S230). The default keeps
/// nothing else: the plain clip.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ClipOptions {
    /// Keep each way's first node outside next to a run inside: the crossing segment.
    pub stubs: bool,
    /// Keep the nodes of motorway, trunk and their slip-road ways within this many metres of the
    /// region (0: none).
    pub main_road_buffer_m: f64,
}

/// Metres per degree of latitude (the mean meridian degree; exact enough at a city's scale).
const M_PER_DEG_LAT: f64 = 111_320.0;

/// The `highway` values a main-road buffer keeps.
const MAIN_ROADS: [&str; 4] = ["motorway", "motorway_link", "trunk", "trunk_link"];

impl Region {
    /// How far a point is from the region, in metres: 0 inside, otherwise the distance to its
    /// boundary on a local plane at the point's latitude (S230).
    #[must_use]
    pub fn distance_m(&self, lon: f64, lat: f64) -> f64 {
        if self.contains(lon, lat) {
            return 0.0;
        }
        let kx = M_PER_DEG_LAT * lat.to_radians().cos();
        match self {
            Self::BBox { west, south, east, north } => {
                let dx = (west - lon).max(lon - east).max(0.0) * kx;
                let dy = (south - lat).max(lat - north).max(0.0) * M_PER_DEG_LAT;
                dx.hypot(dy)
            }
            Self::Polygon(v) => {
                let mut best = f64::INFINITY;
                let mut j = v.len() - 1;
                for i in 0..v.len() {
                    let (ax, ay) = ((v[j].0 - lon) * kx, (v[j].1 - lat) * M_PER_DEG_LAT);
                    let (bx, by) = ((v[i].0 - lon) * kx, (v[i].1 - lat) * M_PER_DEG_LAT);
                    let (ex, ey) = (bx - ax, by - ay);
                    let len2 = ex * ex + ey * ey;
                    let t = if len2 > 0.0 {
                        (-(ax * ex + ay * ey) / len2).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    best = best.min((ax + t * ex).hypot(ay + t * ey));
                    j = i;
                }
                best
            }
        }
    }

    /// The region's bounding box, `(west, south, east, north)`.
    fn bounds(&self) -> (f64, f64, f64, f64) {
        match self {
            Self::BBox { west, south, east, north } => (*west, *south, *east, *north),
            Self::Polygon(v) => v.iter().fold(
                (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY),
                |(w, s, e, n), &(x, y)| (w.min(x), s.min(y), e.max(x), n.max(y)),
            ),
        }
    }

    /// A rectangle, `west < east` and `south < north`, all finite, within the
    /// valid ranges of longitude and latitude.
    ///
    /// # Errors
    ///
    /// [`RegionError`] if a value is not finite, an edge is out of range, or the
    /// rectangle is empty. A region spanning the antimeridian is not supported.
    pub fn bbox(west: f64, south: f64, east: f64, north: f64) -> Result<Self, RegionError> {
        if ![west, south, east, north].iter().all(|v| v.is_finite()) {
            return Err(RegionError("a region's coordinates must be finite".into()));
        }
        if !(-180.0..=180.0).contains(&west) || !(-180.0..=180.0).contains(&east) {
            return Err(RegionError("longitudes must lie in [-180, 180]".into()));
        }
        if !(-90.0..=90.0).contains(&south) || !(-90.0..=90.0).contains(&north) {
            return Err(RegionError("latitudes must lie in [-90, 90]".into()));
        }
        if west >= east || south >= north {
            return Err(RegionError(
                "a bounding box needs west < east and south < north (one across the \
                 antimeridian is not supported)"
                    .into(),
            ));
        }
        Ok(Self::BBox { west, south, east, north })
    }

    /// A polygon of at least three distinct `(lon, lat)` vertices. A repeated
    /// closing vertex is accepted and dropped.
    ///
    /// # Errors
    ///
    /// [`RegionError`] if a coordinate is not finite or out of range, or fewer
    /// than three distinct vertices remain.
    pub fn polygon(mut vertices: Vec<(f64, f64)>) -> Result<Self, RegionError> {
        if vertices.len() > 1 && vertices.first() == vertices.last() {
            vertices.pop();
        }
        if vertices.len() < 3 {
            return Err(RegionError("a polygon needs at least three vertices".into()));
        }
        if vertices.iter().any(|&(lon, lat)| {
            !lon.is_finite()
                || !lat.is_finite()
                || !(-180.0..=180.0).contains(&lon)
                || !(-90.0..=90.0).contains(&lat)
        }) {
            return Err(RegionError(
                "polygon vertices must be finite (lon, lat) in degrees".into(),
            ));
        }
        Ok(Self::Polygon(vertices))
    }

    /// Whether a point lies inside.
    #[must_use]
    pub fn contains(&self, lon: f64, lat: f64) -> bool {
        match self {
            Self::BBox { west, south, east, north } => {
                (*west..=*east).contains(&lon) && (*south..=*north).contains(&lat)
            }
            Self::Polygon(v) => {
                let mut inside = false;
                let mut j = v.len() - 1;
                for i in 0..v.len() {
                    let ((xi, yi), (xj, yj)) = (v[i], v[j]);
                    if (yi > lat) != (yj > lat) && lon < (xj - xi) * (lat - yi) / (yj - yi) + xi {
                        inside = !inside;
                    }
                    j = i;
                }
                inside
            }
        }
    }
}

/// An [`OsmSource`] restricted to a [`Region`], keeping what its [`ClipOptions`] say besides.
///
/// Costs one extra pass over the nodes, the first time ways are asked for (and with options, one
/// over the ways), and holds the ids of the nodes kept while the import runs.
pub struct ClippedSource<'a> {
    inner: &'a dyn OsmSource,
    region: &'a Region,
    options: ClipOptions,
    /// The nodes inside, and those kept besides (stubs, buffered main roads).
    kept: RefCell<Option<Kept>>,
}

/// What a clip keeps.
struct Kept {
    inside: HashSet<i64>,
    /// Nodes outside within the main-road buffer.
    near: HashSet<i64>,
    /// Every node some way keeps.
    all: HashSet<i64>,
}

impl<'a> ClippedSource<'a> {
    /// `inner`, seen through `region`.
    #[must_use]
    pub fn new(inner: &'a dyn OsmSource, region: &'a Region) -> Self {
        Self::with_options(inner, region, ClipOptions::default())
    }

    /// `inner`, seen through `region`, keeping what `options` say besides (S230).
    #[must_use]
    pub fn with_options(
        inner: &'a dyn OsmSource,
        region: &'a Region,
        options: ClipOptions,
    ) -> Self {
        Self { inner, region, options, kept: RefCell::new(None) }
    }

    fn ensure_kept(&self) -> Result<(), OsmError> {
        if self.kept.borrow().is_some() {
            return Ok(());
        }
        let buffer = self.options.main_road_buffer_m.max(0.0);
        // A cheap box test before the exact distance: the region's box grown by the buffer.
        let (w, s, e, n) = self.region.bounds();
        let grow_lat = buffer / M_PER_DEG_LAT;
        let grow_lon = buffer / (M_PER_DEG_LAT * s.abs().max(n.abs()).to_radians().cos().max(1e-6));
        let (mut inside, mut near) = (HashSet::new(), HashSet::new());
        self.inner.for_each_node(&mut |node| {
            if self.region.contains(node.lon, node.lat) {
                inside.insert(node.id);
            } else if buffer > 0.0
                && node.lon >= w - grow_lon
                && node.lon <= e + grow_lon
                && node.lat >= s - grow_lat
                && node.lat <= n + grow_lat
                && self.region.distance_m(node.lon, node.lat) <= buffer
            {
                near.insert(node.id);
            }
        })?;
        let mut all = inside.clone();
        if self.options.stubs || buffer > 0.0 {
            self.inner.for_each_way(&mut |way| {
                for (k, keep) in self.positions(&way, &inside, &near).into_iter().enumerate() {
                    if keep {
                        all.insert(way.node_ids[k]);
                    }
                }
            })?;
        }
        *self.kept.borrow_mut() = Some(Kept { inside, near, all });
        Ok(())
    }

    /// Which of a way's nodes it keeps: those inside; a main road's within the buffer; and, with
    /// stubs, the node just outside either end of every run so far.
    fn positions(&self, way: &OsmWay, inside: &HashSet<i64>, near: &HashSet<i64>) -> Vec<bool> {
        let main = self.options.main_road_buffer_m > 0.0
            && way.tag("highway").is_some_and(|h| MAIN_ROADS.contains(&h));
        let mut keep: Vec<bool> =
            way.node_ids.iter().map(|n| inside.contains(n) || (main && near.contains(n))).collect();
        if self.options.stubs {
            let base = keep.clone();
            for k in 0..base.len() {
                if base[k] {
                    if k > 0 {
                        keep[k - 1] = true;
                    }
                    if k + 1 < base.len() {
                        keep[k + 1] = true;
                    }
                }
            }
        }
        keep
    }
}

impl OsmSource for ClippedSource<'_> {
    fn for_each_way(&self, visit: &mut dyn FnMut(OsmWay)) -> Result<(), OsmError> {
        self.ensure_kept()?;
        let guard = self.kept.borrow();
        let kept = guard.as_ref().expect("computed just above");
        self.inner.for_each_way(&mut |way| {
            let keep = self.positions(&way, &kept.inside, &kept.near);
            // The common cases first: a way wholly kept, and one with fewer than two nodes kept.
            let count = keep.iter().filter(|k| **k).count();
            if count == keep.len() {
                visit(way);
                return;
            }
            if count < 2 {
                return;
            }
            let ids = &way.node_ids;
            let mut start: Option<usize> = None;
            let mut runs: Vec<(usize, usize)> = Vec::new();
            for (k, &kept_here) in keep.iter().enumerate() {
                match (kept_here, start) {
                    (true, None) => start = Some(k),
                    (false, Some(s)) => {
                        runs.push((s, k));
                        start = None;
                    }
                    _ => {}
                }
            }
            if let Some(s) = start {
                runs.push((s, ids.len()));
            }
            for (s, e) in runs {
                if e - s >= 2 {
                    visit(OsmWay {
                        id: way.id,
                        node_ids: ids[s..e].to_vec(),
                        tags: way.tags.clone(),
                    });
                }
            }
        })
    }

    fn for_each_node(&self, visit: &mut dyn FnMut(OsmNode)) -> Result<(), OsmError> {
        self.ensure_kept()?;
        let guard = self.kept.borrow();
        let kept = guard.as_ref().expect("computed just above");
        self.inner.for_each_node(&mut |node| {
            if kept.all.contains(&node.id) {
                visit(node);
            }
        })
    }
}
