//! The network: links with an upstream and a downstream end, free-flow time as length over speed, capacity, signals stored and never applied.
//!
//! | Module | What it holds |
//! |---|---|
//! | [`network`] | the network, built once from node and link rows by [`network::NetworkBuilder`], then immutable |
//! | [`geometry`] | longitude/latitude to metres: the UTM projection, from scratch |
//! | [`csr`] | the compressed-sparse-row adjacency behind each node's out-links and in-links |
//!
//! A bike is a mode on the same links as a car and a walker, with its own free-flow speed; there
//! is one network. Units are metric: metres, metres per second, vehicles per second inside;
//! kilometres per hour and vehicles per hour only in the rows, converted once as they are read.

pub mod csr;
pub mod geometry;
pub mod network;
