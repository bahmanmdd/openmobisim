//! Writing the three tables of a run (trips, links by time bin, iterations) as three Parquet
//! files in a folder, and nothing else.
//!
//! [`write`] takes the `Run` the loop returned, the `Network` and the `Demand` it ran on (for the
//! external ids), and a folder; it writes `trips.parquet`, `links.parquet` and
//! `iterations.parquet` there and returns their paths and row counts ([`Written`]). One row per
//! entity, external ids as strings, every column named with its unit; null, never `NaN`, where
//! the loop's `NaN` means "no value". The files are written with the `parquet` crate's default
//! settings: no compression, dictionary encoding, one row group per 1 048 576 rows.

use core::fmt;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow_array::RecordBatch;
use openmobisim_demand::demand::Demand;
use openmobisim_graph::network::Network;
use openmobisim_loop::run::Run;
use openmobisim_types::ids::EntityId;
use parquet::arrow::ArrowWriter;

mod iterations;
mod links;
mod trips;

/// Why a write was refused or failed.
#[derive(Debug)]
pub enum WriteError {
    /// The demand given is not the run's: its trip count differs from the run's trips table.
    NotTheRunsDemand {
        /// Rows of the run's trips table.
        run_trips: usize,
        /// Trips of the demand given.
        demand_trips: u32,
    },
    /// The network given is not the run's: a row of the links table names a link it lacks.
    NotTheRunsNetwork {
        /// The link's internal id in the run.
        link: u32,
        /// Links of the network given.
        network_links: u32,
    },
    /// The folder or a file could not be created or written.
    Io(std::io::Error),
    /// The Arrow or Parquet writer rejected the data.
    Parquet(String),
}

impl fmt::Display for WriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotTheRunsDemand { run_trips, demand_trips } => write!(
                f,
                "the run has {run_trips} trips and the demand {demand_trips}: not the run's demand"
            ),
            Self::NotTheRunsNetwork { link, network_links } => write!(
                f,
                "the run names link {link} and the network has {network_links}: not the run's network"
            ),
            Self::Io(e) => write!(f, "could not write the file: {e}"),
            Self::Parquet(m) => write!(f, "could not encode the table: {m}"),
        }
    }
}

impl core::error::Error for WriteError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for WriteError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<parquet::errors::ParquetError> for WriteError {
    fn from(e: parquet::errors::ParquetError) -> Self {
        Self::Parquet(e.to_string())
    }
}

impl From<arrow_schema::ArrowError> for WriteError {
    fn from(e: arrow_schema::ArrowError) -> Self {
        Self::Parquet(e.to_string())
    }
}

/// One file written: where, and how many rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    /// The file's path: the folder joined with the table's file name.
    pub path: PathBuf,
    /// Its rows.
    pub rows: usize,
}

/// The three files written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Written {
    /// `trips.parquet`: one row per trip.
    pub trips: Table,
    /// `links.parquet`: one row per (link, bin of entry) that someone entered.
    pub links: Table,
    /// `iterations.parquet`: one row per iteration.
    pub iterations: Table,
}

/// Write the three tables of `run` as `trips.parquet`, `links.parquet` and
/// `iterations.parquet` in `folder`, created if missing; a file already there is replaced.
///
/// # Errors
///
/// [`WriteError`]: a demand or a network that is not the run's; the folder or a file that
/// cannot be created or written; the writer's refusal.
pub fn write(
    run: &Run,
    network: &Network,
    demand: &Demand,
    folder: &Path,
) -> Result<Written, WriteError> {
    if run.trips.class.len() != demand.trip_count() as usize {
        return Err(WriteError::NotTheRunsDemand {
            run_trips: run.trips.class.len(),
            demand_trips: demand.trip_count(),
        });
    }
    let network_links = network.link_ids().count();
    if let Some(link) = run.links.links().iter().map(|l| l.raw()).find(|&l| l >= network_links) {
        return Err(WriteError::NotTheRunsNetwork { link, network_links });
    }
    std::fs::create_dir_all(folder)?;
    Ok(Written {
        trips: file(folder, "trips.parquet", &trips::batch(&run.trips, demand)?)?,
        links: file(folder, "links.parquet", &links::batch(&run.links, network)?)?,
        iterations: file(folder, "iterations.parquet", &iterations::batch(&run.iterations)?)?,
    })
}

/// Write `batch` to `folder/name` as one Parquet file at the writer's default settings.
fn file(folder: &Path, name: &str, batch: &RecordBatch) -> Result<Table, WriteError> {
    let path = folder.join(name);
    let mut writer = ArrowWriter::try_new(File::create(&path)?, batch.schema(), None)?;
    writer.write(batch)?;
    writer.close()?;
    Ok(Table { path, rows: batch.num_rows() })
}

/// A value of the loop's that is `NaN` when there is none: null then.
fn value(x: f64) -> Option<f64> {
    (!x.is_nan()).then_some(x)
}

/// A column of `Arc`'d array, the form a `RecordBatch` takes.
fn column(array: impl arrow_array::Array + 'static) -> arrow_array::ArrayRef {
    Arc::new(array)
}
