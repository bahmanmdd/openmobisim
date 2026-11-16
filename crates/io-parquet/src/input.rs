//! Reading one input table, nodes, links or trips, from a Parquet file: its column names and,
//! by name, one column as cells of a kind (text, integer or float), null where the file holds
//! null. Which columns a table holds, which may be null and what each row means is the caller's
//! contract (the Python binding's, `crates/binding/README.md`), the same for a file as for a
//! table in memory; this module decodes the file and checks each column's type against the kind
//! asked for.
//!
//! Types read: text from `Utf8` or `LargeUtf8`; integers from `Int32`, `Int64`, `UInt32` or
//! `UInt64` (a `UInt64` above 2⁶³ − 1 is refused as out of range); floats from `Float64` or one
//! of those four integer types; a column of Arrow's `Null` type (pyarrow's type for a column of
//! nulls alone) as nulls of any kind. Any other type is refused, naming the column and its type.
//! A column name the file holds twice is refused by name.
//! Compression read: none and snappy (pyarrow's default); any other codec is refused, naming it,
//! before a page is decoded.
//!
//! For bikes: a bike permission is text (`bike` in a link's or a trip's `modes`), read as any
//! other text; nothing here knows a mode.

use core::fmt;
use std::fs::File;
use std::path::Path;

use arrow_array::cast::AsArray;
use arrow_array::types::{Float64Type, Int32Type, Int64Type, UInt32Type, UInt64Type};
use arrow_array::{Array, ArrayRef, RecordBatch};
use arrow_schema::{DataType, SchemaRef};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::basic::Compression;

/// The kind of cell a column is read as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Text: `Utf8` or `LargeUtf8`.
    Text,
    /// An integer: `Int32`, `Int64`, `UInt32` or `UInt64`.
    Integer,
    /// A float: `Float64`, or one of the four integer types.
    Float,
}

impl Kind {
    /// The Arrow types read as this kind, for a message.
    fn types(self) -> &'static str {
        match self {
            Self::Text => "a text column is Utf8 or LargeUtf8",
            Self::Integer => "an integer column is Int32, Int64, UInt32 or UInt64",
            Self::Float => "a float column is Float64, Int32, Int64, UInt32 or UInt64",
        }
    }
}

/// One column's cells in row order, `None` where the file holds null.
#[derive(Clone, Debug, PartialEq)]
pub enum Cells {
    /// Text cells.
    Text(Vec<Option<String>>),
    /// Integer cells.
    Integer(Vec<Option<i64>>),
    /// Float cells.
    Float(Vec<Option<f64>>),
}

/// Why a file was not read.
#[derive(Debug)]
pub enum ReadError {
    /// The file could not be opened or read.
    Io(std::io::Error),
    /// A column chunk is compressed by a codec other than snappy.
    Codec(&'static str),
    /// The file has no column of this name.
    NoColumn(String),
    /// The file has two columns of this name.
    TwoColumns(String),
    /// A column's type is not one read as the kind asked for.
    Type {
        /// The column.
        column: String,
        /// Its Arrow type, as Arrow writes it (`Float32`, `Boolean`).
        data_type: String,
        /// The kind asked for.
        kind: Kind,
    },
    /// A `UInt64` cell above 2⁶³ − 1.
    OutOfRange {
        /// The column.
        column: String,
        /// The row, from 0.
        row: usize,
        /// The value.
        value: u64,
    },
    /// The Parquet or Arrow decoder's refusal: not a Parquet file, or a damaged one.
    Parquet(String),
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "could not read the file: {e}"),
            Self::Codec(codec) => write!(
                f,
                "the file is compressed by {codec}; the reader reads uncompressed and snappy files"
            ),
            Self::NoColumn(column) => write!(f, "the file has no column {column}"),
            Self::TwoColumns(column) => write!(f, "the file has two columns {column}"),
            Self::Type { column, data_type, kind } => {
                write!(f, "column {column} has type {data_type}; {}", kind.types())
            }
            Self::OutOfRange { column, row, value } => write!(
                f,
                "column {column}, row {row}: {value} is above 9223372036854775807 (2^63 - 1), the \
                 largest integer read"
            ),
            Self::Parquet(m) => write!(f, "not a readable Parquet file: {m}"),
        }
    }
}

impl core::error::Error for ReadError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<parquet::errors::ParquetError> for ReadError {
    fn from(e: parquet::errors::ParquetError) -> Self {
        Self::Parquet(e.to_string())
    }
}

impl From<arrow_schema::ArrowError> for ReadError {
    fn from(e: arrow_schema::ArrowError) -> Self {
        Self::Parquet(e.to_string())
    }
}

/// A Parquet file read whole: its schema and its record batches.
#[derive(Debug)]
pub struct InputTable {
    schema: SchemaRef,
    batches: Vec<RecordBatch>,
}

