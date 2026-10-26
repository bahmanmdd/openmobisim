//! Equilibration: repeating choice and loading until the pattern settles (design
//! §5, §11).
//!
//! A run without it (`free_flow`) makes only the free-flow loading (below). With it the run
//! **iterates** from there: load, read the link times the
//! loading produced, let some travellers choose again on those times, load again.
//! An [`Equilibration`] says how many times, who chooses again, and when to stop.
//!
//! Built in:
//!
//! | name | what it does |
//! |---|---|
//! | `free_flow` (the default, [`DEFAULT_STRATEGY`]) | The free-flow loading alone (below): one choice, built up in five groups by default, and one loading of everyone |
//! | `msa` | The method of successive averages **in traveller form**: at iteration `i` (from 1; iteration 0 is everyone's first choice) each traveller chooses again with probability `1/(i + 1)`, decided by a draw keyed on `(traveller, iteration)` from its own stream, so who moves does not depend on thread count or order; the others keep their route. In expectation the route flows are the average of the flows of every iteration so far, which is classical MSA with step `1/(i + 1)` |
//! | `frank_wolfe` | **Level 1 only** ([`FrankWolfe`]): the static assignment's Frank–Wolfe solver, the step that minimises the Beckmann objective; at level 1 `msa` is the static method of successive averages and `free_flow` one all-or-nothing assignment on free-flow times (`core_loading::level1`) |
//! | `sra` | Self-regulated averaging ([`Sra`]), `msa` with a step that regulates itself: at iteration `i` each traveller chooses again with probability `1/η_i`, where `η_1 = 2` and η grows by `eta_fell` for a loading whose disequilibrium fell and by `eta_rose` for one where it rose, so the step shrinks slowly while the run improves and fast when it does not; the share for loading `i` reacts to the loadings up to `i − 2`. The same draws as `msa`'s; with both options 1 it is `msa` |
//!
//! **The free-flow loading** (iteration 0) is where every run starts, and all
//! `free_flow` does. Two options shape it, for either strategy:
//! - `increments` ([`Equilibration::increments`]): the travellers are split, by a keyed draw,
//!   into this many groups of about equal size; the first group chooses on free-flow costs,
//!   then the network is loaded with the groups so far and the next group chooses on the times
//!   that loading produced, until every group has chosen and the whole demand is loaded
//!   (incremental assignment: Sheffi 1985, *Urban Transportation Networks*, §5.2). Each group
//!   sees the congestion of those before it, so routes spread before the first full loading.
//!   5 by default (C1: one default table for the core and the Python layer); 1 lets
//!   everyone choose at once on free-flow costs; at level 0 the run uses 1 whatever is set.
//! - `warmup` ([`Equilibration::warmup_iterations`]): with 1 or more (1 by default), the
//!   free-flow loading, its increments included, uses the point-queue model: queues delay
//!   but take no space, so nothing spills back and nothing locks.
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
//! [`Registry`] to be selected by name. `replanning` is reserved,
//! not built.

use std::collections::BTreeMap;
use std::fmt;

use openmobisim_core_demand::Mode;

use openmobisim_core_types::rng::{DrawAddress, StreamRng};

/// The strategy used unless another is asked for.
pub const DEFAULT_STRATEGY: &str = "free_flow";

/// A strategy's options: numbers by name. Unknown names are an error.
pub type Options = BTreeMap<String, f64>;

/// The most iterations a strategy may ask for.
pub const MAX_ITERATIONS: u32 = 1000;

/// The most groups the free-flow loading may be built up in ([`Equilibration::increments`]).
pub const MAX_INCREMENTS: u32 = 20;

