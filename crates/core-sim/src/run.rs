//! The `Run` (S62): orchestration, trip completion statistics (S57), driven
//! by an event queue in seconds (S88).
//!
//! One run: `core-demand`'s travellers and trips, a route set
//! for every origin-destination pair (`core-routes`, S166), a route per trip
//! from a choice model (`core-choice`, S169), a loading by the chosen
//! [`FlowMotor`], and — under an [`Equilibration`] — choice and loading
//! repeated, with the route sets grown between iterations by a
//! [`RouteUpdate`] and a convergence report per iteration (S170, S176). That is
//! for car trips; **bike and walk trips** travel on their own static layer
//! ([`crate::layers`], S195), routed once and never part of the car's choice or
//! gap; **transit, park-and-ride and bike-and-ride trips** choose an itinerary
//! ([`crate::itinerary_choice`], M4) on expected costs before each loading and
//! execute it on the loading's times after it: the car of a park-and-ride trip is
//! in the loading with every other car, and parks at the hub it chose
//! ([`crate::parking`]); none of them is part of the car routes' choice or gap.
//!
//! The defaults here are the core's own and unconditional (`Level0`,
//! `deterministic`, `none`); the Python `Scenario` layers its own on top
//! (S179, S187, S191).

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
    Demand, NodeSnapper, RouteKey, RouteSetGenerator, RouteSets, TripDemand, default_generator,
};
use openmobisim_core_types::diagnostics::{
    Category, DiagKey, Diagnostics, ElementRef, Severity, codes,
};
use openmobisim_core_types::ids::{EntityId, EntityKind, LinkId, TravellerId, TripId, VehicleId};
use openmobisim_core_types::rng::{DrawAddress, RngKey, Stream, StreamRng};
use openmobisim_core_types::time::{EventKey, Second};
use openmobisim_core_types::units::{Duration, Pcu};

use crate::equilibration::{Equilibration, FreeFlow, IterationReport};
use crate::events::{EventRow, EventType};
use crate::identity::{Inputs, RunDescription, describe};
use crate::itinerary_choice::{
    self, CarRoutes, ChooseInputs, Chosen, Followed, Itineraries, ItineraryResult, Planner, Shape,
    Simulated, WalkEnd, parking_kind_of,
};
use crate::layers::{ModeDefaults, StaticLayers, StaticRoute, StaticRoutes, static_layer_of};
use crate::link_times::{LinkTimes, relative_time_change};
use crate::loading_rules::LoadingOptions;
use crate::parking::{
    self as parking_mod, ExpectedAvailability, ParkingEvent, ParkingResult, ParkingSetup,
};
use crate::reroute::Rerouter;
use crate::route_cache::{RouteSetCache, generation_key};
use crate::route_choice::{self, NO_ROUTE, RouteChoices};
use crate::route_update::{NoRouteUpdate, RouteUpdate, UpdateContext};
use crate::timings::{Stage, Timings};
use crate::transit::{TransitResult, TransitSetup, par_map};
use openmobisim_core_graph::hubs::ParkingKind;
use openmobisim_core_graph::layers::StaticLayer;
use openmobisim_core_routes::{Reach, SearchContext};
use openmobisim_core_transit::{JourneyLeg, Raptor};

/// Which loading engine a [`Run`] uses.
///
/// **Opt-in (G4): [`Run::new`]'s behaviour and Phase 1's acceptance number
/// are unchanged unless a caller explicitly asks for the LTM** via
/// [`Run::with_flow_motor`]. Nothing about `Level0` — S133/S134's original
/// placeholder — has changed; it stays available on purpose, as the cheap,
/// interaction-free debugging rung design §10.1's fidelity ladder names it.
#[derive(Clone, Debug, Default)]
pub enum FlowMotor {
    /// S133/S134's placeholder: each vehicle traverses independently at
    /// free-flow time. Phase 1's default, and still is.
    #[default]
    Level0,
    /// S84/S85's iterative LTM (`core_loading::ltm`) — vehicles genuinely
    /// interact through shared link curves and the node model. What both
    /// shipped presets eventually run (design §10.1, `level` = `Full`).
    Ltm {
        /// The network's turn table (built once, outside `Run`, since it
        /// needs [`openmobisim_core_graph::defaults::SignalDefaults`], which
        /// `Run` itself has no other reason to carry).
        turns: Arc<TurnTable>,
        /// The loading step (S84/S89 default: 300 s).
        step: Duration,
        /// Which term of the triangular diagram is in force (S76).
        level: FidelityLevel,
    },
}

