//! The `Run`: one simulation of a scenario, from the demand to the results.
//!
//! | Module | What it owns |
//! |---|---|
//! | [`run`] | [`run::Run`] — the orchestration loop, [`run::RunResult`], [`run::TripCompletionStats`] (S57) |
//! | [`equilibration`] | [`equilibration::Equilibration`] — repeating choice and loading: the free-flow loading (iteration 0, S223) and `msa` in traveller form, with the per-iteration [`equilibration::IterationReport`] |
//! | [`layers`] | [`layers::StaticLayers`] — the bike and walk trips' routes on their layers, once per run, and the choice-set limits by class (S196, S235) |
//! | [`itinerary`] | [`itinerary::Itinerary`] — a trip's legs across modes and hubs (S198) |
//! | [`itinerary_choice`] | Itinerary choice: the competitive itineraries of transit, park-and-ride and bike-and-ride trips, and, with mode choice, every mode a trip's class may use (S201–S205) |
//! | [`transit`] | [`transit::TransitSetup`] — a timetable linked to the walk and bike layers: stop hubs, walks, RAPTOR; buses on the roads (S199–S200) |
//! | [`parking`] | [`parking::ParkingSetup`] — parkings as hubs on the layers, and how full they are over the day (M4) |
//! | [`route_choice`] | [`route_choice::RouteChoices`] — which route each car trip takes, from its pair's route set and a choice model (S169) |
//! | [`route_update`] | [`route_update::RouteUpdate`] — growing the route sets between iterations: `none` and `best_response` (S176) |
//! | [`route_cache`] | [`route_cache::RouteSetCache`] — generated route sets kept for the next run that asks for the same (S178) |
//! | [`link_times`] | [`link_times::LinkTimes`] — link travel times by time of day from the last loading, and the change between two loadings (S170) |
//! | [`link_values`] | [`link_values::LinkValues`] — values a user gives per link, summed along each alternative for the choice models (S237) |
//! | [`prices`] | [`prices::Prices`] — what an alternative costs in euros (running cost, tolls, parking fees, fares), offered to the choice models (S248) |
//! | [`loading_rules`] | [`loading_rules::LoadingOptions`] — the loading's rules by name: turn pockets, en-route rerouting, priority at merges (S213–S218) |
//! | [`disruptions`] | [`disruptions::Disruptions`] — roads closed and transit stopped at a time of day, known or not in advance (S239) |
//! | [`skims`] | [`skims::Skimmer`] — door-to-door times between points by mode, after a run (S238) |
//! | [`events`] | [`events::EventRow`] — one row per trip outcome, what `io-parquet`'s `events.parquet` writer reads from |
//! | [`identity`] | [`identity::RunDescription`] — a run's master seed and fingerprint, made before it executes (S168) |
//! | [`timings`] | [`timings::Timings`] — wall-clock time by stage, kept apart from the results (S223) |
//!
//! # What a run does
//!
//! It builds the travellers, prepares the layers, the timetable and the parkings, makes the
//! car route sets, then repeats choice (routes; itineraries and modes) and a loading of the road
//! network (cars and buses together) until the equilibration stops, and assembles the results.
//! Walking and cycling have static costs: no interaction with traffic (S196).
//!
//! **[`run::RunResult::total_travel_time`] is *traveller-weight-scaled***
//! (population total, `Σ w · travel_time` over completed trips), not the raw
//! sum over simulated travellers (confirmed by the user, 2026-09-14); the
//! people the completed trips stand for are counted beside it, so a mean per
//! person is one division (S232).

pub mod disruptions;
pub mod equilibration;
pub mod events;
pub mod identity;
pub mod itinerary;
pub mod itinerary_choice;
pub mod layers;
pub mod link_times;
pub mod link_values;
pub mod loading_rules;
pub mod parking;
pub mod prices;
mod reroute;
pub mod route_cache;
pub mod route_choice;
pub mod route_update;
pub mod run;
pub mod skims;
pub mod timings;
pub mod transit;

pub use disruptions::{Disruptions, RoadDisruption, TransitDisruption, TransitEffect};
pub use equilibration::{Equilibration, FreeFlow, IterationReport, Msa};
pub use events::{EventRow, EventType};
pub use identity::RunDescription;
pub use itinerary::{Itinerary, LegRoute, Networks};
pub use itinerary_choice::{ATTRIBUTES, ItineraryResult, NO_MODE, NO_PARKING};
pub use layers::{ClassLimits, LayerSetup, ModeDefaults, StaticLayers};
pub use link_times::{LinkTimes, relative_time_change};
pub use link_values::{LinkValues, ValueLayer};
pub use loading_rules::LoadingOptions;
pub use parking::{
    ParkingBins, ParkingDefaults, ParkingError, ParkingResult, ParkingSetup, ParkingSetupReport,
};
pub use prices::{COST_ATTRIBUTES, Prices, TOLL_COLUMN};
pub use route_cache::RouteSetCache;
pub use route_choice::{NO_ROUTE, ROUTE_ATTRIBUTES, RouteChoices};
pub use route_update::{BestResponse, NoRouteUpdate, RouteUpdate};
pub use run::{FlowMotor, ModeTotals, Run, RunError, RunResult, TripCompletionStats};
pub use skims::Skimmer;
pub use timings::{Stage, Timings};
pub use transit::{BusReport, BusSummary, TransitResult, TransitSetup};
