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
//! ([`crate::itinerary_choice`], M4) on expected costs before each loading and
//! execute it on the loading's times after it: the car of a park-and-ride trip is
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
    Demand, RouteKey, RouteSetGenerator, RouteSets, TripDemand, TripSnapper, default_generator,
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
use crate::identity::{Inputs, RunDescription, describe};
use crate::itinerary_choice::{
    self, CarRoutes, ChooseInputs, Chosen, Followed, Itineraries, ItineraryResult, NoAlternative,
    Planner, Shape, Simulated, WalkEnd, parking_kind_of,
};
use crate::layers::{
    ClassLimits, ModeDefaults, StaticLayers, StaticRoute, StaticRoutes, class_limits,
    static_layer_of,
};
use crate::link_times::{LinkTimes, relative_time_change};
use crate::link_values::LinkValues;
use crate::loading_rules::LoadingOptions;
use crate::parking::{
    self as parking_mod, ExpectedAvailability, ParkingEvent, ParkingResult, ParkingSetup,
};
use crate::prices::{PreparedPrices, Prices};
use crate::reroute::Rerouter;
use crate::route_cache::{RouteSetCache, generation_key};
use crate::route_choice::{self, NO_ROUTE, RouteChoices};
use crate::route_update::{NoRouteUpdate, RouteUpdate, UpdateContext, make_room};
use crate::timings::{Stage, Timings};
use crate::transit::{TransitResult, TransitSetup, par_map};
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
    completion: TripCompletionStats,
    events: Vec<EventRow>,
    /// Traffic by the bin it left each link in: what a flow map draws.
    link_bins: Option<LinkBins>,
    /// Link times by the bin of entry and waits at origins by the bin of departure:
    /// what costs the next iteration's routes (only recorded when there is one).
    entry_bins: Option<EntryTables>,
    /// The bike and walk layers' per-link results, if asked for.
    layer_bins: [Option<LinkBins>; 2],
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
}

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
    /// Walk legs to bin: links and the second they start.
    walks: Vec<(Vec<LinkId>, u32)>,
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
    /// A traveller is offered only the routes whose expected time is within this share of the
    /// best's; 0 offers them all. [`DEFAULT_CHOICE_DETOUR_LIMIT`] by default.
    choice_detour_limit: f64,
    /// A traveller is offered at most this many routes, the fastest at their departure.
    /// [`DEFAULT_CHOICE_MAX_ROUTES`] by default.
    choice_max_routes: u32,
    /// Generated route sets kept for the next run that asks for the same: none, by default.
    route_cache: Option<Arc<RouteSetCache>>,
    /// The bike and walk layers: none, by default.
    layers: Arc<StaticLayers>,
    /// The timetable, linked to the walk and bike layers: none, by default.
    transit: Option<Arc<TransitSetup>>,
    /// The parkings, linked to the layers and stops: none, by default.
    parking: Option<Arc<ParkingSetup>>,
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
            choice_detour_limit: DEFAULT_CHOICE_DETOUR_LIMIT,
            choice_max_routes: DEFAULT_CHOICE_MAX_ROUTES,
            route_cache: None,
            layers: Arc::new(StaticLayers::default()),
            transit: None,
            parking: None,
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
    /// loading, which records none (free-flow times stand in).
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

    /// The same run, offering each traveller only the routes of the pair's set whose expected time
    /// (at the times of the last loading; at free flow for the first choice) is within `limit`
    /// of the best route's: **a time-dependent choice set**. A route far slower than
    /// the best at the moment is no realistic alternative, and in a logit it only takes
    /// probability that belongs to routes that compete (the gap grows with the size of the set).
    /// `0.0` offers every route. The best route is always offered; a route dropped
    /// keeps its place in the sets and can return when the times change. The gap is still
    /// measured against the whole set.
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
    /// offers every such route). The gap is still measured against the whole set.
    #[must_use]
    pub fn with_choice_max_routes(mut self, n: u32) -> Self {
        self.choice_max_routes = n;
        self
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

    /// The loading's level: `None` at free flow.
    fn level(&self) -> Option<FidelityLevel> {
        match &self.flow_motor {
            FlowMotor::Ltm { level, .. } => Some(*level),
            FlowMotor::Level0 => None,
        }
    }

    /// What went into this run: its seed and its fingerprint. Take it
    /// before [`Self::execute`]; it depends only on the inputs.
    #[must_use]
    pub fn description(&self) -> RunDescription {
        let read = self.choice_model.required_attributes();
        let reads = |name: &str| read.as_ref().is_none_or(|names| names.iter().any(|n| n == name));
        describe(&Inputs {
            threads: self.threads(),
            network: &self.network,
            travellers: &self.travellers,
            trips: &self.trips,
            window: self.window,
            window_start: self.window_start,
            flow_motor: &self.flow_motor,
            link_bin_seconds: self.link_bin_seconds,
            route_method: self.route_generator.name(),
            route_descriptor: &self.route_generator.descriptor(),
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
            choice_detour_limit: self.choice_detour_limit,
            choice_max_routes: self.choice_max_routes,
            layers: &self.layers,
            transit: self.transit.as_deref(),
            parking: self.parking.as_deref(),
            mode_choice: self.mode_choice.as_deref(),
            mode_defaults: &self.mode_defaults,
            class_modes: &self.class_modes,
            class_limits: &self.class_limits,
            link_values: &self.link_values,
            disruptions: &self.disruptions,
            loading: &self.loading.resolved(self.level(), !self.disruptions.road.is_empty()),
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
        // Bike and walk trips' routes on their layers: once per run, since their
        // costs are static. A trip choosing its mode gets its route on each layer offered.
        let static_routes = StaticRoutes::build(
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
        );
        clock = timings.lap("static_routes", None, clock);
        // Shared handles, so the loading (which moves vehicles about in `self`) and the
        // chooser (which only reads the inputs) do not borrow each other.
        let (network, travellers, trips) =
            (self.network.clone(), self.travellers.clone(), self.trips.clone());
        // Itinerary trips (M4, S201): transit, park-and-ride and bike-and-ride choose an
        // itinerary on expected costs — free flow, the schedule and the parkings as the day
        // starts at first, the last loading's after; and, with mode choice (M5), every trip
        // whose mode was not stated.
        let (transit_arc, parking_arc, layers_arc) =
            (self.transit.clone(), self.parking.clone(), self.layers.clone());
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
            FlowMotor::Ltm { turns, .. } => turns.clone(),
            FlowMotor::Level0 => Arc::new(TurnTable::build(&self.network, self.network.signals())),
        };
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
        let generate = || {
            let generator: Arc<dyn RouteSetGenerator> = match &demand_rows {
                Some(rows) => {
                    let demand = Demand { network: &self.network, turns: &turns, trips: rows };
                    self.route_generator
                        .with_demand(&demand)
                        .map_or_else(|| self.route_generator.clone(), Arc::from)
                }
                None => self.route_generator.clone(),
            };
            RouteSets::generate(&self.network, &turns, &car_keys, generator.as_ref())
        };
        // Sets already generated for these inputs are handed back, if the run has a cache (S178).
        let mut route_sets = match &self.route_cache {
            Some(cache) => {
                let key = generation_key(
                    &self.network,
                    self.route_generator.as_ref(),
                    &car_keys,
                    demand_rows.as_deref(),
                );
                cache.get_or_generate(key, generate)
            }
            None => Arc::new(generate()),
        };
        clock = timings.lap("route_sets", None, clock);

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
            turns: &turns,
            detour_limit: self.choice_detour_limit,
            max_routes: self.choice_max_routes,
            link_values: &link_values,
            link_value_names: &link_value_names,
            prices: &prices,
            demand_values: &demand_values,
        };
        let mut chooser = route_choice::Chooser::new(&inputs, route_sets.clone())?;
        // What the full gap is measured in (S293): the model's utility, or time.
        let currency = Currency::new(model.as_ref(), &travellers, &chooser.wanted_names())?;
        let mut route_choices = chooser.choose_all(&choice_rng, 0)?;
        clock = timings.lap("route_choice", Some(0), clock);
        let route_update = self.route_update.clone();

        let itinerary_wanted = match &itineraries {
            Some(_) => itinerary_choice::wanted(
                model.as_ref(),
                &itinerary_choice::ATTRIBUTES,
                &link_value_names,
            )?,
            None => Vec::new(),
        };
        let car_ctx = SearchContext::new(&network, &turns);
        let bike_ctx = layers_arc.bike.as_ref().map(|b| {
            SearchContext::with_costs(b.network().network(), b.turns(), b.costs().to_vec())
        });
        let bike_planner = layers_arc.bike.as_ref().zip(bike_ctx.as_ref());
        let window_s = f64::from(self.window.get());
        let window_end = self.window.get();
        let start_s = f64::from(self.window_start.get());
        let mut availability =
            parking_arc.as_deref().map(|p| ExpectedAvailability::initial(p, start_s, window_s));
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
            FlowMotor::Level0 => 1,
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
            let free = LinkTimes::free_flow(&network);
            let attributes =
                (mode_choice.is_some() || fallback_car).then(|| route_sets.attributes(&network));
            let planner = Planner {
                transit: transit_arc.as_deref().map(|t| (t, t.scheduled())),
                parking: parking_arc.as_deref(),
                car: &car_ctx,
                bike: bike_planner,
                times: &free,
                availability: availability.as_ref(),
                detour_limit: self.choice_detour_limit,
                max_routes: self.choice_max_routes,
                car_access_s: self.mode_defaults.car_access_s,
                car_egress_s: self.mode_defaults.car_egress_s,
                car_routes: attributes
                    .as_ref()
                    .map(|attributes| CarRoutes { sets: &route_sets, attributes }),
            };
            chosen = Some(
                itin.choose(&planner, &choose_inputs, None, 0, None, first_group.as_deref())?.0,
            );
            clock = timings.lap("itinerary_choice", Some(0), clock);
        }

        let strategy = self.equilibration.clone();
        let max_iterations = strategy.max_iterations().max(1);
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
                let times = loaded.entry_bins.as_ref().map_or_else(
                    || LinkTimes::free_flow(&network),
                    |bins| LinkTimes::from_tables(&self.network, bins),
                );
                let choosing: Vec<bool> = group.iter().map(|&g| g == k).collect();
                // The group's pairs searched on these times: routes that avoid the congestion of
                // the groups before it, for it to choose among.
                if route_update.is_active() {
                    let trip_choosing: Vec<bool> = (0..total_trips)
                        .map(|i| choosing[trips.traveller(TripId::new(i)).index()])
                        .collect();
                    let found = route_update.update(&UpdateContext {
                        network: &network,
                        turns: &turns,
                        trips: &trips,
                        trip_keys: &car_keys,
                        route_sets: &route_sets,
                        times: &times,
                        iteration: 0,
                        active: Some(&trip_choosing),
                    });
                    increment_searches += found.searches;
                    if !found.routes.is_empty() {
                        let kept_routes = itinerary_routes_kept(
                            &route_sets,
                            &car_keys,
                            itineraries.as_ref().zip(chosen.as_ref()),
                        );
                        let (grown, added) = grow_sets(
                            &route_sets,
                            found.routes,
                            route_update.as_ref(),
                            &mut route_choices,
                            &trip_keys,
                            &kept_routes,
                            1,
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
                    let transit = transit_arc.as_deref();
                    let realised = transit.zip(loaded.transit.as_ref()).map(|(transit, t)| {
                        transit.on_times(&transit.expected(&t.times, window_end))
                    });
                    let attributes = (mode_choice.is_some() || fallback_car)
                        .then(|| route_sets.attributes(&network));
                    let planner = Planner {
                        transit: transit
                            .map(|t| (t, realised.as_ref().unwrap_or_else(|| t.scheduled()))),
                        parking: parking_arc.as_deref(),
                        car: &car_ctx,
                        bike: bike_planner,
                        times: &times,
                        availability: availability.as_ref(),
                        detour_limit: self.choice_detour_limit,
                        max_routes: self.choice_max_routes,
                        car_access_s: self.mode_defaults.car_access_s,
                        car_egress_s: self.mode_defaults.car_egress_s,
                        car_routes: attributes
                            .as_ref()
                            .map(|attributes| CarRoutes { sets: &route_sets, attributes }),
                    };
                    let (next_chosen, _) = itin.choose(
                        &planner,
                        &choose_inputs,
                        Some(current),
                        0,
                        None,
                        Some(&choosing),
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
            let mut report = IterationReport::unmeasured(iteration);
            report.gap_currency = currency.label();
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
            let mut pending_chosen: Option<Chosen> = None;
            if let (Some(before), Some(now)) = (&previous_bins, &loaded.entry_bins) {
                report.time_change = relative_time_change(&self.network, before, now);
            }

            let mut changes = Vec::new();
            let mut times_now: Option<LinkTimes> = None;
            // What the assessment left for the whole-network gap: who was left out, and what the
            // model expected of each trip.
            let mut assessed: Option<(Vec<bool>, Vec<f64>)> = None;
            if max_iterations > 1 {
                if let Some(bins) = &loaded.entry_bins {
                    let times = LinkTimes::from_tables(&self.network, bins);
                    let next = iteration + 1;
                    // The sets grow between loadings (S176): routes the congested times of
                    // this one show to be worth having, for the next to choose among. Not
                    // after the last loading, which no choice follows.
                    if next < max_iterations && route_update.is_active() {
                        let found = route_update.update(&UpdateContext {
                            network: &network,
                            turns: &turns,
                            trips: &trips,
                            trip_keys: &car_keys,
                            route_sets: &route_sets,
                            times: &times,
                            iteration: next,
                            active: None,
                        });
                        report.route_searches = found.searches;
                        if !found.routes.is_empty() {
                            let kept_routes = itinerary_routes_kept(
                                &route_sets,
                                &car_keys,
                                itineraries.as_ref().zip(chosen.as_ref()),
                            );
                            let (grown, added) = grow_sets(
                                &route_sets,
                                found.routes,
                                route_update.as_ref(),
                                &mut route_choices,
                                &trip_keys,
                                &kept_routes,
                                next,
                            );
                            report.routes_added = added;
                            route_sets = Arc::new(grown);
                            chooser = route_choice::Chooser::new(&inputs, route_sets.clone())?;
                        }
                        clock = timings.lap("route_update", Some(iteration), clock);
                    }
                    let strategy_for_next =
                        (next < max_iterations).then_some((strategy.as_ref(), &reselect_rng));
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
                    let update = chooser.update(
                        &route_choices,
                        &times,
                        next,
                        strategy_for_next,
                        &choice_rng,
                        &unfinished,
                    )?;
                    clock = timings.lap("route_choice", Some(iteration), clock);
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
                        if let (Some(av), Some(real)) =
                            (availability.as_mut(), &loaded.availability)
                        {
                            av.absorb(real);
                        }
                        let transit = transit_arc.as_deref();
                        let realised = transit.zip(loaded.transit.as_ref()).map(|(transit, t)| {
                            transit.on_times(&transit.expected(&t.times, window_end))
                        });
                        let attributes = (mode_choice.is_some() || fallback_car)
                            .then(|| route_sets.attributes(&network));
                        let planner = Planner {
                            transit: transit
                                .map(|t| (t, realised.as_ref().unwrap_or_else(|| t.scheduled()))),
                            parking: parking_arc.as_deref(),
                            car: &car_ctx,
                            bike: bike_planner,
                            times: &times,
                            availability: availability.as_ref(),
                            detour_limit: self.choice_detour_limit,
                            max_routes: self.choice_max_routes,
                            car_access_s: self.mode_defaults.car_access_s,
                            car_egress_s: self.mode_defaults.car_egress_s,
                            car_routes: attributes
                                .as_ref()
                                .map(|attributes| CarRoutes { sets: &route_sets, attributes }),
                        };
                        let (next_chosen, found) = itin.choose(
                            &planner,
                            &choose_inputs,
                            Some(current),
                            next,
                            strategy_for_next,
                            None,
                        )?;
                        clock = timings.lap("itinerary_choice", Some(iteration), clock);
                        // The full gap (S293): every trip choosing its mode and every car
                        // itinerary, on the same costs.
                        itin.gap(
                            &planner,
                            &choose_inputs,
                            current,
                            next,
                            &ended,
                            &currency,
                            &mut gap_trips,
                        )?;
                        clock = timings.lap("gap", Some(iteration), clock);
                        // Every mode's: a trip choosing its mode is measured against the best
                        // of the mode it took (M5).
                        for mode in Mode::ALL {
                            report.itinerary_gap[mode.index()] = found.gap(mode);
                        }
                        report.itinerary_recosted = found.recosted;
                        report.itinerary_gap_excess = found.gap_excess_pooled();
                        mode_changed_by = found.mode_changed_share();
                        mode_floor_by = found.mode_changed_floor();
                        pending_chosen = Some(next_chosen);
                    }
                    // The full gap (S293): every car trip of the route choice, at every loading.
                    chooser.gap(&route_choices, &times, next, &ended, &currency, &mut gap_trips)?;
                    gap_trips.report_into(&mut report);
                    clock = timings.lap("gap", Some(iteration), clock);
                    last_gap = Some(gap_trips);
                    times_now = Some(times);
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
                if let (Some(times), Some((unfinished, expected)), true) =
                    (&times_now, &assessed, strategy.network_gap_sample() > 0)
                {
                    let (gap, excess) = chooser.network_gap(
                        &route_choices,
                        times,
                        strategy.network_gap_sample(),
                        &reselect_rng,
                        unfinished,
                        expected,
                    );
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
        let trip_outcomes =
            TripOutcomes::of(&loaded.events, &taken, &no_alternative, &loaded.at_end);
        let [bike_link_bins, walk_link_bins] = loaded.layer_bins;
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
        timings.lap("results_assembled", None, clock);
        self.timings = timings;
        Ok(RunResult {
            total_travel_time: loaded.total_travel_time,
            completion: loaded.completion,
            events: loaded.events,
            link_bins,
            route_sets: Some(route_sets),
            route_choices: Some(route_choices),
            iterations: reports,
            converged,
            by_mode,
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
            // A trip that chose a direct leg (M5) runs as a trip of that mode, on the car
            // route it chose (a bike or walk route is its layer's one).
            let mut chosen_route: Option<Vec<LinkId>> = None;
            // A car trip for want of any alternative (S271, W-1): no car of the traveller's.
            let mut fallback = false;
            if let Some((itineraries, chosen)) = itin {
                if let Some(position) = itineraries.position(trip) {
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
                    if let Some(next) = self.next_trip_of(traveller, trip) {
                        self.enqueue(&mut queue, next);
                    }
                    continue;
                }
            }
            if mode != Mode::Car {
                match self.static_outcome(trip, mode, static_routes) {
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
                        if trajectory.arrival() <= self.window {
                            completion.completed += 1;
                            total_travel_time += trajectory.total_travel_time() * f64::from(weight);
                            events.push(EventRow::trip(
                                trajectory.arrival(),
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
                                    departure,
                                );
                                let trajectory = traverse_free_flow(&vehicle, &self.network);
                                if record_bins.is_some() {
                                    level0_vehicles.push(vehicle);
                                }
                                if trajectory.arrival() <= self.window {
                                    completion.completed += 1;
                                    total_travel_time +=
                                        trajectory.total_travel_time() * f64::from(weight);
                                    events.push(EventRow::trip(
                                        trajectory.arrival(),
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
                                    departure,
                                );
                                pending_ltm.push((trip, vehicle, weight));
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
            let mut vehicles: Vec<Vehicle> =
                pending_ltm.iter().map(|(_, vehicle, _)| vehicle.clone()).collect();
            vehicles.extend(itin_cars.iter().map(|(_, v)| v.clone()));
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
            // point queue it is on only in a loading with road disruptions (S289).
            let road_disrupted = disrupted && !self.disruptions.road.is_empty();
            let o = self.loading.resolved(self.level(), road_disrupted);
            let rerouting = o.reroutes(self.level(), road_disrupted);
            let rules = Rules {
                priority: o.priority,
                reroute: rerouting.then(|| RerouteRule {
                    after_s: o.reroute_after_s,
                    max: u8::try_from(o.reroute_max).unwrap_or(u8::MAX),
                }),
                pocket_length_m: o.pocket_length_m,
                start_s: f64::from(self.window_start.get()),
                capacity_changes: if disrupted {
                    self.disruptions.capacity_changes()
                } else {
                    Vec::new()
                },
            };
            let mut rerouter = rerouting.then(|| Rerouter::new(car_ctx, expected, &o, total_trips));
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
            reroute_searches = rerouter.as_ref().map_or(0, |r| r.offers);
            peak_pcu = Some(out.peak_occupancy);
            let (trajectories, bins, entry) = (out.trajectories, out.link_bins, out.entry);
            // Where each vehicle still travelling at the end was (a car's vehicle id is its
            // trip's index, an itinerary car's likewise; buses are not trips).
            let unfinished: HashMap<VehicleId, LinkId> = out.unfinished.into_iter().collect();
            for (i, vehicle) in &itin_cars {
                if let Some(&link) = unfinished.get(&vehicle.id) {
                    at_end.push((pending_itin[*i].trip, link));
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
            for (i, vehicle) in &itin_cars {
                pending_itin[*i].vehicle_arrival =
                    by_vehicle.get(&vehicle.id).map(|t| f64::from(t.arrival().get()));
            }
            if let (Some(load), Some(transit)) = (&bus_load, &transit) {
                let id = |i: usize| {
                    VehicleId::new(total_trips + u32::try_from(i).expect("vehicles fit u32"))
                };
                bus_times = Some(transit.realised(load, |i| by_vehicle.get(&id(i))));
            }

            for (trip, vehicle, weight) in pending_ltm {
                match by_vehicle.get(&vehicle.id) {
                    Some(trajectory) => {
                        completion.completed += 1;
                        total_travel_time += trajectory.total_travel_time() * f64::from(weight);
                        events.push(EventRow::trip(
                            trajectory.arrival(),
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
                        if let Some(&link) = unfinished.get(&vehicle.id) {
                            at_end.push((trip, link));
                        }
                    }
                }
            }
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
                    data,
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

        // The bike and walk layers' per-link results, from their own vehicles.
        let mut layer_bins: [Option<LinkBins>; 2] = [None, None];
        if let Some(bin_seconds) = record_bins {
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

        Loaded {
            total_travel_time,
            completion,
            events,
            link_bins,
            entry_bins,
            layer_bins,
            transit: transit_result,
            parking: parking_result,
            availability: availability_real,
            itinerary,
            gridlock,
            reroutes,
            routes_realised,
            reroute_searches,
            peak_pcu,
            at_end,
        }
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
            (Shape::Transit | Shape::Direct, _) | (_, None) => None,
            (Shape::Out, Some(k)) => {
                let vk = vehicle_kind(k);
                let parking = self.parking.clone().expect("an out itinerary has parkings");
                if !self.vehicles.is_at_origin(&self.travellers, traveller, vk, origin) {
                    no_vehicle(out, diagnostics);
                    return;
                }
                self.vehicles.park(traveller, vk, alt.parking, parking.position(alt.parking));
                self.drive(trip, alt, k, f64::from(departure.get()), weight, index, want_bins, out)
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
                FlowMotor::Level0 => {
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
    /// leg's links, from `walk_paths` or searched once and kept there.
    #[allow(clippy::too_many_arguments, reason = "the loading's pieces")]
    #[allow(clippy::too_many_lines, reason = "one pass: follow, then the walks")]
    fn execute_itineraries(
        &self,
        transit: &TransitSetup,
        data: &openmobisim_core_transit::RaptorData,
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
            || (Reach::new(walk, seconds), Raptor::new(data, d.max_rides as usize)),
            |(reach, raptor), pend| {
                let alt =
                    chosen.alt[pend.position].as_ref().expect("a pending trip has its choice");
                let nodes = itineraries.nodes(pend.position);
                let departure = trips.departure(pend.trip).get();
                let truncated = || {
                    let e = Executed {
                        outcome: ItinOutcome::Truncated,
                        followed: None,
                        mismatch: None,
                        walks: Vec::new(),
                    };
                    (e, Vec::new())
                };
                let (at_first, start, end) = match alt.shape {
                    // (A direct leg is never pending: the loop runs it as a trip of its mode.)
                    Shape::Transit | Shape::Direct => {
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
                        let paid = p.defaults().parking_seconds(p.kind(alt.parking), a);
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
                };
                let followed = {
                    let reach = &mut *reach;
                    itinerary_choice::follow(data, raptor, alt, at_first, || match alt.shape {
                        Shape::Back(pk) => parking
                            .map(|p| {
                                p.stops(pk)
                                    .iter()
                                    .map(|&(s, w)| (s, w.saturating_add(transfer)))
                                    .collect()
                            })
                            .unwrap_or_default(),
                        _ => transit.egress(reach, nodes.walk_d, nodes.access_walk_s),
                    })
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
                        let fetch = alt.parking_s;
                        let mismatch = followed
                            .as_ref()
                            .map(|f| (f64::from(f.end) - (alt.vehicle_departure - fetch)).abs());
                        #[allow(
                            clippy::cast_possible_truncation,
                            clippy::cast_sign_loss,
                            reason = "a second"
                        )]
                        let outcome = pend.vehicle_arrival.map_or(ItinOutcome::Truncated, |t| {
                            ItinOutcome::Arrived(t.max(0.0).floor() as u32)
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
                (Executed { outcome, followed, mismatch, walks: Vec::new() }, walks)
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
        let vehicle = Vehicle::new(
            VehicleId::new(trip.raw()),
            static_routes.links(layer, index),
            Pcu(f64::from(weight)),
            self.trips.departure(trip),
        );
        let trajectory = traverse_timed(&vehicle, setup.network().network(), setup.seconds());
        let slot = usize::from(layer == openmobisim_core_graph::layers::StaticLayer::Walk);
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

fn grow_sets(
    sets: &RouteSets,
    mut additions: Vec<(usize, Vec<Route>)>,
    update: &dyn RouteUpdate,
    choices: &mut RouteChoices,
    trip_keys: &[RouteKey],
    kept: &[bool],
    stamp: u32,
) -> (RouteSets, u32) {
    let count = |a: &[(usize, Vec<Route>)]| {
        u32::try_from(a.iter().map(|(_, r)| r.len()).sum::<usize>()).expect("few routes are added")
    };
    let overflows = |max: usize| {
        additions.iter().any(|(k, routes)| sets.route_range(*k).len() + routes.len() > max)
    };
    match update.max_routes() {
        Some(max) if overflows(max) => {
            let removed =
                make_room(sets, &mut additions, &choices.route, &choices.weight, kept, max);
            let added = count(&additions);
            let (grown, map) = sets.revised(&additions, &removed, stamp, &update.descriptor());
            choices.revised(&map, &grown, trip_keys);
            (grown, added)
        }
        _ => {
            let added = count(&additions);
            let (grown, shift) = sets.extended(&additions, stamp, &update.descriptor());
            choices.grown(sets, &shift, &grown, trip_keys);
            (grown, added)
        }
    }
}
