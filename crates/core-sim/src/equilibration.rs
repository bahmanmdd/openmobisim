//! Equilibration: repeating choice and loading until the pattern settles (design
//! §5, §11; S170).
//!
//! A run without it chooses every trip's route once, on free-flow costs, and loads
//! the network once. With it the run **iterates**: load, read the link times the
//! loading produced, let some travellers choose again on those times, load again.
//! An [`Equilibration`] says how many times, who chooses again, and when to stop.
//!
//! Built in:
//!
//! | name | what it does |
//! |---|---|
//! | `none` (the default, [`DEFAULT_STRATEGY`]) | One choice, one loading: a run is what it was before iteration existed |
//! | `msa` | The method of successive averages **in traveller form** (S86): at iteration `i` (from 1; iteration 0 is everyone's first choice) each traveller chooses again with probability `1/(i + 1)`, decided by a draw keyed on `(traveller, iteration)` from its own stream, so who moves does not depend on thread count or order; the others keep their route. In expectation the route flows are the average of the flows of every iteration so far, which is classical MSA with step `1/(i + 1)` |
//!
//! **Every iteration is reported** ([`IterationReport`]): the stability of the
//! pattern (who changed, how much the link times moved) and **the gap**: how much
//! more the chosen routes cost than the shortest congested route, the usual measure
//! of how far an assignment is from equilibrium. Below 5% is good and below 15%
//! acceptable for a stochastic dynamic assignment ([`gap_verdict`]); what each number
//! is: [`IterationReport`].
//!
//! # Extending it
//!
//! A new strategy is a type implementing [`Equilibration`], added to a
//! [`Registry`] to be selected by name. `replanning` (design §5) is reserved,
//! not built.

use std::collections::BTreeMap;
use std::fmt;

use openmobisim_core_types::rng::{DrawAddress, StreamRng};

/// The strategy used unless another is asked for.
pub const DEFAULT_STRATEGY: &str = "none";

/// A strategy's options: numbers by name. Unknown names are an error.
pub type Options = BTreeMap<String, f64>;

/// The most iterations a strategy may ask for.
pub const MAX_ITERATIONS: u32 = 1000;

