//! `core-demand`: the `trips.parquet`/`persons.parquet` readers, travellers,
//! and vehicle-location state (Phase 1 step 4).
//!
//! Fixtures are written to real Parquet files with the `arrow`/`parquet`
//! writer API rather than checked into the repository — the same reasoning
//! `io-osm` gives for building OSM fixtures in memory (`source.rs`): a test
//! can then state the exact row it is about, instead of shipping an opaque
//! binary and hoping it still contains what the test name claims.

use std::fs::File;
use std::sync::Arc;

use arrow_array::{ArrayRef, RecordBatch, StringArray, UInt32Array};
use arrow_schema::{DataType, Field, Schema};
use openmobisim_core_demand::travellers::codes;
use openmobisim_core_demand::{
    ClassDefaults, Ownership, RawPerson, VehicleKind, VehicleLocations, build_travellers,
    read_persons_parquet, read_trips_parquet,
};
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::{EntityId, TravellerId};
use openmobisim_core_types::time::Second;
use parquet::arrow::ArrowWriter;

/// One row of a `trips.parquet` fixture.
struct TripRow {
    traveller_id: &'static str,
    trip_seq: u32,
    origin: LonLat,
    destination: LonLat,
    departure_time_s: u32,
    user_class: &'static str,
    weight: Option<u32>,
}

fn trip(
    traveller_id: &'static str,
    trip_seq: u32,
    origin: (f64, f64),
    destination: (f64, f64),
) -> TripRow {
    TripRow {
        traveller_id,
        trip_seq,
        origin: LonLat::new(origin.0, origin.1),
        destination: LonLat::new(destination.0, destination.1),
        departure_time_s: trip_seq * 3600,
        user_class: "commuter",
        weight: None,
    }
}

