//! A run whose every value is derived by hand, written to the four files. This test checks the
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
use openmobisim_types::repairs::Repairs;
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
fn given<'a>(parameters: &'a Parameters, demand: &'a Demand) -> Given<'a> {
    Given {
        source: Source::Tables,
        files: Files::default(),
        build_values: VALUES,
        stop_walk_max: STOP_WALK_MAX,
        transit: None,
        parameters,
        network_repairs: &NO_REPAIRS,
        trips_given: u64::from(demand.trip_count()),
        demand_repairs: demand.repairs(),
    }
}

/// No repair: the hand run's tables need none.
static NO_REPAIRS: Repairs = Repairs::new();

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
    let first =
        write(&r, &net, &dem, &given(&p, &dem), &folder("io-parquet-hand-run")).expect("written");
    let again = write(&r, &net, &dem, &given(&p, &dem), &folder("io-parquet-hand-run-again"))
        .expect("written");
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
        write(&r, &net, &other, &given(&p, &other), &out),
        Err(WriteError::NotTheRunsDemand { run_trips: 7, demand_trips: 0 })
    ));
    let mut b = NetworkBuilder::new();
    b.add_node("a", Position::Metres(Xy::new(0.0, 0.0)));
    b.add_node("b", Position::Metres(Xy::new(1_500.0, 0.0)));
    b.add_link("ab", "a", "b", street(1_500.0, 50.0, 1_800.0));
    let small = b.build(VALUES).expect("a network");
    assert!(matches!(
        write(&r, &small, &dem, &given(&p, &dem), &out),
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
        write(&r, &net, &dem, &given(&p, &dem), &blocker.join("inside")),
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
    let first =
        write(&r, &net, &dem, &given(&p, &dem), &folder("io-parquet-record")).expect("written");
    let again = write(&r, &net, &dem, &given(&p, &dem), &folder("io-parquet-record-again"))
        .expect("written");
    assert_eq!(first.record, folder("io-parquet-record").join("run_record.toml"));
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
    let Err(WriteError::NotWritable { key, .. }) = write(&r, &net, &dem, &given(&p, &dem), &out)
    else {
        panic!("a coefficient that is not a number was written");
    };
    assert_eq!(key, "parameters.coefficients.\"leisure\".walk_min");
    let mut p = parameters();
    p.seed = u64::MAX;
    let Err(WriteError::NotWritable { key, .. }) = write(&r, &net, &dem, &given(&p, &dem), &out)
    else {
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
        write(&r, &net, &dem, &given(&p, &dem), &out),
        Err(WriteError::NotTheRunsParameters(_))
    ));
    let p = Parameters { coefficients: vec![TIME_ONLY; 3], ..parameters() };
    assert!(matches!(
        write(&r, &net, &dem, &given(&p, &dem), &out),
        Err(WriteError::NotTheRunsParameters(_))
    ));
    let mut b = TimetableBuilder::new();
    b.add_stop("S", (Metres(0.0), Metres(0.0)));
    let timetable = b.build().expect("a timetable of one stop");
    let report = GtfsReport::default();
    let p = parameters();
    let transit = Transit { timetable: &timetable, report: &report, transfer_walk_max: 400.0 };
    let with_transit = Given { transit: Some(transit), ..given(&p, &dem) };
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
        let wrong = Given { build_values: other, ..given(&p, &dem) };
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
    // A folder holding a record from an earlier run and a settings file, `run.toml`, where
    // `links.parquet` cannot be written (a folder of that name): the write fails, and the old
    // record is gone, so no record describes tables it did not write; the settings file, which
    // no run writes, is the same bytes.
    let net = network();
    let dem = demand(&net);
    let r = hand_run(&net, &dem);
    let p = parameters();
    let out = folder("io-parquet-used-folder");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(out.join("links.parquet")).expect("a folder");
    std::fs::write(out.join("run_record.toml"), "# an earlier run's record\n").expect("a file");
    let settings = "window_start_s = 25200\nwindow_end_s = 28800\n";
    std::fs::write(out.join("run.toml"), settings).expect("a file");
    assert!(matches!(write(&r, &net, &dem, &given(&p, &dem), &out), Err(WriteError::Io(_))));
    assert!(!out.join("run_record.toml").exists(), "the earlier record is left beside new tables");
    let kept = std::fs::read_to_string(out.join("run.toml")).expect("the settings file");
    assert_eq!(kept, settings, "a run changed the settings file");
}