/// What one iteration showed. `NaN` marks a number that was not measured (the
/// first iteration has nothing before it to have moved from, and a model with no
/// probabilities has no gap).
#[derive(Clone, Copy, Debug)]
pub struct IterationReport {
    /// The iteration: 0 is everyone's first choice, on free-flow costs.
    pub iteration: u32,
    /// The share of travellers (by weight) who chose again.
    pub reselected_share: f64,
    /// The share of travellers whose route changed.
    pub changed_share: f64,
    /// Total travel time of this iteration's loading (traveller-weight scaled).
    pub total_travel_time_s: f64,
    /// Trips that arrived inside the window.
    pub completed: u32,
    /// Trips still under way when the window ended.
    pub truncated: u32,
    /// How much the link times moved since the last loading, as a share of the
    /// last loading's, weighted by traffic. `NaN` at iteration 0.
    pub time_change: f64,
    /// **The gap** (S171): how much more travel time the chosen routes cost than the
    /// least-cost route of the same choice set at the times this loading produced,
    /// `Σ w (t_chosen − t_least) / Σ w t_least`, **over the travellers whose trip finished
    /// inside the window** (S178: the times of trips still under way at the end are lower
    /// bounds, not measurements; [`Self::incomplete_share`] says how many were left out). The
    /// usual relative gap of traffic assignment, against the shortest congested path among the
    /// alternatives. **A stochastic choice model does not reach 0 by design**: some travellers
    /// take slower routes with the probability the model gives them ([`Self::gap_expected`]);
    /// what says how far a run is from equilibrium is [`Self::gap_excess`].
    pub gap: f64,
    /// **What the choice model itself expects the gap to be** at these times (S178): the same
    /// sum with each traveller's *expected* travel time under the model's probabilities in
    /// place of the time of the route sampled. It is the gap a perfectly converged run has, by
    /// design (about 7% for a logit at −0.2 per minute). 0 for an all-or-nothing model; `NaN` for
    /// a model that gives no probabilities.
    pub gap_expected: f64,
    /// **The disequilibrium** (the headline, S178): `gap − gap_expected`, the part of the gap the
    /// choice model does not explain, over the trips that finished (it also holds the
    /// sampling noise of a finite population). Below [`GAP_GOOD`] is good, below
    /// [`GAP_ACCEPTABLE`] acceptable ([`gap_verdict`]); it is what `gap_tolerance` stops on.
    /// Equal to `gap` for an all-or-nothing model; `NaN` if the model gives no probabilities
    /// ([`Self::disequilibrium`] then falls back to `gap`).
    pub gap_excess: f64,
    /// The share of travellers (by weight) whose trip had not finished when the window ended, and
    /// who are therefore left out of the gaps.
    pub incomplete_share: f64,
    /// The same measure against the **whole network**: the least travel time over
    /// *any* route at the current times (found by a time-dependent search), for a
    /// keyed sample of the trips that finished, at the last iteration only; `NaN` elsewhere.
    /// Shows whether the choice set was missing a route that traffic has made worthwhile.
    pub gap_network: f64,
    /// The disequilibrium against the whole network (S178): `gap_network` less the part the choice
    /// model explains **inside the set** (what a converged run over the same set would still show
    /// by design), so a route the set was missing counts in full. `NaN` where `gap_network` is, or
    /// if the model gives no probabilities.
    pub gap_network_excess: f64,
    /// A consistency check of the stochastic choice model with itself: the share of
    /// travellers whose route differs from where the model's probabilities, at the
    /// current times, would put them (half the total variation between observed and
    /// expected route flows, by pair). Not a gap to the shortest path: a logit sends
    /// some travellers down slower routes at equilibrium, by design.
    pub gap_flow: f64,
    /// What `gap_flow` would be by chance alone: the same measure for a fresh sample
    /// drawn from the same probabilities.
    pub gap_flow_floor: f64,
    /// `gap_flow − gap_flow_floor`: the disequilibrium of the stochastic model left
    /// after noise.
    pub gap_flow_excess: f64,
    /// How many routes the route update (S176) added to the sets **after this loading**,
    /// for the next iteration to choose among; 0 without an update, at the last
    /// iteration (no choice follows it) and when no route beat the set. **`gap` is measured
    /// against the sets as grown**, the routes the travellers may now choose from.
    pub routes_added: u32,
    /// How many searches the route update made to find them.
    pub route_searches: u32,
}

/// A disequilibrium below this is good (the user, S171: "perfect"; S178: 5% on the disequilibrium).
pub const GAP_GOOD: f64 = 0.05;

/// A disequilibrium below this is acceptable for a stochastic dynamic assignment, which cannot
/// reach full equilibrium (the user, S171: "anything below 10–15% is acceptable").
pub const GAP_ACCEPTABLE: f64 = 0.15;

/// How close to equilibrium a gap says a run is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GapVerdict {
    /// Below [`GAP_GOOD`].
    Good,
    /// Below [`GAP_ACCEPTABLE`], not good.
    Acceptable,
    /// [`GAP_ACCEPTABLE`] or more.
    Poor,
}

impl GapVerdict {
    /// `"good"`, `"acceptable"` or `"poor"`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Good => "good",
            Self::Acceptable => "acceptable",
            Self::Poor => "poor",
        }
    }
}

/// What a relative `gap` says, or `None` if it was not measured (`NaN`).
#[must_use]
pub fn gap_verdict(gap: f64) -> Option<GapVerdict> {
    if gap.is_nan() {
        None
    } else if gap < GAP_GOOD {
        Some(GapVerdict::Good)
    } else if gap < GAP_ACCEPTABLE {
        Some(GapVerdict::Acceptable)
    } else {
        Some(GapVerdict::Poor)
    }
}

impl PartialEq for IterationReport {
    fn eq(&self, other: &Self) -> bool {
        let floats = |r: &Self| {
            [
                r.reselected_share,
                r.changed_share,
                r.total_travel_time_s,
                r.time_change,
                r.gap,
                r.gap_expected,
                r.gap_excess,
                r.incomplete_share,
                r.gap_network,
                r.gap_network_excess,
                r.gap_flow,
                r.gap_flow_floor,
                r.gap_flow_excess,
            ]
            .map(f64::to_bits)
        };
        self.iteration == other.iteration
            && self.completed == other.completed
            && self.truncated == other.truncated
            && self.routes_added == other.routes_added
            && self.route_searches == other.route_searches
            && floats(self) == floats(other)
    }
}

