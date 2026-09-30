//! Parkings from OpenStreetMap (M4, S201): park-and-ride car parks and bike
//! parkings, as a parking table ([`ParkingRow`]) a run makes hubs of.
//!
//! **What is read** (the zero-calibration default of D11; a user's own table
//! replaces it):
//!
//! | Kind | Tags |
//! |---|---|
//! | car | `amenity=parking` that is a park-and-ride: `park_ride` present and not `no`, or a name that says so (`P+R`, `P&R`, `park and ride`, `transferium`) |
//! | bike | `amenity=bicycle_parking` |
//!
//! From nodes and from ways (an area's centroid; an open way's middle), inside
//! the region if one is given. `access=private` or `access=no` is left out.
//!
//! **Capacity:** the `capacity` tag where it holds a number; else, for an area,
//! its ground area divided by the area one vehicle takes; else a default. The
//! three options are uncalibrated defaults ([`ParkingReadOptions`]); the report
//! says how many parkings took each.
//!
//! **Duplicates:** a node inside the bounds of an area of the same kind is the
//! same parking mapped twice (a node for the entrance, a way for the lot) and is
//! left out.
//!
//! **Sites:** parkings of one kind within [`ParkingReadOptions::merge_m`] of each
//! other are merged into one, their capacities added, at their
//! capacity-weighted centre, under the id of the largest. Street racks a few
//! metres apart are one place to a cyclist, and a station's several garages one
//! choice, so the traveller's candidates are places, not racks.
//!
//! **Cost:** one pass over the ways and one over the nodes; memory for the
//! parkings and their ways' nodes only.

use std::collections::HashMap;

use openmobisim_core_graph::geometry::{LonLat, ground_distance_metres};
use openmobisim_core_graph::hubs::{ParkingKind, ParkingRow};

use crate::region::Region;
use crate::source::{OsmError, OsmSource, tag_of};

/// The reader's options: uncalibrated defaults, each overridable (S202).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParkingReadOptions {
    /// Parkings of one kind closer than this, in metres, are one site.
    ///
    /// *An assumption: about a street block's width.*
    pub merge_m: f64,
    /// The capacity of a car park mapped as a point without a `capacity` tag.
    ///
    /// *An assumption. CITATION OWED.*
    pub capacity_car: u32,
    /// The capacity of a bike parking mapped as a point without a `capacity`
    /// tag: most are street racks.
    ///
    /// *An assumption. CITATION OWED.*
    pub capacity_bike: u32,
    /// The ground area one car takes in a car park, aisles included, in m².
    ///
    /// *CITATION OWED: design guides give 25–30 m² per space for surface car
    /// parks.*
    pub area_per_car_m2: f64,
    /// The ground area one bike takes in a bike parking, aisles included, in m².
    ///
    /// *CITATION OWED: single-tier racks take about 1.2–2 m² per bike.*
    pub area_per_bike_m2: f64,
}

impl ParkingReadOptions {
    /// The shipped values.
    pub const SHIPPED: ParkingReadOptions = ParkingReadOptions {
        merge_m: 100.0,
        capacity_car: 100,
        capacity_bike: 10,
        area_per_car_m2: 25.0,
        area_per_bike_m2: 1.5,
    };
}

impl Default for ParkingReadOptions {
    fn default() -> Self {
        Self::SHIPPED
    }
}

/// What the reader found and did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ParkingReadReport {
    /// Park-and-ride car parks found (before merging).
    pub car_found: u32,
    /// Bike parkings found (before merging).
    pub bike_found: u32,
    /// Left out: a node mapped inside an area of the same kind.
    pub duplicates_dropped: u32,
    /// Left out: `access=private` or `access=no`.
    pub private_dropped: u32,
    /// Capacity from the `capacity` tag.
    pub capacity_tagged: u32,
    /// Capacity from the area.
    pub capacity_from_area: u32,
    /// Capacity from the default.
    pub capacity_default: u32,
    /// Car sites after merging.
    pub sites_car: u32,
    /// Bike sites after merging.
    pub sites_bike: u32,
}

