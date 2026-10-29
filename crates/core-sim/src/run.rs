//! The `Run`: orchestration, trip completion statistics, driven
//! by an event queue in seconds.
//!
//! One run: `core-demand`'s travellers and trips, a route set
//! for every origin-destination pair (`core-routes`), a route per trip
//! from a choice model (`core-choice`), a loading by the chosen
//! [`FlowMotor`], and — under an [`Equilibration`] — choice and loading
//! repeated, with the route sets grown between iterations by a
//! [`RouteUpdate`] and a convergence report per iteration. That is
//! for car trips; **bike and walk trips** travel on their own static layer
//! ([`crate::layers`]), routed once and never part of the car's choice or
//! gap; **transit, park-and-ride and bike-and-ride trips** choose an itinerary
//! ([`crate::itinerary_choice`], M4) on expected costs before each loading (transit on
//! the timetable, always) and execute it on the loading's times after it: the car of a
//! park-and-ride trip is
//! in the loading with every other car, and parks at the hub it chose
//! ([`crate::parking`]); none of them is part of the car routes' choice or gap.
//!
//! The defaults here are the core's own and unconditional (`Level0`,
//! `deterministic`, `none`); the Python `Scenario` layers its own on top.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::sync::Arc;
use std::time::Instant;

use openmobisim_core_choice::{ChoiceError, ChoiceModel, Deterministic};
use openmobisim_core_demand::{Mode, Travellers, Trips, VehicleKind, VehicleLocations};
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_loading::{
    Chain, EntryTables, FidelityLevel, LinkBins, LockReport, Recording, Reroute, RerouteRecord,
    RerouteRule, Rules, Trajectory, Vehicle, load_level_0_binned, load_level_0_recorded,
    load_timed_binned, run_ltm_chained, traverse_free_flow, traverse_timed,
};
use openmobisim_core_routes::{
    ClassExtras, Demand, ExtrasRequest, RouteKey, RouteSetGenerator, RouteSets, TripDemand,
    TripSnapper, default_generator,
};
use openmobisim_core_types::diagnostics::{
    Category, DiagKey, Diagnostics, ElementRef, Severity, codes,
};
use openmobisim_core_types::ids::{
    EntityId, EntityKind, LinkId, NodeId, TravellerId, TripId, VehicleId,
};
use openmobisim_core_types::rng::{DrawAddress, RngKey, Stream, StreamRng};
use openmobisim_core_types::time::{EventKey, Second};
use openmobisim_core_types::units::{Duration, Pcu};

use crate::demand_values::DemandValues;
use crate::disruptions::Disruptions;
use crate::equilibration::{Equilibration, FreeFlow, IterationReport};
use crate::events::{EventRow, EventType};
use crate::gap::{Currency, Ended, GapTrips, NO_LINK};
use crate::generalised_cost::{LinkTerms, RouteCost, class_extras, road_seconds};
use crate::identity::{Inputs, RunDescription, describe};
use crate::itinerary_choice::{
    self, CarReach, CarRoutes, ChooseInputs, Chosen, DockIndex, Followed, GapPlans, HubDocks,
    Itineraries, ItineraryResult, NoAlternative, Planner, Shape, ShareContext, Simulated, WalkEnd,
    parking_kind_of,
};
use crate::layers::{
    ClassLimits, ModeDefaults, StaticLayers, StaticRoute, StaticRoutes, class_limits,
    static_layer_of,
};
use crate::link_times::{LinkTimes, relative_time_change};
use crate::link_values::{LinkValues, PreparedLinkValues};
use crate::loading_rules::LoadingOptions;
use crate::parking::{
    self as parking_mod, ExpectedAvailability, ParkingEvent, ParkingResult, ParkingSetup,
};
use crate::prices::{PreparedPrices, Prices};
use crate::reroute::Rerouter;
use crate::route_cache::{RouteSetCache, generation_key};
use crate::route_choice::{self, NO_ROUTE, RouteChoices};
use crate::route_update::{NoRouteUpdate, RouteUpdate, UpdateContext, make_room, make_room_by};
use crate::services::costs::DockCosts;
use crate::services::engine::Leg;
use crate::services::in_run::{ServicesRun, ShareDay, ShareLeg};
use crate::services::{ServiceKind, ServiceSetup};
use crate::timings::{Stage, Timings};
use crate::transit::{TransitResult, TransitSetup, par_map};
use crate::trees::{SearchTrees, TreeCost};
use crate::trip_results::TripOutcomes;
use openmobisim_core_graph::hubs::ParkingKind;
use openmobisim_core_graph::layers::StaticLayer;
use openmobisim_core_routes::{Reach, Route, SearchContext};
use openmobisim_core_transit::{JourneyLeg, Raptor};

/// Which loading engine a [`Run`] uses.
///
/// **Opt-in: [`Run::new`] loads at level 0 unless a caller asks for the LTM** via
/// [`Run::with_flow_motor`]. `Level0` stays available on purpose, as the cheap,
/// interaction-free debugging rung the fidelity ladder names it.
#[derive(Clone, Debug, Default)]
pub enum FlowMotor {
    /// Level 0: each vehicle traverses independently at free-flow time. [`Run::new`]'s
    /// default.
    #[default]
    Level0,
    /// The iterative LTM (`core_loading::ltm`) — vehicles genuinely
    /// interact through shared link curves and the node model. What both
    /// shipped presets eventually run (`level` = `Full`).
    Ltm {
        /// The network's turn table (built once, outside `Run`, since it
        /// needs [`openmobisim_core_graph::defaults::SignalDefaults`], which
        /// `Run` itself has no other reason to carry).
        turns: Arc<TurnTable>,
        /// The loading step (300 s by default).
        step: Duration,
        /// Which term of the triangular diagram is in force.
        level: FidelityLevel,
    },
    /// Level 1 (S294): the static user-equilibrium assignment, one flow per link for the whole
    /// period, link times by the volume-delay function ([`crate::LoadingOptions::bpr`]), the
    /// solver by the run's equilibration strategy (`frank_wolfe`, else the method of successive
    /// averages); see [`crate::static_run`]. Car trips only: transit, parkings, mode choice
    /// and disruptions the travellers do not know of are refused.
    Static {
        /// The network's turn table, as for [`FlowMotor::Ltm`].
        turns: Arc<TurnTable>,
        /// The period the demand's hourly flows are of, in seconds (at least 1): a link's flow
        /// is the PCU routed over it over this period in hours.
        period_s: u32,
    },
}

/// Diagnostic codes this module records, beyond the core codes it reuses
/// (`codes::NO_FEASIBLE_PATH`, `codes::TRIP_TRUNCATED`).
pub mod local_codes {
    use openmobisim_core_types::diagnostics::DiagCode;

    /// A trip that states a mode needing a vehicle (a car or a bike) found its
    /// traveller's vehicle not at its origin, or the traveller owns none. A trip
    /// that chooses its mode is offered only the modes its traveller can use.
    pub const NO_VEHICLE_AVAILABLE: DiagCode = DiagCode("no_vehicle_available");
    /// The trip's mode cannot be simulated by this run: what it needs is not
    /// there (a bike or walk layer, a timetable for transit and its
    /// combinations, parkings for park-and-ride and bike-and-ride).
    pub const MODE_NOT_AVAILABLE: DiagCode = DiagCode("mode_not_available");
    /// More user classes have their own coefficients on a route's link-additive terms than the
    /// run searches by (`max_cost_vectors`): these classes' routes are searched by the nearest
    /// kept class's by value of time ([`crate::generalised_cost::RouteCost`]); one per class.
    pub const COST_VECTORS_GROUPED: DiagCode = DiagCode("cost_vectors_grouped");
}

/// Trip-level outcomes across a run.
///
/// `total_trips` is every trip in the demand; `no_vehicle_available`,
/// `no_feasible_path` and `mode_not_available` are trips that never travel;
/// `completed` and `truncated` are the statistics proper, over trips that
/// *did* set off.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct TripCompletionStats {
    /// Every trip in the demand.
    pub total_trips: u32,
    /// The trip's stated mode needs a vehicle its traveller does not have at the origin.
    pub no_vehicle_available: u32,
    /// A car was available, but no path existed to the destination.
    pub no_feasible_path: u32,
    /// Arrived within the simulation window.
    pub completed: u32,
    /// Still in progress when the window ended.
    pub truncated: u32,
    /// The trip's mode cannot be simulated by this run: what it needs
    /// (a layer, a timetable, parkings) is missing.
    pub mode_not_available: u32,
}

impl TripCompletionStats {
    /// Trips that actually entered the network: `completed + truncated`.
    #[must_use]
    pub fn attempted(&self) -> u32 {
        self.completed + self.truncated
    }

    /// `completed / attempted`, among trips that entered the network.
    ///
    /// `1.0` if none did — vacuously, nothing was left incomplete — rather
    /// than a division by zero.
    #[must_use]
    pub fn completion_rate(&self) -> f64 {
        let attempted = self.attempted();
        if attempted == 0 { 1.0 } else { f64::from(self.completed) / f64::from(attempted) }
    }
}

/// One mode's outcomes in a run: its trips' completion counts and
/// their total travel time, weighted by traveller weight as
/// [`RunResult::total_travel_time`] is.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct ModeTotals {
    /// The mode's trips' outcomes.
    pub completion: TripCompletionStats,
    /// Total travel time of its completed trips, weighted.
    pub total_travel_time: Duration,
    /// Its trips weighted by their travellers' weights: the trips they stand for (M5, for
    /// the mode share).
    pub weighted_trips: f64,
    /// Its completed trips weighted the same way: the people who arrived.
    pub weighted_completed: f64,
    /// Its trips still under way at the end, weighted the same way.
    pub weighted_truncated: f64,
    /// Its trips that were never simulated (no vehicle, no feasible path, the mode not
    /// available), weighted the same way: with the completed and the truncated, all of
    /// its people.
    pub weighted_not_run: f64,
}

/// Why a run could not finish.
#[derive(Clone, PartialEq, Debug)]
pub enum RunError {
    /// The choice model could not choose: it needs an attribute routes do
    /// not carry, it failed, or its answer does not fit.
    Choice(ChoiceError),
    /// An input does not fit the run: a link value's column is not as long as its
    /// layer.
    Input(String),
}

impl core::fmt::Display for RunError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Choice(e) => write!(f, "{e}"),
            Self::Input(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for RunError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Choice(e) => Some(e),
            Self::Input(_) => None,
        }
    }
}

impl From<ChoiceError> for RunError {
    fn from(e: ChoiceError) -> Self {
        Self::Choice(e)
    }
}

/// What a run produced: its results, the statistics that make it
/// honest about what it did and did not simulate, and the per-trip events
/// `io-parquet`'s `events.parquet` writer reads from.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct RunResult {
    /// Total travel time, summed over completed trips, weighted by
    /// traveller weight (a simulated traveller represents *w* people).
    /// **An assumption, not a settled decision** — see the crate docs.
    pub total_travel_time: Duration,
    /// The completed trips' time in their own vehicles (the drives and the bike rides), weighted
    /// likewise: `total_travel_time` less the fixed times, the walks, the waits and the rides.
    pub total_vehicle_time: Duration,
    /// The completion statistics.
    pub completion: TripCompletionStats,
    /// One row per trip outcome, unsampled — `io-parquet`'s writer applies
    /// the "sampled at 1% by default" at write time, since
    /// sampling is a property of the output format, not of what a run
    /// tracks internally.
    pub events: Vec<EventRow>,
    /// Per-link, per-time-bin results, when asked for with
    /// [`Run::with_link_bins`]: every link traversal that finished
    /// inside the window, including those of trips still under way when it
    /// ended.
    pub link_bins: Option<LinkBins>,
    /// The route sets the trips were routed from: every alternative the
    /// method found for every origin-destination pair the demand asked for.
    /// Each trip takes the route the choice model picks from its pair's set.
    pub route_sets: Option<RouteSets>,
    /// Which route each trip took, out of how many, and how likely the model
    /// found it.
    pub route_choices: Option<RouteChoices>,
    /// The bike route sets (S298), with [`Run::with_bike_sampled`]; `None` otherwise.
    pub bike_route_sets: Option<RouteSets>,
    /// Each stated bike trip's route among its set (S298), with [`Run::with_bike_sampled`];
    /// `None` otherwise.
    pub bike_route_choices: Option<RouteChoices>,
    /// What each iteration showed: one report for a run without
    /// equilibration, one per loading under `msa`. `route_choices`, `events`,
    /// `link_bins`, the completion counts and the total travel time are those of
    /// the last; with disruptions travellers did not know of, those of the one more loading
    /// made with them, which no report here describes.
    pub iterations: Vec<IterationReport>,
    /// Whether the run met its strategy's stopping rule by its last loading (under `msa`,
    /// the three-iteration mean disequilibrium under `gap_tolerance` with the sets grown; before,
    /// only a run that stopped early counted, not one that met the rule at its last allowed
    /// iteration). Always false with a `gap_tolerance` of 0, or under `free_flow`.
    pub converged: bool,
    /// The outcomes by mode, in [`Mode::ALL`] order; with [`Self::no_mode`] they add up
    /// to [`Self::completion`] and [`Self::total_travel_time`].
    pub by_mode: [ModeTotals; Mode::COUNT],
    /// What the run's services did (S302), kept in memory: `None` in a run without services.
    pub services: Option<ServicesRun>,
    /// The trips that chose their mode and had none to choose (counted under the car
    /// before, a trip's stated mode).
    pub no_mode: ModeTotals,
    /// The trips that had no alternative and travelled by car, in trip order:
    /// counted under the car in [`Self::by_mode`], listed here.
    pub fallback_car: Vec<TripId>,
    /// The trips the last choice left with no alternative, in trip order,
    /// with why each mode gave them none; those that then travelled by car
    /// ([`crate::ModeDefaults::fallback_car`]) among them.
    pub no_alternative: Vec<NoAlternative>,
    /// The bike layer's per-link, per-time-bin results, when per-link results
    /// were asked for and some bike trip ran; `pcu` there counts
    /// travellers.
    pub bike_link_bins: Option<LinkBins>,
    /// The walk layer's, likewise.
    pub walk_link_bins: Option<LinkBins>,
    /// What transit did, if the run has a timetable: the times the runs
    /// kept and who boarded and alighted where.
    pub transit: Option<TransitResult>,
    /// What the parkings did, if the run has parkings.
    pub parking: Option<ParkingResult>,
    /// Each itinerary trip's choice and how it went, if the run has any.
    pub itineraries: Option<ItineraryResult>,
    /// What stood still when the last loading's window ended: the vehicles that did not
    /// finish, where they are, and the closed loops of links waiting on one another, with the
    /// check on the loading's guarantee ([`LockReport::room_waits_with_room`], always 0).
    /// `None` at level 0, where nothing waits.
    pub gridlock: Option<LockReport>,
    /// Every reroute of the last loading, in the order they happened; a car's vehicle id
    /// is its trip's index. Empty without rerouting.
    pub reroutes: Vec<RerouteRecord>,
    /// The **realised route** of each trip that re-routed in the last loading and arrived, by
    /// trip: the links it took. Every other trip followed its planned route (its route choice);
    /// nothing is kept for those.
    pub routes_realised: Vec<(TripId, Vec<LinkId>)>,
    /// Per link, the most PCU it held at once in the last loading; `None` at level 0.
    /// Above the link's storage under the point queue: where the queue would have spilled back.
    pub link_peak_pcu: Option<Vec<f64>>,
    /// Every trip's outcome in the last loading, by trip index: what became of it, when it
    /// arrived, and the road link of a car still travelling at the window's end
    /// ([`crate::TripTable`] makes the per-trip table of them).
    pub trip_outcomes: TripOutcomes,
    /// The full gap's numbers of every trip (S293, [`crate::gap`]) at the last loading a report
    /// describes; `None` for a run of one loading, which measures no gap.
    pub gap_trips: Option<GapTrips>,
}

/// How one loading is made besides the routes it follows.
#[derive(Clone, Copy)]
struct LoadPlan<'a> {
    /// Record per-link results in bins of this many seconds, if asked.
    record_bins: Option<u32>,
    /// Also record the entry-time tables the next iteration's costs are read from.
    want_entry: bool,
    /// Load with this level of the link transmission model instead of the run's (the
    /// warm-up's point-queue model).
    level_override: Option<FidelityLevel>,
    /// Load only the travellers marked here, by traveller (the free-flow loading's
    /// increments); everyone if `None`.
    active: Option<&'a [bool]>,
    /// Whether this loading has the run's disruptions.
    disrupted: bool,
}

/// One loading of the demand.
struct Loaded {
    total_travel_time: Duration,
    /// The completed trips' seconds in their own vehicles (drives and bike rides), weighted.
    total_vehicle_time: Duration,
    /// Per completed trip with a vehicle of its own: its seconds in it and its fixed seconds.
    trip_parts: Vec<(TripId, u32, u32)>,
    completion: TripCompletionStats,
    events: Vec<EventRow>,
    /// Traffic by the bin it left each link in: what a flow map draws.
    link_bins: Option<LinkBins>,
    /// Link times by the bin of entry and waits at origins by the bin of departure:
    /// what costs the next iteration's routes (only recorded when there is one).
    entry_bins: Option<EntryTables>,
    /// The bike and walk layers' vehicles and the bin length, if per-link results were asked
    /// for: binned only for the loading whose results the run keeps ([`Run::layer_bins`]), not
    /// after every loading (S294).
    layer_vehicles: Option<(u32, [Vec<Vehicle>; 2])>,
    /// What transit did, if the run has a timetable.
    transit: Option<TransitResult>,
    /// What the parkings did, and their realised availability per parking and bin.
    parking: Option<ParkingResult>,
    availability: Option<Vec<f64>>,
    /// How the itineraries went.
    itinerary: ItineraryTally,
    /// What stood still when the window ended (the link transmission model only).
    gridlock: Option<LockReport>,
    /// Every reroute, and the realised routes of the trips that re-routed and arrived.
    reroutes: Vec<RerouteRecord>,
    routes_realised: Vec<(TripId, Vec<LinkId>)>,
    /// How many reroute offers (searches) the loading made.
    reroute_searches: u32,
    /// Per link, the most PCU it held at once; `None` at level 0.
    peak_pcu: Option<Vec<f64>>,
    /// The road link of each car trip still travelling when the window ended.
    at_end: Vec<(TripId, LinkId)>,
    /// The shared-bike trips' engine day (S302), if any trip took a shared bike.
    share: Option<ShareDay>,
}

/// How the itinerary trips of a loading went.
#[derive(Clone, Debug, Default)]
struct ItineraryTally {
    /// Per itinerary trip (by position): vehicles boarded.
    rides: Vec<u32>,
    /// Trips whose chosen line could not be followed.
    replanned: u32,
    /// Σ |realised − expected| arrival at the parking of trips back, and how many.
    mismatch_sum: f64,
    mismatch_count: u32,
    /// Σ w · (the second a transit part ended − the second it would have on the timetable
    /// planned on) over the trips whose transit part arrived, and Σ w (S299).
    plan_error_sum: f64,
    plan_error_weight: f64,
}

/// A shared-bike leg's ride on the bike layer, and, for a shared bike to transit, its place
/// among the loading's pending itineraries (S302).
type ShareRest = (Vec<LinkId>, Option<usize>);

/// An itinerary trip waiting for the loading.
struct PendingItin {
    trip: TripId,
    position: usize,
    /// When its vehicle reached the parking (out) or the destination (back), if
    /// it did; unused for plain transit.
    vehicle_arrival: Option<f64>,
}

/// How an itinerary trip's transit part went.
enum ItinOutcome {
    Arrived(u32),
    Truncated,
    NoPath,
}

/// What executing one itinerary trip produced.
struct Executed {
    outcome: ItinOutcome,
    followed: Option<Followed>,
    /// Back: |realised − expected| arrival at the parking.
    mismatch: Option<f64>,
    /// Its transit part's end less where it would have ended on the timetable planned on,
    /// followed the same way from the same moment at the first stop (S299); 0 when the
    /// loading ran that timetable; `None` if either way did not get there.
    plan_error: Option<f64>,
    /// Walk legs to bin: links and the second they start.
    walks: Vec<(Vec<LinkId>, u32)>,
    /// The seconds in the trip's own vehicle (the drive or the ride), and its fixed seconds
    /// (reaching the vehicle, leaving it, the parking's time); 0 and 0 without one.
    parts: (u32, u32),
}

/// Where [`Run::start_itinerary`] puts what it starts.
struct ItinStart<'a> {
    pending: &'a mut Vec<PendingItin>,
    cars: &'a mut Vec<(usize, Vehicle)>,
    parking_events: &'a mut Vec<ParkingEvent>,
    level0_vehicles: &'a mut Vec<Vehicle>,
    bike_vehicles: &'a mut Vec<Vehicle>,
    completion: &'a mut TripCompletionStats,
    events: &'a mut Vec<EventRow>,
}

impl Run {
    /// The whole seconds after a car trip's departure its car enters the roads: `car_access_s`.
    /// The loading starts the car there, and every reading of a car route's cost at its
    /// departure (the route choice, the search trees, the route update, the itinerary planner,
    /// the gap) starts there too.
    fn car_start_s(&self) -> u32 {
        whole_seconds(self.mode_defaults.car_access_s)
    }
}

/// `x` seconds as the loading's whole seconds (0 for a negative or absent value).
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "whole seconds")]
fn whole_seconds(x: f64) -> u32 {
    x.max(0.0).round() as u32
}

/// The vehicle kind of a parking kind.
fn vehicle_kind(kind: ParkingKind) -> VehicleKind {
    match kind {
        ParkingKind::Car => VehicleKind::Car,
        ParkingKind::Bike => VehicleKind::Bike,
    }
}

/// What happened to one bike or walk trip.
enum StaticOutcome {
    /// The mode cannot be simulated by this run.
    NotAvailable,
    /// The traveller's bike was not at the origin.
    NoVehicle,
    /// The layer has no route.
    NoPath,
    /// Origin and destination are one node.
    Here,
    /// The trip travelled on its layer (slot 0 bike, 1 walk).
    Travelled { vehicle: Vehicle, trajectory: Trajectory, slot: usize },
}