impl IterationReport {
    /// The number a run is judged by (S178): [`Self::gap_excess`], the disequilibrium, or the
    /// plain [`Self::gap`] where the model gives no probabilities and the two cannot be told apart.
    #[must_use]
    pub fn disequilibrium(&self) -> f64 {
        if self.gap_excess.is_nan() { self.gap } else { self.gap_excess }
    }

    /// A report with the run's numbers and nothing else measured.
    #[must_use]
    pub fn unmeasured(iteration: u32) -> Self {
        Self {
            iteration,
            reselected_share: f64::NAN,
            changed_share: f64::NAN,
            total_travel_time_s: f64::NAN,
            completed: 0,
            truncated: 0,
            time_change: f64::NAN,
            gap: f64::NAN,
            gap_expected: f64::NAN,
            gap_excess: f64::NAN,
            incomplete_share: f64::NAN,
            gap_network: f64::NAN,
            gap_network_excess: f64::NAN,
            gap_flow: f64::NAN,
            gap_flow_floor: f64::NAN,
            gap_flow_excess: f64::NAN,
            routes_added: 0,
            route_searches: 0,
        }
    }
}

/// Why a strategy could not be made.
#[derive(Clone, PartialEq, Debug)]
pub enum EquilibrationError {
    /// No strategy has this name.
    UnknownStrategy {
        /// The name asked for.
        name: String,
        /// The names that exist.
        known: Vec<String>,
    },
    /// The strategy has no such option.
    UnknownOption {
        /// The strategy.
        strategy: String,
        /// The option asked for.
        option: String,
        /// The options it has.
        known: Vec<&'static str>,
    },
    /// An option's value is not allowed.
    BadOption {
        /// The strategy.
        strategy: String,
        /// The option.
        option: String,
        /// What is wrong with it.
        reason: String,
    },
}

impl fmt::Display for EquilibrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownStrategy { name, known } => write!(
                f,
                "no equilibration strategy called {name:?}; the strategies are: {}",
                known.join(", ")
            ),
            Self::UnknownOption { strategy, option, known } if known.is_empty() => {
                write!(f, "equilibration {strategy:?} has no option {option:?}; it has no options")
            }
            Self::UnknownOption { strategy, option, known } => write!(
                f,
                "equilibration {strategy:?} has no option {option:?}; its options are: {}",
                known.join(", ")
            ),
            Self::BadOption { strategy, option, reason } => {
                write!(f, "equilibration {strategy:?}, option {option:?}: {reason}")
            }
        }
    }
}

impl std::error::Error for EquilibrationError {}

/// How a run repeats choice and loading. See the [module docs](self).
pub trait Equilibration: Send + Sync {
    /// The strategy's name, as it is selected.
    fn name(&self) -> &str;

    /// The name and every option with defaults filled in, canonical; part of the
    /// run's fingerprint.
    fn descriptor(&self) -> String;

    /// The most loadings the run makes (at least 1).
    fn max_iterations(&self) -> u32;

    /// The length in seconds of the time bins in which link times are recorded
    /// for costing the next iteration.
    fn cost_bin_seconds(&self) -> u32;

    /// Whether the strategy draws from the re-selection stream (the manifest
    /// records the stream as live exactly then).
    fn draws_reselection(&self) -> bool;

    /// Whether `traveller` chooses again at `iteration` (1 or more). Must be a
    /// pure function of its arguments and `rng`'s key, so that it does not depend
    /// on order or threads.
    fn reselects(&self, rng: &StreamRng, traveller: u32, iteration: u32) -> bool;

    /// How many trips to test against the whole network at the last iteration
    /// ([`IterationReport::gap_network`]); 0 means none.
    fn network_gap_sample(&self) -> u32 {
        0
    }

    /// How many of the first loadings the run makes with the **point-queue model** (level 2),
    /// which cannot gridlock, in place of the full one (S178). The first loading of a narrow
    /// route set is often in permanent gridlock (S177: 3 215 of 9 619 trips never finish) and every
    /// later iteration inherits its times. At most `max_iterations − 1` are used, so the last
    /// loading, the run's result, is always at the full level; no effect under a loading model that
    /// is not the link transmission model or is already at level 2.
    fn warmup_iterations(&self) -> u32 {
        0
    }

    /// Whether to stop before `max_iterations`, given every report so far.
    fn is_converged(&self, _reports: &[IterationReport]) -> bool {
        false
    }
}