/// Whether an `amenity=parking` element is a park-and-ride, by its tags.
#[must_use]
pub fn is_park_and_ride(tags: &[(String, String)]) -> bool {
    if tag_of(tags, "park_ride").is_some_and(|v| v != "no") {
        return true;
    }
    tag_of(tags, "name").is_some_and(|name| {
        let n: String = name.to_lowercase().chars().filter(|c| !c.is_whitespace()).collect();
        n.contains("p+r")
            || n.contains("p&r")
            || n.contains("parkandride")
            || n.contains("park&ride")
            || n.contains("transferium")
    })
}

/// The parking kind an element's tags say, if it is one this reader keeps.
#[must_use]
pub fn parking_kind(tags: &[(String, String)]) -> Option<ParkingKind> {
    match tag_of(tags, "amenity")? {
        "parking" if is_park_and_ride(tags) => Some(ParkingKind::Car),
        "bicycle_parking" => Some(ParkingKind::Bike),
        _ => None,
    }
}

/// The number a `capacity` tag holds: its first run of digits, if more than 0.
#[must_use]
pub fn capacity_tag(tags: &[(String, String)]) -> Option<u32> {
    let v = tag_of(tags, "capacity")?;
    let digits: String =
        v.chars().skip_while(|c| !c.is_ascii_digit()).take_while(char::is_ascii_digit).collect();
    digits.parse::<u32>().ok().filter(|&c| c > 0)
}

fn is_private(tags: &[(String, String)]) -> bool {
    matches!(tag_of(tags, "access"), Some("private" | "no"))
}

/// Where a parking's capacity came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CapacityFrom {
    Tag,
    Area,
    Default,
}

/// One parking as found, before merging.
#[derive(Clone, Debug)]
struct Found {
    id: String,
    name: Option<String>,
    kind: ParkingKind,
    position: LonLat,
    capacity: u32,
    capacity_from: CapacityFrom,
    /// The bounds of its area, `(west, south, east, north)`, for a way.
    bounds: Option<(f64, f64, f64, f64)>,
    from_node: bool,
    private: bool,
}

/// The ground area of a closed ring of points, in m² (the shoelace formula on a
/// local plane: exact enough for a car park).
fn ring_area_m2(ring: &[LonLat]) -> f64 {
    if ring.len() < 4 {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss, reason = "the vertices of one car park")]
    let lat0 = ring.iter().map(|p| p.lat).sum::<f64>() / ring.len() as f64;
    let m_lat = 111_320.0;
    let m_lon = m_lat * lat0.to_radians().cos();
    let mut twice = 0.0;
    for w in ring.windows(2) {
        let (x0, y0) = (w[0].lon * m_lon, w[0].lat * m_lat);
        let (x1, y1) = (w[1].lon * m_lon, w[1].lat * m_lat);
        twice += x0 * y1 - x1 * y0;
    }
    (twice / 2.0).abs()
}