/// One simulation run: immutable shared inputs plus the mutable state
/// a `Run` owns — here, just vehicle locations,
/// since there are no curves, hubs or choice state yet.
pub struct Run {
    network: Arc<RoadNetwork>,
    travellers: Arc<Travellers>,
    trips: Arc<Trips>,
    vehicles: VehicleLocations,
    /// Trips still in progress after this second are truncated.
    window: Second,
    /// The window's start: the loading's clock starts here; 0 (midnight) unless set.
    window_start: Second,
    flow_motor: FlowMotor,
    /// Length of the time bins of the per-link results, if asked for.
    link_bin_seconds: Option<u32>,
    /// How route sets are generated: the penalty method by default.
    route_generator: Arc<dyn RouteSetGenerator>,
    /// The scenario's master seed: the one number that starts every
    /// random stream (the choice draws and `msa`'s reselection); see [`RunDescription`].
    master_seed: u64,
    /// How each trip picks a route from its pair's set: all-or-nothing on
    /// the best route by default, so a run is what it was before choice existed.
    choice_model: Arc<dyn ChoiceModel>,
    /// How choice and loading are repeated: once, by default.
    equilibration: Arc<dyn Equilibration>,
    /// How the route sets grow between iterations: not at all, by default.
    route_update: Arc<dyn RouteUpdate>,
    /// Bike route sets (S298): the sampled generator that makes them on the bike layer, with
    /// each class's extras for the bike's attributes; `None` (the default) routes every bike trip
    /// by one shortest route under the bike cost, as before.
    bike_sampled: Option<openmobisim_core_routes::Sampled>,
    /// A traveller is offered only the routes whose expected time is within this share of the
    /// best's; 0 offers them all. [`DEFAULT_CHOICE_DETOUR_LIMIT`] by default.
    choice_detour_limit: f64,
    /// A traveller is offered at most this many routes, the fastest at their departure.
    /// [`DEFAULT_CHOICE_MAX_ROUTES`] by default.
    choice_max_routes: u32,
    /// The most distinct coefficient vectors the route searches are made by (S299):
    /// [`crate::generalised_cost::DEFAULT_MAX_COST_VECTORS`] by default.
    max_cost_vectors: u32,
    /// Generated route sets kept for the next run that asks for the same: none, by default.
    route_cache: Option<Arc<RouteSetCache>>,
    /// The bike and walk layers: none, by default.
    layers: Arc<StaticLayers>,
    /// The timetable, linked to the walk and bike layers: none, by default.
    transit: Option<Arc<TransitSetup>>,
    /// The parkings, linked to the layers and stops: none, by default.
    parking: Option<Arc<ParkingSetup>>,
    /// The shared-mobility services (S302): none, by default.
    services: Option<Arc<Vec<ServiceSetup>>>,
    /// How far a dock may be from a stop to join its hub, in metres (S302): read only with
    /// services.
    hub_join_m: f64,
    /// The modes a trip without a stated mode chooses among: no choice, by default.
    mode_choice: Option<Vec<Mode>>,
    /// How long a walk or ride mode choice offers: the shipped values, by default.
    mode_defaults: ModeDefaults,
    loading: LoadingOptions,
    /// Wall-clock time by stage of the last execution.
    timings: Timings,
    /// How many threads the run's parallel work uses: all logical cores, by default.
    threads: Option<std::num::NonZeroUsize>,
    /// The modes each traveller class may use, by class index; empty: every class all.
    class_modes: Vec<[bool; Mode::COUNT]>,
    /// Each traveller class's own choice-set limits, by class index; empty: the run's.
    class_limits: Vec<ClassLimits>,
    /// The user's link values, offered to choice models as attributes: none, by default.
    link_values: LinkValues,
    /// The last loading's link times, kept after a run for skims: `None` before one, or
    /// for a run of one loading, which records none.
    final_times: Option<Arc<LinkTimes>>,
    /// Disruptions at a time of day: none, by default.
    disruptions: Disruptions,
    /// The prices alternatives are costed at: the shipped values, by default.
    prices: Prices,
    /// The user's traveller and trip values, offered to choice models: none, by default.
    demand_values: DemandValues,
    /// The walks of itinerary trips' legs found so far in an execution, by (from, to) walk node:
    /// the walk layer is static, so each is searched once per execution, not once per
    /// loading. `None`: no walk within the bound.
    walk_paths: WalkPaths,
    /// Level 1's equilibrium for the execution under way (S294): what its one loading drives
    /// on. `None` at every other level.
    static_solution: Option<Arc<crate::static_run::StaticSolution>>,
    /// What re-routing searches by in the execution under way (S299): `None` where the route
    /// cost is time alone.
    reroute_cost: Option<Arc<crate::reroute::RerouteCost>>,
}

/// Walk paths by (from, to) walk node: see [`Run`]'s `walk_paths`.
type WalkPaths = HashMap<(u32, u32), Option<Box<[LinkId]>>>;

/// A walk leg to bin: from and to walk node, and the second it starts.
type WalkLeg = (u32, u32, u32);

/// The key of the draw that puts a traveller in a group of the free-flow loading's increments,
/// apart from every other draw on the re-selection stream: who chooses again at
/// iteration `i` (keys 1 to `MAX_ITERATIONS`), the itinerary gap's sample (from `1 << 31`) and
/// the network gap's (`u32::MAX`, keyed on trips).
const INCREMENT_KEY: u32 = u32::MAX - 1;

/// The group (0 to `increments − 1`) of the free-flow loading `traveller` is in: a draw
/// keyed on the traveller alone, so groups are the same for any thread count and order, and
/// about equal in size.
fn increment_of(rng: &StreamRng, traveller: u32, increments: u32) -> u32 {
    let u = rng.unit(DrawAddress::from_pair(traveller, INCREMENT_KEY));
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a draw in [0, 1) times a small count"
    )]
    let group = (u * f64::from(increments)) as u32;
    group.min(increments - 1)
}

/// The share above the best route's expected time beyond which a route is not offered to a
/// traveller, unless the run says otherwise. 0 offers every route
/// of the pair's set. At 0.5 a route is offered only if it is at most 50% slower than the best;
/// every built-in method's routes are within 30% (`penalty`) or 100% (`montecarlo`) of the best
/// at free flow, so a run that does not iterate changes only for `montecarlo`, and an iterating
/// one drops the routes congestion has made much slower than the best.
pub const DEFAULT_CHOICE_DETOUR_LIMIT: f64 = 0.5;

/// The most routes a traveller is offered by default: the fastest five
/// at their departure, of those within the detour limit. *An assumption* on the size of a
/// consideration set: Prato (2009), "Route choice modeling: past, present and future research
/// directions", *Journal of Choice Modelling* 2(1), 65-100,
/// <https://doi.org/10.1016/S1755-5345(13)70005-8>, reviews choice sets of a handful of routes. The
/// sets themselves may hold more (`BestResponse::max_routes`): a route not offered now can be later.
pub const DEFAULT_CHOICE_MAX_ROUTES: u32 = 5;

impl Run {
    /// Build a run. Vehicle locations are seeded so that every owned car
    /// starts at its traveller's first trip's origin. Uses
    /// [`FlowMotor::Level0`] — call [`Self::with_flow_motor`] for the LTM.
    #[must_use]
    pub fn new(
        network: Arc<RoadNetwork>,
        travellers: Arc<Travellers>,
        trips: Arc<Trips>,
        window: Second,
    ) -> Self {
        let vehicles = VehicleLocations::at_first_trip_origin(&travellers, &trips);
        Self {
            network,
            travellers,
            trips,
            vehicles,
            window,
            window_start: Second(0),
            flow_motor: FlowMotor::default(),
            link_bin_seconds: None,
            route_generator: Arc::from(default_generator()),
            master_seed: 0,
            choice_model: Arc::new(Deterministic),
            equilibration: Arc::new(FreeFlow::default()),
            route_update: Arc::new(NoRouteUpdate),
            bike_sampled: None,
            choice_detour_limit: DEFAULT_CHOICE_DETOUR_LIMIT,
            choice_max_routes: DEFAULT_CHOICE_MAX_ROUTES,
            max_cost_vectors: crate::generalised_cost::DEFAULT_MAX_COST_VECTORS,
            route_cache: None,
            layers: Arc::new(StaticLayers::default()),
            transit: None,
            parking: None,
            services: None,
            hub_join_m: crate::services::hubs::DEFAULT_HUB_JOIN_M,
            mode_choice: None,
            mode_defaults: ModeDefaults::SHIPPED,
            loading: LoadingOptions::SHIPPED,
            timings: Timings::new(),
            threads: None,
            class_modes: Vec::new(),
            class_limits: Vec::new(),
            link_values: LinkValues::new(),
            final_times: None,
            disruptions: Disruptions::default(),
            prices: Prices::SHIPPED,
            demand_values: DemandValues::new(),
            walk_paths: WalkPaths::new(),
            static_solution: None,
            reroute_cost: None,
        }
    }

    /// The same run with disruptions at a time of day ([`crate::disruptions`]): roads
    /// narrowed or closed for a while, a line's runs delayed or cancelled; known to travellers
    /// or not. Checked when the run executes ([`RunError::Input`]).
    #[must_use]
    pub fn with_disruptions(mut self, disruptions: Disruptions) -> Self {
        self.disruptions = disruptions;
        self
    }

    /// The link times of the last loading of the last execution: what a car took on
    /// each link, by time bin, for travel-time matrices after the run
    /// ([`crate::skims::Skimmer`]). `None` before an execution, or after one of a single
    /// loading, which records none (free-flow times stand in); at level 1 the equilibrium's
    /// times, the same all day.
    #[must_use]
    pub fn final_link_times(&self) -> Option<Arc<LinkTimes>> {
        self.final_times.clone()
    }

    /// The same run with the modes each traveller class may use, by class index (the
    /// travellers' [`openmobisim_core_demand::Travellers::user_class`]): a trip choosing its
    /// mode is offered only those its class may use. A class past the end may use all.
    #[must_use]
    pub fn with_class_modes(mut self, class_modes: Vec<[bool; Mode::COUNT]>) -> Self {
        self.class_modes = class_modes;
        self
    }

    /// The same run with each traveller class's own choice-set limits, by
    /// class index as [`Self::with_class_modes`]: the longest walk and ride a trip choosing its
    /// mode is offered, and the longest walk to and from a stop of any of its transit
    /// itineraries. A limit a class does not give, and a class past the end, take the run's.
    /// A class walking to stops further than the timetable's defaults needs a
    /// [`TransitSetup`] searching as far ([`TransitSetup::with_access_search_s`]).
    #[must_use]
    pub fn with_class_limits(mut self, class_limits: Vec<ClassLimits>) -> Self {
        self.class_limits = class_limits;
        self
    }

    /// The same run with the user's link values: each column a value
    /// per link of its layer, offered to choice models as `<layer>_<name>_km` and
    /// `<layer>_<name>_sum` ([`crate::link_values`]). A column must have one value per link
    /// of its layer ([`RunError::Input`] otherwise).
    #[must_use]
    pub fn with_link_values(mut self, link_values: LinkValues) -> Self {
        self.link_values = link_values;
        self
    }

    /// The same run with the user's traveller and trip values, offered
    /// to choice models as `person_<name>` and `trip_<name>` ([`crate::demand_values`]). A column
    /// must have one value per traveller or trip of the run's demand ([`RunError::Input`]
    /// otherwise).
    #[must_use]
    pub fn with_demand_values(mut self, values: DemandValues) -> Self {
        self.demand_values = values;
        self
    }

    /// The same run with `prices`: what driving, riding, parking and
    /// fares cost, offered to choice models as `cost_eur` and its parts ([`crate::prices`]).
    #[must_use]
    pub fn with_prices(mut self, prices: Prices) -> Self {
        self.prices = prices;
        self
    }

    /// Wall-clock time by stage of the last [`Self::execute`] or [`Self::try_execute`],
    /// in the order the stages ended; empty before. Kept apart from [`RunResult`], since it
    /// differs from run to run. The names: [`Timings::lap`].
    #[must_use]
    pub fn timings(&self) -> &[Stage] {
        self.timings.stages()
    }

    /// The same run, with a timetable for the trips whose mode is
    /// [`Mode::Transit`]: they walk to a stop, ride and walk on, by the
    /// journey the choice model picks among the competitive ones. Without it such a trip is counted as
    /// `mode_not_available`. Its walks are counted on the walk layer's per-link
    /// results when that layer is the run's own ([`Self::with_layers`]).
    #[must_use]
    pub fn with_transit(mut self, transit: Arc<TransitSetup>) -> Self {
        self.transit = Some(transit);
        self
    }

    /// Parkings, for trips whose mode is [`Mode::CarTransit`] or
    /// [`Mode::BikeTransit`]: they drive or ride to a parking the choice model
    /// picks, park and go on by transit, or come back by transit to where their
    /// vehicle is. They need the timetable too ([`Self::with_transit`]), and
    /// bike-and-ride the bike layer.
    #[must_use]
    pub fn with_parking(mut self, parking: Arc<ParkingSetup>) -> Self {
        self.parking = Some(parking);
        self
    }

    /// The same run with shared-mobility services (S302): a docked bike service offers the mode
    /// [`Mode::BikeShare`] to the trips choosing their mode, when [`Self::with_mode_choice`]
    /// lists it and the run has the walk and bike layers. Its trips walk to a dock, ride a shared
    /// bike on the bike layer, return it at a dock and walk on; after each loading the fleet
    /// engine plays their legs, and the docks' costs it realises are averaged into what the next
    /// choice expects. One docked bike service per run, and no taxi yet ([`RunError::Input`]
    /// otherwise). An empty list is no service.
    #[must_use]
    pub fn with_services(mut self, services: Arc<Vec<ServiceSetup>>) -> Self {
        self.services = (!services.is_empty()).then_some(services);
        self
    }

    /// The same run with docks joining a stop's hub within `metres` of it in a straight line
    /// (S302; [`crate::services::hubs::DEFAULT_HUB_JOIN_M`] by default): a dock at a stop's hub
    /// is where [`Mode::BikeShareTransit`] returns its shared bike. Read only in a run with
    /// services, so a run without one is what it was.
    #[must_use]
    pub fn with_hub_join_m(mut self, metres: f64) -> Self {
        self.hub_join_m = metres;
        self
    }

    /// The same run, in which **a trip whose mode was not stated chooses one** among
    /// `modes`, with its route, by the run's choice model: the walk, the bike route,
    /// the car's routes, transit journeys, park-and-ride and bike-and-ride, each offered
    /// where the traveller can use it (their car or bike where the trip starts; the layer,
    /// timetable or parkings in the run; park-and-ride and bike-and-ride only beyond
    /// [`crate::parking::ParkingDefaults::pr_min_km`]). A traveller's trips choose in order,
    /// since each may move a vehicle the next needs. A trip with a stated mode keeps it.
    ///
    /// With one mode there is nothing to choose: such trips simply take it (so `[Mode::Car]`
    /// is the run without this call, exactly). With none, the call changes nothing.
    #[must_use]
    pub fn with_mode_choice(mut self, modes: &[Mode]) -> Self {
        let mut modes = modes.to_vec();
        modes.sort_unstable();
        modes.dedup();
        match modes.as_slice() {
            [] => {}
            [one] => {
                self.trips = Arc::new(self.trips.with_unstated_mode(*one));
                self.vehicles =
                    VehicleLocations::at_first_trip_origin(&self.travellers, &self.trips);
                self.mode_choice = None;
            }
            _ => self.mode_choice = Some(modes),
        }
        self
    }

    /// The same run, its loading applying `options`: priority at merges, en-route
    /// rerouting. Only the link transmission model has junctions and queues; level 0 ignores
    /// them.
    #[must_use]
    pub fn with_loading_options(mut self, options: LoadingOptions) -> Self {
        self.loading = options;
        self
    }

    /// The same run, offering mode choice's walk and bike alternatives only up to the
    /// times of `defaults`; a trip given the mode takes it at any length.
    #[must_use]
    pub fn with_mode_defaults(mut self, defaults: ModeDefaults) -> Self {
        self.mode_defaults = defaults;
        self
    }

    /// The same run, with bike and walk layers for the trips whose mode is
    /// [`Mode::Bike`] or [`Mode::Walk`]. Without them such a trip is
    /// counted as `mode_not_available`.
    #[must_use]
    pub fn with_layers(mut self, layers: Arc<StaticLayers>) -> Self {
        self.layers = layers;
        self
    }

    /// The same run, loaded by `flow_motor` instead of the default
    /// [`FlowMotor::Level0`].
    #[must_use]
    pub fn with_flow_motor(mut self, flow_motor: FlowMotor) -> Self {
        self.flow_motor = flow_motor;
        self
    }

    /// The same run, routing its trips from route sets made by `generator`
    /// instead of the default penalty method. Any
    /// [`RouteSetGenerator`] works: select a built-in one by name with
    /// `openmobisim_core_routes::generator`, or pass your own.
    #[must_use]
    pub fn with_route_generator(mut self, generator: Arc<dyn RouteSetGenerator>) -> Self {
        self.route_generator = generator;
        self
    }

    /// The same run, with `master_seed` as its master seed (default 0).
    ///
    /// Every random stream of a run is keyed from this one number, so a run
    /// is reproduced by giving it the same seed. No stochastic step draws from
    /// it yet (the choice layer is the first), so today it changes the run's
    /// [`fingerprint`](RunDescription::fingerprint) and nothing else.
    #[must_use]
    pub fn with_master_seed(mut self, master_seed: u64) -> Self {
        self.master_seed = master_seed;
        self
    }

    /// The same run, choosing each trip's route with `model` instead of the
    /// default all-or-nothing [`Deterministic`]. Select a built-in model
    /// by name with `openmobisim_core_choice::model`, or pass your own
    /// [`ChoiceModel`].
    #[must_use]
    pub fn with_choice_model(mut self, model: Arc<dyn ChoiceModel>) -> Self {
        self.choice_model = model;
        self
    }

    /// The same run, repeating choice and loading under `strategy` instead of
    /// once. Select a built-in one by name with
    /// [`crate::equilibration::strategy`], or pass your own [`Equilibration`].
    #[must_use]
    pub fn with_equilibration(mut self, strategy: Arc<dyn Equilibration>) -> Self {
        self.equilibration = strategy;
        self
    }

    /// The same run, growing its route sets between iterations with `update` instead of
    /// leaving them as the generator made them. Select a built-in one by name with
    /// [`crate::route_update::update`], or pass your own [`RouteUpdate`]. It changes nothing
    /// unless the run iterates (an [`Equilibration`] with more than one loading): the
    /// sets grow *between* loadings.
    #[must_use]
    pub fn with_route_update(mut self, update: Arc<dyn RouteUpdate>) -> Self {
        self.route_update = update;
        self
    }

    /// The same run with **bike route sets** (S298): `generator`, the sampled method, makes a set
    /// of routes per pair on the bike layer (its extras composed from the choice model's
    /// coefficients on the bike's attributes), every stated bike trip chooses its route once
    /// through the choice model on the layer's static times, and a trip choosing its mode is
    /// offered the routes as bike alternatives. Without it every bike trip takes its one
    /// shortest route under the bike cost, as before, and the run hashes as it did.
    #[must_use]
    pub fn with_bike_sampled(mut self, generator: openmobisim_core_routes::Sampled) -> Self {
        self.bike_sampled = Some(generator);
        self
    }

    /// How bike routes are made (S298), with the extras composed for the bike layer; `Shortest`
    /// without [`Self::with_bike_sampled`] or without a bike layer.
    fn bike_routing(&self, link_values: &PreparedLinkValues) -> crate::layers::BikeRouting {
        match (&self.bike_sampled, self.layers.get(StaticLayer::Bike)) {
            (Some(generator), Some(setup)) => {
                let extras = match generator.extras_wanted() {
                    Some(request) => {
                        let terms = LinkTerms::bike(setup.network(), &self.prices, link_values);
                        class_extras(
                            request,
                            self.choice_model.as_ref(),
                            &self.travellers,
                            &terms,
                            setup.seconds(),
                        )
                    }
                    None => Vec::new(),
                };
                crate::layers::BikeRouting::Sampled { generator: generator.clone(), extras }
            }
            _ => crate::layers::BikeRouting::Shortest,
        }
    }

    /// The same run, offering each traveller only the routes of the pair's set whose expected time
    /// (at the times of the last loading; at free flow for the first choice) is within `limit`
    /// of the best route's: **a time-dependent choice set**. A route far slower than
    /// the best at the moment is no realistic alternative, and in a logit it only takes
    /// probability that belongs to routes that compete (the gap grows with the size of the set).
    /// `0.0` offers every route. The best route is always offered; a route dropped
    /// keeps its place in the sets and can return when the times change. The gap is still
    /// measured against the whole set. Where a class's utility weighs more than time (S299), a
    /// route's expected time is read with its extras under the class's coefficient vector: the
    /// limit is on that generalised cost.
    ///
    /// # Panics
    ///
    /// Panics if `limit` is negative or not finite.
    #[must_use]
    pub fn with_choice_detour_limit(mut self, limit: f64) -> Self {
        assert!(
            limit.is_finite() && limit >= 0.0,
            "the detour limit must be 0 or more, got {limit}"
        );
        self.choice_detour_limit = limit;
        self
    }

    /// The same run, offering each traveller at most `n` routes, the fastest at their departure
    /// among those within the detour limit ([`DEFAULT_CHOICE_MAX_ROUTES`] by default; 0
    /// offers every such route), the cheapest in the class's generalised cost where its utility
    /// weighs more than time (S299). The gap is still measured against the whole set.
    #[must_use]
    pub fn with_choice_max_routes(mut self, n: u32) -> Self {
        self.choice_max_routes = n;
        self
    }

