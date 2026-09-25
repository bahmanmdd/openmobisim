//! Demand for openmobisim: the `trips.parquet`/`persons.parquet` readers (S97),
//! travellers as structure-of-arrays with integer weights (S86, S94), and
//! vehicle-location state (S98).
//!
//! | Module | What it owns |
//! |---|---|
//! | [`trips`] | Reading `trips.parquet` into unsorted, unvalidated rows |
//! | [`mode`] | A trip's mode, the optional `mode` column (S195) |
//! | [`persons`] | Reading the optional `persons.parquet` |
//! | [`travellers`] | [`travellers::Travellers`] and [`travellers::Trips`]: the dense, sorted, chain-validated structures everything downstream reads |
//! | [`vehicles`] | [`vehicles::VehicleLocations`] — per-run mutable state (S98), seeded from S129's rule |
//!
//! # What this crate is not, yet
//!
//! This is Phase 1 step 4: the reader, the traveller structure and the
//! vehicle-location *storage*. It does not choose which alternative a
//! traveller takes — that is the choice layer (Phase 2 item 7), which needs
//! route sets (Phase 2 item 6) and the mode-sequence automaton (design §22)
//! that do not exist yet. `Ownership` and [`vehicles::VehicleLocations`]
//! exist so those layers have somewhere to read from and write to; nothing
//! here decides who drives.
//!
//! Per-traveller **class ownership defaults** ([`travellers::ClassDefaults`])
//! are a build parameter, not something this crate invents: the `[user_classes]`
//! table that would supply them is scenario schema, which brief §6a puts in
//! the user's hands, not the assistant's. An empty [`travellers::ClassDefaults`]
//! — every class defaults to owning nothing — is a safe, fully-overridable
//! placeholder until that table exists.
//!
//! Per-trip attribute columns beyond the ones S97 names ("carried as
//! attributes for choice models") are deferred for the same reason: nothing
//! reads them until the choice layer exists to.

pub mod mode;
pub mod persons;
pub mod travellers;
pub mod trips;
pub mod vehicles;

pub(crate) mod columns;

pub use mode::Mode;
pub use persons::{RawPerson, read_persons_parquet};
pub use travellers::{ClassDefaults, Ownership, Travellers, Trips, build as build_travellers};
pub use trips::{RawTrip, read_trips_parquet};
pub use vehicles::{VehicleKind, VehicleLocations};

use core::fmt;

/// Something that went wrong reading or validating demand input.
///
/// Hand-written rather than derived, matching `io-osm`'s `OsmError` — this
/// crate's dependency list is already `parquet` and `arrow-array`; it does
/// not need `thiserror` too.
#[derive(Debug)]
pub enum DemandError {
    /// The file could not be opened or read.
    Io(std::io::Error),
    /// The file was opened but its Parquet/Arrow structure could not be read.
    Format(String),
    /// A required column was missing, or of a type this reader does not
    /// handle (§2.4's schema tables name the accepted shapes).
    Schema(String),
}

impl fmt::Display for DemandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DemandError::Io(e) => write!(f, "could not read the demand input: {e}"),
            DemandError::Format(m) => write!(f, "could not decode the demand input: {m}"),
            DemandError::Schema(m) => write!(f, "demand input schema problem: {m}"),
        }
    }
}

impl std::error::Error for DemandError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DemandError::Io(e) => Some(e),
            DemandError::Format(_) | DemandError::Schema(_) => None,
        }
    }
}

impl From<std::io::Error> for DemandError {
    fn from(e: std::io::Error) -> Self {
        DemandError::Io(e)
    }
}

impl From<parquet::errors::ParquetError> for DemandError {
    fn from(e: parquet::errors::ParquetError) -> Self {
        DemandError::Format(e.to_string())
    }
}

impl From<arrow_schema::ArrowError> for DemandError {
    fn from(e: arrow_schema::ArrowError) -> Self {
        DemandError::Format(e.to_string())
    }
}
