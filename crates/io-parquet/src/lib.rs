//! Writing the four tables of a run (trips, links by time bin, iterations, routes) as four
//! Parquet files in a folder, and the run's record, `run_record.toml`, beside them; and reading
//! them back.
//!
//! [`write`] takes the `Run` the loop returned, the `Network` and the `Demand` it ran on (for the
//! external ids), what else the run was given ([`Given`]: the values its network, transit and
//! demand were built with, the loop's parameters, the readers' counts), and a folder; it writes
//! `trips.parquet`, `links.parquet`, `iterations.parquet`, `routes.parquet` and `run_record.toml` there
//! and returns their paths and the tables' row counts ([`Written`]). [`read`] and
//! [`read_iterations`] read back what the run's pages need of such a folder, refusing only a file
//! that cannot be read and warning on the rest (`src/read.rs`). The record's layout is in
//! `src/record.rs`. A run never writes `run.toml`, the settings a user writes for the binding's
//! `run_file`: that file and the record are two files with fixed names.
//! One row per
//! entity, external ids as strings, every column named with its unit; null, never `NaN`, where
//! the loop's `NaN` means "no value". Every time of day is a second of the clock (seconds after
//! midnight of the run's day): the loop's seconds after the window's start plus the window's
//! start, converted here and nowhere else. The files are written with the `parquet` crate's
//! default settings: no compression, dictionary encoding, one row group per 1 048 576 rows.
//!
//! [`read_table`] reads an input table (nodes, links or trips) from a Parquet file, uncompressed
//! or snappy: its column names and each column's cells by kind (`src/input.rs`).

use core::fmt;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow_array::RecordBatch;
use openmobisim_demand::demand::Demand;
use openmobisim_graph::network::{BuildValues, Network};
use openmobisim_io_gtfs::Report as GtfsReport;
use openmobisim_io_osm::Report as OsmReport;
use openmobisim_loop::parameters::Parameters;
use openmobisim_loop::run::Run;
use openmobisim_transit::timetable::Timetable;
use openmobisim_types::ids::EntityId;
use openmobisim_types::repairs::Repairs;
use openmobisim_types::units::Speed;
use parquet::arrow::ArrowWriter;

mod case;
mod input;
mod iterations;
mod links;
mod read;
mod record;
mod routes;
mod trips;

pub use case::write_case;
pub use input::{Cells, InputTable, ReadError, read_table};
pub use read::{
    Links, OutputError, Routes, Tables, TripRows, agrees, read, read_iterations, read_links,
    read_routes, read_trips,
};
pub use record::KEYS;
pub use routes::RouteLeg;

/// Where the network came from.
#[derive(Clone, Copy, Debug)]
pub enum Source<'a> {
    /// An OpenStreetMap extract, with the reader's counts.
    Osm(&'a OsmReport),
    /// Node and link tables.
    Tables,
}

/// The run's transit: its timetable, read from a GTFS feed, the reader's counts and the
/// `transfer_walk_max` it was read with (m).
#[derive(Clone, Copy, Debug)]
pub struct Transit<'a> {
    /// The timetable the run and its demand were built on.
    pub timetable: &'a Timetable,
    /// The GTFS reader's counts.
    pub report: &'a GtfsReport,
    /// The longest footpath between stops the reader derived, metres.
    pub transfer_walk_max: f64,
}

/// The files the run's inputs were read from, each a canonical path (absolute, symbolic links
/// resolved), `None` for an input given in memory: the record writes each relative to its own
/// folder, so that the record alone can run again.
#[derive(Clone, Copy, Debug, Default)]
pub struct Files<'a> {
    /// The OpenStreetMap extract of the network.
    pub osm: Option<&'a Path>,
    /// The nodes table of the network.
    pub nodes: Option<&'a Path>,
    /// The links table of the network.
    pub links: Option<&'a Path>,
    /// The trips table of the demand.
    pub trips: Option<&'a Path>,
    /// The GTFS feed of the transit.
    pub gtfs: Option<&'a Path>,
}

