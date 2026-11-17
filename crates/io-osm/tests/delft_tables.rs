//! Delft against version 1's reader: `crates/binding/tests/fixtures/delft-tables/` (21 566 nodes;
//! 12 460 car, 20 652 bike and 55 400 walk links, one mode each) is version 1's reader's output
//! for `fixtures/delft/network.osm.pbf` with the region (4.315, 51.965, 4.410, 52.035). This
//! reader on the same extract and region must reproduce it.
//!
//! **How links are compared.** Each link, of either network, is the list of OpenStreetMap nodes
//! it passes (its two ends and its shape, positions to 1e-7 degree), so the directed segments
//! between them. Per mode, the links of both networks that share a segment, transitively, form a
//! stretch; a stretch of one link each, the same segments, the same length (within the
//! tolerance) and the same lanes, speed, capacity and road class is an exact match. Every other
//! difference is counted, and must be one of these, each traced on the extract's own tags or on
//! version 1's own rule:
//!
//! - `merged: <cause>`: a node inside a link here is a link end in version 1, which kept the
//!   street split there for something this reader trimmed: `busway`, a bus-only road through
//!   the node (version 1 contracted cars and buses together); or the two ways either side
//!   differing in a tag version 1's contraction compared and this one does not
//!   (`bike_facility`: `cycleway…`, `cyclestreet`, `bicycle_road`, `bicycle`, `steps`, the bike
//!   layer's infrastructure and dismounting; `bike_speed_limit`: the bike layer's `maxspeed`;
//!   `bus_lanes`; `roundabout`: `junction`; `tagged_or_default`: `lanes…` or `maxspeed…`, a value
//!   tagged on one side and the class's equal default on the other).
//! - `merged: knock_on`: a link end of version 1 inside a link here, at a node where version 1's
//!   own links are not one street's two directions or would make a loop, in a stretch where
//!   version 1 lost a movement (next): its earlier merge left it this node.
//! - `kept: not one street`: a link end here inside a link of version 1: two links in and two
//!   out that are not one street's two directions (a one-way street meeting a two-way one).
//!   Version 1 paired them by the order it met them and merged the node away, losing a movement
//!   (the one-way street's continuation); this reader keeps the node (a bug fixed).
//! - `kept: one_sided_lanes`, `kept: bus_lanes`: a link end here inside a link of version 1, a way
//!   at the node tagging lanes the two causes below read otherwise (a two-way street is merged
//!   only when both its directions are): on a segment at the node, in one direction, the car link
//!   here and version 1's have the lanes that cause's rule gives, from the way's tags.
//! - `signal_band`: a car link's capacity, version 1's being its lanes times the speed band's
//!   saturation flow at a signal (1 000, 1 200, 1 500 veh/h per lane up to 30, 50, 70 km/h on an
//!   at-grade road), lower than its class's, its downstream node tagged
//!   `highway=traffic_signals` in the extract; this reader gives its lanes times its class's
//!   capacity per lane (bands trimmed).
//! - `bus_lanes`: a car link with fewer lanes in version 1, which took the bus lanes from the cars
//!   (bus lanes trimmed): its way's `bus:lanes` (one-way) or `bus:lanes:forward`/`:backward`
//!   lists one value per lane, as many as its lanes here, and version 1's are those not
//!   `designated`.
//! - `one_sided_lanes`: the fixed bug, a two-way way tagged `lanes` and one of
//!   `lanes:forward`/`:backward`: on the other side, its lanes here are total − given −
//!   `lanes:both_ways`, version 1's total / 2 rounded down.
//! - `least_length`: a link of under 0.5 m that version 1 gave its least length, 0.5 m; this
//!   reader gives the geodesic.
//!
//! **Capacity.** Lanes and speeds are compared as they are. The class table's capacities per lane
//! are no longer version 1's: they count the class's junctions in (the crate's README), so a car
//! link whose capacity version 1 took from its class table is compared as version 1's capacity ×
//! (this class's capacity per lane / version 1's); a link of version 1 whose capacity is not its
//! class's (a signal band) is traced as above, its capacity here its lanes × this class's.
//!
//! Tolerances: lengths within 1e-8 relative (this reader sums Vincenty's geodesics, version 1 a
//! mid-latitude formula exact to well under a millimetre per segment); speeds and capacities
//! within 1e-9 relative (version 1 went through m/s).
//!
//! For bikes: the bike links are compared as the car links are (segments, length, class), and
//! the bike layer's merges named by the bike tags version 1 compared.
//!
//! Release only: `cargo test --release -p openmobisim-io-osm --test delft_tables -- --ignored
//! --nocapture` (about 2 s; CI runs it with `delft.rs`).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Read;
use std::path::Path;
use std::time::Instant;

