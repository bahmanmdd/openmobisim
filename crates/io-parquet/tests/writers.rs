//! Round-trip tests: write each artifact, read it back with the same
//! `parquet` reader `core-demand`'s own tests use, check what landed.

use std::fs::File;
use std::path::PathBuf;

use arrow_array::{Array, Float64Array, StringArray, UInt32Array, UInt64Array};
use openmobisim_core_demand::{ClassDefaults, Ownership, RawTrip, build_travellers};
use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::network::{LinkSpec, RoadNetwork, RoadNetworkBuilder};
use openmobisim_core_loading::LinkBinRecorder;
use openmobisim_core_sim::{EventRow, EventType, RunDescription, RunResult, TripCompletionStats};
use openmobisim_core_types::diagnostics::{
    Category, DiagKey, Diagnostics, ElementRef, Severity, codes,
};
use openmobisim_core_types::ids::{EntityId, LinkId, TripId};
use openmobisim_core_types::time::Second;
use openmobisim_core_types::units::Duration;
use openmobisim_io_parquet::manifest::Manifest;
use openmobisim_io_parquet::{
    write_diagnostics, write_events, write_kpis, write_link_bins, write_manifest,
};
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
        route_sets: None,
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
    let description = sample_description(None, None);
    let manifest = Manifest::for_run(&travellers, &result, Second(3600), 1, &description);

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
    // The run's identity (S168): the seed, the fingerprints, the settings.
    assert!(json.contains("\"master_seed\": 42"));
    assert!(json.contains("\"run_fingerprint\": \"00000000deadbeef\""));
    assert!(json.contains("\"network_fingerprint\": \"00000000000000ab\""));
    assert!(json.contains("\"flow_level\": 0"));
    assert!(json.contains("\"flow_step_seconds\": null"));
    assert!(json.contains("\"route_descriptor\": \"penalty(max_paths=5)\""));
    assert!(
        json.contains("\"link_bin_seconds\": null") && json.contains("\"link_bins_file\": null")
    );
    assert!(json.starts_with("{\n") && json.ends_with("\n}\n"));

    // With the loading model and per-link results on, those say so.
    let ltm = sample_description(Some(300.0), Some(600));
    let json = Manifest::for_run(&travellers, &result, Second(3600), 1, &ltm).to_json();
    assert!(json.contains("\"flow_step_seconds\": 300"), "{json}");
    assert!(json.contains("\"link_bin_seconds\": 600"));
    assert!(json.contains("\"link_bins_file\": \"link_bins.parquet\""));
}

fn sample_description(step: Option<f64>, bins: Option<u32>) -> RunDescription {
    RunDescription {
        fingerprint: 0xdead_beef,
        master_seed: 42,
        network_fingerprint: 0xab,
        flow_level: if step.is_some() { 4 } else { 0 },
        flow_step_seconds: step,
        route_method: "penalty".to_string(),
        route_descriptor: "penalty(max_paths=5)".to_string(),
        link_bin_seconds: bins,
    }
}

fn two_link_network() -> RoadNetwork {
    let mut b = RoadNetworkBuilder::new();
    b.add_node("a", LonLat::new(4.800, 45.700));
    b.add_node("b", LonLat::new(4.801, 45.700));
    b.add_link("way/9:fwd", "a", "b", LinkSpec::new(RoadClass::Primary));
    b.add_link("way/9:bwd", "b", "a", LinkSpec::new(RoadClass::Primary));
    b.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
        .expect("buildable")
}

#[test]
fn link_bins_round_trip_with_external_ids_and_the_run_identity() {
    let network = two_link_network();
    // Ids follow sorted external ids, so "way/9:bwd" is link 0 and "way/9:fwd" link 1.
    let bwd = network.link_external_ids().typed_id_of::<LinkId>("way/9:bwd").expect("known");
    let fwd = network.link_external_ids().typed_id_of::<LinkId>("way/9:fwd").expect("known");
    let mut recorder = LinkBinRecorder::new(2, 300, 3600.0);
    recorder.record(fwd, 10.0, 40.0, 1.0); // bin 0, 30 s
    recorder.record(fwd, 20.0, 80.0, 2.0); // bin 0, 60 s x 2 PCU
    recorder.record(bwd, 250.0, 310.0, 1.0); // bin 1
    recorder.record(fwd, 700.0, 720.0, 1.0); // bin 2
    let bins = recorder.finish();
    assert_eq!(bins.len(), 3, "rows: (0, fwd), (1, bwd), (2, fwd)");

    let path = temp_path("link_bins.parquet");
    write_link_bins(&path, "lb-run", &bins, &network, &sample_description(Some(300.0), Some(300)))
        .expect("write");
    let batch = read_first_batch(&path);
    assert_eq!(batch.num_rows(), 3);
    let names: Vec<_> = batch.schema().fields().iter().map(|f| f.name().clone()).collect();
    assert_eq!(
        names,
        ["run_id", "bin", "start_s", "link", "link_index", "crossings", "pcu", "pcu_seconds"]
    );
    let col = |i: usize| batch.column(i).clone();
    let u32s = |i: usize| -> Vec<u32> {
        col(i).as_any().downcast_ref::<UInt32Array>().expect("u32").values().to_vec()
    };
    let strings = |i: usize| -> Vec<String> {
        let a = col(i);
        let a = a.as_any().downcast_ref::<StringArray>().expect("string");
        (0..a.len()).map(|r| a.value(r).to_string()).collect()
    };
    let f64s = |i: usize| -> Vec<f64> {
        col(i).as_any().downcast_ref::<Float64Array>().expect("f64").values().to_vec()
    };
    assert_eq!(strings(0), ["lb-run"; 3]);
    assert_eq!(u32s(1), [0, 1, 2]);
    assert_eq!(u32s(2), [0, 300, 600], "start_s is bin x bin_seconds");
    assert_eq!(strings(3), ["way/9:fwd", "way/9:bwd", "way/9:fwd"], "the external ids");
    assert_eq!(u32s(4), [fwd.raw(), bwd.raw(), fwd.raw()], "and the internal ones");
    assert_eq!(u32s(5), [2, 1, 1]);
    assert_eq!(f64s(6), [3.0, 1.0, 1.0]);
    assert_eq!(f64s(7), [30.0 + 120.0, 60.0, 20.0]);

    // The file says what run and network it belongs to.
    let file = File::open(&path).expect("open");
    let reader = ParquetRecordBatchReaderBuilder::try_new(file).expect("valid parquet");
    let kv = reader.metadata().file_metadata().key_value_metadata().expect("metadata");
    let get = |key: &str| kv.iter().find(|e| e.key == key).and_then(|e| e.value.clone());
    assert_eq!(get("openmobisim.bin_seconds").as_deref(), Some("300"));
    assert_eq!(get("openmobisim.run_fingerprint").as_deref(), Some("00000000deadbeef"));
    assert_eq!(get("openmobisim.network_fingerprint").as_deref(), Some("00000000000000ab"));
    assert_eq!(get("openmobisim.master_seed").as_deref(), Some("42"));
}

#[test]
fn link_bins_writer_handles_a_run_with_no_traffic() {
    let network = two_link_network();
    let bins = LinkBinRecorder::new(2, 300, 3600.0).finish();
    let path = temp_path("link_bins_empty.parquet");
    write_link_bins(&path, "quiet", &bins, &network, &sample_description(None, Some(300)))
        .expect("write");
    assert_eq!(count_rows(&path), 0);
}