#[test]
fn without_a_class_the_record_has_no_coefficients_comment_or_table() {
    let net = network();
    let none = Demand::build(&[], &[], &net, None, Metres(STOP_WALK_MAX)).expect("no trips");
    let p = Parameters { link_bin: 60, iteration_cap: 1, ..Parameters::defaults(0) };
    let r = run(&net, &none, None, (Second(START), Second(START + 3_600)), &p).expect("a run");
    let w =
        write(&r, &net, &none, &given(&p, &none), &folder("io-parquet-no-class")).expect("written");
    let text = std::fs::read_to_string(&w.record).expect("read");
    assert!(!text.contains("coefficients"), "{text}");
}

#[test]
fn a_refused_write_into_a_folder_holding_a_complete_run_leaves_its_five_files_unchanged() {
    // Every refusal comes before anything is touched: wrong speeds, parameters not the run's
    // (another link bin) and a value the record cannot hold (a coefficient not a number).
    let net = network();
    let dem = demand(&net);
    let r = hand_run(&net, &dem);
    let p = parameters();
    let out = folder("io-parquet-kept");
    let w = write(&r, &net, &dem, &given(&p, &dem), &out).expect("written");
    let paths = [&w.trips.path, &w.links.path, &w.iterations.path, &w.routes.path, &w.record];
    let before: Vec<Vec<u8>> = paths.iter().map(|f| std::fs::read(f).expect("read")).collect();
    let speeds = BuildValues { walk_speed: 5.0, ..VALUES };
    let bin = Parameters { link_bin: 300, ..parameters() };
    let mut nan = parameters();
    nan.coefficients[0][0] = f64::NAN;
    let refused = [
        write(&r, &net, &dem, &Given { build_values: speeds, ..given(&p, &dem) }, &out),
        write(&r, &net, &dem, &given(&bin, &dem), &out),
        write(&r, &net, &dem, &given(&nan, &dem), &out),
    ];
    assert!(matches!(refused[0], Err(WriteError::NotTheNetworksSpeeds { .. })));
    assert!(matches!(refused[1], Err(WriteError::NotTheRunsParameters(_))));
    assert!(matches!(refused[2], Err(WriteError::NotWritable { .. })));
    let after: Vec<Vec<u8>> = paths.iter().map(|f| std::fs::read(f).expect("read")).collect();
    assert!(before == after, "a refused write changed a file of the earlier run");
}

/// The fourth file: per trip with an alternative a gap of 0 s (every door at a node), its route
/// and a gap of 0 s; `lost` none: 6 × 3 = 18 rows, the same bytes twice. Each of the four tables
/// read back by the crate's readers is what was written: the loop's trips, links, iterations
/// and the legs each trip took (the values themselves are `read_back.py`'s, by pyarrow).
#[test]
fn the_routes_file_has_18_rows_and_each_table_reads_back_as_written() {
    let net = network();
    let dem = demand(&net);
    let r = hand_run(&net, &dem);
    let p = parameters();
    let first =
        write(&r, &net, &dem, &given(&p, &dem), &folder("io-parquet-routes")).expect("written");
    let again = write(&r, &net, &dem, &given(&p, &dem), &folder("io-parquet-routes-again"))
        .expect("written");
    assert_eq!(first.routes.rows, 18);
    assert_eq!(first.routes.path, folder("io-parquet-routes").join("routes.parquet"));
    let bytes = |t: &openmobisim_io_parquet::Table| std::fs::read(&t.path).expect("read");
    assert!(bytes(&first.routes) == bytes(&again.routes), "two writes differ");
    let tables = openmobisim_io_parquet::read(
        &folder("io-parquet-routes"),
        &dem,
        &net,
        None,
        r.links.window(),
        r.dropped(),
    )
    .expect("read back");
    let same =
        |a: &[f64], b: &[f64]| a.iter().map(|x| x.to_bits()).eq(b.iter().map(|x| x.to_bits()));
    // What the pages read of the trips: the mode, the realised time and whether it finished.
    let (t, w) = (&tables.trips, &r.trips);
    assert_eq!((&t.mode, &t.finished), (&w.mode, &w.finished));
    assert!(same(&t.travel_time, &w.travel_time));
    let l = &tables.links;
    assert_eq!(l.link, r.links.links());
    assert_eq!(l.bin, r.links.bins().iter().map(|b| b.0).collect::<Vec<_>>());
    assert_eq!(
        (&l.cars[..], &l.bikes[..], &l.walkers[..]),
        (r.links.cars(), r.links.bikes(), r.links.walkers())
    );
    assert!(same(&l.car_time, &r.links.car_time().iter().map(|d| d.get()).collect::<Vec<_>>()));
    let (iterations, _) = openmobisim_io_parquet::read_iterations(
        &folder("io-parquet-routes").join("iterations.parquet"),
    )
    .expect("read back");
    assert_eq!(iterations.len(), r.iterations.len());
    for (a, b) in iterations.iter().zip(&r.iterations) {
        assert_eq!(format!("{a:?}"), format!("{b:?}"));
    }
    let legs: Vec<Option<usize>> = tables.routes.iter().map(|l| l.as_ref().map(Vec::len)).collect();
    let three = Some(3);
    assert_eq!(
        legs,
        [three, three, three, three, Some(0), three, three],
        "bike, car1, car2, car3, lost, walk, walk-late"
    );
    for (trip, legs) in tables.routes.iter().enumerate() {
        let taken =
            openmobisim_io_parquet::agrees(r.trips.mode[trip], legs.as_deref().unwrap_or(&[]));
        assert!(taken, "trip {trip}");
    }
    assert!(tables.warnings.is_empty(), "{:?}", tables.warnings);
}

