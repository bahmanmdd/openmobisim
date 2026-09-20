//! The one piece every Parquet writer in this crate shares: open a file,
//! write one batch, close. Not public — `kpis`, `diagnostics`, `events` and
//! `link_bins` each build their own schema and columns; this is just the part
//! that would otherwise be copied four times.

use std::fs::File;
use std::path::Path;
use std::sync::Arc;

use arrow_array::RecordBatch;
use arrow_schema::Schema;
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::metadata::KeyValue;
use parquet::file::properties::WriterProperties;

use crate::WriteError;

pub(crate) fn write_single_batch(
    path: &Path,
    schema: Arc<Schema>,
    batch: &RecordBatch,
) -> Result<(), WriteError> {
    write_single_batch_with(path, schema, batch, &[], Compression::UNCOMPRESSED)
}

/// [`write_single_batch`], compressed with `compression` and also recording
/// `metadata` as the file's key-value metadata, where `pq.read_metadata` and
/// other Parquet tools find it.
pub(crate) fn write_single_batch_with(
    path: &Path,
    schema: Arc<Schema>,
    batch: &RecordBatch,
    metadata: &[(String, String)],
    compression: Compression,
) -> Result<(), WriteError> {
    let file = File::create(path)?;
    let mut properties = WriterProperties::builder().set_compression(compression);
    if !metadata.is_empty() {
        properties = properties.set_key_value_metadata(Some(
            metadata.iter().map(|(k, v)| KeyValue::new(k.clone(), v.clone())).collect(),
        ));
    }
    let mut writer = ArrowWriter::try_new(file, schema, Some(properties.build()))?;
    writer.write(batch)?;
    writer.close()?;
    Ok(())
}