/// Read the parkings of `source` (see the [module docs](self)), inside `region`
/// if given.
///
/// # Errors
///
/// [`OsmError`] if the source cannot be read.
///
/// # Panics
///
/// Never in practice: counts of parkings fit `u32`.
pub fn read_parkings(
    source: &dyn OsmSource,
    region: Option<&Region>,
    options: ParkingReadOptions,
) -> Result<(Vec<ParkingRow>, ParkingReadReport), OsmError> {
    let mut report = ParkingReadReport::default();
    // Pass 1: the ways, and the nodes they need.
    // A parking way: its id, nodes, kind, name and tagged capacity.
    type Way = (i64, Vec<i64>, ParkingKind, Option<String>, Option<u32>);
    let mut ways: Vec<Way> = Vec::new();
    let mut needed: HashMap<i64, Option<LonLat>> = HashMap::new();
    let mut private_ways: Vec<bool> = Vec::new();
    source.for_each_way(&mut |way| {
        let Some(kind) = parking_kind(&way.tags) else { return };
        for &n in &way.node_ids {
            needed.insert(n, None);
        }
        let name = tag_of(&way.tags, "name").map(str::to_string);
        private_ways.push(is_private(&way.tags));
        ways.push((way.id, way.node_ids, kind, name, capacity_tag(&way.tags)));
    })?;
    // Pass 2: the nodes — tagged parkings, and the ways' geometry.
    let mut found: Vec<Found> = Vec::new();
    source.for_each_node(&mut |node| {
        if let Some(slot) = needed.get_mut(&node.id) {
            *slot = Some(LonLat::new(node.lon, node.lat));
        }
        let Some(kind) = parking_kind(&node.tags) else { return };
        let (capacity, capacity_from) = match capacity_tag(&node.tags) {
            Some(c) => (c, CapacityFrom::Tag),
            None => (
                match kind {
                    ParkingKind::Car => options.capacity_car,
                    ParkingKind::Bike => options.capacity_bike,
                },
                CapacityFrom::Default,
            ),
        };
        found.push(Found {
            id: format!("osm:n{}", node.id),
            name: tag_of(&node.tags, "name").map(str::to_string),
            kind,
            position: LonLat::new(node.lon, node.lat),
            capacity,
            capacity_from,
            bounds: None,
            from_node: true,
            private: is_private(&node.tags),
        });
    })?;
    for ((id, node_ids, kind, name, tagged), private) in ways.into_iter().zip(private_ways) {
        let points: Vec<LonLat> =
            node_ids.iter().filter_map(|n| needed.get(n).copied().flatten()).collect();
        if points.is_empty() {
            continue;
        }
        let closed = node_ids.len() >= 4 && node_ids.first() == node_ids.last();
        // The centre: the mean of the distinct vertices.
        let distinct = if closed { &points[..points.len() - 1] } else { &points[..] };
        #[allow(clippy::cast_precision_loss, reason = "the vertices of one car park")]
        let n = distinct.len().max(1) as f64;
        let position = LonLat::new(
            distinct.iter().map(|p| p.lon).sum::<f64>() / n,
            distinct.iter().map(|p| p.lat).sum::<f64>() / n,
        );
        let bounds = points.iter().fold(
            (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY),
            |(w, s, e, nn), p| (w.min(p.lon), s.min(p.lat), e.max(p.lon), nn.max(p.lat)),
        );
        let area =
            if closed && points.len() == node_ids.len() { ring_area_m2(&points) } else { 0.0 };
        let per = match kind {
            ParkingKind::Car => options.area_per_car_m2,
            ParkingKind::Bike => options.area_per_bike_m2,
        };
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a count of spaces, far below u32::MAX"
        )]
        let from_area = (per > 0.0 && area > 0.0).then(|| ((area / per).floor() as u32).max(1));
        let (capacity, capacity_from) = if let Some(c) = tagged {
            (c, CapacityFrom::Tag)
        } else if let Some(c) = from_area {
            (c, CapacityFrom::Area)
        } else {
            (
                match kind {
                    ParkingKind::Car => options.capacity_car,
                    ParkingKind::Bike => options.capacity_bike,
                },
                CapacityFrom::Default,
            )
        };
        found.push(Found {
            id: format!("osm:w{id}"),
            name,
            kind,
            position,
            capacity,
            capacity_from,
            bounds: closed.then_some(bounds),
            from_node: false,
            private,
        });
    }
    if let Some(region) = region {
        found.retain(|f| region.contains(f.position.lon, f.position.lat));
    }
    let before = found.len();
    found.retain(|f| !f.private);
    report.private_dropped = u32::try_from(before - found.len()).expect("few parkings");
    // A node inside an area of the same kind is the same parking twice.
    let areas: Vec<(ParkingKind, (f64, f64, f64, f64))> =
        found.iter().filter_map(|f| f.bounds.map(|b| (f.kind, b))).collect();
    let before = found.len();
    found.retain(|f| {
        !(f.from_node
            && areas.iter().any(|&(k, (w, s, e, n))| {
                k == f.kind
                    && (w..=e).contains(&f.position.lon)
                    && (s..=n).contains(&f.position.lat)
            }))
    });
    report.duplicates_dropped = u32::try_from(before - found.len()).expect("few parkings");
    for f in &found {
        match f.kind {
            ParkingKind::Car => report.car_found += 1,
            ParkingKind::Bike => report.bike_found += 1,
        }
        match f.capacity_from {
            CapacityFrom::Tag => report.capacity_tagged += 1,
            CapacityFrom::Area => report.capacity_from_area += 1,
            CapacityFrom::Default => report.capacity_default += 1,
        }
    }
    let rows = merge(found, options.merge_m);
    for r in &rows {
        match r.kind {
            ParkingKind::Car => report.sites_car += 1,
            ParkingKind::Bike => report.sites_bike += 1,
        }
    }
    Ok((rows, report))
}