use openmobisim_graph::geometry::{LonLat, Xy};
use openmobisim_graph::network::{BuildValues, Network};
use openmobisim_io_osm::{Osm, OsmNode, OsmWay, Region, network, read_pbf};
use openmobisim_types::ids::{EntityId, LinkId};
use openmobisim_types::mode::Mode;

const VALUES: BuildValues =
    BuildValues { capacity_per_lane: 1_800.0, bike_speed: 15.0, walk_speed: 4.8 };
const TABLES: &str = "../binding/tests/fixtures/delft-tables";
const LENGTH_TOL: f64 = 1e-8;
const VALUE_TOL: f64 = 1e-9;

/// A position to 1e-7 degree, as OpenStreetMap stores it.
type At = (i64, i64);

/// One link of either network.
#[derive(Clone, Debug)]
struct Row {
    mode: String,
    /// Its ends and its shape, in its own direction.
    points: Vec<At>,
    length: f64,
    lanes: Option<u8>,
    speed: Option<f64>,
    capacity: Option<f64>,
    class: String,
    /// The OpenStreetMap way of its first piece.
    way: i64,
}

impl Row {
    fn up(&self) -> At {
        self.points[0]
    }

    fn down(&self) -> At {
        self.points[self.points.len() - 1]
    }
}

fn at((lon, lat): (f64, f64)) -> At {
    #[allow(clippy::cast_possible_truncation, reason = "degrees times 1e7 fit i64")]
    let r = |x: f64| (x * 1e7).round() as i64;
    (r(lon), r(lat))
}

fn near(a: f64, b: f64, rel: f64) -> bool {
    (a - b).abs() <= rel * b.abs().max(1.0)
}

fn gz(name: &str) -> Vec<Vec<String>> {
    let file = std::fs::File::open(Path::new(TABLES).join(name)).expect("the fixture");
    let mut text = String::new();
    flate2::read::GzDecoder::new(file).read_to_string(&mut text).expect("gzip text");
    // No cell holds a comma (`shape` is `lon lat;lon lat`): a plain split reads the CSV.
    text.lines().skip(1).map(|l| l.split(',').map(str::to_owned).collect()).collect()
}

fn way_of(id: &str) -> i64 {
    id.split(':').nth(1).and_then(|w| w.parse().ok()).expect("<mode>:<way>:…")
}

/// The fixture's links (columns: link_id, upstream, downstream, modes, length_m, lanes,
/// car_speed_km_h, capacity_veh_h, shape, road_class).
fn fixture() -> Vec<Row> {
    let parse = |lon: &str, lat: &str| at((lon.parse().unwrap(), lat.parse().unwrap()));
    let nodes: HashMap<String, At> =
        gz("nodes.csv.gz").into_iter().map(|r| (r[0].clone(), parse(&r[1], &r[2]))).collect();
    gz("links.csv.gz")
        .into_iter()
        .map(|r| {
            let mut points = vec![nodes[&r[1]]];
            for p in r[8].split(';').filter(|p| !p.is_empty()) {
                let (lon, lat) = p.split_once(' ').expect("lon lat");
                points.push(parse(lon, lat));
            }
            points.push(nodes[&r[2]]);
            Row {
                mode: r[3].clone(),
                points,
                length: r[4].parse().unwrap(),
                lanes: r[5].parse().ok(),
                speed: r[6].parse().ok(),
                capacity: r[7].parse().ok(),
                class: r[9].clone(),
                way: way_of(&r[0]),
            }
        })
        .collect()
}