/// Diagnostic codes this module records, beyond the core codes it reuses
/// (`codes::NO_FEASIBLE_PATH`, `codes::TRIP_TRUNCATED`).
pub mod local_codes {
    use openmobisim_core_types::diagnostics::DiagCode;

    /// A trip's traveller owns no car reachable from its origin, and Phase 1
    /// has no other mode to offer it (car-only, S133/S134's scope). Once a
    /// mode-choice layer exists (Phase 2), unavailability becomes a choice
    /// among alternatives, not the absence of a trip.
    pub const NO_VEHICLE_AVAILABLE: DiagCode = DiagCode("no_vehicle_available");
    /// The trip's mode cannot be simulated by this run: its layer is not
    /// there, or the mode is not built yet (transit and the combinations with
    /// it, S195).
    pub const MODE_NOT_AVAILABLE: DiagCode = DiagCode("mode_not_available");
}

/// Trip-level outcomes across a run (S57).
///
/// `total_trips` is every trip in the demand; `no_vehicle_available` and
/// `no_feasible_path` are trips this Phase 1 cut never simulates at all
/// (car-only, no route-set fallback); `completed` and `truncated` are S57's
/// statistics proper, over trips that *did* enter the network.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct TripCompletionStats {
    /// Every trip in the demand.
    pub total_trips: u32,
    /// No owned car was at the trip's origin (Phase 1 has no other mode).
    pub no_vehicle_available: u32,
    /// A car was available, but no path existed to the destination.
    pub no_feasible_path: u32,
    /// Arrived within the simulation window.
    pub completed: u32,
    /// Still in progress when the window ended (S57).
    pub truncated: u32,
    /// The trip's mode cannot be simulated by this run (S195): its layer is
    /// missing, or the mode is not built yet.
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

/// One mode's outcomes in a run (S195): its trips' completion counts and
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
}

/// Why a run could not finish.
#[derive(Clone, PartialEq, Debug)]
pub enum RunError {
    /// The choice model could not choose (S169): it needs an attribute routes do
    /// not carry, it failed, or its answer does not fit.
    Choice(ChoiceError),
}

impl core::fmt::Display for RunError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Choice(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for RunError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Choice(e) => Some(e),
        }
    }
}

impl From<ChoiceError> for RunError {
    fn from(e: ChoiceError) -> Self {
        Self::Choice(e)
    }
}

/// What a run produced: Phase 1's one KPI, the statistics that make it
/// honest about what it did and did not simulate, and the per-trip events
/// `io-parquet`'s `events.parquet` writer reads from.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct RunResult {
    /// Total travel time, summed over completed trips, weighted by
    /// traveller weight (S86: a simulated traveller represents *w* people).
    /// **An assumption, not a settled decision** — see the crate docs.
    pub total_travel_time: Duration,
    /// S57's completion statistics.
    pub completion: TripCompletionStats,
    /// One row per trip outcome, unsampled — `io-parquet`'s writer applies
    /// Foundations §6's "sampled at 1% by default" at write time, since
    /// sampling is a property of the output format, not of what a run
    /// tracks internally.
    pub events: Vec<EventRow>,
    /// Per-link, per-time-bin results, when asked for with
    /// [`Run::with_link_bins`] (S163): every link traversal that finished
    /// inside the window, including those of trips still under way when it
    /// ended.
    pub link_bins: Option<LinkBins>,
    /// The route sets the trips were routed from (S165): every alternative the
    /// method found for every origin-destination pair the demand asked for.
    /// Each trip takes the route the choice model picks from its pair's set.
    pub route_sets: Option<RouteSets>,
    /// Which route each trip took, out of how many, and how likely the model
    /// found it (S169).
    pub route_choices: Option<RouteChoices>,
    /// What each iteration showed (S170): one report for a run without
    /// equilibration, one per loading under `msa`. `route_choices`, `events`,
    /// `link_bins`, the completion counts and the total travel time are those of
    /// the last.
    pub iterations: Vec<IterationReport>,
    /// Whether the strategy stopped before its most iterations because it had
    /// converged.
    pub converged: bool,
    /// The outcomes by mode, in [`Mode::ALL`] order (S195); they add up to
    /// [`Self::completion`] and [`Self::total_travel_time`].
    pub by_mode: [ModeTotals; Mode::COUNT],
    /// The bike layer's per-link, per-time-bin results, when per-link results
    /// were asked for and some bike trip ran (S195); `pcu` there counts
    /// travellers.
    pub bike_link_bins: Option<LinkBins>,
    /// The walk layer's, likewise.
    pub walk_link_bins: Option<LinkBins>,
    /// What transit did (S199), if the run has a timetable: the times the runs
    /// kept and who boarded and alighted where.
    pub transit: Option<TransitResult>,
    /// What the parkings did (M4), if the run has parkings.
    pub parking: Option<ParkingResult>,
    /// Each itinerary trip's choice and how it went (M4), if the run has any.
    pub itineraries: Option<ItineraryResult>,
    /// What stood still when the last loading's window ended (S213): the vehicles that did not
    /// finish, where they are, and the closed loops of links waiting on one another, with the
    /// check on the loading's guarantee ([`LockReport::room_waits_with_room`], always 0).
    /// `None` at level 0, where nothing waits.
    pub gridlock: Option<LockReport>,
    /// Every reroute of the last loading (S213), in the order they happened; a car's vehicle id
    /// is its trip's index. Empty without rerouting.
    pub reroutes: Vec<RerouteRecord>,
    /// The **realised route** of each trip that re-routed in the last loading and arrived, by
    /// trip: the links it took. Every other trip followed its planned route (its route choice);
    /// nothing is kept for those.
    pub routes_realised: Vec<(TripId, Vec<LinkId>)>,
    /// Per link, the most PCU it held at once in the last loading (S229); `None` at level 0.
    /// Above the link's storage under the point queue: where the queue would have spilled back.
    pub link_peak_pcu: Option<Vec<f64>>,
}

