//! Where OSM elements come from — and why that is an abstraction rather than a
//! file reader.
//!
//! The interesting part of importing OpenStreetMap is not decoding protobuf.
//! It is deciding what a `highway=residential` way with `oneway=-1` and
//! `lanes=3` *means*, splitting it at the right nodes, and getting the geometry
//! length right. That logic is worth testing directly, and testing it through a
//! `.osm.pbf` file would mean either shipping binary fixtures or writing a PBF
//! encoder for the test suite — both of which make the tests harder to read
//! than the code they check.
//!
//! So the importer consumes an [`OsmSource`]. [`MemorySource`] is the one the
//! tests use; `PbfSource` is a thin adapter over the file format. A second
//! input format — an `.osm` XML file, a database — would implement the same
//! trait and reuse everything else.

use std::collections::HashMap;
use std::fmt;

/// An OSM node: a point, and possibly some tags.
#[derive(Clone, Debug, PartialEq)]
pub struct OsmNode {
    /// The OSM node id. Negative ids appear in hand-edited extracts and are
    /// allowed — they are only ever used as an opaque key.
    pub id: i64,
    /// Longitude in WGS84 degrees.
    pub lon: f64,
    /// Latitude in WGS84 degrees.
    pub lat: f64,
    /// Tags. Only a handful of nodes carry any; most are pure geometry.
    pub tags: Vec<(String, String)>,
}

impl OsmNode {
    /// A plain geometry node.
    #[must_use]
    pub fn new(id: i64, lon: f64, lat: f64) -> Self {
        Self { id, lon, lat, tags: Vec::new() }
    }

    /// The same node with tags attached.
    #[must_use]
    pub fn with_tags<K: Into<String>, V: Into<String>>(
        mut self,
        tags: impl IntoIterator<Item = (K, V)>,
    ) -> Self {
        self.tags = tags.into_iter().map(|(k, v)| (k.into(), v.into())).collect();
        self
    }

    /// Look up a tag.
    #[must_use]
    pub fn tag(&self, key: &str) -> Option<&str> {
        tag_of(&self.tags, key)
    }
}

/// An OSM way: an ordered list of node ids, and tags.
#[derive(Clone, Debug, PartialEq)]
pub struct OsmWay {
    /// The OSM way id.
    pub id: i64,
    /// The nodes the way passes through, in order.
    pub node_ids: Vec<i64>,
    /// Tags.
    pub tags: Vec<(String, String)>,
}

impl OsmWay {
    /// A way through `node_ids` with the given tags.
    #[must_use]
    pub fn new<K: Into<String>, V: Into<String>>(
        id: i64,
        node_ids: impl IntoIterator<Item = i64>,
        tags: impl IntoIterator<Item = (K, V)>,
    ) -> Self {
        Self {
            id,
            node_ids: node_ids.into_iter().collect(),
            tags: tags.into_iter().map(|(k, v)| (k.into(), v.into())).collect(),
        }
    }

    /// Look up a tag.
    #[must_use]
    pub fn tag(&self, key: &str) -> Option<&str> {
        tag_of(&self.tags, key)
    }

    /// Whether the way's first and last node are the same — a closed way.
    ///
    /// Closed ways are roundabouts, building outlines and squares; which of
    /// those it is depends on other tags, and the importer decides.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.node_ids.len() >= 2 && self.node_ids.first() == self.node_ids.last()
    }
}

/// Look a tag up in a list.
///
/// A linear scan. Ways carry a handful of tags, so a `HashMap` per way would
/// cost more in allocation than the scan costs in comparisons.
#[must_use]
pub fn tag_of<'a>(tags: &'a [(String, String)], key: &str) -> Option<&'a str> {
    tags.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

/// Why reading a source failed.
///
/// These are failures of the *input*, not of the data in it: a missing file, a
/// corrupt block. Anything that is merely odd about the OSM data — a way with
/// one node, an unparseable speed limit — is a diagnostic, not an error,
/// because the simulation never stops (brief §3c).
#[derive(Debug)]
pub enum OsmError {
    /// The file could not be opened or read.
    Io(std::io::Error),
    /// The file was opened but could not be decoded.
    Format(String),
}