/// The largest itinerary gap sample [`Msa`] accepts (more than any run's itinerary trips: every
/// trip planned in full).
pub const MAX_ITINERARY_GAP_SAMPLE: u32 = 100_000_000;

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
    /// The people those trips stand for: their travellers' weights, summed, so a
    /// mean trip is [`Self::total_travel_time_s`] over it, not over [`Self::completed`].
    pub completed_people: f64,
    /// Trips still under way when the window ended.
    pub truncated: u32,
    /// How much the link times moved since the last loading, as a share of the
    /// last loading's, weighted by traffic. `NaN` at iteration 0.
    pub time_change: f64,
    /// **The gap**: how much more travel time the chosen routes cost than the
    /// least-cost route of the same choice set at the times this loading produced,
    /// `Σ w (t_chosen − t_least) / Σ w t_least`, **over the travellers whose trip finished
    /// inside the window** (the times of trips still under way at the end are lower
    /// bounds, not measurements; [`Self::incomplete_share`] says how many were left out). The
    /// usual relative gap of traffic assignment, against the shortest congested path among the
    /// alternatives. **A stochastic choice model does not reach 0 by design**: some travellers
    /// take slower routes with the probability the model gives them ([`Self::gap_expected`]);
    /// what says how far a run is from equilibrium is [`Self::gap_excess`].
    pub gap: f64,
    /// **What the choice model itself expects the gap to be** at these times: the same
    /// sum with each traveller's *expected* travel time under the model's probabilities in
    /// place of the time of the route sampled. It is the gap a perfectly converged run has, by
    /// design (about 7% for a logit at −0.2 per minute). 0 for an all-or-nothing model; `NaN` for
    /// a model that gives no probabilities.
    pub gap_expected: f64,
    /// **The disequilibrium** (the headline): `gap − gap_expected`, the part of the gap the
    /// choice model does not explain, over the trips that finished (it also holds the
    /// sampling noise of a finite population). Below [`GAP_GOOD`] is good, below
    /// [`GAP_ACCEPTABLE`] acceptable ([`gap_verdict`]); it is what `gap_tolerance` stops on
    /// (but in a run by the route cost, [`Self::stop_generalised`]).
    /// Equal to `gap` for an all-or-nothing model; `NaN` if the model gives no probabilities
    /// ([`Self::disequilibrium`] then falls back to `gap`).
    pub gap_excess: f64,
    /// The share of travellers (by weight) whose trip had not finished when the window ended, and
    /// who are therefore left out of the gaps.
    pub incomplete_share: f64,
    /// The same measure against the **whole network**: the least travel time over
    /// *any* route at the current times (read off the run's shared search trees, one
    /// time-dependent search per origin and departure bin, [`crate::trees`]), for every trip
    /// that finished, at the last iteration only; `NaN` elsewhere. Shows whether the choice set
    /// was missing a route that traffic has made worthwhile.
    pub gap_network: f64,
    /// The disequilibrium against the whole network: `gap_network` less the part the choice
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
    /// How many routes the route update added to the sets **after this loading**,
    /// for the next iteration to choose among; 0 without an update, at the last
    /// iteration (no choice follows it) and when no route beat the set. **`gap` is measured
    /// against the sets as grown**, the routes the travellers may now choose from. Iteration
    /// 0's also counts the routes added while the free-flow loading was built up: every
    /// route found on the times of iteration `i`'s loadings is stamped `i + 1`.
    pub routes_added: u32,
    /// How many searches the route update made to find them.
    pub route_searches: u32,
    /// **The itinerary trips' gap**, by mode ([`Mode::index`]; `NaN` for a mode
    /// without itinerary trips): the same relative gap as [`Self::gap`] for the
    /// transit, park-and-ride and bike-and-ride trips, each alternative kept against
    /// the least door-to-door time in its choice set, at the costs this loading
    /// produced. With mode choice also the car, bike and walk trips that chose
    /// their mode, each against the best of the mode it took.
    pub itinerary_gap: [f64; Mode::COUNT],
    /// How many itinerary trips that kept their itinerary out through a parking were
    /// **re-costed** there rather than re-planned (outside the gap's sample, see
    /// [`Equilibration::itinerary_gap_sample`]); 0 at the last iteration, which plans every
    /// trip in full.
    pub itinerary_recosted: u32,
    /// How many times vehicles re-routed en route in this loading; 0 without rerouting.
    pub reroutes: u32,
    /// The itinerary disequilibrium: over every transit, park-and-ride,
    /// bike-and-ride and mode-choosing trip, the gap to the best alternative of the mode it kept
    /// less what the choice model itself expects (`Σ_m (paid − expected) / Σ_m least`); the plain
    /// gap if the model gives no probabilities; NaN without such trips. Part of
    /// [`Self::disequilibrium`].
    pub itinerary_gap_excess: f64,
    /// The floor of [`Self::mode_changed_share`]: the share whose mode would change
    /// by chance alone among those who chose again, from the model's probabilities; NaN where
    /// that share is.
    pub mode_changed_floor: f64,
    /// How many reroute offers this iteration's loadings made: each is one search for a
    /// faster route from where a stuck vehicle is, whether or not it re-routed. Iteration 0's
    /// counts the free-flow loading's groups' loadings too. 0 without rerouting.
    pub reroute_searches: u32,
    /// **The hub expectation mismatch**: per traveller who parked,
    /// the mean absolute difference between the parking time expected at the choice
    /// and the one paid, in seconds; `NaN` without parkings.
    pub hub_mismatch_s: f64,
    /// **The planned-against-realised difference of transit** (S299): over the trips with a
    /// transit part that ended in this loading (transit, park-and-ride, bike-and-ride; by
    /// traveller weight), the mean of the second the transit part ended (at the destination,
    /// or at the parking for a trip back) less the second it would have ended had every vehicle
    /// kept to the timetable the trip planned on, the trip followed the same way from the same
    /// moment at its first stop, in seconds (negative: earlier). 0 when every vehicle kept to
    /// that timetable (buses not on the roads); `NaN` when no trip rode.
    pub gap_transit_plan_s: f64,
    /// Its share of the itineraries' gap with transit ([`Self::gap_transit_plan_s`] over itself
    /// plus the mean seconds the transit, park-and-ride and bike-and-ride trips assessed paid
    /// above the best of their mode on the costs they planned with): the part of what those
    /// trips lost that their planning could not see. 0 when the difference is 0; `NaN` when it
    /// is, or nothing with transit was assessed.
    pub gap_transit_plan_share: f64,
    /// **Mode choice**: of the trips choosing their mode that had chosen before, the
    /// share (by traveller weight) whose mode changed on the way to this iteration's
    /// loading; `NaN` at the first, or without mode choice.
    pub mode_changed_share: f64,
    /// **The full route gap** (S293, [`crate::gap`]): over **every** trip that drove a car route
    /// in this loading (given the car, or choosing its mode and keeping a car alternative), how
    /// much more the routes taken cost than the best route each trip was offered, in the choice
    /// model's own currency ([`Self::gap_currency`]), `Σ w (g_chosen − g_best) / Σ w g_best`,
    /// both at the trip's departure on this loading's link times. **A trip still under way at
    /// the end counts**, as what it paid plus what it still had to pay ([`crate::gap`]).
    /// Measured after the last loading, or after every loading with
    /// [`Equilibration::full_gap_every_loading`]; `NaN` where nothing was (a loading not
    /// measured, a run of one loading). Read by nothing that stops or judges the run.
    pub gap_route: f64,
    /// What the choice model itself expects [`Self::gap_route`] to be: the same sum with each
    /// trip's expected `g` under the model's probabilities over the routes it was offered in
    /// place of the route's; `NaN` for a model that gives no probabilities.
    pub gap_route_expected: f64,
    /// `gap_route − gap_route_expected`: the deterministic part of the duality gap (the relative
    /// duality gap of the route choice less the part the logit's own dispersion produces).
    pub gap_route_excess: f64,
    /// [`Self::gap_route`] against the **whole network**: the best of the routes offered and
    /// the route a bounded time-dependent search finds for the trip (the fastest; where money
    /// weighs, a route cheaper in `g` may exist, so this gap is a lower bound). Every trip.
    pub gap_route_network: f64,
    /// Its disequilibrium: [`Self::gap_route_network`] less what the model expects **inside the
    /// set**, `Σ w (g_chosen − g_expected + g_best − g_best_network) / Σ w g_best_network`, so a
    /// route the set was missing counts in full.
    pub gap_route_network_excess: f64,
    /// **The full mode gap**: over every trip choosing its mode, how much more the alternative
    /// taken costs than the best of its whole choice set, every mode (as planned on this
    /// loading's costs), `Σ w (g_chosen − g_best_all) / Σ w g_best_all`; `NaN` without mode
    /// choice.
    pub gap_mode: f64,
    /// What the choice model itself expects [`Self::gap_mode`] to be, from its probabilities over
    /// the whole choice set.
    pub gap_mode_expected: f64,
    /// `gap_mode − gap_mode_expected`.
    pub gap_mode_excess: f64,
    /// The deterministic part of the route gap as a cost per traveller:
    /// `Σ w (g_chosen − g_expected) / Σ w`,
    /// in generalised minutes (the average excess cost less what the model expects).
    pub excess_min_route: f64,
    /// The same for the trips choosing their mode, over their whole choice sets.
    pub excess_min_mode: f64,
    /// Of what the route gap's trips paid (`Σ w g_chosen`), the share that is what trips still
    /// under way at the end had still to pay; 0 when every trip arrived.
    pub unfinished_cost_share: f64,
    /// What the full gaps are measured in: `"utility"` (generalised minutes from the choice
    /// model's utilities) or `"time"` (door-to-door minutes: a model without utilities, or a time
    /// coefficient not below 0).
    pub gap_currency: &'static str,
    /// Whether the stopping rule reads the generalised-minute gap (S299): set by a run whose
    /// route cost is not time alone (a class's utility weighs money, length or a user link
    /// value); [`Self::disequilibrium`] reads [`Self::gap_generalised_excess`] instead of
    /// [`Self::gap_excess`]. The time gaps are still reported. Not a column of the convergence
    /// table.
    pub stop_generalised: bool,
    /// The in-set gap's deterministic part in generalised minutes (S299): what the stopping rule
    /// reads with [`Self::stop_generalised`]: `(Σ w g_taken − Σ w g_expected) / Σ w g_least` over
    /// the finished trips whose route taken is offered, `g` an alternative's utility over its
    /// class's time coefficient, as [`Self::gap_excess`] in time. `NaN` where not measured: a
    /// run whose route cost is time alone (its stopping rule reads [`Self::gap_excess`], the
    /// counterpart in travel time), a model without probabilities, or a run of one loading. A
    /// column of the convergence table.
    pub gap_generalised_excess: f64,
}

