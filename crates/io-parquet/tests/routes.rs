//! `routes.parquet` on a hand timetable: rides and a transfer walk, read with the Arrow reader
//! alone, and read back by the crate's reader as the legs the trip took.
//!
//! Stops s1 (0 m), s2 (1 000), s3 and s3b (2 000), s4 (3 000), on a line of footways; run r0
//! s1 → s2 → s3 at 200, 300, 400 s of the clock; run r1 s3b → s4 at 500, 600 s; a footpath s3 →
//! s3b of 30 s; the boarding slack its default, 60 s. One trip, `t`, leaves at 0 s from 100 m east
//! of s1 for s4: it walks 100 m to s1 at 4.8 km/h, 75 s (a walk gap; at the stop at 75 s, in time
//! for r0 with the slack, 135 ≤ 200), rides r0 from s1 (200 s) to s3 (400 s), walks the footpath
//! to s3b (30 s: 430 + 60 ≤ 500), rides r1 from s3b (500 s) to s4 (600 s), and walks 0 m to its
//! door (a gap of 0 s): five legs. The same with every time moved 86 100 s on: a window that
//! does not start at 0, and rides past midnight (86 300 to 86 700 s of the clock).
//!
//! The batches: 8 191, 8 192 and 8 193 walkers over one footway, the trips without an alternative
//! (transit only, no timetable) at either side of the batch boundary of 8 192 trips: the rows by
//! construction, three per walker, in trip id order.

use std::fs::File;

use arrow_array::cast::AsArray;
use arrow_array::types::{Float64Type, UInt32Type};
use openmobisim_demand::demand::Demand;
use openmobisim_demand::input::TripRow;
use openmobisim_graph::geometry::{Position, Xy};
use openmobisim_graph::network::{BuildValues, LinkRow, Network, NetworkBuilder};
use openmobisim_io_gtfs::Report as GtfsReport;
use openmobisim_io_parquet::{
    Files, Given, RouteLeg, Source, Transit, WriteError, read_routes, write,
};
use openmobisim_loop::parameters::{ChoiceModel, Parameters};
use openmobisim_loop::run::run;
use openmobisim_transit::raptor::{Footpath, Ride};
use openmobisim_transit::timetable::{Call, Timetable, TimetableBuilder};
use openmobisim_types::ids::{EntityId, StopId, TransitRunId};
use openmobisim_types::mode::{Mode, ModeSet};
use openmobisim_types::time::Second;
use openmobisim_types::units::{Duration, Metres};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

const VALUES: BuildValues =
    BuildValues { capacity_per_lane: 1_800.0, bike_speed: 15.0, walk_speed: 4.8 };
const TIME_ONLY: [f64; 16] =
    [1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];

fn network() -> Network {
    let footway = LinkRow {
        modes: ModeSet::of(&[Mode::Walk]),
        lanes: None,
        car_speed_km_h: None,
        length_m: Some(1_000.0),
        capacity_veh_h: None,
        signal_at_downstream_end: false,
    };
    let mut b = NetworkBuilder::new();
    for i in 0..4 {
        b.add_node(format!("n{i}"), Position::Metres(Xy::new(1_000.0 * f64::from(i), 0.0)));
    }
    for i in 0..3 {
        b.add_link(format!("f{i}"), format!("n{i}"), format!("n{}", i + 1), footway);
        b.add_link(format!("b{i}"), format!("n{}", i + 1), format!("n{i}"), footway);
    }
    b.build(VALUES).expect("a network")
}

fn timetable(offset: u32) -> Timetable {
    let mut b = TimetableBuilder::new();
    for (id, x) in
        [("s1", 0.0), ("s2", 1_000.0), ("s3", 2_000.0), ("s3b", 2_000.0), ("s4", 3_000.0)]
    {
        b.add_stop(id, (Metres(x), Metres(0.0)));
    }
    let call = |stop: &str, s: u32| Call {
        stop: stop.into(),
        arrival: Second(offset + s),
        departure: Second(offset + s),
    };
    b.add_run("r0", vec![call("s1", 200), call("s2", 300), call("s3", 400)]);
    b.add_run("r1", vec![call("s3b", 500), call("s4", 600)]);
    b.add_footpath("s3", "s3b", Duration(30.0));
    b.build().expect("a timetable")
}

fn parameters() -> Parameters {
    Parameters {
        link_bin: 60,
        iteration_cap: 1,
        choice: ChoiceModel::Deterministic,
        coefficients: vec![TIME_ONLY],
        ..Parameters::defaults(1)
    }
}

