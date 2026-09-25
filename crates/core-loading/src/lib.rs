//! The flow motor: vehicles on the curves (S85).
//!
//! | Module | What it owns |
//! |---|---|
//! | [`vehicle`] | [`vehicle::Vehicle`] — a route, a PCU weight, a departure time |
//! | [`level0`] | Level 0 of the fidelity ladder: free-flow traversal, no curves, no interaction |
//! | [`curves`] | [`curves::LinkCurves`] — per-link cumulative counts with bounded history; the receiving condition |
//! | [`node_model`] | S48/S77's node model in aggregate form, the reference statement of the rules [`ltm`] applies per vehicle |
//! | [`link_bins`] | [`link_bins::LinkBins`] — per-link, per-time-bin results recorded as the loading runs (S163) |
//! | [`ltm`] | [`ltm::run_ltm`] — levels 2–4: the LTM with vehicles on the curves, processed in time order (S151) |
//!
//! # Where this crate is, and where it is going
//!
//! Design §10's fidelity ladder is levels 0–4, **nested — the same code with
//! parameters changed**. Phase 1 shipped level 0 only, to exercise the
//! [`vehicle::Vehicle`]/[`level0::Trajectory`] plumbing that levels 2–4 reuse.
//! [`ltm`] is levels 2–4 — see its module docs for the mechanism and its
//! recorded biases. Level 1 (volume-delay) is not built. **The flow
//! motor never computes a route** (design §10.4) — every [`vehicle::Vehicle`]
//! arrives with one already assigned; where that route comes from before
//! Phase 2 item 6's route-set system exists is `core-sim`'s question, not
//! this crate's (S133).

pub mod curves;
pub mod level0;
pub mod link_bins;
pub mod ltm;
pub mod node_model;
pub mod vehicle;

pub use curves::LinkCurves;
pub use level0::{
    LinkTraversal, Trajectory, load_level_0, load_level_0_binned, load_level_0_recorded,
    load_timed_binned, traverse_free_flow, traverse_timed,
};
pub use link_bins::{EntryTables, LinkBinRecorder, LinkBins};
pub use ltm::{FidelityLevel, LtmNetwork, run_ltm, run_ltm_binned, run_ltm_recorded};
pub use node_model::{TurnDemand, solve_node};
pub use vehicle::Vehicle;
