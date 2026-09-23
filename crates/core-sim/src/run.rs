//! The `Run` (S62): orchestration, trip completion statistics (S57), driven
//! by an event queue in seconds (S88).
//!
//! One run, car trips only: `core-demand`'s travellers and trips, a route set
//! for every origin-destination pair (`core-routes`, S166), a route per trip
//! from a choice model (`core-choice`, S169), a loading by the chosen
//! [`FlowMotor`], and — under an [`Equilibration`] — choice and loading
//! repeated, with the route sets grown between iterations by a
//! [`RouteUpdate`] and a convergence report per iteration (S170, S176). No
//! hubs and no other mode yet: those are `core-sim`'s later job (Foundations
//! §10).
//!
//! The defaults here are the core's own and unconditional (`Level0`,
//! `deterministic`, `none`); the Python `Scenario` layers its own on top
//! (S179, S187, S191).

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::sync::Arc;

use openmobisim_core_choice::{ChoiceError, ChoiceModel, Deterministic};
use openmobisim_core_demand::{Travellers, Trips, VehicleKind, VehicleLocations};
use openmobisim_core_graph::defaults::SignalDefaults;
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_loading::{
    EntryTables, FidelityLevel, LinkBins, Vehicle, load_level_0_binned, load_level_0_recorded,
    run_ltm_binned, run_ltm_recorded, traverse_free_flow,
};
use openmobisim_core_routes::{
    Demand, NodeSnapper, RouteKey, RouteSetGenerator, RouteSets, TripDemand, default_generator,
};
use openmobisim_core_types::diagnostics::{
    Category, DiagKey, Diagnostics, ElementRef, Severity, codes,
};
use openmobisim_core_types::ids::{EntityId, EntityKind, LinkId, TravellerId, TripId, VehicleId};
use openmobisim_core_types::rng::{RngKey, Stream, StreamRng};
use openmobisim_core_types::time::{EventKey, Second};
use openmobisim_core_types::units::{Duration, Pcu};

use crate::equilibration::{Equilibration, IterationReport, NoEquilibration};
use crate::events::{EventRow, EventType};
use crate::identity::{Inputs, RunDescription, describe};
use crate::link_times::{LinkTimes, relative_time_change};
use crate::route_cache::{RouteSetCache, generation_key};
use crate::route_choice::{self, NO_ROUTE, RouteChoices};
use crate::route_update::{NoRouteUpdate, RouteUpdate, UpdateContext};

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
}

