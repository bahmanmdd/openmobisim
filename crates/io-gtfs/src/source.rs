//! Where a feed's files come from: a `.zip`, or a folder of `.txt` files.

use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::{Path, PathBuf};

use zip::ZipArchive;

use crate::GtfsError;

/// A GTFS feed on disk.
pub struct FeedSource {
    inner: Inner,
}

enum Inner {
    Zip(Box<ZipArchive<BufReader<File>>>),
    Folder(PathBuf),
}

impl FeedSource {
    /// Open the feed at `path`: a folder of GTFS files, or a zip of them (the
    /// files at its root, or in one folder inside it).
    ///
    /// # Errors
    ///
    /// [`GtfsError::Io`] if the path cannot be read, [`GtfsError::Zip`] if it
    /// is a file but not a zip.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, GtfsError> {
        let path = path.as_ref();
        if path.is_dir() {
            return Ok(Self { inner: Inner::Folder(path.to_path_buf()) });
        }
        let file =
            File::open(path).map_err(|e| GtfsError::Io(format!("{}: {e}", path.display())))?;
        let archive = ZipArchive::new(BufReader::new(file))
            .map_err(|e| GtfsError::Zip(format!("{}: {e}", path.display())))?;
        Ok(Self { inner: Inner::Zip(Box::new(archive)) })
    }

    /// The file `name` (for example `stops.txt`), if the feed has it.
    ///
    /// # Errors
    ///
    /// [`GtfsError::Io`] or [`GtfsError::Zip`] if it is there but cannot be read.
    pub fn file(&mut self, name: &str) -> Result<Option<Box<dyn Read + '_>>, GtfsError> {
        match &mut self.inner {
            Inner::Folder(dir) => {
                let path = dir.join(name);
                if !path.is_file() {
                    return Ok(None);
                }
                let file = File::open(&path)
                    .map_err(|e| GtfsError::Io(format!("{}: {e}", path.display())))?;
                Ok(Some(Box::new(file)))
            }
            Inner::Zip(archive) => {
                // At the root, or else in the first folder (by name) that has it.
                let suffix = format!("/{name}");
                let mut entry: Option<String> = None;
                for candidate in archive.file_names() {
                    if candidate == name {
                        entry = Some(candidate.to_string());
                        break;
                    }
                    if candidate.ends_with(&suffix)
                        && candidate.matches('/').count() == 1
                        && entry.as_deref().is_none_or(|e| candidate < e)
                    {
                        entry = Some(candidate.to_string());
                    }
                }
                let Some(entry) = entry else { return Ok(None) };
                let file =
                    archive.by_name(&entry).map_err(|e| GtfsError::Zip(format!("{entry}: {e}")))?;
                Ok(Some(Box::new(file)))
            }
        }
    }
}

impl From<io::Error> for GtfsError {
    fn from(e: io::Error) -> Self {
        GtfsError::Io(e.to_string())
    }
}