/// How one loading is made besides the routes it follows.
#[derive(Clone, Copy)]
struct LoadPlan<'a> {
    /// Record per-link results in bins of this many seconds, if asked.
    record_bins: Option<u32>,
    /// Also record the entry-time tables the next iteration's costs are read from.
    want_entry: bool,
    /// Load with this level of the link transmission model instead of the run's (S178: the
    /// warm-up's point-queue model).
    level_override: Option<FidelityLevel>,
    /// Load only the travellers marked here, by traveller (S223: the free-flow loading's
    /// increments); everyone if `None`.
    active: Option<&'a [bool]>,
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
    /// Every reroute (S213), and the realised routes of the trips that re-routed and arrived.
    reroutes: Vec<RerouteRecord>,
    routes_realised: Vec<(TripId, Vec<LinkId>)>,
    /// How many reroute offers (searches) the loading made (S223).
    reroute_searches: u32,
    /// Per link, the most PCU it held at once (S229); `None` at level 0.
    peak_pcu: Option<Vec<f64>>,
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
/// Foundations §5 says a `Run` owns (S62) — here, just vehicle locations,
/// since there are no curves, hubs or choice state yet.
pub struct Run {
    network: Arc<RoadNetwork>,
    travellers: Arc<Travellers>,
    trips: Arc<Trips>,
    vehicles: VehicleLocations,
    /// Trips still in progress after this second are truncated (S57).
    window: Second,
    flow_motor: FlowMotor,
    /// Length of the time bins of the per-link results, if asked for.
    link_bin_seconds: Option<u32>,
    /// How route sets are generated (S165): the penalty method by default.
    route_generator: Arc<dyn RouteSetGenerator>,
    /// The scenario's master seed (S168): the one number that starts every
    /// random stream. Nothing draws from it yet; see [`RunDescription`].
    master_seed: u64,
    /// How each trip picks a route from its pair's set (S169): all-or-nothing on
    /// the best route by default, so a run is what it was before choice existed.
    choice_model: Arc<dyn ChoiceModel>,
    /// How choice and loading are repeated (S170): once, by default.
    equilibration: Arc<dyn Equilibration>,
    /// How the route sets grow between iterations (S176): not at all, by default.
    route_update: Arc<dyn RouteUpdate>,
    /// A traveller is offered only the routes whose expected time is within this share of the
    /// best's (S178); 0 offers them all. [`DEFAULT_CHOICE_DETOUR_LIMIT`] by default.
    choice_detour_limit: f64,
    /// Generated route sets kept for the next run that asks for the same (S178): none, by default.
    route_cache: Option<Arc<RouteSetCache>>,
    /// The bike and walk layers (S195): none, by default.
    layers: Arc<StaticLayers>,
    /// The timetable, linked to the walk and bike layers (S199): none, by default.
    transit: Option<Arc<TransitSetup>>,
    /// The parkings, linked to the layers and stops (M4): none, by default.
    parking: Option<Arc<ParkingSetup>>,
    /// The modes a trip without a stated mode chooses among (M5): no choice, by default.
    mode_choice: Option<Vec<Mode>>,
    /// How long a walk or ride mode choice offers (S209): the shipped values, by default.
    mode_defaults: ModeDefaults,
    loading: LoadingOptions,
    /// Wall-clock time by stage of the last execution (S223).
    timings: Timings,
    /// The modes each traveller class may use, by class index (S231); empty: every class all.
    class_modes: Vec<[bool; Mode::COUNT]>,
}

