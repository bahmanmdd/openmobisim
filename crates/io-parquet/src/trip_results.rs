//! `trip_results.parquet` — every trip of the run and what became of it.
//!
//! One row per trip, in trip order (by traveller, then by the order of their day):
//! `run_id, trip, traveller_id, trip_seq, user_class, weight, origin_lon, origin_lat,
//! destination_lon, destination_lat, departure_s, mode, mode_choice, outcome, arrival_s,
//! travel_time_s, link_at_end, vehicle_s, access_egress_s`. The same table
//! `Run.trip_results()` returns in Python.
//!
//! - `trip` is the trip's index in the run (as in `Run.route_changes()`), `trip_seq` its place
//!   in its traveller's day from 0, `weight` the people its traveller stands for.
//! - `mode` is the mode taken (with mode choice, the mode chosen); **null** for a trip that
//!   chose and had none to choose. `outcome` is `completed`, `truncated` (still travelling when
//!   the window ended), `no_vehicle_available`, `no_feasible_path`, `mode_not_available` or
//!   `no_alternative` (the trips with a null `mode`).
//! - Times are seconds after midnight; `arrival_s` and `travel_time_s` are **null** unless the
//!   trip completed.
//! - `link_at_end` is, for a trip still travelling when the window ended whose car was on the
//!   roads, the road link it was on or waited at its origin to enter, as an index into the
//!   network's links (as `link_bins.parquet`'s `link`); **null** otherwise.
//! - `travel_time_s` is door to door in a run that compares modes or has a timetable: the
//!   fixed times of the trip's own vehicle, its walks, waits and rides included; the drive alone
//!   in a run of stated car trips. `vehicle_s` is the completed trip's seconds in a vehicle of
//!   its own (the car's drive, the bike's ride; 0 without one) and `access_egress_s` its fixed
//!   seconds around it (reaching it, leaving it, its parking's time; 0 without one); both
//!   **null** unless the trip completed.
//! - The outcomes are those of the last loading. The key-value metadata carries
//!   `openmobisim.run_fingerprint` and `openmobisim.master_seed`.
//!
//! **Cost.** About 100 bytes per trip in memory while writing; Snappy-compressed, the ids
//! dictionary-encoded.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use arrow_array::{ArrayRef, BooleanArray, Float64Array, RecordBatch, StringArray, UInt32Array};
use arrow_schema::{DataType, Field, Schema};
use openmobisim_core_sim::{RunDescription, TripTable};
use parquet::basic::Compression;

use crate::WriteError;
use crate::io::write_single_batch_with;

/// Write `trip_results.parquet` for one run.
///
/// # Errors
///
/// [`WriteError`] if the file cannot be created or the Parquet writer rejects
/// the data.
///
/// # Panics
///
/// Panics if the run has more trips than `u32` counts, which a run cannot have.
pub fn write_trip_results(
    path: impl AsRef<Path>,
    run_id: &str,
    table: &TripTable,
    description: &RunDescription,
) -> Result<(), WriteError> {
    let n = table.len();
    let index = u32::try_from(n).expect("the trips of a run fit u32");
    let floats = |v: Vec<f64>| -> ArrayRef { Arc::new(Float64Array::from(v)) };
    let columns: Vec<ArrayRef> = vec![
        Arc::new(StringArray::from(vec![run_id; n])),
        Arc::new(UInt32Array::from((0..index).collect::<Vec<_>>())),
        Arc::new(StringArray::from(table.traveller_id.clone())),
        Arc::new(UInt32Array::from(table.trip_seq.clone())),
        Arc::new(StringArray::from(table.user_class.clone())),
        Arc::new(UInt32Array::from(table.weight.clone())),
        floats(table.origin.iter().map(|p| p.lon).collect()),
        floats(table.origin.iter().map(|p| p.lat).collect()),
        floats(table.destination.iter().map(|p| p.lon).collect()),
        floats(table.destination.iter().map(|p| p.lat).collect()),
        Arc::new(UInt32Array::from(table.departure_s.clone())),
        Arc::new(StringArray::from(
            table.mode.iter().map(|m| m.map(|m| m.as_str())).collect::<Vec<_>>(),
        )),
        Arc::new(BooleanArray::from(table.mode_choice.clone())),
        Arc::new(StringArray::from(table.outcome.iter().map(|o| o.as_str()).collect::<Vec<_>>())),
        Arc::new(UInt32Array::from(table.arrival_s.clone())),
        Arc::new(UInt32Array::from(table.travel_time_s())),
        Arc::new(UInt32Array::from(table.link_at_end.clone())),
        Arc::new(UInt32Array::from(table.vehicle_s.clone())),
        Arc::new(UInt32Array::from(table.access_egress_s.clone())),
    ];
    let metadata = vec![
        ("openmobisim.run_fingerprint".to_string(), description.fingerprint_hex()),
        ("openmobisim.master_seed".to_string(), description.master_seed.to_string()),
    ];
    let field = |name: &str, t: DataType, nullable: bool| Field::new(name, t, nullable);
    let schema = Arc::new(
        Schema::new(vec![
            field("run_id", DataType::Utf8, false),
            field("trip", DataType::UInt32, false),
            field("traveller_id", DataType::Utf8, false),
            field("trip_seq", DataType::UInt32, false),
            field("user_class", DataType::Utf8, false),
            field("weight", DataType::UInt32, false),
            field("origin_lon", DataType::Float64, false),
            field("origin_lat", DataType::Float64, false),
            field("destination_lon", DataType::Float64, false),
            field("destination_lat", DataType::Float64, false),
            field("departure_s", DataType::UInt32, false),
            field("mode", DataType::Utf8, true),
            field("mode_choice", DataType::Boolean, false),
            field("outcome", DataType::Utf8, false),
            field("arrival_s", DataType::UInt32, true),
            field("travel_time_s", DataType::UInt32, true),
            field("link_at_end", DataType::UInt32, true),
            field("vehicle_s", DataType::UInt32, true),
            field("access_egress_s", DataType::UInt32, true),
        ])
        .with_metadata(metadata.iter().cloned().collect::<HashMap<_, _>>()),
    );
    let batch = RecordBatch::try_new(schema.clone(), columns)?;
    write_single_batch_with(path.as_ref(), schema, &batch, &metadata, Compression::SNAPPY)
}