/// A disequilibrium below this is good (5 % on the disequilibrium).
pub const GAP_GOOD: f64 = 0.05;

/// A disequilibrium below this is acceptable for a stochastic dynamic assignment, which cannot
/// reach full equilibrium ("anything below 10–15% is acceptable").
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
        // Every field by name, with no `..`: a field added to the report does not compile here
        // until it is compared too. Floats compare by their bits, so a repeated run is checked
        // to the bit and a NaN (a number not measured) equals itself.
        let Self {
            iteration,
            reselected_share,
            changed_share,
            total_travel_time_s,
            completed,
            completed_people,
            truncated,
            time_change,
            gap,
            gap_expected,
            gap_excess,
            incomplete_share,
            gap_network,
            gap_network_excess,
            gap_flow,
            gap_flow_floor,
            gap_flow_excess,
            routes_added,
            route_searches,
            itinerary_gap,
            itinerary_recosted,
            reroutes,
            itinerary_gap_excess,
            mode_changed_floor,
            reroute_searches,
            hub_mismatch_s,
            gap_transit_plan_s,
            gap_transit_plan_share,
            mode_changed_share,
            gap_route,
            gap_route_expected,
            gap_route_excess,
            gap_route_network,
            gap_route_network_excess,
            gap_mode,
            gap_mode_expected,
            gap_mode_excess,
            excess_min_route,
            excess_min_mode,
            unfinished_cost_share,
            gap_currency,
            stop_generalised,
            gap_generalised_excess,
        } = self;
        let floats = [
            (*reselected_share, other.reselected_share),
            (*changed_share, other.changed_share),
            (*total_travel_time_s, other.total_travel_time_s),
            (*completed_people, other.completed_people),
            (*time_change, other.time_change),
            (*gap, other.gap),
            (*gap_expected, other.gap_expected),
            (*gap_excess, other.gap_excess),
            (*incomplete_share, other.incomplete_share),
            (*gap_network, other.gap_network),
            (*gap_network_excess, other.gap_network_excess),
            (*gap_flow, other.gap_flow),
            (*gap_flow_floor, other.gap_flow_floor),
            (*gap_flow_excess, other.gap_flow_excess),
            (*itinerary_gap_excess, other.itinerary_gap_excess),
            (*mode_changed_floor, other.mode_changed_floor),
            (*hub_mismatch_s, other.hub_mismatch_s),
            (*gap_transit_plan_s, other.gap_transit_plan_s),
            (*gap_transit_plan_share, other.gap_transit_plan_share),
            (*mode_changed_share, other.mode_changed_share),
            (*gap_route, other.gap_route),
            (*gap_route_expected, other.gap_route_expected),
            (*gap_route_excess, other.gap_route_excess),
            (*gap_route_network, other.gap_route_network),
            (*gap_route_network_excess, other.gap_route_network_excess),
            (*gap_mode, other.gap_mode),
            (*gap_mode_expected, other.gap_mode_expected),
            (*gap_mode_excess, other.gap_mode_excess),
            (*excess_min_route, other.excess_min_route),
            (*excess_min_mode, other.excess_min_mode),
            (*unfinished_cost_share, other.unfinished_cost_share),
        ];
        *iteration == other.iteration
            && *completed == other.completed
            && *truncated == other.truncated
            && *routes_added == other.routes_added
            && *route_searches == other.route_searches
            && *itinerary_recosted == other.itinerary_recosted
            && *reroutes == other.reroutes
            && *reroute_searches == other.reroute_searches
            && *gap_currency == other.gap_currency
            && *stop_generalised == other.stop_generalised
            && gap_generalised_excess.to_bits() == other.gap_generalised_excess.to_bits()
            && floats.iter().all(|(a, b)| a.to_bits() == b.to_bits())
            && itinerary_gap.map(f64::to_bits) == other.itinerary_gap.map(f64::to_bits)
    }
}