/// Who would change, written and read back. Each model reports the one measure that can reach 0.
/// The hand run under the logit (time alone, the same coefficients): every trip has one
/// alternative, which the logit chooses at any cost, so nobody would change: `would_change_share`
/// is 0 at each of the three iterations, the column right after `found_share`; the gap is not
/// reported under the logit: null. The hand run as it is, under the least cost: the share is not
/// reported, null, and the gap is 0 (each trip takes its set's least). `read_iterations` reads
/// both back as written (null as `NaN`), without a warning. The logit's file without the column,
/// as a run written before that share was reported has it, opens: every other value as written,
/// the share unknown (`NaN`), and one warning naming the column.
#[test]
fn the_share_that_would_change_is_written_and_read_back_and_a_file_without_it_opens_with_a_warning()
{
    use openmobisim_io_parquet::{Cells, read_iterations, read_table};
    use parquet::arrow::ArrowWriter;
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

    let net = network();
    let dem = demand(&net);
    let p = Parameters { choice: ChoiceModel::Logit, ..parameters() };
    let r = run(&net, &dem, None, (Second(START), Second(START + 3_600)), &p).expect("a run");
    let written = write(&r, &net, &dem, &given(&p, &dem), &folder("io-parquet-would-change"))
        .expect("written");
    let table = read_table(&written.iterations.path).expect("read");
    let names = table.names();
    let at = |name: &str| names.iter().position(|n| n == name).expect("a column");
    assert_eq!(at("would_change_share"), at("found_share") + 1, "right after found_share");
    assert_eq!(
        table.column("would_change_share").expect("a column"),
        Cells::Float(vec![Some(0.0); 3])
    );
    assert_eq!(table.column("gap").expect("a column"), Cells::Float(vec![None; 3]));
    let (rows, warnings) = read_iterations(&written.iterations.path).expect("read back");
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(format!("{rows:?}"), format!("{:?}", r.iterations), "as written");
    assert!(rows.iter().all(|it| it.would_change_share == 0.0 && it.gap.is_nan()));

    // The least cost: the share null, the gap 0.
    let (least_cost, q) = (hand_run(&net, &dem), parameters());
    let written_least =
        write(&least_cost, &net, &dem, &given(&q, &dem), &folder("io-parquet-least-cost"))
            .expect("written");
    let table = read_table(&written_least.iterations.path).expect("read");
    assert_eq!(table.column("would_change_share").expect("a column"), Cells::Float(vec![None; 3]));
    assert_eq!(table.column("gap").expect("a column"), Cells::Float(vec![Some(0.0); 3]));
    let (rows, warnings) = read_iterations(&written_least.iterations.path).expect("read back");
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(format!("{rows:?}"), format!("{:?}", least_cost.iterations), "as written");

    // The same file without the column.
    let file = std::fs::File::open(&written.iterations.path).expect("open");
    let batches: Vec<_> = ParquetRecordBatchReaderBuilder::try_new(file)
        .expect("a Parquet file")
        .build()
        .expect("a reader")
        .collect::<Result<_, _>>()
        .expect("batches");
    let old = folder("io-parquet-without-would-change");
    std::fs::create_dir_all(&old).expect("a folder");
    let path = old.join("iterations.parquet");
    let mut writer: Option<ArrowWriter<std::fs::File>> = None;
    for mut batch in batches {
        batch.remove_column(at("would_change_share"));
        let w = writer.get_or_insert_with(|| {
            let out = std::fs::File::create(&path).expect("a file");
            ArrowWriter::try_new(out, batch.schema(), None).expect("a writer")
        });
        w.write(&batch).expect("written");
    }
    writer.expect("a batch").close().expect("closed");
    assert!(!read_table(&path).expect("read").names().iter().any(|n| n == "would_change_share"));
    let (rows, warnings) = read_iterations(&path).expect("it opens");
    let unknown: Vec<_> = r
        .iterations
        .iter()
        .map(|it| openmobisim_loop::assess::Iteration {
            would_change_share: f64::NAN,
            ..it.clone()
        })
        .collect();
    assert_eq!(format!("{rows:?}"), format!("{unknown:?}"), "every other value as written");
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("would_change_share"), "{}", warnings[0]);
    println!("{}", warnings[0]);
}

