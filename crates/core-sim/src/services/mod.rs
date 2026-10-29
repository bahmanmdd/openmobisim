//! Shared-mobility services in a run, and the hubs they and every other subsystem share.
//!
//! | Module | What it owns |
//! |---|---|
//! | [`hubs`] | [`hubs::RunHubs`] — the run's one hub set: stops, parkings, docks, chargers and stands, joined by hub id or by distance |

pub mod hubs;