impl IterationReport {
    /// The number a run is judged by: [`Self::gap_excess`], the disequilibrium, or the
    /// plain [`Self::gap`] where the model gives no probabilities and the two cannot be told apart;
    /// with [`Self::stop_generalised`], [`Self::gap_generalised_excess`] (the same in-set
    /// deterministic part of the gap, in generalised minutes) where it was measured.
    #[must_use]
    pub fn disequilibrium(&self) -> f64 {
        let routes = if self.stop_generalised && !self.gap_generalised_excess.is_nan() {
            self.gap_generalised_excess
        } else if self.gap_excess.is_nan() {
            self.gap
        } else {
            self.gap_excess
        };
        // The worse of the routes' and the itineraries' (I-al, S227): one meaning of converged.
        match (routes.is_nan(), self.itinerary_gap_excess.is_nan()) {
            (false, false) => routes.max(self.itinerary_gap_excess),
            (true, false) => self.itinerary_gap_excess,
            _ => routes,
        }
    }

    /// A report with the run's numbers and nothing else measured (its [`Self::gap_currency`]
    /// `"time"` until a run says what its gaps are measured in).
    #[must_use]
    pub fn unmeasured(iteration: u32) -> Self {
        Self {
            iteration,
            reselected_share: f64::NAN,
            changed_share: f64::NAN,
            total_travel_time_s: f64::NAN,
            completed: 0,
            completed_people: f64::NAN,
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
            itinerary_gap: [f64::NAN; Mode::COUNT],
            itinerary_recosted: 0,
            reroutes: 0,
            reroute_searches: 0,
            itinerary_gap_excess: f64::NAN,
            mode_changed_floor: f64::NAN,
            hub_mismatch_s: f64::NAN,
            gap_transit_plan_s: f64::NAN,
            gap_transit_plan_share: f64::NAN,
            mode_changed_share: f64::NAN,
            gap_route: f64::NAN,
            gap_route_expected: f64::NAN,
            gap_route_excess: f64::NAN,
            gap_route_network: f64::NAN,
            gap_route_network_excess: f64::NAN,
            gap_mode: f64::NAN,
            gap_mode_expected: f64::NAN,
            gap_mode_excess: f64::NAN,
            excess_min_route: f64::NAN,
            excess_min_mode: f64::NAN,
            unfinished_cost_share: f64::NAN,
            gap_currency: "time",
            stop_generalised: false,
            gap_generalised_excess: f64::NAN,
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

    /// The share of travellers who choose again at `iteration` (1 or more), given `reports`,
    /// those of the loadings already assessed when the travellers of `iteration` choose (the
    /// run passes every report made so far). The run asks once per iteration and gives the
    /// answer to [`Self::reselects`], so a share that reads the reports is still the same for
    /// every traveller, thread and order. `1/(iteration + 1)` unless a strategy says otherwise.
    fn reselect_share(&self, iteration: u32, _reports: &[IterationReport]) -> f64 {
        1.0 / (f64::from(iteration) + 1.0)
    }

    /// Whether `traveller` chooses again at `iteration` (1 or more), when the run has settled
    /// that `share` of the travellers do ([`Self::reselect_share`]). Must be a pure function of
    /// its arguments and `rng`'s key, so that it does not depend on order or threads.
    fn reselects(&self, rng: &StreamRng, traveller: u32, iteration: u32, share: f64) -> bool;

    /// Whether to test the trips against the whole network at the last iteration
    /// ([`IterationReport::gap_network`]): 0 means none, any other value every finished trip
    /// (S294: read off the shared search trees, the test costs no search, so no sample is
    /// taken).
    fn network_gap_sample(&self) -> u32 {
        0
    }

    /// Whether the full gap in the choice model's currency ([`IterationReport::gap_route`] and the
    /// rest of it) is measured after every loading, or after the last one alone (the default: it
    /// costs the choice sets' costing once more per loading, and the stopping rule reads the in-set
    /// disequilibrium the choice measures anyway; S294).
    fn full_gap_every_loading(&self) -> bool {
        false
    }

    /// How many itinerary trips, besides those choosing again, plan their whole choice set after
    /// a loading to measure the itinerary gap; the others that keep an itinerary out
    /// through a parking are re-costed at that parking instead of re-planned. A number at least the run's count of itinerary trips plans every
    /// trip in full (the default here: a strategy that does not say).
    fn itinerary_gap_sample(&self) -> u32 {
        u32::MAX
    }

    /// How many of the first loadings the run makes with the **point-queue model** (level 2),
    /// which cannot gridlock, in place of the full one. The first loading of a narrow
    /// route set is often in permanent gridlock (3 215 of 9 619 trips never finish) and every
    /// later iteration inherits its times. At most `max_iterations − 1` are used when the run
    /// iterates, so its last loading, the result, is always at the full level; a run of one
    /// loading is its free-flow loading, which uses the point queue if this is 1 or more.
    /// The free-flow loading's increments use it too. No effect under a loading model that is not
    /// the link transmission model or is already at level 2.
    fn warmup_iterations(&self) -> u32 {
        0
    }

    /// In how many groups the free-flow loading (iteration 0) is built up (1 to
    /// [`MAX_INCREMENTS`]). Each traveller is put in one group by a draw keyed on the traveller
    /// alone, from the re-selection stream; group 0 chooses on free-flow costs, and each group
    /// after it on the link times of a loading of the groups before it, after the route update
    /// (if any) has searched its pairs on those times. The last loading, of everyone, is
    /// iteration 0's. **Cost:** `increments − 1` loadings of part of the demand (`1/k, 2/k, …`
    /// of it), a route update and a choice for one group after each. 1, the default: no groups.
    /// No effect at free flow (level 0), where loading changes no time.
    fn increments(&self) -> u32 {
        1
    }

    /// Whether to stop before `max_iterations`, given every report so far.
    fn is_converged(&self, _reports: &[IterationReport]) -> bool {
        false
    }

    /// The gap below which the run stops: what the static assignment (flow level 1) stops its
    /// solver on, the relative gap of the flows (S294). 0, never early, unless a strategy says.
    fn gap_tolerance(&self) -> f64 {
        0.0
    }
}

/// One free-flow loading: everyone chooses once, in `increments` groups, and the network
/// is loaded once with everyone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FreeFlow {
    /// In how many groups the loading is built up (1 to [`MAX_INCREMENTS`]; see
    /// [`Equilibration::increments`]). 5 by default (the one default table; 1 lets
    /// everyone choose at once on free flow; at level 0 the run uses 1 whatever is set).
    pub increments: u32,
    /// 1 (or more) loads with the point-queue model, 0 at the run's own level (see
    /// [`Equilibration::warmup_iterations`]). 1 by default: the single loading of a
    /// `free_flow` run at levels 3–4 is then a point-queue loading, which cannot gridlock; 0
    /// loads it at the run's own level.
    pub warmup: u32,
}

impl Default for FreeFlow {
    fn default() -> Self {
        Self { increments: 5, warmup: 1 }
    }
}

impl Equilibration for FreeFlow {
    fn name(&self) -> &str {
        "free_flow"
    }
    fn descriptor(&self) -> String {
        // Every field, always (S280): a descriptor that named only what differs from the
        // default would change with the default, and the fingerprint with it.
        format!("free_flow;increments={};warmup={}", self.increments, self.warmup)
    }
    fn max_iterations(&self) -> u32 {
        1
    }
    fn cost_bin_seconds(&self) -> u32 {
        300
    }
    fn draws_reselection(&self) -> bool {
        self.increments > 1
    }
    fn reselects(&self, _rng: &StreamRng, _traveller: u32, _iteration: u32, _share: f64) -> bool {
        false
    }
    fn warmup_iterations(&self) -> u32 {
        self.warmup
    }
    fn increments(&self) -> u32 {
        self.increments
    }
}

impl FreeFlow {
    const OPTIONS: [&'static str; 2] = ["increments", "warmup"];

    /// Make the strategy from its options, defaults for those not given.
    ///
    /// # Errors
    ///
    /// [`EquilibrationError::UnknownOption`] for a name it does not have,
    /// [`EquilibrationError::BadOption`] for a value out of range.
    pub fn from_options(options: &Options) -> Result<Self, EquilibrationError> {
        check_known("free_flow", options, &Self::OPTIONS)?;
        let mut m = Self::default();
        for (option, &v) in options {
            match option.as_str() {
                "increments" => m.increments = whole("free_flow", option, v, 1, MAX_INCREMENTS)?,
                _ => m.warmup = whole("free_flow", option, v, 0, MAX_ITERATIONS)?,
            }
        }
        Ok(m)
    }
}

/// `v` as a whole number from `lo` to `hi`, or the error that says it is not.
fn whole(
    strategy: &str,
    option: &str,
    v: f64,
    lo: u32,
    hi: u32,
) -> Result<u32, EquilibrationError> {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "checked to be a whole number within a small range"
    )]
    let n = v as u32;
    if v.is_finite() && v.fract() == 0.0 && v >= f64::from(lo) && v <= f64::from(hi) {
        Ok(n)
    } else {
        Err(EquilibrationError::BadOption {
            strategy: strategy.to_string(),
            option: option.to_string(),
            reason: format!("must be a whole number from {lo} to {hi}, got {v}"),
        })
    }
}

