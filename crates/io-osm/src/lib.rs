//! Reading one OpenStreetMap extract into the network: every street way becomes directed links
//! between junction nodes, each with the modes (car, bike, walk) the wiki's access rules give
//! it in its own direction, and, for a car link, its lanes and free-flow speed.
//!
//! | Module | What it holds |
//! |---|---|
//! | [`source`] | the input ([`Osm`]: nodes and ways as plain lists) and the errors |
//! | [`pbf`] | the `.osm.pbf` decoder, which fills an [`Osm`] |
//! | [`class`] | the street classes: default modes, the car's default speed and lanes |
//! | [`access`] | who may use a way, in which direction |
//! | [`car`] | a car link's lanes and free-flow speed from the tags |
//! | [`geodesic`] | a segment's length on the WGS84 ellipsoid |
//! | [`mod@import`] | splitting, measuring, the car network's strong component, the build |
//!
//! A bike is a mode on the same links as a car and a walker: a cycleway is a link for bikes
//! and walkers with no car field; a street a bike may ride carries bike in its mode set. Units
//! are metric: metres, km/h (as the graph's rows take them).

pub mod access;
pub mod car;
pub mod class;
pub mod geodesic;
pub mod import;
pub mod pbf;
pub mod source;
mod strong;

use std::path::Path;

use openmobisim_graph::network::{BuildValues, Network};

pub use import::{Report, network};
pub use pbf::read_pbf;
pub use source::{Osm, OsmError, OsmNode, OsmWay};

/// Read the `.osm.pbf` file at `path` into the network, built with `values` (the caller's:
/// `capacity_per_lane`, `bike_speed`, `walk_speed`, see `PARAMETERS.md`), and its counts.
///
/// # Errors
///
/// An [`OsmError`]: the file cannot be read or decoded, a tag cannot be read (naming the way),
/// a way names a node the file lacks, no way is a street, or the builder refuses a row.
pub fn read(path: &Path, values: BuildValues) -> Result<(Network, Report), OsmError> {
    network(&read_pbf(path)?, values)
}