    /// The same run, its route searches made by at most `n` distinct coefficient vectors (S299,
    /// [`crate::generalised_cost::RouteCost`]): user classes whose coefficients on a route's
    /// link-additive terms (length, money, tolls, user link values) differ each get their own
    /// search trees, up to `n`; beyond, a class is searched by the nearest kept class's by value
    /// of time, and the run records it (`cost_vectors_grouped`). Each vector costs about one set
    /// of search trees per loading. In the fingerprint only where a class's cost is not time
    /// alone.
    ///
    /// # Panics
    ///
    /// Panics if `n` is 0.
    #[must_use]
    pub fn with_max_cost_vectors(mut self, n: u32) -> Self {
        assert!(n >= 1, "a run searches by at least one coefficient vector");
        self.max_cost_vectors = n;
        self
    }

    /// The static layers' search costs from the choice model (S299): the bike layer's where its
    /// bike cost is [`openmobisim_core_graph::layers::BikeCost::Utility`], the walk layer's where
    /// the walk utility weighs more than time; each the travel time plus the extras of the one
    /// coefficient vector serving the most people (a layer's routes are one per pair, shared by
    /// every class). Returns them and how many classes were grouped onto another's vector.
    fn utility_costs(
        &self,
        link_values: &PreparedLinkValues,
    ) -> (crate::layers::UtilityCosts, usize) {
        let mut grouped = 0;
        let mut compose = |setup: &crate::layers::LayerSetup, terms: &LinkTerms| {
            let cost = RouteCost::compose(
                self.choice_model.as_ref(),
                &self.travellers,
                terms,
                setup.seconds(),
                1,
            );
            grouped += cost.grouped().len();
            cost.extras(0).map(|e| {
                setup.seconds().iter().zip(e.iter()).map(|(s, x)| s + x).collect::<Vec<f64>>()
            })
        };
        let bike = self
            .layers
            .get(StaticLayer::Bike)
            .filter(|b| b.cost() == openmobisim_core_graph::layers::BikeCost::Utility)
            .and_then(|b| compose(b, &LinkTerms::bike(b.network(), &self.prices, link_values)));
        let walk = self
            .layers
            .get(StaticLayer::Walk)
            .and_then(|w| compose(w, &LinkTerms::walk(w.network(), link_values)));
        (crate::layers::UtilityCosts { bike, walk }, grouped)
    }

    /// The road's route cost for car searches (S299): composed from the choice model's
    /// coefficients, the prices and the user's road columns, floored on the free-flow times.
    fn road_cost(&self, link_values: &PreparedLinkValues) -> RouteCost {
        let terms = LinkTerms::road(&self.network, &self.prices, link_values);
        RouteCost::compose(
            self.choice_model.as_ref(),
            &self.travellers,
            &terms,
            &road_seconds(&self.network),
            self.max_cost_vectors as usize,
        )
    }

    /// The same run, taking its route sets from `cache` if it holds the sets this run would
    /// generate, and leaving them there if not: see [`RouteSetCache`]. Results do not depend
    /// on whether the cache was used.
    #[must_use]
    pub fn with_route_cache(mut self, cache: Arc<RouteSetCache>) -> Self {
        self.route_cache = Some(cache);
        self
    }

    /// The same run, its parallel work on `threads` threads; `None`, the
    /// default, on every logical core. The run uses a thread pool of its own, not rayon's
    /// global one, so `RAYON_NUM_THREADS` does not change it. **Results do not depend on it**
    /// (reductions in a fixed order, random draws keyed to the seed, the
    /// traveller and the iteration, never to a thread), so it is recorded in the manifest
    /// ([`RunDescription::threads`]) and not in the fingerprint.
    #[must_use]
    pub fn with_threads(mut self, threads: Option<std::num::NonZeroUsize>) -> Self {
        self.threads = threads;
        self
    }

