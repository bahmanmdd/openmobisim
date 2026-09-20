//! Round-trip tests: write each artifact, read it back with the same
//! `parquet` reader `core-demand`'s own tests use, check what landed.

use std::fs::File;
use std::path::PathBuf;

use arrow_array::{Array, Float64Array, StringArray, UInt32Array, UInt64Array};
use openmobisim_core_demand::{ClassDefaults, Ownership, RawTrip, build_travellers};
use openmobisim_core_sim::{EventRow, EventType, RunResult, TripCompletionStats};
use openmobisim_core_types::diagnostics::{
    Category, DiagKey, Diagnostics, ElementRef, Severity, codes,
};
use openmobisim_core_types::ids::{EntityId, TripId};
use openmobisim_core_types::time::Second;
use openmobisim_core_types::units::Duration;
use openmobisim_io_parquet::manifest::Manifest;
use openmobisim_io_parquet::{write_diagnostics, write_events, write_kpis, write_manifest};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

fn temp_path(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("openmobisim-io-parquet-test");
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir.join(name)
}

fn read_first_batch(path: &std::path::Path) -> arrow_array::RecordBatch {
    let file = File::open(path).expect("open written file");
    let mut reader = ParquetRecordBatchReaderBuilder::try_new(file)
        .expect("valid parquet")
        .build()
        .expect("reader");
    reader.next().expect("at least one batch").expect("valid batch")
}

/// Total rows across every batch — unlike [`read_first_batch`], this
/// tolerates a file with no row groups at all, which is what an empty
/// `RecordBatch` writes as.
fn count_rows(path: &std::path::Path) -> usize {
    let file = File::open(path).expect("open written file");
    let reader = ParquetRecordBatchReaderBuilder::try_new(file)
        .expect("valid parquet")
        .build()
        .expect("reader");
    reader.map(|batch| batch.expect("valid batch").num_rows()).sum()
}

fn sample_result() -> RunResult {
    RunResult {
        total_travel_time: Duration(725.0),
        completion: TripCompletionStats {
            total_trips: 5,
            no_vehicle_available: 1,
            no_feasible_path: 0,
            completed: 3,
            truncated: 1,
        },
        events: vec![
            EventRow::trip(Second(100), EventType::TripCompleted, TripId::new(0)),
            EventRow::trip(Second(150), EventType::TripCompleted, TripId::new(1)),
            EventRow::trip(Second(200), EventType::TripTruncated, TripId::new(2)),
            EventRow::trip(Second(0), EventType::NoVehicleAvailable, TripId::new(3)),
        ],
        link_bins: None,
    }
}

#[test]
fn kpis_round_trip_every_metric() {
    let path = temp_path("kpis.parquet");
    let result = sample_result();
    write_kpis(&path, "run-1", "design-a", 0, 0, &result).expect("write");

    let batch = read_first_batch(&path);
    assert_eq!(batch.num_rows(), 6);

    let run_id = batch.column(0).as_any().downcast_ref::<StringArray>().unwrap();
    for i in 0..batch.num_rows() {
        assert_eq!(run_id.value(i), "run-1");
    }

    let metric = batch.column(4).as_any().downcast_ref::<StringArray>().unwrap();
    let value = batch.column(5).as_any().downcast_ref::<Float64Array>().unwrap();
    let row_of = |name: &str| (0..batch.num_rows()).find(|&i| metric.value(i) == name).unwrap();

    #[allow(clippy::float_cmp, reason = "values are exact, written by this same test")]
    {
        assert_eq!(value.value(row_of("total_travel_time_s")), 725.0);
        assert_eq!(value.value(row_of("completed_trips")), 3.0);
        assert_eq!(value.value(row_of("truncated_trips")), 1.0);
        assert_eq!(value.value(row_of("no_vehicle_available_trips")), 1.0);
        assert_eq!(value.value(row_of("no_feasible_path_trips")), 0.0);
        assert_eq!(value.value(row_of("completion_rate")), 3.0 / 4.0);
    }
}

