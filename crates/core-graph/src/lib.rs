//! The multilayer network: geometry, structure and the parameters that turn
//! OSM tags into a traffic model.
//!
//! | Module | What it owns |
//! |---|---|
//! | [`geometry`] | WGS84 → projected metres, from scratch, UTM only |
//! | [`connectivity`] | Strongly connected components of the node graph and of the link graph (legal turns) |
//! | [`csr`] | The compressed-sparse-row adjacency every structure here is built from |
//! | [`defaults`] | The versioned, cited, overridable table that turns OSM tags into a fundamental diagram |
//! | [`network`] | The road network itself: structure of arrays, built once, then immutable |
//! | [`binary`] | The binary form of a built network, read back bit for bit: a network cache file |
//! | [`turns`] | (incoming link, outgoing link) pairs — the node model's unit of work |
//! | [`link_geometry`] | The shape of each street, kept outside `RoadNetwork` |
//! | [`layers`] | The bike and walk layers: static-cost networks over the same geometry |
//! | [`hubs`] | Hubs: access points on layers and the transfers between them |
//! | [`examples`] | `manhattan_grid` and `toy_network` — the shared synthetic-network fixtures |
//!
//! # The one thing to know
//!
//! **Everything in this crate is built once and then immutable.** The network,
//! its turns and its parameters are shared behind an `Arc` by every run in a
//! batch; nothing here is per-run state. A disruption does not
//! mutate the network — it writes new immutable arrays at its scheduled time
//! (§3e).

#![cfg_attr(docsrs, feature(doc_cfg))]

pub mod binary;
pub mod connectivity;
pub mod csr;
pub mod defaults;
pub mod examples;
pub mod geometry;
pub mod hubs;
pub mod layers;
pub mod link_geometry;
pub mod network;
pub mod network_options;
pub mod turns;

pub use csr::Csr;
pub use defaults::{
    DEFAULTS_VERSION, DefaultRow, GlobalMultipliers, LinkParameters, ParameterNote, RoadClass,
    SaturationBands, SignalDefaults, default_row,
};
pub use examples::{
    manhattan_grid, toy_network, toy_network_hubs, toy_network_layers, toy_network_parkings,
};
pub use geometry::{
    Hemisphere, LonLat, Projected, Projection, ProjectionError, UtmZone, ground_distance_metres,
    haversine_metres, polyline_length_metres, scale_factor,
};
pub use hubs::{
    AccessPoint, HubKind, HubSet, HubSpec, ParkingKind, ParkingRow, Resource, ResourceKind,
    UNLIMITED,
};
pub use layers::{
    BikeCost, BikeInfrastructure, Layer, StaticLayer, StaticLayerDefaults, StaticLink,
    StaticNetwork, StaticNetworkBuilder,
};
pub use link_geometry::{LinkGeometry, NetworkFingerprint};
pub use network::{LinkSpec, RoadNetwork, RoadNetworkBuilder};
pub use network_options::{ClassTable, NetworkDefaults};
pub use turns::TurnTable;