    /// How many threads the run's parallel work uses: [`Self::with_threads`]'s, else every
    /// logical core (1 without the `parallel` feature).
    #[must_use]
    pub fn threads(&self) -> usize {
        if cfg!(feature = "parallel") {
            self.threads.map_or_else(
                || std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get),
                std::num::NonZeroUsize::get,
            )
        } else {
            1
        }
    }

    /// Whether the run's choice model reads money: a model that lists no attributes reads
    /// them all. Only then do the prices shape the run, and its fingerprint records them.
    fn reads_money(&self) -> bool {
        self.choice_model.required_attributes().is_none_or(|names| {
            names.iter().any(|n| crate::prices::COST_ATTRIBUTES.contains(&n.as_str()))
        })
    }

    /// Whether the run has road disruptions the travellers did not know of: at the point queue,
    /// its disrupted loading then re-routes at closures (S300).
    fn unknown_road_disruptions(&self) -> bool {
        !self.disruptions.known && !self.disruptions.capacity_changes().is_empty()
    }

    /// The loading's level: `None` at free flow.
    fn level(&self) -> Option<FidelityLevel> {
        match &self.flow_motor {
            FlowMotor::Ltm { level, .. } => Some(*level),
            FlowMotor::Level0 | FlowMotor::Static { .. } => None,
        }
    }

    /// Extra seconds per link by user class and preference draw (S298), as a method or update
    /// that searches by generalised cost asks for them: composed from the choice model's
    /// coefficients, the prices and the user's `link_values`, floored on the road's free-flow
    /// times. Cheap, and made only when asked.
    fn compose_extras(
        &self,
        request: ExtrasRequest,
        link_values: &PreparedLinkValues,
    ) -> ClassExtras {
        let terms = LinkTerms::road(&self.network, &self.prices, link_values);
        class_extras(
            request,
            self.choice_model.as_ref(),
            &self.travellers,
            &terms,
            &road_seconds(&self.network),
        )
    }

    /// The route method with the extras it asked for, if any (S298), or, for a method searching
    /// by time, made to search by the run's route cost where it is not time alone (S299,
    /// [`openmobisim_core_routes::ByCost`]): its descriptor, and so the run's fingerprint and the
    /// route cache's key, then name them.
    fn keyed_generator(&self, link_values: &PreparedLinkValues) -> Arc<dyn RouteSetGenerator> {
        match self.route_generator.extras_wanted().map(|r| self.compose_extras(r, link_values)) {
            Some(extras) => self
                .route_generator
                .with_extras(extras)
                .map_or_else(|| self.route_generator.clone(), Arc::from),
            None => {
                let cost = self.road_cost(link_values);
                if cost.is_time_only() {
                    self.route_generator.clone()
                } else {
                    Arc::new(openmobisim_core_routes::ByCost::new(
                        self.route_generator.clone(),
                        cost.vectors().to_vec(),
                    ))
                }
            }
        }
    }

    /// What went into this run: its seed and its fingerprint. Take it
    /// before [`Self::execute`]; it depends only on the inputs.
    #[must_use]
    pub fn description(&self) -> RunDescription {
        let read = self.choice_model.required_attributes();
        let reads = |name: &str| read.as_ref().is_none_or(|names| names.iter().any(|n| n == name));
        // The method as the run will use it, its extras named (S298): the fingerprint and the
        // manifest say what the sets were made with. Link values that the run would refuse give
        // the method without extras; the run then fails on them before anything else.
        let (generator, bike_routing, road_cost) =
            match self.link_values.prepare(&self.network, &self.layers) {
                Ok(values) => (
                    self.keyed_generator(&values),
                    self.bike_routing(&values).descriptor(),
                    self.road_cost(&values),
                ),
                Err(_) => {
                    (self.route_generator.clone(), "shortest".to_string(), RouteCost::time_only())
                }
            };
        let road_cost = (!road_cost.is_time_only())
            .then(|| format!("{};max={}", road_cost.descriptor(), self.max_cost_vectors));
        describe(&Inputs {
            threads: self.threads(),
            network: &self.network,
            travellers: &self.travellers,
            trips: &self.trips,
            window: self.window,
            window_start: self.window_start,
            flow_motor: &self.flow_motor,
            link_bin_seconds: self.link_bin_seconds,
            route_method: generator.name(),
            route_descriptor: &generator.descriptor(),
            master_seed: self.master_seed,
            choice_model: self.choice_model.name(),
            choice_descriptor: &self.choice_model.descriptor(),
            choice_sampled: self.choice_model.is_sampled(),
            equilibration: self.equilibration.name(),
            equilibration_descriptor: &self.equilibration.descriptor(),
            equilibration_draws: self.equilibration.draws_reselection(),
            max_iterations: self.equilibration.max_iterations(),
            route_update: self.route_update.name(),
            route_update_descriptor: &self.route_update.descriptor(),
            route_update_active: self.route_update.is_active(),
            bike_routing: &bike_routing,
            road_cost: road_cost.as_deref(),
            choice_detour_limit: self.choice_detour_limit,
            choice_max_routes: self.choice_max_routes,
            layers: &self.layers,
            transit: self.transit.as_deref(),
            parking: self.parking.as_deref(),
            services: self.services.as_deref().map_or(&[], Vec::as_slice),
            hub_join_m: self.hub_join_m,
            mode_choice: self.mode_choice.as_deref(),
            mode_defaults: &self.mode_defaults,
            class_modes: &self.class_modes,
            class_limits: &self.class_limits,
            link_values: &self.link_values,
            disruptions: &self.disruptions,
            loading: &self.loading.resolved(self.level(), self.unknown_road_disruptions()),
            prices: self.reads_money().then_some(&self.prices),
            demand_values: &self.demand_values,
            reads: &reads,
        })
    }

    /// The same run, also recording per-link, per-time-bin results in
    /// bins of `bin_seconds`. Costs 20 bytes per link of scratch and a
    /// 28-byte row per (link, bin) that saw traffic; nothing when not asked.
    ///
    /// # Panics
    ///
    /// Panics if `bin_seconds` is zero.
    #[must_use]
    pub fn with_link_bins(mut self, bin_seconds: u32) -> Self {
        assert!(bin_seconds > 0, "a time bin must be at least one second long");
        self.link_bin_seconds = Some(bin_seconds);
        self
    }

    /// Start the simulation window at `start` rather than midnight: the loading's clock
    /// starts there, and the parkings' bins with it. The demand is the caller's to fit: a trip
    /// departing before `start` sets off at it.
    ///
    /// # Panics
    ///
    /// Panics if `start` is not before the window's end.
    #[must_use]
    pub fn with_window_start(mut self, start: Second) -> Self {
        assert!(start < self.window, "the window starts at {start:?}, after its end");
        self.window_start = start;
        self
    }

    /// Run the whole demand through the selected loading engine and report
    /// the result.
    ///
    /// Processes trips in departure-time order via an event queue keyed the
    /// way every event in the core is —
    /// `(second, entity_kind, entity_id)` — so ties resolve identically
    /// every run. Under [`FlowMotor::Level0`] nothing in the *loading*
    /// depends on processing order (vehicles never interact);
    /// under [`FlowMotor::Ltm`] every eligible trip's vehicle is collected
    /// here and loaded together, once, after the queue drains, since the LTM
    /// needs the whole batch to let vehicles interact. Either way a
    /// traveller's own trips still have to be *collected* trip-by-trip, in
    /// order, for vehicle-location hand-over between them to mean anything —
    /// which is exactly what the event queue gives for free.
    ///
    /// # Panics
    ///
    /// Panics if the choice model cannot choose (see [`Self::try_execute`], which
    /// reports that instead). The built-in models never fail on route choice.
    pub fn execute(&mut self, diagnostics: &mut Diagnostics) -> RunResult {
        self.try_execute(diagnostics).unwrap_or_else(|e| panic!("the run could not finish: {e}"))
    }

    /// [`Self::execute`], reporting a failed choice as an error.
    ///
    /// # Errors
    ///
    /// [`RunError::Choice`] if the choice model needs an attribute routes do not
    /// carry, fails, or gives an answer that does not fit its batch; [`RunError::Input`] if a
    /// link value's column is not as long as its layer.
    ///
    /// # Panics
    ///
    /// If a traveller class walks to stops further than the timetable's setup searches
    /// ([`Self::with_class_limits`]). Otherwise never in practice: the panic guards the
    /// invariant that at least one iteration always runs.
    pub fn try_execute(&mut self, diagnostics: &mut Diagnostics) -> Result<RunResult, RunError> {
        // The run's own pool (S277): its parallel work on `threads()` threads, whatever the
        // global pool has. A choice model in Python is called from the pool's thread, so a
        // caller holding Python's lock releases it around the run.
        #[cfg(feature = "parallel")]
        {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(self.threads())
                .build()
                .map_err(|e| RunError::Input(format!("the run's thread pool: {e}")))?;
            pool.install(|| self.execute_here(diagnostics))
        }
        #[cfg(not(feature = "parallel"))]
        self.execute_here(diagnostics)
    }

    /// [`Self::try_execute`]'s work, on the calling thread's pool.
    fn execute_here(&mut self, diagnostics: &mut Diagnostics) -> Result<RunResult, RunError> {
        self.walk_paths.clear();
        if let Some(transit) = &self.transit {
            let widest =
                self.class_limits.iter().filter_map(|l| l.access_walk_max_s).fold(0.0, f64::max);
            assert!(
                widest <= transit.access_search_s(),
                "a class walks {widest} s to a stop, beyond the {} s the timetable's setup \
                 searches: build it with TransitSetup::with_access_search_s",
                transit.access_search_s()
            );
        }
        let total_trips = self.trips.len();
        // The user's link values, checked against their layers and weighted by length (S236).
        let link_values =
            self.link_values.prepare(&self.network, &self.layers).map_err(RunError::Input)?;
        // The user's traveller and trip values (S249), checked against the demand; their
        // attributes are offered beside the link values'.
        self.demand_values
            .check_lengths(self.travellers.len() as usize, self.trips.len() as usize)
            .map_err(RunError::Input)?;
        // A copy for the choices, which borrow it while the run loads (one value per traveller or
        // trip and column, once per run).
        let demand_values = self.demand_values.clone();
        let mut link_value_names = self.link_values.attribute_names();
        link_value_names.extend(self.demand_values.attribute_names());
        // Disruptions (S239): checked; in every loading if travellers know of them, else in one
        // more loading after the run's equilibrium.
        self.disruptions
            .check(&self.network, self.transit.as_ref().map(|t| t.timetable().route_count()))
            .map_err(RunError::Input)?;
        let known_disruptions = self.disruptions.known && !self.disruptions.is_empty();
        // Closures the travellers know of are the network as it will be: every route cost reads
        // them from the closure itself (S300). A known narrowing is read from the link times the
        // loadings record, so it starts and ends on the edges of the cost bins: no bin's times mix
        // before and during it.
        let known_closures =
            if self.disruptions.known { self.disruptions.closures() } else { Vec::new() };
        if self.disruptions.known {
            let bin = f64::from(self.equilibration.cost_bin_seconds());
            let mut factor: std::collections::HashMap<u32, f64> = std::collections::HashMap::new();
            for c in self.disruptions.capacity_changes() {
                let before = factor.insert(c.link.raw(), c.factor).unwrap_or(1.0);
                if c.factor > 0.0 && before > 0.0 && c.time % bin != 0.0 {
                    return Err(RunError::Input(format!(
                        "a road narrowing the travellers know of changes link {} at {} s, inside \
                         a cost bin of {bin} s: the link times averaged over that bin would mix \
                         the times before and during it; start and end it on a multiple of {bin} \
                         s, or set equilibration_options cost_bin_s to a length that divides its \
                         times",
                        self.network.link_external_ids().external_of(c.link).unwrap_or("?"),
                        c.time
                    )));
                }
            }
        }
        // Services (S302): one docked bike service, and no taxi yet.
        if let Some(services) = &self.services {
            if services.len() > 1 || services.iter().any(|s| s.kind() != ServiceKind::BikeDocked) {
                return Err(RunError::Input(
                    "a run takes one docked bike service for now (taxis in a run come later)"
                        .to_string(),
                ));
            }
        }
        // Level 1 (S294) loads car trips on one period's flows, and refuses what it cannot run.
        if let FlowMotor::Static { .. } = self.flow_motor {
            let refused = crate::static_run::refusal(
                self.transit.is_some(),
                self.parking.is_some(),
                self.mode_choice.as_ref().is_some_and(|m| !m.is_empty()),
                &self.disruptions,
            );
            if let Some(why) = refused {
                return Err(RunError::Input(why));
            }
        }
        let mut timings = Timings::new();
        let mut clock = Instant::now();

        // Route sets for every origin-destination pair the demand asks for,
        // made once, in parallel, before any trip runs (S165): each trip's
        // endpoints are snapped to the nearest street node with a grid, not
        // a scan (a motorway-only node only within `car_snap_motorway_m`, U-41),
        // and the pair's set is looked up, not searched for.
        //
        // A trip that is not a car trip gets a key from a node to itself, which no
        // route set, choice or gap takes part in (S195): its own layer routes it. So does a
        // trip choosing its mode (M5): its car routes are among its alternatives, not the
        // route choice's.
        let mode_choice = self.mode_choice.clone();
        let choosing = |trip: TripId| mode_choice.is_some() && !self.trips.mode_given(trip);
        let offers = |mode: Mode| mode_choice.as_ref().is_some_and(|m| m.contains(&mode));
        let snapper = TripSnapper::new(&self.network, self.mode_defaults.car_snap_motorway_m);
        let trip_keys: Vec<RouteKey> = (0..total_trips)
            .map(|i| {
                let trip = TripId::new(i);
                let origin = snapper.nearest(&self.network, self.trips.origin(trip));
                if self.trips.mode(trip) == Mode::Car && !choosing(trip) {
                    RouteKey::new(
                        origin,
                        snapper.nearest(&self.network, self.trips.destination(trip)),
                    )
                } else {
                    RouteKey::new(origin, origin)
                }
            })
            .collect();
        // What the car's route searches minimise (S299): the choice model's link-additive utility
        // per coefficient vector, or time alone (every search then as without prices).
        let road_cost = self.road_cost(&link_values);
        if !road_cost.grouped().is_empty() {
            diagnostics.record_n(
                openmobisim_core_types::diagnostics::DiagKey::run_level(
                    Category::Modelling,
                    local_codes::COST_VECTORS_GROUPED,
                    Severity::Warning,
                ),
                road_cost.grouped().len() as u64,
            );
        }
        // Each trip's user class, for the per-class draws of a sampled route update (S298).
        let trip_class: Vec<u32> = (0..total_trips)
            .map(|i| {
                let class = self.travellers.user_class(self.trips.traveller(TripId::new(i)));
                u32::try_from(class.index()).expect("few classes")
            })
            .collect();
        // Bike and walk trips' routes on their layers: once per run, since their
        // costs are static. A trip choosing its mode gets its route on each layer offered.
        let bike_routing = self.bike_routing(&link_values);
        // The bike and walk layers' search costs from the choice model (S299), where asked for.
        let (utility_costs, grouped) = self.utility_costs(&link_values);
        if grouped > 0 {
            diagnostics.record_n(
                openmobisim_core_types::diagnostics::DiagKey::run_level(
                    Category::Modelling,
                    local_codes::COST_VECTORS_GROUPED,
                    Severity::Warning,
                ),
                grouped as u64,
            );
        }
        let mut static_routes = StaticRoutes::build(
            &self.layers,
            &self.trips,
            &|trip, layer| {
                choosing(trip)
                    && offers(match layer {
                        StaticLayer::Bike => Mode::Bike,
                        StaticLayer::Walk => Mode::Walk,
                    })
            },
            &|trip, layer| {
                let class = self.travellers.user_class(self.trips.traveller(trip)).index();
                class_limits(&self.class_limits, class)
                    .modes(&self.mode_defaults)
                    .max_seconds(layer)
            },
            &bike_routing,
            &utility_costs,
        );
        clock = timings.lap("static_routes", None, clock);
        // Shared handles, so the loading (which moves vehicles about in `self`) and the
        // chooser (which only reads the inputs) do not borrow each other.
        let (network, travellers, trips) =
            (self.network.clone(), self.travellers.clone(), self.trips.clone());
        // Itinerary trips (M4, S201): transit, park-and-ride and bike-and-ride choose an
        // itinerary on expected costs — free flow and the parkings as the day starts at first,
        // the last loading's after, and transit on the timetable throughout; and, with mode
        // choice (M5), every trip whose mode was not stated.
        let (transit_arc, parking_arc, layers_arc) =
            (self.transit.clone(), self.parking.clone(), self.layers.clone());
        // The docked bike service (S302), when the run offers shared bikes in its choice.
        let services_arc = self.services.clone();
        let share_service: Option<&ServiceSetup> =
            services_arc.as_deref().and_then(|s| s.first()).filter(|s| {
                s.kind() == ServiceKind::BikeDocked
                    && (offers(Mode::BikeShare) || offers(Mode::BikeShareTransit))
            });
        let itineraries: Option<Itineraries> = (transit_arc.is_some() || mode_choice.is_some())
            .then(|| {
                Itineraries::new(
                    &trips,
                    &Simulated {
                        transit: transit_arc.as_deref(),
                        parking: parking_arc.as_deref(),
                        road: &network,
                        layers: &layers_arc,
                        choice: mode_choice.as_deref(),
                        static_routes: &static_routes,
                        modes: &self.mode_defaults,
                        class_modes: &self.class_modes,
                        class_limits: &self.class_limits,
                        travellers: &travellers,
                        share: share_service,
                    },
                )
            })
            .filter(|i| !i.is_empty());
        clock = timings.lap("itineraries_setup", None, clock);
        // A trip with no alternative travels by car (S271, W-1): its car pair has route sets.
        let fallback_car = self.mode_defaults.fallback_car && itineraries.is_some();
        // The pairs route sets are made for: the car trips', and the car pairs of the trips
        // choosing their mode (M5), which the route update grows too.
        let car_keys: Vec<RouteKey> = match &itineraries {
            Some(itin) if mode_choice.is_some() || fallback_car => (0..total_trips)
                .map(|i| {
                    let trip = TripId::new(i);
                    itin.position(trip)
                        .and_then(|p| itin.car_key(p))
                        .unwrap_or(trip_keys[i as usize])
                })
                .collect(),
            _ => trip_keys.clone(),
        };
        let turns = match &self.flow_motor {
            FlowMotor::Ltm { turns, .. } | FlowMotor::Static { turns, .. } => turns.clone(),
            FlowMotor::Level0 => Arc::new(TurnTable::build(&self.network, self.network.signals())),
        };
        // Level 1 (S294): the equilibrium, where the other levels make their route sets; its
        // routes are the run's sets and its apportionment the trips' choices.
        self.static_solution = match &self.flow_motor {
            FlowMotor::Static { period_s, .. } => {
                Some(Arc::new(crate::static_run::solve(&crate::static_run::Inputs {
                    network: &self.network,
                    turns: &turns,
                    travellers: &self.travellers,
                    trips: &self.trips,
                    keys: &car_keys,
                    bpr: self.loading.bpr,
                    strategy: self.equilibration.as_ref(),
                    disruptions: &self.disruptions,
                    period_s: *period_s,
                    start_s: self.window_start.get(),
                })))
            }
            _ => None,
        };
        let static_solution = self.static_solution.clone();
        // A method that reads the demand (the Monte Carlo method's bias, S176) is told it
        // first; one that does not costs nothing here.
        let demand_rows: Option<Vec<TripDemand>> = self.route_generator.reads_demand().then(|| {
            (0..total_trips)
                .filter(|&i| {
                    let trip = TripId::new(i);
                    self.trips.mode(trip) == Mode::Car && !choosing(trip)
                })
                .map(|i| {
                    let trip = TripId::new(i);
                    TripDemand {
                        key: trip_keys[i as usize],
                        weight: self.travellers.weight(self.trips.traveller(trip)),
                        departure: self.trips.departure(trip).get(),
                    }
                })
                .collect()
        });
        // Extras (S298): a method or update that searches by generalised cost asks for extra
        // seconds per link, per user class and preference draw, composed from the choice
        // model's coefficients, the prices and the link values. Cheap, and only when asked.
        let update_extras: Option<ClassExtras> =
            self.route_update.extras_wanted().map(|r| self.compose_extras(r, &link_values));
        // The method with its extras, if it asked for them: its descriptor (and so the cache's
        // key and the manifest) names them.
        let keyed_generator = self.keyed_generator(&link_values);
        let generate = || {
            let generator: Arc<dyn RouteSetGenerator> = match &demand_rows {
                Some(rows) => {
                    let demand = Demand { network: &self.network, turns: &turns, trips: rows };
                    keyed_generator
                        .with_demand(&demand)
                        .map_or_else(|| keyed_generator.clone(), Arc::from)
                }
                None => keyed_generator.clone(),
            };
            RouteSets::generate(&self.network, &turns, &car_keys, generator.as_ref())
        };
        // Sets already generated for these inputs are handed back, if the run has a cache (S178).
        let mut route_sets = match (&static_solution, &self.route_cache) {
            (Some(solution), _) => Arc::new(solution.route_sets.clone()),
            (None, Some(cache)) => {
                let key = generation_key(
                    &self.network,
                    keyed_generator.as_ref(),
                    &car_keys,
                    demand_rows.as_deref(),
                );
                cache.get_or_generate(key, generate)
            }
            (None, None) => Arc::new(generate()),
        };
        // Closures the travellers know of (S300): a pair whose every route crosses a closed link is
        // given the least free-flow route round them, so that the first choice can plan round.
        if !known_closures.is_empty() && static_solution.is_none() {
            route_sets =
                Arc::new(with_routes_round(&self.network, &turns, &route_sets, &known_closures));
        }
        clock = timings.lap(
            if static_solution.is_some() { "static_assignment" } else { "route_sets" },
            None,
            clock,
        );

        // Choice and equilibration (S169, S170): every trip chooses a route on
        // free-flow costs; then, under an equilibration strategy, the network is
        // loaded, the link times it produced re-cost the routes, some travellers
        // choose again, and the network is loaded again.
        let choice_rng = StreamRng::new(RngKey::from_seed(self.master_seed), Stream::Choice);
        let reselect_rng =
            StreamRng::new(RngKey::from_seed(self.master_seed), Stream::MsaReselection);
        let model = self.choice_model.clone();
        // The prices (S248), each parking's fee resolved and the toll column found.
        let prices = PreparedPrices::new(self.prices, parking_arc.as_deref(), &link_values);
        let inputs = route_choice::Inputs {
            network: &network,
            travellers: &travellers,
            trips: &trips,
            trip_keys: &trip_keys,
            model: model.as_ref(),
            detour_limit: self.choice_detour_limit,
            max_routes: self.choice_max_routes,
            link_values: &link_values,
            link_value_names: &link_value_names,
            prices: &prices,
            demand_values: &demand_values,
            car_start_s: self.car_start_s(),
            mode: Mode::Car,
            bike_layer: None,
            cost: Some(&road_cost),
        };
        let mut chooser = route_choice::Chooser::new(&inputs, route_sets.clone())?;
        // The bike route choice (S298): with bike route sets, every stated bike trip chooses its
        // route once, on the layer's static times, through the run's choice model; a trip
        // choosing its mode is offered the routes as bike alternatives instead.
        let mut bike_route_choices: Option<RouteChoices> = None;
        let bike_sets: Option<Arc<RouteSets>> =
            match (&bike_routing, self.layers.get(StaticLayer::Bike)) {
                (crate::layers::BikeRouting::Sampled { .. }, Some(setup)) => static_routes
                    .sets(StaticLayer::Bike)
                    .map(|sets| (setup, Arc::new(sets.clone()))),
                _ => None,
            }
            .map(|(setup, sets)| {
                let graph = setup.network().network();
                let none = RouteKey::new(NodeId::new(0), NodeId::new(0));
                let bike_keys: Vec<RouteKey> = (0..total_trips)
                    .map(|i| {
                        let trip = TripId::new(i);
                        if self.trips.mode(trip) != Mode::Bike {
                            return none;
                        }
                        static_routes
                            .routes_of(trip, StaticLayer::Bike)
                            .map_or(none, |range| sets.keys()[sets.key_of_route(range.start)])
                    })
                    .collect();
                let bike_inputs = route_choice::Inputs {
                    network: graph,
                    travellers: &travellers,
                    trips: &trips,
                    trip_keys: &bike_keys,
                    model: model.as_ref(),
                    detour_limit: self.choice_detour_limit,
                    max_routes: self.choice_max_routes,
                    link_values: &link_values,
                    link_value_names: &link_value_names,
                    prices: &prices,
                    demand_values: &demand_values,
                    car_start_s: whole_seconds(self.mode_defaults.bike_access_s),
                    mode: Mode::Bike,
                    bike_layer: Some(setup.network()),
                    cost: None,
                };
                let bike_chooser = route_choice::Chooser::new(&bike_inputs, sets.clone())?;
                let times = LinkTimes::constant(graph, setup.seconds().to_vec());
                let choices = bike_chooser.choose_all(&choice_rng, 0, Some(&times))?;
                for (i, &route) in choices.route.iter().enumerate() {
                    if route != NO_ROUTE {
                        static_routes.set_route(TripId::from_index(i), StaticLayer::Bike, route);
                    }
                }
                bike_route_choices = Some(choices);
                Ok::<_, ChoiceError>(sets)
            })
            .transpose()?;
        clock = timings.lap("bike_route_choice", None, clock);
        // What the full gap is measured in (S293): the model's utility, or time.
        let currency = Currency::new(model.as_ref(), &travellers, &chooser.wanted_names())?;
        let mut route_choices = match &static_solution {
            Some(solution) => solution.route_choices.clone(),
            // On free-flow times and the closures the travellers know of (S300).
            None if !known_closures.is_empty() => {
                let free = LinkTimes::free_flow(&network).with_closures(&known_closures);
                chooser.choose_all(&choice_rng, 0, Some(&free))?
            }
            None => chooser.choose_all(&choice_rng, 0, None)?,
        };
        clock = timings.lap("route_choice", Some(0), clock);
        let route_update = self.route_update.clone();

        // A run offering shared bikes offers their attributes too (S302); no other run does.
        let mut builtin_attributes = itinerary_choice::ATTRIBUTES.to_vec();
        if share_service.is_some() {
            builtin_attributes.extend(itinerary_choice::SERVICE_ATTRIBUTES);
        }
        let itinerary_wanted = match &itineraries {
            Some(_) => {
                itinerary_choice::wanted(model.as_ref(), &builtin_attributes, &link_value_names)?
            }
            None => Vec::new(),
        };
        let car_ctx = SearchContext::new(&network, &turns);
        // Re-routing by each trip's vector, where the cost is not time alone (S299).
        self.reroute_cost = (!road_cost.is_time_only()).then(|| {
            Arc::new(crate::reroute::RerouteCost {
                extras: road_cost
                    .vectors()
                    .iter()
                    .map(|v| v.as_ref().map(|e| car_ctx.extras(e.to_vec())))
                    .collect(),
                trip_vector: trip_class
                    .iter()
                    .map(|&c| road_cost.vector_of_class(c as usize))
                    .collect(),
            })
        });
        // The route cost's extras prepared for the car's searches (S299): none by time alone.
        let car_extras: Vec<Option<openmobisim_core_routes::Extras>> = if road_cost.is_time_only() {
            Vec::new()
        } else {
            road_cost
                .vectors()
                .iter()
                .map(|v| v.as_ref().map(|e| car_ctx.extras(e.to_vec())))
                .collect()
        };
        let bike_ctx = layers_arc.bike.as_ref().map(|b| {
            let costs = utility_costs.costs(StaticLayer::Bike, b).to_vec();
            SearchContext::with_costs(b.network().network(), b.turns(), costs)
        });
        let bike_planner = layers_arc.bike.as_ref().zip(bike_ctx.as_ref());
        let window_s = f64::from(self.window.get());
        let start_s = f64::from(self.window_start.get());
        // The timetable travellers plan transit on, every choice and gap of the run (S299): the
        // schedule, with the disruptions they know of (a delayed run late, a cancelled one gone).
        // Never the times a loading made: nobody knows a bus's delay before the day.
        let planned_times = transit_arc.as_deref().filter(|_| known_disruptions).and_then(|t| {
            (!self.disruptions.transit.is_empty()).then(|| {
                let mut times = t.timetable().scheduled().clone();
                self.disruptions.apply_to_times(t.timetable(), &mut times, &|_| false);
                t.on_times(&times)
            })
        });
        let planned = transit_arc
            .as_deref()
            .map(|t| (t, planned_times.as_ref().unwrap_or_else(|| t.scheduled())));
        let mut availability =
            parking_arc.as_deref().map(|p| ExpectedAvailability::initial(p, start_s, window_s));
        // The docks' expected costs (S302): the start-of-day counts', then the loadings' average.
        let share_walk = layers_arc.walk.as_ref();
        let dock_index = share_service
            .zip(share_walk)
            .map(|(s, w)| DockIndex::new(s, w.network().network().node_count()));
        let mut dock_costs = share_service.map(|s| DockCosts::initial(s, (start_s, window_s)));
        // The docks at stops' hubs (S302), when the run offers a shared bike to transit: the
        // run's one hub set, its stops and parkings first, then the service's docks.
        let hub_docks = match (share_service, transit_arc.as_deref(), share_walk, &layers_arc.bike)
        {
            (Some(service), Some(transit), Some(walk), Some(bike))
                if offers(Mode::BikeShareTransit) =>
            {
                let hubs = crate::services::hubs::RunHubs::build(
                    Some(transit),
                    &[],
                    parking_arc.as_deref(),
                    &service.resource_rows(),
                    self.hub_join_m,
                );
                let d = parking_arc
                    .as_deref()
                    .map_or(crate::parking::ParkingDefaults::SHIPPED, ParkingSetup::defaults);
                Some(HubDocks::new(
                    service,
                    &hubs,
                    transit,
                    walk,
                    bike.network().network().node_count(),
                    (d.reach_bike_s, d.rank_speed_km_h),
                ))
            }
            _ => None,
        };
        let mut dock_seen = 0_u32;
        let mut services_run = services_arc.as_ref().map(|s| ServicesRun::of(&s[0]));
        let choose_inputs = ChooseInputs {
            trips: &trips,
            travellers: &travellers,
            model: model.as_ref(),
            rng: &choice_rng,
            wanted: &itinerary_wanted,
            link_values: &link_values,
            prices: &prices,
            demand_values: &demand_values,
        };
        // The free-flow loading's groups (S223), below: the first chooses on free flow here; the
        // others choose there, each on the congestion of the groups before it, so only the first
        // is planned here (S267: the others' free-flow choices were never loaded).
        let increments = match self.flow_motor {
            FlowMotor::Ltm { .. } => self.equilibration.increments().max(1),
            FlowMotor::Level0 | FlowMotor::Static { .. } => 1,
        };
        let group: Vec<u32> = if increments > 1 {
            (0..travellers.len()).map(|t| increment_of(&reselect_rng, t, increments)).collect()
        } else {
            Vec::new()
        };
        let first_group: Option<Vec<bool>> =
            (increments > 1).then(|| group.iter().map(|&g| g == 0).collect());
        let mut chosen: Option<Chosen> = None;
        if let Some(itin) = &itineraries {
            let free = LinkTimes::free_flow(&network).with_closures(&known_closures);
            let planner = Planner {
                transit: planned,
                parking: parking_arc.as_deref(),
                car: &car_ctx,
                bike: bike_planner,
                times: &free,
                availability: availability.as_ref(),
                detour_limit: self.choice_detour_limit,
                max_routes: self.choice_max_routes,
                car_access_s: self.mode_defaults.car_access_s,
                car_egress_s: self.mode_defaults.car_egress_s,
                bike_access_s: self.mode_defaults.bike_access_s,
                bike_egress_s: self.mode_defaults.bike_egress_s,
                car_cost: Some(&road_cost),
                car_extras: &car_extras,
                share: share_context(
                    share_service,
                    share_walk,
                    dock_index.as_ref(),
                    dock_costs.as_ref(),
                    hub_docks.as_ref(),
                ),
                car_routes: (mode_choice.is_some() || fallback_car)
                    .then(|| CarRoutes { sets: &route_sets }),
            };
            chosen = Some(
                itin.choose(
                    &planner,
                    &choose_inputs,
                    None,
                    0,
                    None,
                    first_group.as_deref(),
                    &mut CarReach::new(),
                    None,
                )?
                .0,
            );
            clock = timings.lap("itinerary_choice", Some(0), clock);
        }

        let strategy = self.equilibration.clone();
        // Level 1's solver iterated inside the one loading it makes (S294).
        let max_iterations =
            if static_solution.is_some() { 1 } else { strategy.max_iterations().max(1) };
        // The departure bin of the shared search trees (S294): the route update's, so that it
        // and the gaps read the same trees; the cost bin where the update reads none.
        let tree_step_s =
            route_update.tree_step_s().unwrap_or_else(|| strategy.cost_bin_seconds()).max(1);
        // Whether park-and-ride's shared drives share the trees' bin, to the bit: then a drive
        // leaves at the very second its tree does, and the trees answer it too (S299).
        let same_step = |p: &ParkingSetup| {
            p.defaults().reach_car_step_s.to_bits() == f64::from(tree_step_s).to_bits()
        };
        // Link times are recorded whenever there is a next iteration to cost: the flow map at
        // the user's bin length if they asked for one, the times the next choice reads at the
        // strategy's (S232).
        let record_bins = if max_iterations > 1 {
            Some(self.link_bin_seconds.unwrap_or_else(|| strategy.cost_bin_seconds()))
        } else {
            self.link_bin_seconds
        };
        let mut reports: Vec<IterationReport> = Vec::new();
        let mut previous_bins: Option<EntryTables> = None;
        // Who moved on the way to this iteration: (share that chose again, share that changed).
        let mut arrived_by = (1.0, f64::NAN);
        // Of the trips choosing their mode, the share whose mode changed on the way (M5), and
        // what chance alone would change (I-al).
        let mut mode_changed_by = f64::NAN;
        let mut mode_floor_by = f64::NAN;
        let mut converged = false;
        let mut last: Option<(Loaded, Diagnostics)> = None;
        // The full gap's per-trip numbers of the latest loading assessed (S293).
        let mut last_gap: Option<GapTrips> = None;

        // The first loadings may be made with the point-queue model, which cannot gridlock (S178);
        // the last of an iterating run, its result, never is. A run of one loading is its
        // free-flow loading, which may (S223).
        let warmup = strategy.warmup_iterations().min((max_iterations - 1).max(1));
        // The link times the last loading produced: what a vehicle that re-routes knows (S213).
        let mut expected_times: Option<LinkTimes> = None;

        // The free-flow loading's increments (S223): travellers in groups, each choosing on the
        // times of a loading of the groups before it, so the first loading of everyone (iteration
        // 0's, below) is on routes that already spread around congestion. Not at free flow, where
        // loading changes no time.
        // What the route update found while it was built up: iteration 0's report counts it.
        let (mut increment_searches, mut increment_added) = (0_u32, 0_u32);
        let mut increment_reroute_searches = 0_u32;
        if increments > 1 {
            for k in 1..increments {
                let loading: Vec<bool> = group.iter().map(|&g| g < k).collect();
                // Its diagnostics are not the run's: the run's are iteration 0's, of everyone.
                let mut partial_diagnostics = Diagnostics::new();
                let loaded = self.load_once(
                    &trip_keys,
                    &route_sets,
                    &route_choices,
                    &static_routes,
                    LoadPlan {
                        record_bins: Some(strategy.cost_bin_seconds()),
                        want_entry: true,
                        level_override: (warmup > 0).then_some(FidelityLevel::PointQueue),
                        active: Some(&loading),
                        disrupted: known_disruptions,
                    },
                    itineraries.as_ref().zip(chosen.as_ref()),
                    (&car_ctx, expected_times.as_ref()),
                    &mut partial_diagnostics,
                );
                clock = timings.lap("partial_loading", Some(0), clock);
                increment_reroute_searches += loaded.reroute_searches;
                // Every group chooses (S267: none chose before), on free flow if this loading
                // recorded no times.
                let times = loaded
                    .entry_bins
                    .as_ref()
                    .map_or_else(
                        || LinkTimes::free_flow(&network),
                        |bins| LinkTimes::from_tables(&self.network, bins),
                    )
                    .with_closures(&known_closures);
                let choosing: Vec<bool> = group.iter().map(|&g| g == k).collect();
                // The group's park-and-ride drives, answered by its trees (S299), if any.
                let mut car_reach = CarReach::new();
                // The group's pairs searched on these times: routes that avoid the congestion of
                // the groups before it, for it to choose among.
                if route_update.is_active() {
                    let trip_choosing: Vec<bool> = (0..total_trips)
                        .map(|i| choosing[trips.traveller(TripId::new(i)).index()])
                        .collect();
                    let drives_ahead = match (&itineraries, &chosen, parking_arc.as_deref()) {
                        (Some(itin), Some(current), Some(parking)) if same_step(parking) => {
                            let planner = Planner {
                                transit: None,
                                parking: Some(parking),
                                car: &car_ctx,
                                bike: bike_planner,
                                times: &times,
                                availability: availability.as_ref(),
                                detour_limit: self.choice_detour_limit,
                                max_routes: self.choice_max_routes,
                                car_access_s: self.mode_defaults.car_access_s,
                                car_egress_s: self.mode_defaults.car_egress_s,
                                bike_access_s: self.mode_defaults.bike_access_s,
                                bike_egress_s: self.mode_defaults.bike_egress_s,
                                car_cost: Some(&road_cost),
                                car_extras: &car_extras,
                                share: share_context(
                                    share_service,
                                    share_walk,
                                    dock_index.as_ref(),
                                    dock_costs.as_ref(),
                                    hub_docks.as_ref(),
                                ),
                                car_routes: None,
                            };
                            Some(itin.drives_ahead(
                                &planner,
                                &trips,
                                &travellers,
                                current,
                                Some(&choosing),
                            ))
                        }
                        _ => None,
                    };
                    // The group's shared search trees (S294), on these times.
                    let (trees, drives) = SearchTrees::build_with_drives(
                        (&network, &turns, &times),
                        &trips,
                        &car_keys,
                        &route_sets,
                        Some(&trip_choosing),
                        self.car_start_s(),
                        f64::from(tree_step_s),
                        drives_ahead
                            .as_deref()
                            .zip(parking_arc.as_deref().map(|p| p.defaults().reach_car_s)),
                        Some(TreeCost { cost: &road_cost, trip_class: &trip_class }),
                    );
                    car_reach = itinerary_choice::car_reach_of(drives);
                    clock = timings.lap("trees", Some(0), clock);
                    let found = route_update.update(&UpdateContext {
                        network: &network,
                        turns: &turns,
                        trips: &trips,
                        trip_keys: &car_keys,
                        route_sets: &route_sets,
                        times: &times,
                        car_start_s: self.car_start_s(),
                        iteration: 0,
                        active: Some(&trip_choosing),
                        trees: Some(&trees),
                        trip_class: &trip_class,
                        extras: update_extras.as_ref(),
                        route_cost: Some(&road_cost),
                    });
                    increment_searches += found.searches;
                    if !found.routes.is_empty() {
                        let kept_routes = itinerary_routes_kept(
                            &route_sets,
                            &car_keys,
                            itineraries.as_ref().zip(chosen.as_ref()),
                        );
                        let (grown, added) = grow_sets(
                            &network,
                            &route_sets,
                            found.routes,
                            route_update.as_ref(),
                            &mut route_choices,
                            &trip_keys,
                            &kept_routes,
                            1,
                            &road_cost,
                        );
                        increment_added += added;
                        route_sets = Arc::new(grown);
                        chooser = route_choice::Chooser::new(&inputs, route_sets.clone())?;
                    }
                    clock = timings.lap("route_update", Some(0), clock);
                }
                for c in chooser.choose_some(&times, 0, &choosing, &choice_rng)? {
                    route_choices.route[c.trip] = c.route;
                    route_choices.probability[c.trip] = c.probability;
                }
                clock = timings.lap("route_choice", Some(0), clock);
                if let (Some(itin), Some(current)) = (&itineraries, &chosen) {
                    let planner = Planner {
                        transit: planned,
                        parking: parking_arc.as_deref(),
                        car: &car_ctx,
                        bike: bike_planner,
                        times: &times,
                        availability: availability.as_ref(),
                        detour_limit: self.choice_detour_limit,
                        max_routes: self.choice_max_routes,
                        car_access_s: self.mode_defaults.car_access_s,
                        car_egress_s: self.mode_defaults.car_egress_s,
                        bike_access_s: self.mode_defaults.bike_access_s,
                        bike_egress_s: self.mode_defaults.bike_egress_s,
                        car_cost: Some(&road_cost),
                        car_extras: &car_extras,
                        share: share_context(
                            share_service,
                            share_walk,
                            dock_index.as_ref(),
                            dock_costs.as_ref(),
                            hub_docks.as_ref(),
                        ),
                        car_routes: (mode_choice.is_some() || fallback_car)
                            .then(|| CarRoutes { sets: &route_sets }),
                    };
                    let (next_chosen, _) = itin.choose(
                        &planner,
                        &choose_inputs,
                        Some(current),
                        0,
                        None,
                        Some(&choosing),
                        &mut car_reach,
                        None,
                    )?;
                    chosen = Some(next_chosen);
                    clock = timings.lap("itinerary_choice", Some(0), clock);
                }
                expected_times = Some(times);
            }
        }
        for iteration in 0..max_iterations {
            let mut iteration_diagnostics = Diagnostics::new();
            let loaded = self.load_once(
                &trip_keys,
                &route_sets,
                &route_choices,
                &static_routes,
                LoadPlan {
                    record_bins,
                    want_entry: max_iterations > 1,
                    level_override: (iteration < warmup).then_some(FidelityLevel::PointQueue),
                    active: None,
                    disrupted: known_disruptions,
                },
                itineraries.as_ref().zip(chosen.as_ref()),
                (&car_ctx, expected_times.as_ref()),
                &mut iteration_diagnostics,
            );
            clock = timings.lap("loading", Some(iteration), clock);
            if let Some(services) = services_run.as_mut() {
                let day = loaded.share.as_ref();
                services.mismatch_s.push(day.map_or(f64::NAN, |d| d.mismatch_s));
                services.bias_s.push(day.map_or(f64::NAN, |d| d.bias_s));
                services.legs_people.push(
                    day.map_or(0.0, |d| d.legs.iter().map(|l| f64::from(l.leg.weight)).sum()),
                );
                services.engine_s += day.map_or(0.0, |d| d.engine_s);
            }
            let mut report = IterationReport::unmeasured(iteration);
            report.gap_currency = currency.label();
            // The stopping rule in the choice's own currency where a class weighs more than time
            // (S299): the in-set gap in generalised minutes, measured with the choice.
            report.stop_generalised = !road_cost.is_time_only();
            report.reselected_share = arrived_by.0;
            report.changed_share = arrived_by.1;
            report.mode_changed_share = mode_changed_by;
            report.mode_changed_floor = mode_floor_by;
            report.total_travel_time_s = loaded.total_travel_time.get();
            report.completed = loaded.completion.completed;
            report.completed_people =
                completed_people(&loaded.events, &self.trips, &self.travellers);
            report.truncated = loaded.completion.truncated;
            report.reroutes = u32::try_from(loaded.reroutes.len()).unwrap_or(u32::MAX);
            report.reroute_searches = loaded.reroute_searches;
            report.hub_mismatch_s = loaded.parking.as_ref().map_or(f64::NAN, |p| p.mismatch_s);
            let tally = &loaded.itinerary;
            report.gap_transit_plan_s = if tally.plan_error_weight > 0.0 {
                tally.plan_error_sum / tally.plan_error_weight
            } else {
                f64::NAN
            };
            let mut pending_chosen: Option<Chosen> = None;
            if let (Some(before), Some(now)) = (&previous_bins, &loaded.entry_bins) {
                report.time_change = relative_time_change(&self.network, before, now);
            }

            let mut changes = Vec::new();
            let mut times_now: Option<LinkTimes> = None;
            // The shared search trees on this loading's times (S294): what the route update and
            // the gaps read instead of searching.
            let mut trees_now: Option<SearchTrees> = None;
            // What the assessment left for the whole-network gap: who was left out, and what the
            // model expected of each trip.
            let mut assessed: Option<(Vec<bool>, Vec<f64>)> = None;
            if max_iterations > 1 {
                if let Some(bins) = &loaded.entry_bins {
                    let times =
                        LinkTimes::from_tables(&self.network, bins).with_closures(&known_closures);
                    let next = iteration + 1;
                    // The parkings' availability this loading realised, for the itineraries
                    // chosen on its costs.
                    if let (Some(_), Some(_), Some(av), Some(real)) =
                        (&itineraries, &chosen, availability.as_mut(), &loaded.availability)
                    {
                        av.absorb(real);
                    }
                    // The docks' costs this loading realised (S302), averaged in.
                    if let (Some(expected), Some(day)) = (dock_costs.as_mut(), &loaded.share) {
                        expected.absorb(&day.realised, dock_seen);
                        dock_seen += 1;
                    }
                    // What the itinerary choice and the gap plan on, but the timetable and the
                    // car's route sets (set below, once the sets have grown).
                    let base_planner = Planner {
                        transit: None,
                        parking: parking_arc.as_deref(),
                        car: &car_ctx,
                        bike: bike_planner,
                        times: &times,
                        availability: availability.as_ref(),
                        detour_limit: self.choice_detour_limit,
                        max_routes: self.choice_max_routes,
                        car_access_s: self.mode_defaults.car_access_s,
                        car_egress_s: self.mode_defaults.car_egress_s,
                        bike_access_s: self.mode_defaults.bike_access_s,
                        bike_egress_s: self.mode_defaults.bike_egress_s,
                        car_cost: Some(&road_cost),
                        car_extras: &car_extras,
                        share: share_context(
                            share_service,
                            share_walk,
                            dock_index.as_ref(),
                            dock_costs.as_ref(),
                            hub_docks.as_ref(),
                        ),
                        car_routes: None,
                    };
                    // Park-and-ride's shared drives leave from a road node at a step's middle on
                    // these times, as the trees do: when the step is the trees' bin, the trees
                    // answer them too, in the same searches (S299).
                    let drives_ahead = match (&itineraries, &chosen, parking_arc.as_deref()) {
                        (Some(itin), Some(current), Some(parking)) if same_step(parking) => Some(
                            itin.drives_ahead(&base_planner, &trips, &travellers, current, None),
                        ),
                        _ => None,
                    };
                    // One search per origin and departure bin (S294), for every pair with a set:
                    // the route update reads them for the routes worth adding, the gaps for the
                    // network's best.
                    let (trees, drives) = SearchTrees::build_with_drives(
                        (&network, &turns, &times),
                        &trips,
                        &car_keys,
                        &route_sets,
                        None,
                        self.car_start_s(),
                        f64::from(tree_step_s),
                        drives_ahead
                            .as_deref()
                            .zip(parking_arc.as_deref().map(|p| p.defaults().reach_car_s)),
                        Some(TreeCost { cost: &road_cost, trip_class: &trip_class }),
                    );
                    let mut car_reach = itinerary_choice::car_reach_of(drives);
                    clock = timings.lap("trees", Some(iteration), clock);
                    // The sets grow between loadings (S176): routes the congested times of
                    // this one show to be worth having, for the next to choose among. After the
                    // last loading too, which no choice follows: the gaps below read the best
                    // route of each set, so every loading is measured on the same footing as a
                    // longer run's (S300).
                    if route_update.is_active() {
                        let found = route_update.update(&UpdateContext {
                            network: &network,
                            turns: &turns,
                            trips: &trips,
                            trip_keys: &car_keys,
                            route_sets: &route_sets,
                            times: &times,
                            car_start_s: self.car_start_s(),
                            iteration: next,
                            active: None,
                            trees: Some(&trees),
                            trip_class: &trip_class,
                            extras: update_extras.as_ref(),
                            route_cost: Some(&road_cost),
                        });
                        report.route_searches = found.searches;
                        if !found.routes.is_empty() {
                            let kept_routes = itinerary_routes_kept(
                                &route_sets,
                                &car_keys,
                                itineraries.as_ref().zip(chosen.as_ref()),
                            );
                            let (grown, added) = grow_sets(
                                &network,
                                &route_sets,
                                found.routes,
                                route_update.as_ref(),
                                &mut route_choices,
                                &trip_keys,
                                &kept_routes,
                                next,
                                &road_cost,
                            );
                            report.routes_added = added;
                            route_sets = Arc::new(grown);
                            chooser = route_choice::Chooser::new(&inputs, route_sets.clone())?;
                        }
                        clock = timings.lap("route_update", Some(iteration), clock);
                    }
                    // The share who choose again for the next loading, settled once from the
                    // reports so far (this loading's is not made yet) and the same for every
                    // traveller, thread and order.
                    let share = strategy.reselect_share(next, &reports);
                    let strategy_for_next = (next < max_iterations).then_some((
                        strategy.as_ref(),
                        &reselect_rng,
                        share,
                    ));
                    // The trips still under way at the end have no measured time: the gaps leave
                    // them out (S178).
                    let mut unfinished = vec![false; total_trips as usize];
                    for e in &loaded.events {
                        if e.event_type == EventType::TripTruncated {
                            unfinished[e.entity_id as usize] = true;
                        }
                    }
                    // The full gap (S293) costs them instead: where each car still under way stood.
                    let mut link_at_end = vec![NO_LINK; total_trips as usize];
                    for &(trip, link) in &loaded.at_end {
                        link_at_end[trip.index()] = link.raw();
                    }
                    let ended = Ended {
                        unfinished: &unfinished,
                        link_at_end: &link_at_end,
                        window_end: window_s,
                    };
                    let mut gap_trips = GapTrips::new(&trips, &travellers, &unfinished);
                    // The full gap after every loading, or after the last alone (S294): whether
                    // this loading is the last is read from the stopping rule on the reports so
                    // far with this one's in-set measures, before the gap is made.
                    let gap_every = strategy.full_gap_every_loading();
                    let is_last = |report: &IterationReport| {
                        gap_every || next == max_iterations || {
                            let mut so_far = reports.clone();
                            so_far.push(*report);
                            strategy.is_converged(&so_far)
                        }
                    };
                    let mut gap_made = false;
                    let update = chooser.update(
                        &route_choices,
                        &times,
                        next,
                        strategy_for_next,
                        &choice_rng,
                        &unfinished,
                        (!road_cost.is_time_only()).then_some(&currency),
                    )?;
                    clock = timings.lap("route_choice", Some(iteration), clock);
                    report.gap_generalised_excess = update.assessment.gap_generalised_excess;
                    report.gap = update.assessment.gap;
                    report.gap_expected = update.assessment.gap_expected;
                    report.gap_excess = update.assessment.gap_excess;
                    report.incomplete_share = update.assessment.incomplete_share;
                    report.gap_flow = update.assessment.gap_flow;
                    report.gap_flow_floor = update.assessment.gap_flow_floor;
                    report.gap_flow_excess =
                        update.assessment.gap_flow - update.assessment.gap_flow_floor;
                    // The itineraries, on the costs this loading produced (M4).
                    if let (Some(itin), Some(current)) = (&itineraries, &chosen) {
                        let planner = Planner {
                            transit: planned,
                            car_routes: (mode_choice.is_some() || fallback_car)
                                .then(|| CarRoutes { sets: &route_sets }),
                            ..base_planner
                        };
                        // When the full gap surely follows on this planner, the choice leaves it
                        // the sets it would plan again (S299).
                        let mut gap_plans =
                            (gap_every || next == max_iterations).then(GapPlans::new);
                        let (next_chosen, found) = itin.choose(
                            &planner,
                            &choose_inputs,
                            Some(current),
                            next,
                            strategy_for_next,
                            None,
                            &mut car_reach,
                            gap_plans.as_mut(),
                        )?;
                        clock = timings.lap("itinerary_choice", Some(iteration), clock);
                        // Every mode's: a trip choosing its mode is measured against the best
                        // of the mode it took (M5).
                        for mode in Mode::ALL {
                            report.itinerary_gap[mode.index()] = found.gap(mode);
                        }
                        report.itinerary_recosted = found.recosted;
                        report.gap_transit_plan_share =
                            found.plan_error_share(report.gap_transit_plan_s);
                        report.itinerary_gap_excess = found.gap_excess_pooled();
                        // The full gap (S293): every trip choosing its mode and every car
                        // itinerary, on the same costs; after the last loading, or every one.
                        if is_last(&report) {
                            itin.gap(
                                &planner,
                                &choose_inputs,
                                current,
                                next,
                                &ended,
                                &currency,
                                &mut gap_trips,
                                &mut car_reach,
                                gap_plans.take(),
                            )?;
                            gap_made = true;
                            clock = timings.lap("gap", Some(iteration), clock);
                        }
                        mode_changed_by = found.mode_changed_share();
                        mode_floor_by = found.mode_changed_floor();
                        pending_chosen = Some(next_chosen);
                    }
                    // The full gap (S293): every car trip of the route choice; after the last
                    // loading, or every one (S294).
                    if gap_made || is_last(&report) {
                        chooser.gap(
                            &route_choices,
                            &times,
                            &trees,
                            next,
                            &ended,
                            &currency,
                            &mut gap_trips,
                        )?;
                        gap_trips.report_into(&mut report);
                        clock = timings.lap("gap", Some(iteration), clock);
                        last_gap = Some(gap_trips);
                    }
                    times_now = Some(times);
                    trees_now = Some(trees);
                    arrived_by =
                        (update.assessment.reselected_share, update.assessment.changed_share);
                    changes = update.changes;
                    assessed = Some((unfinished, update.expected_seconds));
                }
            }
            if iteration == 0 {
                report.route_searches += increment_searches;
                report.routes_added += increment_added;
                report.reroute_searches += increment_reroute_searches;
            }
            reports.push(report);
            previous_bins = loaded.entry_bins.clone();
            last = Some((loaded, iteration_diagnostics));

            let stop = iteration + 1 == max_iterations || strategy.is_converged(&reports);
            if stop {
                // The last iteration is also tested against the whole network (S171).
                if let (Some(times), Some(trees), Some((unfinished, expected)), true) =
                    (&times_now, &trees_now, &assessed, strategy.network_gap_sample() > 0)
                {
                    let (gap, excess) =
                        chooser.network_gap(&route_choices, times, trees, unfinished, expected);
                    if let Some(last_report) = reports.last_mut() {
                        last_report.gap_network = gap;
                        last_report.gap_network_excess = excess;
                    }
                    clock = timings.lap("network_gap", Some(iteration), clock);
                }
                converged = strategy.is_converged(&reports);
                self.final_times = times_now.map(Arc::new);
                break;
            }
            for c in changes {
                route_choices.route[c.trip] = c.route;
                route_choices.probability[c.trip] = c.probability;
            }
            if pending_chosen.is_some() {
                chosen = pending_chosen;
            }
            expected_times = times_now;
        }

        // Disruptions travellers did not know of (S239): the day once more with them, every
        // choice as the run left it; what happens within the day (rerouting, passengers whose
        // run never comes) is all that reacts. This loading is the run's result.
        if !self.disruptions.is_empty() && !self.disruptions.known {
            let mut disrupted_diagnostics = Diagnostics::new();
            let expected = self.final_times.clone().or(expected_times.map(Arc::new));
            let loaded = self.load_once(
                &trip_keys,
                &route_sets,
                &route_choices,
                &static_routes,
                LoadPlan {
                    record_bins,
                    want_entry: true,
                    level_override: None,
                    active: None,
                    disrupted: true,
                },
                itineraries.as_ref().zip(chosen.as_ref()),
                (&car_ctx, expected.as_deref()),
                &mut disrupted_diagnostics,
            );
            clock = timings.lap("disrupted_loading", None, clock);
            if let Some(bins) = &loaded.entry_bins {
                self.final_times = Some(Arc::new(LinkTimes::from_tables(&self.network, bins)));
            }
            last = Some((loaded, disrupted_diagnostics));
        }

        let (loaded, iteration_diagnostics) = last.expect("at least one iteration ran");
        // Level 1 (S294): the solver's iterations are the run's reports, its gap the verdict's.
        if let Some(solution) = &static_solution {
            let people = completed_people(&loaded.events, &self.trips, &self.travellers);
            reports = solution.reports(loaded.completion.completed, people);
            converged = solution.result.converged;
            // What a skim reads after the run: the equilibrium's times, the same all day.
            self.final_times =
                Some(Arc::new(LinkTimes::constant(&self.network, solution.times_s().to_vec())));
        }
        diagnostics.merge(&iteration_diagnostics);
        // The chooser holds the other handle on the sets; without it they are ours.
        drop(chooser);
        let route_sets =
            Arc::try_unwrap(route_sets).unwrap_or_else(|shared| RouteSets::clone(&shared));
        // The per-link table is the user's only if they asked for it.
        let link_bins = if self.link_bin_seconds.is_some() { loaded.link_bins } else { None };
        // Each trip under the mode it took: its choice (M5), none if it chose and had none to
        // choose (S269), else its stated mode.
        let taken: Vec<Option<Mode>> = (0..total_trips)
            .map(|i| {
                let trip = TripId::new(i);
                match itineraries.as_ref().zip(chosen.as_ref()) {
                    Some((it, c)) => match it.position(trip) {
                        Some(p) => match c.alt[p].as_ref() {
                            Some(a) => Some(a.mode),
                            // A stated mode stays the trip's mode, whatever became of it.
                            None if !it.chooses(p) => Some(self.trips.mode(trip)),
                            None => None,
                        },
                        None => Some(self.trips.mode(trip)),
                    },
                    None => Some(self.trips.mode(trip)),
                }
            })
            .collect();
        let (by_mode, no_mode) = mode_totals(&loaded.events, &self.trips, &self.travellers, &taken);
        let fallback_car: Vec<TripId> = match itineraries.as_ref().zip(chosen.as_ref()) {
            Some((it, c)) => it
                .trips
                .iter()
                .enumerate()
                .filter(|&(p, _)| c.alt[p].as_ref().is_some_and(|a| a.fallback))
                .map(|(_, &trip)| trip)
                .collect(),
            None => Vec::new(),
        };
        // Why the trips left with no alternative had none (S276), once, over those trips alone.
        let no_alternative = match itineraries.as_ref().zip(chosen.as_ref()) {
            Some((it, c)) => it.explain_none(
                &self.trips,
                &self.travellers,
                c,
                transit_arc.as_deref(),
                parking_arc.as_deref(),
                Some(&route_sets),
            ),
            None => Vec::new(),
        };
        let trip_outcomes = TripOutcomes::of(
            &loaded.events,
            &taken,
            &no_alternative,
            &loaded.at_end,
            &loaded.trip_parts,
        );
        let [bike_link_bins, walk_link_bins] = self.layer_bins(loaded.layer_vehicles);
        let transit = loaded.transit;
        let itinerary_result = match (&itineraries, &chosen) {
            (Some(itin), Some(c)) => Some(ItineraryResult::of(
                itin,
                c,
                &loaded.itinerary.rides,
                loaded.itinerary.replanned,
                if loaded.itinerary.mismatch_count > 0 {
                    loaded.itinerary.mismatch_sum / f64::from(loaded.itinerary.mismatch_count)
                } else {
                    f64::NAN
                },
            )),
            _ => None,
        };
        if let Some(services) = services_run.as_mut() {
            services.last = loaded.share;
            services.expected = dock_costs;
        }
        timings.lap("results_assembled", None, clock);
        self.timings = timings;
        Ok(RunResult {
            total_travel_time: loaded.total_travel_time,
            total_vehicle_time: loaded.total_vehicle_time,
            completion: loaded.completion,
            events: loaded.events,
            link_bins,
            route_sets: Some(route_sets),
            route_choices: Some(route_choices),
            bike_route_sets: bike_sets.map(|s| (*s).clone()),
            bike_route_choices,
            iterations: reports,
            converged,
            by_mode,
            services: services_run,
            no_mode,
            fallback_car,
            no_alternative,
            bike_link_bins,
            walk_link_bins,
            transit,
            parking: loaded.parking,
            itineraries: itinerary_result,
            gridlock: loaded.gridlock,
            reroutes: loaded.reroutes,
            routes_realised: loaded.routes_realised,
            link_peak_pcu: loaded.peak_pcu,
            trip_outcomes,
            gap_trips: last_gap,
        })
    }

    /// Load the whole demand once, each trip on the route `route_choices` gives it,
    /// recording per-link results in bins of `record_bins` seconds if asked.
    #[allow(clippy::too_many_arguments, reason = "the loading's inputs")]
    fn load_once(
        &mut self,
        trip_keys: &[RouteKey],
        route_sets: &RouteSets,
        route_choices: &RouteChoices,
        static_routes: &StaticRoutes,
        plan: LoadPlan<'_>,
        itin: Option<(&Itineraries, &Chosen)>,
        (car_ctx, expected): (&SearchContext<'_>, Option<&LinkTimes>),
        diagnostics: &mut Diagnostics,
    ) -> Loaded {
        let LoadPlan { record_bins, want_entry, level_override, active, disrupted } = plan;
        // The fixed times of reaching and leaving a vehicle (S295, S299): paid by every car and
        // bike trip as set (0 pays nothing). Whole seconds, the loading's clock; the car's start
        // is what every reading of its route's cost starts from (`Self::car_start_s`).
        let car_access_s = whole_seconds(self.mode_defaults.car_access_s);
        let car_egress_s = whole_seconds(self.mode_defaults.car_egress_s);
        let bike_access_s = whole_seconds(self.mode_defaults.bike_access_s);
        let bike_egress_s = whole_seconds(self.mode_defaults.bike_egress_s);
        // Per completed trip with a vehicle of its own: the trip, its seconds in the vehicle
        // and its fixed seconds (access, egress and parking).
        let mut trip_parts: Vec<(TripId, u32, u32)> = Vec::new();
        let mut total_vehicle_time = Duration::default();
        // The times the next choice reads are binned at the strategy's length, whatever bins
        // the flow map is recorded in (S232).
        let entry_bin_seconds = self.equilibration.cost_bin_seconds();
        let total_trips = self.trips.len();
        let mut queue: BinaryHeap<Reverse<EventKey>> = BinaryHeap::new();
        // Every iteration starts from where the day starts.
        self.vehicles = VehicleLocations::at_first_trip_origin(&self.travellers, &self.trips);

        // Every traveller's first trip is enqueued unconditionally, even one
        // who owns no car at all: `VehicleLocations::is_at_origin` already
        // reports "not available" correctly for a traveller who owns
        // nothing (`location` returns `None`), so filtering here would just
        // mean reimplementing that check twice — and, worse, would leave a
        // traveller who owns no car entirely uncounted rather than counted
        // as `no_vehicle_available`.
        for raw in 0..self.travellers.len() {
            if active.is_some_and(|a| !a[raw as usize]) {
                continue;
            }
            let traveller = TravellerId::new(raw);
            self.enqueue(&mut queue, self.travellers.first_trip(traveller));
        }

        let mut completion = TripCompletionStats { total_trips, ..TripCompletionStats::default() };
        let mut total_travel_time = Duration::ZERO;
        let mut events = Vec::new();
        // Only used under `FlowMotor::Ltm`: a trip whose vehicle is deferred
        // to the batch loading after the queue drains, keyed the same way
        // the vehicle itself is (`VehicleId::new(trip.raw())`) so the
        // result can be folded straight back by id, with the weight kept
        // alongside since `Vehicle` itself only carries PCU (weight already
        // multiplied in).
        let mut pending_ltm: Vec<(TripId, Vehicle, u32)> = Vec::new();
        // Only used under `FlowMotor::Level0` when per-link results are asked
        // for: level 0 visits vehicles one at a time, so they are kept to be
        // binned together afterwards.
        let mut level0_vehicles: Vec<Vehicle> = Vec::new();
        let mut link_bins: Option<LinkBins> = None;
        let mut entry_bins: Option<EntryTables> = None;
        let mut gridlock: Option<LockReport> = None;
        let mut reroutes: Vec<RerouteRecord> = Vec::new();
        let mut reroute_searches = 0_u32;
        let mut peak_pcu: Option<Vec<f64>> = None;
        let mut routes_realised: Vec<(TripId, Vec<LinkId>)> = Vec::new();
        // Where each car trip still travelling at the window's end was (its road link).
        let mut at_end: Vec<(TripId, LinkId)> = Vec::new();
        // Bike and walk vehicles, kept to be binned per layer when asked.
        let mut layer_vehicles: [Vec<Vehicle>; 2] = [Vec::new(), Vec::new()];
        // Itinerary trips (M4), executed once the loading has made the day's times; the
        // cars of those that drive, loaded with the others; vehicles fetched from parkings.
        let mut pending_itin: Vec<PendingItin> = Vec::new();
        let mut itin_cars: Vec<(usize, Vehicle)> = Vec::new();
        let mut parking_events: Vec<ParkingEvent> = Vec::new();
        // Shared-bike trips (S302), played on the fleet engine once the loading is done: their
        // walks and rides are static, only the docks' counts depend on the others. Each with
        // its ride's links on the bike layer, and, for a shared bike to transit, its place in
        // `pending_itin` (its transit part follows from where the engine left it).
        let mut pending_share: Vec<(ShareLeg, Vec<LinkId>, Option<usize>)> = Vec::new();

        while let Some(Reverse(key)) = queue.pop() {
            let trip = TripId::new(key.entity);
            let traveller = self.trips.traveller(trip);
            let origin = self.trips.origin(trip);
            let destination = self.trips.destination(trip);
            let departure = self.trips.departure(trip);

            // A trip that cannot be simulated — no vehicle, no path — does
            // not relocate the vehicle and does not stop the chain: the
            // *next* trip is enqueued regardless, and its own `is_at_origin`
            // check naturally reports "not available" too if the vehicle
            // never actually arrived where the file says the day continues
            // from. Stranding propagates by itself; nothing here needs to
            // decide to stop early.
            let mut mode = self.trips.mode(trip);
            // A trip that chose a direct leg (M5) runs as a trip of that mode, on the route it
            // chose (a walk route is its layer's one; a bike route its own, among several with
            // bike route sets).
            let mut chosen_route: Option<Vec<LinkId>> = None;
            // A car trip for want of any alternative (S271, W-1): no car of the traveller's.
            let mut fallback = false;
            if let Some((itineraries, chosen)) = itin {
                if let Some(position) = itineraries.position(trip) {
                    if let Some(alt) =
                        chosen.alt[position].as_ref().filter(|a| a.shape == Shape::Share)
                    {
                        let nodes = itineraries.nodes(position);
                        let leg = Leg {
                            service: 0,
                            trip: trip.raw(),
                            weight: self.travellers.weight(traveller),
                            request_s: f64::from(departure.get()) + f64::from(alt.access_walk_s),
                            origin: nodes.share_o,
                            destination: nodes.share_d,
                            pickup_dock: alt.share_docks.0,
                            return_dock: alt.share_docks.1,
                            ride_s: alt.vehicle_s,
                            egress_s: f64::from(alt.egress_walk_s),
                        };
                        pending_share.push((
                            ShareLeg { trip, leg, expected_dock_s: alt.dock_s },
                            alt.vehicle_links.clone(),
                            None,
                        ));
                        if let Some(next) = self.next_trip_of(traveller, trip) {
                            self.enqueue(&mut queue, next);
                        }
                        continue;
                    }
                    if let Some(alt) =
                        chosen.alt[position].as_ref().filter(|a| a.shape == Shape::Direct)
                    {
                        mode = alt.mode;
                        chosen_route = Some(alt.vehicle_links.clone());
                        fallback = alt.fallback;
                    }
                }
            }
            if let (Some((itineraries, chosen)), None) = (itin, &chosen_route) {
                if let Some(position) = itineraries.position(trip) {
                    self.start_itinerary(
                        trip,
                        position,
                        chosen,
                        record_bins.is_some(),
                        &mut ItinStart {
                            pending: &mut pending_itin,
                            cars: &mut itin_cars,
                            parking_events: &mut parking_events,
                            level0_vehicles: &mut level0_vehicles,
                            bike_vehicles: &mut layer_vehicles[0],
                            completion: &mut completion,
                            events: &mut events,
                        },
                        diagnostics,
                    );
                    // A shared bike to transit (S302): its ride to the hub's dock on the engine;
                    // the bike returned there (or the traveller arrived there on foot), the
                    // transit part follows from that second.
                    if let (Some(alt), Some(services)) = (
                        chosen.alt[position].as_ref().filter(|a| a.shape == Shape::ShareTransit),
                        &self.services,
                    ) {
                        let crate::services::ServiceParts::BikeDocked(docks, _) =
                            services[0].parts()
                        else {
                            unreachable!("a run's shared bikes are its docked service's")
                        };
                        let leg = Leg {
                            service: 0,
                            trip: trip.raw(),
                            weight: self.travellers.weight(traveller),
                            request_s: alt.vehicle_departure,
                            origin: itineraries.nodes(position).share_o,
                            destination: docks.walk_node[alt.share_docks.1 as usize],
                            pickup_dock: alt.share_docks.0,
                            return_dock: alt.share_docks.1,
                            ride_s: alt.vehicle_s,
                            egress_s: 0.0,
                        };
                        pending_share.push((
                            ShareLeg { trip, leg, expected_dock_s: alt.dock_s },
                            alt.vehicle_links.clone(),
                            Some(pending_itin.len() - 1),
                        ));
                    }
                    if let Some(next) = self.next_trip_of(traveller, trip) {
                        self.enqueue(&mut queue, next);
                    }
                    continue;
                }
            }
            if mode != Mode::Car {
                match self.static_outcome(
                    trip,
                    mode,
                    static_routes,
                    bike_access_s,
                    chosen_route.as_deref(),
                ) {
                    StaticOutcome::NotAvailable => {
                        diagnostics.record(DiagKey::new(
                            Category::Modelling,
                            local_codes::MODE_NOT_AVAILABLE,
                            Severity::Warning,
                            ElementRef::of(trip),
                        ));
                        completion.mode_not_available += 1;
                        events.push(EventRow::trip(departure, EventType::ModeNotAvailable, trip));
                    }
                    StaticOutcome::NoVehicle => {
                        diagnostics.record(DiagKey::new(
                            Category::Modelling,
                            local_codes::NO_VEHICLE_AVAILABLE,
                            Severity::Info,
                            ElementRef::of(trip),
                        ));
                        completion.no_vehicle_available += 1;
                        events.push(EventRow::trip(departure, EventType::NoVehicleAvailable, trip));
                    }
                    StaticOutcome::NoPath => {
                        diagnostics.record(DiagKey::new(
                            Category::Modelling,
                            codes::NO_FEASIBLE_PATH,
                            Severity::Warning,
                            ElementRef::of(trip),
                        ));
                        completion.no_feasible_path += 1;
                        events.push(EventRow::trip(departure, EventType::NoFeasiblePath, trip));
                    }
                    StaticOutcome::Here => {
                        completion.completed += 1;
                        events.push(EventRow::trip(departure, EventType::TripCompleted, trip));
                    }
                    StaticOutcome::Travelled { vehicle, trajectory, slot } => {
                        let weight = self.travellers.weight(traveller);
                        // A bike trip ends when the bike is parked and locked; a walk at its end.
                        let (access_s, egress_s) =
                            if slot == 0 { (bike_access_s, bike_egress_s) } else { (0, 0) };
                        let end = Second(trajectory.arrival().get() + egress_s);
                        if end <= self.window {
                            completion.completed += 1;
                            let ride = trajectory.total_travel_time();
                            total_travel_time += ride * f64::from(weight);
                            total_travel_time += Duration::from_clock(Second(access_s + egress_s))
                                * f64::from(weight);
                            if slot == 0 {
                                total_vehicle_time += ride * f64::from(weight);
                                trip_parts.push((
                                    trip,
                                    whole_seconds(ride.get()),
                                    access_s + egress_s,
                                ));
                            }
                            events.push(EventRow::trip(end, EventType::TripCompleted, trip));
                        } else {
                            completion.truncated += 1;
                            diagnostics.record(DiagKey::new(
                                Category::Modelling,
                                codes::TRIP_TRUNCATED,
                                Severity::Info,
                                ElementRef::of(trip),
                            ));
                            events.push(EventRow::trip(
                                self.window,
                                EventType::TripTruncated,
                                trip,
                            ));
                        }
                        if record_bins.is_some() {
                            layer_vehicles[slot].push(vehicle);
                        }
                    }
                }
            } else if !fallback
                && !self.vehicles.is_at_origin(
                    &self.travellers,
                    traveller,
                    VehicleKind::Car,
                    origin,
                )
            {
                diagnostics.record(DiagKey::new(
                    Category::Modelling,
                    local_codes::NO_VEHICLE_AVAILABLE,
                    Severity::Info,
                    ElementRef::of(trip),
                ));
                completion.no_vehicle_available += 1;
                events.push(EventRow::trip(departure, EventType::NoVehicleAvailable, trip));
            } else {
                let route_key = trip_keys[trip.index()];
                let route = if let Some(links) = chosen_route {
                    Some(links)
                } else if route_key.origin == route_key.destination {
                    Some(Vec::new())
                } else {
                    let chosen = route_choices.route[trip.index()];
                    (chosen != NO_ROUTE).then(|| {
                        route_sets
                            .route(chosen as usize)
                            .links
                            .iter()
                            .map(|&l| LinkId::new(l))
                            .collect::<Vec<LinkId>>()
                    })
                };
                match route {
                    None => {
                        diagnostics.record(DiagKey::new(
                            Category::Modelling,
                            codes::NO_FEASIBLE_PATH,
                            Severity::Warning,
                            ElementRef::of(trip),
                        ));
                        completion.no_feasible_path += 1;
                        events.push(EventRow::trip(departure, EventType::NoFeasiblePath, trip));
                    }
                    Some(path) if path.is_empty() => {
                        // Origin and destination snapped to the same node:
                        // nothing to traverse. `core_loading::Vehicle`
                        // requires a non-empty route, so this is handled
                        // here rather than passed down.
                        if !fallback {
                            self.vehicles.relocate(traveller, VehicleKind::Car, destination);
                        }
                        completion.completed += 1;
                        events.push(EventRow::trip(departure, EventType::TripCompleted, trip));
                    }
                    Some(path) => {
                        let weight = self.travellers.weight(traveller);
                        if !fallback {
                            self.vehicles.relocate(traveller, VehicleKind::Car, destination);
                        }

                        match &self.flow_motor {
                            FlowMotor::Level0 => {
                                let vehicle = Vehicle::new(
                                    VehicleId::new(traveller.raw()),
                                    path,
                                    Pcu(f64::from(weight)),
                                    Second(departure.get() + car_access_s),
                                );
                                let trajectory = traverse_free_flow(&vehicle, &self.network);
                                if record_bins.is_some() {
                                    level0_vehicles.push(vehicle);
                                }
                                // The trip ends when the traveller has left the car.
                                let end = Second(trajectory.arrival().get() + car_egress_s);
                                if end <= self.window {
                                    completion.completed += 1;
                                    let drive = trajectory.total_travel_time();
                                    total_travel_time += drive * f64::from(weight);
                                    total_travel_time +=
                                        Duration::from_clock(Second(car_access_s + car_egress_s))
                                            * f64::from(weight);
                                    total_vehicle_time += drive * f64::from(weight);
                                    trip_parts.push((
                                        trip,
                                        whole_seconds(drive.get()),
                                        car_access_s + car_egress_s,
                                    ));
                                    events.push(EventRow::trip(
                                        end,
                                        EventType::TripCompleted,
                                        trip,
                                    ));
                                } else {
                                    completion.truncated += 1;
                                    diagnostics.record(DiagKey::new(
                                        Category::Modelling,
                                        codes::TRIP_TRUNCATED,
                                        Severity::Info,
                                        ElementRef::of(trip),
                                    ));
                                    events.push(EventRow::trip(
                                        self.window,
                                        EventType::TripTruncated,
                                        trip,
                                    ));
                                    // The link it had not left by the window's end.
                                    if let Some(t) =
                                        trajectory.links.iter().find(|t| t.exit > self.window)
                                    {
                                        at_end.push((trip, t.link));
                                    }
                                }
                            }
                            FlowMotor::Ltm { .. } => {
                                // One vehicle identity per *trip*, not per
                                // traveller: under the LTM a traveller's
                                // successive trips can be simultaneously
                                // in flight (vehicle relocation is instant,
                                // decoupled from actual arrival, same as
                                // under `Level0` above), so reusing
                                // `traveller.raw()` the way `Level0` safely
                                // can — because it only ever has one trip
                                // in flight at a time — would collide.
                                let vehicle = Vehicle::new(
                                    VehicleId::new(trip.raw()),
                                    path,
                                    Pcu(f64::from(weight)),
                                    Second(departure.get() + car_access_s),
                                );
                                pending_ltm.push((trip, vehicle, weight));
                            }
                            FlowMotor::Static { .. } => {
                                // Level 1 (S294): on its route at the equilibrium's times. A
                                // static model has no end of the day: every trip arrives.
                                let vehicle = Vehicle::new(
                                    VehicleId::new(trip.raw()),
                                    path,
                                    Pcu(f64::from(weight)),
                                    Second(departure.get() + car_access_s),
                                );
                                let times = self.static_solution.as_ref().map(|s| s.times_s());
                                let trajectory = traverse_timed(
                                    &vehicle,
                                    &self.network,
                                    times.expect("level 1 solves before it loads"),
                                );
                                completion.completed += 1;
                                let drive = trajectory.total_travel_time();
                                total_travel_time += drive * f64::from(weight);
                                total_travel_time +=
                                    Duration::from_clock(Second(car_access_s + car_egress_s))
                                        * f64::from(weight);
                                total_vehicle_time += drive * f64::from(weight);
                                trip_parts.push((
                                    trip,
                                    whole_seconds(drive.get()),
                                    car_access_s + car_egress_s,
                                ));
                                events.push(EventRow::trip(
                                    Second(trajectory.arrival().get() + car_egress_s),
                                    EventType::TripCompleted,
                                    trip,
                                ));
                            }
                        }
                    }
                }
            }

            if let Some(next) = self.next_trip_of(traveller, trip) {
                self.enqueue(&mut queue, next);
            }
        }

        // The day's buses, if they ride this network's roads (S199): loaded with the cars,
        // and their times read back for the passengers.
        let transit = self.transit.clone();
        let disruptions = &self.disruptions;
        let window_end_s = f64::from(self.window.get());
        let bus_load = transit.as_ref().filter(|t| t.rides(&self.network)).map(|t| {
            let timetable = t.timetable();
            t.bus_load(total_trips, window_end_s, &|run| {
                if disrupted { disruptions.effect_on(timetable, run) } else { None }
            })
        });
        let mut bus_times = None;
        if let (FlowMotor::Level0, Some(load), Some(transit)) =
            (&self.flow_motor, &bus_load, &transit)
        {
            // At free flow a leg follows its leader alone, in chain order.
            let follows: HashMap<usize, (usize, f64, f64)> =
                load.chains.iter().map(|&(v, a, w, nb)| (v, (a, w, nb))).collect();
            let mut done: Vec<Option<Trajectory>> = vec![None; load.vehicles.len()];
            let window = f64::from(self.window.get());
            for (i, vehicle) in load.vehicles.iter().enumerate() {
                let start = match follows.get(&i) {
                    Some(&(a, w, nb)) => match &done[a] {
                        Some(leader) => (f64::from(leader.arrival().get()) + w).max(nb),
                        None => continue,
                    },
                    None => f64::from(vehicle.departure.get()),
                };
                if start >= window {
                    continue;
                }
                #[allow(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "a second of the day"
                )]
                let leg = Vehicle { departure: Second(start as u32), ..vehicle.clone() };
                let trajectory = traverse_free_flow(&leg, &self.network);
                if trajectory.arrival() <= self.window {
                    done[i] = Some(trajectory);
                }
                if record_bins.is_some() {
                    level0_vehicles.push(leg);
                }
            }
            bus_times = Some(transit.realised(load, |i| done[i].as_ref()));
        }

        if let FlowMotor::Ltm { turns, step, level } = &self.flow_motor {
            let level = &level_override.unwrap_or(*level);
            // The vehicles move into the loading, their ids stay (S294).
            let mut vehicles: Vec<Vehicle> =
                Vec::with_capacity(pending_ltm.len() + itin_cars.len());
            let pending_ltm: Vec<(TripId, VehicleId, u32)> = pending_ltm
                .into_iter()
                .map(|(trip, vehicle, weight)| {
                    let id = vehicle.id;
                    vehicles.push(vehicle);
                    (trip, id, weight)
                })
                .collect();
            let itin_cars: Vec<(usize, VehicleId)> = std::mem::take(&mut itin_cars)
                .into_iter()
                .map(|(i, vehicle)| {
                    let id = vehicle.id;
                    vehicles.push(vehicle);
                    (i, id)
                })
                .collect();
            let window = Duration::from_clock(self.window);
            // Buses (S199) ride chained from stop to stop; cars alone have no chains. One loading
            // either way (the same loading as `run_ltm_recorded` and its kin when there are none).
            let cars = vehicles.len();
            let mut chains: Vec<Chain> = Vec::new();
            if let Some(load) = &bus_load {
                vehicles.extend(load.vehicles.iter().cloned());
                chains.extend(load.chains.iter().map(|&(v, a, wait, not_before)| Chain {
                    vehicle: cars + v,
                    after: cars + a,
                    wait,
                    not_before,
                }));
            }
            let recording = match record_bins {
                Some(b) if want_entry => Recording::BinsAndEntry(b, entry_bin_seconds),
                Some(b) => Recording::Bins(b),
                None => Recording::Trajectories,
            };
            // The rerouting setting settled for the run's own level (S280, C1): a warm-up
            // loading of a level-4 run keeps the run's rule, as the fingerprint does. At the
            // point queue only a loading with road disruptions the travellers did not know of
            // re-routes, and there only at closures (S300).
            let capacity_changes =
                if disrupted { self.disruptions.capacity_changes() } else { Vec::new() };
            let unknown = !self.disruptions.known && !capacity_changes.is_empty();
            let o = self.loading.resolved(self.level(), unknown);
            let rerouting = o.reroutes(self.level(), unknown);
            let rules = Rules {
                priority: o.priority,
                reroute: rerouting.then(|| RerouteRule {
                    after_s: o.reroute_after_s,
                    max: u8::try_from(o.reroute_max).unwrap_or(u8::MAX),
                    closures_only: *level == FidelityLevel::PointQueue,
                }),
                pocket_length_m: o.pocket_length_m,
                node_inflow: o.node_inflow,
                origin_inflow: o.origin_inflow,
                start_s: f64::from(self.window_start.get()),
                capacity_changes,
            };
            let reroute_cost = self.reroute_cost.clone();
            let mut rerouter = rerouting.then(|| {
                Rerouter::new(car_ctx, expected, &o, total_trips, reroute_cost.as_deref())
            });
            let out = run_ltm_chained(
                &self.network,
                turns,
                &vehicles,
                &chains,
                window,
                *step,
                *level,
                recording,
                rules,
                rerouter.as_mut().map(|r| r as &mut dyn Reroute),
            );
            reroutes = out.reroutes;
            reroute_searches = rerouter.as_ref().map_or(0, |r| r.searches);
            peak_pcu = Some(out.peak_occupancy);
            let (trajectories, bins, entry) = (out.trajectories, out.link_bins, out.entry);
            // Where each vehicle still travelling at the end was (a car's vehicle id is its
            // trip's index, an itinerary car's likewise; buses are not trips).
            let unfinished: HashMap<VehicleId, LinkId> = out.unfinished.into_iter().collect();
            for &(i, id) in &itin_cars {
                if let Some(&link) = unfinished.get(&id) {
                    at_end.push((pending_itin[i].trip, link));
                }
            }
            for cycle in &out.lock.loops {
                diagnostics.record(DiagKey::new(
                    Category::Modelling,
                    codes::GRIDLOCK,
                    Severity::Warning,
                    ElementRef::of(cycle[0]),
                ));
            }
            gridlock = Some(out.lock);
            link_bins = bins;
            entry_bins = entry;
            let by_vehicle: HashMap<VehicleId, _> =
                trajectories.into_iter().map(|t| (t.vehicle, t)).collect();
            // The realised route of each trip that re-routed and arrived: its trajectory's links
            // (a car's vehicle id is its trip's index). The others followed their plan (S213).
            let mut seen: Vec<VehicleId> = reroutes.iter().map(|r| r.vehicle).collect();
            seen.sort_unstable();
            seen.dedup();
            for v in seen {
                if let Some(t) = by_vehicle.get(&v) {
                    let links = t.links.iter().map(|x| x.link).collect();
                    routes_realised.push((TripId::new(v.raw()), links));
                }
            }
            for &(i, id) in &itin_cars {
                pending_itin[i].vehicle_arrival =
                    by_vehicle.get(&id).map(|t| f64::from(t.arrival().get()));
            }
            if let (Some(load), Some(transit)) = (&bus_load, &transit) {
                let id = |i: usize| {
                    VehicleId::new(total_trips + u32::try_from(i).expect("vehicles fit u32"))
                };
                bus_times = Some(transit.realised(load, |i| by_vehicle.get(&id(i))));
            }

            for (trip, id, weight) in pending_ltm {
                // The trip ends when the traveller has left the car: a car that arrived too
                // late for that before the window's end leaves its trip truncated, off the roads.
                match by_vehicle
                    .get(&id)
                    .filter(|t| Second(t.arrival().get() + car_egress_s) <= self.window)
                {
                    Some(trajectory) => {
                        completion.completed += 1;
                        let drive = trajectory.total_travel_time();
                        total_travel_time += drive * f64::from(weight);
                        total_travel_time +=
                            Duration::from_clock(Second(car_access_s + car_egress_s))
                                * f64::from(weight);
                        total_vehicle_time += drive * f64::from(weight);
                        trip_parts.push((
                            trip,
                            whole_seconds(drive.get()),
                            car_access_s + car_egress_s,
                        ));
                        events.push(EventRow::trip(
                            Second(trajectory.arrival().get() + car_egress_s),
                            EventType::TripCompleted,
                            trip,
                        ));
                    }
                    None => {
                        completion.truncated += 1;
                        diagnostics.record(DiagKey::new(
                            Category::Modelling,
                            codes::TRIP_TRUNCATED,
                            Severity::Info,
                            ElementRef::of(trip),
                        ));
                        events.push(EventRow::trip(self.window, EventType::TripTruncated, trip));
                        if let Some(&link) = unfinished.get(&id) {
                            at_end.push((trip, link));
                        }
                    }
                }
            }
        }

        // Level 1 (S294): the equilibrium's flows over the period, not the traversals (everyone
        // sets off at the period's start, which would draw a wave through the network).
        if let (Some(solution), Some(bin_seconds)) = (&self.static_solution, record_bins) {
            link_bins = Some(solution.link_bins(bin_seconds, self.window_start.get()));
        }
        if let (FlowMotor::Level0, Some(bin_seconds)) = (&self.flow_motor, record_bins) {
            let window = f64::from(self.window.get());
            if want_entry {
                let (_, exit, entry) = load_level_0_recorded(
                    &level0_vehicles,
                    &self.network,
                    window,
                    bin_seconds,
                    entry_bin_seconds,
                );
                link_bins = Some(exit);
                entry_bins = Some(entry);
            } else {
                link_bins = Some(
                    load_level_0_binned(&level0_vehicles, &self.network, window, bin_seconds).1,
                );
            }
        }

        // Shared bikes (S302): their legs on the fleet engine, with no operator, before the
        // transit parts, which a shared bike to transit starts from where the engine left it; a
        // pickup that fails walks on, to the destination or to the hub's dock.
        let mut share_day: Option<(ShareDay, Vec<ShareRest>)> = None;
        if let (Some(services), false) = (self.services.clone(), pending_share.is_empty()) {
            pending_share.sort_by_key(|(l, _, _)| l.trip.raw());
            let (legs, rest): (Vec<ShareLeg>, Vec<ShareRest>) =
                pending_share.into_iter().map(|(l, links, at)| (l, (links, at))).unzip();
            let start = f64::from(self.window_start.get());
            let end = f64::from(self.window.get());
            let day = crate::services::in_run::play(
                &services,
                &self.layers,
                legs,
                (start, end),
                self.master_seed,
            );
            for ((_, o), (_, at)) in day.legs.iter().zip(&day.outcome.legs).zip(&rest) {
                if let Some(i) = *at {
                    pending_itin[i].vehicle_arrival =
                        (o.arrived_s.is_finite() && o.arrived_s <= end).then_some(o.arrived_s);
                }
            }
            share_day = Some((day, rest));
        }

        // Transit, park-and-ride and bike-and-ride trips, on the day's times (M4).
        let mut transit_result = None;
        let mut parking_result = None;
        let mut availability_real = None;
        let mut itinerary = ItineraryTally::default();
        if let Some(transit) = transit {
            // The disruptions (S239): cancelled runs run for no one, delayed ones by the schedule
            // late (a delayed bus on the roads carries its delay already).
            if disrupted && !self.disruptions.transit.is_empty() {
                let timetable = transit.timetable();
                let on_roads: std::collections::HashSet<u32> = bus_load
                    .as_ref()
                    .map(|l| l.runs().map(|r| r.raw()).collect())
                    .unwrap_or_default();
                let mut times = bus_times.take().unwrap_or_else(|| timetable.scheduled().clone());
                self.disruptions
                    .apply_to_times(timetable, &mut times, &|r| on_roads.contains(&r.raw()));
                bus_times = Some(times);
            }
            // The timetable the travellers planned on, if this loading ran another (buses on the
            // roads, or disruptions they did not know of): each transit part is followed on it
            // too, for the gap's planned-against-realised part (S299). The schedule, with the
            // disruptions they knew of.
            let differs = bus_load.is_some()
                || (disrupted && !self.disruptions.known && !self.disruptions.transit.is_empty());
            let known_transit =
                disrupted && self.disruptions.known && !self.disruptions.transit.is_empty();
            let planned_known = (differs && known_transit).then(|| {
                let timetable = transit.timetable();
                let mut times = timetable.scheduled().clone();
                self.disruptions.apply_to_times(timetable, &mut times, &|_| false);
                transit.on_times(&times)
            });
            let planned =
                differs.then(|| planned_known.as_ref().unwrap_or_else(|| transit.scheduled()));
            // On the loading's times if the buses rode, else on the schedule.
            let realised = bus_times.as_ref().map(|times| transit.on_times(times));
            let data = realised.as_ref().unwrap_or_else(|| transit.scheduled());
            let buses =
                bus_load.as_ref().zip(bus_times.as_ref()).map(|(l, t)| transit.bus_summary(l, t));
            let times = bus_times.unwrap_or_else(|| transit.timetable().scheduled().clone());
            let mut result = TransitResult::empty(times);
            result.buses = buses;
            if let Some((itineraries, chosen)) = itin {
                // Walks are binned on the walk layer when it is the run's own.
                let bin_walks = record_bins.is_some()
                    && self
                        .layers
                        .walk
                        .as_ref()
                        .is_some_and(|w| Arc::ptr_eq(w.network(), transit.walk()));
                let (parking, avail) =
                    self.park_arrivals(&pending_itin, chosen, &mut parking_events);
                parking_result = parking;
                let mut walk_paths = std::mem::take(&mut self.walk_paths);
                let executed = self.execute_itineraries(
                    &transit,
                    (data, planned),
                    itineraries,
                    chosen,
                    &pending_itin,
                    avail.as_deref(),
                    (bin_walks, &mut walk_paths),
                );
                self.walk_paths = walk_paths;
                itinerary.rides = vec![0; itineraries.trips.len()];
                for (pend, done) in pending_itin.iter().zip(executed) {
                    let trip = pend.trip;
                    let departure = self.trips.departure(trip);
                    let weight = f64::from(self.travellers.weight(self.trips.traveller(trip)));
                    if let Some(f) = &done.followed {
                        itinerary.replanned += u32::from(f.replanned);
                        for leg in &f.rides {
                            if let JourneyLeg::Ride { board_call, alight_call, .. } = *leg {
                                result.boardings[board_call as usize] += weight;
                                result.alightings[alight_call as usize] += weight;
                            }
                        }
                        itinerary.rides[pend.position] =
                            u32::try_from(f.rides.len()).expect("few rides");
                    }
                    if let Some(m) = done.mismatch {
                        itinerary.mismatch_sum += m;
                        itinerary.mismatch_count += 1;
                    }
                    if let (Some(e), ItinOutcome::Arrived(at)) = (done.plan_error, &done.outcome) {
                        if Second(*at) <= self.window {
                            itinerary.plan_error_sum += weight * e;
                            itinerary.plan_error_weight += weight;
                        }
                    }
                    for (links, start) in done.walks {
                        layer_vehicles[1].push(Vehicle::new(
                            VehicleId::new(trip.raw()),
                            links,
                            Pcu(weight),
                            Second(start),
                        ));
                    }
                    match done.outcome {
                        ItinOutcome::Arrived(at) if Second(at) <= self.window => {
                            completion.completed += 1;
                            total_travel_time += (Duration::from_clock(Second(at))
                                - Duration::from_clock(departure))
                                * weight;
                            if done.parts != (0, 0) {
                                total_vehicle_time +=
                                    Duration::from_clock(Second(done.parts.0)) * weight;
                                trip_parts.push((trip, done.parts.0, done.parts.1));
                            }
                            events.push(EventRow::trip(Second(at), EventType::TripCompleted, trip));
                        }
                        ItinOutcome::Arrived(_) | ItinOutcome::Truncated => {
                            completion.truncated += 1;
                            diagnostics.record(DiagKey::new(
                                Category::Modelling,
                                codes::TRIP_TRUNCATED,
                                Severity::Info,
                                ElementRef::of(trip),
                            ));
                            events.push(EventRow::trip(
                                self.window,
                                EventType::TripTruncated,
                                trip,
                            ));
                        }
                        ItinOutcome::NoPath => {
                            diagnostics.record(DiagKey::new(
                                Category::Modelling,
                                codes::NO_FEASIBLE_PATH,
                                Severity::Warning,
                                ElementRef::of(trip),
                            ));
                            completion.no_feasible_path += 1;
                            events.push(EventRow::trip(departure, EventType::NoFeasiblePath, trip));
                        }
                    }
                }
                availability_real = avail;
            }
            transit_result = Some(result);
        }

        // The shared-bike trips' ends (S302): a shared bike alone ends where the engine left it;
        // a shared bike to transit ended with its transit part, above.
        let mut share = None;
        if let (Some(services), Some((day, rest))) = (self.services.clone(), share_day) {
            let end = f64::from(self.window.get());
            let pickup_s = match services[0].parts() {
                crate::services::ServiceParts::BikeDocked(_, d) => d.pickup_s,
                crate::services::ServiceParts::Taxi(..) => 0.0,
            };
            for ((l, o), (links, at)) in day.legs.iter().zip(&day.outcome.legs).zip(rest) {
                let trip = l.trip;
                let weight = f64::from(l.leg.weight);
                let departure = self.trips.departure(trip);
                // The shared bike on the bike layer's per-link results, as one's own: a ride
                // between the planned docks, with no redirection.
                let planned = o.picked == Some(l.leg.pickup_dock)
                    && o.returned == Some(l.leg.return_dock)
                    && o.redirections == 0;
                if at.is_some() {
                    if record_bins.is_some() && planned && !links.is_empty() && o.arrived_s <= end {
                        #[allow(
                            clippy::cast_possible_truncation,
                            clippy::cast_sign_loss,
                            reason = "s"
                        )]
                        let leaves = Second((o.picked_s + pickup_s).floor() as u32);
                        layer_vehicles[0].push(Vehicle::new(
                            VehicleId::new(trip.raw()),
                            links,
                            Pcu(weight),
                            leaves,
                        ));
                    }
                    continue;
                }
                if o.arrived_s.is_finite() && o.arrived_s <= end {
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "s")]
                    let at = Second((o.arrived_s.floor() as u32).max(departure.get()));
                    completion.completed += 1;
                    total_travel_time +=
                        (Duration::from_clock(at) - Duration::from_clock(departure)) * weight;
                    events.push(EventRow::trip(at, EventType::TripCompleted, trip));
                    if record_bins.is_some() && planned && !links.is_empty() {
                        #[allow(
                            clippy::cast_possible_truncation,
                            clippy::cast_sign_loss,
                            reason = "s"
                        )]
                        let leaves = Second((o.picked_s + pickup_s).floor() as u32);
                        layer_vehicles[0].push(Vehicle::new(
                            VehicleId::new(trip.raw()),
                            links,
                            Pcu(weight),
                            leaves,
                        ));
                    }
                } else {
                    completion.truncated += 1;
                    diagnostics.record(DiagKey::new(
                        Category::Modelling,
                        codes::TRIP_TRUNCATED,
                        Severity::Info,
                        ElementRef::of(trip),
                    ));
                    events.push(EventRow::trip(self.window, EventType::TripTruncated, trip));
                }
            }
            share = Some(day);
        }

        // The bike and walk layers' vehicles, binned once the run knows which loading it keeps.
        let layer_vehicles = record_bins.map(|b| (b, layer_vehicles));

        Loaded {
            total_travel_time,
            completion,
            events,
            link_bins,
            entry_bins,
            layer_vehicles,
            transit: transit_result,
            parking: parking_result,
            availability: availability_real,
            itinerary,
            total_vehicle_time,
            trip_parts,
            gridlock,
            reroutes,
            routes_realised,
            reroute_searches,
            peak_pcu,
            at_end,
            share,
        }
    }

    /// The bike and walk layers' per-link results, from the vehicles a loading kept and the bin
    /// length (`None`: none asked for): binned for the loading whose results the run keeps
    /// alone (S294).
    fn layer_bins(&self, vehicles: Option<(u32, [Vec<Vehicle>; 2])>) -> [Option<LinkBins>; 2] {
        let mut layer_bins: [Option<LinkBins>; 2] = [None, None];
        if let Some((bin_seconds, layer_vehicles)) = vehicles {
            let window = f64::from(self.window.get());
            for (slot, vehicles) in layer_vehicles.iter().enumerate() {
                let layer =
                    if slot == 0 { self.layers.bike.as_ref() } else { self.layers.walk.as_ref() };
                if let (Some(setup), false) = (layer, vehicles.is_empty()) {
                    let graph = setup.network().network();
                    layer_bins[slot] = Some(
                        load_timed_binned(vehicles, graph, setup.seconds(), window, bin_seconds).1,
                    );
                }
            }
        }
        layer_bins
    }

    /// Decide and make one bike or walk trip: whether its mode can run, whether
    /// the traveller's bike is at the origin, and its traversal of its route on
    /// the layer. A bike that travels is left at the destination.
    /// Start an itinerary trip at its departure: check its vehicle is where
    /// its choice needs it, move the vehicle (parked out, fetched back), put the
    /// vehicle leg on its layer — a car into the loading — and leave the rest for
    /// after the loading.
    #[allow(clippy::too_many_lines, reason = "one match over the itinerary's shapes")]
    fn start_itinerary(
        &mut self,
        trip: TripId,
        position: usize,
        chosen: &Chosen,
        want_bins: bool,
        out: &mut ItinStart<'_>,
        diagnostics: &mut Diagnostics,
    ) {
        let traveller = self.trips.traveller(trip);
        let departure = self.trips.departure(trip);
        let (origin, destination) = (self.trips.origin(trip), self.trips.destination(trip));
        let weight = self.travellers.weight(traveller);
        let stated = parking_kind_of(self.trips.mode(trip));
        let no_vehicle = |out: &mut ItinStart<'_>, diagnostics: &mut Diagnostics| {
            diagnostics.record(DiagKey::new(
                Category::Modelling,
                local_codes::NO_VEHICLE_AVAILABLE,
                Severity::Info,
                ElementRef::of(trip),
            ));
            out.completion.no_vehicle_available += 1;
            out.events.push(EventRow::trip(departure, EventType::NoVehicleAvailable, trip));
        };
        let Some(alt) = chosen.alt[position].as_ref() else {
            // No alternative: the vehicle was not where it could be used, or no journey.
            let usable = stated.map(vehicle_kind).is_none_or(|vk| {
                self.vehicles.is_at_origin(&self.travellers, traveller, vk, origin)
                    || self.vehicles.parking(&self.travellers, traveller, vk).is_some()
            });
            if usable {
                diagnostics.record(DiagKey::new(
                    Category::Modelling,
                    codes::NO_FEASIBLE_PATH,
                    Severity::Warning,
                    ElementRef::of(trip),
                ));
                out.completion.no_feasible_path += 1;
                out.events.push(EventRow::trip(departure, EventType::NoFeasiblePath, trip));
            } else {
                no_vehicle(out, diagnostics);
            }
            return;
        };
        let index = out.pending.len();
        // (A direct leg never comes here: the loop runs it as a trip of its mode.)
        let vehicle_arrival = match (alt.shape, parking_kind_of(alt.mode)) {
            // A shared bike to transit: its arrival at the hub's dock comes from the engine.
            (Shape::Transit | Shape::Direct | Shape::Share | Shape::ShareTransit, _)
            | (_, None) => None,
            (Shape::Out, Some(k)) => {
                let vk = vehicle_kind(k);
                let parking = self.parking.clone().expect("an out itinerary has parkings");
                if !self.vehicles.is_at_origin(&self.travellers, traveller, vk, origin) {
                    no_vehicle(out, diagnostics);
                    return;
                }
                self.vehicles.park(traveller, vk, alt.parking, parking.position(alt.parking));
                // The drive starts once the vehicle is reached (the choice counted the same).
                let start = f64::from(departure.get()) + self.mode_defaults.access_s(k);
                self.drive(trip, alt, k, start, weight, index, want_bins, out)
            }
            (Shape::Back(p), Some(k)) => {
                let vk = vehicle_kind(k);
                if self.vehicles.parking(&self.travellers, traveller, vk) != Some(p) {
                    no_vehicle(out, diagnostics);
                    return;
                }
                self.vehicles.relocate(traveller, vk, destination);
                out.parking_events.push((alt.vehicle_departure, p, -f64::from(weight)));
                self.drive(trip, alt, k, alt.vehicle_departure, weight, index, want_bins, out)
            }
        };
        out.pending.push(PendingItin { trip, position, vehicle_arrival });
    }

    /// The vehicle leg of an itinerary, leaving at `start`: a car at free flow now
    /// (level 0) or into the loading (its arrival then comes after it), a bike on
    /// its layer now. Its arrival, if known now.
    #[allow(clippy::too_many_arguments, reason = "the leg, and where its pieces go")]
    fn drive(
        &self,
        trip: TripId,
        alt: &itinerary_choice::Alternative,
        kind: ParkingKind,
        start: f64,
        weight: u32,
        index: usize,
        want_bins: bool,
        out: &mut ItinStart<'_>,
    ) -> Option<f64> {
        if alt.vehicle_links.is_empty() {
            return Some(start);
        }
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a second of the day"
        )]
        let at = Second(start.max(0.0).floor() as u32);
        let vehicle = Vehicle::new(
            VehicleId::new(trip.raw()),
            alt.vehicle_links.clone(),
            Pcu(f64::from(weight)),
            at,
        );
        match kind {
            ParkingKind::Car => match &self.flow_motor {
                FlowMotor::Level0 | FlowMotor::Static { .. } => {
                    let trajectory = traverse_free_flow(&vehicle, &self.network);
                    if want_bins {
                        out.level0_vehicles.push(vehicle);
                    }
                    Some(f64::from(trajectory.arrival().get()))
                }
                FlowMotor::Ltm { .. } => {
                    out.cars.push((index, vehicle));
                    None
                }
            },
            ParkingKind::Bike => {
                let layer = self.layers.bike.as_ref()?;
                let trajectory =
                    traverse_timed(&vehicle, layer.network().network(), layer.seconds());
                if want_bins {
                    out.bike_vehicles.push(vehicle);
                }
                Some(f64::from(trajectory.arrival().get()))
            }
        }
    }

    /// Park the vehicles of the out itineraries that reached their parking, tally
    /// the parkings over the day, and say what that cost: the parking result and
    /// the realised availability per parking and bin (none without parkings).
    fn park_arrivals(
        &self,
        pending: &[PendingItin],
        chosen: &Chosen,
        events: &mut Vec<ParkingEvent>,
    ) -> (Option<ParkingResult>, Option<Vec<f64>>) {
        let Some(parking) = self.parking.as_deref() else { return (None, None) };
        let mut arrivals: Vec<(usize, usize)> = Vec::new();
        for (i, pend) in pending.iter().enumerate() {
            let alt = chosen.alt[pend.position].as_ref().expect("a pending trip has its choice");
            if let (Shape::Out, Some(t)) = (alt.shape, pend.vehicle_arrival) {
                let w = f64::from(self.travellers.weight(self.trips.traveller(pend.trip)));
                arrivals.push((i, events.len()));
                events.push((t, alt.parking, w));
            }
        }
        let window = f64::from(self.window.get());
        let start = f64::from(self.window_start.get());
        let (bins, full) = parking_mod::tally(parking, events, start, window);
        let avail = parking_mod::availability(&bins);
        let (mut total, mut overflow, mut mismatch) = (0.0, 0.0, 0.0);
        let mut by_kind = [parking_mod::ParkingTotals::default(); 2];
        let mut kind_mismatch = [0.0; 2];
        for &(i, e) in &arrivals {
            let alt = chosen.alt[pending[i].position].as_ref().expect("chosen");
            let (t, p, w) = events[e];
            let k = parking.kind(p).index();
            total += w;
            by_kind[k].arrivals += w;
            if full[e] {
                overflow += w;
                by_kind[k].overflow_arrivals += w;
            }
            let paid = parking.defaults().parking_seconds(
                parking.kind(p),
                parking_mod::availability_at(&avail, &bins, p, t),
                self.mode_defaults.egress_s(parking.kind(p)),
            );
            mismatch += w * (paid - alt.parking_s).abs();
            kind_mismatch[k] += w * (paid - alt.parking_s).abs();
        }
        // What is still parked when the window ends.
        let mut stock: Vec<f64> = (0..parking.count())
            .map(|p| f64::from(parking.initial_occupancy(u32::try_from(p).expect("fits u32"))))
            .collect();
        for &(_, p, change) in events.iter() {
            stock[p as usize] += change;
        }
        for (p, &left) in stock.iter().enumerate() {
            let p = u32::try_from(p).expect("parkings fit u32");
            let k = parking.kind(p).index();
            by_kind[k].left_at_end += (left - f64::from(parking.initial_occupancy(p))).max(0.0);
        }
        for k in 0..2 {
            by_kind[k].mismatch_s = if by_kind[k].arrivals > 0.0 {
                kind_mismatch[k] / by_kind[k].arrivals
            } else {
                f64::NAN
            };
        }
        let result = ParkingResult {
            bins,
            by_kind,
            arrivals: total,
            overflow_arrivals: overflow,
            mismatch_s: if total > 0.0 { mismatch / total } else { f64::NAN },
        };
        (Some(result), Some(avail))
    }

    /// Execute every pending itinerary's transit part on `data`: see
    /// [`itinerary_choice::follow`]. In parallel, in fixed chunks. With `bin_walks`, each walk
    /// leg's links, from `walk_paths` or searched once and kept there. With `planned` (the
    /// timetable the travellers planned on, when `data` is another), each transit part is
    /// followed on it too, from the same moment at its first stop, and the difference kept.
    #[allow(clippy::too_many_arguments, reason = "the loading's pieces")]
    #[allow(clippy::too_many_lines, reason = "one pass: follow, then the walks")]
    fn execute_itineraries(
        &self,
        transit: &TransitSetup,
        (data, planned): (
            &openmobisim_core_transit::RaptorData,
            Option<&openmobisim_core_transit::RaptorData>,
        ),
        itineraries: &Itineraries,
        chosen: &Chosen,
        pending: &[PendingItin],
        avail: Option<&[f64]>,
        (bin_walks, walk_paths): (bool, &mut WalkPaths),
    ) -> Vec<Executed> {
        let parking = self.parking.as_deref();
        let trips = &*self.trips;
        let walk = transit.walk().network();
        let seconds = transit.walk_link_seconds();
        let d = transit.defaults();
        let transfer = transit.transfer_s();
        let walk_bound = transit
            .access_search_s()
            .max(d.transfer_walk_max_s)
            .max(parking.map_or(0.0, |p| p.defaults().walk_max_s))
            + 1.0;
        let bins = parking.zip(avail).map(|(p, _)| {
            let (start, end) = (self.window_start.get(), self.window.get());
            parking_mod::tally(p, &[], f64::from(start), f64::from(end)).0
        });
        // Each trip followed, with its walk legs as (from, to, start); their links after.
        let followed: Vec<(Executed, Vec<WalkLeg>)> = par_map(
            pending,
            itinerary_choice::SCRATCH_CHUNK,
            || {
                (
                    Reach::new(walk, seconds),
                    Raptor::new(data, d.max_rides as usize),
                    planned.map(|p| Raptor::new(p, d.max_rides as usize)),
                )
            },
            |(reach, raptor, raptor_planned), pend| {
                let alt =
                    chosen.alt[pend.position].as_ref().expect("a pending trip has its choice");
                let nodes = itineraries.nodes(pend.position);
                let departure = trips.departure(pend.trip).get();
                let truncated = || {
                    let e = Executed {
                        outcome: ItinOutcome::Truncated,
                        followed: None,
                        mismatch: None,
                        plan_error: None,
                        walks: Vec::new(),
                        parts: (0, 0),
                    };
                    (e, Vec::new())
                };
                let mut paid_at_parking: Option<f64> = None;
                let (at_first, start, end) = match alt.shape {
                    // (A direct leg or a shared bike is never pending: the loop runs it.)
                    Shape::Transit | Shape::Direct | Shape::Share => {
                        (departure.saturating_add(alt.access_walk_s), nodes.walk_o, nodes.walk_d)
                    }
                    Shape::Out => {
                        let (Some(t), Some(p)) = (pend.vehicle_arrival, parking) else {
                            return truncated();
                        };
                        let a = match (avail, &bins) {
                            (Some(v), Some(b)) => {
                                parking_mod::availability_at(v, b, alt.parking, t)
                            }
                            _ => 1.0,
                        };
                        let paid = p.defaults().parking_seconds(
                            p.kind(alt.parking),
                            a,
                            self.mode_defaults.egress_s(p.kind(alt.parking)),
                        );
                        paid_at_parking = Some(paid);
                        #[allow(
                            clippy::cast_possible_truncation,
                            clippy::cast_sign_loss,
                            reason = "a second"
                        )]
                        let parked = (t + paid).floor() as u32;
                        (
                            parked.saturating_add(alt.access_walk_s),
                            p.walk_node(alt.parking),
                            nodes.walk_d,
                        )
                    }
                    Shape::Back(pk) => {
                        let p = parking.expect("a back itinerary has parkings");
                        (departure.saturating_add(alt.access_walk_s), nodes.walk_o, p.walk_node(pk))
                    }
                    // A shared bike to transit (S302): from the second the engine left the
                    // traveller at the hub's dock, the bike returned (or on foot).
                    Shape::ShareTransit => {
                        let (Some(t), Some(dock)) =
                            (pend.vehicle_arrival, self.dock_walk_node(alt.share_docks.1))
                        else {
                            return truncated();
                        };
                        #[allow(
                            clippy::cast_possible_truncation,
                            clippy::cast_sign_loss,
                            reason = "a second"
                        )]
                        let returned = t.floor() as u32;
                        (returned.saturating_add(alt.access_walk_s), dock, nodes.walk_d)
                    }
                };
                let mut egress = || match alt.shape {
                    Shape::Back(pk) => parking
                        .map(|p| {
                            p.stops(pk)
                                .iter()
                                .map(|&(s, w)| (s, w.saturating_add(transfer)))
                                .collect()
                        })
                        .unwrap_or_default(),
                    _ => transit.egress(reach, nodes.walk_d, nodes.access_walk_s),
                };
                let followed = itinerary_choice::follow(data, raptor, alt, at_first, &mut egress);
                // The same, on the timetable planned on (S299).
                let plan_error = match (&followed, planned.zip(raptor_planned.as_mut())) {
                    (None, _) => None,
                    (Some(_), None) => Some(0.0),
                    (Some(f), Some((p, r))) => {
                        itinerary_choice::follow(p, r, alt, at_first, egress)
                            .map(|g| f64::from(f.end) - f64::from(g.end))
                    }
                };
                let mut walks = Vec::new();
                if let (true, Some(f)) = (bin_walks, &followed) {
                    let resolve = |e: WalkEnd| match e {
                        WalkEnd::Start => Some(start),
                        WalkEnd::End => Some(end),
                        WalkEnd::Stop(s) => transit.stop_walk_node(s),
                    };
                    for seg in &f.walks {
                        let (Some(a), Some(b)) = (resolve(seg.from), resolve(seg.to)) else {
                            continue;
                        };
                        if a.is_null() || b.is_null() {
                            continue;
                        }
                        walks.push((a.raw(), b.raw(), seg.departure));
                    }
                }
                let (outcome, mismatch) = match alt.shape {
                    Shape::Back(_) => {
                        let kind = parking_kind_of(alt.mode).expect("a back leg has a vehicle");
                        let fetch = self.mode_defaults.access_s(kind);
                        let egress = self.mode_defaults.egress_s(kind);
                        let mismatch = followed
                            .as_ref()
                            .map(|f| (f64::from(f.end) - (alt.vehicle_departure - fetch)).abs());
                        // The trip ends when the vehicle has been left at the destination.
                        #[allow(
                            clippy::cast_possible_truncation,
                            clippy::cast_sign_loss,
                            reason = "a second"
                        )]
                        let outcome = pend.vehicle_arrival.map_or(ItinOutcome::Truncated, |t| {
                            ItinOutcome::Arrived((t + egress).max(0.0).floor() as u32)
                        });
                        (outcome, mismatch)
                    }
                    _ => (
                        followed
                            .as_ref()
                            .map_or(ItinOutcome::NoPath, |f| ItinOutcome::Arrived(f.end)),
                        None,
                    ),
                };
                // The vehicle's own seconds and the fixed ones (S295), for the trip's row.
                #[allow(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "whole seconds"
                )]
                let parts = match (alt.shape, parking_kind_of(alt.mode), pend.vehicle_arrival) {
                    (Shape::Out, Some(kind), Some(t)) => {
                        let access = self.mode_defaults.access_s(kind);
                        let drive = (t - f64::from(departure) - access).max(0.0);
                        let paid = paid_at_parking.unwrap_or(0.0);
                        (drive.round() as u32, (access + paid).round() as u32)
                    }
                    (Shape::Back(_), Some(kind), Some(t)) => {
                        let drive = (t - alt.vehicle_departure).max(0.0);
                        let fixed =
                            self.mode_defaults.access_s(kind) + self.mode_defaults.egress_s(kind);
                        (drive.round() as u32, fixed.round() as u32)
                    }
                    _ => (0, 0),
                };
                (
                    Executed { outcome, followed, mismatch, plan_error, walks: Vec::new(), parts },
                    walks,
                )
            },
        );
        // The walks not searched yet in this execution, each once, in parallel.
        let mut missing: Vec<(u32, u32)> = followed
            .iter()
            .flat_map(|(_, walks)| walks.iter().map(|&(a, b, _)| (a, b)))
            .filter(|pair| !walk_paths.contains_key(pair))
            .collect();
        missing.sort_unstable();
        missing.dedup();
        let found = par_map(
            &missing,
            itinerary_choice::SCRATCH_CHUNK,
            || (Reach::new(walk, seconds), vec![false; transit.walk_node_count()]),
            |(reach, wanted), &(a, b)| {
                TransitSetup::walk_path(reach, wanted, NodeId::new(a), NodeId::new(b), walk_bound)
                    .map(Vec::into_boxed_slice)
            },
        );
        walk_paths.extend(missing.into_iter().zip(found));
        followed
            .into_iter()
            .map(|(mut executed, walks)| {
                executed.walks = walks
                    .into_iter()
                    .filter_map(|(a, b, start)| {
                        let links = walk_paths.get(&(a, b))?.as_deref()?;
                        (!links.is_empty()).then(|| (links.to_vec(), start))
                    })
                    .collect();
                executed
            })
            .collect()
    }

    fn static_outcome(
        &mut self,
        trip: TripId,
        mode: Mode,
        static_routes: &StaticRoutes,
        bike_access_s: u32,
        chosen: Option<&[LinkId]>,
    ) -> StaticOutcome {
        let (Some(layer), Some(setup)) = (static_layer_of(mode), self.layers.for_mode(mode)) else {
            return StaticOutcome::NotAvailable;
        };
        let traveller = self.trips.traveller(trip);
        let (origin, destination) = (self.trips.origin(trip), self.trips.destination(trip));
        if let Some(kind) = mode.vehicle() {
            if !self.vehicles.is_at_origin(&self.travellers, traveller, kind, origin) {
                return StaticOutcome::NoVehicle;
            }
        }
        let index = match static_routes.of(trip, layer) {
            StaticRoute::None => return StaticOutcome::NotAvailable,
            StaticRoute::Unreachable => return StaticOutcome::NoPath,
            StaticRoute::Here => {
                if let Some(kind) = mode.vehicle() {
                    self.vehicles.relocate(traveller, kind, destination);
                }
                return StaticOutcome::Here;
            }
            StaticRoute::Route(index) => index,
        };
        if let Some(kind) = mode.vehicle() {
            self.vehicles.relocate(traveller, kind, destination);
        }
        let weight = self.travellers.weight(traveller);
        let slot = usize::from(layer == openmobisim_core_graph::layers::StaticLayer::Walk);
        // A bike sets off once reached and unlocked; a walk at the departure.
        let access_s = if slot == 0 { bike_access_s } else { 0 };
        // A trip that chose a bike route among several in mode choice (S298) rides the one it
        // chose; its layer's route otherwise.
        let links = chosen.map_or_else(|| static_routes.links(layer, index), <[LinkId]>::to_vec);
        let vehicle = Vehicle::new(
            VehicleId::new(trip.raw()),
            links,
            Pcu(f64::from(weight)),
            Second(self.trips.departure(trip).get() + access_s),
        );
        let trajectory = traverse_timed(&vehicle, setup.network().network(), setup.seconds());
        StaticOutcome::Travelled { vehicle, trajectory, slot }
    }

    fn enqueue(&self, queue: &mut BinaryHeap<Reverse<EventKey>>, trip: TripId) {
        queue.push(Reverse(EventKey::new(
            self.trips.departure(trip),
            EntityKind::Trip,
            trip.raw(),
        )));
    }

    /// The trip right after `current` in `traveller`'s day, if any.
    ///
    /// [`Travellers::trips_of`] documents that a traveller's trips occupy a
    /// contiguous `TripId` range in `trip_seq` order, so "the next trip" is
    /// always `current.raw() + 1`, provided that is still inside the range.
    fn next_trip_of(&self, traveller: TravellerId, current: TripId) -> Option<TripId> {
        let last = self.travellers.trips_of(traveller).last()?;
        (current.raw() < last.raw()).then(|| TripId::new(current.raw() + 1))
    }
}

