//! `diagnostics.parquet` — aggregated counters (Foundations §6):
//! `run_id, category, code, severity, element_kind, element_id, count`.
//!
//! One row per `(category, code, severity, element)`, in the deterministic
//! order [`Diagnostics::rows`] already guarantees — the same order on every
//! run of the same scenario, which is what lets the run-twice CI gate (S111)
//! compare two runs' `diagnostics.parquet` byte for byte once it exists.
//!
//! No `detail` column: Foundations §6 names one, but [`Diagnostics`] does
//! not carry free-text detail yet (S131 already noted this gap when it was
//! found). Added when it does.

use std::path::Path;
use std::sync::Arc;

use arrow_array::{ArrayRef, RecordBatch, StringArray, UInt32Array, UInt64Array};
use arrow_schema::{DataType, Field, Schema};
use openmobisim_core_types::diagnostics::Diagnostics;

use crate::WriteError;
use crate::io::write_single_batch;

/// Write `diagnostics.parquet` for one run.
///
/// # Errors
///
/// [`WriteError`] if the file cannot be created or the Parquet writer
/// rejects the data.
pub fn write_diagnostics(
    path: impl AsRef<Path>,
    run_id: &str,
    diagnostics: &Diagnostics,
) -> Result<(), WriteError> {
    let rows = diagnostics.rows();
    let n = rows.len();

    let run_id_col: ArrayRef = Arc::new(StringArray::from(vec![run_id; n]));
    let category_col: ArrayRef = Arc::new(StringArray::from(
        rows.iter().map(|r| r.key.category.as_str()).collect::<Vec<_>>(),
    ));
    let code_col: ArrayRef =
        Arc::new(StringArray::from(rows.iter().map(|r| r.key.code.as_str()).collect::<Vec<_>>()));
    let severity_col: ArrayRef = Arc::new(StringArray::from(
        rows.iter().map(|r| r.key.severity.as_str()).collect::<Vec<_>>(),
    ));
    let element_kind_col: ArrayRef = Arc::new(StringArray::from(
        rows.iter().map(|r| r.key.element.kind().map(|k| k.as_str())).collect::<Vec<_>>(),
    ));
    let element_id_col: ArrayRef =
        Arc::new(UInt32Array::from(rows.iter().map(|r| r.key.element.id()).collect::<Vec<_>>()));
    let count_col: ArrayRef =
        Arc::new(UInt64Array::from(rows.iter().map(|r| r.count).collect::<Vec<_>>()));

    let schema = Arc::new(Schema::new(vec![
        Field::new("run_id", DataType::Utf8, false),
        Field::new("category", DataType::Utf8, false),
        Field::new("code", DataType::Utf8, false),
        Field::new("severity", DataType::Utf8, false),
        Field::new("element_kind", DataType::Utf8, true),
        Field::new("element_id", DataType::UInt32, true),
        Field::new("count", DataType::UInt64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            run_id_col,
            category_col,
            code_col,
            severity_col,
            element_kind_col,
            element_id_col,
            count_col,
        ],
    )?;

    write_single_batch(path.as_ref(), schema, &batch)
}
