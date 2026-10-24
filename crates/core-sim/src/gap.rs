//! The full gap (S293): how far every trip's choice is from the best alternative, at every
//! loading, over every trip, in the choice model's own currency.
//!
//! **Generalised minutes `g`** of an alternative: its systematic utility from the run's choice
//! model ([`ChoiceModel::utilities`]: the overlap term left out, since it has no value for an
//! alternative outside the set it was computed over) divided by the traveller's time
//! coefficient ([`ChoiceModel::time_coefficient`], the class's own if it sets one): minutes,
//! larger is worse, and for a car route that costs no money its travel time. A model that gives
//! no utilities (a choice model written in Python, `deterministic`), or a time coefficient that
//! is not below 0, measures in **door-to-door minutes** instead: the gaps' currency
//! ([`IterationReport::gap_currency`]) is then `"time"`, else `"utility"`.
//!
//! [`GapTrips`] holds each trip's numbers for one loading: the route gap's (every trip that drives
//! a car route: given the car, or choosing its mode and keeping a car alternative), filled by
//! [`crate::route_choice`] and [`crate::itinerary_choice`], and the mode gap's (every trip choosing
//! its mode), filled by the latter.
//!
//! **A trip still under way when the window ended is costed, not left out**: by car, what it paid
//! by the end plus the time of the links it had not yet left, read at each link's last recorded
//! time held ([`LinkTimes::link_seconds_held`]), the link it stood on counted in full, so a run
//! with many unfinished trips reads a worse gap than one that left them out. Where the queue is
//! clearing at the end this overestimates what is left; where it is still growing, the last
//! recorded time (of those who entered before and have left) is below what those behind them
//! will take, and it can underestimate. A trip by another mode keeps its planned cost.
//!
//! **Cost:** per loading, the route choice's batches built once more and each trip's car routes
//! costed in the model's utility; one bounded time-dependent search per car trip, in parallel; and
//! for each trip choosing its mode its whole choice set planned once more. Nothing is drawn.

use openmobisim_core_choice::{ChoiceBatch, ChoiceError, ChoiceModel};
use openmobisim_core_demand::{Travellers, Trips};
use openmobisim_core_types::ids::{EntityId, TripId};

use crate::equilibration::IterationReport;
use crate::link_times::LinkTimes;

/// How a run's gaps are costed: in generalised minutes, from the choice model's utilities, or
/// in door-to-door minutes. See the [module docs](self).
#[derive(Clone, Debug)]
pub(crate) struct Currency {
    /// Each traveller class's time coefficient, by class index; `None`: the gaps are in time.
    by_class: Option<Vec<f64>>,
}

impl Currency {
    /// The currency of a run choosing by `model` among alternatives carrying the attributes
    /// `names`: utility if the model gives utilities and every class of `travellers` has a time
    /// coefficient below 0, time otherwise.
    ///
    /// # Errors
    ///
    /// [`ChoiceError`] if the model fails on an empty batch of these attributes.
    pub(crate) fn new(
        model: &dyn ChoiceModel,
        travellers: &Travellers,
        names: &[&str],
    ) -> Result<Self, ChoiceError> {
        if model.utilities(&ChoiceBatch::new(0, names))?.is_none() {
            return Ok(Self { by_class: None });
        }
        let classes = travellers.class_external_ids();
        let coefficients: Vec<Option<f64>> =
            (0..classes.count()).map(|c| model.time_coefficient(classes.external(c))).collect();
        let by_class = coefficients
            .iter()
            .all(|b| b.is_some_and(|b| b.is_finite() && b < 0.0))
            .then(|| coefficients.iter().map(|b| b.unwrap_or(f64::NAN)).collect());
        Ok(Self { by_class })
    }

