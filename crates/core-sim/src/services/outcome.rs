//! What a fleet day returns: per leg what became of it, and the day's counts.

/// What became of one service leg by the window's end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LegStatus {
    /// Its departure is after the window's end: never requested.
    NotRequested,
    /// It asked for a vehicle and is waiting for one.
    Waiting,
    /// A vehicle or bike was taken and the leg is under way.
    Riding,
    /// It reached its destination.
    Served,
    /// No vehicle or bike was found: the leg was not ridden.
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
    /// The second it reached its destination (`NaN` if it did not).
    pub arrived_s: f64,
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
}
