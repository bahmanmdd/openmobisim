//! Output writers for openmobisim: `kpis.parquet`, `diagnostics.parquet`, `link_bins.parquet`,
//! `transit_calls.parquet`, `parking_bins.parquet`, `trip_results.parquet`, `events.parquet` and
//! `manifest.json`.
//!
//! | Module | What it writes |
//! |---|---|
//! | [`kpis`] | Long format: `run_id, design_id, replication, iteration, metric, value` |
//! | [`diagnostics`] | Aggregated counters, from [`openmobisim_core_types::diagnostics::Diagnostics`] |
//! | [`events`] | Sampled per-trip event rows, from [`openmobisim_core_sim::RunResult::events`] |
//! | [`link_bins`] | Per-link, per-time-bin results, from [`openmobisim_core_sim::RunResult::link_bins`] |
//! | [`transit_calls`] | Every call of the timetable with its times and passengers, from [`openmobisim_core_sim::RunResult::transit`] |
//! | [`parking_bins`] | Every parking's occupancy bin by bin, from [`openmobisim_core_sim::RunResult::parking`] |
//! | [`trip_results`] | Every trip and what became of it, from [`openmobisim_core_sim::TripTable`] |
//! | [`manifest`] | [`manifest::Manifest`] — the file that makes a run reproducible |
//!
//! # What is not written yet
//!
//! This crate writes every field
//! the run has a true value for and no others — a placeholder value would
//! violate "never present an unvalidated result" as much as a fabricated KPI
//! would. `kpis.parquet` carries the run's trip metrics (by mode), the
//! convergence measures of every iteration, transit and parking; not written:
//!
//! - **`kpis.parquet`**: no per-design comparison (`DesignSpace` / `run_batch`
//!   do not exist yet): `design_id` and `replication` are always 0.
//! - **`diagnostics.parquet`**: no `detail` column — [`Diagnostics`](openmobisim_core_types::diagnostics::Diagnostics)
//!   does not carry free-text detail.
//! - **`events.parquet`**: no `payload` column; boardings, parking and
//!   reroutes have files of their own (`transit_calls.parquet`,
//!   `parking_bins.parquet`, `Run.route_changes()`).
//! - **`manifest.json`**: no `design_vector`, `replication` count, per-artifact
//!   fingerprints beyond the network's, warm-start source, `adaptation` or
//!   `max_parallel_runs` — each names a mechanism (the artifact cache,
//!   `run_batch`) that does not exist yet. Add the field when the mechanism
//!   lands, not before.

use core::fmt;

pub mod diagnostics;
pub mod events;
mod io;
pub mod kpis;
pub mod link_bins;
pub mod manifest;
pub mod parking_bins;
pub mod transit_calls;
pub mod trip_results;

pub use diagnostics::write_diagnostics;
pub use events::write_events;
pub use kpis::{write_kpis, write_kpis_csv};
pub use link_bins::{write_layer_link_bins, write_link_bins};
pub use manifest::{Manifest, write_manifest};
pub use parking_bins::write_parking_bins;
pub use transit_calls::write_transit_calls;
pub use trip_results::write_trip_results;

/// Something that went wrong writing an output artifact.
///
/// Hand-written rather than derived, matching `core-demand`'s `DemandError`
/// — this crate's dependency list is already `parquet` and `arrow-array`; it
/// does not need `thiserror` too.
#[derive(Debug)]
pub enum WriteError {
    /// The file could not be created or written.
    Io(std::io::Error),
    /// The Arrow/Parquet writer rejected the data.
    Parquet(String),
}

impl fmt::Display for WriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WriteError::Io(e) => write!(f, "could not write the output artifact: {e}"),
            WriteError::Parquet(m) => write!(f, "could not encode the output artifact: {m}"),
        }
    }
}

impl std::error::Error for WriteError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            WriteError::Io(e) => Some(e),
            WriteError::Parquet(_) => None,
        }
    }
}

impl From<std::io::Error> for WriteError {
    fn from(e: std::io::Error) -> Self {
        WriteError::Io(e)
    }
}

impl From<parquet::errors::ParquetError> for WriteError {
    fn from(e: parquet::errors::ParquetError) -> Self {
        WriteError::Parquet(e.to_string())
    }
}

impl From<arrow_schema::ArrowError> for WriteError {
    fn from(e: arrow_schema::ArrowError) -> Self {
        WriteError::Parquet(e.to_string())
    }
}

/// The version of the output files' layout: written to every Parquet file as the
/// key-value `openmobisim.schema_version` ([`SCHEMA_VERSION_KEY`]) and to `manifest.json` as
/// `manifest_version`. A reader checks it before trusting a column; it is raised when a column
/// is renamed, removed or changes meaning (the version 1 files are the first versioned ones).
pub const SCHEMA_VERSION: u32 = 1;

/// The Parquet key-value metadata key under which every file carries [`SCHEMA_VERSION`].
pub const SCHEMA_VERSION_KEY: &str = "openmobisim.schema_version";

/// The target triple this crate was built for — the manifest's `platform`
/// field. Recorded at build time (`build.rs`), the same pattern
/// `py-bindings` uses: the determinism guarantee is same-platform, so the
/// platform a run's artifacts came from is part of what makes them
/// comparable to another run's.
pub const TARGET: &str = env!("OPENMOBISIM_TARGET");
