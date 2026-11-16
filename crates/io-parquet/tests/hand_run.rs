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
use openmobisim_io_gtfs::Report as GtfsReport;
use openmobisim_io_parquet::{Files, Given, Source, Transit, WriteError, write};
use openmobisim_loop::parameters::{ChoiceModel, Parameters};
use openmobisim_loop::run::{Run, run};
use openmobisim_transit::timetable::TimetableBuilder;
use openmobisim_types::mode::{Mode, ModeSet};
use openmobisim_types::time::Second;
use openmobisim_types::units::Metres;

/// The default per-lane capacity (veh/h), bike speed and walking speed (km/h) of `PARAMETERS.md`.
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
    Demand::build(&rows, &classes, network, None, Metres(STOP_WALK_MAX)).expect("a demand")
}

/// The `stop_walk_max` the demand is built with, metres.
const STOP_WALK_MAX: f64 = 400.0;

fn parameters() -> Parameters {
    Parameters {
        link_bin: 60,
        iteration_cap: 3,
        choice: ChoiceModel::Deterministic,
        coefficients: vec![TIME_ONLY; 2],
        ..Parameters::defaults(2)
    }
}

fn hand_run(network: &Network, demand: &Demand) -> Run {
    run(network, demand, None, (Second(START), Second(START + 3_600)), &parameters())
        .expect("a run")
}

/// What the hand run was given besides its network and demand.
fn given(parameters: &Parameters) -> Given<'_> {
    Given {
        source: Source::Tables,
        files: Files::default(),
        build_values: VALUES,
        stop_walk_max: STOP_WALK_MAX,
        transit: None,
        parameters,
    }
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
    let p = parameters();
    let first = write(&r, &net, &dem, &given(&p), &folder("io-parquet-hand-run")).expect("written");
    let again =
        write(&r, &net, &dem, &given(&p), &folder("io-parquet-hand-run-again")).expect("written");
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
    let p = parameters();
    let other = Demand::build(&[], &["commute".to_string()], &net, None, Metres(400.0))
        .expect("an empty demand");
    let out = folder("io-parquet-refused");
    assert!(matches!(
        write(&r, &net, &other, &given(&p), &out),
        Err(WriteError::NotTheRunsDemand { run_trips: 7, demand_trips: 0 })
    ));
    let mut b = NetworkBuilder::new();
    b.add_node("a", Position::Metres(Xy::new(0.0, 0.0)));
    b.add_node("b", Position::Metres(Xy::new(1_500.0, 0.0)));
    b.add_link("ab", "a", "b", street(1_500.0, 50.0, 1_800.0));
    let small = b.build(VALUES).expect("a network");
    assert!(matches!(
        write(&r, &small, &dem, &given(&p), &out),
        Err(WriteError::NotTheRunsNetwork { link: 1, network_links: 1 })
    ));
    assert!(!out.exists(), "nothing written on a refusal");
}

#[test]
fn a_folder_that_cannot_be_made_is_an_io_error() {
    let net = network();
    let dem = demand(&net);
    let r = hand_run(&net, &dem);
    let p = parameters();
    let blocker = folder("io-parquet-a-file");
    std::fs::write(&blocker, b"a file, not a folder").expect("a file");
    assert!(matches!(
        write(&r, &net, &dem, &given(&p), &blocker.join("inside")),
        Err(WriteError::Io(_))
    ));
}

#[test]
fn the_record_is_written_beside_the_tables_and_byte_identical_twice() {
    // Its keys and values are checked by another tool, `read_record.py` (Python's `tomllib`),
    // against the numbers written in that script, on the folder of the first test above.
    let net = network();
    let dem = demand(&net);
    let r = hand_run(&net, &dem);
    let p = parameters();
    let first = write(&r, &net, &dem, &given(&p), &folder("io-parquet-record")).expect("written");
    let again =
        write(&r, &net, &dem, &given(&p), &folder("io-parquet-record-again")).expect("written");
    assert_eq!(first.record, folder("io-parquet-record").join("run.toml"));
    let (x, y) =
        (std::fs::read(&first.record).expect("read"), std::fs::read(&again.record).expect("read"));
    assert!(x == y, "{} and {} differ", first.record.display(), again.record.display());
}

#[test]
fn a_record_value_toml_cannot_hold_is_refused_by_name_and_nothing_written() {
    // The record holds finite numbers only (TOML's nan and inf are never written), and TOML 1.0
    // has no integer above 2^63 - 1: a coefficient that is not a number and a seed of 2^64 - 1
    // are refused, naming the key.
    let net = network();
    let dem = demand(&net);
    let r = hand_run(&net, &dem);
    let out = folder("io-parquet-not-writable");
    let mut p = parameters();
    p.coefficients[1][2] = f64::NAN;
    let Err(WriteError::NotWritable { key, .. }) = write(&r, &net, &dem, &given(&p), &out) else {
        panic!("a coefficient that is not a number was written");
    };
    assert_eq!(key, "parameters.coefficients.\"leisure\".walk_min");
    let mut p = parameters();
    p.seed = u64::MAX;
    let Err(WriteError::NotWritable { key, .. }) = write(&r, &net, &dem, &given(&p), &out) else {
        panic!("a seed above 2^63 - 1 was written");
    };
    assert_eq!(key, "parameters.seed");
    assert!(!out.exists(), "nothing written on a refusal");
}

