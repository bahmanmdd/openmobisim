//! `legs.parquet` of a frozen day — a run's service legs as its last loading played them.
//!
//! One row per leg, in the order the run played them (by `trip`):
//! `trip, traveller_id, trip_seq, service, weight, request_s, origin_node, destination_node,
//! dock_from, dock_to, ride_s, egress_s, expected_dock_s`.
//!
//! - `trip` is the trip's index in the run (as in `trip_results.parquet`), the key the fleet
//!   engine orders legs by at one second; `traveller_id` and `trip_seq` name it as the run's
//!   other tables do.
//! - `service` is the service's name; `weight` the people the leg stands for.
//! - `request_s` is the second it asks for a bike at its pickup dock (its departure plus the walk
//!   there), in seconds from midnight.
//! - `origin_node` and `destination_node` are the trip's ends as nodes of the network's walk
//!   layer (internal indices, meaningful only with the network the day was frozen on).
//! - `dock_from` and `dock_to` are the docks the trip chose (dock ids); `ride_s` the planned
//!   ride between them, the service's `ride_factor` included; `egress_s` the walk from the return
//!   dock to the destination; `expected_dock_s` the extra seconds at the docks its choice
//!   expected.
//!
//! Every number is written and read back bit for bit (Parquet's 64-bit floats), so a replay of
//! the file plays the legs the run played. **Cost:** about 100 bytes per leg in memory while
//! writing or reading.

use std::fs::File;
use std::path::Path;
use std::sync::Arc;

use arrow_array::{Array, ArrayRef, Float64Array, RecordBatch, StringArray, UInt32Array};
use arrow_schema::{DataType, Field, Schema};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::basic::Compression;

use crate::WriteError;
use crate::io::write_single_batch_with;

/// One leg of a frozen day (see the [module docs](self)).
#[derive(Clone, Debug, PartialEq)]
pub struct FrozenLegRow {
    /// The trip's index in the run.
    pub trip: u32,
    /// Its traveller's id.
    pub traveller_id: String,
    /// Its place in its traveller's day, from 0.
    pub trip_seq: u32,
    /// The service's name.
    pub service: String,
    /// The people it stands for.
    pub weight: u32,
    /// The second it asks for a bike, from midnight.
    pub request_s: f64,
    /// Its origin, a walk-layer node index.
    pub origin_node: u32,
    /// Its destination, a walk-layer node index.
    pub destination_node: u32,
    /// The pickup dock it chose (dock id).
    pub dock_from: String,
    /// The return dock it chose (dock id).
    pub dock_to: String,
    /// The planned ride between the two, in seconds.
    pub ride_s: f64,
    /// The walk from the return dock to the destination, in seconds.
    pub egress_s: f64,
    /// The extra seconds at the docks its choice expected.
    pub expected_dock_s: f64,
}

fn schema() -> Arc<Schema> {
    let u = |n: &str| Field::new(n, DataType::UInt32, false);
    let f = |n: &str| Field::new(n, DataType::Float64, false);
    let s = |n: &str| Field::new(n, DataType::Utf8, false);
    Arc::new(Schema::new(vec![
        u("trip"),
        s("traveller_id"),
        u("trip_seq"),
        s("service"),
        u("weight"),
        f("request_s"),
        u("origin_node"),
        u("destination_node"),
        s("dock_from"),
        s("dock_to"),
        f("ride_s"),
        f("egress_s"),
        f("expected_dock_s"),
    ]))
}

/// Write a frozen day's `legs.parquet`, with `metadata` as the file's key-value metadata.
///
/// # Errors
///
/// [`WriteError`] if the file cannot be created or the Parquet writer rejects the data.
pub fn write_frozen_legs(
    path: impl AsRef<Path>,
    rows: &[FrozenLegRow],
    metadata: &[(String, String)],
) -> Result<(), WriteError> {
    let u32s = |f: fn(&FrozenLegRow) -> u32| -> ArrayRef {
        Arc::new(UInt32Array::from(rows.iter().map(f).collect::<Vec<_>>()))
    };
    let f64s = |f: fn(&FrozenLegRow) -> f64| -> ArrayRef {
        Arc::new(Float64Array::from(rows.iter().map(f).collect::<Vec<_>>()))
    };
    let strs = |f: fn(&FrozenLegRow) -> &str| -> ArrayRef {
        Arc::new(StringArray::from(rows.iter().map(f).collect::<Vec<_>>()))
    };
    let schema = schema();
    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![
            u32s(|r| r.trip),
            strs(|r| &r.traveller_id),
            u32s(|r| r.trip_seq),
            strs(|r| &r.service),
            u32s(|r| r.weight),
            f64s(|r| r.request_s),
            u32s(|r| r.origin_node),
            u32s(|r| r.destination_node),
            strs(|r| &r.dock_from),
            strs(|r| &r.dock_to),
            f64s(|r| r.ride_s),
            f64s(|r| r.egress_s),
            f64s(|r| r.expected_dock_s),
        ],
    )?;
    write_single_batch_with(path.as_ref(), schema, &batch, metadata, Compression::SNAPPY)
}

