//! Output writers for openmobisim: `kpis.parquet`, `diagnostics.parquet`,
//! `events.parquet`, `manifest.json` (Foundations §6). Phase 1 step 7.
//!
//! | Module | What it writes |
//! |---|---|
//! | [`kpis`] | Long format: `run_id, design_id, replication, iteration, metric, value` |
//! | [`diagnostics`] | Aggregated counters, from [`openmobisim_core_types::diagnostics::Diagnostics`] |
//! | [`events`] | Sampled per-trip event rows, from [`openmobisim_core_sim::RunResult::events`] |
//! | [`link_bins`] | Per-link, per-time-bin results (S168), from [`openmobisim_core_sim::RunResult::link_bins`] |
//! | [`manifest`] | [`manifest::Manifest`] — the file that makes a run reproducible |
//!
//! # What Foundations §6 asks for that this does not write yet
//!
//! Foundations §6 describes the *eventual* schema, built out as the
//! mechanisms it reports on land. This crate writes every field Phase 1
//! actually has a true value for and no others — a placeholder value would
//! violate "never present an unvalidated result" as much as a fabricated
//! KPI would. Specifically not written, and why:
//!
//! - **`kpis.parquet`**: only `total_travel_time` and S57's completion
//!   counts are metrics yet — no per-iteration convergence report (no
//!   equilibration exists), no per-design comparison (no `DesignSpace` /
//!   `run_batch` exists).
//! - **`diagnostics.parquet`**: no `detail` column — [`Diagnostics`](openmobisim_core_types::diagnostics::Diagnostics)
//!   does not carry free-text detail yet (noted as a gap in S131's log).
//! - **`events.parquet`**: no `payload` column — Phase 1 has no boardings,
//!   hub or store events, or disruptions to put one on (S135's `EventRow`
//!   already documents this).
//! - **`manifest.json`**: no `design_vector`, `replication` count,
//!   `scenario_hash` (S168's `run_fingerprint` is its Phase 1 form), per-artifact
//!   fingerprints beyond the network's, live stochastic streams, warm-start
//!   source, `adaptation` or `max_parallel_runs` — every one of these names a
//!   mechanism (the artifact cache, disruption scheduling, `run_batch`) that
//!   does not exist in the pipeline yet. Add the field when the mechanism
//!   lands, not before. (`master_seed` is written since S168: the seed and
//!   `RngKey` exist, though no stochastic step draws from them yet.)

use core::fmt;

pub mod diagnostics;
pub mod events;
mod io;
pub mod kpis;
pub mod link_bins;
pub mod manifest;

pub use diagnostics::write_diagnostics;
pub use events::write_events;
pub use kpis::write_kpis;
pub use link_bins::{write_layer_link_bins, write_link_bins};
pub use manifest::{Manifest, write_manifest};

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

/// The target triple this crate was built for — the manifest's `platform`
/// field. Recorded at build time (`build.rs`), the same pattern
/// `py-bindings` uses: S60's determinism guarantee is same-platform, so the
/// platform a run's artifacts came from is part of what makes them
/// comparable to another run's.
pub const TARGET: &str = env!("OPENMOBISIM_TARGET");
