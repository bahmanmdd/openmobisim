//! The demand's fingerprint as a property: built twice from the same rows it is the same, and
//! changing one value of one input column, all else equal, changes it, for every column a
//! demand is built from (one named case per column).
//!
//! The network: nodes `a` (0, 0) and `b` (1 000, 0) metres, a street each way for car, bike and
//! walk. The classes `commute`, `leisure` and `spare` (no trip's, so its id can change alone).
//! Trip `t1` (commute) from (0, 0) to (1 000, 0) at 08:00:00 (28 800 s) by car or bike; trip
//! `t2` (leisure) from (10, 5) to (990, −5) at 28 900 s on foot. For bikes: a trip's modes are a
//! case of their own, here the bike taken out of `t1`'s set.

use openmobisim_demand::demand::Demand;
use openmobisim_demand::input::TripRow;
use openmobisim_graph::geometry::{Position, Xy};
use openmobisim_graph::network::{BuildValues, LinkRow, Network, NetworkBuilder};
use openmobisim_types::fingerprint::Fingerprint;
use openmobisim_types::mode::{Mode, ModeSet};
use openmobisim_types::time::Second;
use openmobisim_types::units::Metres;

fn network() -> Network {
    let street = LinkRow {
        modes: ModeSet::of(&[Mode::Car, Mode::Bike, Mode::Walk]),
        lanes: Some(1),
        car_speed_km_h: Some(50.0),
        length_m: None,
        capacity_veh_h: None,
        signal_at_downstream_end: false,
    };
    let mut b = NetworkBuilder::new();
    b.add_node("a", Position::Metres(Xy::new(0.0, 0.0)));
    b.add_node("b", Position::Metres(Xy::new(1_000.0, 0.0)));
    b.add_link("ab", "a", "b", street);
    b.add_link("ba", "b", "a", street);
    b.build(BuildValues { capacity_per_lane: 1_800.0, bike_speed: 15.0, walk_speed: 4.8 })
        .expect("a network")
}

#[derive(Clone)]
struct Rows {
    classes: Vec<String>,
    trips: Vec<TripRow>,
}

fn rows() -> Rows {
    let trip = |id: &str, from: (f64, f64), to: (f64, f64), at: u32, class: &str, modes| TripRow {
        id: id.to_string(),
        origin: Position::Metres(Xy::new(from.0, from.1)),
        destination: Position::Metres(Xy::new(to.0, to.1)),
        departure: Second(at),
        class: class.to_string(),
        modes: ModeSet::of(modes),
    };
    Rows {
        classes: ["commute", "leisure", "spare"].map(String::from).to_vec(),
        trips: vec![
            trip("t1", (0.0, 0.0), (1_000.0, 0.0), 28_800, "commute", &[Mode::Car, Mode::Bike]),
            trip("t2", (10.0, 5.0), (990.0, -5.0), 28_900, "leisure", &[Mode::Walk]),
        ],
    }
}

fn fingerprint(network: &Network, r: &Rows) -> u64 {
    let demand = Demand::build(&r.trips, &r.classes, network, None, Metres(1_500.0));
    demand.expect("a demand").fingerprint()
}

/// Move a position by `dx` and `dy` metres.
fn shift(p: &mut Position, dx: f64, dy: f64) {
    let Position::Metres(xy) = *p else { unreachable!("the rows are in metres") };
    *p = Position::Metres(Xy::new(xy.x + dx, xy.y + dy));
}

/// Each case changes one value of one column, all else equal.
type Case = (&'static str, fn(&mut Rows));

const CASES: [Case; 9] = [
    ("class id", |r| r.classes[2] = "spare2".to_string()),
    ("trip id", |r| r.trips[1].id = "t3".to_string()),
    ("trip class", |r| r.trips[0].class = "leisure".to_string()),
    ("trip departure", |r| r.trips[0].departure = Second(28_801)),
    ("trip modes (the bike taken out)", |r| r.trips[0].modes = ModeSet::of(&[Mode::Car])),
    ("trip origin x", |r| shift(&mut r.trips[1].origin, 1.0, 0.0)),
    ("trip origin y", |r| shift(&mut r.trips[1].origin, 0.0, 1.0)),
    ("trip destination x", |r| shift(&mut r.trips[1].destination, 1.0, 0.0)),
    ("trip destination y", |r| shift(&mut r.trips[1].destination, 0.0, 1.0)),
];

#[test]
fn built_twice_from_the_same_rows_the_fingerprint_is_the_same() {
    let net = network();
    assert_eq!(fingerprint(&net, &rows()), fingerprint(&net, &rows()));
}

#[test]
fn one_value_changed_in_any_column_changes_the_fingerprint() {
    let net = network();
    let base = fingerprint(&net, &rows());
    let mut unchanged = Vec::new();
    for (name, change) in CASES {
        let mut changed = rows();
        change(&mut changed);
        if fingerprint(&net, &changed) == base {
            unchanged.push(name);
        }
    }
    assert!(unchanged.is_empty(), "unchanged by: {unchanged:?}");
}

#[test]
fn the_same_rows_in_another_order_have_the_same_fingerprint() {
    let net = network();
    let mut reversed = rows();
    reversed.classes.reverse();
    reversed.trips.reverse();
    assert_eq!(fingerprint(&net, &reversed), fingerprint(&net, &rows()));
}

#[test]
fn one_class_and_one_trip_hash_to_the_byte_stream_the_readme_gives() {
    // The README's order, written out by hand, hashed by the raw-bytes FNV-1a (anchored on the
    // published vectors in crates/types). Class c; trip t of class c at 28 800 s, by bike alone
    // (bit 1 of the mode byte), from (1, 2) to (998, 3) metres.
    let net = network();
    let r = Rows {
        classes: vec!["c".to_string()],
        trips: vec![TripRow {
            id: "t".to_string(),
            origin: Position::Metres(Xy::new(1.0, 2.0)),
            destination: Position::Metres(Xy::new(998.0, 3.0)),
            departure: Second(28_800),
            class: "c".to_string(),
            modes: ModeSet::of(&[Mode::Bike]),
        }],
    };
    let mut s: Vec<u8> = Vec::new();
    let f = |s: &mut Vec<u8>, x: f64| s.extend(x.to_bits().to_le_bytes());
    let n = |s: &mut Vec<u8>, x: u32| s.extend(x.to_le_bytes());
    let text = |s: &mut Vec<u8>, x: &str| {
        s.extend((x.len() as u64).to_le_bytes());
        s.extend(x.as_bytes());
    };
    n(&mut s, 1); // classes
    text(&mut s, "c");
    n(&mut s, 1); // trips
    text(&mut s, "t");
    n(&mut s, 0); // class c
    n(&mut s, 28_800); // departure
    s.push(0b10); // bike
    for x in [1.0, 2.0, 998.0, 3.0] {
        f(&mut s, x); // origin x, y, destination x, y
    }
    let mut h = Fingerprint::new();
    h.bytes(&s);
    assert_eq!(fingerprint(&net, &r), h.finish());
}