#[test]
fn diagnostics_round_trip_with_and_without_an_element() {
    let path = temp_path("diagnostics.parquet");
    let mut diagnostics = Diagnostics::new();
    diagnostics.record(DiagKey::new(
        Category::Modelling,
        codes::NO_FEASIBLE_PATH,
        Severity::Warning,
        ElementRef::of(TripId::new(7)),
    ));
    diagnostics.record_run_level(Category::DataQuality, codes::TRIP_TRUNCATED, Severity::Info);

    write_diagnostics(&path, "run-1", &diagnostics).expect("write");

    let batch = read_first_batch(&path);
    assert_eq!(batch.num_rows(), 2);

    let code = batch.column(2).as_any().downcast_ref::<StringArray>().unwrap();
    let element_kind = batch.column(4).as_any().downcast_ref::<StringArray>().unwrap();
    let element_id = batch.column(5).as_any().downcast_ref::<UInt32Array>().unwrap();
    let count = batch.column(6).as_any().downcast_ref::<UInt64Array>().unwrap();

    let with_element =
        (0..batch.num_rows()).find(|&i| code.value(i) == "no_feasible_path").unwrap();
    assert_eq!(element_kind.value(with_element), "trip");
    assert_eq!(element_id.value(with_element), 7);
    assert_eq!(count.value(with_element), 1);

    let run_level = (0..batch.num_rows()).find(|&i| code.value(i) == "trip_truncated").unwrap();
    assert!(element_kind.is_null(run_level));
    assert!(element_id.is_null(run_level));
}

#[test]
fn events_sampling_keeps_everything_at_rate_one_and_fewer_below_it() {
    let events: Vec<EventRow> = (0..1000)
        .map(|i| EventRow::trip(Second(i), EventType::TripCompleted, TripId::new(i)))
        .collect();

    let full_path = temp_path("events_full.parquet");
    write_events(&full_path, "run-1", &events, 1.0).expect("write");
    assert_eq!(read_first_batch(&full_path).num_rows(), 1000);

    let sampled_path = temp_path("events_sampled.parquet");
    write_events(&sampled_path, "run-1", &events, 0.01).expect("write");
    let sampled_rows = read_first_batch(&sampled_path).num_rows();
    assert!(
        sampled_rows > 0 && sampled_rows < 1000,
        "1% sampling of 1000 rows must land strictly between 0 and 1000, got {sampled_rows}"
    );
    assert_eq!(sampled_rows, 10, "every 100th row, deterministically");
}

#[test]
fn events_writer_handles_an_empty_run() {
    let path = temp_path("events_empty.parquet");
    write_events(&path, "run-1", &[], openmobisim_io_parquet::events::DEFAULT_SAMPLE_RATE)
        .expect("write");
    assert_eq!(count_rows(&path), 0);
}

#[test]
fn manifest_reports_the_fields_phase_1_actually_has() {
    let raw_trips = vec![
        RawTrip {
            traveller_id: "alice".to_string(),
            trip_seq: 0,
            origin: openmobisim_core_graph::geometry::LonLat::new(4.80, 45.70),
            destination: openmobisim_core_graph::geometry::LonLat::new(4.81, 45.70),
            departure_time: Second(0),
            user_class: "commuter".to_string(),
            weight: None,
        },
        RawTrip {
            traveller_id: "bob".to_string(),
            trip_seq: 0,
            origin: openmobisim_core_graph::geometry::LonLat::new(4.80, 45.70),
            destination: openmobisim_core_graph::geometry::LonLat::new(4.81, 45.70),
            departure_time: Second(0),
            user_class: "pedestrian".to_string(), // no declared default
            weight: None,
        },
    ];
    let class_defaults =
        ClassDefaults::new().with_default("commuter", Ownership { car: true, ..Ownership::NONE });
    let mut build_diagnostics = Diagnostics::new();
    let (travellers, _trips) =
        build_travellers(raw_trips, Vec::new(), &class_defaults, 1, &mut build_diagnostics)
            .expect("buildable");

    let result = sample_result();
    let manifest = Manifest::for_run(&travellers, &result, Second(3600), 1);

    assert_eq!(manifest.simulated_travellers, 2);
    assert_eq!(manifest.car_owning_travellers, 1, "only alice owns a car");
    assert_eq!(manifest.window_seconds, 3600);
    assert_eq!(manifest.default_weight, 1);
    assert_eq!(manifest.kpi_weighting, "traveller_weight_scaled");
    assert_eq!(manifest.code_version, openmobisim_core_types::CODE_VERSION);
    assert_eq!(manifest.defaults_version, openmobisim_core_graph::DEFAULTS_VERSION);

    let path = temp_path("manifest.json");
    write_manifest(&path, &manifest).expect("write");
    let json = std::fs::read_to_string(&path).expect("read back");
    assert!(json.contains("\"simulated_travellers\": 2"));
    assert!(json.contains("\"car_owning_travellers\": 1"));
    assert!(json.contains("\"kpi_weighting\": \"traveller_weight_scaled\""));
}