impl Run {
    /// A dock's node on the walk layer, of the run's docked service (S302).
    fn dock_walk_node(&self, dock: u32) -> Option<NodeId> {
        match self.services.as_deref()?.first()?.parts() {
            crate::services::ServiceParts::BikeDocked(docks, _) => {
                docks.walk_node.get(dock as usize).copied()
            }
            crate::services::ServiceParts::Taxi(..) => None,
        }
    }
}

impl RunResult {
    /// The modes the run's outputs list (S302): a service's mode only in a run with a service,
    /// so a run without one lists what it did before services existed.
    #[must_use]
    pub fn modes_listed(&self) -> &'static [Mode] {
        Mode::listed(self.services.is_some())
    }
}

/// A run's docked service as the choice sees it (S302), once its docks' expected costs are
/// known: `None` without one.
fn share_context<'a>(
    setup: Option<&'a ServiceSetup>,
    walk: Option<&'a crate::layers::LayerSetup>,
    index: Option<&'a DockIndex>,
    costs: Option<&'a DockCosts>,
    hubs: Option<&'a HubDocks>,
) -> Option<ShareContext<'a>> {
    Some(ShareContext { setup: setup?, walk: walk?, index: index?, costs: costs?, hubs })
}

/// The people the completed trips of a loading stand for: their travellers' weights, from its
/// events (one per trip).
fn completed_people(events: &[EventRow], trips: &Trips, travellers: &Travellers) -> f64 {
    events
        .iter()
        .filter(|e| e.event_type == EventType::TripCompleted)
        .map(|e| f64::from(travellers.weight(trips.traveller(TripId::new(e.entity_id)))))
        .sum()
}

