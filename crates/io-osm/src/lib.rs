//! OpenStreetMap import.
//!
//! | Module | What it owns |
//! |---|---|
//! | [`source`] | Where elements come from — a trait, an in-memory source, and the PBF adapter |
//! | [`tags`] | Interpreting OSM tags: class, direction, lanes, speed limits |
//! | [`mod@import`] | Classifying, splitting, contracting, and handing the result to the graph builder |
//! | [`pbf`] | The `.osm.pbf` adapter — thin, and the only part that knows about a file format |
//!
//! # The shape of an import
//!
//! 1. **Ways first.** Classify each way, and count how many kept ways use each
//!    node. A node used by two or more is a junction.
//! 2. **Nodes second**, keeping only the ones some kept way references. This is
//!    the pass that decides whether importing a city needs two gigabytes or
//!    twelve.
//! 3. **Split** each way at its junctions, so that a link runs from one
//!    junction to the next and the geometry in between becomes its length.
//! 4. **Hand to the graph builder**, which assigns ids, projects, applies the
//!    defaults table and builds the turns.
//!
//! Nothing here fails because of odd data. A way with an uninterpretable speed
//! limit, a node referenced but never defined, a way of one node — each has a
//! documented fallback and a diagnostic, because the simulation never stops
//! (brief §3c).

#![cfg_attr(docsrs, feature(doc_cfg))]

pub mod import;
#[cfg(feature = "pbf")]
#[cfg_attr(docsrs, doc(cfg(feature = "pbf")))]
pub mod pbf;
pub mod source;
pub mod tags;

pub use import::{ImportOptions, ImportReport, import};
#[cfg(feature = "pbf")]
pub use pbf::PbfSource;
pub use source::{MemorySource, OsmError, OsmNode, OsmSource, OsmWay};
pub use tags::{Direction, Lanes, Maxspeed, Rejection};