/// The run of `rows` on the line, with `timetable` if any, over the hour from `offset`, written
/// into `name`; its demand and the routes file.
fn written(
    net: &Network,
    tt: Option<&Timetable>,
    rows: &[TripRow],
    offset: u32,
    name: &str,
) -> (Demand, openmobisim_io_parquet::Table) {
    let classes = ["all".to_string()];
    let dem = Demand::build(rows, &classes, net, tt, Metres(1_500.0)).expect("a demand");
    let p = parameters();
    let r = run(net, &dem, tt, (Second(offset), Second(offset + 3_600)), &p).expect("a run");
    let report = GtfsReport::default();
    let given = Given {
        source: Source::Tables,
        files: Files::default(),
        build_values: VALUES,
        stop_walk_max: 1_500.0,
        transit: tt.map(|timetable| Transit {
            timetable,
            report: &report,
            transfer_walk_max: 400.0,
        }),
        parameters: &p,
        network_repairs: net.repairs(),
        trips_given: u64::from(dem.trip_count()),
        demand_repairs: dem.repairs(),
    };
    let folder = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let written = write(&r, net, &dem, &given, &folder).expect("written");
    (dem, written.routes)
}

/// The routes file read whole by the Arrow reader alone, as one batch.
fn batch(path: &std::path::Path) -> arrow_array::RecordBatch {
    let file = File::open(path).expect("the file");
    let batches: Vec<_> = ParquetRecordBatchReaderBuilder::try_new(file)
        .expect("parquet")
        .with_batch_size(1 << 20)
        .build()
        .expect("a reader")
        .collect::<Result<_, _>>()
        .expect("batches");
    assert_eq!(batches.len(), 1);
    batches.into_iter().next().expect("a batch")
}

/// The transit trip of the header with every time `offset` s on.
fn rides_by_hand(offset: u32, name: &str) {
    let (net, tt) = (network(), timetable(offset));
    let row = TripRow {
        id: "t".into(),
        origin: Position::Metres(Xy::new(100.0, 0.0)),
        destination: Position::Metres(Xy::new(3_000.0, 0.0)),
        departure: Second(offset),
        class: "all".into(),
        modes: ModeSet::of(&[Mode::Transit]),
    };
    let (dem, routes) = written(&net, Some(&tt), &[row], offset, name);
    assert_eq!(routes.rows, 5);
    let b = batch(&routes.path);
    let text = |i: usize| -> Vec<Option<String>> {
        b.column(i).as_string::<i32>().iter().map(|s| s.map(str::to_owned)).collect()
    };
    let some = |v: &[&str]| -> Vec<Option<String>> {
        v.iter().map(|s| (!s.is_empty()).then(|| (*s).to_owned())).collect()
    };
    let int = |i: usize| -> Vec<Option<u32>> {
        b.column(i).as_primitive::<UInt32Type>().iter().collect()
    };
    let on = |s: u32| Some(offset + s);
    assert_eq!(text(2), some(&["walk_gap", "ride", "transfer_walk", "ride", "walk_gap"]));
    let seconds: Vec<Option<f64>> = b.column(3).as_primitive::<Float64Type>().iter().collect();
    assert_eq!(seconds, [Some(75.0), None, Some(30.0), None, Some(0.0)]);
    assert_eq!(b.column(4).null_count(), 5, "no route on links");
    assert_eq!(text(5), some(&["", "r0", "", "r1", ""]));
    assert_eq!(text(6), some(&["", "s1", "", "s3b", ""]));
    assert_eq!(text(7), some(&["", "s3", "", "s4", ""]));
    assert_eq!(int(8), [None, on(200), None, on(500), None]);
    assert_eq!(int(9), [None, on(400), None, on(600), None]);
    assert_eq!(text(10), some(&["", "", "s3", "", ""]));
    assert_eq!(text(11), some(&["", "", "s3b", "", ""]));

    // Read back: every leg, by hand (stops in id order s1, s2, s3, s3b, s4 = 0 to 4; runs r0, r1).
    let (read, warnings) = read_routes(&routes.path, &dem, &net, Some(&tt)).expect("read back");
    assert!(warnings.is_empty(), "{warnings:?}");
    let ride = |run: u32, (a, at): (u32, u32), (b, bt): (u32, u32)| {
        RouteLeg::Ride(Ride {
            run: TransitRunId::new(run),
            board: (StopId::new(a), Second(offset + at)),
            alight: (StopId::new(b), Second(offset + bt)),
        })
    };
    let footpath = Footpath { from: StopId::new(2), to: StopId::new(3), seconds: Duration(30.0) };
    let hand = vec![
        RouteLeg::WalkGap(Duration(75.0)),
        ride(0, (0, 200), (2, 400)),
        RouteLeg::TransferWalk(footpath),
        ride(1, (3, 500), (4, 600)),
        RouteLeg::WalkGap(Duration(0.0)),
    ];
    assert_eq!(read, [Some(hand)]);
}

