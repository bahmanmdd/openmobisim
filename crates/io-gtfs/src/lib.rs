//! GTFS import: one service day of a feed, clipped to a study area, as a
//! [`Timetable`](openmobisim_core_transit::Timetable) (S199).
//!
//! | Module | What it owns |
//! |---|---|
//! | [`source`] | A feed on disk: a zip, or a folder of `.txt` files |
//! | [`mod@read`] | Stops, the service day, runs, transfers: see its docs for every rule |
//! | `csv` | A streaming RFC 4180 reader over `csv-core` |
//!
//! Nothing here fails because of odd data. A row with a bad number or an
//! unknown stop is skipped and counted in the [`GtfsReport`]; only a missing
//! file or column, or a day with no service, is an error.

mod csv;
pub mod read;
pub mod source;

pub use read::{GtfsReport, read_feed, read_gtfs};
pub use source::FeedSource;

/// Why a feed could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GtfsError {
    /// Reading from disk failed.
    Io(String),
    /// The file is not a readable zip.
    Zip(String),
    /// A file every feed must have is missing.
    MissingFile(&'static str),
    /// A column a file must have is missing.
    MissingColumn {
        /// The file.
        file: &'static str,
        /// The column.
        column: &'static str,
    },
    /// Nothing runs on the day asked for, or on any weekday.
    NoService(String),
}

impl core::fmt::Display for GtfsError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            GtfsError::Io(e) => write!(f, "reading the feed: {e}"),
            GtfsError::Zip(e) => write!(f, "reading the feed's zip: {e}"),
            GtfsError::MissingFile(name) => write!(f, "the feed has no {name}"),
            GtfsError::MissingColumn { file, column } => {
                write!(f, "{file} has no {column} column")
            }
            GtfsError::NoService(what) => write!(f, "{what}"),
        }
    }
}

impl std::error::Error for GtfsError {}