/// What the run was given besides its network and its demand, for its record: every value as
/// the run used it, defaults included.
#[derive(Clone, Copy, Debug)]
pub struct Given<'a> {
    /// Where the network came from.
    pub source: Source<'a>,
    /// The files its inputs were read from.
    pub files: Files<'a>,
    /// The network's three build values: veh/h per lane, km/h, km/h.
    pub build_values: BuildValues,
    /// The `stop_walk_max` the demand was built with, metres.
    pub stop_walk_max: f64,
    /// The transit, if the run had one.
    pub transit: Option<Transit<'a>>,
    /// The loop's parameters.
    pub parameters: &'a Parameters,
    /// What the network's readers set right (the OSM reader's, or the tables' reader's and the
    /// network build's), by kind.
    pub network_repairs: &'a Repairs,
    /// How many trip rows were given, the dropped ones with them.
    pub trips_given: u64,
    /// What the demand's readers set right or dropped (the trips table's reader's and the
    /// demand build's), by kind.
    pub demand_repairs: &'a Repairs,
}

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
    /// The transit given is not the run's: its stop count is not the demand's.
    NotTheRunsTransit {
        /// Stops the demand was built with (0 without transit).
        demand_stops: u32,
        /// Stops of the transit given (0 without).
        transit_stops: u32,
    },
    /// The bike or walk speed of the build values given is not the network's: `Given` holds
    /// them again because the network keeps them in m/s, and km/h is not recovered exactly.
    NotTheNetworksSpeeds {
        /// The bike speed given, km/h.
        bike_km_h: f64,
        /// The walk speed given, km/h.
        walk_km_h: f64,
    },
    /// A journey taken rides a run, or calls at a stop, that the transit given does not have:
    /// the trip.
    NotTheRunsJourney(String),
    /// The parameters given are not the run's: a coefficient row per class, or the link bin of
    /// the run's links table, differs.
    NotTheRunsParameters(String),
    /// A value the record refuses: a float that is not finite (TOML's `nan` and `inf` are never
    /// written), an integer above TOML's largest, 2⁶³ − 1.
    NotWritable {
        /// The key, with its table (`parameters.nest_scale`).
        key: String,
        /// The value refused.
        value: String,
    },
    /// A file the write would replace is one of the run's input files (canonical paths
    /// compared): writing would destroy the input the record names.
    WouldReplaceInput(PathBuf),
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
            Self::NotTheRunsTransit { demand_stops, transit_stops } => write!(
                f,
                "the demand was built with {demand_stops} stops and the transit has \
                 {transit_stops}: not the run's transit"
            ),
            Self::NotTheNetworksSpeeds { bike_km_h, walk_km_h } => write!(
                f,
                "a bike speed of {bike_km_h} km/h and a walk speed of {walk_km_h} km/h are not \
                 the network's: not the build values it was built with"
            ),
            Self::NotTheRunsJourney(trip) => write!(
                f,
                "trip {trip} rides a run, or calls at a stop, the transit given does not have: \
                 not the run's transit"
            ),
            Self::NotTheRunsParameters(why) => write!(f, "not the run's parameters: {why}"),
            Self::NotWritable { key, value } => {
                write!(f, "{key} = {value} cannot be written in the run's record")
            }
            Self::WouldReplaceInput(path) => write!(
                f,
                "{} is one of the run's input files; writing the run there would replace it: \
                 write the run into another folder",
                path.display()
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

/// The four files written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Written {
    /// `trips.parquet`: one row per trip.
    pub trips: Table,
    /// `links.parquet`: one row per (link, bin of entry) that someone entered.
    pub links: Table,
    /// `iterations.parquet`: one row per iteration.
    pub iterations: Table,
    /// `routes.parquet`: one row per trip and leg of what each trip took.
    pub routes: Table,
    /// `run_record.toml`: the record of the run, its path.
    pub record: PathBuf,
}