/// How one loading is made besides the routes it follows.
#[derive(Clone, Copy)]
struct LoadPlan {
    /// Record per-link results in bins of this many seconds, if asked.
    record_bins: Option<u32>,
    /// Also record the entry-time tables the next iteration's costs are read from.
    want_entry: bool,
    /// Load with this level of the link transmission model instead of the run's (S178: the
    /// warm-up's point-queue model).
    level_override: Option<FidelityLevel>,
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
            equilibration: Arc::new(NoEquilibration),
            route_update: Arc::new(NoRouteUpdate),
            choice_detour_limit: DEFAULT_CHOICE_DETOUR_LIMIT,
            route_cache: None,
        }
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

        // Route sets for every origin-destination pair the demand asks for,
        // made once, in parallel, before any trip runs (S165): each trip's
        // endpoints are snapped to the nearest drivable node with a grid, not
        // a scan, and the pair's set is looked up, not searched for.
        let snapper = NodeSnapper::new(&self.network);
        let trip_keys: Vec<RouteKey> = (0..total_trips)
            .map(|i| {
                let trip = TripId::new(i);
                RouteKey::new(
                    snapper.nearest(&self.network, self.trips.origin(trip)),
                    snapper.nearest(&self.network, self.trips.destination(trip)),
                )
            })
            .collect();
        let turns = match &self.flow_motor {
            FlowMotor::Ltm { turns, .. } => turns.clone(),
            FlowMotor::Level0 => Arc::new(TurnTable::build(&self.network, SignalDefaults::SHIPPED)),
        };
        // A method that reads the demand (the Monte Carlo method's bias, S176) is told it
        // first; one that does not costs nothing here.
        let demand_rows: Option<Vec<TripDemand>> = self.route_generator.reads_demand().then(|| {
            (0..total_trips)
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
            RouteSets::generate(&self.network, &turns, &trip_keys, generator.as_ref())
        };
        // Sets already generated for these inputs are handed back, if the run has a cache (S178).
        let mut route_sets = match &self.route_cache {
            Some(cache) => {
                let key = generation_key(
                    &self.network,
                    self.route_generator.as_ref(),
                    &trip_keys,
                    demand_rows.as_deref(),
                );
                cache.get_or_generate(key, generate)
            }
            None => Arc::new(generate()),
        };

        // Choice and equilibration (S169, S170): every trip chooses a route on
        // free-flow costs; then, under an equilibration strategy, the network is
        // loaded, the link times it produced re-cost the routes, some travellers
        // choose again, and the network is loaded again.
        let choice_rng = StreamRng::new(RngKey::from_seed(self.master_seed), Stream::Choice);
        let reselect_rng =
            StreamRng::new(RngKey::from_seed(self.master_seed), Stream::MsaReselection);
        // Shared handles, so the loading (which moves vehicles about in `self`) and the
        // chooser (which only reads the inputs) do not borrow each other.
        let (network, travellers, trips) =
            (self.network.clone(), self.travellers.clone(), self.trips.clone());
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
        let route_update = self.route_update.clone();

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
        let mut converged = false;
        let mut last: Option<(Loaded, Diagnostics)> = None;

        // The first loadings may be made with the point-queue model, which cannot gridlock (S178);
        // the last, the run's result, never is.
        let warmup = strategy.warmup_iterations().min(max_iterations - 1);
        for iteration in 0..max_iterations {
            let mut iteration_diagnostics = Diagnostics::new();
            let loaded = self.load_once(
                &trip_keys,
                &route_sets,
                &route_choices,
                LoadPlan {
                    record_bins,
                    want_entry: max_iterations > 1,
                    level_override: (iteration < warmup).then_some(FidelityLevel::PointQueue),
                },
                &mut iteration_diagnostics,
            );
            let mut report = IterationReport::unmeasured(iteration);
            report.reselected_share = arrived_by.0;
            report.changed_share = arrived_by.1;
            report.total_travel_time_s = loaded.total_travel_time.get();
            report.completed = loaded.completion.completed;
            report.truncated = loaded.completion.truncated;
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
                            trip_keys: &trip_keys,
                            route_sets: &route_sets,
                            times: &times,
                            iteration: next,
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
                    report.gap = update.assessment.gap;
                    report.gap_expected = update.assessment.gap_expected;
                    report.gap_excess = update.assessment.gap_excess;
                    report.incomplete_share = update.assessment.incomplete_share;
                    report.gap_flow = update.assessment.gap_flow;
                    report.gap_flow_floor = update.assessment.gap_flow_floor;
                    report.gap_flow_excess =
                        update.assessment.gap_flow - update.assessment.gap_flow_floor;
                    times_now = Some(times);
                    arrived_by =
                        (update.assessment.reselected_share, update.assessment.changed_share);
                    changes = update.changes;
                    assessed = Some((unfinished, update.expected_seconds));
                }
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
                }
                converged = iteration + 1 < max_iterations;
                break;
            }
            for c in changes {
                route_choices.route[c.trip] = c.route;
                route_choices.probability[c.trip] = c.probability;
            }
        }

        let (loaded, iteration_diagnostics) = last.expect("at least one iteration ran");
        diagnostics.merge(&iteration_diagnostics);
        // The chooser holds the other handle on the sets; without it they are ours.
        drop(chooser);
        let route_sets =
            Arc::try_unwrap(route_sets).unwrap_or_else(|shared| RouteSets::clone(&shared));
        // The per-link table is the user's only if they asked for it.
        let link_bins = if self.link_bin_seconds.is_some() { loaded.link_bins } else { None };
        Ok(RunResult {
            total_travel_time: loaded.total_travel_time,
            completion: loaded.completion,
            events: loaded.events,
            link_bins,
            route_sets: Some(route_sets),
            route_choices: Some(route_choices),
            iterations: reports,
            converged,
        })
    }

    /// Load the whole demand once, each trip on the route `route_choices` gives it,
    /// recording per-link results in bins of `record_bins` seconds if asked.
    fn load_once(
        &mut self,
        trip_keys: &[RouteKey],
        route_sets: &RouteSets,
        route_choices: &RouteChoices,
        plan: LoadPlan,
        diagnostics: &mut Diagnostics,
    ) -> Loaded {
        let LoadPlan { record_bins, want_entry, level_override } = plan;
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
            if !self.vehicles.is_at_origin(&self.travellers, traveller, VehicleKind::Car, origin) {
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
                let route = if route_key.origin == route_key.destination {
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

        if let FlowMotor::Ltm { turns, step, level } = &self.flow_motor {
            let level = &level_override.unwrap_or(*level);
            let vehicles: Vec<Vehicle> =
                pending_ltm.iter().map(|(_, vehicle, _)| vehicle.clone()).collect();
            let window = Duration::from_clock(self.window);
            let (trajectories, bins, entry) = match record_bins {
                Some(bin_seconds) if want_entry => {
                    let (t, b, e) = run_ltm_recorded(
                        &self.network,
                        turns,
                        &vehicles,
                        window,
                        *step,
                        *level,
                        bin_seconds,
                    );
                    (t, Some(b), Some(e))
                }
                Some(bin_seconds) => {
                    let (t, b) = run_ltm_binned(
                        &self.network,
                        turns,
                        &vehicles,
                        window,
                        *step,
                        *level,
                        bin_seconds,
                    );
                    (t, Some(b), None)
                }
                None => (
                    openmobisim_core_loading::run_ltm(
                        &self.network,
                        turns,
                        &vehicles,
                        window,
                        *step,
                        *level,
                    ),
                    None,
                    None,
                ),
            };
            link_bins = bins;
            entry_bins = entry;
            let by_vehicle: HashMap<VehicleId, _> =
                trajectories.into_iter().map(|t| (t.vehicle, t)).collect();

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

        Loaded { total_travel_time, completion, events, link_bins, entry_bins }
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