/// The method of successive averages, in traveller form.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Msa {
    /// How many loadings to make at most (1 to [`MAX_ITERATIONS`]).
    pub iterations: u32,
    /// Stop once the **disequilibrium** ([`IterationReport::disequilibrium`]: the gap less what
    /// the choice model itself expects), averaged over the last three iterations, is below
    /// this share; 0 means never stop early. 0.05 by default. 0.05 is good, 0.15 acceptable
    /// ([`GAP_GOOD`], [`GAP_ACCEPTABLE`]). Never while a route update is still adding routes:
    /// the last update must have added routes for at most
    /// [`Self::route_growth_tolerance`] of the pairs it searched.
    pub gap_tolerance: f64,
    /// Whether the trips are tested against the whole network at the last iteration
    /// (0 to 100 000; 0 means none, any other value every finished trip):
    /// [`IterationReport::gap_network`]. The shared search trees cost no search, so no sample is
    /// taken.
    pub gap_sample: u32,
    /// How many itinerary trips, besides those choosing again, plan their whole choice set in an
    /// iteration to measure the itinerary gap (0 to [`MAX_ITINERARY_GAP_SAMPLE`]; see
    /// [`Equilibration::itinerary_gap_sample`]): a keyed sample of travellers, drawn again each
    /// iteration. The last assessment plans every trip in full, so the run's last itinerary gap is
    /// exact; an iteration's before it is an estimate. **By default every trip**
    /// ([`MAX_ITINERARY_GAP_SAMPLE`]): measured on Amsterdam, a sample of 2 000 saved 6%
    /// of a run with 10 000 park-and-ride and bike-and-ride trips, and its gaps before the last
    /// iteration read low (2.7–3.9% against 4.3–4.6%: a kept itinerary that has gone stale is
    /// set aside as having to choose again, rather than counted).
    pub itinerary_gap_sample: u32,
    /// The length in seconds of the time bins link times are recorded in (at
    /// least 1). Shorter follows a queue more closely and costs more memory.
    pub cost_bin_s: u32,
    /// How many of the first loadings use the point-queue model (0 to [`MAX_ITERATIONS`]; see
    /// [`Equilibration::warmup_iterations`]). 1 by default.
    pub warmup: u32,
    /// In how many groups the free-flow loading is built up (1 to [`MAX_INCREMENTS`]; see
    /// [`Equilibration::increments`]). 5 by default (1 at level 0 whatever is set).
    pub increments: u32,
    /// A run may stop at its [`Self::gap_tolerance`] only while the last route update added
    /// routes for at most this share of the pairs it searched (0 to 1). 1 by default:
    /// the gap alone decides; 0 stops no run while the update adds any route. A
    /// few routes found late barely move the gap.
    pub route_growth_tolerance: f64,
    /// Whether the full gap in the choice model's currency is measured after every loading (1)
    /// or after the last loading alone (0, the default; see
    /// [`Equilibration::full_gap_every_loading`]).
    pub gap_every: u32,
}