impl fmt::Display for OsmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OsmError::Io(e) => write!(f, "could not read the OSM input: {e}"),
            OsmError::Format(m) => write!(f, "could not decode the OSM input: {m}"),
        }
    }
}

impl std::error::Error for OsmError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            OsmError::Io(e) => Some(e),
            OsmError::Format(_) => None,
        }
    }
}

impl From<std::io::Error> for OsmError {
    fn from(e: std::io::Error) -> Self {
        OsmError::Io(e)
    }
}

/// Something the importer can read OSM elements from.
///
/// # Two passes, on purpose
///
/// The importer reads ways first — to decide which nodes matter — and nodes
/// second. Doing it the other way round means holding every node in the extract
/// in memory, and an extract of a large city is tens of millions of them.
/// Reading ways first lets the node pass keep only the ones some road actually
/// uses, which is typically a fifth of the file.
///
/// A source must therefore be readable twice. For a file that is a second pass
/// over the same bytes; for [`MemorySource`] it is free.
pub trait OsmSource {
    /// Visit every way.
    ///
    /// # Errors
    ///
    /// [`OsmError`] if the source cannot be read.
    fn for_each_way(&self, visit: &mut dyn FnMut(OsmWay)) -> Result<(), OsmError>;

    /// Visit every node.
    ///
    /// # Errors
    ///
    /// [`OsmError`] if the source cannot be read.
    fn for_each_node(&self, visit: &mut dyn FnMut(OsmNode)) -> Result<(), OsmError>;
}

/// An in-memory source, for tests and for callers that already hold elements.
///
/// # Examples
///
/// ```
/// use openmobisim_io_osm::source::{MemorySource, OsmNode, OsmSource, OsmWay};
///
/// let source = MemorySource::new()
///     .node(OsmNode::new(1, 4.80, 45.70))
///     .node(OsmNode::new(2, 4.81, 45.70))
///     .way(OsmWay::new(100, [1, 2], [("highway", "residential")]));
///
/// let mut ways = 0;
/// source.for_each_way(&mut |_| ways += 1).unwrap();
/// assert_eq!(ways, 1);
/// ```
#[derive(Clone, Debug, Default)]
pub struct MemorySource {
    nodes: Vec<OsmNode>,
    ways: Vec<OsmWay>,
}

impl MemorySource {
    /// An empty source.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a node.
    #[must_use]
    pub fn node(mut self, node: OsmNode) -> Self {
        self.nodes.push(node);
        self
    }

    /// Add a way.
    #[must_use]
    pub fn way(mut self, way: OsmWay) -> Self {
        self.ways.push(way);
        self
    }

    /// Add many nodes.
    #[must_use]
    pub fn nodes(mut self, nodes: impl IntoIterator<Item = OsmNode>) -> Self {
        self.nodes.extend(nodes);
        self
    }

    /// Add many ways.
    #[must_use]
    pub fn ways(mut self, ways: impl IntoIterator<Item = OsmWay>) -> Self {
        self.ways.extend(ways);
        self
    }

    /// How many nodes.
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// How many ways.
    #[must_use]
    pub fn way_count(&self) -> usize {
        self.ways.len()
    }

    /// The nodes, keyed by id — a convenience for tests that check geometry.
    #[must_use]
    pub fn node_map(&self) -> HashMap<i64, &OsmNode> {
        self.nodes.iter().map(|n| (n.id, n)).collect()
    }
}

impl OsmSource for MemorySource {
    fn for_each_way(&self, visit: &mut dyn FnMut(OsmWay)) -> Result<(), OsmError> {
        for w in &self.ways {
            visit(w.clone());
        }
        Ok(())
    }

    fn for_each_node(&self, visit: &mut dyn FnMut(OsmNode)) -> Result<(), OsmError> {
        for n in &self.nodes {
            visit(n.clone());
        }
        Ok(())
    }
}
