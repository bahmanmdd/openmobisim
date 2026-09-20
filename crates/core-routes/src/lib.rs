//! Route sets for openmobisim (Phase 2 item 6; design §13, §16).
//!
//! | Module | What it holds |
//! |---|---|
//! | [`search`] | The turn-aware shortest-path search every method is built on |
//! | [`attributes`] | [`RouteAttributes`]: each route's length and path size, for the choice layer |
//! | [`generate`] | The [`RouteSetGenerator`] interface, the built-in methods (`penalty`, the default, and `shortest`) and the by-name [`Registry`] |
//! | [`store`] | [`RouteSets`]: routes flat in CSR form, with identity, an inverted link index and per-route metadata |
//! | [`snap`] | [`NodeSnapper`]: the nearest drivable node to a point, from a grid, not a scan |
//!
//! # Extending it
//!
//! A new way to make route sets is a type that implements
//! [`RouteSetGenerator`]. Pass it to [`RouteSets::generate`] directly, or add a
//! factory to a [`Registry`] to select it by name like the built-in ones. The
//! store, the parallel driver, the identity and (through `py-bindings`) the
//! Python surface take no other change.
//!
//! **The flow motor never computes a route** (design §10.4): it receives
//! vehicles that carry theirs. Nothing in `core-loading` depends on this crate.

pub mod attributes;
pub mod generate;
pub mod search;
pub mod snap;
pub mod store;

pub use attributes::RouteAttributes;
pub use generate::{
    DEFAULT_METHOD, Options, Penalty, Registry, RouteError, RouteSetGenerator, Shortest,
    default_generator, generator,
};
pub use search::{LinkFactors, MAX_ROUTES_PER_SET, Route, Search, SearchContext};
pub use snap::NodeSnapper;
pub use store::{LinkIndex, RouteKey, RouteSets, RouteView};
