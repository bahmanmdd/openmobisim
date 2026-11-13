//! The timetable's fingerprint as a property: built twice from the same rows it is the same,
//! and changing one value of one input column, all else equal, changes it, for every column a
//! timetable is built from (one named case per column).
//!
//! The rows: stops `A` (0, 0), `B` (1 000, 0), `C` (2 000, 0), `D` (2 000, 300) and `E` (0, 300)
//! metres, `E` called by no run (so its id can change alone); run `r1`
//! calls A at 08:00:00 (28 800 s), B arriving 28 980 s and leaving 29 000 s, C at 29 200 s; run
//! `r2` calls C at 29 400 s and D at 29 700 s; footpaths C to D of 250 s and D to C of 260 s. For bikes: nothing
//! here is a bike (a run carries passengers; the walks are on foot), so no case is a bike's.

use openmobisim_transit::timetable::{Call, TimetableBuilder};
use openmobisim_types::fingerprint::Fingerprint;
use openmobisim_types::time::Second;
use openmobisim_types::units::{Duration, Metres};

/// A call as a row: the stop, the arrival and the departure (seconds of the clock).
type CallRow = (&'static str, u32, u32);

#[derive(Clone)]
struct Rows {
    stops: Vec<(&'static str, f64, f64)>,
    runs: Vec<(&'static str, Vec<CallRow>)>,
    footpaths: Vec<(&'static str, &'static str, f64)>,
}

fn rows() -> Rows {
    Rows {
        stops: vec![
            ("A", 0.0, 0.0),
            ("B", 1_000.0, 0.0),
            ("C", 2_000.0, 0.0),
            ("D", 2_000.0, 300.0),
            ("E", 0.0, 300.0),
        ],
        runs: vec![
            ("r1", vec![("A", 28_800, 28_800), ("B", 28_980, 29_000), ("C", 29_200, 29_200)]),
            ("r2", vec![("C", 29_400, 29_400), ("D", 29_700, 29_700)]),
        ],
        footpaths: vec![("C", "D", 250.0), ("D", "C", 260.0)],
    }
}

fn fingerprint(r: &Rows) -> u64 {
    let mut b = TimetableBuilder::new();
    for &(id, x, y) in &r.stops {
        b.add_stop(id, (Metres(x), Metres(y)));
    }
    for (id, calls) in &r.runs {
        let calls = calls
            .iter()
            .map(|&(stop, a, d)| Call {
                stop: stop.to_string(),
                arrival: Second(a),
                departure: Second(d),
            })
            .collect();
        b.add_run(*id, calls);
    }
    for &(from, to, seconds) in &r.footpaths {
        b.add_footpath(from, to, Duration(seconds));
    }
    b.build().expect("a timetable").fingerprint()
}

/// Each case changes one value of one column, all else equal.
type Case = (&'static str, fn(&mut Rows));

const CASES: [Case; 10] = [
    ("stop id", |r| r.stops[4].0 = "F"),
    ("stop x", |r| r.stops[1].1 = 1_001.0),
    ("stop y", |r| r.stops[1].2 = 1.0),
    ("run id", |r| r.runs[1].0 = "r3"),
    ("call stop", |r| r.runs[0].1[1].0 = "D"),
    ("call arrival", |r| r.runs[0].1[1].1 = 28_990),
    ("call departure", |r| r.runs[0].1[1].2 = 29_010),
    ("footpath from", |r| r.footpaths[0].0 = "B"),
    ("footpath to", |r| r.footpaths[0].1 = "A"),
    ("footpath seconds", |r| r.footpaths[0].2 = 251.0),
];

#[test]
fn built_twice_from_the_same_rows_the_fingerprint_is_the_same() {
    assert_eq!(fingerprint(&rows()), fingerprint(&rows()));
}

#[test]
fn one_value_changed_in_any_column_changes_the_fingerprint() {
    let base = fingerprint(&rows());
    let mut unchanged = Vec::new();
    for (name, change) in CASES {
        let mut changed = rows();
        change(&mut changed);
        if fingerprint(&changed) == base {
            unchanged.push(name);
        }
    }
    assert!(unchanged.is_empty(), "unchanged by: {unchanged:?}");
}

#[test]
fn the_same_rows_in_another_order_have_the_same_fingerprint() {
    let mut reversed = rows();
    reversed.stops.reverse();
    reversed.runs.reverse();
    reversed.footpaths.reverse();
    assert_eq!(fingerprint(&reversed), fingerprint(&rows()));
}

#[test]
fn two_stops_one_run_and_one_footpath_hash_to_the_byte_stream_the_readme_gives() {
    // The README's order, written out by hand, hashed by the raw-bytes FNV-1a (anchored on the
    // published vectors in crates/types). Stops A (1, 2) and B (3, 4) metres; run r calls A
    // arriving 100 s and leaving 110 s, then B arriving 200 s and leaving 210 s; a footpath A
    // to B of 30.5 s.
    let r = Rows {
        stops: vec![("A", 1.0, 2.0), ("B", 3.0, 4.0)],
        runs: vec![("r", vec![("A", 100, 110), ("B", 200, 210)])],
        footpaths: vec![("A", "B", 30.5)],
    };
    let mut s: Vec<u8> = Vec::new();
    let f = |s: &mut Vec<u8>, x: f64| s.extend(x.to_bits().to_le_bytes());
    let n = |s: &mut Vec<u8>, x: u32| s.extend(x.to_le_bytes());
    let text = |s: &mut Vec<u8>, x: &str| {
        s.extend((x.len() as u64).to_le_bytes());
        s.extend(x.as_bytes());
    };
    n(&mut s, 2); // stops
    for (id, x, y) in [("A", 1.0, 2.0), ("B", 3.0, 4.0)] {
        text(&mut s, id);
        f(&mut s, x);
        f(&mut s, y);
    }
    n(&mut s, 1); // runs
    text(&mut s, "r");
    n(&mut s, 2); // calls
    for (stop, arrival, departure) in [(0, 100, 110), (1, 200, 210)] {
        n(&mut s, stop);
        n(&mut s, arrival);
        n(&mut s, departure);
    }
    s.extend(1_u64.to_le_bytes()); // footpaths
    n(&mut s, 0); // from A
    n(&mut s, 1); // to B
    f(&mut s, 30.5); // seconds
    let mut h = Fingerprint::new();
    h.bytes(&s);
    assert_eq!(fingerprint(&r), h.finish());
}