/// A links table edited by hand: the hand run's row of `st2` (3 cars entering in the bin from
/// 25 320 s) given to `ab`, a path for bikes and walkers that does not allow cars, and given a
/// bike as well. A run writes no such row; the table opens, that row set aside whole, its bike
/// with its cars, and the three others read as written (`ab` from 25 200 s, a bike and a walker;
/// `ab` from 28 200 s, a walker; `st1` from 25 200 s, 3 cars): 1 bike in all, the hand run's.
/// Exactly one warning names the row.
#[test]
fn a_links_row_with_cars_on_a_link_without_cars_is_set_aside_with_one_warning() {
    use arrow_array::{Array, ArrayRef, RecordBatch, StringArray, UInt32Array};
    use openmobisim_types::ids::{EntityId, LinkId};
    use parquet::arrow::ArrowWriter;
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

    let net = network();
    let dem = demand(&net);
    let (r, p) = (hand_run(&net, &dem), parameters());
    let dir = folder("io-parquet-cars-on-a-path");
    write(&r, &net, &dem, &given(&p, &dem), &dir).expect("written");
    let path = dir.join("links.parquet");
    let file = std::fs::File::open(&path).expect("open");
    let batches: Vec<RecordBatch> = ParquetRecordBatchReaderBuilder::try_new(file)
        .expect("a Parquet file")
        .build()
        .expect("a reader")
        .collect::<Result<_, _>>()
        .expect("batches");
    let mut writer = ArrowWriter::try_new(
        std::fs::File::create(&path).expect("a file"),
        batches[0].schema(),
        None,
    )
    .expect("a writer");
    let mut moved = 0;
    for batch in batches {
        let at = batch.schema().index_of("link_id").expect("a column");
        let ids = batch.column(at).as_any().downcast_ref::<StringArray>().expect("text");
        let edited: StringArray = ids
            .iter()
            .map(|id| {
                moved += usize::from(id == Some("st2"));
                id.map(|id| if id == "st2" { "ab" } else { id })
            })
            .collect();
        let b = batch.schema().index_of("bikes").expect("a column");
        let bikes = batch.column(b).as_any().downcast_ref::<UInt32Array>().expect("counts");
        let with_a_bike: UInt32Array = (0..bikes.len())
            .map(|k| Some(if ids.value(k) == "st2" { 1 } else { bikes.value(k) }))
            .collect();
        let mut columns: Vec<ArrayRef> = batch.columns().to_vec();
        columns[at] = std::sync::Arc::new(edited);
        columns[b] = std::sync::Arc::new(with_a_bike);
        writer.write(&RecordBatch::try_new(batch.schema(), columns).expect("a batch")).expect("ok");
    }
    writer.close().expect("closed");
    assert_eq!(moved, 1, "the hand run has one row of st2");

    let (links, warnings) =
        openmobisim_io_parquet::read_links(&path, &net, r.links.window()).expect("it opens");
    let id = |name: &str| LinkId::new(net.link_ids().id_of(name).expect("a link"));
    assert_eq!(links.link, [id("ab"), id("ab"), id("st1")]);
    assert_eq!(links.bin, [0, 50, 0], "from 25 200, 28 200 and 25 200 s, bins of 60 s");
    assert_eq!(
        (&links.cars[..], &links.bikes[..], &links.walkers[..]),
        (&[0, 0, 3][..], &[1, 0, 0][..], &[1, 1, 0][..])
    );
    assert_eq!(links.bikes.iter().sum::<u32>(), 1, "the edited row's bike set aside with it");
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].contains(
            "1 rows set aside: they hold cars on a link that does not allow cars: ab at 25320"
        ),
        "{}",
        warnings[0]
    );
    println!("{}", warnings[0]);
}
