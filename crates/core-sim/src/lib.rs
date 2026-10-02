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
//! | [`transit`] | [`transit::TransitSetup`] — a timetable linked to the walk and bike layers: stop hubs, walks, RAPTOR (S199) |
//! | [`parking`] | [`parking::ParkingSetup`] — parkings as hubs on the layers, and how full they are over the day (M4) |
//! | [`itinerary_choice`] | Itinerary choice: the competitive itineraries of transit, park-and-ride and bike-and-ride trips, and, with mode choice, every mode a trip without one can use, chosen by the choice model and executed on the loading's times (M4, M5) |
//!
//! # What this crate is, and is not, yet
//!
//! Orchestration for car trips (S127's `Ownership::car`), because no other
//! mode layer exists yet: route sets, route choice, a loading, and repeated
//! choice and loading towards an equilibrium with a convergence report per
//! iteration (see the table above). No hubs and no other modes — Foundations
//! §10 names those as `core-sim`'s later job.
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
pub mod itinerary;
pub mod itinerary_choice;
pub mod layers;
pub mod link_times;
pub mod parking;
pub mod route_cache;
pub mod route_choice;
pub mod route_update;
pub mod run;
pub mod transit;

pub use equilibration::{Equilibration, IterationReport, Msa, NoEquilibration};
pub use events::{EventRow, EventType};
pub use identity::RunDescription;
pub use itinerary::{Itinerary, LegRoute, Networks};
pub use itinerary_choice::{ATTRIBUTES, ItineraryResult, NO_MODE, NO_PARKING};
pub use layers::{LayerSetup, ModeDefaults, StaticLayers};
pub use link_times::{LinkTimes, relative_time_change};
pub use parking::{
    ParkingBins, ParkingDefaults, ParkingError, ParkingResult, ParkingSetup, ParkingSetupReport,
};
pub use route_cache::RouteSetCache;
pub use route_choice::{NO_ROUTE, ROUTE_ATTRIBUTES, RouteChoices};
pub use route_update::{BestResponse, NoRouteUpdate, RouteUpdate};
pub use run::{FlowMotor, ModeTotals, Run, RunError, RunResult, TripCompletionStats};
pub use transit::{BusReport, BusSummary, TransitResult, TransitSetup};
