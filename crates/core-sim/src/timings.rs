//! Wall-clock time by stage (S223): what a run spent where, on the machine's own clock.
//!
//! Times differ from one run to the next, so they are kept apart from every result and from the
//! fingerprint: [`crate::Run::timings`] after the run, never in [`crate::RunResult`]. A stage is
//! named once and may be recorded several times (one row per loading of the free-flow loading's
//! increments, for instance); a reader adds up the rows it wants.
//!
//! **Cost:** one clock reading and one 32-byte row per stage; a run of ten iterations records
//! about fifty.

use std::time::Instant;

/// One stage's elapsed wall-clock time.
#[derive(Clone, Debug, PartialEq)]
pub struct Stage {
    /// What was timed (see [`Timings::lap`] for the names a run records).
    pub name: &'static str,
    /// The iteration it belongs to; `None` before the iterations start or after they end.
    pub iteration: Option<u32>,
    /// Elapsed wall-clock seconds.
    pub seconds: f64,
}

/// The stages timed so far, in the order they ended.
#[derive(Clone, Debug, Default)]
pub struct Timings {
    stages: Vec<Stage>,
}

impl Timings {
    /// No stages yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record the time from `start` to now as `name` in `iteration`, and return now, the start
    /// of whatever comes next.
    ///
    /// The names a run records: `static_routes` (bike and walk routes), `itineraries_setup`,
    /// `route_sets`; per iteration `route_choice` and `itinerary_choice` (the choices made for
    /// it; iteration 0's are everyone's first), `partial_loading`, `route_update` (iteration 0's
    /// increments, S223: the loadings of part of the demand and what follows each), `loading`,
    /// then, after a loading, `route_update`, `route_choice`, `itinerary_choice` for the next
    /// iteration (recorded under the loading's) and `network_gap` after the last; and
    /// `results_assembled` at the end.
    pub fn lap(&mut self, name: &'static str, iteration: Option<u32>, start: Instant) -> Instant {
        let now = Instant::now();
        self.stages.push(Stage { name, iteration, seconds: (now - start).as_secs_f64() });
        now
    }

    /// The stages, in the order they ended.
    #[must_use]
    pub fn stages(&self) -> &[Stage] {
        &self.stages
    }
}