/// Each mode's outcomes, from the events of a loading (one per trip) and the
/// trips' departures: the same travel time a trip adds to the run's
/// total, filed under its mode.
/// Each mode's outcomes, and those of the trips with no mode.
fn mode_totals(
    events: &[EventRow],
    trips: &Trips,
    travellers: &Travellers,
    taken: &[Option<Mode>],
) -> ([ModeTotals; Mode::COUNT], ModeTotals) {
    let mut out = [ModeTotals::default(); Mode::COUNT];
    let mut none = ModeTotals::default();
    for raw in 0..trips.len() {
        let trip = TripId::new(raw);
        let totals = match taken[trip.index()] {
            Some(m) => &mut out[m.index()],
            None => &mut none,
        };
        totals.completion.total_trips += 1;
        totals.weighted_trips += f64::from(travellers.weight(trips.traveller(trip)));
    }
    for e in events {
        let trip = TripId::new(e.entity_id);
        let totals = match taken[trip.index()] {
            Some(m) => &mut out[m.index()],
            None => &mut none,
        };
        let weight = f64::from(travellers.weight(trips.traveller(trip)));
        let c = &mut totals.completion;
        match e.event_type {
            EventType::TripCompleted => {
                c.completed += 1;
                totals.weighted_completed += weight;
                totals.total_travel_time += (Duration::from_clock(e.second)
                    - Duration::from_clock(trips.departure(trip)))
                    * weight;
            }
            EventType::TripTruncated => {
                c.truncated += 1;
                totals.weighted_truncated += weight;
            }
            EventType::NoVehicleAvailable => {
                c.no_vehicle_available += 1;
                totals.weighted_not_run += weight;
            }
            EventType::NoFeasiblePath => {
                c.no_feasible_path += 1;
                totals.weighted_not_run += weight;
            }
            EventType::ModeNotAvailable => {
                c.mode_not_available += 1;
                totals.weighted_not_run += weight;
            }
        }
    }
    (out, none)
}

