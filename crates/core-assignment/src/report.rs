//! What each assignment iteration reports.
//!
//! An [`IterationReport`] holds the convergence measures of one iteration, each defined over
//! the trips of the demand, every trip weighted by its traveller's weight. They are measures,
//! not a judgement: the report has no field that says whether the assignment converged; the
//! stopping rule ([`crate::Params`]) reads the measures against named tolerances.
//!
//! Costs below are in utility units (minus the systematic utility of the trip's user class,
//! [`crate::Coefficients::cost`]), read on the link travel times the iteration's loading
//! produced, after the measure stage re-priced every chain of every set on them.

/// The wall-clock seconds each stage of one iteration took.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StageSeconds {
    /// The chain search, with the stopping check.
    pub search: f64,
    /// The choice.
    pub choose: f64,
    /// The loading.
    pub load: f64,
    /// The re-pricing of every chain on the loaded link times.
    pub measure: f64,
    /// The convergence measures.
    pub assess: f64,
}

/// The convergence measures of one assignment iteration. See the [module docs](self).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IterationReport {
    /// The iteration, from 0.
    pub iteration: u32,
    /// How many trips the search found a chain for that was not yet in their choice set.
    pub found: u32,
    /// The share of trips (weighted) the search found a new chain for.
    pub found_share: f64,
    /// The share of trips facing a choice (weighted) whose chain after this iteration's choice
    /// differs from the one before it; a trip that had no chain before counts as switching.
    pub switch_share: f64,
    /// What the switch share would be by chance alone at a stationary state:
    /// `Σ w · α · (1 − Σ_k P_k²) / Σ w` over the trips facing a choice, `α` the probability of
    /// re-choosing in this iteration (1 for a trip that had no chain), `P_k` the choice model's
    /// probability of chain `k` at this iteration's costs.
    pub switch_floor: f64,
    /// The switch share's excess over its floor per re-chooser, `(switch share − floor) /
    /// (Σ w · α / Σ w)`: the share of the trips that reconsidered which moved beyond chance; 0
    /// at a stationary state, and under a deterministic model the share of trips that were not
    /// on their best chain. The stopping rule reads it.
    pub switch_excess: f64,
    /// The summed weight of the trips that set out (every trip with a chain).
    pub departed: f64,
    /// The summed weight of the trips that reached their destination before the loading ended.
    pub arrived: f64,
    /// The summed weight of the trips still under way when the loading ended.
    pub unfinished: f64,
    /// [`Self::unfinished`] over [`Self::departed`] (0 when nobody set out).
    pub unfinished_share: f64,
    /// The summed weight of the trips with no chain at all (no route found, or no planner for
    /// their given mode); they do not travel.
    pub without_chain: f64,
    /// The relative duality gap within the choice sets: `(Σ w · paid − Σ w · least) / Σ w · least`,
    /// where `paid` is the cost of the chain the trip took and `least` the least cost of any
    /// chain of its set (0 when every least cost is 0).
    pub duality_gap: f64,
    /// The deterministic part of the duality gap: `(Σ w · paid − Σ w · expected) / Σ w · least`,
    /// where `expected = Σ_k P_k · cost_k` over the set under the choice model at these costs:
    /// the duality gap less the part the model's own dispersion produces. Equal to the duality
    /// gap under a deterministic model.
    pub gap_deterministic: f64,
    /// The spread: the share of trips (weighted) on a chain of least cost in their set.
    pub on_cheapest_share: f64,
    /// The mean time from departure to arrival, in seconds, over the trips that arrived
    /// (weighted), as the loading moved them.
    pub mean_trip_seconds: f64,
    /// What each stage cost in wall-clock seconds.
    pub stage_seconds: StageSeconds,
}

impl IterationReport {
    /// The same report with every stage time set to zero: what two runs that should agree are
    /// compared on (wall-clock times never repeat).
    #[must_use]
    pub fn without_times(&self) -> Self {
        Self { stage_seconds: StageSeconds::default(), ..*self }
    }
}
