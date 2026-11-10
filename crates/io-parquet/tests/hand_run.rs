//! A run whose every value is derived by hand, written to the three files. This test checks the
//! files exist, their row counts, and that a second write of the same run is byte-identical; the
//! values, the column names, the Arrow types and the nullability are checked by another tool,
//! `read_back.py` (pyarrow), against the hand numbers written in that script:
//!
//! ```text
//! cargo test -p openmobisim-io-parquet --test hand_run
//! python3 crates/io-parquet/tests/read_back.py target/tmp/io-parquet-hand-run
//! ```
//!
//! The network: nodes a (0, 0), b (1 500, 0), c (2 500, 0) metres; a path `ab` a→b of 1 500 m
//! for bikes (15 km/h: 360 s) and walkers (4.8 km/h: 1 125 s); a street `st1` a→b of 2 000 m at
//! 50 km/h (144 s), 1 800 veh/h (a car every 2 s); a street `st2` b→c of 1 000 m at 36 km/h
//! (100 s), 900 veh/h (a car every 4 s). The window from 07:00:00 to 08:00:00, that is
//! S = 25 200 s to 28 800 s of the clock (seconds after midnight), in bins of 60 s, three
//! iterations, the least cost on time alone (a coefficient of 1 on each time column, in
//! minutes, 0 on the rest: a cost is the door-to-door time in minutes, seconds / 60). Every
//! trip starts and ends at a node (no gaps).
//! The window does not start at 0, so a time written after the window's start instead of on
//! the clock differs from the hand number by S. Below, times after the window's start come
//! first, then the clock: S + that.
//!
//! - `car1`, `car2`, `car3` (class `commute`), a→c at 0, 1 and 2 s after the start (on the
//!   clock 25 200 + 0, 1, 2 = 25 200, 25 201, 25 202 s): on `st1` they leave at
//!   max(t + 144, previous + 2) = 144, 146, 148 s after the start, on the clock
//!   25 200 + 144, 146, 148 = 25 344, 25 346, 25 348 s; on `st2` at max(t + 100, previous + 4)
//!   = 244, 248, 252 s, on the clock 25 200 + 244, 248, 252 = 25 444, 25 448, 25 452 s, which are
//!   their arrivals. Their travel times 244, 247, 250 s (durations, the same on either clock):
//!   25 200 + 244 = 25 444, 25 201 + 247 = 25 448, 25 202 + 250 = 25 452. They enter `st2` at
//!   144 to 148 s after the start, in the bin from 120 s, on the clock 25 200 + 120 = 25 320 s.
//! - `bike` a→b at 0 (25 200): 360 s, arriving at 25 200 + 360 = 25 560. `walk` a→b at 0
//!   (25 200): 1 125 s, arriving at 25 200 + 1 125 = 26 325. Both enter `ab` in the bin from
//!   25 200. `walk-late` a→b at 3 000 s after the start (25 200 + 3 000 = 28 200), entering `ab`
//!   in the bin from 28 200: it would arrive at 4 125 s, past the window's end at 3 600 s
//!   (28 800), so it is unfinished, its arrival null, its cost 1 125 / 60 = 18.75 min. `lost`, transit only and
//!   no timetable, at 0 (25 200): no alternative, its mode, arrival and cost null (all class
//!   `leisure`).

use std::path::{Path, PathBuf};

use openmobisim_demand::demand::Demand;
use openmobisim_demand::input::TripRow;
use openmobisim_graph::geometry::{Position, Xy};
use openmobisim_graph::network::{BuildValues, LinkRow, Network, NetworkBuilder};
use openmobisim_io_parquet::{WriteError, write};
use openmobisim_loop::parameters::{ChoiceModel, Parameters};
use openmobisim_loop::run::{Run, run};
use openmobisim_types::mode::{Mode, ModeSet};
use openmobisim_types::time::Second;
use openmobisim_types::units::Metres;

/// The build values of the rulings.
const VALUES: BuildValues =
    BuildValues { capacity_per_lane: 1_800.0, bike_speed: 15.0, walk_speed: 4.8 };

/// Time alone: 1 on each of the nine time columns (minutes), 0 on the rest.
const TIME_ONLY: [f64; 16] =
    [1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];

/// The window's start, 07:00:00: seconds of the clock.
const START: u32 = 25_200;

/// A one-lane car street of `length_m` at `speed_km_h` passing `capacity_veh_h`.
fn street(length_m: f64, speed_km_h: f64, capacity_veh_h: f64) -> LinkRow {
    LinkRow {
        modes: ModeSet::of(&[Mode::Car]),
        lanes: Some(1),
        car_speed_km_h: Some(speed_km_h),
        length_m: Some(length_m),
        capacity_veh_h: Some(capacity_veh_h),
        signal_at_downstream_end: false,
    }
}