impl Default for Msa {
    fn default() -> Self {
        Self {
            iterations: 10,
            gap_tolerance: 0.05,
            gap_sample: 300,
            itinerary_gap_sample: MAX_ITINERARY_GAP_SAMPLE,
            cost_bin_s: 300,
            warmup: 1,
            increments: 5,
            route_growth_tolerance: 1.0,
            gap_every: 0,
        }
    }
}

impl Msa {
    const OPTIONS: [&'static str; 9] = [
        "cost_bin_s",
        "gap_every",
        "gap_sample",
        "gap_tolerance",
        "increments",
        "itinerary_gap_sample",
        "iterations",
        "route_growth_tolerance",
        "warmup",
    ];

    /// Make the strategy from its options, defaults for those not given.
    ///
    /// # Errors
    ///
    /// [`EquilibrationError::UnknownOption`] for a name it does not have,
    /// [`EquilibrationError::BadOption`] for a value out of range.
    pub fn from_options(options: &Options) -> Result<Self, EquilibrationError> {
        check_known("msa", options, &Self::OPTIONS)?;
        Self::read("msa", options)
    }

    /// `msa`'s options among `options`, defaults for those not given, the others passed over;
    /// errors name `strategy`, the strategy they were given to.
    fn read(strategy: &str, options: &Options) -> Result<Self, EquilibrationError> {
        let mut m = Self::default();
        let bad = |option: &str, reason: &str| EquilibrationError::BadOption {
            strategy: strategy.to_string(),
            option: option.to_string(),
            reason: reason.to_string(),
        };
        let whole = |option: &str, v: f64, lo: u32, hi: u32| whole(strategy, option, v, lo, hi);
        for (option, &v) in options {
            match option.as_str() {
                "iterations" => m.iterations = whole(option, v, 1, MAX_ITERATIONS)?,
                "cost_bin_s" => m.cost_bin_s = whole(option, v, 1, 86_400)?,
                "gap_sample" => m.gap_sample = whole(option, v, 0, 100_000)?,
                "gap_every" => m.gap_every = whole(option, v, 0, 1)?,
                "itinerary_gap_sample" => {
                    m.itinerary_gap_sample = whole(option, v, 0, MAX_ITINERARY_GAP_SAMPLE)?;
                }
                "warmup" => m.warmup = whole(option, v, 0, MAX_ITERATIONS)?,
                "increments" => m.increments = whole(option, v, 1, MAX_INCREMENTS)?,
                "route_growth_tolerance" => {
                    if !(v.is_finite() && (0.0..=1.0).contains(&v)) {
                        return Err(bad(option, &format!("must be from 0 to 1, got {v}")));
                    }
                    m.route_growth_tolerance = v;
                }
                "gap_tolerance" => {
                    if !(v.is_finite() && (0.0..=1.0).contains(&v)) {
                        return Err(bad(option, &format!("must be from 0 to 1, got {v}")));
                    }
                    m.gap_tolerance = v;
                }
                _ => {}
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
            "msa;cost_bin_s={};gap_every={};gap_sample={};gap_tolerance={};increments={};\
             itinerary_gap_sample={};iterations={};route_growth_tolerance={};warmup={}",
            self.cost_bin_s,
            self.gap_every,
            self.gap_sample,
            self.gap_tolerance,
            self.increments,
            self.itinerary_gap_sample,
            self.iterations,
            self.route_growth_tolerance,
            self.warmup
        )
    }
    fn max_iterations(&self) -> u32 {
        self.iterations
    }
    fn cost_bin_seconds(&self) -> u32 {
        self.cost_bin_s
    }
    fn draws_reselection(&self) -> bool {
        self.iterations > 1 || self.increments > 1
    }
    fn network_gap_sample(&self) -> u32 {
        self.gap_sample
    }
    fn full_gap_every_loading(&self) -> bool {
        self.gap_every == 1
    }
    fn itinerary_gap_sample(&self) -> u32 {
        self.itinerary_gap_sample
    }
    fn warmup_iterations(&self) -> u32 {
        self.warmup
    }
    fn increments(&self) -> u32 {
        self.increments
    }
    fn reselects(&self, rng: &StreamRng, traveller: u32, iteration: u32, share: f64) -> bool {
        rng.unit(DrawAddress::from_pair(traveller, iteration)) < share
    }
    fn gap_tolerance(&self) -> f64 {
        self.gap_tolerance
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
        if reports.last().is_some_and(|r| {
            f64::from(r.routes_added) > self.route_growth_tolerance * f64::from(r.route_searches)
        }) {
            return false;
        }
        let last = &reports[reports.len() - 3..];
        last.iter().map(IterationReport::disequilibrium).sum::<f64>() / 3.0 < self.gap_tolerance
    }
}

/// **Self-regulated averaging** in traveller form (Liu, He and He 2009, *Method of successive
/// weighted averages (MSWA) and self-regulated averaging schemes for solving stochastic user
/// equilibrium problem*, *Networks and Spatial Economics* 9, 485–503,
/// <https://doi.org/10.1007/s11067-009-9108-9>; in a dynamic model, Ma and He 2024; as a share
/// of travellers, Taale and Pel 2015): [`Msa`], every option of it with its meaning and default,
/// with the share of travellers who choose again at iteration `k` set to `1/η_k` instead of
/// `1/(k + 1)`, by the same keyed draws. `η_1 = 2`; after it, η grows once per iteration, by
/// [`Self::eta_fell`] for a loading whose disequilibrium ([`IterationReport::disequilibrium`])
/// fell below the loading's before it, and by [`Self::eta_rose`] for one where it rose or stayed
/// (or either is not a number): the step shrinks slowly while the run improves, fast when it does
/// not. With both at 1, `η_k = k + 1`: exactly `msa`.
///
/// **Which loadings decide** ([`Self::eta`]): the run measures a loading's disequilibrium in the
/// same pass that draws who chooses again after it, so when the travellers of loading `k` choose,
/// the last loading assessed is `k − 2`: **the share for loading `k` reacts to the disequilibrium
/// measured up to loading `k − 2`**, one loading later than in Liu, He and He. The growth to `η_k`
/// is decided by loading `k − 2` against `k − 3`, and the growth to `η_2`, with one report only,
/// counts as a rise. Cost: one comparison per earlier report, once per iteration.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sra {
    /// The loadings, the stopping rule, the samples, the cost bins, the warm-up and the
    /// increments, as [`Msa`]'s, with its defaults.
    pub msa: Msa,
    /// γ: how much η grows for a loading whose disequilibrium fell (0 to 1).
    ///
    /// *Uncalibrated: 0.2, after Ma and He (2024), who used 0.2; Liu, He and He (2009) tried 0.01
    /// to 0.5.*
    pub eta_fell: f64,
    /// Γ: how much η grows for a loading whose disequilibrium rose or stayed (1 to 10).
    ///
    /// *Uncalibrated: 1.5, within the 1.5 to 2 Liu, He and He (2009) tried; Ma and He (2024)
    /// used 1.1.*
    pub eta_rose: f64,
}

