//! Typed column extraction from Arrow `RecordBatch`es.
//!
//! Not a public API: `trips` and `persons` are the two readers, and both need
//! the same handful of column shapes (an id that may be string or integer, a
//! required `f64`, a required or optional integer, an optional `bool`,
//! an optional string). Kept here once rather than duplicated in each.

use std::fs::File;
use std::path::Path;

use arrow_array::{
    Array, BooleanArray, Float64Array, Int32Array, Int64Array, RecordBatch, StringArray,
    UInt32Array, UInt64Array,
};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

use crate::DemandError;

/// Read every record batch in a Parquet file, in file order.
pub(crate) fn read_batches(path: &Path) -> Result<Vec<RecordBatch>, DemandError> {
    let file = File::open(path)?;
    let reader = ParquetRecordBatchReaderBuilder::try_new(file)?.build()?;
    let mut batches = Vec::new();
    for batch in reader {
        batches.push(batch?);
    }
    Ok(batches)
}

fn column<'a>(batch: &'a RecordBatch, name: &str) -> Option<&'a std::sync::Arc<dyn Array>> {
    batch.column_by_name(name)
}

/// An id column, `traveller_id` in the schema tables: "string/int". Integers
/// are rendered with `to_string`, so `7` and `"7"` become the same external
/// id — the same convention `io-osm` uses for OSM node and way ids.
pub(crate) fn required_id(batch: &RecordBatch, name: &str) -> Result<Vec<String>, DemandError> {
    let array = column(batch, name)
        .ok_or_else(|| DemandError::Schema(format!("column `{name}` is missing")))?;
    if let Some(a) = array.as_any().downcast_ref::<StringArray>() {
        return Ok((0..a.len()).map(|i| a.value(i).to_string()).collect());
    }
    if let Some(a) = array.as_any().downcast_ref::<Int64Array>() {
        return Ok((0..a.len()).map(|i| a.value(i).to_string()).collect());
    }
    if let Some(a) = array.as_any().downcast_ref::<Int32Array>() {
        return Ok((0..a.len()).map(|i| a.value(i).to_string()).collect());
    }
    Err(DemandError::Schema(format!(
        "column `{name}` must be a string or integer id, got {:?}",
        array.data_type()
    )))
}

/// A required `f64` column (WGS84 coordinates, per Foundations §9).
pub(crate) fn required_f64(batch: &RecordBatch, name: &str) -> Result<Vec<f64>, DemandError> {
    let array = column(batch, name)
        .ok_or_else(|| DemandError::Schema(format!("column `{name}` is missing")))?;
    let a = array.as_any().downcast_ref::<Float64Array>().ok_or_else(|| {
        DemandError::Schema(format!("column `{name}` must be float64, got {:?}", array.data_type()))
    })?;
    Ok((0..a.len()).map(|i| a.value(i)).collect())
}

/// A required non-negative integer column, widened to `i64` and then checked.
pub(crate) fn required_u32(batch: &RecordBatch, name: &str) -> Result<Vec<u32>, DemandError> {
    required_i64(batch, name)?
        .into_iter()
        .map(|v| {
            u32::try_from(v).map_err(|_| {
                DemandError::Schema(format!(
                    "column `{name}` has a negative or oversized value: {v}"
                ))
            })
        })
        .collect()
}

fn required_i64(batch: &RecordBatch, name: &str) -> Result<Vec<i64>, DemandError> {
    let array = column(batch, name)
        .ok_or_else(|| DemandError::Schema(format!("column `{name}` is missing")))?;
    integer_column(array, name)
}

fn integer_column(array: &std::sync::Arc<dyn Array>, name: &str) -> Result<Vec<i64>, DemandError> {
    if let Some(a) = array.as_any().downcast_ref::<Int64Array>() {
        return Ok((0..a.len()).map(|i| a.value(i)).collect());
    }
    if let Some(a) = array.as_any().downcast_ref::<Int32Array>() {
        return Ok((0..a.len()).map(|i| i64::from(a.value(i))).collect());
    }
    if let Some(a) = array.as_any().downcast_ref::<UInt32Array>() {
        return Ok((0..a.len()).map(|i| i64::from(a.value(i))).collect());
    }
    if let Some(a) = array.as_any().downcast_ref::<UInt64Array>() {
        return (0..a.len())
            .map(|i| {
                i64::try_from(a.value(i)).map_err(|_| {
                    DemandError::Schema(format!("column `{name}` value out of i64 range"))
                })
            })
            .collect();
    }
    Err(DemandError::Schema(format!(
        "column `{name}` must be an integer, got {:?}",
        array.data_type()
    )))
}

/// An optional integer column: `None` if the column is absent from the file,
/// `Some` with one `Option<u32>` per row otherwise (a row may still be null).
pub(crate) fn optional_u32(
    batch: &RecordBatch,
    name: &str,
) -> Result<Option<Vec<Option<u32>>>, DemandError> {
    let Some(array) = column(batch, name) else { return Ok(None) };
    let raw = integer_column(array, name)?;
    let mut out = Vec::with_capacity(raw.len());
    for (i, v) in raw.into_iter().enumerate() {
        if array.is_null(i) {
            out.push(None);
            continue;
        }
        let v = u32::try_from(v).map_err(|_| {
            DemandError::Schema(format!("column `{name}` has a negative or oversized value: {v}"))
        })?;
        out.push(Some(v));
    }
    Ok(Some(out))
}

/// An optional boolean column: `None` if the column is absent from the file
/// (S127: "omit the column to take the class default for everyone"), `Some`
/// with one `Option<bool>` per row otherwise (a present column may still hold
/// a null for one traveller, which falls back the same way).
pub(crate) fn optional_bool(
    batch: &RecordBatch,
    name: &str,
) -> Result<Option<Vec<Option<bool>>>, DemandError> {
    let Some(array) = column(batch, name) else { return Ok(None) };
    let a = array.as_any().downcast_ref::<BooleanArray>().ok_or_else(|| {
        DemandError::Schema(format!("column `{name}` must be boolean, got {:?}", array.data_type()))
    })?;
    Ok(Some((0..a.len()).map(|i| (!a.is_null(i)).then(|| a.value(i))).collect()))
}

/// An optional string column, `None` if absent from the file entirely.
pub(crate) fn optional_string(
    batch: &RecordBatch,
    name: &str,
) -> Result<Option<Vec<Option<String>>>, DemandError> {
    let Some(array) = column(batch, name) else { return Ok(None) };
    let a = array.as_any().downcast_ref::<StringArray>().ok_or_else(|| {
        DemandError::Schema(format!(
            "column `{name}` must be a string, got {:?}",
            array.data_type()
        ))
    })?;
    Ok(Some((0..a.len()).map(|i| (!a.is_null(i)).then(|| a.value(i).to_string())).collect()))
}
