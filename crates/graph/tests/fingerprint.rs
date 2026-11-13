//! The network's fingerprint as a property: built twice from the same rows it is the same, and
//! changing one value of one input column, all else equal, changes it, for every column a
//! network is built from (one named case per column).
//!
//! The rows: nodes `a` (0, 0), `b` (1 000, 0), `c` (1 000, 800), `d` (0, 800) metres and `e`
//! (500, 400), which no link names (so its id can change alone); `ab` a→b for car, bike and walk,
//! 2 lanes, 50 km/h, 1 000 m, 3 000 veh/h (a capacity given, so its lanes change alone); `bc`
//! b→c for cars, 1 lane, 30 km/h, no length, no capacity (lanes × `capacity_per_lane`); `cd`
//! c→d a cycle path for bike and walk, 1 000 m. The same rows in longitude/latitude give the two
//! cases of a projected network. For bikes: the bike permission of a link (its modes) and the
//! bike speed are cases of their own.

use openmobisim_graph::geometry::{LonLat, Position, Projection, Xy};
use openmobisim_graph::network::{BuildValues, LinkRow, Network, NetworkBuilder};
use openmobisim_types::fingerprint::Fingerprint;
use openmobisim_types::mode::{Mode, ModeSet};

#[derive(Clone)]
struct Rows {
    nodes: Vec<(&'static str, Position)>,
    signals: Vec<&'static str>,
    links: Vec<(&'static str, &'static str, &'static str, LinkRow)>,
    values: BuildValues,
}

fn car_row(lanes: u8, km_h: f64, length_m: Option<f64>, capacity: Option<f64>) -> LinkRow {
    LinkRow {
        modes: ModeSet::of(&[Mode::Car]),
        lanes: Some(lanes),
        car_speed_km_h: Some(km_h),
        length_m,
        capacity_veh_h: capacity,
        signal_at_downstream_end: false,
    }
}

fn rows(lon_lat: bool) -> Rows {
    let at = |x: f64, y: f64| {
        if lon_lat {
            Position::LonLat(LonLat::new(4.36 + x / 68_700.0, 52.0 + y / 111_200.0))
        } else {
            Position::Metres(Xy::new(x, y))
        }
    };
    let ab = LinkRow {
        modes: ModeSet::of(&[Mode::Car, Mode::Bike, Mode::Walk]),
        ..car_row(2, 50.0, Some(1_000.0), Some(3_000.0))
    };
    let cd = LinkRow {
        modes: ModeSet::of(&[Mode::Bike, Mode::Walk]),
        lanes: None,
        car_speed_km_h: None,
        length_m: Some(1_000.0),
        capacity_veh_h: None,
        signal_at_downstream_end: false,
    };
    Rows {
        nodes: vec![
            ("a", at(0.0, 0.0)),
            ("b", at(1_000.0, 0.0)),
            ("c", at(1_000.0, 800.0)),
            ("d", at(0.0, 800.0)),
            ("e", at(500.0, 400.0)),
        ],
        signals: Vec::new(),
        links: vec![
            ("ab", "a", "b", ab),
            ("bc", "b", "c", car_row(1, 30.0, None, None)),
            ("cd", "c", "d", cd),
        ],
        values: BuildValues { capacity_per_lane: 1_800.0, bike_speed: 15.0, walk_speed: 4.8 },
    }
}

fn fingerprint(r: &Rows) -> u64 {
    let mut b = NetworkBuilder::new();
    for &(id, at) in &r.nodes {
        b.add_node(id, at);
    }
    for &id in &r.signals {
        b.mark_signalised(id);
    }
    for &(id, up, down, row) in &r.links {
        b.add_link(id, up, down, row);
    }
    let network: Network = b.build(r.values).expect("a network");
    network.fingerprint()
}

/// Move node `a` by `dx` and `dy` (metres, or degrees for longitude/latitude).
fn move_a(r: &mut Rows, dx: f64, dy: f64) {
    r.nodes[0].1 = match r.nodes[0].1 {
        Position::Metres(p) => Position::Metres(Xy::new(p.x + dx, p.y + dy)),
        Position::LonLat(p) => Position::LonLat(LonLat::new(p.lon + dx, p.lat + dy)),
    };
}

/// Each case changes one value of one column, all else equal.
type Case = (&'static str, bool, fn(&mut Rows));

const CASES: [Case; 18] = [
    ("node id", false, |r| r.nodes[4].0 = "f"),
    ("node x", false, |r| move_a(r, 1.0, 0.0)),
    ("node y", false, |r| move_a(r, 0.0, 1.0)),
    ("node longitude", true, |r| move_a(r, 1e-5, 0.0)),
    ("node latitude", true, |r| move_a(r, 0.0, 1e-5)),
    ("node signal mark", false, |r| r.signals.push("b")),
    ("link id", false, |r| r.links[2].0 = "cz"),
    ("link upstream", false, |r| r.links[2].1 = "b"),
    ("link downstream", false, |r| r.links[2].2 = "a"),
    ("link modes (bikes no longer allowed)", false, |r| {
        r.links[2].3.modes = ModeSet::of(&[Mode::Walk]);
    }),
    ("link length_m", false, |r| r.links[0].3.length_m = Some(1_001.0)),
    ("link lanes", false, |r| r.links[0].3.lanes = Some(3)),
    ("link car_speed_km_h", false, |r| r.links[0].3.car_speed_km_h = Some(51.0)),
    ("link capacity_veh_h", false, |r| r.links[0].3.capacity_veh_h = Some(3_001.0)),
    ("link signal_at_downstream_end", false, |r| r.links[0].3.signal_at_downstream_end = true),
    ("capacity_per_lane", false, |r| r.values.capacity_per_lane = 1_900.0),
    ("bike_speed", false, |r| r.values.bike_speed = 16.0),
    ("walk_speed", false, |r| r.values.walk_speed = 5.0),
];

#[test]
fn built_twice_from_the_same_rows_the_fingerprint_is_the_same() {
    for lon_lat in [false, true] {
        assert_eq!(fingerprint(&rows(lon_lat)), fingerprint(&rows(lon_lat)));
    }
}

#[test]
fn one_value_changed_in_any_column_changes_the_fingerprint() {
    let mut unchanged = Vec::new();
    for (name, lon_lat, change) in CASES {
        let base = rows(lon_lat);
        let mut changed = base.clone();
        change(&mut changed);
        if fingerprint(&changed) == fingerprint(&base) {
            unchanged.push(name);
        }
    }
    assert!(unchanged.is_empty(), "unchanged by: {unchanged:?}");
}

#[test]
fn the_same_rows_in_another_order_have_the_same_fingerprint() {
    for lon_lat in [false, true] {
        let mut reversed = rows(lon_lat);
        reversed.nodes.reverse();
        reversed.links.reverse();
        assert_eq!(fingerprint(&reversed), fingerprint(&rows(lon_lat)));
    }
}

#[test]
fn two_nodes_and_one_link_hash_to_the_byte_stream_the_readme_gives() {
    // The README's order, written out by hand, hashed by the raw-bytes FNV-1a (anchored on the
    // published vectors in crates/types). Nodes a (1, 2) and b (101, 2) metres, no projection;
    // link ab a→b for car, bike and walk: 100 m, 1 lane, 72 km/h = 20 m/s, 3 600 veh/h = 1 veh/s.
    // Bike 36 km/h = 10 m/s, walk 7.2 km/h = 2 m/s (36 000 / 3 600 and 7 200 / 3 600, exact).
    let mut b = NetworkBuilder::new();
    b.add_node("a", Position::Metres(Xy::new(1.0, 2.0)));
    b.add_node("b", Position::Metres(Xy::new(101.0, 2.0)));
    let row = LinkRow {
        modes: ModeSet::of(&[Mode::Car, Mode::Bike, Mode::Walk]),
        ..car_row(1, 72.0, Some(100.0), Some(3_600.0))
    };
    b.add_link("ab", "a", "b", row);
    let values = BuildValues { capacity_per_lane: 1_800.0, bike_speed: 36.0, walk_speed: 7.2 };
    let network = b.build(values).expect("a network");
    let mut s: Vec<u8> = Vec::new();
    let f = |s: &mut Vec<u8>, x: f64| s.extend(x.to_bits().to_le_bytes());
    let n = |s: &mut Vec<u8>, x: u32| s.extend(x.to_le_bytes());
    let text = |s: &mut Vec<u8>, x: &str| {
        s.extend((x.len() as u64).to_le_bytes());
        s.extend(x.as_bytes());
    };
    s.push(0); // no projection
    f(&mut s, 10.0); // bike speed, m/s
    f(&mut s, 2.0); // walk speed, m/s
    n(&mut s, 2); // nodes
    for (id, x, y) in [("a", 1.0, 2.0), ("b", 101.0, 2.0)] {
        text(&mut s, id);
        f(&mut s, x);
        f(&mut s, y);
        s.push(0); // not signalised
    }
    n(&mut s, 1); // links
    text(&mut s, "ab");
    n(&mut s, 0); // upstream a
    n(&mut s, 1); // downstream b
    f(&mut s, 100.0); // length, m
    s.push(1); // lanes
    s.push(0b111); // car, bike, walk: bits 0, 1, 2
    f(&mut s, 20.0); // car speed, m/s
    f(&mut s, 1.0); // capacity, veh/s
    s.push(0); // no signal downstream
    let mut h = Fingerprint::new();
    h.bytes(&s);
    assert_eq!(network.fingerprint(), h.finish());
}

#[test]
fn a_projected_network_hashes_its_zone_and_hemisphere_as_the_readme_gives() {
    // The same stream for two nodes in longitude/latitude south of the equator: a (151.20 E,
    // 33.87 S) and b (151.21 E, 33.87 S). By the UTM grid's definition the zone is
    // floor((151.205 + 180) / 6) + 1 = 56, south: the bytes 1 (projected), 56, 1 (south). The
    // positions are the projection's (tested against the UTM definition in tests/geometry.rs);
    // here the stream is judged. Link ab a→b for bike and walk, 100 m; bike 36 km/h = 10 m/s,
    // walk 7.2 km/h = 2 m/s; a link without cars: no car speed (NaN), no lanes, infinite capacity.
    let (a, b) = (LonLat::new(151.20, -33.87), LonLat::new(151.21, -33.87));
    let mut builder = NetworkBuilder::new();
    builder.add_node("a", Position::LonLat(a));
    builder.add_node("b", Position::LonLat(b));
    let path = LinkRow {
        modes: ModeSet::of(&[Mode::Bike, Mode::Walk]),
        lanes: None,
        car_speed_km_h: None,
        length_m: Some(100.0),
        capacity_veh_h: None,
        signal_at_downstream_end: false,
    };
    builder.add_link("ab", "a", "b", path);
    let values = BuildValues { capacity_per_lane: 1_800.0, bike_speed: 36.0, walk_speed: 7.2 };
    let network = builder.build(values).expect("a network");
    let projection = Projection::for_extent([a, b]).expect("a projection");
    let mut s: Vec<u8> = Vec::new();
    let f = |s: &mut Vec<u8>, x: f64| s.extend(x.to_bits().to_le_bytes());
    let n = |s: &mut Vec<u8>, x: u32| s.extend(x.to_le_bytes());
    let text = |s: &mut Vec<u8>, x: &str| {
        s.extend((x.len() as u64).to_le_bytes());
        s.extend(x.as_bytes());
    };
    s.extend([1, 56, 1]); // projected, zone 56, south
    f(&mut s, 10.0); // bike speed, m/s
    f(&mut s, 2.0); // walk speed, m/s
    n(&mut s, 2); // nodes
    for (id, at) in [("a", a), ("b", b)] {
        let xy = projection.project(at);
        text(&mut s, id);
        f(&mut s, xy.x);
        f(&mut s, xy.y);
        s.push(0); // not signalised
    }
    n(&mut s, 1); // links
    text(&mut s, "ab");
    n(&mut s, 0); // upstream a
    n(&mut s, 1); // downstream b
    f(&mut s, 100.0); // length, m
    s.push(0); // no lanes
    s.push(0b110); // bike, walk: bits 1, 2
    f(&mut s, f64::NAN); // no car speed
    f(&mut s, f64::INFINITY); // no capacity limit
    s.push(0); // no signal downstream
    let mut h = Fingerprint::new();
    h.bytes(&s);
    assert_eq!(network.fingerprint(), h.finish());
}
