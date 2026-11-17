//! A study area cut out of an extract (version 1's clip, with its one behaviour): the nodes
//! inside a box or a polygon, each street's first node outside (a stub, so the segment crossing
//! the edge stays), and the motorways, trunk roads and their slip roads within
//! [`MAIN_ROAD_BUFFER_M`] of the area, their carriageways cut at the buffer's edge joined by a
//! turnaround. A way leaving the area and coming back is two pieces: nothing is bridged.
//!
//! A motorway or trunk carriageway cut at the buffer's edge would end in a sink (or start at a
//! source), and the car network's strong component would then take it back to its last
//! interchange (Delft's A13 and A4 vanished 2 km inside the area). So each such cut end that no
//! kept road leaves (or enters) is joined to the nearest cut end of the other kind within
//! [`TURNAROUND_MAX_M`], one with the same `ref` first, by a one-way slip road at 30 km/h: the
//! only roads the reader adds.

use std::collections::{HashMap, HashSet};
use std::fmt;

use crate::source::OsmWay;
use crate::tags::{Direction, RoadClass, direction};

/// A study area in WGS84 degrees.
#[derive(Clone, Debug, PartialEq)]
pub enum Region {
    /// A rectangle; its edges are inside.
    BBox {
        /// The western edge (longitude).
        west: f64,
        /// The southern edge (latitude).
        south: f64,
        /// The eastern edge.
        east: f64,
        /// The northern edge.
        north: f64,
    },
    /// A polygon of `(lon, lat)` vertices, either winding, closed implicitly; inside by the
    /// even–odd rule on the plane of degrees.
    Polygon(Vec<(f64, f64)>),
}

/// Why a region cannot be made, in words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegionError(pub String);

impl fmt::Display for RegionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl core::error::Error for RegionError {}

/// How far outside the area motorways, trunk roads and their slip roads are kept, metres.
pub const MAIN_ROAD_BUFFER_M: f64 = 2_000.0;

/// The farthest apart two cut ends of a main road's carriageways may be to be joined, metres: a
/// dual carriageway's two sides are tens of metres apart.
pub const TURNAROUND_MAX_M: f64 = 300.0;

/// The first way id of the turnarounds, counting up: far below any id an extract holds.
pub const TURNAROUND_FIRST_ID: i64 = i64::MIN / 2;

/// Metres per degree of latitude (the mean meridian degree; exact enough at a city's scale).
const M_PER_DEG_LAT: f64 = 111_320.0;

/// The `highway` values the buffer keeps.
const MAIN_ROADS: [&str; 4] = ["motorway", "motorway_link", "trunk", "trunk_link"];

impl Region {
    /// A rectangle: finite, longitudes in [−180, 180], latitudes in [−90, 90], `west < east`,
    /// `south < north`.
    ///
    /// # Errors
    ///
    /// A [`RegionError`] saying which of these fails (a box across the antimeridian included).
    pub fn bbox(west: f64, south: f64, east: f64, north: f64) -> Result<Self, RegionError> {
        let fail = |m: &str| Err(RegionError(m.to_owned()));
        if ![west, south, east, north].iter().all(|v| v.is_finite()) {
            return fail("a region's coordinates must be finite");
        }
        if !(-180.0..=180.0).contains(&west) || !(-180.0..=180.0).contains(&east) {
            return fail("longitudes must lie in [-180, 180]");
        }
        if !(-90.0..=90.0).contains(&south) || !(-90.0..=90.0).contains(&north) {
            return fail("latitudes must lie in [-90, 90]");
        }
        if west >= east || south >= north {
            return fail(
                "a box needs west < east and south < north (one across the antimeridian is not \
                 read)",
            );
        }
        Ok(Self::BBox { west, south, east, north })
    }