fn write_trips(path: &std::path::Path, rows: &[TripRow]) {
    let traveller_id: ArrayRef =
        Arc::new(StringArray::from(rows.iter().map(|r| r.traveller_id).collect::<Vec<_>>()));
    let trip_seq: ArrayRef =
        Arc::new(UInt32Array::from(rows.iter().map(|r| r.trip_seq).collect::<Vec<_>>()));
    let origin_lon: ArrayRef = Arc::new(arrow_array::Float64Array::from(
        rows.iter().map(|r| r.origin.lon).collect::<Vec<_>>(),
    ));
    let origin_lat: ArrayRef = Arc::new(arrow_array::Float64Array::from(
        rows.iter().map(|r| r.origin.lat).collect::<Vec<_>>(),
    ));
    let destination_lon: ArrayRef = Arc::new(arrow_array::Float64Array::from(
        rows.iter().map(|r| r.destination.lon).collect::<Vec<_>>(),
    ));
    let destination_lat: ArrayRef = Arc::new(arrow_array::Float64Array::from(
        rows.iter().map(|r| r.destination.lat).collect::<Vec<_>>(),
    ));
    let departure_time_s: ArrayRef =
        Arc::new(UInt32Array::from(rows.iter().map(|r| r.departure_time_s).collect::<Vec<_>>()));
    let user_class: ArrayRef =
        Arc::new(StringArray::from(rows.iter().map(|r| r.user_class).collect::<Vec<_>>()));
    let weight: ArrayRef =
        Arc::new(UInt32Array::from(rows.iter().map(|r| r.weight).collect::<Vec<_>>()));

    let schema = Arc::new(Schema::new(vec![
        Field::new("traveller_id", DataType::Utf8, false),
        Field::new("trip_seq", DataType::UInt32, false),
        Field::new("origin_lon", DataType::Float64, false),
        Field::new("origin_lat", DataType::Float64, false),
        Field::new("destination_lon", DataType::Float64, false),
        Field::new("destination_lat", DataType::Float64, false),
        Field::new("departure_time_s", DataType::UInt32, false),
        Field::new("user_class", DataType::Utf8, false),
        Field::new("weight", DataType::UInt32, true),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            traveller_id,
            trip_seq,
            origin_lon,
            origin_lat,
            destination_lon,
            destination_lat,
            departure_time_s,
            user_class,
            weight,
        ],
    )
    .expect("valid batch");

    let file = File::create(path).expect("create fixture");
    let mut writer = ArrowWriter::try_new(file, schema, None).expect("writer");
    writer.write(&batch).expect("write batch");
    writer.close().expect("close writer");
}

fn write_persons(path: &std::path::Path, rows: &[(&str, Option<bool>, Option<bool>)]) {
    let traveller_id: ArrayRef =
        Arc::new(StringArray::from(rows.iter().map(|r| r.0).collect::<Vec<_>>()));
    let owns_car: ArrayRef =
        Arc::new(arrow_array::BooleanArray::from(rows.iter().map(|r| r.1).collect::<Vec<_>>()));
    let owns_bike: ArrayRef =
        Arc::new(arrow_array::BooleanArray::from(rows.iter().map(|r| r.2).collect::<Vec<_>>()));

    let schema = Arc::new(Schema::new(vec![
        Field::new("traveller_id", DataType::Utf8, false),
        Field::new("owns_car", DataType::Boolean, true),
        Field::new("owns_bike", DataType::Boolean, true),
    ]));
    let batch =
        RecordBatch::try_new(schema.clone(), vec![traveller_id, owns_car, owns_bike]).unwrap();

    let file = File::create(path).expect("create fixture");
    let mut writer = ArrowWriter::try_new(file, schema, None).expect("writer");
    writer.write(&batch).expect("write batch");
    writer.close().expect("close writer");
}

fn temp_path(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("openmobisim-core-demand-test");
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir.join(name)
}

#[test]
fn reads_a_simple_two_trip_traveller() {
    let path = temp_path("simple.parquet");
    write_trips(
        &path,
        &[
            trip("alice", 0, (4.80, 45.70), (4.81, 45.70)),
            trip("alice", 1, (4.81, 45.70), (4.80, 45.70)),
        ],
    );

    let raw = read_trips_parquet(&path).expect("readable");
    assert_eq!(raw.len(), 2);

    let mut diagnostics = Diagnostics::new();
    let (travellers, trips) =
        build_travellers(raw, Vec::new(), &ClassDefaults::new(), 1, &mut diagnostics)
            .expect("buildable");

    assert_eq!(travellers.len(), 1);
    assert_eq!(trips.len(), 2);
    let alice = TravellerId::new(0);
    assert_eq!(travellers.weight(alice), 1, "no weight column: falls back to default_weight");
    assert!(diagnostics.is_empty(), "clean data produces no diagnostics: {:?}", diagnostics.rows());
}

#[test]
fn the_trip_chain_wins_over_a_disagreeing_stated_origin() {
    // S128: trip 2's stated origin (4.90, 45.80) disagrees with trip 1's
    // destination (4.81, 45.70). The chain must win.
    let path = temp_path("chain.parquet");
    write_trips(
        &path,
        &[
            trip("bob", 0, (4.80, 45.70), (4.81, 45.70)),
            trip("bob", 1, (4.90, 45.80), (4.80, 45.70)),
        ],
    );
    let raw = read_trips_parquet(&path).expect("readable");

    let mut diagnostics = Diagnostics::new();
    let (travellers, trips) =
        build_travellers(raw, Vec::new(), &ClassDefaults::new(), 1, &mut diagnostics)
            .expect("buildable");

    let bob = TravellerId::new(0);
    let second_trip = travellers.trips_of(bob).nth(1).expect("two trips");
    assert_eq!(
        trips.origin(second_trip),
        LonLat::new(4.81, 45.70),
        "the second trip must start where the first ended, not where the file said"
    );
    assert_eq!(diagnostics.count_of(codes::TRIP_CHAIN_ORIGIN_OVERRIDDEN), 1);
}

#[test]
fn ownership_is_the_class_default_unless_a_person_overrides_it() {
    let path = temp_path("ownership.parquet");
    write_trips(
        &path,
        &[
            trip("carol", 0, (4.80, 45.70), (4.81, 45.70)),
            trip("dana", 0, (4.80, 45.70), (4.81, 45.70)),
        ],
    );
    let raw_trips = read_trips_parquet(&path).expect("readable");

    let persons_path = temp_path("ownership_persons.parquet");
    write_persons(&persons_path, &[("dana", None, Some(true))]);
    let raw_persons = read_persons_parquet(&persons_path).expect("readable");

    let class_defaults =
        ClassDefaults::new().with_default("commuter", Ownership { car: true, ..Ownership::NONE });

    let mut diagnostics = Diagnostics::new();
    let (travellers, _trips) =
        build_travellers(raw_trips, raw_persons, &class_defaults, 1, &mut diagnostics)
            .expect("buildable");

    // Sorted external ids: "carol" < "dana".
    let carol = TravellerId::new(0);
    let dana = TravellerId::new(1);
    assert_eq!(
        travellers.ownership(carol),
        Ownership { car: true, bike: false, transit_pass: false },
        "no persons.parquet row: takes the class default exactly"
    );
    assert_eq!(
        travellers.ownership(dana),
        Ownership { car: true, bike: true, transit_pass: false },
        "owns_bike is overridden; owns_car (not stated) keeps the class default"
    );
    assert_eq!(diagnostics.count_of(codes::OWNERSHIP_OVERRIDDEN), 1, "only dana was overridden");
}

#[test]
fn a_persons_row_for_an_unknown_traveller_is_recorded_and_ignored() {
    let path = temp_path("unknown_person.parquet");
    write_trips(&path, &[trip("erin", 0, (4.80, 45.70), (4.81, 45.70))]);
    let raw_trips = read_trips_parquet(&path).expect("readable");

    let raw_persons =
        vec![RawPerson { traveller_id: "nobody".to_string(), ..RawPerson::default() }];

    let mut diagnostics = Diagnostics::new();
    let (travellers, _) =
        build_travellers(raw_trips, raw_persons, &ClassDefaults::new(), 1, &mut diagnostics)
            .expect("buildable");

    assert_eq!(travellers.len(), 1);
    assert_eq!(diagnostics.count_of(codes::PERSON_UNKNOWN_TRAVELLER), 1);
}

#[test]
fn duplicate_trip_seq_keeps_the_first_and_is_diagnosed() {
    let path = temp_path("duplicate_seq.parquet");
    write_trips(
        &path,
        &[
            trip("frank", 0, (4.80, 45.70), (4.81, 45.70)),
            // A second row also claiming trip_seq = 0.
            trip("frank", 0, (4.99, 45.99), (4.98, 45.98)),
        ],
    );
    let raw = read_trips_parquet(&path).expect("readable");

    let mut diagnostics = Diagnostics::new();
    let (travellers, trips) =
        build_travellers(raw, Vec::new(), &ClassDefaults::new(), 1, &mut diagnostics)
            .expect("buildable");

    let frank = TravellerId::new(0);
    assert_eq!(travellers.trips_of(frank).count(), 1, "the duplicate row was dropped");
    let kept = travellers.first_trip(frank);
    assert_eq!(trips.origin(kept), LonLat::new(4.80, 45.70), "the first row in file order wins");
    assert_eq!(diagnostics.count_of(codes::DUPLICATE_TRIP_SEQ), 1);
}

#[test]
fn an_empty_trips_file_is_an_error_not_a_silent_empty_population() {
    let mut diagnostics = Diagnostics::new();
    let result =
        build_travellers(Vec::new(), Vec::new(), &ClassDefaults::new(), 1, &mut diagnostics);
    assert!(result.is_err());
}

#[test]
fn vehicles_start_at_the_first_trip_s_origin() {
    // S129: a traveller's owned vehicles start where their day starts, and
    // that is S46's "base" for the overnight rule.
    let path = temp_path("vehicles.parquet");
    write_trips(
        &path,
        &[
            trip("gia", 0, (4.70, 45.60), (4.80, 45.70)),
            trip("gia", 1, (4.80, 45.70), (4.70, 45.60)),
        ],
    );
    let raw = read_trips_parquet(&path).expect("readable");

    let class_defaults =
        ClassDefaults::new().with_default("commuter", Ownership { car: true, ..Ownership::NONE });
    let mut diagnostics = Diagnostics::new();
    let (travellers, trips) =
        build_travellers(raw, Vec::new(), &class_defaults, 1, &mut diagnostics).expect("buildable");

    let locations = VehicleLocations::at_first_trip_origin(&travellers, &trips);
    let gia = TravellerId::new(0);
    let base = LonLat::new(4.70, 45.60);

    assert_eq!(locations.location(&travellers, gia, VehicleKind::Car), Some(base));
    assert_eq!(
        locations.location(&travellers, gia, VehicleKind::Bike),
        None,
        "gia does not own a bike"
    );
    assert!(locations.is_at_origin(&travellers, gia, VehicleKind::Car, base));
    assert!(!locations.is_at_origin(&travellers, gia, VehicleKind::Car, LonLat::new(4.80, 45.70)));

    let second_trip = travellers.trips_of(gia).nth(1).expect("two trips");
    assert_eq!(
        trips.origin(second_trip),
        LonLat::new(4.80, 45.70),
        "sanity: the traveller's second trip starts at the first trip's destination"
    );
}

#[test]
fn weight_falls_back_and_inconsistency_is_diagnosed() {
    let path = temp_path("weight.parquet");
    let mut rows = vec![
        trip("hank", 0, (4.80, 45.70), (4.81, 45.70)),
        trip("hank", 1, (4.81, 45.70), (4.80, 45.70)),
    ];
    rows[0].weight = Some(3);
    rows[1].weight = Some(5); // disagrees with the first row
    write_trips(&path, &rows);
    let raw = read_trips_parquet(&path).expect("readable");

    let mut diagnostics = Diagnostics::new();
    let (travellers, _) =
        build_travellers(raw, Vec::new(), &ClassDefaults::new(), 1, &mut diagnostics)
            .expect("buildable");

    let hank = TravellerId::new(0);
    assert_eq!(travellers.weight(hank), 3, "the first stated value wins");
    assert_eq!(diagnostics.count_of(codes::INCONSISTENT_TRAVELLER_WEIGHT), 1);
}

#[test]
fn departure_time_round_trips() {
    let path = temp_path("departure.parquet");
    write_trips(&path, &[trip("ivy", 0, (4.80, 45.70), (4.81, 45.70))]);
    let raw = read_trips_parquet(&path).expect("readable");

    let mut diagnostics = Diagnostics::new();
    let (travellers, trips) =
        build_travellers(raw, Vec::new(), &ClassDefaults::new(), 1, &mut diagnostics)
            .expect("buildable");
    let ivy = TravellerId::new(0);
    assert_eq!(trips.departure(travellers.first_trip(ivy)), Second(0));
}