/// This reader's links, their points traced back to the extract's nodes through the network's
/// own projection.
fn ours(net: &Network, osm: &Osm) -> Vec<Row> {
    let projection = net.projection().expect("longitude/latitude");
    let of: HashMap<(u64, u64), At> = osm
        .nodes
        .iter()
        .filter(|n| LonLat::new(n.lon, n.lat).is_projectable())
        .map(|n| {
            let xy = projection.project(LonLat::new(n.lon, n.lat));
            ((xy.x.to_bits(), xy.y.to_bits()), at((n.lon, n.lat)))
        })
        .collect();
    let point = |xy: Xy| of[&(xy.x.to_bits(), xy.y.to_bits())];
    LinkId::iter_space(net.link_count())
        .map(|l| {
            let mode = [Mode::Car, Mode::Bike, Mode::Walk]
                .into_iter()
                .find(|&m| net.modes(l).contains(m))
                .expect("one mode");
            let mut points = vec![point(net.node_position(net.upstream(l)))];
            points.extend(net.shape(l).iter().map(|&xy| point(xy)));
            points.push(point(net.node_position(net.downstream(l))));
            let car = mode == Mode::Car;
            Row {
                mode: mode.to_string(),
                points,
                length: net.length(l).0,
                lanes: car.then(|| net.lanes(l)),
                speed: car.then(|| net.free_flow_speed(l, Mode::Car).0 * 3.6),
                capacity: car.then(|| net.capacity(l).0 * 3600.0),
                class: net.road_class(l).unwrap_or_default().to_owned(),
                way: way_of(net.link_ids().external(l.raw())),
            }
        })
        .collect()
}

/// What the extract says about the causes.
struct Extract<'a> {
    ways: HashMap<i64, &'a OsmWay>,
    positions: HashMap<i64, At>,
    /// Nodes tagged `highway=traffic_signals`.
    signals: HashSet<At>,
    /// Nodes of a road for buses alone: `highway=busway|bus_guideway`, or a car road closed to
    /// cars (`access`, `motor_vehicle`, `motorcar`, `vehicle` `no|private`) and open to buses
    /// (`bus`, `psv` `yes|designated`): version 1's busways.
    bus_only: HashSet<At>,
    /// The ways of each segment, either way round.
    segment_ways: HashMap<(At, At), Vec<i64>>,
}