/// Read the Parquet file at `path` whole.
///
/// # Errors
///
/// [`ReadError::Io`] when it cannot be opened or read (a directory included);
/// [`ReadError::Codec`] when a column chunk is compressed by another codec than snappy;
/// [`ReadError::TwoColumns`] when a column name is held twice; [`ReadError::Parquet`] when it is
/// not a Parquet file or cannot be decoded.
pub fn read_table(path: &Path) -> Result<InputTable, ReadError> {
    let file = File::open(path).map_err(ReadError::Io)?;
    if file.metadata().map_err(ReadError::Io)?.is_dir() {
        let e = std::io::Error::new(std::io::ErrorKind::InvalidInput, "a directory, not a file");
        return Err(ReadError::Io(e));
    }
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    for group in builder.metadata().row_groups() {
        for chunk in group.columns() {
            let codec = match chunk.compression() {
                Compression::UNCOMPRESSED | Compression::SNAPPY => continue,
                Compression::GZIP(_) => "gzip",
                Compression::LZO => "lzo",
                Compression::BROTLI(_) => "brotli",
                Compression::LZ4 => "lz4",
                Compression::ZSTD(_) => "zstd",
                Compression::LZ4_RAW => "lz4_raw",
            };
            return Err(ReadError::Codec(codec));
        }
    }
    let schema = builder.schema().clone();
    let names: Vec<&String> = schema.fields().iter().map(|f| f.name()).collect();
    if let Some((_, twice)) = names.iter().enumerate().find(|(k, n)| names[..*k].contains(n)) {
        return Err(ReadError::TwoColumns((*twice).clone()));
    }
    let batches = builder.build()?.collect::<Result<Vec<_>, _>>()?;
    Ok(InputTable { schema, batches })
}

impl InputTable {
    /// The column names, in the file's order.
    #[must_use]
    pub fn names(&self) -> Vec<String> {
        self.schema.fields().iter().map(|f| f.name().clone()).collect()
    }

    /// The column `name` read as cells of `kind`, in row order.
    ///
    /// # Errors
    ///
    /// [`ReadError::NoColumn`] when there is no such column; [`ReadError::Type`] when its type
    /// is not read as `kind`; [`ReadError::OutOfRange`] for a `UInt64` cell above 2⁶³ − 1.
    pub fn column(&self, name: &str, kind: Kind) -> Result<Cells, ReadError> {
        let index =
            self.schema.index_of(name).map_err(|_| ReadError::NoColumn(name.to_string()))?;
        let data_type = self.schema.field(index).data_type();
        let arrays: Vec<&ArrayRef> = self.batches.iter().map(|b| b.column(index)).collect();
        let text = |a: &&ArrayRef| -> Vec<Option<String>> {
            let owned = |s: Option<&str>| s.map(str::to_string);
            match data_type {
                DataType::Utf8 => a.as_string::<i32>().iter().map(owned).collect(),
                _ => a.as_string::<i64>().iter().map(owned).collect(),
            }
        };
        let rows = arrays.iter().map(|a| a.len()).sum();
        match (kind, data_type) {
            (Kind::Text, DataType::Null) => Ok(Cells::Text(vec![None; rows])),
            (Kind::Integer, DataType::Null) => Ok(Cells::Integer(vec![None; rows])),
            (Kind::Float, DataType::Null) => Ok(Cells::Float(vec![None; rows])),
            (Kind::Text, DataType::Utf8 | DataType::LargeUtf8) => {
                Ok(Cells::Text(arrays.iter().flat_map(text).collect()))
            }
            (Kind::Float, DataType::Float64) => {
                let floats = arrays.iter().flat_map(|a| a.as_primitive::<Float64Type>().iter());
                Ok(Cells::Float(floats.collect()))
            }
            (Kind::Integer, t) if is_integer(t) => {
                Ok(Cells::Integer(integers(name, data_type, &arrays)?))
            }
            // An integer read as a float rounds to the nearest, as Python's `float(int)` does:
            // exact up to 2⁵³.
            #[allow(clippy::cast_precision_loss, reason = "rounded to the nearest, as Python does")]
            (Kind::Float, t) if is_integer(t) => {
                let integers = integers(name, data_type, &arrays)?;
                Ok(Cells::Float(integers.into_iter().map(|v| v.map(|v| v as f64)).collect()))
            }
            _ => Err(ReadError::Type {
                column: name.to_string(),
                data_type: data_type.to_string(),
                kind,
            }),
        }
    }
}

/// Whether `data_type` is one of the four integer types read.
fn is_integer(data_type: &DataType) -> bool {
    matches!(data_type, DataType::Int32 | DataType::Int64 | DataType::UInt32 | DataType::UInt64)
}

/// The cells of an integer column of `data_type` (one of the four) as `i64`: a `UInt64` above
/// 2⁶³ − 1 is refused.
fn integers(
    name: &str,
    data_type: &DataType,
    arrays: &[&ArrayRef],
) -> Result<Vec<Option<i64>>, ReadError> {
    let mut cells = Vec::with_capacity(arrays.iter().map(|a| a.len()).sum());
    for a in arrays {
        match data_type {
            DataType::Int32 => {
                cells.extend(a.as_primitive::<Int32Type>().iter().map(|v| v.map(i64::from)));
            }
            DataType::Int64 => cells.extend(a.as_primitive::<Int64Type>().iter()),
            DataType::UInt32 => {
                cells.extend(a.as_primitive::<UInt32Type>().iter().map(|v| v.map(i64::from)));
            }
            _ => {
                for value in a.as_primitive::<UInt64Type>().iter() {
                    let cell = value.map(|v| {
                        i64::try_from(v).map_err(|_| ReadError::OutOfRange {
                            column: name.to_string(),
                            row: cells.len(),
                            value: v,
                        })
                    });
                    cells.push(cell.transpose()?);
                }
            }
        }
    }
    Ok(cells)
}
