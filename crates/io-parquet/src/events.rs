//! `events.parquet` — sampled per-trip events (Foundations §6):
//! `run_id, second, event_type, entity_kind, entity_id`.
//!
//! No `payload` column: [`EventRow`] does not carry one yet — Phase 1 has no
//! boardings, hub or store events, or disruptions to put one on. Added when
//! one of those exists to produce it.

use std::path::Path;
use std::sync::Arc;

use arrow_array::{ArrayRef, RecordBatch, StringArray, UInt32Array};
use arrow_schema::{DataType, Field, Schema};
use openmobisim_core_sim::EventRow;

use crate::WriteError;
use crate::io::write_single_batch;

/// Foundations §6's stated default: "sampled at 1% by default."
pub const DEFAULT_SAMPLE_RATE: f64 = 0.01;

/// Keep every `round(1 / sample_rate)`-th row, by position — a fixed
/// stride, not a random draw, so the sample is a pure function of `events`'
/// own (already deterministic) order rather than of a seed this crate has
/// no reason to own. `sample_rate >= 1.0` keeps everything;
/// `events` empty always keeps nothing.
fn sample(events: &[EventRow], sample_rate: f64) -> Vec<&EventRow> {
    if events.is_empty() {
        return Vec::new();
    }
    if sample_rate >= 1.0 {
        return events.iter().collect();
    }
    let rate = sample_rate.max(f64::MIN_POSITIVE);
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "rate is positive and finite by construction; the stride is a small positive \
                  integer in any realistic sample rate"
    )]
    let stride = (1.0 / rate).round().max(1.0) as usize;
    events.iter().step_by(stride).collect()
}

/// Write `events.parquet` for one run, sampling `events` at `sample_rate`
/// ([`DEFAULT_SAMPLE_RATE`] matches Foundations §6's stated default).
///
/// # Errors
///
/// [`WriteError`] if the file cannot be created or the Parquet writer
/// rejects the data.
pub fn write_events(
    path: impl AsRef<Path>,
    run_id: &str,
    events: &[EventRow],
    sample_rate: f64,
) -> Result<(), WriteError> {
    let sampled = sample(events, sample_rate);
    let n = sampled.len();

    let run_id_col: ArrayRef = Arc::new(StringArray::from(vec![run_id; n]));
    let second_col: ArrayRef =
        Arc::new(UInt32Array::from(sampled.iter().map(|e| e.second.get()).collect::<Vec<_>>()));
    let event_type_col: ArrayRef = Arc::new(StringArray::from(
        sampled.iter().map(|e| e.event_type.as_str()).collect::<Vec<_>>(),
    ));
    let entity_kind_col: ArrayRef = Arc::new(StringArray::from(
        sampled.iter().map(|e| e.entity_kind.as_str()).collect::<Vec<_>>(),
    ));
    let entity_id_col: ArrayRef =
        Arc::new(UInt32Array::from(sampled.iter().map(|e| e.entity_id).collect::<Vec<_>>()));

    let schema = Arc::new(Schema::new(vec![
        Field::new("run_id", DataType::Utf8, false),
        Field::new("second", DataType::UInt32, false),
        Field::new("event_type", DataType::Utf8, false),
        Field::new("entity_kind", DataType::Utf8, false),
        Field::new("entity_id", DataType::UInt32, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![run_id_col, second_col, event_type_col, entity_kind_col, entity_id_col],
    )?;

    write_single_batch(path.as_ref(), schema, &batch)
}