/// The key of the draw that puts a traveller in a group of the free-flow loading's increments
/// (S223), apart from every other draw on the re-selection stream: who chooses again at
/// iteration `i` (keys 1 to `MAX_ITERATIONS`), the itinerary gap's sample (from `1 << 31`) and
/// the network gap's (`u32::MAX`, keyed on trips).
const INCREMENT_KEY: u32 = u32::MAX - 1;

/// The group (0 to `increments − 1`) of the free-flow loading `traveller` is in (S223): a draw
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
/// traveller, unless the run says otherwise (S178; the user chose 0.5, S179). 0 offers every route
/// of the pair's set. At 0.5 a route is offered only if it is at most 50% slower than the best;
/// every built-in method's routes are within 30% (`penalty`) or 100% (`montecarlo`) of the best
/// at free flow, so a run that does not iterate changes only for `montecarlo`, and an iterating
/// one drops the routes congestion has made much slower than the best.
pub const DEFAULT_CHOICE_DETOUR_LIMIT: f64 = 0.5;

impl Run {
    /// Build a run. Vehicle locations are seeded per S129: every owned car
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
            flow_motor: FlowMotor::default(),
            link_bin_seconds: None,
            route_generator: Arc::from(default_generator()),
            master_seed: 0,
            choice_model: Arc::new(Deterministic),
            equilibration: Arc::new(FreeFlow::default()),
            route_update: Arc::new(NoRouteUpdate),
            choice_detour_limit: DEFAULT_CHOICE_DETOUR_LIMIT,
            route_cache: None,
            layers: Arc::new(StaticLayers::default()),
            transit: None,
            parking: None,
            mode_choice: None,
            mode_defaults: ModeDefaults::SHIPPED,
            loading: LoadingOptions::SHIPPED,
            timings: Timings::new(),
            class_modes: Vec::new(),
        }
    }

    /// The same run with the modes each traveller class may use (S231), by class index (the
    /// travellers' [`openmobisim_core_demand::Travellers::user_class`]): a trip choosing its
    /// mode is offered only those its class may use. A class past the end may use all.
    #[must_use]
    pub fn with_class_modes(mut self, class_modes: Vec<[bool; Mode::COUNT]>) -> Self {
        self.class_modes = class_modes;
        self
    }

    /// Wall-clock time by stage of the last [`Self::execute`] or [`Self::try_execute`] (S223),
    /// in the order the stages ended; empty before. Kept apart from [`RunResult`], since it
    /// differs from run to run. The names: [`Timings::lap`].
    #[must_use]
    pub fn timings(&self) -> &[Stage] {
        self.timings.stages()
    }

    /// The same run, with a timetable for the trips whose mode is
    /// [`Mode::Transit`] (S199): they walk to a stop, ride and walk on, by the
    /// journey the choice model picks among the competitive ones (M4). Without it such a trip is counted as
    /// `mode_not_available`. Its walks are counted on the walk layer's per-link
    /// results when that layer is the run's own ([`Self::with_layers`]).
    #[must_use]
    pub fn with_transit(mut self, transit: Arc<TransitSetup>) -> Self {
        self.transit = Some(transit);
        self
    }

    /// Parkings (M4), for trips whose mode is [`Mode::CarTransit`] or
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
    /// `modes` (M5), with its route, by the run's choice model: the walk, the bike route,
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

    /// The same run, its loading applying `options` (S213): priority at merges, en-route
    /// rerouting. Only the link transmission model has junctions and queues; level 0 ignores
    /// them.
    #[must_use]
    pub fn with_loading_options(mut self, options: LoadingOptions) -> Self {
        self.loading = options;
        self
    }

    /// The same run, offering mode choice's walk and bike alternatives only up to the
    /// times of `defaults` (S209); a trip given the mode takes it at any length.
    #[must_use]
    pub fn with_mode_defaults(mut self, defaults: ModeDefaults) -> Self {
        self.mode_defaults = defaults;
        self
    }

    /// The same run, with bike and walk layers for the trips whose mode is
    /// [`Mode::Bike`] or [`Mode::Walk`] (S195). Without them such a trip is
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
    /// instead of the default penalty method (S165). Any
    /// [`RouteSetGenerator`] works: select a built-in one by name with
    /// `openmobisim_core_routes::generator`, or pass your own.
    #[must_use]
    pub fn with_route_generator(mut self, generator: Arc<dyn RouteSetGenerator>) -> Self {
        self.route_generator = generator;
        self
    }

    /// The same run, with `master_seed` as its master seed (S168; default 0).
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
    /// default all-or-nothing [`Deterministic`] (S169). Select a built-in model
    /// by name with `openmobisim_core_choice::model`, or pass your own
    /// [`ChoiceModel`].
    #[must_use]
    pub fn with_choice_model(mut self, model: Arc<dyn ChoiceModel>) -> Self {
        self.choice_model = model;
        self
    }

    /// The same run, repeating choice and loading under `strategy` instead of
    /// once (S170). Select a built-in one by name with
    /// [`crate::equilibration::strategy`], or pass your own [`Equilibration`].
    #[must_use]
    pub fn with_equilibration(mut self, strategy: Arc<dyn Equilibration>) -> Self {
        self.equilibration = strategy;
        self
    }

    /// The same run, growing its route sets between iterations with `update` instead of
    /// leaving them as the generator made them (S176). Select a built-in one by name with
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
    /// of the best route's: **a time-dependent choice set** (S178). A route far slower than
    /// the best at the moment is no realistic alternative, and in a logit it only takes
    /// probability that belongs to routes that compete (the gap grows with the size of the set, S177).
    /// `0.0` offers every route, as before. The best route is always offered; a route dropped
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

    /// The same run, taking its route sets from `cache` if it holds the sets this run would
    /// generate, and leaving them there if not (S178): see [`RouteSetCache`]. Results do not depend
    /// on whether the cache was used.
    #[must_use]
    pub fn with_route_cache(mut self, cache: Arc<RouteSetCache>) -> Self {
        self.route_cache = Some(cache);
        self
    }

    /// What went into this run: its seed and its fingerprint (S168). Take it
    /// before [`Self::execute`]; it depends only on the inputs.
    #[must_use]
    pub fn description(&self) -> RunDescription {
        describe(&Inputs {
            network: &self.network,
            travellers: &self.travellers,
            trips: &self.trips,
            window: self.window,
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
            layers: &self.layers,
            transit: self.transit.as_deref(),
            parking: self.parking.as_deref(),
            mode_choice: self.mode_choice.as_deref(),
            mode_defaults: &self.mode_defaults,
            class_modes: &self.class_modes,
            loading: &self.loading,
        })
    }

    /// The same run, also recording per-link, per-time-bin results (S163) in
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

    /// Run the whole demand through the selected loading engine and report
    /// the result.
    ///
    /// Processes trips in departure-time order via an event queue keyed the
    /// way Foundations §2 fixes for every event in the core —
    /// `(second, entity_kind, entity_id)` — so ties resolve identically
    /// every run (S60). Under [`FlowMotor::Level0`] nothing in the *loading*
    /// depends on processing order (vehicles never interact, design §10.4);
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
    /// carry, fails, or gives an answer that does not fit its batch.
    ///
    /// # Panics
    ///
    /// Never in practice: the panic guards the invariant that at least one
    /// iteration always runs.
    pub fn try_execute(&mut self, diagnostics: &mut Diagnostics) -> Result<RunResult, RunError> {
        let total_trips = self.trips.len();
        let mut timings = Timings::new();
        let mut clock = Instant::now();

        // Route sets for every origin-destination pair the demand asks for,
        // made once, in parallel, before any trip runs (S165): each trip's
        // endpoints are snapped to the nearest drivable node with a grid, not
        // a scan, and the pair's set is looked up, not searched for.
        //
        // A trip that is not a car trip gets a key from a node to itself, which no
        // route set, choice or gap takes part in (S195): its own layer routes it. So does a
        // trip choosing its mode (M5): its car routes are among its alternatives, not the
        // route choice's.
        let mode_choice = self.mode_choice.clone();
        let choosing = |trip: TripId| mode_choice.is_some() && !self.trips.mode_given(trip);
        let offers = |mode: Mode| mode_choice.as_ref().is_some_and(|m| m.contains(&mode));
        let snapper = NodeSnapper::new(&self.network);
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
            &self.mode_defaults,
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
                    },
                )
            })
            .filter(|i| !i.is_empty());
        clock = timings.lap("itineraries_setup", None, clock);
        // The pairs route sets are made for: the car trips', and the car pairs of the trips
        // choosing their mode (M5), which the route update grows too.
        let car_keys: Vec<RouteKey> = match &itineraries {
            Some(itin) if mode_choice.is_some() => (0..total_trips)
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
        let inputs = route_choice::Inputs {
            network: &network,
            travellers: &travellers,
            trips: &trips,
            trip_keys: &trip_keys,
            model: model.as_ref(),
            turns: &turns,
            detour_limit: self.choice_detour_limit,
        };
        let mut chooser = route_choice::Chooser::new(&inputs, route_sets.clone())?;
        let mut route_choices = chooser.choose_all(&choice_rng, 0)?;
        clock = timings.lap("route_choice", Some(0), clock);
        let route_update = self.route_update.clone();

        let itinerary_wanted = match &itineraries {
            Some(_) => itinerary_choice::wanted(model.as_ref())?,
            None => Vec::new(),
        };
        let car_ctx = SearchContext::new(&network, &turns);
        let bike_ctx = layers_arc.bike.as_ref().map(|b| {
            SearchContext::with_costs(b.network().network(), b.turns(), b.costs().to_vec())
        });
        let bike_planner = layers_arc.bike.as_ref().zip(bike_ctx.as_ref());
        let window_s = f64::from(self.window.get());
        let mut availability =
            parking_arc.as_deref().map(|p| ExpectedAvailability::initial(p, window_s));
        let choose_inputs = ChooseInputs {
            trips: &trips,
            travellers: &travellers,
            model: model.as_ref(),
            rng: &choice_rng,
            wanted: &itinerary_wanted,
        };
        let mut chosen: Option<Chosen> = None;
        if let Some(itin) = &itineraries {
            let free = LinkTimes::free_flow(&network);
            let attributes = mode_choice.as_ref().map(|_| route_sets.attributes(&network));
            let planner = Planner {
                transit: transit_arc.as_deref().map(|t| (t, t.scheduled())),
                parking: parking_arc.as_deref(),
                car: &car_ctx,
                bike: bike_planner,
                times: &free,
                availability: availability.as_ref(),
                detour_limit: self.choice_detour_limit,
                car_routes: attributes
                    .as_ref()
                    .map(|attributes| CarRoutes { sets: &route_sets, attributes }),
            };
            chosen = Some(itin.choose(&planner, &choose_inputs, None, 0, None, None)?.0);
            clock = timings.lap("itinerary_choice", Some(0), clock);
        }

        let strategy = self.equilibration.clone();
        let max_iterations = strategy.max_iterations().max(1);
        // Link times are recorded whenever there is a next iteration to cost, at the
        // user's bin length if they asked for one, else the strategy's.
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
        let increments = match self.flow_motor {
            FlowMotor::Ltm { .. } => strategy.increments().max(1),
            FlowMotor::Level0 => 1,
        };
        // What the route update found while it was built up: iteration 0's report counts it.
        let (mut increment_searches, mut increment_added) = (0_u32, 0_u32);
        let mut increment_reroute_searches = 0_u32;
        if increments > 1 {
            let group: Vec<u32> =
                (0..travellers.len()).map(|t| increment_of(&reselect_rng, t, increments)).collect();
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
                    },
                    itineraries.as_ref().zip(chosen.as_ref()),
                    (&car_ctx, expected_times.as_ref()),
                    &mut partial_diagnostics,
                );
                clock = timings.lap("partial_loading", Some(0), clock);
                increment_reroute_searches += loaded.reroute_searches;
                let Some(bins) = &loaded.entry_bins else { break };
                let times = LinkTimes::from_tables(&self.network, bins);
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
                    increment_added +=
                        u32::try_from(found.route_count()).expect("few routes are added");
                    if !found.routes.is_empty() {
                        let (grown, shift) =
                            route_sets.extended(&found.routes, 1, &route_update.descriptor());
                        route_choices.grown(&route_sets, &shift, &grown, &trip_keys);
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
                    let realised = transit
                        .zip(loaded.transit.as_ref())
                        .map(|(transit, t)| transit.on_times(&t.times));
                    let attributes = mode_choice.as_ref().map(|_| route_sets.attributes(&network));
                    let planner = Planner {
                        transit: transit
                            .map(|t| (t, realised.as_ref().unwrap_or_else(|| t.scheduled()))),
                        parking: parking_arc.as_deref(),
                        car: &car_ctx,
                        bike: bike_planner,
                        times: &times,
                        availability: availability.as_ref(),
                        detour_limit: self.choice_detour_limit,
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
                },
                itineraries.as_ref().zip(chosen.as_ref()),
                (&car_ctx, expected_times.as_ref()),
                &mut iteration_diagnostics,
            );
            clock = timings.lap("loading", Some(iteration), clock);
            let mut report = IterationReport::unmeasured(iteration);
            report.reselected_share = arrived_by.0;
            report.changed_share = arrived_by.1;
            report.mode_changed_share = mode_changed_by;
            report.mode_changed_floor = mode_floor_by;
            report.total_travel_time_s = loaded.total_travel_time.get();
            report.completed = loaded.completion.completed;
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
                        report.routes_added =
                            u32::try_from(found.route_count()).expect("few routes are added");
                        if !found.routes.is_empty() {
                            let (grown, shift) = route_sets.extended(
                                &found.routes,
                                next,
                                &route_update.descriptor(),
                            );
                            route_choices.grown(&route_sets, &shift, &grown, &trip_keys);
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
                        let realised = transit
                            .zip(loaded.transit.as_ref())
                            .map(|(transit, t)| transit.on_times(&t.times));
                        let attributes =
                            mode_choice.as_ref().map(|_| route_sets.attributes(&network));
                        let planner = Planner {
                            transit: transit
                                .map(|t| (t, realised.as_ref().unwrap_or_else(|| t.scheduled()))),
                            parking: parking_arc.as_deref(),
                            car: &car_ctx,
                            bike: bike_planner,
                            times: &times,
                            availability: availability.as_ref(),
                            detour_limit: self.choice_detour_limit,
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
                converged = iteration + 1 < max_iterations;
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

        let (loaded, iteration_diagnostics) = last.expect("at least one iteration ran");
        diagnostics.merge(&iteration_diagnostics);
        // The chooser holds the other handle on the sets; without it they are ours.
        drop(chooser);
        let route_sets =
            Arc::try_unwrap(route_sets).unwrap_or_else(|shared| RouteSets::clone(&shared));
        // The per-link table is the user's only if they asked for it.
        let link_bins = if self.link_bin_seconds.is_some() { loaded.link_bins } else { None };
        // Each trip under the mode it took: its choice (M5), else its stated mode.
        let taken: Vec<Mode> = (0..total_trips)
            .map(|i| {
                let trip = TripId::new(i);
                itineraries
                    .as_ref()
                    .zip(chosen.as_ref())
                    .and_then(|(it, c)| it.position(trip).and_then(|p| c.alt[p].as_ref()))
                    .map_or_else(|| self.trips.mode(trip), |a| a.mode)
            })
            .collect();
        let by_mode = mode_totals(&loaded.events, &self.trips, &self.travellers, &taken);
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
            bike_link_bins,
            walk_link_bins,
            transit,
            parking: loaded.parking,
            itineraries: itinerary_result,
            gridlock: loaded.gridlock,
            reroutes: loaded.reroutes,
            routes_realised: loaded.routes_realised,
            link_peak_pcu: loaded.peak_pcu,
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
        let LoadPlan { record_bins, want_entry, level_override, active } = plan;
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
            if let Some((itineraries, chosen)) = itin {
                if let Some(position) = itineraries.position(trip) {
                    if let Some(alt) =
                        chosen.alt[position].as_ref().filter(|a| a.shape == Shape::Direct)
                    {
                        mode = alt.mode;
                        chosen_route = Some(alt.vehicle_links.clone());
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
            } else if !self.vehicles.is_at_origin(
                &self.travellers,
                traveller,
                VehicleKind::Car,
                origin,
            ) {
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
                        self.vehicles.relocate(traveller, VehicleKind::Car, destination);
                        completion.completed += 1;
                        events.push(EventRow::trip(departure, EventType::TripCompleted, trip));
                    }
                    Some(path) => {
                        let weight = self.travellers.weight(traveller);
                        self.vehicles.relocate(traveller, VehicleKind::Car, destination);

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
        let bus_load =
            transit.as_ref().filter(|t| t.rides(&self.network)).map(|t| t.bus_load(total_trips));
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
                Some(b) if want_entry => Recording::BinsAndEntry(b),
                Some(b) => Recording::Bins(b),
                None => Recording::Trajectories,
            };
            let o = self.loading;
            let rules = Rules {
                priority: o.priority,
                reroute: o.reroute.then(|| RerouteRule {
                    after_s: o.reroute_after_s,
                    max: u8::try_from(o.reroute_max).unwrap_or(u8::MAX),
                }),
                pocket_length_m: o.pocket_length_m,
            };
            let mut rerouter = o.reroute.then(|| Rerouter::new(car_ctx, expected, &o, total_trips));
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
                    }
                }
            }
        }

        if let (FlowMotor::Level0, Some(bin_seconds)) = (&self.flow_motor, record_bins) {
            let window = f64::from(self.window.get());
            if want_entry {
                let (_, exit, entry) =
                    load_level_0_recorded(&level0_vehicles, &self.network, window, bin_seconds);
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
                let executed = self.execute_itineraries(
                    &transit,
                    data,
                    itineraries,
                    chosen,
                    &pending_itin,
                    avail.as_deref(),
                    bin_walks,
                );
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
        }
    }

    /// Decide and make one bike or walk trip: whether its mode can run, whether
    /// the traveller's bike is at the origin, and its traversal of its route on
    /// the layer. A bike that travels is left at the destination.
    /// Start an itinerary trip at its departure (M4): check its vehicle is where
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
        let (bins, full) = parking_mod::tally(parking, events, window);
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

    /// Execute every pending itinerary's transit part on `data` (M4): see
    /// [`itinerary_choice::follow`]. In parallel, in fixed chunks.
    #[allow(clippy::too_many_arguments, reason = "the loading's pieces")]
    fn execute_itineraries(
        &self,
        transit: &TransitSetup,
        data: &openmobisim_core_transit::RaptorData,
        itineraries: &Itineraries,
        chosen: &Chosen,
        pending: &[PendingItin],
        avail: Option<&[f64]>,
        bin_walks: bool,
    ) -> Vec<Executed> {
        let parking = self.parking.as_deref();
        let trips = &*self.trips;
        let walk = transit.walk().network();
        let seconds = transit.walk_link_seconds();
        let d = transit.defaults();
        let transfer = transit.transfer_s();
        let walk_bound = d
            .access_walk_max_s
            .max(d.transfer_walk_max_s)
            .max(parking.map_or(0.0, |p| p.defaults().walk_max_s))
            + 1.0;
        let bins = parking
            .zip(avail)
            .map(|(p, _)| parking_mod::tally(p, &[], f64::from(self.window.get())).0);
        par_map(
            pending,
            itinerary_choice::SCRATCH_CHUNK,
            || {
                (
                    Reach::new(walk, seconds),
                    Raptor::new(data, d.max_rides as usize),
                    vec![false; transit.walk_node_count()],
                )
            },
            |(reach, raptor, wanted), pend| {
                let alt =
                    chosen.alt[pend.position].as_ref().expect("a pending trip has its choice");
                let nodes = itineraries.nodes(pend.position);
                let departure = trips.departure(pend.trip).get();
                let truncated = || Executed {
                    outcome: ItinOutcome::Truncated,
                    followed: None,
                    mismatch: None,
                    walks: Vec::new(),
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
                        _ => transit.egress(reach, nodes.walk_d),
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
                        if let Some(links) =
                            TransitSetup::walk_path(reach, wanted, a, b, walk_bound)
                        {
                            if !links.is_empty() {
                                walks.push((links, seg.departure));
                            }
                        }
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
                Executed { outcome, followed, mismatch, walks }
            },
        )
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

/// Each mode's outcomes, from the events of a loading (one per trip) and the
/// trips' departures (S195): the same travel time a trip adds to the run's
/// total, filed under its mode.
fn mode_totals(
    events: &[EventRow],
    trips: &Trips,
    travellers: &Travellers,
    taken: &[Mode],
) -> [ModeTotals; Mode::COUNT] {
    let mut out = [ModeTotals::default(); Mode::COUNT];
    for raw in 0..trips.len() {
        let trip = TripId::new(raw);
        let totals = &mut out[taken[trip.index()].index()];
        totals.completion.total_trips += 1;
        totals.weighted_trips += f64::from(travellers.weight(trips.traveller(trip)));
    }
    for e in events {
        let trip = TripId::new(e.entity_id);
        let totals = &mut out[taken[trip.index()].index()];
        let c = &mut totals.completion;
        match e.event_type {
            EventType::TripCompleted => {
                c.completed += 1;
                let weight = travellers.weight(trips.traveller(trip));
                totals.total_travel_time += (Duration::from_clock(e.second)
                    - Duration::from_clock(trips.departure(trip)))
                    * f64::from(weight);
            }
            EventType::TripTruncated => c.truncated += 1,
            EventType::NoVehicleAvailable => c.no_vehicle_available += 1,
            EventType::NoFeasiblePath => c.no_feasible_path += 1,
            EventType::ModeNotAvailable => c.mode_not_available += 1,
        }
    }
    out
}