impl Extract<'_> {
    fn new(osm: &Osm) -> Extract<'_> {
        let positions: HashMap<i64, At> =
            osm.nodes.iter().map(|n| (n.id, at((n.lon, n.lat)))).collect();
        let for_buses = |w: &OsmWay| {
            let open = |k| matches!(w.tag(k), Some("yes" | "designated"));
            let closed = |k| matches!(w.tag(k), Some("no" | "private"));
            let not_car = ["footway", "cycleway", "path", "pedestrian", "steps", "track"];
            matches!(w.tag("highway"), Some("busway" | "bus_guideway"))
                || ((open("bus") || open("psv"))
                    && w.tag("highway").is_some_and(|h| !not_car.contains(&h))
                    && ["access", "motor_vehicle", "motorcar", "vehicle"].into_iter().any(closed))
        };
        let mut segment_ways: HashMap<(At, At), Vec<i64>> = HashMap::new();
        for w in &osm.ways {
            for s in w.nodes.windows(2) {
                if let (Some(&a), Some(&b)) = (positions.get(&s[0]), positions.get(&s[1])) {
                    segment_ways.entry((a.min(b), a.max(b))).or_default().push(w.id);
                }
            }
        }
        let signal =
            |n: &&OsmNode| n.tags.iter().any(|(k, v)| k == "highway" && v == "traffic_signals");
        Extract {
            segment_ways,
            signals: osm.nodes.iter().filter(signal).map(|n| at((n.lon, n.lat))).collect(),
            ways: osm.ways.iter().map(|w| (w.id, w)).collect(),
            bus_only: osm
                .ways
                .iter()
                .filter(|w| for_buses(w))
                .flat_map(|w| w.nodes.iter().filter_map(|n| positions.get(n).copied()))
                .collect(),
            positions,
        }
    }

    fn tags(&self, way: i64) -> BTreeMap<&str, &str> {
        self.ways.get(&way).map_or_else(BTreeMap::new, |w| {
            w.tags.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect()
        })
    }

    /// The way of the segment from `a` to `b` (the first, if several ways share it).
    fn way(&self, a: At, b: At) -> i64 {
        self.segment_ways.get(&(a.min(b), a.max(b))).map_or(0, |w| w[0])
    }

    /// Why version 1 kept a street split at `node`, between the segments `before` → `node` and
    /// `node` → `after`.
    fn split_cause(&self, mode: &str, before: At, node: At, after: At) -> &'static str {
        if mode == "car" && self.bus_only.contains(&node) {
            return "merged: busway";
        }
        let (ta, tb) = (self.tags(self.way(before, node)), self.tags(self.way(node, after)));
        let differ = |keys: &dyn Fn(&str) -> bool| {
            !ta.iter().filter(|(k, _)| keys(k)).eq(tb.iter().filter(|(k, _)| keys(k)))
        };
        let bike_facility = |k: &str| {
            k.starts_with("cycleway") || matches!(k, "cyclestreet" | "bicycle_road" | "bicycle")
        };
        let steps = ta.get("highway") != tb.get("highway")
            && [ta.get("highway"), tb.get("highway")].contains(&Some(&"steps"));
        let bus = |k: &str| k.contains("bus") || k.contains("psv") || k.contains("hov");
        match mode {
            "bike" if differ(&bike_facility) || steps => "merged: bike_facility",
            "bike" if differ(&|k| k.starts_with("maxspeed")) => "merged: bike_speed_limit",
            "car" if differ(&bus) => "merged: bus_lanes",
            "car" if differ(&|k| k == "junction") => "merged: roundabout",
            "car" if differ(&|k| k.starts_with("lanes") || k.starts_with("maxspeed")) => {
                "merged: tagged_or_default"
            }
            _ => "UNEXPLAINED merge",
        }
    }

    /// Whether the segment from `a` to `b` runs along `way`'s node order (`Some(true)`), against
    /// it (`Some(false)`), or is not one of its segments (`None`).
    fn along(&self, way: i64, a: At, b: At) -> Option<bool> {
        let nodes = &self.ways.get(&way)?.nodes;
        nodes.windows(2).find_map(|s| {
            let (p, q) = (self.positions.get(&s[0])?, self.positions.get(&s[1])?);
            if (*p, *q) == (a, b) { Some(true) } else { ((*p, *q) == (b, a)).then_some(false) }
        })
    }

    /// The lanes of `way` in one direction (`forward`: along its node order) by each lane rule
    /// of version 1 this reader trimmed or fixed, where the two differ: (cause, lanes here, lanes
    /// in version 1), all read from the way's tags.
    ///
    /// - `bus_lanes`: `bus:lanes` (one-way) or `bus:lanes:forward`/`:backward` lists one value
    ///   per lane; here every lane, in version 1 those not `designated`.
    /// - `one_sided_lanes`: a two-way way tagging `lanes` and one side only; the other side here
    ///   total − given − `lanes:both_ways`, in version 1 total / 2 rounded down.
    fn lane_rules(&self, way: i64, forward: bool) -> Vec<(&'static str, u8, u8)> {
        let t = self.tags(way);
        let count = |k: &str| t.get(k).and_then(|v| v.trim().parse::<u8>().ok());
        let side = if forward { "forward" } else { "backward" };
        let two_way = matches!(t.get("oneway"), None | Some(&"no"));
        let mut rules = Vec::new();
        let bus_key = if two_way { format!("bus:lanes:{side}") } else { "bus:lanes".into() };
        if let Some(list) = t.get(bus_key.as_str()) {
            let all = list.split('|').count();
            let bus = list.split('|').filter(|v| v.trim() == "designated").count();
            if let (Ok(here), Ok(bus)) = (u8::try_from(all), u8::try_from(bus)) {
                rules.push(("bus_lanes", here, here - bus));
            }
        }
        let other = if forward { "backward" } else { "forward" };
        let given = count(&format!("lanes:{other}"));
        if let (true, Some(total), Some(given), None) =
            (two_way, count("lanes"), given, count(&format!("lanes:{side}")))
        {
            let middle = count("lanes:both_ways").unwrap_or(0);
            let rest = total.checked_sub(given).and_then(|r| r.checked_sub(middle));
            if let Some(here) = rest.filter(|&r| r > 0) {
                rules.push(("one_sided_lanes", here, total / 2));
            }
        }
        rules.retain(|&(_, here, there)| here != there);
        rules
    }

    /// The rule of [`Extract::lane_rules`] that gives a car link on `way` in one direction its
    /// lanes here and in version 1 (`here`, `there`), exactly.
    fn lanes_cause(
        &self,
        here: Option<u8>,
        there: Option<u8>,
        way: i64,
        forward: Option<bool>,
    ) -> &'static str {
        let rules = forward.map(|f| self.lane_rules(way, f)).unwrap_or_default();
        rules
            .into_iter()
            .find(|&(_, h, t)| (Some(h), Some(t)) == (here, there))
            .map_or("UNEXPLAINED lanes", |(cause, _, _)| cause)
    }
}