/// Write the four tables of `run` as `trips.parquet`, `links.parquet`, `iterations.parquet` and
/// `routes.parquet`, and its record as `run_record.toml`, in `folder`, created if missing; a file
/// already there is replaced. Every refusal comes before anything is written: the inputs
/// checked against the run, every journey against the timetable given, the record and the three
/// small tables built. Then the earlier record is removed, the three tables written, the routes
/// table streamed from the run (only an input or output failure can stop it), and the record
/// written last.
///
/// # Errors
///
/// [`WriteError`]: a demand, a network, a transit or parameters that are not the run's; a
/// value the record cannot hold; a file to be written that is one of the run's input files; the
/// folder or a file that cannot be created or written; the writer's refusal.
pub fn write(
    run: &Run,
    network: &Network,
    demand: &Demand,
    given: &Given<'_>,
    folder: &Path,
) -> Result<Written, WriteError> {
    check(run, network, demand, given)?;
    replaces_no_input(&given.files, folder)?;
    // Everything that can be refused is checked or built first: the journeys against the
    // timetable, the record and the three batches of trips, links and iterations. The routes
    // table is streamed from the run once they are written; it refuses nothing. The record is
    // built again once the folder exists, since an input file's path is written relative to the
    // folder's canonical path; the paths themselves are never refused.
    routes::check(run, demand, given.transit.map(|t| t.timetable))?;
    record::text(run, network, demand, given, None)?;
    let start = run.links.window().origin();
    let trips = trips::batch(&run.trips, demand, start)?;
    let links = links::batch(&run.links, network)?;
    let iterations = iterations::batch(&run.iterations)?;
    std::fs::create_dir_all(folder)?;
    let canonical = std::fs::canonicalize(folder)?;
    let record = record::text(run, network, demand, given, Some(&canonical))?;
    // A record left from an earlier run goes first and the new one is written last, so a folder
    // that holds a record holds the complete run it describes.
    let record_path = folder.join("run_record.toml");
    match std::fs::remove_file(&record_path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
        _ => {}
    }
    let written = Written {
        trips: file(folder, "trips.parquet", &trips)?,
        links: file(folder, "links.parquet", &links)?,
        iterations: file(folder, "iterations.parquet", &iterations)?,
        routes: routes::file(
            &folder.join("routes.parquet"),
            run,
            demand,
            network,
            given.transit.map(|t| t.timetable),
        )?,
        record: record_path,
    };
    std::fs::write(&written.record, record)?;
    Ok(written)
}

/// Refuse a demand, a network, a transit, build values or parameters that are not the run's.
fn check(
    run: &Run,
    network: &Network,
    demand: &Demand,
    given: &Given<'_>,
) -> Result<(), WriteError> {
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
    let transit_stops = given.transit.map_or(0, |t| t.timetable.stop_count());
    if demand.stop_count() != transit_stops {
        return Err(WriteError::NotTheRunsTransit {
            demand_stops: demand.stop_count(),
            transit_stops,
        });
    }
    let (rows, classes) = (given.parameters.coefficients.len(), demand.class_count());
    if rows != classes as usize {
        return Err(WriteError::NotTheRunsParameters(format!(
            "{rows} coefficient rows for {classes} classes"
        )));
    }
    let (bin, run_bin) = (given.parameters.link_bin, run.links.window().step_seconds());
    if bin != run_bin {
        return Err(WriteError::NotTheRunsParameters(format!(
            "a link bin of {bin} s where the run's is {run_bin} s"
        )));
    }
    let (bike, walk) = (given.build_values.bike_speed, given.build_values.walk_speed);
    let same = |km_h: f64, m_s: Speed| {
        Speed::from_km_per_hour(km_h).get().to_bits() == m_s.get().to_bits()
    };
    if !same(bike, network.bike_speed()) || !same(walk, network.walk_speed()) {
        return Err(WriteError::NotTheNetworksSpeeds { bike_km_h: bike, walk_km_h: walk });
    }
    Ok(())
}

/// Refuse a write into `folder` whose `trips.parquet`, `links.parquet`, `iterations.parquet`,
/// `routes.parquet` or `run_record.toml` is one of the run's input files `files`, canonical paths compared: a file of those
/// names that does not exist yet (or a folder that does not exist yet) is no input. Paths are
/// compared, not files: a hard link of an input under one of those names is not recognised.
///
/// [`write`] calls it before anything is written, so that no caller of `write` ever replaces an
/// input; a caller that knows the files and the folder before the run (the Python binding)
/// calls it before the loop too, so that the mistake costs no run.
///
/// # Errors
///
/// [`WriteError::WouldReplaceInput`], naming the file.
pub fn replaces_no_input(files: &Files<'_>, folder: &Path) -> Result<(), WriteError> {
    let inputs = [files.osm, files.nodes, files.links, files.trips, files.gtfs];
    for name in [
        "trips.parquet",
        "links.parquet",
        "iterations.parquet",
        "routes.parquet",
        "run_record.toml",
    ] {
        let Ok(output) = std::fs::canonicalize(folder.join(name)) else { continue };
        if inputs.iter().flatten().any(|input| *input == output) {
            return Err(WriteError::WouldReplaceInput(output));
        }
    }
    Ok(())
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