/// The sets with an update's routes in them, and the trips' choices renumbered to match. Where
/// the update caps its sets and some would overflow, they make room first ([`make_room`]:
/// a trip on a route that made room moves to the route that replaced it); otherwise every route
/// stays. Returns the sets and how many routes went in.
/// Which routes of `sets` the itinerary trips drive as their direct car alternative: no
/// route choice names them, so `make_room` is told to keep them (a dropped one forced its
/// trip to choose again outside the strategy's reselection).
fn itinerary_routes_kept(
    sets: &RouteSets,
    car_keys: &[RouteKey],
    itin: Option<(&Itineraries, &Chosen)>,
) -> Vec<bool> {
    let mut kept = vec![false; sets.route_count()];
    let Some((itineraries, chosen)) = itin else { return kept };
    for (p, &trip) in itineraries.trips.iter().enumerate() {
        let Some(alt) = chosen.alt[p].as_ref() else { continue };
        if alt.shape != Shape::Direct || alt.mode != Mode::Car || alt.vehicle_links.is_empty() {
            continue;
        }
        let Some(k) = sets.key_index(car_keys[trip.index()]) else { continue };
        for r in sets.route_range(k) {
            let links = sets.route(r).links;
            if links.len() == alt.vehicle_links.len()
                && links.iter().zip(&alt.vehicle_links).all(|(a, b)| *a == b.raw())
            {
                kept[r] = true;
                break;
            }
        }
    }
    kept
}