    /// `"utility"` or `"time"`: what the gaps are measured in
    /// ([`IterationReport::gap_currency`]).
    pub(crate) fn label(&self) -> &'static str {
        if self.by_class.is_some() { "utility" } else { "time" }
    }

    /// The generalised minutes of every alternative of `batch`, whose door-to-door seconds are
    /// `seconds`: each one's utility over its situation's class's time coefficient, or its
    /// seconds in minutes.
    ///
    /// # Errors
    ///
    /// [`ChoiceError`] if the model fails, or gives no utilities or the wrong number of them.
    pub(crate) fn minutes(
        &self,
        model: &dyn ChoiceModel,
        batch: &ChoiceBatch,
        seconds: &[f64],
    ) -> Result<Vec<f64>, ChoiceError> {
        let Some(by_class) = &self.by_class else {
            return Ok(seconds.iter().map(|s| s / 60.0).collect());
        };
        let utility = model.utilities(batch)?.ok_or_else(|| {
            ChoiceError::BadAnswer("the model gave utilities once and then none".to_string())
        })?;
        if utility.len() != batch.alternatives() {
            return Err(ChoiceError::BadAnswer(format!(
                "{} utilities for {} alternatives",
                utility.len(),
                batch.alternatives()
            )));
        }
        let mut g = vec![f64::NAN; utility.len()];
        for (s, &class) in batch.classes().iter().enumerate() {
            let b = by_class.get(class as usize).copied().unwrap_or(f64::NAN);
            for a in batch.range(s) {
                g[a] = utility[a] / b;
            }
        }
        Ok(g)
    }
}

/// No road link: a trip that was not on the roads when the window ended.
pub(crate) const NO_LINK: u32 = u32::MAX;

/// What the window's end left of a loading: who was still under way, and where.
pub(crate) struct Ended<'a> {
    /// Per trip: whether it was still under way when the window ended.
    pub unfinished: &'a [bool],
    /// Per trip: the road link a car still under way stood on at the end (driving, at its
    /// stop line, or waiting at its origin to enter it), or [`NO_LINK`].
    pub link_at_end: &'a [u32],
    /// The window's end, in seconds.
    pub window_end: f64,
}

impl Ended<'_> {
    /// What trip `t`, still under way at the end and driving `links` from `departure` (when its
    /// car entered the roads), paid by
    /// then (`window_end − departure`) and what it still had to pay: the links from the one it
    /// stood on to the last, that one counted in full, walked from the window's end at each
    /// link's last recorded time held ([`LinkTimes::link_seconds_held`]). All of `links` if it
    /// stood on none of them (it re-routed, or none was recorded). In seconds.
    pub(crate) fn car_seconds(
        &self,
        t: usize,
        links: &[u32],
        departure: f64,
        times: &LinkTimes,
    ) -> (f64, f64) {
        let paid = (self.window_end - departure).max(0.0);
        let from = links.iter().position(|&l| l == self.link_at_end[t]).unwrap_or(0);
        let mut at = self.window_end;
        for &l in &links[from..] {
            at += times.link_seconds_held(l, at);
        }
        (paid, at - self.window_end)
    }
}

/// Every trip's place in the full gap of one loading (S293; see the [module docs](crate::gap)),
/// by trip index. A number is `NaN` where the trip is not measured: a trip in neither gap, the
/// route gap's numbers for a trip that drove no car route, the mode gap's for a trip given its
/// mode, and an expectation without the model's probabilities. All in generalised minutes.
#[derive(Clone, Debug, Default)]
pub struct GapTrips {
    /// The alternative the trip took, at this loading's costs; a trip still under way at the
    /// window's end costed as what it paid and still had to pay (by car), or as planned.
    pub g_chosen: Vec<f64>,
    /// The best of the car routes it was offered: a trip that drove a car route (given the car,
    /// or choosing its mode and keeping a car alternative).
    pub g_best_set: Vec<f64>,
    /// The best over the whole network: the least of [`Self::g_best_set`] and the route a bounded
    /// time-dependent search finds for the trip (the fastest, costed as any route). The search
    /// minimises time, not `g`: where money weighs, a route cheaper in `g` can exist, so this is
    /// an upper bound on the network's best, and the network gap a lower bound.
    pub g_best_network: Vec<f64>,
    /// What the choice model expects of it among the car routes it was offered: `Σ p g`, with
    /// the probabilities renormalised over the car routes for a trip choosing its mode.
    pub g_expected_set: Vec<f64>,
    /// The best over its whole choice set, every mode: a trip choosing its mode.
    pub g_best_all: Vec<f64>,
    /// What the choice model expects of it over its whole choice set: `Σ p g`, every mode.
    pub g_expected_all: Vec<f64>,
    /// Of [`Self::g_chosen`], the minutes a trip still under way at the end had still to pay
    /// beyond what it paid by then; 0 for a trip that arrived.
    pub g_remaining: Vec<f64>,
    /// Whether the trip was still under way when the window ended.
    pub unfinished: Vec<bool>,
    /// The people it stands for: its traveller's weight.
    pub weight: Vec<f64>,
}

