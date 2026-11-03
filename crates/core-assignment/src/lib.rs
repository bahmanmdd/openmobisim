//! The iterated dynamic traffic assignment: travellers choose door-to-door chains, their
//! vehicles are loaded on the road network, and the choices are brought towards equilibrium
//! over assignment iterations.
//!
//! # The loop
//!
//! Every assignment iteration runs five stages, in this order and nothing else
//! ([`assign()`]; details in [`mod@assign`]):
//!
//! 1. **search**: the cheapest chain of every trip at the current link travel times, added to
//!    the trip's choice set if new; the stopping check;
//! 2. **choose**: one choice over every trip's set by the choice model, the averaging rule
//!    deciding who re-chooses;
//! 3. **load**: the chosen chains' vehicles moved on the network by the loader;
//! 4. **measure**: the link travel times by time of day built from what the loading recorded,
//!    and every chain of every set priced again on them;
//! 5. **assess**: the convergence measures ([`IterationReport`]).
//!
//! The [`State`] (link travel times, and per trip the choice set, the current chain and the
//! last choice probabilities) is a value the loop takes and returns; nothing else holds
//! anything between iterations.
//!
//! # One cost
//!
//! Every alternative is priced in **utility units**: minus the systematic utility of the
//! traveller's user class, `cost = Σ coefficient × attribute` over in-vehicle, walking and
//! waiting minutes, transfers, money and kilometres ([`Coefficients`]). The route search
//! minimises it link by link, the choice model reads it, the duality gap is measured in it. A
//! **time-based run** is the run whose coefficients are [`Coefficients::time_based`] (a minute
//! of travel time costs 1, everything else 0); it runs the same code.
//!
//! # Modules
//!
//! | Module | What it holds |
//! |---|---|
//! | [`attributes`] | [`Attributes`] of a leg or chain, a class's [`Coefficients`], the utility |
//! | [`chain`] | [`Place`], [`Leg`], [`Chain`] with its identity across iterations |
//! | [`state`] | [`State`], the value carried between iterations; [`TripState`] |
//! | [`planner`] | the [`LegPlanner`] contract, the [`CarPlanner`], the [`CostSource`] |
//! | [`search`] | the [`ChainSearch`] and the one feasibility rule, [`feasible`] |
//! | [`chooser`] | what the choice model is handed, [`HighestUtility`], [`logit`], the averaging rule |
//! | [`loader`] | the [`Loader`] contract, [`Loaded`], level 0 [`FreeFlow`], level 2 [`PointQueue`] |
//! | [`hooks`] | [`StageHook`], code run at fixed points of the loop, and [`NoHooks`] |
//! | [`report`] | [`IterationReport`], the convergence measures of one iteration |
//! | [`mod@assign`] | [`assign()`], its [`Inputs`] and [`Params`], the five stages |
//! | [`error`] | [`AssignError`] |
//!
//! # What exists so far
//!
//! Car trips only, one leg each, loaded at free flow (level 0) or by the point queue (level 2,
//! queues at the links' downstream ends discharging at capacity, signals included): the chain
//! search finds one route per trip per iteration, so a choice set grows by at most one chain per
//! iteration and nothing is enumerated in advance. A further mode is a further [`LegPlanner`],
//! a further loading a further [`Loader`], behind the same contracts. Everything runs on one
//! thread; every random draw is addressed by identity, so splitting the stages over threads
//! would change no result.
//!
//! Nothing in the loop is specific to the car: a bicycle leg would be planned by a planner of
//! its own on the bicycle layer, and chosen, loaded and measured by the same stages.

pub mod assign;
pub mod attributes;
pub mod chain;
pub mod chooser;
pub mod error;
pub mod hooks;
pub mod loader;
pub mod planner;
pub mod report;
pub mod search;
pub mod state;

pub use assign::{Inputs, Params, assign};
pub use attributes::{Attributes, Coefficients};
pub use chain::{Chain, Leg, Place};
pub use chooser::{HighestUtility, logit, rechoice_probability, rechooses};
pub use error::AssignError;
pub use hooks::{NoHooks, StageHook};
pub use loader::{FreeFlow, Loaded, Loader, PointQueue};
pub use planner::{CarPlanner, CostSource, LegPlanner};
pub use report::{IterationReport, StageSeconds};
pub use search::{ChainSearch, feasible};
pub use state::{State, TripState};
