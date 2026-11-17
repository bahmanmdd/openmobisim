//! Reading one input table, nodes, links or trips, from a Parquet file: its column names and,
//! by name, one column as cells of its own kind (text, integer or float), null where the file
//! holds null. Which columns a table holds, which may be null, how a cell of one kind is read
//! as a field of another (an integer id as text, a whole float as an integer) and what each row
//! means is the caller's contract (the Python binding's, `crates/binding/README.md`), the same
//! for a file as for a table in memory; this module decodes the file.
//!
//! Kinds read: text from `Utf8` or `LargeUtf8`; integers from `Int8`, `Int16`, `Int32`,
//! `Int64`, `UInt8`, `UInt16`, `UInt32` or `UInt64` (as `i128`, so every value is exact);
//! floats from `Float32` (widened exactly) or `Float64`; a column of Arrow's `Null` type
//! (pyarrow's type for a column of nulls alone) as nulls. Any other type (a boolean, a date, a
//! list) is refused, naming the column and its type. A column name the file holds twice is
//! refused by name. Compression read: none and snappy (pyarrow's default); any other codec is
//! refused, naming it, before a page is decoded.
//!
//! For bikes: a bike permission is text (`bike` in a link's or a trip's `modes`), read as any
//! other text; nothing here knows a mode.

use core::fmt;
use std::fs::File;
use std::path::Path;

use arrow_array::cast::AsArray;
use arrow_array::types::{
    Float32Type, Float64Type, Int8Type, Int16Type, Int32Type, Int64Type, UInt8Type, UInt16Type,
    UInt32Type, UInt64Type,
};
use arrow_array::{Array, ArrayRef, RecordBatch};
use arrow_schema::{DataType, SchemaRef};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::basic::Compression;

/// One column's cells in row order, of the column's own kind, `None` where the file holds null.
#[derive(Clone, Debug, PartialEq)]
pub enum Cells {
    /// Text cells.
    Text(Vec<Option<String>>),
    /// Integer cells, exact.
    Integer(Vec<Option<i128>>),
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
    /// A column's type is not one read: not text, an integer or a float.
    Type {
        /// The column.
        column: String,
        /// Its Arrow type, as Arrow writes it (`Boolean`, `Date32`).
        data_type: String,
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
            Self::Type { column, data_type } => write!(
                f,
                "column {column} has type {data_type}; a column is text (Utf8, LargeUtf8), an \
                 integer (Int8 to Int64, UInt8 to UInt64) or a float (Float32, Float64)"
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

    /// The column `name` as cells of its own kind, in row order.
    ///
    /// # Errors
    ///
    /// [`ReadError::NoColumn`] when there is no such column; [`ReadError::Type`] when its type
    /// is not text, an integer or a float.
    pub fn column(&self, name: &str) -> Result<Cells, ReadError> {
        let index =
            self.schema.index_of(name).map_err(|_| ReadError::NoColumn(name.to_string()))?;
        let data_type = self.schema.field(index).data_type();
        let arrays: Vec<&ArrayRef> = self.batches.iter().map(|b| b.column(index)).collect();
        let rows = arrays.iter().map(|a| a.len()).sum();
        let owned = |s: Option<&str>| s.map(str::to_string);
        Ok(match data_type {
            DataType::Null => Cells::Float(vec![None; rows]),
            DataType::Utf8 => Cells::Text(
                arrays.iter().flat_map(|a| a.as_string::<i32>().iter().map(owned)).collect(),
            ),
            DataType::LargeUtf8 => Cells::Text(
                arrays.iter().flat_map(|a| a.as_string::<i64>().iter().map(owned)).collect(),
            ),
            DataType::Float32 => Cells::Float(
                arrays
                    .iter()
                    .flat_map(|a| a.as_primitive::<Float32Type>().iter().map(|v| v.map(f64::from)))
                    .collect(),
            ),
            DataType::Float64 => Cells::Float(
                arrays.iter().flat_map(|a| a.as_primitive::<Float64Type>().iter()).collect(),
            ),
            t => Cells::Integer(integers(t, &arrays).ok_or_else(|| ReadError::Type {
                column: name.to_string(),
                data_type: data_type.to_string(),
            })?),
        })
    }
}

/// The cells of an integer column of `data_type`, exact; `None` when it is not an integer type.
fn integers(data_type: &DataType, arrays: &[&ArrayRef]) -> Option<Vec<Option<i128>>> {
    fn all<T: arrow_array::ArrowPrimitiveType>(arrays: &[&ArrayRef]) -> Vec<Option<i128>>
    where
        T::Native: Into<i128>,
    {
        arrays
            .iter()
            .flat_map(|a| a.as_primitive::<T>().iter().map(|v| v.map(Into::into)))
            .collect()
    }
    Some(match data_type {
        DataType::Int8 => all::<Int8Type>(arrays),
        DataType::Int16 => all::<Int16Type>(arrays),
        DataType::Int32 => all::<Int32Type>(arrays),
        DataType::Int64 => all::<Int64Type>(arrays),
        DataType::UInt8 => all::<UInt8Type>(arrays),
        DataType::UInt16 => all::<UInt16Type>(arrays),
        DataType::UInt32 => all::<UInt32Type>(arrays),
        DataType::UInt64 => all::<UInt64Type>(arrays),
        _ => return None,
    })
}
