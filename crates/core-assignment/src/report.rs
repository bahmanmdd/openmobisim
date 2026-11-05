//! What each assignment iteration reports.
//!
//! An [`IterationReport`] holds the convergence measures of one iteration, each defined over
//! the trips of the demand, every trip weighted by its traveller's weight. They are measures,
//! not a judgement: the report has no field that says whether the assignment converged; the
//! stopping rule ([`crate::Params`]) reads the measures against named tolerances.
//!
//! **The stopping shares count the trips that could change their choice** (the trips of
//! [`IterationReport::choosing`]): car trips, which may change route, and trips choosing their
//! mode, which may change route or mode ([`crate::modes`]). A trip whose mode is stated as the
//! bike or walking has its chains planned once, on static costs (one chain, or with sampled bike
//! route sets one per route of its pair, among which its choice probabilities never change), so
//! it is left out of the
//! found, added and switch shares, the switch floor and both switch excesses; a run with many
//! such trips reports the same shares for the same car traffic. **The duality gap, its
//! deterministic part and the spread** are over the same trips (those of them with a chain),
//! for the same reason: a trip of one chain pays its least cost and would only lower them, and
//! one of a sampled bike set pays the static spread of its choice model. The
//! trips' counts (departed, arrived, unfinished, without a chain) and the mean trip time are over
//! every trip.
//!
//! **Each mode's shares** ([`PerMode`]). Two are realised shares, grouped by the mode of the
//! chain each trip held at the start of the iteration (before its search and choice): the switch
//! share among the trips that held a car chain, a bike chain, a walking chain
//! ([`IterationReport::switch_share_by_mode`]), and likewise the found share. They describe who
//! moved; **they are not measures of stationarity**: grouping by the chain held conditions on
//! the outcome of the last choice (a walker holds walking although walking was unlikely for most
//! trips like theirs), so a group's share is not 0, nor its own floor, at a stationary state. A
//! trip with no chain yet (every trip in iteration 0) is in no mode's group; a mode no trip held
//! has no share (`None`).
//!
//! **The stationarity measure by mode** is the expected change of each mode's share,
//! [`IterationReport::mode_share_change_expected`]: over the trips that could change their
//! choice and face one, `D_m = Σ_t w_t (P_t,m − 1[c_t is of mode m]) / Σ_t w_t`, with `P_t,m`
//! the model's probability of all of trip `t`'s chains of mode `m` at this iteration's costs
//! (before its choice), `c_t` the chain the trip holds and `w_t` its traveller's weight (a trip
//! with no chain yet contributes 0, as in the expected switch excess). If every trip
//! reconsidered, mode `m`'s share among these trips would become `Σ w P_m / Σ w` in expectation,
//! against `Σ w 1[c is m] / Σ w` now: `D_m` is the difference. **It is 0 in expectation at a
//! stationary state**: there each trip's chain is a draw from its probabilities, so
//! `E[1[c_t is m]] = P_t,m` and every term has mean 0 (its sampling deviation is
//! `√(Σ w² P_m (1 − P_m)) / Σ w`). Away from it, `D_m > 0` says mode `m` would still gain trips,
//! `D_m < 0` that it would lose them. The three sum to 0 (each trip holds one chain, and its
//! probabilities sum to 1). With them, the share of the trips choosing their mode that changed
//! **mode** in the choice ([`IterationReport::mode_change_share`]: not only route) and the split
//! of those trips over the modes after it ([`IterationReport::mode_split`]).
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

/// One value per mode of the engine's chains: the car, the bike, walking, transit; `None` for a mode
/// whose group of trips is empty (see the [module docs](self)).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PerMode {
    /// The car's.
    pub car: Option<f64>,
    /// The bike's.
    pub bike: Option<f64>,
    /// Walking's.
    pub walk: Option<f64>,
    /// Transit's.
    pub transit: Option<f64>,
}

impl PerMode {
    /// The value of `mode` (`None` for a mode other than the four).
    #[must_use]
    pub fn of(&self, mode: openmobisim_core_demand::Mode) -> Option<f64> {
        use openmobisim_core_demand::Mode;
        match mode {
            Mode::Car => self.car,
            Mode::Bike => self.bike,
            Mode::Walk => self.walk,
            Mode::Transit => self.transit,
            _ => None,
        }
    }

    /// `part / whole` per mode, `None` where the whole is 0, from sums by mode in the order car,
    /// bike, walking, transit.
    pub(crate) fn shares(part: [f64; 4], whole: [f64; 4]) -> Self {
        let share = |k: usize| (whole[k] > 0.0).then(|| part[k] / whole[k]);
        Self { car: share(0), bike: share(1), walk: share(2), transit: share(3) }
    }
}

