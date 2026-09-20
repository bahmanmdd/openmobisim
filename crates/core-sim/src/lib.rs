//! The `Run`: orchestration, the event queue, and Phase 1's one KPI (S62).
//!
//! | Module | What it owns |
//! |---|---|
//! | [`run`] | [`run::Run`] — the orchestration loop, [`run::RunResult`], [`run::TripCompletionStats`] (S57) |
//! | [`events`] | [`events::EventRow`] — one row per trip outcome, what `io-parquet`'s `events.parquet` writer reads from |
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

pub mod events;
pub mod identity;
pub mod run;

pub use events::{EventRow, EventType};
pub use identity::RunDescription;
pub use run::{FlowMotor, Run, RunResult, TripCompletionStats};