/// Version 1's saturation band at a signal, veh/h per lane, on an at-grade road.
fn band(class: &str, speed: f64) -> Option<f64> {
    let main = class.starts_with("motorway") || class.starts_with("trunk");
    let per_lane = [(30.0, 1000.0), (50.0, 1200.0), (70.0, 1500.0)]
        .into_iter()
        .find(|&(top, _)| speed <= top + 1e-9)
        .map(|(_, flow)| flow);
    per_lane.filter(|_| !main)
}

/// The capacity per lane by class, veh/h: version 1's table, and this reader's (the README's).
const PER_LANE: [(&str, f64, f64); 14] = [
    ("motorway", 2000.0, 2000.0),
    ("motorway_link", 1600.0, 1550.0),
    ("trunk", 1800.0, 1800.0),
    ("trunk_link", 1500.0, 1550.0),
    ("primary", 1900.0, 1080.0),
    ("primary_link", 1500.0, 900.0),
    ("secondary", 1800.0, 900.0),
    ("secondary_link", 1500.0, 900.0),
    ("tertiary", 1700.0, 810.0),
    ("tertiary_link", 1400.0, 720.0),
    ("unclassified", 1600.0, 720.0),
    ("residential", 1400.0, 600.0),
    ("living_street", 900.0, 450.0),
    ("service", 800.0, 450.0),
];

/// (version 1's, this reader's) capacity per lane of `class`.
fn per_lane(class: &str) -> (f64, f64) {
    PER_LANE.iter().find(|r| r.0 == class).map_or((f64::NAN, f64::NAN), |r| (r.1, r.2))
}

