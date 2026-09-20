//! The `Run` (S62): orchestration for Phase 1 — one KPI, trip completion
//! statistics (S57), driven by an event queue in seconds (S88).
//!
//! Phase 1's whole vertical slice: `core-demand`'s travellers and trips,
//! S133's placeholder routing, `core-loading`'s level-0 traversal, one
//! number out. No hubs, no equilibration, no convergence report — those are
//! part of `core-sim`'s eventual job (Foundations §10) but not this first,
//! narrowest cut of it.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::sync::Arc;

use openmobisim_core_demand::{Travellers, Trips, VehicleKind, VehicleLocations};
use openmobisim_core_graph::defaults::SignalDefaults;
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_loading::{
    FidelityLevel, LinkBins, Vehicle, load_level_0_binned, run_ltm_binned, traverse_free_flow,
};
use openmobisim_core_routes::{
    NodeSnapper, RouteKey, RouteSetGenerator, RouteSets, default_generator,
};
use openmobisim_core_types::diagnostics::{
    Category, DiagKey, Diagnostics, ElementRef, Severity, codes,
};
use openmobisim_core_types::ids::{EntityId, EntityKind, LinkId, TravellerId, TripId, VehicleId};
use openmobisim_core_types::time::{EventKey, Second};
use openmobisim_core_types::units::{Duration, Pcu};

use crate::events::{EventRow, EventType};

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
    /// Until the choice layer exists each trip takes its pair's best route.
    pub route_sets: Option<RouteSets>,
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
}

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
    pub fn execute(&mut self, diagnostics: &mut Diagnostics) -> RunResult {
        let mut queue: BinaryHeap<Reverse<EventKey>> = BinaryHeap::new();
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
        let route_sets =
            RouteSets::generate(&self.network, &turns, &trip_keys, self.route_generator.as_ref());

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
                    route_sets
                        .best(route_key)
                        .map(|r| r.links.iter().map(|&l| LinkId::new(l)).collect::<Vec<LinkId>>())
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
                                if self.link_bin_seconds.is_some() {
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
            let vehicles: Vec<Vehicle> =
                pending_ltm.iter().map(|(_, vehicle, _)| vehicle.clone()).collect();
            let window = Duration::from_clock(self.window);
            let (trajectories, bins) = match self.link_bin_seconds {
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
                    (t, Some(b))
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
                ),
            };
            link_bins = bins;
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

        if let (FlowMotor::Level0, Some(bin_seconds)) = (&self.flow_motor, self.link_bin_seconds) {
            let window = f64::from(self.window.get());
            link_bins =
                Some(load_level_0_binned(&level0_vehicles, &self.network, window, bin_seconds).1);
        }

        RunResult { total_travel_time, completion, events, link_bins, route_sets: Some(route_sets) }
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
