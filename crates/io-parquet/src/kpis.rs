//! `kpis.parquet` — long format (Foundations §6):
//! `run_id, design_id, replication, iteration, metric, value`.
//!
//! A new KPI needs no schema change, only a new row — which is exactly what
//! lets Phase 1 write a real (if short) `kpis.parquet` today and grow the
//! set of metrics later without touching this file's shape.

use std::path::Path;
use std::sync::Arc;

use arrow_array::{ArrayRef, Float64Array, RecordBatch, StringArray, UInt32Array};
use arrow_schema::{DataType, Field, Schema};
use openmobisim_core_sim::RunResult;

use crate::WriteError;
use crate::io::write_single_batch;

/// Every metric this Phase 1 cut can report, as `(name, value)` pairs.
///
/// `total_travel_time_s` is traveller-weight-scaled (`Σ w · travel_time`) —
/// see `core-sim`'s own docs for why, and for the standing note that an
/// unweighted metric row joins this list, rather than replacing this one,
/// once comprehensive KPIs exist (confirmed by the user, 2026-09-14).
fn metrics(result: &RunResult) -> [(&'static str, f64); 6] {
    let c = &result.completion;
    [
        ("total_travel_time_s", result.total_travel_time.get()),
        ("completed_trips", f64::from(c.completed)),
        ("truncated_trips", f64::from(c.truncated)),
        ("no_vehicle_available_trips", f64::from(c.no_vehicle_available)),
        ("no_feasible_path_trips", f64::from(c.no_feasible_path)),
        ("completion_rate", c.completion_rate()),
    ]
}

/// Write `kpis.parquet` for one run.
///
/// `design_id`, `replication` and `iteration` are accepted as given rather
/// than computed here: Phase 1 has no `DesignSpace`/`run_batch` (P7/P12) and
/// no equilibration, so there is exactly one design, one replication and one
/// (implicit) iteration — the caller states that rather than this module
/// inventing an identity for concepts that do not exist yet.
///
/// # Errors
///
/// [`WriteError`] if the file cannot be created or the Parquet writer
/// rejects the data.
pub fn write_kpis(
    path: impl AsRef<Path>,
    run_id: &str,
    design_id: &str,
    replication: u32,
    iteration: u32,
    result: &RunResult,
) -> Result<(), WriteError> {
    let rows = metrics(result);
    let n = rows.len();

    let run_id_col: ArrayRef = Arc::new(StringArray::from(vec![run_id; n]));
    let design_id_col: ArrayRef = Arc::new(StringArray::from(vec![design_id; n]));
    let replication_col: ArrayRef = Arc::new(UInt32Array::from(vec![replication; n]));
    let iteration_col: ArrayRef = Arc::new(UInt32Array::from(vec![iteration; n]));
    let metric_col: ArrayRef =
        Arc::new(StringArray::from(rows.iter().map(|(name, _)| *name).collect::<Vec<_>>()));
    let value_col: ArrayRef =
        Arc::new(Float64Array::from(rows.iter().map(|(_, value)| *value).collect::<Vec<_>>()));

    let schema = Arc::new(Schema::new(vec![
        Field::new("run_id", DataType::Utf8, false),
        Field::new("design_id", DataType::Utf8, false),
        Field::new("replication", DataType::UInt32, false),
        Field::new("iteration", DataType::UInt32, false),
        Field::new("metric", DataType::Utf8, false),
        Field::new("value", DataType::Float64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![run_id_col, design_id_col, replication_col, iteration_col, metric_col, value_col],
    )?;

    write_single_batch(path.as_ref(), schema, &batch)
}
