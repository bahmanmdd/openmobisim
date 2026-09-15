//! Fixtures shared by the network and turn tests.
//!
//! Thin wrappers around [`openmobisim_core_graph::examples::manhattan_grid`] —
//! the fixture itself was promoted out of this module and into the crate's
//! public API in Phase 1 step 8 (S105), since it needed to be reusable
//! outside tests too. What is kept here is just this test suite's own
//! historical names (`grid`, `grid_link_count`), so the three test files
//! that already call them did not need to change.

#![allow(dead_code, reason = "each test binary uses a different part of this module")]

use openmobisim_core_graph::examples;
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_types::diagnostics::Diagnostics;

/// The block spacing this test suite has always used — "roughly 100 m",
/// same as before promotion; nothing here asserts an exact coordinate, so
/// the precise value is not load-bearing.
pub const BLOCK_METRES: f64 = 100.0;

/// The node external id for grid position `(row, column)`.
#[must_use]
pub fn node_name(row: u32, col: u32) -> String {
    examples::node_name(row, col)
}

/// An `n × n` grid of nodes with bidirectional links along every edge.
///
/// `signalised` marks every interior node as signal-controlled, which is how
/// the control-delay and turn-capacity paths get exercised.
#[must_use]
pub fn grid(n: u32, signalised: bool) -> (RoadNetwork, Diagnostics) {
    examples::manhattan_grid(n, BLOCK_METRES, signalised)
}

/// How many links an `n × n` grid has.
#[must_use]
pub fn grid_link_count(n: u32) -> u32 {
    examples::link_count(n)
}
