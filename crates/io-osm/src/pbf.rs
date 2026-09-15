//! Reading `.osm.pbf` files.
//!
//! A thin adapter, on purpose: everything interesting about importing
//! OpenStreetMap lives in [`crate::tags`] and [`mod@crate::import`], which know
//! nothing about file formats and are tested without one.
//!
//! # Why this dependency is acceptable
//!
//! `osmpbf` is selected with `default-features = false, features = ["rust-zlib"]`,
//! which routes decompression through `flate2`'s pure-Rust backend. The
//! resulting dependency tree contains **no C code and no `-sys` crate**, so the
//! wheels still build on all three operating systems with no compiler and no
//! system libraries — which is the promise that ruled out PROJ and the mature
//! map matchers. Check this before upgrading: a feature flag drifting back to
//! `zlib-ng` would cost the install story without failing any test.
//!
//! # Two passes over the file
//!
//! [`OsmSource`] is read twice — ways first to learn which nodes matter, nodes
//! second to keep only those. `osmpbf`'s reader is consumed by a pass, so each
//! call reopens the file. Reading the bytes twice is far cheaper than holding
//! every node of a city extract in memory, most of which belong to buildings.

use std::path::{Path, PathBuf};

use osmpbf::{Element, ElementReader};

use crate::source::{OsmError, OsmNode, OsmSource, OsmWay};

/// An `.osm.pbf` file on disk.
///
/// # Examples
///
/// ```no_run
/// use openmobisim_core_types::diagnostics::Diagnostics;
/// use openmobisim_io_osm::import::{import, ImportOptions};
/// use openmobisim_io_osm::pbf::PbfSource;
///
/// let source = PbfSource::new("lyon.osm.pbf");
/// let mut diagnostics = Diagnostics::new();
/// let (network, report, _geometry) = import(&source, ImportOptions::default(), &mut diagnostics)?;
///
/// println!(
///     "{} links from {} ways; contraction removed {:.0}%",
///     network.link_count(),
///     report.ways_kept,
///     report.contraction_ratio() * 100.0
/// );
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Clone, Debug)]
pub struct PbfSource {
    path: PathBuf,
}

impl PbfSource {
    /// A source reading the file at `path`.
    ///
    /// The file is not opened until the import reads it, so constructing this
    /// cannot fail.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The path being read.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn reader(&self) -> Result<ElementReader<std::io::BufReader<std::fs::File>>, OsmError> {
        ElementReader::from_path(&self.path).map_err(|e| to_osm_error(&e))
    }
}

/// Collect an element's tags into the owned form the importer works with.
///
/// Allocating per element is deliberate. A city extract has a few hundred
/// thousand ways with a handful of tags each; the allocations cost well under a
/// second, and borrowing from the reader would push a lifetime through every
/// signature in this crate to save it.
fn owned_tags<'a>(tags: impl Iterator<Item = (&'a str, &'a str)>) -> Vec<(String, String)> {
    tags.map(|(k, v)| (k.to_owned(), v.to_owned())).collect()
}

fn to_osm_error(e: &osmpbf::Error) -> OsmError {
    OsmError::Format(e.to_string())
}

impl OsmSource for PbfSource {
    fn for_each_way(&self, visit: &mut dyn FnMut(OsmWay)) -> Result<(), OsmError> {
        self.reader()?
            .for_each(|element| {
                if let Element::Way(way) = element {
                    visit(OsmWay {
                        id: way.id(),
                        node_ids: way.refs().collect(),
                        tags: owned_tags(way.tags()),
                    });
                }
            })
            .map_err(|e| to_osm_error(&e))
    }

    fn for_each_node(&self, visit: &mut dyn FnMut(OsmNode)) -> Result<(), OsmError> {
        self.reader()?
            .for_each(|element| match element {
                // Plain nodes: rare in practice, but present in hand-made and
                // older extracts.
                Element::Node(node) => visit(OsmNode {
                    id: node.id(),
                    lon: node.lon(),
                    lat: node.lat(),
                    tags: owned_tags(node.tags()),
                }),
                // Dense nodes: how essentially every real extract stores its
                // geometry. Missing this arm would import an empty network from
                // a perfectly good file — silently, because an empty network
                // fails later and somewhere else. It is the single most
                // dangerous line in this crate, and the one the test suite
                // cannot currently reach; see `tests/pbf.rs`.
                Element::DenseNode(node) => visit(OsmNode {
                    id: node.id(),
                    lon: node.lon(),
                    lat: node.lat(),
                    tags: owned_tags(node.tags()),
                }),
                Element::Way(_) | Element::Relation(_) => {}
            })
            .map_err(|e| to_osm_error(&e))
    }
}
