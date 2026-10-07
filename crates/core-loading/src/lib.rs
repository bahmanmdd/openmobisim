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
//! # The fidelity ladder
//!
//! Design §10's levels, **nested — the same code with parameters changed**:
//! level 0 is free flow ([`level0`]: no curves, no interaction); levels 2–4 are
//! [`ltm`], the link transmission model with vehicles on the curves: **2, the
//! point queue** (capacities at every link and junction, queues that take no
//! road space; the default since S229), 3 (storage, without the backward
//! wave's delay) and **4, the full model** (spillback with the backward wave,
//! turn pockets, en-route rerouting). See [`ltm`]'s module docs for the
//! mechanism and its recorded biases. Level 1 (volume-delay) is not built.
//!
//! **The flow motor never computes a route** (design §10.4): every
//! [`vehicle::Vehicle`] arrives with one already assigned (a reroute's new
//! route included, chosen by the rule the run hands in).

pub mod curves;
mod events;
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
pub use ltm::{
    CapacityChange, Chain, FidelityLevel, LiveTimes, LockReport, LtmNetwork, LtmOutput, Recording,
    Reroute, RerouteReason, RerouteRecord, RerouteRule, Rules, run_ltm, run_ltm_binned,
    run_ltm_chained, run_ltm_recorded,
};
pub use node_model::{TurnDemand, solve_node};
pub use vehicle::Vehicle;