/// The convergence measures of one assignment iteration. See the [module docs](self).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IterationReport {
    /// The iteration, from 0.
    pub iteration: u32,
    /// How many trips that could change their choice ([`Self::choosing`]) the search found a
    /// chain for that was not yet in their choice set and
    /// that beats the cheapest chain of the set by more than the indifference band,
    /// `cost_new < cost_best − ε · |cost_best|` ([`crate::Params::indifference_band`]; every
    /// chain of a trip whose set was empty).
    pub found: u32,
    /// The share of the trips that could change their choice ([`Self::choosing`], weighted) of
    /// [`Self::found`]: what the stopping rule reads.
    pub found_share: f64,
    /// How many trips that could change their choice had a chain added to their choice set
    /// (every chain not yet in it,
    /// within the band or beyond it): at least [`Self::found`].
    pub added: u32,
    /// The share of the trips that could change their choice (weighted) of [`Self::added`].
    pub added_share: f64,
    /// The summed weight of the trips that could change their choice: car trips and trips
    /// choosing their mode, every trip whose mode is not stated as the bike or walking. The
    /// found and added shares are over these trips; the switch shares, the floor and the
    /// excesses over those of them facing a choice (with a non-empty choice set).
    pub choosing: f64,
    /// The share of the trips that could change their choice and face one (weighted) whose
    /// chain after this iteration's choice differs from the one before it; a trip that had no
    /// chain before counts as switching.
    pub switch_share: f64,
    /// What the switch share would be by chance alone at a stationary state:
    /// `Σ w · α · (1 − Σ_k P_k²) / Σ w` over the trips of the switch share, `α` the probability of
    /// re-choosing in this iteration (1 for a trip that had no chain), `P_k` the choice model's
    /// probability of chain `k` at this iteration's costs.
    pub switch_floor: f64,
    /// The switch share's excess over its floor per re-chooser, `(switch share − floor) /
    /// (Σ w · α / Σ w)`: the share of the trips that reconsidered which moved beyond chance; 0
    /// at a stationary state. Reported only: a share among the travellers who happened to
    /// reconsider, it is a noisy estimate of [`Self::switch_excess_expected`], which the
    /// stopping rule reads.
    ///
    /// Under the deterministic choice with its indifference band the floor counts the tie
    /// chance (`α · (1 − 1 / k)` for a trip facing `k` tied chains), so the excess is the
    /// share of re-choosers that were on a chain beyond the band of their best (or that the even
    /// split moved more often than chance would): the excess beyond the band, 0 at a stationary
    /// state within the even split's rounding.
    pub switch_excess: f64,
    /// The switch share above its floor over all trips, `switch share − floor`: the share of
    /// all trips (weighted) that changed chain beyond chance, which is [`Self::switch_excess`]
    /// times the share that reconsidered. Reported, not read by the stopping rule: it shrinks
    /// with the averaging step whether or not the re-choosers are near a stationary state.
    pub switch_above_floor: f64,
    /// The expected switch excess over the whole population, before this iteration's choice:
    /// `Σ w (Σ_k P_k² − P_current) / Σ w` over the trips of the switch share, `P` the model's
    /// probabilities at the costs the last loading produced, `P_current` that of the chain the
    /// trip has (a trip with no chain yet contributes 0, as in the realised excess): the share of trips that would change chain beyond the
    /// stationary chance if everyone reconsidered. 0 at a stationary state; under the
    /// deterministic choice the share of trips on a chain beyond the band of their best; it can
    /// be negative for the logit. The stopping rule reads it ([`crate::Params::switch_tolerance`]).
    pub switch_excess_expected: f64,
    /// [`Self::found_share`] grouped by the mode of each trip's chain at the start of the
    /// iteration: per mode, the share (weighted) of the trips that could change and held a chain
    /// of that mode for which the search found a chain beyond the indifference band. Realised and
    /// descriptive; not a measure of stationarity (see the [module docs](self)).
    pub found_share_by_mode: PerMode,
    /// [`Self::switch_share`] grouped by the mode of the chain each trip held at the start of the
    /// iteration: per mode, the share (weighted) of the trips that could change, faced a choice
    /// and held a chain of that mode whose chain after the choice is another one (of any mode).
    /// Realised and descriptive; not a measure of stationarity (see the [module docs](self)).
    pub switch_share_by_mode: PerMode,
    /// The expected change of each mode's share if everyone reconsidered, before this
    /// iteration's choice: `Σ w (P_m − 1[the chain held is of mode m]) / Σ w` over the trips of
    /// the switch share (see the [module docs](self)). 0 in expectation at a stationary state;
    /// the three sum to 0; `None` when no trip faces a choice.
    pub mode_share_change_expected: PerMode,
    /// Of the trips choosing their mode that held a chain before the choice (weighted), the
    /// share whose chain after it is of another mode; `None` if no such trip.
    pub mode_change_share: Option<f64>,
    /// Of the trips choosing their mode that hold a chain after the choice (weighted), the share
    /// on a chain of each mode (summing to 1); `None` for every mode if no such trip.
    pub mode_split: PerMode,
    /// The summed weight of the trips that set out (every trip with a chain).
    pub departed: f64,
    /// The summed weight of the trips that reached their destination before the loading ended
    /// (a car trip once its car is parked and left: the car's arrival plus the egress time).
    pub arrived: f64,
    /// The summed weight of the trips still under way when the loading ended (parking and
    /// leaving the car included).
    pub unfinished: f64,
    /// [`Self::unfinished`] over [`Self::departed`] (0 when nobody set out).
    pub unfinished_share: f64,
    /// The summed weight of the trips with no chain at all (no route found, or no planner for
    /// their given mode); they do not travel.
    pub without_chain: f64,
    /// The relative duality gap within the choice sets: `(Σ w · paid − Σ w · least) / Σ w · least`
    /// over the trips that could change their choice and have a chain, where `paid` is the cost
    /// of the chain the trip took and `least` the least cost of any chain of its set (0 when every
    /// least cost is 0).
    pub duality_gap: f64,
    /// The deterministic part of the duality gap: `(Σ w · paid − Σ w · expected) / Σ w · least`,
    /// where `expected = Σ_k P_k · cost_k` over the set under the choice model at these costs:
    /// the duality gap less the part the model's own dispersion produces. Equal to the duality
    /// gap under a deterministic model.
    pub gap_deterministic: f64,
    /// The spread: the share of the trips that could change their choice and have a chain
    /// (weighted) on a chain of least cost in their set.
    pub on_cheapest_share: f64,
    /// The mean time from departure to arrival, in seconds, over the trips that arrived
    /// (weighted), as the loading moved them: door to door, a car trip's fixed times included.
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
