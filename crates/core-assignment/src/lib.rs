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
//!    the trip's choice set if new, and counted as found only if it is cheaper than the set's
//!    best by more than the indifference band ([`Params::indifference_band`]); the stopping
//!    check;
//! 2. **choose**: one choice over every trip's set by the choice model, the averaging rule
//!    deciding who re-chooses (by seeded draws, or by the quota of each group of trips facing
//!    the same choice, [`Params::allocation`]); the deterministic choice treats the chains within the band of
//!    the best as tied and splits the trips facing a tie evenly over them, by trip id;
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
//! traveller's user class, `cost = Σ coefficient × attribute` over in-vehicle, walking,
//! waiting and parking minutes, transfers, money and kilometres ([`Coefficients`]). The route search
//! minimises it link by link, the choice model reads it, the duality gap is measured in it. A
//! **time-based run** is the run whose coefficients are [`Coefficients::time_based`] (a minute
//! of travel time costs 1, everything else 0); it runs the same code.
//!
//! # Modules
//!
//! | Module | What it holds |
//! |---|---|
//! | [`attributes`] | [`Attributes`] of a leg or chain, a class's [`Coefficients`], the utility |
//! | [`chain`] | [`Place`], [`Leg`] with its [`FixedTimes`], [`Chain`] with its identity across iterations |
//! | [`state`] | [`State`], the value carried between iterations; [`TripState`] |
//! | [`planner`] | the [`LegPlanner`] contract, the [`CarPlanner`], the [`CostSource`] |
//! | [`layer_planner`] | the bike and walk [`Layers`] and their [`StaticPlanner`] |
//! | [`search`] | the [`ChainSearch`] and the one feasibility rule, [`feasible`] |
//! | [`chooser`] | what the choice model is handed, [`HighestUtility`], [`logit`], the averaging rule |
//! | [`quota`] | the quota [`Allocation`]: a group's travellers in the model's proportions |
//! | [`loader`] | the [`Loader`] contract, [`Loaded`], level 0 [`FreeFlow`], level 2 [`PointQueue`] |
//! | [`hooks`] | [`StageHook`], code run at fixed points of the loop, and [`NoHooks`] |
//! | [`report`] | [`IterationReport`], the convergence measures of one iteration |
//! | [`mod@assign`] | [`assign()`], its [`Inputs`] and [`Params`], the five stages; [`restore`], a state rebuilt from what a run wrote ([`SavedTrip`]) |
//! | [`error`] | [`AssignError`] |
//!
//! # What exists so far
//!
//! Car trips, one leg each, loaded at free flow (level 0) or by the point queue (level 2,
//! queues at the links' downstream ends discharging at capacity, signals included), each car
//! leg with its fixed times of reaching the car and of parking and leaving it
//! ([`Params::car_fixed_times`], [`FixedTimes`]): the chain
//! search finds one route per trip per iteration, so a choice set grows by at most one chain per
//! iteration and nothing is enumerated in advance. **Bike and walking trips whose mode is
//! given**, one leg each on their own layer ([`Inputs::layers`], [`StaticPlanner`]): the route of
//! least cost on the layer at its static speeds, planned once, a bike leg with its fixed times
//! ([`Params::bike_fixed_times`]); they load nothing on the road network, so their chain never
//! changes and they never switch. A further mode is a further [`LegPlanner`], a further loading a
//! further [`Loader`], behind the same contracts.
//!
//! **Threads and shared searches.** The car planner answers the trips of one origin, user class
//! and departure window with as few searches as give every trip, link for link, the route its
//! own search would give ([`planner`], *shared searches*). The search, the choice's draws and
//! path sizes, the re-pricing and the per-trip terms of the convergence measures run in
//! parallel in fixed chunks, joined in order; every random draw is addressed by identity and
//! every sum over trips is taken in trip order, so the results are the same, to the bit, on one
//! thread or on many (the `parallel` feature, on by default; without it everything runs on the
//! calling thread).
//!
//! Nothing in the loop is specific to the car: a bike leg is planned by a planner of its own on
//! the bike layer, and chosen, measured and assessed by the same stages.

pub mod assign;
pub mod attributes;
pub mod chain;
pub mod chooser;
pub mod error;
pub mod hooks;
pub mod layer_planner;
pub mod loader;
mod parallel;
pub mod planner;
pub mod quota;
pub mod report;
pub mod search;
pub mod state;

pub use assign::{Inputs, Params, SavedTrip, assign, restore};
pub use attributes::{Attributes, Coefficients};
pub use chain::{Chain, FixedTimes, Leg, Place};
pub use chooser::{HighestUtility, logit, rechoice_probability, rechooses};
pub use error::AssignError;
pub use hooks::{NoHooks, StageHook};
pub use layer_planner::{Layer, Layers, StaticPlanner};
pub use loader::{FreeFlow, Loaded, Loader, PointQueue};
pub use planner::{CarPlanner, CostSource, LegPlanner};
pub use quota::Allocation;
pub use report::{IterationReport, StageSeconds};
pub use search::{ChainSearch, feasible};
pub use state::{State, TripState};