/// One choice, one loading.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NoEquilibration;

impl Equilibration for NoEquilibration {
    fn name(&self) -> &str {
        "none"
    }
    fn descriptor(&self) -> String {
        "none".to_string()
    }
    fn max_iterations(&self) -> u32 {
        1
    }
    fn cost_bin_seconds(&self) -> u32 {
        300
    }
    fn draws_reselection(&self) -> bool {
        false
    }
    fn reselects(&self, _rng: &StreamRng, _traveller: u32, _iteration: u32) -> bool {
        false
    }
}

impl NoEquilibration {
    /// Make the strategy from its options (it has none).
    ///
    /// # Errors
    ///
    /// [`EquilibrationError::UnknownOption`] for any option.
    pub fn from_options(options: &Options) -> Result<Self, EquilibrationError> {
        check_known("none", options, &[])?;
        Ok(Self)
    }
}

/// The method of successive averages, in traveller form.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Msa {
    /// How many loadings to make at most (1 to [`MAX_ITERATIONS`]).
    pub iterations: u32,
    /// Stop once the **disequilibrium** ([`IterationReport::disequilibrium`]: the gap less what
    /// the choice model itself expects, S178), averaged over the last three iterations, is below
    /// this share; 0 means never stop early. 0.05 is good, 0.15 acceptable
    /// ([`GAP_GOOD`], [`GAP_ACCEPTABLE`]). Never while a
    /// route update is still adding routes (S176): the last update must have added none.
    pub gap_tolerance: f64,
    /// How many trips are tested against the whole network at the last iteration
    /// (0 to 100 000; 0 means none): the sample that
    /// [`IterationReport::gap_network`] is measured on.
    pub gap_sample: u32,
    /// The length in seconds of the time bins link times are recorded in (at
    /// least 1). Shorter follows a queue more closely and costs more memory.
    pub cost_bin_s: u32,
    /// How many of the first loadings use the point-queue model (0 to [`MAX_ITERATIONS`]; see
    /// [`Equilibration::warmup_iterations`]). 0 by default.
    pub warmup: u32,
}

impl Default for Msa {
    fn default() -> Self {
        Self { iterations: 10, gap_tolerance: 0.0, gap_sample: 300, cost_bin_s: 300, warmup: 0 }
    }
}

impl Msa {
    const OPTIONS: [&'static str; 5] =
        ["cost_bin_s", "gap_sample", "gap_tolerance", "iterations", "warmup"];

    /// Make the strategy from its options, defaults for those not given.
    ///
    /// # Errors
    ///
    /// [`EquilibrationError::UnknownOption`] for a name it does not have,
    /// [`EquilibrationError::BadOption`] for a value out of range.
    pub fn from_options(options: &Options) -> Result<Self, EquilibrationError> {
        check_known("msa", options, &Self::OPTIONS)?;
        let mut m = Self::default();
        let bad = |option: &str, reason: &str| EquilibrationError::BadOption {
            strategy: "msa".to_string(),
            option: option.to_string(),
            reason: reason.to_string(),
        };
        let whole = |option: &str, v: f64, lo: u32, hi: u32| -> Result<u32, EquilibrationError> {
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "checked to be a whole number within a small range"
            )]
            let n = v as u32;
            if v.is_finite() && v.fract() == 0.0 && v >= f64::from(lo) && v <= f64::from(hi) {
                Ok(n)
            } else {
                Err(bad(option, &format!("must be a whole number from {lo} to {hi}, got {v}")))
            }
        };
        for (option, &v) in options {
            match option.as_str() {
                "iterations" => m.iterations = whole(option, v, 1, MAX_ITERATIONS)?,
                "cost_bin_s" => m.cost_bin_s = whole(option, v, 1, 86_400)?,
                "gap_sample" => m.gap_sample = whole(option, v, 0, 100_000)?,
                "warmup" => m.warmup = whole(option, v, 0, MAX_ITERATIONS)?,
                _ => {
                    if !(v.is_finite() && (0.0..=1.0).contains(&v)) {
                        return Err(bad(option, &format!("must be from 0 to 1, got {v}")));
                    }
                    m.gap_tolerance = v;
                }
            }
        }
        Ok(m)
    }
}

