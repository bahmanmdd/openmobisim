//! `transit_calls.parquet` — every call of the run's timetable, with its times
//! in the run and its passengers (S199; the schema ruled by the user, S200).
//!
//! One row per call — a run at a stop — in run order, then stop order:
//! `run_id, run, line, kind, stop_id, sequence, scheduled_arrival_s,
//! scheduled_departure_s, arrival_s, departure_s, boardings, alightings,
//! on_board`. The same table `Run.transit_calls()` returns in Python.
//!
//! - `run` is the GTFS `trip_id`, `line` the route's short name, `kind` the
//!   service (`bus`, `tram`, `metro`, `rail`, `ferry`, `other`), `sequence` the
//!   call's place in its run from 0.
//! - Times are seconds after the service day's midnight (the run clock's zero).
//!   `arrival_s` and `departure_s` are the times in the run: a bus that rode the
//!   roads keeps the loading's, every other run the schedule's; **null** where a
//!   bus had not reached the call by the end of the window.
//! - `boardings`, `alightings` and `on_board` (as the vehicle leaves) are weighted
//!   by traveller weight.
//! - The key-value metadata carries `openmobisim.run_fingerprint`,
//!   `openmobisim.master_seed` and `openmobisim.service_date`.
//!
//! **Cost.** About 60 bytes per call in memory while writing; Amsterdam's 112 000
//! calls are about 2 MB on disk (Snappy; the id columns dictionary-encoded).

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use arrow_array::{ArrayRef, Float64Array, RecordBatch, StringArray, UInt32Array};
use arrow_schema::{DataType, Field, Schema};
use openmobisim_core_sim::{RunDescription, TransitResult};
use openmobisim_core_transit::{Timetable, UNKNOWN_TIME};
use openmobisim_core_types::ids::{EntityId, TransitRunId};
use parquet::basic::Compression;

use crate::WriteError;
use crate::io::write_single_batch_with;

/// Write `transit_calls.parquet` for one run.
///
/// # Errors
///
/// [`WriteError`] if the file cannot be created or the Parquet writer rejects
/// the data.
///
/// # Panics
///
/// Panics if `result` was not made on `timetable` (its arrays have another length).
pub fn write_transit_calls(
    path: impl AsRef<Path>,
    run_id: &str,
    timetable: &Timetable,
    result: &TransitResult,
    description: &RunDescription,
) -> Result<(), WriteError> {
    let n = timetable.call_count();
    assert_eq!(result.boardings.len(), n, "the result is of this timetable");
    let mut run = Vec::with_capacity(n);
    let mut line = Vec::with_capacity(n);
    let mut kind = Vec::with_capacity(n);
    let mut stop = Vec::with_capacity(n);
    let mut sequence = Vec::with_capacity(n);
    for r in 0..timetable.run_count() {
        let id = TransitRunId::new(r);
        let route = timetable.run_route(id);
        for (k, c) in timetable.run_calls(id).enumerate() {
            run.push(timetable.run_ids().external(r));
            line.push(timetable.route_short_name(route));
            kind.push(timetable.route_kind(route).as_str());
            stop.push(timetable.stop_ids().external(timetable.call_stop(c).raw()));
            sequence.push(u32::try_from(k).expect("calls of a run fit u32"));
        }
    }
    let times = |v: &[u32]| -> ArrayRef {
        Arc::new(UInt32Array::from(
            v.iter().map(|&t| (t != UNKNOWN_TIME).then_some(t)).collect::<Vec<_>>(),
        ))
    };
    let scheduled = timetable.scheduled();
    let columns: Vec<ArrayRef> = vec![
        Arc::new(StringArray::from(vec![run_id; n])),
        Arc::new(StringArray::from(run)),
        Arc::new(StringArray::from(line)),
        Arc::new(StringArray::from(kind)),
        Arc::new(StringArray::from(stop)),
        Arc::new(UInt32Array::from(sequence)),
        times(&scheduled.arrival),
        times(&scheduled.departure),
        times(&result.times.arrival),
        times(&result.times.departure),
        Arc::new(Float64Array::from(result.boardings.clone())),
        Arc::new(Float64Array::from(result.alightings.clone())),
        Arc::new(Float64Array::from(result.on_board(timetable))),
    ];
    let metadata = vec![
        ("openmobisim.run_fingerprint".to_string(), description.fingerprint_hex()),
        ("openmobisim.master_seed".to_string(), description.master_seed.to_string()),
        ("openmobisim.service_date".to_string(), timetable.date().to_string()),
    ];
    let field = |name: &str, t: DataType, nullable: bool| Field::new(name, t, nullable);
    let schema = Arc::new(
        Schema::new(vec![
            field("run_id", DataType::Utf8, false),
            field("run", DataType::Utf8, false),
            field("line", DataType::Utf8, false),
            field("kind", DataType::Utf8, false),
            field("stop_id", DataType::Utf8, false),
            field("sequence", DataType::UInt32, false),
            field("scheduled_arrival_s", DataType::UInt32, false),
            field("scheduled_departure_s", DataType::UInt32, false),
            field("arrival_s", DataType::UInt32, true),
            field("departure_s", DataType::UInt32, true),
            field("boardings", DataType::Float64, false),
            field("alightings", DataType::Float64, false),
            field("on_board", DataType::Float64, false),
        ])
        .with_metadata(metadata.iter().cloned().collect::<HashMap<_, _>>()),
    );
    let batch = RecordBatch::try_new(schema.clone(), columns)?;
    write_single_batch_with(path.as_ref(), schema, &batch, &metadata, Compression::SNAPPY)
}