impl Default for Sra {
    fn default() -> Self {
        Self { msa: Msa::default(), eta_fell: 0.2, eta_rose: 1.5 }
    }
}

impl Sra {
    const OPTIONS: [&'static str; 11] = [
        "cost_bin_s",
        "eta_fell",
        "eta_rose",
        "gap_every",
        "gap_sample",
        "gap_tolerance",
        "increments",
        "itinerary_gap_sample",
        "iterations",
        "route_growth_tolerance",
        "warmup",
    ];

    /// Make the strategy from its options, defaults for those not given.
    ///
    /// # Errors
    ///
    /// [`EquilibrationError::UnknownOption`] for a name it does not have,
    /// [`EquilibrationError::BadOption`] for a value out of range.
    pub fn from_options(options: &Options) -> Result<Self, EquilibrationError> {
        check_known("sra", options, &Self::OPTIONS)?;
        let mut s = Self { msa: Msa::read("sra", options)?, ..Self::default() };
        let within = |option: &str, v: f64, lo: f64, hi: f64| {
            if v.is_finite() && (lo..=hi).contains(&v) {
                Ok(v)
            } else {
                Err(EquilibrationError::BadOption {
                    strategy: "sra".to_string(),
                    option: option.to_string(),
                    reason: format!("must be from {lo} to {hi}, got {v}"),
                })
            }
        };
        for (option, &v) in options {
            match option.as_str() {
                "eta_fell" => s.eta_fell = within(option, v, 0.0, 1.0)?,
                "eta_rose" => s.eta_rose = within(option, v, 1.0, 10.0)?,
                _ => {}
            }
        }
        Ok(s)
    }

    /// `η_k` at `iteration` `k` (1 or more; 1 at 0) given `reports`: 2, plus one growth for
    /// each of the iterations 2 to `k`. The growths are read from the comparisons of
    /// consecutive reports, the last comparison deciding the last growth: [`Self::eta_fell`]
    /// where the later report's disequilibrium is below the earlier's, [`Self::eta_rose`] where
    /// it is not or either is not a number, and [`Self::eta_rose`] for each growth with no
    /// comparison left. Given every report up to loading `k − 1`, the growth to `η_k` reads
    /// `k − 1` against `k − 2`; the run gives those up to `k − 2` (see the type's docs).
    #[must_use]
    pub fn eta(&self, iteration: u32, reports: &[IterationReport]) -> f64 {
        if iteration == 0 {
            return 1.0;
        }
        let growths = (iteration - 1) as usize;
        let comparisons: Vec<f64> = reports
            .windows(2)
            .map(|w| {
                if w[1].disequilibrium() < w[0].disequilibrium() {
                    self.eta_fell
                } else {
                    self.eta_rose
                }
            })
            .collect();
        let read = comparisons.len().min(growths);
        let mut eta = 2.0;
        for _ in read..growths {
            eta += self.eta_rose;
        }
        for &g in &comparisons[comparisons.len() - read..] {
            eta += g;
        }
        eta
    }
}

