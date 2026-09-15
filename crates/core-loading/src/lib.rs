//! The flow motor: vehicles on the curves (S85).
//!
//! | Module | What it owns |
//! |---|---|
//! | [`vehicle`] | [`vehicle::Vehicle`] — a route, a PCU weight, a departure time |
//! | [`level0`] | Level 0 of the fidelity ladder: free-flow traversal, no curves, no interaction |
//! | [`curves`] | [`curves::LinkCurves`] — per-link cumulative curves, sending/receiving flow |
//! | [`node_model`] | S48/S77's node model: per-turn demand, per-link supply, no fixed point |
//! | [`ltm`] | [`ltm::run_ltm`] — S84's iterative LTM, S85's vehicle hand-over, Phase 2 item 2's N3 prototype |
//!
//! # Where this crate is, and where it is going
//!
//! Design §10's fidelity ladder is levels 0–4, **nested — the same code with
//! parameters changed**. Phase 1 shipped level 0 only, to exercise the
//! [`vehicle::Vehicle`]/[`level0::Trajectory`] plumbing that levels 1–4 (the
//! iterative LTM, S84) reuse without the numerical machinery those levels
//! need. [`ltm`] is Phase 2 item 2's **prototype** of that machinery — see
//! its module docs for exactly what it does and does not yet do. **The flow
//! motor never computes a route** (design §10.4) — every [`vehicle::Vehicle`]
//! arrives with one already assigned; where that route comes from before
//! Phase 2 item 6's route-set system exists is `core-sim`'s question, not
//! this crate's (S133).

pub mod curves;
pub mod level0;
pub mod ltm;
pub mod node_model;
pub mod vehicle;

pub use curves::LinkCurves;
pub use level0::{LinkTraversal, Trajectory, load_level_0, traverse_free_flow};
pub use ltm::{FidelityLevel, LtmNetwork, run_ltm};
pub use node_model::{TurnDemand, solve_node};
pub use vehicle::Vehicle;
