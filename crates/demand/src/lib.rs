//! Demand: the trips of a run, each end linked to the network, and the transit access and
//! egress lists of each trip.
//!
//! | Module | What it holds |
//! |---|---|
//! | [`input`] | [`input::TripRow`], one trip as given, and [`input::DemandError`], what is refused |
//! | [`demand`] | [`demand::Demand::build`] and the [`demand::Demand`] it returns: trips by id, their ends linked per mode, their access and egress lists |
//! | [`index`] | [`index::NodeIndex`]: the nearest node a mode may use, by a grid of cells searched in rings |
//! | [`reach`] | the stops within walking reach of every node, found once per stop; the nearest stop of an end with none within reach |
//!
//! A bike's end is its own: the nearest node with a link a bike may use, which may differ from
//! the car's and the walker's. Units are metric: metres and seconds.

pub mod demand;
pub mod index;
pub mod input;
pub mod reach;