impl Equilibration for Sra {
    fn name(&self) -> &str {
        "sra"
    }
    fn descriptor(&self) -> String {
        // `msa`'s options in its order, with the two of its own in their alphabetical places.
        let m = &self.msa;
        format!(
            "sra;cost_bin_s={};eta_fell={};eta_rose={};gap_every={};gap_sample={};\
             gap_tolerance={};increments={};itinerary_gap_sample={};iterations={};\
             route_growth_tolerance={};warmup={}",
            m.cost_bin_s,
            self.eta_fell,
            self.eta_rose,
            m.gap_every,
            m.gap_sample,
            m.gap_tolerance,
            m.increments,
            m.itinerary_gap_sample,
            m.iterations,
            m.route_growth_tolerance,
            m.warmup
        )
    }
    fn max_iterations(&self) -> u32 {
        self.msa.max_iterations()
    }
    fn cost_bin_seconds(&self) -> u32 {
        self.msa.cost_bin_seconds()
    }
    fn draws_reselection(&self) -> bool {
        self.msa.draws_reselection()
    }
    fn network_gap_sample(&self) -> u32 {
        self.msa.network_gap_sample()
    }
    fn full_gap_every_loading(&self) -> bool {
        self.msa.full_gap_every_loading()
    }
    fn itinerary_gap_sample(&self) -> u32 {
        self.msa.itinerary_gap_sample()
    }
    fn warmup_iterations(&self) -> u32 {
        self.msa.warmup_iterations()
    }
    fn increments(&self) -> u32 {
        self.msa.increments()
    }
    fn reselect_share(&self, iteration: u32, reports: &[IterationReport]) -> f64 {
        1.0 / self.eta(iteration, reports)
    }
    fn reselects(&self, rng: &StreamRng, traveller: u32, iteration: u32, share: f64) -> bool {
        self.msa.reselects(rng, traveller, iteration, share)
    }
    fn is_converged(&self, reports: &[IterationReport]) -> bool {
        self.msa.is_converged(reports)
    }
    fn gap_tolerance(&self) -> f64 {
        self.msa.gap_tolerance
    }
}

/// The most iterations [`FrankWolfe`] may make: a static assignment's iteration is one
/// shortest-path tree per origin, far cheaper than a loading.
pub const MAX_STATIC_ITERATIONS: u32 = 100_000;

/// **Frank–Wolfe**, the static assignment's solver at flow level 1 (S294; Frank and Wolfe 1956,
/// *An algorithm for quadratic programming*, *Naval Research Logistics Quarterly* 3; LeBlanc,
/// Morlok and Pierskalla 1975, *Transportation Research* 9): from the all-or-nothing assignment
/// on free-flow times, each iteration moves the link flows towards the all-or-nothing flows on
/// the current times by the step that minimises the Beckmann objective along that direction.
/// It makes no loading of its own: at level 1 the run makes one loading, and the solver iterates
/// inside it (`core_loading::level1`). **Refused at every other level** (the pipeline says so),
/// where there is nothing for it to solve.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrankWolfe {
    /// The most iterations (1 to [`MAX_STATIC_ITERATIONS`]); 1000 by default. *An assumption:
    /// about what Sioux Falls at TNTP's demand takes to reach the default tolerance (the Python
    /// tests run it).*
    pub iterations: u32,
    /// Stop once the relative gap of the flows, `(Σ x t − Σ d t_min) / Σ d t_min`, is below this
    /// (0 to 1; 0 never stops early); 1e-4 by default. *An assumption: a common target of
    /// static assignment (Boyce, Ralevic-Dekic and Bar-Gera 2004, *Journal of Transportation
    /// Engineering* 130(1), argue for 1e-4 or below).*
    pub gap_tolerance: f64,
}

impl Default for FrankWolfe {
    fn default() -> Self {
        Self { iterations: 1000, gap_tolerance: 1e-4 }
    }
}

impl FrankWolfe {
    const OPTIONS: [&'static str; 2] = ["gap_tolerance", "iterations"];

    /// Make the strategy from its options, defaults for those not given.
    ///
    /// # Errors
    ///
    /// [`EquilibrationError::UnknownOption`] for a name it does not have,
    /// [`EquilibrationError::BadOption`] for a value out of range.
    pub fn from_options(options: &Options) -> Result<Self, EquilibrationError> {
        check_known("frank_wolfe", options, &Self::OPTIONS)?;
        let mut m = Self::default();
        for (option, &v) in options {
            match option.as_str() {
                "iterations" => {
                    m.iterations = whole("frank_wolfe", option, v, 1, MAX_STATIC_ITERATIONS)?;
                }
                _ => {
                    if !(v.is_finite() && (0.0..=1.0).contains(&v)) {
                        return Err(EquilibrationError::BadOption {
                            strategy: "frank_wolfe".to_string(),
                            option: option.clone(),
                            reason: format!("must be from 0 to 1, got {v}"),
                        });
                    }
                    m.gap_tolerance = v;
                }
            }
        }
        Ok(m)
    }
}

impl Equilibration for FrankWolfe {
    fn name(&self) -> &str {
        "frank_wolfe"
    }
    fn descriptor(&self) -> String {
        format!("frank_wolfe;gap_tolerance={};iterations={}", self.gap_tolerance, self.iterations)
    }
    fn max_iterations(&self) -> u32 {
        self.iterations
    }
    fn cost_bin_seconds(&self) -> u32 {
        300
    }
    fn draws_reselection(&self) -> bool {
        false
    }
    fn reselects(&self, _rng: &StreamRng, _traveller: u32, _iteration: u32, _share: f64) -> bool {
        false
    }
    fn gap_tolerance(&self) -> f64 {
        self.gap_tolerance
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
    /// The built-in strategies: `free_flow`, `msa`, `sra` and `frank_wolfe` (level 1 only).
    #[must_use]
    pub fn builtin() -> Self {
        let mut r = Self { strategies: Vec::new() };
        r.register("free_flow", |o| Ok(Box::new(FreeFlow::from_options(o)?)));
        r.register("msa", |o| Ok(Box::new(Msa::from_options(o)?)));
        r.register("sra", |o| Ok(Box::new(Sra::from_options(o)?)));
        r.register("frank_wolfe", |o| Ok(Box::new(FrankWolfe::from_options(o)?)));
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