/// `sets` with, for each pair whose every route crosses a link closed by a closure the travellers
/// know of (`closures`, `(link, from, to)`), the least free-flow route that crosses none (S300).
fn with_routes_round(
    network: &RoadNetwork,
    turns: &TurnTable,
    sets: &RouteSets,
    closures: &[(u32, f64, f64)],
) -> RouteSets {
    let mut cost: Vec<f64> =
        (0..network.link_count()).map(|i| network.free_flow_time(LinkId::new(i)).get()).collect();
    for &(l, _, _) in closures {
        cost[l as usize] = f64::INFINITY;
    }
    let crosses = |links: &[u32]| links.iter().any(|&l| cost[l as usize].is_infinite());
    let ctx = openmobisim_core_routes::SearchContext::with_costs(network, turns, cost.clone());
    let mut search = openmobisim_core_routes::Search::new(&ctx);
    let mut additions = Vec::new();
    for (k, key) in sets.keys().iter().enumerate() {
        if !sets.route_range(k).all(|r| crosses(sets.route(r).links)) {
            continue;
        }
        let round = search.shortest(NodeId::new(key.origin), NodeId::new(key.destination));
        if let Some(route) = round {
            additions.push((k, vec![route]));
        }
    }
    if additions.is_empty() {
        return sets.clone();
    }
    sets.extended(network, &additions, 0, sets.update()).0
}

#[allow(clippy::too_many_arguments, reason = "the sets, what grows them and what it changes")]
fn grow_sets(
    network: &RoadNetwork,
    sets: &RouteSets,
    mut additions: Vec<(usize, Vec<Route>)>,
    update: &dyn RouteUpdate,
    choices: &mut RouteChoices,
    trip_keys: &[RouteKey],
    kept: &[bool],
    stamp: u32,
    cost: &RouteCost,
) -> (RouteSets, u32) {
    let count = |a: &[(usize, Vec<Route>)]| {
        u32::try_from(a.iter().map(|(_, r)| r.len()).sum::<usize>()).expect("few routes are added")
    };
    let overflows = |max: usize| {
        additions.iter().any(|(k, routes)| sets.route_range(*k).len() + routes.len() > max)
    };
    match update.max_routes() {
        Some(max) if overflows(max) => {
            let removed = if cost.is_time_only() {
                make_room(sets, &mut additions, &choices.route, &choices.weight, kept, max)
            } else {
                // By the route cost (S299): each route's free-flow time plus its extras.
                let of = |v: usize, r: usize| {
                    let view = sets.route(r);
                    #[allow(clippy::cast_possible_truncation, reason = "few vectors")]
                    let v = v as u32;
                    f64::from(view.cost) + cost.route_extra(v, view.links)
                };
                make_room_by(
                    sets,
                    &mut additions,
                    &choices.route,
                    &choices.weight,
                    kept,
                    max,
                    cost.vector_count(),
                    &of,
                )
            };
            let added = count(&additions);
            let (grown, map) =
                sets.revised(network, &additions, &removed, stamp, &update.descriptor());
            choices.revised(&map, &grown, trip_keys);
            (grown, added)
        }
        _ => {
            let added = count(&additions);
            let (grown, shift) = sets.extended(network, &additions, stamp, &update.descriptor());
            choices.grown(sets, &shift, &grown, trip_keys);
            (grown, added)
        }
    }
}
