//! `service_legs.parquet` — every shared-bike leg of a run's last loading, and what became of it.
//!
//! One row per leg, in the order the fleet engine played them (by trip):
//! `run_id, service, mode, traveller_id, trip_seq, weight, requested_s, picked_s, arrived_s,
//! hub_from, hub_to, dock_from, dock_to, waited_s, extra_walk_s, redirections, expected_dock_s,
//! met_dock_s, outcome`. Written only by a run with a service; the same table
//! `Run.service_legs()` returns in Python.
//!
//! - **Who**: `service` is the service's name; `mode` the trip's mode, `bike_share` (a shared
//!   bike from near the origin to near the destination) or `bike_share_transit` (a shared bike to
//!   a stop's hub, transit on from there); `traveller_id` and `trip_seq` name the trip as
//!   `trip_results.parquet` does; `weight` is the people the leg stands for, the bikes it takes.
//! - **When**, in seconds from midnight: `requested_s` the leg asked for a bike at its planned
//!   pickup dock (the trip's departure plus the walk there); `picked_s` it had a bike (null: none
//!   was found); `arrived_s` it reached the leg's end (the trip's destination; for
//!   `bike_share_transit`, the dock at the stop's hub), riding or, after a failed pickup, on foot.
//! - **Where**: `hub_from` and `hub_to` are the hubs (in the run's hub set) of the pickup and
//!   return docks the trip's choice planned; `dock_from` and `dock_to` the docks actually used
//!   (null: no bike taken, or not returned).
//! - **What it cost**, in seconds: `waited_s` the wait for a vehicle (0 for a docked bike: a
//!   traveller who finds a dock empty walks on to another instead of waiting); `extra_walk_s` the
//!   walk between docks looking for a bike; `redirections` the docks tried after the planned one,
//!   at the pickup and at the return; `expected_dock_s` the extra time at the docks the trip's
//!   choice expected; `met_dock_s` the extra time it met, its arrival less its arrival had the
//!   planned pickup dock had a bike and the planned return dock a free slot (null: it did not
//!   arrive). `met_dock_s − expected_dock_s` is the leg's mismatch; its mean weighted by
//!   `weight` is the `service_bias_s` row of `kpis.parquet`.
//! - **`outcome`**: `served` (a bike taken and docked), `failed` (no bike found within the
//!   docks tried; the traveller walked on from the last dock tried), `truncated` (under way when
//!   the window ended), `waiting` or `riding` (left so at the window's end), `not_requested`
//!   (its request came after the window's end).
//! - The key-value metadata carries `openmobisim.run_fingerprint` and `openmobisim.master_seed`.
//!
//! **Cost.** About 150 bytes per row while writing.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use arrow_array::{ArrayRef, Float64Array, RecordBatch, StringArray, UInt32Array};
use arrow_schema::{DataType, Field, Schema};
use openmobisim_core_sim::RunDescription;
use openmobisim_core_sim::services::tables::ServiceLegTable;
use parquet::basic::Compression;

use crate::WriteError;
use crate::io::write_single_batch_with;

/// Write `service_legs.parquet` for one run.
///
/// # Errors
///
/// [`WriteError`] if the file cannot be created or the Parquet writer rejects the data.
pub fn write_service_legs(
    path: impl AsRef<Path>,
    run_id: &str,
    legs: &ServiceLegTable,
    description: &RunDescription,
) -> Result<(), WriteError> {
    let n = legs.len();
    let text = |v: &[String]| -> ArrayRef {
        Arc::new(StringArray::from(v.iter().map(String::as_str).collect::<Vec<_>>()))
    };
    let maybe_text = |v: &[Option<String>]| -> ArrayRef {
        Arc::new(StringArray::from(v.iter().map(Option::as_deref).collect::<Vec<_>>()))
    };
    let maybe = |v: &[Option<f64>]| -> ArrayRef { Arc::new(Float64Array::from(v.to_vec())) };
    let float = |v: &[f64]| -> ArrayRef { Arc::new(Float64Array::from(v.to_vec())) };
    let columns: Vec<ArrayRef> = vec![
        Arc::new(StringArray::from(vec![run_id; n])),
        text(&legs.service),
        Arc::new(StringArray::from(legs.mode.clone())),
        text(&legs.traveller_id),
        Arc::new(UInt32Array::from(legs.trip_seq.clone())),
        Arc::new(UInt32Array::from(legs.weight.clone())),
        maybe(&legs.requested_s),
        maybe(&legs.picked_s),
        maybe(&legs.arrived_s),
        text(&legs.hub_from),
        text(&legs.hub_to),
        maybe_text(&legs.dock_from),
        maybe_text(&legs.dock_to),
        float(&legs.waited_s),
        float(&legs.extra_walk_s),
        Arc::new(UInt32Array::from(legs.redirections.clone())),
        float(&legs.expected_dock_s),
        maybe(&legs.met_dock_s),
        Arc::new(StringArray::from(legs.outcome.clone())),
    ];
    let metadata = vec![
        ("openmobisim.run_fingerprint".to_string(), description.fingerprint_hex()),
        ("openmobisim.master_seed".to_string(), description.master_seed.to_string()),
    ];
    let field = |name: &str, t: DataType, null: bool| Field::new(name, t, null);
    let (s, u, f) = (DataType::Utf8, DataType::UInt32, DataType::Float64);
    let schema = Arc::new(
        Schema::new(vec![
            field("run_id", s.clone(), false),
            field("service", s.clone(), false),
            field("mode", s.clone(), false),
            field("traveller_id", s.clone(), false),
            field("trip_seq", u.clone(), false),
            field("weight", u.clone(), false),
            field("requested_s", f.clone(), true),
            field("picked_s", f.clone(), true),
            field("arrived_s", f.clone(), true),
            field("hub_from", s.clone(), false),
            field("hub_to", s.clone(), false),
            field("dock_from", s.clone(), true),
            field("dock_to", s.clone(), true),
            field("waited_s", f.clone(), false),
            field("extra_walk_s", f.clone(), false),
            field("redirections", u, false),
            field("expected_dock_s", f.clone(), false),
            field("met_dock_s", f, true),
            field("outcome", s, false),
        ])
        .with_metadata(metadata.iter().cloned().collect::<HashMap<_, _>>()),
    );
    let batch = RecordBatch::try_new(schema.clone(), columns)?;
    write_single_batch_with(path.as_ref(), schema, &batch, &metadata, Compression::SNAPPY)
}
