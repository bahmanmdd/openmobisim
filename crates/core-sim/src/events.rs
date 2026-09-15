//! Per-trip event rows — the in-memory shape `io-parquet`'s `events.parquet`
//! writer reads from (Foundations §6).

use openmobisim_core_types::ids::{EntityId, EntityKind, TripId};
use openmobisim_core_types::time::Second;

/// One row of what will become `events.parquet`.
///
/// Foundations §6 also names a `payload` column; Phase 1 has nothing
/// meaningful to put there — no boardings, hubs or disruptions exist yet to
/// produce one — so it is not included here, added when a real payload
/// exists rather than shipped empty now.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct EventRow {
    /// When the event fired.
    pub second: Second,
    /// What happened.
    pub event_type: EventType,
    /// Always [`EntityKind::Trip`] in Phase 1 — the only entity kind this
    /// run produces events about.
    pub entity_kind: EntityKind,
    /// The trip's raw id.
    pub entity_id: u32,
}

impl EventRow {
    /// An event about `trip`.
    #[must_use]
    pub fn trip(second: Second, event_type: EventType, trip: TripId) -> Self {
        Self { second, event_type, entity_kind: EntityKind::Trip, entity_id: trip.raw() }
    }
}

/// What kind of thing happened — Phase 1's vocabulary, matching
/// [`crate::run::TripCompletionStats`]'s four buckets exactly.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EventType {
    /// A trip's trajectory finished within the simulation window.
    TripCompleted,
    /// A trip's trajectory was still in progress when the window ended
    /// (S57).
    TripTruncated,
    /// A trip's traveller had no owned car at the trip's origin.
    NoVehicleAvailable,
    /// A car was available, but `core-sim`'s placeholder router (S133)
    /// found no path.
    NoFeasiblePath,
}

impl EventType {
    /// The stable snake_case name written to `events.parquet`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            EventType::TripCompleted => "trip_completed",
            EventType::TripTruncated => "trip_truncated",
            EventType::NoVehicleAvailable => "no_vehicle_available",
            EventType::NoFeasiblePath => "no_feasible_path",
        }
    }
}