impl Equilibration for Msa {
    fn name(&self) -> &str {
        "msa"
    }
    fn descriptor(&self) -> String {
        format!(
            "msa;cost_bin_s={};gap_sample={};gap_tolerance={};iterations={};warmup={}",
            self.cost_bin_s, self.gap_sample, self.gap_tolerance, self.iterations, self.warmup
        )
    }
    fn max_iterations(&self) -> u32 {
        self.iterations
    }
    fn cost_bin_seconds(&self) -> u32 {
        self.cost_bin_s
    }
    fn draws_reselection(&self) -> bool {
        self.iterations > 1
    }
    fn network_gap_sample(&self) -> u32 {
        self.gap_sample
    }
    fn warmup_iterations(&self) -> u32 {
        self.warmup
    }
    fn reselects(&self, rng: &StreamRng, traveller: u32, iteration: u32) -> bool {
        rng.unit(DrawAddress::from_pair(traveller, iteration)) < 1.0 / (f64::from(iteration) + 1.0)
    }
    fn is_converged(&self, reports: &[IterationReport]) -> bool {
        // The mean of the last three, not the last: with sampled choice and a queue that
        // reacts to it, the gap is noisy (one dip below the tolerance is not a settled
        // pattern), and the first iteration, which is everyone's choice on free flow,
        // never counts.
        if self.gap_tolerance <= 0.0 || reports.len() < 4 {
            return false;
        }
        // While the sets are still growing (S176) the pattern has not settled: the gap is
        // measured against the sets as grown, and a route just added has not been chosen yet.
        if reports.last().is_some_and(|r| r.routes_added > 0) {
            return false;
        }
        let last = &reports[reports.len() - 3..];
        last.iter().map(IterationReport::disequilibrium).sum::<f64>() / 3.0 < self.gap_tolerance
    }
}

fn check_known(
    strategy: &str,
    options: &Options,
    known: &[&'static str],
) -> Result<(), EquilibrationError> {
    match options.keys().find(|k| !known.contains(&k.as_str())) {
        Some(option) => Err(EquilibrationError::UnknownOption {
            strategy: strategy.to_string(),
            option: option.clone(),
            known: known.to_vec(),
        }),
        None => Ok(()),
    }
}

// --- the registry -----------------------------------------------------------------

/// Makes a strategy from its options.
pub type Factory = fn(&Options) -> Result<Box<dyn Equilibration>, EquilibrationError>;

/// Strategies by name; a researcher's is added with [`Registry::register`].
#[derive(Clone, Debug)]
pub struct Registry {
    strategies: Vec<(String, Factory)>,
}

impl Registry {
    /// The built-in strategies: `none` and `msa`.
    #[must_use]
    pub fn builtin() -> Self {
        let mut r = Self { strategies: Vec::new() };
        r.register("none", |o| Ok(Box::new(NoEquilibration::from_options(o)?)));
        r.register("msa", |o| Ok(Box::new(Msa::from_options(o)?)));
        r
    }

    /// Add a strategy under `name`, or replace one of that name.
    pub fn register(&mut self, name: &str, factory: Factory) {
        match self.strategies.iter_mut().find(|(n, _)| n == name) {
            Some(entry) => entry.1 = factory,
            None => self.strategies.push((name.to_string(), factory)),
        }
    }

    /// The names that can be selected, in registration order.
    #[must_use]
    pub fn names(&self) -> Vec<&str> {
        self.strategies.iter().map(|(n, _)| n.as_str()).collect()
    }

    /// Make the strategy called `name` from `options`.
    ///
    /// # Errors
    ///
    /// [`EquilibrationError::UnknownStrategy`] if there is none, or the
    /// strategy's own validation.
    pub fn create(
        &self,
        name: &str,
        options: &Options,
    ) -> Result<Box<dyn Equilibration>, EquilibrationError> {
        match self.strategies.iter().find(|(n, _)| n == name) {
            Some((_, factory)) => factory(options),
            None => Err(EquilibrationError::UnknownStrategy {
                name: name.to_string(),
                known: self.names().into_iter().map(str::to_string).collect(),
            }),
        }
    }
}

/// Make a built-in strategy by name; see [`Registry::builtin`].
///
/// # Errors
///
/// As [`Registry::create`].
pub fn strategy(
    name: &str,
    options: &Options,
) -> Result<Box<dyn Equilibration>, EquilibrationError> {
    Registry::builtin().create(name, options)
}