/// Equal when every entry is, comparing numbers by their bits, so a number not measured
/// (`NaN`) equals itself.
impl PartialEq for GapTrips {
    fn eq(&self, other: &Self) -> bool {
        let Self {
            g_chosen,
            g_best_set,
            g_best_network,
            g_expected_set,
            g_best_all,
            g_expected_all,
            g_remaining,
            unfinished,
            weight,
        } = self;
        let same = |a: &[f64], b: &[f64]| {
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.to_bits() == y.to_bits())
        };
        same(g_chosen, &other.g_chosen)
            && same(g_best_set, &other.g_best_set)
            && same(g_best_network, &other.g_best_network)
            && same(g_expected_set, &other.g_expected_set)
            && same(g_best_all, &other.g_best_all)
            && same(g_expected_all, &other.g_expected_all)
            && same(g_remaining, &other.g_remaining)
            && same(weight, &other.weight)
            && *unfinished == other.unfinished
    }
}

impl GapTrips {
    /// Nothing measured yet, for the trips of `trips`: their travellers' weights and whether
    /// each was `unfinished`.
    pub(crate) fn new(trips: &Trips, travellers: &Travellers, unfinished: &[bool]) -> Self {
        let n = trips.len() as usize;
        let nan = vec![f64::NAN; n];
        Self {
            g_chosen: nan.clone(),
            g_best_set: nan.clone(),
            g_best_network: nan.clone(),
            g_expected_set: nan.clone(),
            g_best_all: nan.clone(),
            g_expected_all: nan.clone(),
            g_remaining: nan,
            unfinished: unfinished.to_vec(),
            weight: (0..n)
                .map(|t| f64::from(travellers.weight(trips.traveller(TripId::from_index(t)))))
                .collect(),
        }
    }

    /// How many trips.
    #[must_use]
    pub fn len(&self) -> usize {
        self.g_chosen.len()
    }

    /// Whether there are no trips.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.g_chosen.is_empty()
    }

    /// The loading's full gaps, summed over the trips in trip order into `report`'s
    /// [`IterationReport::gap_route`] to [`IterationReport::unfinished_cost_share`]: the route gap
    /// over every trip with a [`Self::g_best_set`], the mode gap over every trip with a
    /// [`Self::g_best_all`], each weighted by its people. `NaN` where no trip is measured.
    pub(crate) fn report_into(&self, report: &mut IterationReport) {
        let ratio = |a: f64, b: f64| if b > 0.0 { a / b } else { f64::NAN };
        // The route gap.
        let (mut people, mut chosen, mut best, mut expected) = (0.0, 0.0, 0.0, 0.0);
        let (mut network, mut network_excess, mut excess, mut remaining) = (0.0, 0.0, 0.0, 0.0);
        for t in (0..self.len()).filter(|&t| !self.g_best_set[t].is_nan()) {
            let w = self.weight[t];
            people += w;
            chosen += w * self.g_chosen[t];
            best += w * self.g_best_set[t];
            expected += w * self.g_expected_set[t];
            network += w * self.g_best_network[t];
            network_excess += w
                * (self.g_chosen[t] - self.g_expected_set[t] + self.g_best_set[t]
                    - self.g_best_network[t]);
            excess += w * (self.g_chosen[t] - self.g_expected_set[t]);
            remaining += w * self.g_remaining[t];
        }
        report.gap_route = ratio(chosen - best, best);
        report.gap_route_expected = ratio(expected - best, best);
        report.gap_route_excess = report.gap_route - report.gap_route_expected;
        report.gap_route_network = ratio(chosen - network, network);
        report.gap_route_network_excess = ratio(network_excess, network);
        report.excess_min_route = ratio(excess, people);
        report.unfinished_cost_share = ratio(remaining, chosen);
        // The mode gap.
        let (mut people, mut chosen, mut best, mut expected, mut excess) =
            (0.0, 0.0, 0.0, 0.0, 0.0);
        for t in (0..self.len()).filter(|&t| !self.g_best_all[t].is_nan()) {
            let w = self.weight[t];
            people += w;
            chosen += w * self.g_chosen[t];
            best += w * self.g_best_all[t];
            expected += w * self.g_expected_all[t];
            excess += w * (self.g_chosen[t] - self.g_expected_all[t]);
        }
        report.gap_mode = ratio(chosen - best, best);
        report.gap_mode_expected = ratio(expected - best, best);
        report.gap_mode_excess = report.gap_mode - report.gap_mode_expected;
        report.excess_min_mode = ratio(excess, people);
    }
}