/// Merge parkings of one kind within `merge_m` of a larger one into sites (see
/// the [module docs](self)); in order of capacity, then id, so the result does
/// not depend on the order the file had.
fn merge(mut found: Vec<Found>, merge_m: f64) -> Vec<ParkingRow> {
    found.sort_by(|a, b| b.capacity.cmp(&a.capacity).then_with(|| a.id.cmp(&b.id)));
    let mut taken = vec![false; found.len()];
    let mut rows = Vec::new();
    for i in 0..found.len() {
        if taken[i] {
            continue;
        }
        taken[i] = true;
        let seed = &found[i];
        let mut members = vec![i];
        if merge_m > 0.0 {
            for j in i + 1..found.len() {
                if !taken[j]
                    && found[j].kind == seed.kind
                    && ground_distance_metres(seed.position, found[j].position) <= merge_m
                {
                    taken[j] = true;
                    members.push(j);
                }
            }
        }
        let capacity: u32 = members.iter().map(|&m| found[m].capacity).sum();
        let c = f64::from(capacity.max(1));
        let lon = members
            .iter()
            .map(|&m| found[m].position.lon * f64::from(found[m].capacity))
            .sum::<f64>()
            / c;
        let lat = members
            .iter()
            .map(|&m| found[m].position.lat * f64::from(found[m].capacity))
            .sum::<f64>()
            / c;
        let name = members.iter().find_map(|&m| found[m].name.clone());
        rows.push(ParkingRow {
            parking_id: seed.id.clone(),
            name,
            hub_id: None,
            position: LonLat::new(lon, lat),
            kind: seed.kind,
            capacity,
            initial_occupancy: 0,
        });
    }
    rows.sort_by(|a, b| a.parking_id.cmp(&b.parking_id));
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::{MemorySource, OsmNode, OsmWay};

    fn tags(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|&(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn the_tag_rules_keep_park_and_ride_and_bike_parking_only() {
        assert_eq!(parking_kind(&tags(&[("amenity", "parking")])), None, "a plain car park");
        assert_eq!(
            parking_kind(&tags(&[("amenity", "parking"), ("park_ride", "train")])),
            Some(ParkingKind::Car)
        );
        assert_eq!(parking_kind(&tags(&[("amenity", "parking"), ("park_ride", "no")])), None);
        assert_eq!(
            parking_kind(&tags(&[("amenity", "parking"), ("name", "P + R Zeeburg")])),
            Some(ParkingKind::Car)
        );
        assert_eq!(
            parking_kind(&tags(&[("amenity", "parking"), ("name", "Transferium ArenA")])),
            Some(ParkingKind::Car)
        );
        assert_eq!(parking_kind(&tags(&[("amenity", "bicycle_parking")])), Some(ParkingKind::Bike));
        assert_eq!(parking_kind(&tags(&[("amenity", "bench")])), None);
        assert_eq!(capacity_tag(&tags(&[("capacity", "ca. 200")])), Some(200));
        assert_eq!(capacity_tag(&tags(&[("capacity", "0")])), None);
        assert_eq!(capacity_tag(&tags(&[("capacity", "many")])), None);
    }

    /// A square way of `side` metres around (4.9, 52.37).
    fn square(id: i64, first_node: i64, side: f64, t: &[(&str, &str)]) -> (OsmWay, Vec<OsmNode>) {
        let d_lat = side / 111_320.0;
        let d_lon = side / (111_320.0 * 52.37f64.to_radians().cos());
        let corners = [(0.0, 0.0), (d_lon, 0.0), (d_lon, d_lat), (0.0, d_lat)];
        let nodes: Vec<OsmNode> = corners
            .iter()
            .enumerate()
            .map(|(i, &(x, y))| OsmNode::new(first_node + i as i64, 4.9 + x, 52.37 + y))
            .collect();
        let ids = [first_node, first_node + 1, first_node + 2, first_node + 3, first_node];
        (OsmWay::new(id, ids, t.iter().copied()), nodes)
    }

    #[test]
    fn capacity_comes_from_the_tag_else_the_area_else_the_default_and_a_duplicate_node_is_dropped()
    {
        // A 50 m square P+R lot: 2500 m² / 25 = 100 spaces; the entrance node inside it
        // is the same parking.
        let (lot, lot_nodes) = square(1, 10, 50.0, &[("amenity", "parking"), ("park_ride", "yes")]);
        let entrance = OsmNode::new(99, 4.9001, 52.3701)
            .with_tags([("amenity", "parking"), ("park_ride", "yes")]);
        // A bike rack node 2 km away with a capacity, and one without, far from everything.
        let rack = OsmNode::new(200, 4.93, 52.37)
            .with_tags([("amenity", "bicycle_parking"), ("capacity", "12")]);
        let bare = OsmNode::new(201, 4.96, 52.37).with_tags([("amenity", "bicycle_parking")]);
        let private = OsmNode::new(202, 4.99, 52.37)
            .with_tags([("amenity", "bicycle_parking"), ("access", "private")]);
        let source =
            MemorySource::new().nodes(lot_nodes).nodes([entrance, rack, bare, private]).way(lot);
        let (rows, report) =
            read_parkings(&source, None, ParkingReadOptions::SHIPPED).expect("reads");
        assert_eq!(report.duplicates_dropped, 1);
        assert_eq!(report.private_dropped, 1);
        assert_eq!((report.car_found, report.bike_found), (1, 2));
        assert_eq!(
            (report.capacity_tagged, report.capacity_from_area, report.capacity_default),
            (1, 1, 1)
        );
        let lot = rows.iter().find(|r| r.parking_id == "osm:w1").expect("the lot");
        assert_eq!(lot.kind, ParkingKind::Car);
        assert!((99..=100).contains(&lot.capacity), "{}", lot.capacity);
        let rack = rows.iter().find(|r| r.parking_id == "osm:n200").expect("the rack");
        assert_eq!(rack.capacity, 12);
        let bare = rows.iter().find(|r| r.parking_id == "osm:n201").expect("the bare rack");
        assert_eq!(bare.capacity, ParkingReadOptions::SHIPPED.capacity_bike);
    }

    #[test]
    fn racks_close_together_merge_into_one_site_under_the_largest() {
        let a = OsmNode::new(1, 4.9000, 52.37)
            .with_tags([("amenity", "bicycle_parking"), ("capacity", "10")]);
        // About 35 m east: merged.
        let b = OsmNode::new(2, 4.9005, 52.37)
            .with_tags([("amenity", "bicycle_parking"), ("capacity", "30")]);
        // About 340 m east: its own site.
        let c = OsmNode::new(3, 4.9050, 52.37)
            .with_tags([("amenity", "bicycle_parking"), ("capacity", "5")]);
        let source = MemorySource::new().nodes([a, b, c]);
        let (rows, report) =
            read_parkings(&source, None, ParkingReadOptions::SHIPPED).expect("reads");
        assert_eq!(report.sites_bike, 2);
        let site = rows.iter().find(|r| r.parking_id == "osm:n2").expect("merged under the larger");
        assert_eq!(site.capacity, 40);
        assert!((site.position.lon - (4.9 * 10.0 + 4.9005 * 30.0) / 40.0).abs() < 1e-9);
        // No merging at 0 m.
        let options = ParkingReadOptions { merge_m: 0.0, ..ParkingReadOptions::SHIPPED };
        let source = MemorySource::new().nodes([
            OsmNode::new(1, 4.9, 52.37).with_tags([("amenity", "bicycle_parking")]),
            OsmNode::new(2, 4.9005, 52.37).with_tags([("amenity", "bicycle_parking")]),
        ]);
        assert_eq!(read_parkings(&source, None, options).expect("reads").0.len(), 2);
    }

    #[test]
    fn a_region_keeps_what_lies_inside() {
        let inside = OsmNode::new(1, 4.90, 52.37).with_tags([("amenity", "bicycle_parking")]);
        let outside = OsmNode::new(2, 5.20, 52.37).with_tags([("amenity", "bicycle_parking")]);
        let source = MemorySource::new().nodes([inside, outside]);
        let region = Region::bbox(4.8, 52.3, 5.0, 52.4).expect("a box");
        let (rows, _) =
            read_parkings(&source, Some(&region), ParkingReadOptions::SHIPPED).expect("reads");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].parking_id, "osm:n1");
    }
}