fn network() -> Network {
    let mut b = NetworkBuilder::new();
    for (id, x) in [("a", 0.0), ("b", 1_500.0), ("c", 2_500.0)] {
        b.add_node(id, Position::Metres(Xy::new(x, 0.0)));
    }
    let path = LinkRow {
        modes: ModeSet::of(&[Mode::Bike, Mode::Walk]),
        lanes: None,
        car_speed_km_h: None,
        length_m: Some(1_500.0),
        capacity_veh_h: None,
        signal_at_downstream_end: false,
    };
    b.add_link("ab", "a", "b", path);
    b.add_link("st1", "a", "b", street(2_000.0, 50.0, 1_800.0));
    b.add_link("st2", "b", "c", street(1_000.0, 36.0, 900.0));
    b.build(VALUES).expect("a network")
}

fn demand(network: &Network) -> Demand {
    let row = |id: &str, to: f64, departure: u32, class: &str, mode: Mode| TripRow {
        id: id.to_string(),
        origin: Position::Metres(Xy::new(0.0, 0.0)),
        destination: Position::Metres(Xy::new(to, 0.0)),
        departure: Second(departure),
        class: class.to_string(),
        modes: ModeSet::of(&[mode]),
    };
    let rows = [
        row("car1", 2_500.0, START, "commute", Mode::Car),
        row("car2", 2_500.0, START + 1, "commute", Mode::Car),
        row("car3", 2_500.0, START + 2, "commute", Mode::Car),
        row("bike", 1_500.0, START, "leisure", Mode::Bike),
        row("walk", 1_500.0, START, "leisure", Mode::Walk),
        row("walk-late", 1_500.0, START + 3_000, "leisure", Mode::Walk),
        row("lost", 1_500.0, START, "leisure", Mode::Transit),
    ];
    let classes = ["commute".to_string(), "leisure".to_string()];
    Demand::build(&rows, &classes, network, None, Metres(400.0)).expect("a demand")
}

fn hand_run(network: &Network, demand: &Demand) -> Run {
    let parameters = Parameters {
        link_bin: 60,
        iteration_cap: 3,
        choice: ChoiceModel::Deterministic,
        coefficients: vec![TIME_ONLY; 2],
        ..Parameters::defaults(2)
    };
    run(network, demand, None, (Second(START), Second(START + 3_600)), &parameters).expect("a run")
}

/// The folder `read_back.py` reads.
fn folder(name: &str) -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR")).join(name)
}

#[test]
fn the_hand_run_writes_three_files_of_7_4_and_3_rows_byte_identical_twice() {
    let net = network();
    let dem = demand(&net);
    let r = hand_run(&net, &dem);
    let first = write(&r, &net, &dem, &folder("io-parquet-hand-run")).expect("written");
    let again = write(&r, &net, &dem, &folder("io-parquet-hand-run-again")).expect("written");
    for (table, rows) in [(&first.trips, 7), (&first.links, 4), (&first.iterations, 3)] {
        assert!(table.path.is_file(), "{}", table.path.display());
        assert_eq!(table.rows, rows, "{}", table.path.display());
    }
    for (a, b) in [
        (&first.trips, &again.trips),
        (&first.links, &again.links),
        (&first.iterations, &again.iterations),
    ] {
        let (x, y) = (std::fs::read(&a.path).expect("read"), std::fs::read(&b.path).expect("read"));
        assert!(x == y, "{} and {} differ", a.path.display(), b.path.display());
    }
    println!(
        "written: {} bytes, {} bytes, {} bytes",
        std::fs::metadata(&first.trips.path).expect("a file").len(),
        std::fs::metadata(&first.links.path).expect("a file").len(),
        std::fs::metadata(&first.iterations.path).expect("a file").len()
    );
}

#[test]
fn a_demand_or_a_network_not_the_runs_is_refused() {
    let net = network();
    let dem = demand(&net);
    let r = hand_run(&net, &dem);
    let other = Demand::build(&[], &["commute".to_string()], &net, None, Metres(400.0))
        .expect("an empty demand");
    let out = folder("io-parquet-refused");
    assert!(matches!(
        write(&r, &net, &other, &out),
        Err(WriteError::NotTheRunsDemand { run_trips: 7, demand_trips: 0 })
    ));
    let mut b = NetworkBuilder::new();
    b.add_node("a", Position::Metres(Xy::new(0.0, 0.0)));
    b.add_node("b", Position::Metres(Xy::new(1_500.0, 0.0)));
    b.add_link("ab", "a", "b", street(1_500.0, 50.0, 1_800.0));
    let small = b.build(VALUES).expect("a network");
    assert!(matches!(
        write(&r, &small, &dem, &out),
        Err(WriteError::NotTheRunsNetwork { link: 1, network_links: 1 })
    ));
    assert!(!out.exists(), "nothing written on a refusal");
}

#[test]
fn a_folder_that_cannot_be_made_is_an_io_error() {
    let net = network();
    let dem = demand(&net);
    let r = hand_run(&net, &dem);
    let blocker = folder("io-parquet-a-file");
    std::fs::write(&blocker, b"a file, not a folder").expect("a file");
    assert!(matches!(write(&r, &net, &dem, &blocker.join("inside")), Err(WriteError::Io(_))));
}
