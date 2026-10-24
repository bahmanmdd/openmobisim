//! Route sets for openmobisim (§16).
//!
//! | Module | What it holds |
//! |---|---|
//! | [`search`] | The turn-aware shortest-path search every method is built on |
//! | [`attributes`] | [`RouteAttributes`]: each route's length and path size, for the choice layer |
//! | [`generate`] | The [`RouteSetGenerator`] interface, the built-in methods (`penalty`, the default, `shortest` and the congestion-biased `montecarlo`) and the by-name [`Registry`] |
//! | [`store`] | [`RouteSets`]: routes flat in CSR form, with identity, an inverted link index and per-route metadata; it can be [extended](RouteSets::extended) with routes found later; [`search_map`] runs a search over many items in parallel, in order |
//! | [`snap`] | [`NodeSnapper`]: the nearest drivable node to a point, from a grid, not a scan; [`TripSnapper`]: where a trip's end joins the roads (a street, not a motorway) |
//! | [`reach`] | [`Reach`]: bounded one-to-many searches by node, forward and backward (transit's walks) |
//!
//! # Extending it
//!
//! A new way to make route sets is a type that implements
//! [`RouteSetGenerator`]. Pass it to [`RouteSets::generate`] directly, or add a
//! factory to a [`Registry`] to select it by name like the built-in ones. The
//! store, the parallel driver, the identity and (through `py-bindings`) the
//! Python surface take no other change.
//!
//! **The flow motor never computes a route**: it receives
//! vehicles that carry theirs. Nothing in `core-loading` depends on this crate.

pub mod attributes;
pub mod generate;
pub mod reach;
pub mod search;
pub mod snap;
pub mod store;

pub use attributes::{RouteAttributes, path_sizes, path_sizes_counted};
pub use generate::{
    DEFAULT_METHOD, Demand, MonteCarlo, Options, PROPENSITY_BIN_SECONDS, Penalty, Registry,
    RouteError, RouteSetGenerator, Shortest, TripDemand, congestion_propensity, default_generator,
    generator,
};
pub use reach::Reach;
pub use search::{LinkFactors, MAX_ROUTES_PER_SET, Route, Search, SearchContext};
pub use snap::{NodeSnapper, TripSnapper};
pub use store::{LinkIndex, RouteKey, RouteSets, RouteView, search_map};
