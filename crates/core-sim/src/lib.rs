//! The `Run`: orchestration, the event queue, and Phase 1's one KPI (S62).
//!
//! | Module | What it owns |
//! |---|---|
//! | [`run`] | [`run::Run`] — the orchestration loop, [`run::RunResult`], [`run::TripCompletionStats`] (S57) |
//! | [`equilibration`] | [`equilibration::Equilibration`] — repeating choice and loading: `none` and `msa` in traveller form, and the per-iteration [`equilibration::IterationReport`] (S170) |
//! | [`events`] | [`events::EventRow`] — one row per trip outcome, what `io-parquet`'s `events.parquet` writer reads from |
//! | [`link_times`] | [`link_times::LinkTimes`] — link travel times by time of day from the last loading, and the change between two loadings (S170) |
//! | [`route_cache`] | [`route_cache::RouteSetCache`] — generated route sets kept for the next run that asks for the same (S178) |
//! | [`route_update`] | [`route_update::RouteUpdate`] — growing the route sets between iterations: `none` and `best_response` (S176) |
//! | [`route_choice`] | [`route_choice::RouteChoices`] — which route each trip takes, from its pair's route set and a choice model (S169) |
//! | [`identity`] | [`identity::RunDescription`] — a run's master seed and fingerprint, made before it executes (S168) |
//!
//! # What this crate is, and is not, yet
//!
//! This is Phase 1 step 6: enough orchestration to turn `core-demand`'s
//! travellers and trips into `core-loading` trajectories and report one
//! number. No hubs, no equilibration, no convergence report, no multi-
//! iteration — Foundations §10 names all of those as `core-sim`'s eventual
//! job; this is its first, narrowest vertical slice, car trips only (S127's
//! `Ownership::car`), because no other mode layer exists yet.
//!
//! **[`run::RunResult::total_travel_time`] is *traveller-weight-scaled***
//! (population total, `Σ w · travel_time` over completed trips), not the raw
//! sum over simulated travellers — the assistant's recommendation (brief
//! §6b), implemented and flagged rather than left undone. **Confirmed by the
//! user, 2026-09-14**: fine as Phase 1's only KPI as long as it stays
//! documented; once comprehensive KPIs exist, *both* the weighted and
//! unweighted forms will be reported side by side, not one replacing the
//! other. `io-parquet`'s `kpis.parquet` writer (long format) is where the
//! second metric row lands when that happens — nothing here needs to change
//! to add it.

pub mod equilibration;
pub mod events;
pub mod identity;
pub mod link_times;
pub mod route_cache;
pub mod route_choice;
pub mod route_update;
pub mod run;

pub use equilibration::{Equilibration, IterationReport, Msa, NoEquilibration};
pub use events::{EventRow, EventType};
pub use identity::RunDescription;
pub use link_times::{LinkTimes, relative_time_change};
pub use route_cache::RouteSetCache;
pub use route_choice::{NO_ROUTE, ROUTE_ATTRIBUTES, RouteChoices};
pub use route_update::{BestResponse, NoRouteUpdate, RouteUpdate};
pub use run::{FlowMotor, Run, RunError, RunResult, TripCompletionStats};