#[test]
fn parameters_or_a_transit_not_the_runs_are_refused_and_nothing_written() {
    let net = network();
    let dem = demand(&net);
    let r = hand_run(&net, &dem);
    let out = folder("io-parquet-not-the-runs");
    let p = Parameters { link_bin: 300, ..parameters() };
    assert!(matches!(
        write(&r, &net, &dem, &given(&p), &out),
        Err(WriteError::NotTheRunsParameters(_))
    ));
    let p = Parameters { coefficients: vec![TIME_ONLY; 3], ..parameters() };
    assert!(matches!(
        write(&r, &net, &dem, &given(&p), &out),
        Err(WriteError::NotTheRunsParameters(_))
    ));
    let mut b = TimetableBuilder::new();
    b.add_stop("S", (Metres(0.0), Metres(0.0)));
    let timetable = b.build().expect("a timetable of one stop");
    let report = GtfsReport::default();
    let p = parameters();
    let transit = Transit { timetable: &timetable, report: &report, transfer_walk_max: 400.0 };
    let with_transit = Given { transit: Some(transit), ..given(&p) };
    assert!(matches!(
        write(&r, &net, &dem, &with_transit, &out),
        Err(WriteError::NotTheRunsTransit { demand_stops: 0, transit_stops: 1 })
    ));
    assert!(!out.exists(), "nothing written on a refusal");
}

#[test]
fn build_values_whose_speeds_are_not_the_networks_are_refused_and_nothing_written() {
    // The network was built at 15 km/h for bikes and 4.8 km/h for walkers; a record saying
    // 16 km/h for bikes, or 5 km/h for walkers, would be false.
    let net = network();
    let dem = demand(&net);
    let r = hand_run(&net, &dem);
    let out = folder("io-parquet-not-the-speeds");
    let p = parameters();
    for other in
        [BuildValues { bike_speed: 16.0, ..VALUES }, BuildValues { walk_speed: 5.0, ..VALUES }]
    {
        let wrong = Given { build_values: other, ..given(&p) };
        assert!(
            matches!(
                write(&r, &net, &dem, &wrong, &out),
                Err(WriteError::NotTheNetworksSpeeds { .. })
            ),
            "{other:?}"
        );
    }
    assert!(!out.exists(), "nothing written on a refusal");
}

#[test]
fn a_failed_write_into_a_used_folder_leaves_no_record_beside_the_tables() {
    // A folder holding a record from an earlier run, where `links.parquet` cannot be written
    // (a folder of that name): the write fails, and the old record is gone, so no record
    // describes tables it did not write.
    let net = network();
    let dem = demand(&net);
    let r = hand_run(&net, &dem);
    let p = parameters();
    let out = folder("io-parquet-used-folder");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(out.join("links.parquet")).expect("a folder");
    std::fs::write(out.join("run.toml"), "# an earlier run's record\n").expect("a file");
    assert!(matches!(write(&r, &net, &dem, &given(&p), &out), Err(WriteError::Io(_))));
    assert!(!out.join("run.toml").exists(), "the earlier record is left beside new tables");
}

#[test]
fn without_a_class_the_record_has_no_coefficients_comment_or_table() {
    let net = network();
    let none = Demand::build(&[], &[], &net, None, Metres(STOP_WALK_MAX)).expect("no trips");
    let p = Parameters { link_bin: 60, iteration_cap: 1, ..Parameters::defaults(0) };
    let r = run(&net, &none, None, (Second(START), Second(START + 3_600)), &p).expect("a run");
    let w = write(&r, &net, &none, &given(&p), &folder("io-parquet-no-class")).expect("written");
    let text = std::fs::read_to_string(&w.record).expect("read");
    assert!(!text.contains("coefficients"), "{text}");
}

#[test]
fn a_refused_write_into_a_folder_holding_a_complete_run_leaves_its_four_files_unchanged() {
    // Every refusal comes before anything is touched: wrong speeds, parameters not the run's
    // (another link bin) and a value the record cannot hold (a coefficient not a number).
    let net = network();
    let dem = demand(&net);
    let r = hand_run(&net, &dem);
    let p = parameters();
    let out = folder("io-parquet-kept");
    let w = write(&r, &net, &dem, &given(&p), &out).expect("written");
    let paths = [&w.trips.path, &w.links.path, &w.iterations.path, &w.record];
    let before: Vec<Vec<u8>> = paths.iter().map(|f| std::fs::read(f).expect("read")).collect();
    let speeds = BuildValues { walk_speed: 5.0, ..VALUES };
    let bin = Parameters { link_bin: 300, ..parameters() };
    let mut nan = parameters();
    nan.coefficients[0][0] = f64::NAN;
    let refused = [
        write(&r, &net, &dem, &Given { build_values: speeds, ..given(&p) }, &out),
        write(&r, &net, &dem, &given(&bin), &out),
        write(&r, &net, &dem, &given(&nan), &out),
    ];
    assert!(matches!(refused[0], Err(WriteError::NotTheNetworksSpeeds { .. })));
    assert!(matches!(refused[1], Err(WriteError::NotTheRunsParameters(_))));
    assert!(matches!(refused[2], Err(WriteError::NotWritable { .. })));
    let after: Vec<Vec<u8>> = paths.iter().map(|f| std::fs::read(f).expect("read")).collect();
    assert!(before == after, "a refused write changed a file of the earlier run");
}
