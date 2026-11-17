//! The reader's input: the nodes and ways of one extract, as plain lists, and why reading fails.
//!
//! The rules live in [`crate::class`], [`crate::access`], [`crate::car`] and [`crate::import`],
//! which read an [`Osm`] and know no file format. The `.osm.pbf` decoder ([`crate::pbf`]) fills
//! the same type, so a test states its junction or its tags as a short list instead of a binary
//! fixture.

use core::fmt;
use std::path::PathBuf;

use openmobisim_graph::network::BuildError;

/// An OpenStreetMap node: a point in WGS84 degrees, and its tags (most nodes have none).
#[derive(Clone, Debug, PartialEq)]
pub struct OsmNode {
    /// The node id (negative ids, as in hand-edited files, are kept as they are).
    pub id: i64,
    /// Longitude, WGS84 degrees.
    pub lon: f64,
    /// Latitude, WGS84 degrees.
    pub lat: f64,
    /// Tags, key and value.
    pub tags: Vec<(String, String)>,
}

impl OsmNode {
    /// A node without tags.
    #[must_use]
    pub fn new(id: i64, lon: f64, lat: f64) -> Self {
        Self { id, lon, lat, tags: Vec::new() }
    }

    /// The same node with these tags.
    #[must_use]
    pub fn with_tags(mut self, tags: &[(&str, &str)]) -> Self {
        self.tags = owned(tags);
        self
    }
}

/// An OpenStreetMap way: the nodes it passes, in order, and its tags.
#[derive(Clone, Debug, PartialEq)]
pub struct OsmWay {
    /// The way id.
    pub id: i64,
    /// The node ids, in the way's order (its "forward" direction).
    pub nodes: Vec<i64>,
    /// Tags, key and value.
    pub tags: Vec<(String, String)>,
}

impl OsmWay {
    /// A way through `nodes` with these tags.
    #[must_use]
    pub fn new(id: i64, nodes: &[i64], tags: &[(&str, &str)]) -> Self {
        Self { id, nodes: nodes.to_vec(), tags: owned(tags) }
    }

    /// The value of tag `key`, if the way has it.
    #[must_use]
    pub fn tag(&self, key: &str) -> Option<&str> {
        tag_of(&self.tags, key)
    }
}

/// The nodes and ways of one extract.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Osm {
    /// The nodes, in any order.
    pub nodes: Vec<OsmNode>,
    /// The ways, in any order.
    pub ways: Vec<OsmWay>,
}

/// The value of tag `key` in a tag list, if present.
#[must_use]
pub fn tag_of<'a>(tags: &'a [(String, String)], key: &str) -> Option<&'a str> {
    tags.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

fn owned(tags: &[(&str, &str)]) -> Vec<(String, String)> {
    tags.iter().map(|&(k, v)| (k.to_owned(), v.to_owned())).collect()
}

/// Why an extract could not be read into a network: nothing to read, or nothing to build. Each
/// names what it refuses; everything else is repaired and noted in the `Report`.
#[derive(Debug)]
pub enum OsmError {
    /// The file could not be opened or read.
    Io {
        /// The file.
        path: PathBuf,
        /// What the system said.
        source: std::io::Error,
    },
    /// The file was read but is not a valid `.osm.pbf`.
    Decode {
        /// The file.
        path: PathBuf,
        /// What the decoder said.
        message: String,
    },
    /// No way of the extract is a street of any mode.
    NoStreets,
    /// The network builder refused a row (see [`BuildError`]).
    Build(BuildError),
}

impl fmt::Display for OsmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "cannot read {}: {source}", path.display()),
            Self::Decode { path, message } => {
                write!(f, "{} is not a valid .osm.pbf file: {message}", path.display())
            }
            Self::NoStreets => f.write_str("no way of the extract is a street of any mode"),
            Self::Build(e) => write!(f, "the network builder refused the extract: {e}"),
        }
    }
}

impl core::error::Error for OsmError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Build(e) => Some(e),
            _ => None,
        }
    }
}

impl From<BuildError> for OsmError {
    fn from(e: BuildError) -> Self {
        Self::Build(e)
    }
}
