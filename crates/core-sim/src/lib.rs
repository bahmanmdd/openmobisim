//! The `Run`: one simulation of a scenario, from the demand to the results.
//!
//! | Module | What it owns |
//! |---|---|
//! | [`run`] | [`run::Run`] — the orchestration loop, [`run::RunResult`], [`run::TripCompletionStats`] |
//! | [`equilibration`] | [`equilibration::Equilibration`] — repeating choice and loading: the free-flow loading (iteration 0) and `msa` in traveller form, with the per-iteration [`equilibration::IterationReport`] |
//! | [`layers`] | [`layers::StaticLayers`] — the bike and walk trips' routes on their layers, once per run, and the choice-set limits by class |
//! | [`itinerary`] | [`itinerary::Itinerary`] — a trip's legs across modes and hubs |
//! | [`itinerary_choice`] | Itinerary choice: the competitive itineraries of transit, park-and-ride and bike-and-ride trips, and, with mode choice, every mode a trip's class may use |
//! | [`transit`] | [`transit::TransitSetup`] — a timetable linked to the walk and bike layers: stops' access points, walks, RAPTOR; buses on the roads |
//! | [`parking`] | [`parking::ParkingSetup`] — parkings on the layers, and how full they are over the day |
//! | [`services`] | [`services::hubs::RunHubs`] — the run's one hub set: stops, parkings, docks, chargers and stands |
//! | [`route_choice`] | [`route_choice::RouteChoices`] — which route each car trip takes, from its pair's route set and a choice model |
//! | [`route_update`] | [`route_update::RouteUpdate`] — growing the route sets between iterations: `none` and `best_response` |
//! | [`route_cache`] | [`route_cache::RouteSetCache`] — generated route sets kept for the next run that asks for the same |
//! | [`link_times`] | [`link_times::LinkTimes`] — link travel times by time of day from the last loading, and the change between two loadings |
//! | [`link_values`] | [`link_values::LinkValues`] — values a user gives per link, summed along each alternative for the choice models |
//! | [`prices`] | [`prices::Prices`] — what an alternative costs in euros (running cost, tolls, parking fees, fares), offered to the choice models |
//! | [`loading_rules`] | [`loading_rules::LoadingOptions`] — the loading's rules by name: turn pockets, en-route rerouting, priority at merges; level 1's volume-delay function and period |
//! | [`static_run`] | Flow level 1 in a run: the static user-equilibrium assignment where the other levels make their route sets, its results in the run's shape |
//! | [`demand_values`] | [`demand_values::DemandValues`] — values a user gives per traveller and per trip, offered to the choice models |
//! | [`disruptions`] | [`disruptions::Disruptions`] — roads closed and transit stopped at a time of day, known or not in advance |
//! | [`skims`] | [`skims::Skimmer`] — door-to-door times between points by mode, after a run |
//! | [`events`] | [`events::EventRow`] — one row per trip outcome, what `io-parquet`'s `events.parquet` writer reads from |
//! | [`gap`] | [`gap::GapTrips`] — the full gap: every trip's choice against the best alternative, after the last loading or every one, in the choice model's currency |
//! | [`identity`] | [`identity::RunDescription`] — a run's master seed and fingerprint, made before it executes |
//! | [`timings`] | [`timings::Timings`] — wall-clock time by stage, kept apart from the results |
//!
//! # What a run does
//!
//! It builds the travellers, prepares the layers, the timetable and the parkings, makes the
//! car route sets, then repeats choice (routes; itineraries and modes) and a loading of the road
//! network (cars and buses together) until the equilibration stops, and assembles the results.
//! Walking and cycling have static costs: no interaction with traffic.
//!
//! **[`run::RunResult::total_travel_time`] is *traveller-weight-scaled***
//! (population total, `Σ w · travel_time` over completed trips), not the raw
//! sum over simulated travellers; the
//! people the completed trips stand for are counted beside it, so a mean per
//! person is one division.

pub mod demand_values;
pub mod disruptions;
pub mod equilibration;
pub mod events;
pub mod gap;
pub mod generalised_cost;
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
pub mod services;
pub mod skims;
pub mod static_run;
pub mod timings;
pub mod transit;
pub mod trees;
pub mod trip_results;

pub use demand_values::{DEPARTURE_ATTRIBUTE, DESTINATION_PARKING, DemandValues};
pub use disruptions::{Disruptions, RoadDisruption, TransitDisruption, TransitEffect};
pub use equilibration::{Equilibration, FrankWolfe, FreeFlow, IterationReport, Msa, Sra};
pub use events::{EventRow, EventType};
pub use gap::GapTrips;
pub use identity::RunDescription;
pub use itinerary::{Itinerary, LegRoute, Networks};
pub use itinerary_choice::{
    ATTRIBUTES, ItineraryResult, NO_MODE, NO_PARKING, NoAlternative, NoAlternativeReason,
    TimetableSearches,
};
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
pub use trip_results::{TripOutcome, TripOutcomes, TripTable};