#[test]
fn rides_and_a_transfer_walk_are_written_and_read_back_by_hand() {
    rides_by_hand(0, "io-parquet-routes-transit");
}

#[test]
fn rides_past_midnight_in_a_window_not_at_0_are_on_the_clock() {
    rides_by_hand(86_100, "io-parquet-routes-midnight");
}

/// `n` walkers from n0 (0 m) to n1 (1 000 m), but the trips at `lost` take transit alone (no
/// timetable: no alternative); ids `w00000`, … in construction order.
fn batches_of(n: usize, lost: &[usize]) {
    let net = network();
    let rows: Vec<TripRow> = (0..n)
        .map(|i| TripRow {
            id: format!("w{i:05}"),
            origin: Position::Metres(Xy::new(0.0, 0.0)),
            destination: Position::Metres(Xy::new(1_000.0, 0.0)),
            departure: Second(0),
            class: "all".into(),
            modes: ModeSet::of(&[if lost.contains(&i) { Mode::Transit } else { Mode::Walk }]),
        })
        .collect();
    let (dem, routes) = written(&net, None, &rows, 0, &format!("io-parquet-routes-{n}"));
    let walkers: Vec<usize> = (0..n).filter(|i| !lost.contains(i)).collect();
    assert_eq!(routes.rows, 3 * walkers.len());
    let b = batch(&routes.path);
    let ids: Vec<&str> = b.column(0).as_string::<i32>().iter().map(|s| s.unwrap_or("")).collect();
    let hand: Vec<String> =
        walkers.iter().flat_map(|i| std::iter::repeat_n(format!("w{i:05}"), 3)).collect();
    assert!(ids.iter().eq(hand.iter()), "{n}: the trips' rows are not in construction order");
    let (read, warnings) = read_routes(&routes.path, &dem, &net, None).expect("read back");
    assert!(warnings.is_empty(), "{warnings:?}");
    for (i, legs) in read.iter().enumerate() {
        assert_eq!(legs.as_ref().map(Vec::len), Some(if lost.contains(&i) { 0 } else { 3 }));
    }
}

#[test]
fn the_routes_file_is_whole_at_either_side_of_a_batch() {
    batches_of(8_191, &[8_189, 8_190]);
    batches_of(8_192, &[8_190, 8_191]);
    batches_of(8_193, &[8_191, 8_192]);
}

/// A journey that rides a run the timetable given does not have (the stops alike, run r1 left
/// out) is refused before anything is written: the folder stays empty.
#[test]
fn a_journey_off_the_timetable_given_is_refused_before_anything_is_written() {
    let (net, tt) = (network(), timetable(0));
    let row = TripRow {
        id: "t".into(),
        origin: Position::Metres(Xy::new(100.0, 0.0)),
        destination: Position::Metres(Xy::new(3_000.0, 0.0)),
        departure: Second(0),
        class: "all".into(),
        modes: ModeSet::of(&[Mode::Transit]),
    };
    let classes = ["all".to_string()];
    let dem = Demand::build(&[row], &classes, &net, Some(&tt), Metres(1_500.0)).expect("a demand");
    let p = parameters();
    let r = run(&net, &dem, Some(&tt), (Second(0), Second(3_600)), &p).expect("a run");
    let mut b = TimetableBuilder::new();
    for (id, x) in
        [("s1", 0.0), ("s2", 1_000.0), ("s3", 2_000.0), ("s3b", 2_000.0), ("s4", 3_000.0)]
    {
        b.add_stop(id, (Metres(x), Metres(0.0)));
    }
    let call =
        |stop: &str, s: u32| Call { stop: stop.into(), arrival: Second(s), departure: Second(s) };
    b.add_run("r0", vec![call("s1", 200), call("s2", 300), call("s3", 400)]);
    let other = b.build().expect("a timetable");
    let report = GtfsReport::default();
    let given = Given {
        source: Source::Tables,
        files: Files::default(),
        build_values: VALUES,
        stop_walk_max: 1_500.0,
        transit: Some(Transit { timetable: &other, report: &report, transfer_walk_max: 400.0 }),
        parameters: &p,
        network_repairs: net.repairs(),
        trips_given: u64::from(dem.trip_count()),
        demand_repairs: dem.repairs(),
    };
    let folder =
        std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("io-parquet-routes-refused");
    let _ = std::fs::remove_dir(&folder);
    let refused = write(&r, &net, &dem, &given, &folder);
    assert!(matches!(&refused, Err(WriteError::NotTheRunsJourney(t)) if t == "t"), "{refused:?}");
    assert!(!folder.exists(), "nothing is written");
}
