//! A feed on disk: one `.zip` holding the GTFS files, at its root or in one folder inside it.

use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::Path;

use zip::ZipArchive;

use crate::GtfsError;
use crate::csv::Csv;

/// A GTFS feed: the zip it comes in.
pub(crate) struct Feed {
    archive: ZipArchive<BufReader<File>>,
}

/// One file of the feed, read as CSV.
pub(crate) type FeedCsv<'a> = Csv<Box<dyn Read + 'a>>;

impl Feed {
    /// Open the zip at `path`.
    pub fn open(path: &Path) -> Result<Self, GtfsError> {
        let file =
            File::open(path).map_err(|e| GtfsError::Io(format!("{}: {e}", path.display())))?;
        let archive = ZipArchive::new(BufReader::new(file))
            .map_err(|e| GtfsError::Zip(format!("{}: {e}", path.display())))?;
        Ok(Self { archive })
    }

    /// The file `name` (for example `stops.txt`) as CSV, its header read, if the feed has it:
    /// at the zip's root, or else in the first folder (by name) one level down that has it.
    pub fn optional_csv(&mut self, name: &str) -> Result<Option<FeedCsv<'_>>, GtfsError> {
        let suffix = format!("/{name}");
        let mut entry: Option<String> = None;
        for candidate in self.archive.file_names() {
            if candidate == name {
                entry = Some(candidate.to_owned());
                break;
            }
            if candidate.ends_with(&suffix)
                && candidate.matches('/').count() == 1
                && entry.as_deref().is_none_or(|e| candidate < e)
            {
                entry = Some(candidate.to_owned());
            }
        }
        let Some(entry) = entry else { return Ok(None) };
        let file =
            self.archive.by_name(&entry).map_err(|e| GtfsError::Zip(format!("{entry}: {e}")))?;
        let input: Box<dyn Read + '_> = Box::new(file);
        Ok(Some(Csv::new(input)?))
    }

    /// The file `name` as CSV, which every feed has.
    pub fn csv(&mut self, name: &'static str) -> Result<FeedCsv<'_>, GtfsError> {
        self.optional_csv(name)?.ok_or(GtfsError::MissingFile(name))
    }
}

impl From<io::Error> for GtfsError {
    fn from(e: io::Error) -> Self {
        GtfsError::Io(e.to_string())
    }
}