fn column<'a, T: 'static>(batch: &'a RecordBatch, name: &str) -> Result<&'a T, String> {
    batch
        .column_by_name(name)
        .ok_or_else(|| format!("legs.parquet has no column {name:?}"))?
        .as_any()
        .downcast_ref::<T>()
        .ok_or_else(|| format!("legs.parquet: column {name:?} is not of the type written"))
}

/// Read what [`write_frozen_legs`] wrote, in the file's order.
///
/// # Errors
///
/// A message for a file that cannot be read, or that lacks a column or holds a null.
pub fn read_frozen_legs(path: impl AsRef<Path>) -> Result<Vec<FrozenLegRow>, String> {
    let path = path.as_ref();
    let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let reader = ParquetRecordBatchReaderBuilder::try_new(file)
        .and_then(ParquetRecordBatchReaderBuilder::build)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let mut out = Vec::new();
    for batch in reader {
        let batch = batch.map_err(|e| format!("{}: {e}", path.display()))?;
        if (0..batch.num_columns()).any(|c| batch.column(c).null_count() > 0) {
            return Err(format!("{}: a leg with an empty cell", path.display()));
        }
        let u = |n: &str| column::<UInt32Array>(&batch, n);
        let f = |n: &str| column::<Float64Array>(&batch, n);
        let s = |n: &str| column::<StringArray>(&batch, n);
        let (trip, seq, weight, origin, destination) =
            (u("trip")?, u("trip_seq")?, u("weight")?, u("origin_node")?, u("destination_node")?);
        let (request, ride, egress, expected) =
            (f("request_s")?, f("ride_s")?, f("egress_s")?, f("expected_dock_s")?);
        let (traveller, service, from, to) =
            (s("traveller_id")?, s("service")?, s("dock_from")?, s("dock_to")?);
        for i in 0..batch.num_rows() {
            out.push(FrozenLegRow {
                trip: trip.value(i),
                traveller_id: traveller.value(i).to_string(),
                trip_seq: seq.value(i),
                service: service.value(i).to_string(),
                weight: weight.value(i),
                request_s: request.value(i),
                origin_node: origin.value(i),
                destination_node: destination.value(i),
                dock_from: from.value(i).to_string(),
                dock_to: to.value(i).to_string(),
                ride_s: ride.value(i),
                egress_s: egress.value(i),
                expected_dock_s: expected.value(i),
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legs_read_back_bit_for_bit() {
        let rows: Vec<FrozenLegRow> = (0..5)
            .map(|k| FrozenLegRow {
                trip: k * 3,
                traveller_id: format!("t{k}"),
                trip_seq: k % 2,
                service: "bike_share".into(),
                weight: k + 1,
                request_s: 28_800.0 + f64::from(k) / 3.0,
                origin_node: k,
                destination_node: 99 - k,
                dock_from: "a".into(),
                dock_to: format!("d{k}"),
                ride_s: 0.1 + f64::from(k) * 1e-9,
                egress_s: f64::from(k).sqrt(),
                expected_dock_s: -0.0,
            })
            .collect();
        let dir = std::env::temp_dir().join(format!("oms-frozen-legs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("legs.parquet");
        write_frozen_legs(&path, &rows, &[("k".into(), "v".into())]).unwrap();
        let back = read_frozen_legs(&path).unwrap();
        assert_eq!(back.len(), rows.len());
        for (a, b) in back.iter().zip(&rows) {
            assert_eq!(a, b);
            assert_eq!(a.request_s.to_bits(), b.request_s.to_bits());
            assert_eq!(a.ride_s.to_bits(), b.ride_s.to_bits());
            assert_eq!(a.expected_dock_s.to_bits(), b.expected_dock_s.to_bits());
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
