//! The one piece every Parquet writer in this crate shares: open a file,
//! write one batch, close. Not public — `kpis`, `diagnostics` and `events`
//! each build their own schema and columns; this is just the part that
//! would otherwise be copied three times.

use std::fs::File;
use std::path::Path;
use std::sync::Arc;

use arrow_array::RecordBatch;
use arrow_schema::Schema;
use parquet::arrow::ArrowWriter;

use crate::WriteError;

pub(crate) fn write_single_batch(
    path: &Path,
    schema: Arc<Schema>,
    batch: &RecordBatch,
) -> Result<(), WriteError> {
    let file = File::create(path)?;
    let mut writer = ArrowWriter::try_new(file, schema, None)?;
    writer.write(batch)?;
    writer.close()?;
    Ok(())
}
