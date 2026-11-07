//! The `.osm.pbf` decoder: fills an [`Osm`] with the ways that carry a `highway` tag and the
//! nodes they name, and nothing else.
//!
//! Two passes over the file, ways first: an extract's nodes are mostly buildings and
//! coastlines, so learning which nodes the streets use before reading any keeps a city's
//! import to the nodes it needs. `osmpbf` is built with flate2's pure-Rust backend
//! (`rust-zlib`): the dependency tree holds no C.

use std::collections::HashSet;
use std::path::Path;

use osmpbf::{Element, ElementReader};

use crate::source::{Osm, OsmError, OsmNode, OsmWay};

/// Read the file at `path`: every way with a `highway` tag, and every node those ways name
/// (plain and dense nodes both), with their tags. Ways and nodes are sorted by id.
///
/// # Errors
///
/// [`OsmError::Io`] if the file cannot be opened or read, [`OsmError::Decode`] if it is not
/// a valid `.osm.pbf`.
pub fn read_pbf(path: &Path) -> Result<Osm, OsmError> {
    let mut ways = Vec::new();
    reader(path)?
        .for_each(|element| {
            if let Element::Way(way) = element {
                if way.tags().any(|(k, _)| k == "highway") {
                    ways.push(OsmWay {
                        id: way.id(),
                        nodes: way.refs().collect(),
                        tags: owned(way.tags()),
                    });
                }
            }
        })
        .map_err(|e| decode(path, &e))?;
    let wanted: HashSet<i64> = ways.iter().flat_map(|w| w.nodes.iter().copied()).collect();

    let mut nodes = Vec::with_capacity(wanted.len());
    reader(path)?
        .for_each(|element| match element {
            Element::DenseNode(n) if wanted.contains(&n.id()) => nodes.push(OsmNode {
                id: n.id(),
                lon: n.lon(),
                lat: n.lat(),
                tags: owned(n.tags()),
            }),
            Element::Node(n) if wanted.contains(&n.id()) => nodes.push(OsmNode {
                id: n.id(),
                lon: n.lon(),
                lat: n.lat(),
                tags: owned(n.tags()),
            }),
            _ => {}
        })
        .map_err(|e| decode(path, &e))?;
    ways.sort_unstable_by_key(|w| w.id);
    nodes.sort_unstable_by_key(|n| n.id);
    Ok(Osm { nodes, ways })
}

fn reader(path: &Path) -> Result<ElementReader<std::io::BufReader<std::fs::File>>, OsmError> {
    let file = std::fs::File::open(path)
        .map_err(|source| OsmError::Io { path: path.to_path_buf(), source })?;
    Ok(ElementReader::new(std::io::BufReader::new(file)))
}

fn decode(path: &Path, e: &osmpbf::Error) -> OsmError {
    OsmError::Decode { path: path.to_path_buf(), message: e.to_string() }
}

fn owned<'a>(tags: impl Iterator<Item = (&'a str, &'a str)>) -> Vec<(String, String)> {
    tags.map(|(k, v)| (k.to_owned(), v.to_owned())).collect()
}
