//! What a fleet day returns: per leg what became of it, and the day's counts.

pub use super::decisions::{DecisionKind, DecisionRecord};
pub use super::docks::{DockBins, DockEvent, DockEventKind};
pub use super::taxis::{FLEET_BIN_S, FleetBins, TaxiDay, TaxiRequest, VehicleDay};

/// What became of one service leg by the window's end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LegStatus {
    /// Its departure is after the window's end: never requested.
    NotRequested,
    /// It asked for a vehicle and is waiting for one.
    Waiting,
    /// A vehicle or bike was taken and the leg is under way.
    Riding,
    /// It was carried: a bike docked (its arrival, after the walk on, is [`LegOutcome::arrived_s`]),
    /// or every one of a taxi leg's unit requests dropped at its destination.
    Served,
    /// No vehicle or bike was found: a bike leg was not ridden and the traveller walked on; a
    /// taxi leg: one of its unit requests waited past the longest wait, and the leg ends there,
    /// with no walk on.
    Failed,
    /// Requested or under way when the window ended.
    Truncated,
}

impl LegStatus {
    /// The stable snake_case name, as a results table writes it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            LegStatus::NotRequested => "not_requested",
            LegStatus::Waiting => "waiting",
            LegStatus::Riding => "riding",
            LegStatus::Served => "served",
            LegStatus::Failed => "failed",
            LegStatus::Truncated => "truncated",
        }
    }
}

/// One leg's outcome.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LegOutcome {
    /// What became of it.
    pub status: LegStatus,
    /// The second it asked for a vehicle (`NaN` if never).
    pub requested_s: f64,
    /// The second it reached its destination, by the service or, if a bike leg failed, on
    /// foot (a taxi leg: its last unit request's drop-off; `NaN` if it did not).
    pub arrived_s: f64,
    /// A bike leg's dock the bike was taken from (`None`: not taken).
    pub picked: Option<u32>,
    /// The dock the bike was returned to (`None`: not returned).
    pub returned: Option<u32>,
    /// The second the traveller had a bike, or gave up; a taxi leg: the second its last unit
    /// request's vehicle reached the pickup (`NaN` if neither).
    pub picked_s: f64,
    /// Seconds walked between docks looking for a bike.
    pub extra_walk_s: f64,
    /// A bike leg's extra seconds at the return: the rides between return docks after a full
    /// one, plus the walk from the dock the bike was left at less the planned walk from the
    /// planned return dock (0 for a bike left at the planned return dock, above capacity
    /// included, and for a leg that never returned one). The rest of a leg's extra seconds
    /// (its arrival less its arrival had the planned docks had a bike and a free slot) is the
    /// pickup's.
    pub return_extra_s: f64,
    /// Docks tried after the planned one, at the pickup and at the return.
    pub redirections: u32,
    /// Whether the bike was docked above capacity, every dock near the return being full.
    pub overflow: bool,
    /// A taxi leg's vehicle: the one sent to its last unit request (`None`: none was).
    pub vehicle: Option<u32>,
    /// The second that vehicle was sent (`NaN` if none was).
    pub assigned_s: f64,
    /// A taxi leg's wait, in seconds: from its request until its last unit request's vehicle
    /// reached the pickup, that unit failed, or the window ended (0 for a bike leg).
    pub waited_s: f64,
    /// A bike leg's docks found without a bike for it (the last one included when it gave up).
    pub failed_for_bike: u32,
    /// A bike leg's docks found without a free slot for it (the last one included when it left
    /// its bike above capacity).
    pub failed_for_slot: u32,
    /// Metres walked between docks looking for a bike, as the crow flies.
    pub extra_walk_m: f64,
    /// The second the bike was docked (`NaN`: not docked).
    pub returned_s: f64,
}

impl LegOutcome {
    /// A leg not yet requested.
    pub const NOT_REQUESTED: Self = Self {
        status: LegStatus::NotRequested,
        requested_s: f64::NAN,
        arrived_s: f64::NAN,
        picked: None,
        returned: None,
        picked_s: f64::NAN,
        extra_walk_s: 0.0,
        return_extra_s: 0.0,
        redirections: 0,
        overflow: false,
        vehicle: None,
        assigned_s: f64::NAN,
        waited_s: 0.0,
        failed_for_bike: 0,
        failed_for_slot: 0,
        extra_walk_m: 0.0,
        returned_s: f64::NAN,
    };
}

/// One fleet day's result.
#[derive(Clone, Debug, PartialEq)]
pub struct FleetOutcome {
    /// Per leg, in the order the day gave them.
    pub legs: Vec<LegOutcome>,
    /// The events handled.
    pub events: u64,
    /// The policy steps taken.
    pub steps: u32,
    /// The second the day ended (the window's end).
    pub end_s: f64,
    /// Events after which a conservation check failed: always 0 unless there is a bug (debug
    /// builds stop at the first instead).
    pub invariant_violations: u32,
    /// Per service, in the run's order: a docked service's counts bin by bin (`None` for a
    /// taxi fleet).
    pub docks: Vec<Option<DockBins>>,
    /// Per service: the most bikes docked above a dock's capacity at once (0 for a taxi fleet).
    pub overflow_max: Vec<f64>,
    /// Per service: a docked service's events in time order (`None` for a taxi fleet, or with
    /// the service's `log_events` off).
    pub dock_log: Vec<Option<Vec<DockEvent>>>,
    /// Per service: a taxi fleet's unit requests, vehicles' days and states bin by bin
    /// (`None` for a docked service).
    pub taxis: Vec<Option<TaxiDay>>,
    /// The policy's decisions, in the order taken, each with what became of it (empty without
    /// a policy), and the moves the target solver made.
    pub decisions: Vec<DecisionRecord>,
}
