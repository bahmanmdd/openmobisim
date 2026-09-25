//! The multilayer network: geometry, structure and the parameters that turn
//! OSM tags into a traffic model.
//!
//! | Module | What it owns |
//! |---|---|
//! | [`geometry`] | WGS84 → projected metres, from scratch, UTM only (S81) |
//! | [`connectivity`] | Strongly connected components of the node graph and of the link graph (legal turns) |
//! | [`csr`] | The compressed-sparse-row adjacency every structure here is built from |
//! | [`defaults`] | The versioned, cited, overridable table that turns OSM tags into a fundamental diagram |
//! | [`network`] | The road network itself: structure of arrays, built once, then immutable |
//! | [`turns`] | (incoming link, outgoing link) pairs — the node model's unit of work |
//! | [`link_geometry`] | The shape of each street, kept outside `RoadNetwork` (S125) |
//! | [`layers`] | The bike and walk layers: static-cost networks over the same geometry (S193, S195) |
//! | [`examples`] | `manhattan_grid` and `toy_network` — the shared synthetic-network fixtures (S105, S161) |
//!
//! # The one thing to know
//!
//! **Everything in this crate is built once and then immutable.** The network,
//! its turns and its parameters are shared behind an `Arc` by every run in a
//! batch (Foundations §5); nothing here is per-run state. A disruption does not
//! mutate the network — it writes new immutable arrays at its scheduled time
//! (§3e).

#![cfg_attr(docsrs, feature(doc_cfg))]

pub mod connectivity;
pub mod csr;
pub mod defaults;
pub mod examples;
pub mod geometry;
pub mod layers;
pub mod link_geometry;
pub mod network;
pub mod turns;

pub use csr::Csr;
pub use defaults::{
    DEFAULTS_VERSION, DefaultRow, GlobalMultipliers, LinkParameters, ParameterNote, RoadClass,
    SignalDefaults, default_row,
};
pub use examples::{manhattan_grid, toy_network, toy_network_layers};
pub use geometry::{
    Hemisphere, LonLat, Projected, Projection, ProjectionError, UtmZone, ground_distance_metres,
    haversine_metres, polyline_length_metres, scale_factor,
};
pub use layers::{
    BikeCost, BikeInfrastructure, StaticLayer, StaticLayerDefaults, StaticLink, StaticNetwork,
    StaticNetworkBuilder,
};
pub use link_geometry::{LinkGeometry, NetworkFingerprint};
pub use network::{LinkSpec, RoadNetwork, RoadNetworkBuilder};
pub use turns::TurnTable;
