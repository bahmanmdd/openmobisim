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
/// other Parquet tools find it. Every file also carries the layout version
/// ([`crate::SCHEMA_VERSION_KEY`]).
pub(crate) fn write_single_batch_with(
    path: &Path,
    schema: Arc<Schema>,
    batch: &RecordBatch,
    metadata: &[(String, String)],
    compression: Compression,
) -> Result<(), WriteError> {
    let file = File::create(path)?;
    let version =
        KeyValue::new(crate::SCHEMA_VERSION_KEY.to_string(), crate::SCHEMA_VERSION.to_string());
    let key_values: Vec<KeyValue> = metadata
        .iter()
        .map(|(k, v)| KeyValue::new(k.clone(), v.clone()))
        .chain(std::iter::once(version))
        .collect();
    let properties = WriterProperties::builder()
        .set_compression(compression)
        .set_key_value_metadata(Some(key_values));
    let mut writer = ArrowWriter::try_new(file, schema, Some(properties.build()))?;
    writer.write(batch)?;
    writer.close()?;
    Ok(())
}