/// The cause of a difference between a link here and one of version 1's on the same segment.
fn attribute_cause(o: &Row, f: &Row, x: &Extract<'_>) -> Option<&'static str> {
    let same = |a: Option<f64>, b: Option<f64>| match (a, b) {
        (Some(a), Some(b)) => near(a, b, VALUE_TOL),
        (a, b) => a.is_none() && b.is_none(),
    };
    if o.class != f.class || !same(o.speed, f.speed) {
        return Some("UNEXPLAINED class or speed");
    }
    if o.lanes != f.lanes {
        let forward = x.along(o.way, o.points[0], o.points[1]);
        return Some(x.lanes_cause(o.lanes, f.lanes, o.way, forward));
    }
    let (old, new) = per_lane(&f.class);
    if !same(o.capacity, f.capacity.map(|c| c * new / old)) {
        let lanes = f64::from(f.lanes.unwrap_or(0));
        let banded = band(&f.class, f.speed.unwrap_or(f64::INFINITY))
            .is_some_and(|b| b < old && near(f.capacity.unwrap_or(0.0), lanes * b, VALUE_TOL));
        let here = o.capacity.is_some_and(|c| near(c, lanes * new, VALUE_TOL));
        let signal = x.signals.contains(&f.down());
        return Some(if banded && here && signal { "signal_band" } else { "UNEXPLAINED capacity" });
    }
    None
}

/// The nodes whose links are one in and one out or one street's two directions, here and in
/// version 1; and the lanes of the car link holding each directed segment, here and in version 1.
struct Streets {
    here: HashSet<(String, At)>,
    there: HashSet<(String, At)>,
    lanes_here: HashMap<(At, At), Option<u8>>,
    lanes_there: HashMap<(At, At), Option<u8>>,
}

/// The lanes of the car link holding each directed segment.
fn car_lanes(rows: &[Row]) -> HashMap<(At, At), Option<u8>> {
    rows.iter()
        .filter(|r| r.mode == "car")
        .flat_map(|r| r.points.windows(2).map(|s| ((s[0], s[1]), r.lanes)))
        .collect()
}

/// The nodes, per mode, whose links are one in and one out, or two in and two out that are
/// one street's two directions, from nowhere back to itself: the nodes a contraction may merge away.
fn one_street_nodes(rows: &[Row]) -> HashSet<(String, At)> {
    let mut ins: HashMap<(String, At), Vec<At>> = HashMap::new();
    let mut outs: HashMap<(String, At), Vec<At>> = HashMap::new();
    for r in rows {
        ins.entry((r.mode.clone(), r.down())).or_default().push(r.up());
        outs.entry((r.mode.clone(), r.up())).or_default().push(r.down());
    }
    let mut out = HashSet::new();
    for (node, mut from) in ins {
        let mut to = outs.get(&node).cloned().unwrap_or_default();
        from.sort_unstable();
        to.sort_unstable();
        let one = match from.len() {
            1 => to.len() == 1 && to != from,
            2 => from == to && from[0] != from[1],
            _ => false,
        };
        if one {
            out.insert(node);
        }
    }
    out
}

/// The stretches: per mode, the links of both networks that share a segment, transitively, as
/// (version 1's links, this reader's links).
fn stretches(theirs: &[Row], mine: &[Row]) -> Vec<(Vec<usize>, Vec<usize>)> {
    // Union-find over version 1's links (0..n) and this reader's (n..n + m).
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    let n = theirs.len();
    let mut parent: Vec<usize> = (0..n + mine.len()).collect();
    let mut owner: HashMap<(&str, At, At), usize> = HashMap::new();
    for (k, r) in theirs.iter().chain(mine).enumerate() {
        for s in r.points.windows(2) {
            if let Some(&other) = owner.get(&(r.mode.as_str(), s[0], s[1])) {
                let (a, b) = (root(&mut parent, other), root(&mut parent, k));
                parent[a] = b;
            } else {
                owner.insert((r.mode.as_str(), s[0], s[1]), k);
            }
        }
    }
    let mut groups: BTreeMap<usize, (Vec<usize>, Vec<usize>)> = BTreeMap::new();
    for k in 0..n + mine.len() {
        let g = groups.entry(root(&mut parent, k)).or_default();
        if k < n { g.0.push(k) } else { g.1.push(k - n) }
    }
    groups.into_values().collect()
}

/// The causes of one stretch's differences, none for an exact match.
fn stretch_causes(f: &[&Row], o: &[&Row], streets: &Streets, x: &Extract<'_>) -> Vec<&'static str> {
    let sum = |rows: &[&Row]| rows.iter().map(|r| r.length).sum::<f64>();
    // Two ways on the same nodes (one drawn over the other): each compared with its own.
    let same = |a: &&Row, b: &&Row| a.points == b.points && a.class == b.class;
    if f.len() > 1 && f.len() == o.len() && f.iter().zip(o).all(|(a, b)| same(a, b)) {
        return f.iter().zip(o).flat_map(|(a, b)| stretch_causes(&[a], &[b], streets, x)).collect();
    }
    if f.len() == 1 && o.len() == 1 && f[0].points == o[0].points {
        let (f, o) = (f[0], o[0]);
        if (f.length - 0.5).abs() < 1e-12 && o.length < 0.5 {
            return vec!["least_length"];
        }
        if !near(o.length, f.length, LENGTH_TOL) {
            return vec!["UNEXPLAINED length"];
        }
        return attribute_cause(o, f, x).into_iter().collect();
    }
    if f.is_empty() || o.is_empty() || !near(sum(o), sum(f), LENGTH_TOL) {
        return vec!["UNEXPLAINED stretch"];
    }
    let mut causes = Vec::new();
    // Each pair sharing a segment, compared once; of two ways on the same nodes, the same way's.
    let segments =
        |r: &Row| -> HashSet<(At, At)> { r.points.windows(2).map(|s| (s[0], s[1])).collect() };
    for a in f {
        let sa = segments(a);
        let sharing: Vec<&&Row> =
            o.iter().filter(|b| b.points.windows(2).any(|s| sa.contains(&(s[0], s[1])))).collect();
        let same_class = sharing.iter().any(|b| b.class == a.class);
        for b in sharing.into_iter().filter(|b| !same_class || b.class == a.class) {
            causes.extend(attribute_cause(b, a, x));
        }
    }
    let ends =
        |rows: &[&Row]| -> HashSet<At> { rows.iter().flat_map(|r| [r.up(), r.down()]).collect() };
    let inner = |rows: &[&Row]| -> HashSet<At> {
        rows.iter().flat_map(|r| r.points[1..r.points.len() - 1].iter().copied()).collect()
    };
    let mode = f[0].mode.as_str();
    // The point before `node` on a link into it, the point after on a link out, their lanes.
    let at_node = |rows: &[&Row], node: At| -> Option<(At, At, Option<u8>, Option<u8>)> {
        let into = rows.iter().find(|r| r.down() == node)?;
        let out = rows.iter().find(|r| r.up() == node)?;
        Some((into.points[into.points.len() - 2], out.points[1], into.lanes, out.lanes))
    };
    let lost =
        ends(o).intersection(&inner(f)).any(|&n| !streets.here.contains(&(mode.to_owned(), n)));
    for &node in ends(f).intersection(&inner(o)) {
        causes.push(match at_node(f, node) {
            _ if lost && !streets.there.contains(&(mode.to_owned(), node)) => "merged: knock_on",
            Some((before, after, _, _)) => x.split_cause(mode, before, node, after),
            None => "UNEXPLAINED merge end",
        });
    }
    for &node in ends(o).intersection(&inner(f)) {
        if !streets.here.contains(&(mode.to_owned(), node)) {
            causes.push("kept: not one street");
            continue;
        }
        // Kept for the lanes of either way at the node, in either direction (a two-way street
        // is merged only when both directions are): the car links on a segment at the node have
        // the lanes a rule gives that way and direction, here and in version 1.
        let Some((before, after, _, _)) = at_node(o, node) else {
            causes.push("UNEXPLAINED split");
            continue;
        };
        let lanes = |map: &HashMap<(At, At), Option<u8>>, s| map.get(&s).copied().flatten();
        let kept = [(before, node), (node, after)].into_iter().find_map(|(a, b)| {
            let way = x.way(a, b);
            [(a, b), (b, a)].into_iter().find_map(|s| {
                let (here, there) = (lanes(&streets.lanes_here, s), lanes(&streets.lanes_there, s));
                let cause = x.lanes_cause(here, there, way, x.along(way, s.0, s.1));
                (!cause.starts_with("UNEX")).then_some(cause)
            })
        });
        causes.push(match kept {
            Some("bus_lanes") => "kept: bus_lanes",
            Some(_) => "kept: one_sided_lanes",
            None => "UNEXPLAINED split",
        });
    }
    causes
}

#[test]
#[ignore = "release only: the whole of Delft; CI runs it in release with --ignored"]
fn delft_reproduces_version_1s_tables_but_for_the_named_trims_and_fixes() {
    let started = Instant::now();
    let osm = read_pbf(Path::new("tests/fixtures/delft/network.osm.pbf")).expect("Delft reads");
    let region = Region::bbox(4.315, 51.965, 4.410, 52.035).expect("a box");
    let (net, report) = network(&osm, VALUES, Some(&region)).expect("Delft builds");
    println!("Delft read and built in {:.2} s: {report:?}", started.elapsed().as_secs_f64());
    let (theirs, mine) = (fixture(), ours(&net, &osm));
    let x = Extract::new(&osm);
    let streets = Streets {
        here: one_street_nodes(&mine),
        there: one_street_nodes(&theirs),
        lanes_here: car_lanes(&mine),
        lanes_there: car_lanes(&theirs),
    };

    let mut causes: BTreeMap<(String, &str), usize> = BTreeMap::new();
    let mut exact: BTreeMap<String, usize> = BTreeMap::new();
    for (f, o) in stretches(&theirs, &mine) {
        let f: Vec<&Row> = f.iter().map(|&i| &theirs[i]).collect();
        let o: Vec<&Row> = o.iter().map(|&i| &mine[i]).collect();
        let mode = f.first().or(o.first()).expect("a stretch has a link").mode.clone();
        let found = stretch_causes(&f, &o, &streets, &x);
        if found.is_empty() {
            *exact.entry(mode.clone()).or_default() += 1;
        }
        if std::env::var_os("DELFT_SHOW").is_some() && found.iter().any(|c| c.starts_with("UNEX")) {
            println!("{found:?}\n  version 1: {f:?}\n  here: {o:?}");
        }
        for cause in found {
            *causes.entry((mode.clone(), cause)).or_default() += 1;
        }
    }

    println!(
        "| mode | links, version 1 | links, here | km, version 1 | km, here | exact matches |"
    );
    for mode in ["car", "bike", "walk"] {
        let lengths = |rows: &[Row]| -> Vec<f64> {
            rows.iter().filter(|r| r.mode == mode).map(|r| r.length).collect()
        };
        let (a, b) = (lengths(&theirs), lengths(&mine));
        let km = |v: &[f64]| v.iter().sum::<f64>() / 1e3;
        let n = exact.get(mode).copied().unwrap_or(0);
        println!("| {mode} | {} | {} | {:.3} | {:.3} | {n} |", a.len(), b.len(), km(&a), km(&b));
    }
    println!("| mode | cause | count |");
    for ((mode, cause), n) in &causes {
        println!("| {mode} | {cause} | {n} |");
    }
    let unexplained: Vec<_> =
        causes.iter().filter(|((_, c), _)| c.starts_with("UNEXPLAINED")).collect();
    assert!(unexplained.is_empty(), "differences not traced: {unexplained:?}");
}
