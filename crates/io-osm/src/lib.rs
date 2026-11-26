//! Reading one OpenStreetMap extract, or the part of it a region holds, into the network: three
//! layers of one mode each, the car links, the bike links and the walk links, each from the ways
//! its rules admit, split at its junctions, trimmed to its largest strongly connected
//! component and contracted (version 1's reader). A car link carries its lanes, free-flow speed
//! and capacity from its tags and its road class; every link its shape and road class, for
//! drawing.
//!
//! | Module | What it holds |
//! |---|---|
//! | [`source`] | the input ([`Osm`]: nodes and ways as plain lists) and the errors |
//! | [`pbf`] | the `.osm.pbf` decoder, which fills an [`Osm`] |
//! | [`tags`] | the road classes and their car defaults; who may use a way, which way; lanes, speeds |
//! | [`region`] | a study area cut out of the extract |
//! | [`geodesic`] | a segment's length on the WGS84 ellipsoid |
//! | [`mod@import`] | the three layers: splitting, the strong component, contraction, the build |
//!
//! A bike is a mode with links of its own: a bike link runs where version 1's bike rules let a
//! bike ride (a street, a cycleway, a footway with a `bicycle` tag), in the directions they give.
//! Units are metric: metres, km/h, vehicles per hour.

pub mod geodesic;
pub mod import;
pub mod pbf;
pub mod region;
pub mod source;
mod strong;
pub mod tags;
pub mod trips;

use std::path::Path;

use openmobisim_graph::network::{BuildValues, Network};

pub use import::{Report, network, network_for};
pub use pbf::read_pbf;
pub use region::{Region, RegionError};
pub use source::{Osm, OsmError, OsmNode, OsmWay};
pub use tags::CarRoads;

/// Read the `.osm.pbf` file at `path` (cut to `region`, if given) into the network, built with
/// `values` (the caller's: `capacity_per_lane`, `bike_speed`, `walk_speed`, see
/// `PARAMETERS.md`), and its counts.
///
/// # Errors
///
/// An [`OsmError`]: the file cannot be read or decoded, no way is a street, or the builder
/// refuses (no node left, a build value out of range).
pub fn read(
    path: &Path,
    values: BuildValues,
    region: Option<&Region>,
) -> Result<(Network, Report), OsmError> {
    network(&read_pbf(path)?, values, region)
}