    /// A polygon of at least three `(lon, lat)` vertices, each finite and in range; a closing
    /// vertex equal to the first is dropped.
    ///
    /// # Errors
    ///
    /// A [`RegionError`]: fewer than three vertices, or one not finite or out of range.
    pub fn polygon(mut vertices: Vec<(f64, f64)>) -> Result<Self, RegionError> {
        if vertices.len() > 1 && vertices.first() == vertices.last() {
            vertices.pop();
        }
        if vertices.len() < 3 {
            return Err(RegionError("a polygon needs at least three vertices".into()));
        }
        let bad = |&(lon, lat): &(f64, f64)| {
            !(-180.0..=180.0).contains(&lon) || !(-90.0..=90.0).contains(&lat)
        };
        if vertices.iter().any(bad) {
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

    /// How far a point is from the area, metres: 0 inside, else the distance to its edge on a
    /// local plane at the point's latitude.
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
                    let (ex, ey) = ((v[i].0 - lon) * kx - ax, (v[i].1 - lat) * M_PER_DEG_LAT - ay);
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

    /// Whether a point outside lies within the main roads' buffer: a box test first, then the
    /// exact distance.
    fn near(&self, lon: f64, lat: f64) -> bool {
        let (w, s, e, n) = match self {
            Self::BBox { west, south, east, north } => (*west, *south, *east, *north),
            Self::Polygon(v) => v.iter().fold(
                (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY),
                |(w, s, e, n), &(x, y)| (w.min(x), s.min(y), e.max(x), n.max(y)),
            ),
        };
        let grow_lat = MAIN_ROAD_BUFFER_M / M_PER_DEG_LAT;
        let cos = s.abs().max(n.abs()).to_radians().cos().max(1e-6);
        let grow_lon = MAIN_ROAD_BUFFER_M / (M_PER_DEG_LAT * cos);
        (w - grow_lon..=e + grow_lon).contains(&lon)
            && (s - grow_lat..=n + grow_lat).contains(&lat)
            && self.distance_m(lon, lat) <= MAIN_ROAD_BUFFER_M
    }
}

/// What a region keeps: per way (in the order given), which of its nodes; and the turnarounds
/// it adds, as ways.
pub(crate) struct Clip {
    pub keep: Vec<Vec<bool>>,
    pub turnarounds: Vec<OsmWay>,
}

/// A one-way main road's end where the clip cut it: traffic leaves the network there (a sink)
/// or enters it (a source).
struct CutEnd {
    node: i64,
    sink: bool,
    motorway: bool,
    reference: Option<String>,
}

/// The region's clip of `ways`, whose nodes lie at `positions`.
pub(crate) fn clip(
    ways: &[&OsmWay],
    positions: &HashMap<i64, (f64, f64)>,
    region: &Region,
) -> Clip {
    let mut inside = HashSet::new();
    let mut near = HashMap::new();
    for (&id, &(lon, lat)) in positions {
        if region.contains(lon, lat) {
            inside.insert(id);
        } else if region.near(lon, lat) {
            near.insert(id, (lon, lat));
        }
    }
    let keep: Vec<Vec<bool>> = ways.iter().map(|w| kept(w, &inside, &near)).collect();
    let mut all = HashSet::new();
    let mut cuts = Vec::new();
    for (way, keep) in ways.iter().zip(&keep) {
        all.extend(way.nodes.iter().zip(keep).filter(|(_, k)| **k).map(|(n, _)| *n));
        cut_ends(way, keep, &mut cuts);
    }
    let turnarounds = turnarounds(ways, &cuts, &all, &near);
    Clip { keep, turnarounds }
}

/// Which of a way's nodes the clip keeps: those inside; a main road's within the buffer; and,
/// on any other way, the node just outside either end of each run inside (a stub).
fn kept(way: &OsmWay, inside: &HashSet<i64>, near: &HashMap<i64, (f64, f64)>) -> Vec<bool> {
    let main = way.tag("highway").is_some_and(|h| MAIN_ROADS.contains(&h));
    let base: Vec<bool> =
        way.nodes.iter().map(|n| inside.contains(n) || (main && near.contains_key(n))).collect();
    if main {
        return base;
    }
    let mut keep = base.clone();
    for k in (0..base.len()).filter(|&k| base[k]) {
        keep[k.saturating_sub(1)] = true;
        if k + 1 < base.len() {
            keep[k + 1] = true;
        }
    }
    keep
}

/// A one-way main road's cut ends: where a run of kept nodes stops before the way does (in
/// traffic order a sink), or starts after it began (a source).
fn cut_ends(way: &OsmWay, keep: &[bool], out: &mut Vec<CutEnd>) {
    let Some(highway) = way.tag("highway").filter(|h| MAIN_ROADS.contains(h)) else { return };
    let Some(class) = RoadClass::from_highway(highway) else { return };
    let forward = match direction(&way.tags, class) {
        Direction::Forward => true,
        Direction::Backward => false,
        Direction::Both => return,
    };
    let motorway = highway.starts_with("motorway");
    let reference = way.tag("ref").map(str::to_owned);
    let n = keep.len();
    let mut k = 0;
    while k < n {
        if !keep[k] {
            k += 1;
            continue;
        }
        let start = k;
        while k < n && keep[k] {
            k += 1;
        }
        let (cut_before, cut_after) = (start > 0, k < n);
        if k - start == 1 && cut_before && cut_after {
            continue; // one node kept mid-way: no road to join
        }
        let mut push = |node: i64, sink: bool| {
            out.push(CutEnd { node, sink, motorway, reference: reference.clone() });
        };
        if cut_after {
            push(way.nodes[k - 1], forward);
        }
        if cut_before {
            push(way.nodes[start], !forward);
        }
    }
}

/// The turnarounds: each cut end in the buffer that is still a dead end once every kept road is
/// in (no kept segment of any `highway` way leaves a sink, none enters a source), joined to the
/// nearest dead cut end of the other kind within [`TURNAROUND_MAX_M`], one with the same `ref`
/// first: a `motorway_link` (a `trunk_link` between two trunk roads), one-way, `maxspeed=30`.
fn turnarounds(
    ways: &[&OsmWay],
    cuts: &[CutEnd],
    all: &HashSet<i64>,
    near: &HashMap<i64, (f64, f64)>,
) -> Vec<OsmWay> {
    let candidates: HashSet<i64> = cuts.iter().map(|c| c.node).collect();
    // (leaving, entering) kept segments at each candidate.
    let mut degree: HashMap<i64, (u32, u32)> = HashMap::new();
    for way in ways.iter().filter(|w| w.nodes.iter().any(|n| candidates.contains(n))) {
        let Some(highway) = way.tag("highway") else { continue };
        let dir =
            RoadClass::from_highway(highway).map_or(Direction::Both, |c| direction(&way.tags, c));
        for pair in way.nodes.windows(2).filter(|p| all.contains(&p[0]) && all.contains(&p[1])) {
            let (a, b) = (pair[0], pair[1]);
            let steps: &[(i64, i64)] = match dir {
                Direction::Forward => &[(a, b)],
                Direction::Backward => &[(b, a)],
                Direction::Both => &[(a, b), (b, a)],
            };
            for &(from, to) in steps {
                if candidates.contains(&from) {
                    degree.entry(from).or_default().0 += 1;
                }
                if candidates.contains(&to) {
                    degree.entry(to).or_default().1 += 1;
                }
            }
        }
    }
    let dead = |c: &&CutEnd| {
        let (leaving, entering) = degree.get(&c.node).copied().unwrap_or((0, 0));
        near.contains_key(&c.node) && if c.sink { leaving == 0 } else { entering == 0 }
    };
    let (mut sinks, mut sources): (Vec<&CutEnd>, Vec<&CutEnd>) =
        cuts.iter().filter(dead).partition(|c| c.sink);
    for ends in [&mut sinks, &mut sources] {
        ends.sort_by_key(|c| c.node);
        ends.dedup_by_key(|c| c.node);
    }
    let mut out = Vec::new();
    for sink in sinks {
        let (x0, y0) = near[&sink.node];
        let m_per_deg_lon = M_PER_DEG_LAT * y0.to_radians().cos();
        let best = sources
            .iter()
            .filter(|s| s.node != sink.node)
            .map(|s| {
                let (x, y) = near[&s.node];
                let d = ((x - x0) * m_per_deg_lon).hypot((y - y0) * M_PER_DEG_LAT);
                (s, d, sink.reference.is_some() && sink.reference == s.reference)
            })
            .filter(|&(_, d, _)| d <= TURNAROUND_MAX_M)
            .min_by(|a, b| b.2.cmp(&a.2).then(a.1.total_cmp(&b.1)).then(a.0.node.cmp(&b.0.node)));
        if let Some((source, _, _)) = best {
            let highway =
                if sink.motorway || source.motorway { "motorway_link" } else { "trunk_link" };
            let id = TURNAROUND_FIRST_ID + i64::try_from(out.len()).expect("few turnarounds");
            let tags = [("highway", highway), ("oneway", "yes"), ("maxspeed", "30")];
            out.push(OsmWay::new(id, &[sink.node, source.node], &tags));
        }
    }
    out
}
