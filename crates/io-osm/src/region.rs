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

impl Region {
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

/// An [`OsmSource`] restricted to a [`Region`].
///
/// Costs one extra pass over the nodes, the first time ways are asked for, and
/// holds the ids of the nodes inside while the import runs.
pub struct ClippedSource<'a> {
    inner: &'a dyn OsmSource,
    region: &'a Region,
    inside: RefCell<Option<HashSet<i64>>>,
}

impl<'a> ClippedSource<'a> {
    /// `inner`, seen through `region`.
    #[must_use]
    pub fn new(inner: &'a dyn OsmSource, region: &'a Region) -> Self {
        Self { inner, region, inside: RefCell::new(None) }
    }

    fn ensure_inside(&self) -> Result<(), OsmError> {
        if self.inside.borrow().is_some() {
            return Ok(());
        }
        let mut ids = HashSet::new();
        self.inner.for_each_node(&mut |node| {
            if self.region.contains(node.lon, node.lat) {
                ids.insert(node.id);
            }
        })?;
        *self.inside.borrow_mut() = Some(ids);
        Ok(())
    }
}

impl OsmSource for ClippedSource<'_> {
    fn for_each_way(&self, visit: &mut dyn FnMut(OsmWay)) -> Result<(), OsmError> {
        self.ensure_inside()?;
        let guard = self.inside.borrow();
        let inside = guard.as_ref().expect("computed just above");
        self.inner.for_each_way(&mut |way| {
            let ids = &way.node_ids;
            // The common cases first: a way wholly inside, and one wholly outside.
            let kept = ids.iter().filter(|n| inside.contains(n)).count();
            if kept == ids.len() {
                visit(way);
                return;
            }
            if kept < 2 {
                return;
            }
            let mut start: Option<usize> = None;
            let mut runs: Vec<(usize, usize)> = Vec::new();
            for (k, n) in ids.iter().enumerate() {
                match (inside.contains(n), start) {
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
        self.inner.for_each_node(&mut |node| {
            if self.region.contains(node.lon, node.lat) {
                visit(node);
            }
        })
    }
}
