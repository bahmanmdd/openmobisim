//! The full gap (S293): how far every trip's choice is from the best alternative, after the
//! last loading (or after every one), over every trip, in the choice model's own currency.
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

/// The sums of the **in-set route gap measured after every loading** (S302): over car trips
/// that finished, by traveller weight, what the route taken costs (`paid`), the least of the
/// trip's car routes (`least`) and what the choice model expects among them (`expected`), in
/// seconds of driving; and the same in generalised minutes (`g_*`) where asked. The route
/// choice fills one for the trips given the car, the itinerary choice one for the trips
/// choosing their mode that drove, and the run's `gap`, `gap_expected`, `gap_excess` and
/// `gap_generalised_excess` are made from their sum.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RouteGapSums {
    /// Σ w of the trips summed.
    pub weight: f64,
    pub paid: f64,
    pub least: f64,
    pub expected: f64,
    /// Whether the model gave probabilities for every trip summed (else `expected` is not).
    pub measured: bool,
    pub g_paid: f64,
    pub g_least: f64,
    pub g_expected: f64,
    /// Whether the generalised sums were asked for and measured for every trip summed.
    pub g_measured: bool,
}

impl Default for RouteGapSums {
    fn default() -> Self {
        Self::new(false)
    }
}

impl RouteGapSums {
    /// Nothing summed yet; the generalised sums measured if `generalised`.
    pub(crate) fn new(generalised: bool) -> Self {
        Self {
            weight: 0.0,
            paid: 0.0,
            least: 0.0,
            expected: 0.0,
            measured: true,
            g_paid: 0.0,
            g_least: 0.0,
            g_expected: 0.0,
            g_measured: generalised,
        }
    }

    /// The two together.
    #[must_use]
    pub(crate) fn plus(&self, other: &Self) -> Self {
        Self {
            weight: self.weight + other.weight,
            paid: self.paid + other.paid,
            least: self.least + other.least,
            expected: self.expected + other.expected,
            measured: self.measured && other.measured,
            g_paid: self.g_paid + other.g_paid,
            g_least: self.g_least + other.g_least,
            g_expected: self.g_expected + other.g_expected,
            g_measured: self.g_measured && other.g_measured,
        }
    }

    /// `(gap, gap_expected, gap_excess)`: `(paid − least) / least`, `(expected − least) /
    /// least` and their difference; `NaN` where not measured.
    pub(crate) fn gaps(&self) -> (f64, f64, f64) {
        if self.least > 0.0 {
            let gap = (self.paid - self.least) / self.least;
            if self.measured {
                let expected_gap = (self.expected - self.least) / self.least;
                (gap, expected_gap, gap - expected_gap)
            } else {
                (gap, f64::NAN, f64::NAN)
            }
        } else {
            (f64::NAN, f64::NAN, f64::NAN)
        }
    }

    /// `(g_paid − g_expected) / g_least`, `NaN` where not measured.
    pub(crate) fn generalised_excess(&self) -> f64 {
        if self.g_measured && self.g_least > 0.0 {
            (self.g_paid - self.g_expected) / self.g_least
        } else {
            f64::NAN
        }
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
    /// The choice model's probability of the car route the trip took over the highest
    /// probability among the car routes it was offered (`p / p*`, in (0, 1]; for a trip choosing
    /// its mode the same over its car alternatives, which renormalising does not change). `NaN`
    /// where the route taken was not offered, the trip was still under way at the window's end,
    /// or the model gives no probabilities. What [`IterationReport::spread_route`] sums.
    pub p_ratio_set: Vec<f64>,
    /// The same over the whole choice set, every mode, for a trip choosing its mode: the
    /// probability of the alternative it took over the highest. What
    /// [`IterationReport::spread_mode`] sums.
    pub p_ratio_all: Vec<f64>,
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
            p_ratio_set,
            p_ratio_all,
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
            && same(p_ratio_set, &other.p_ratio_set)
            && same(p_ratio_all, &other.p_ratio_all)
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
            p_ratio_set: nan.clone(),
            p_ratio_all: nan.clone(),
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
    /// [`Self::g_best_all`], each weighted by its people; and the spreads of the choices,
    /// [`IterationReport::spread_route`] over every trip with a finite [`Self::p_ratio_set`] and
    /// [`IterationReport::spread_mode`] over every trip with a finite [`Self::p_ratio_all`].
    /// `NaN` where no trip is measured.
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
        // The spreads: `Σ w (1 − p / p*) / Σ w`.
        let spread = |p_ratio: &[f64]| {
            let (mut people, mut spread) = (0.0, 0.0);
            for (&r, &w) in p_ratio.iter().zip(&self.weight).filter(|(r, _)| r.is_finite()) {
                people += w;
                spread += w * (1.0 - r);
            }
            ratio(spread, people)
        };
        report.spread_route = spread(&self.p_ratio_set);
        report.spread_mode = spread(&self.p_ratio_all);
    }
}

/// The probability `p[taken]` over the highest of `p[range]` (`p / p*`): `NaN` if `taken` is
/// `None` or the highest is not above 0.
pub(crate) fn p_ratio(p: &[f64], range: impl Iterator<Item = usize>, taken: Option<usize>) -> f64 {
    let Some(taken) = taken else { return f64::NAN };
    let highest = range.map(|a| p[a]).fold(f64::NEG_INFINITY, f64::max);
    if highest > 0.0 { p[taken] / highest } else { f64::NAN }
}

#[cfg(test)]
mod tests {
    use super::{GapTrips, RouteGapSums, p_ratio};
    use crate::equilibration::IterationReport;

    #[test]
    fn the_spread_is_one_less_the_probability_ratio_by_people() {
        // Three trips with ratios 1, 0.5 and not measured, standing for 1, 2 and 5 people:
        // (1 × 0 + 2 × 0.5) / (1 + 2) = 1/3; the trip not measured is out of both sums.
        let nan = f64::NAN;
        let gap = GapTrips {
            g_chosen: vec![nan; 3],
            g_best_set: vec![nan; 3],
            g_best_network: vec![nan; 3],
            g_expected_set: vec![nan; 3],
            g_best_all: vec![nan; 3],
            g_expected_all: vec![nan; 3],
            p_ratio_set: vec![1.0, 0.5, nan],
            p_ratio_all: vec![nan, 0.25, 1.0],
            g_remaining: vec![nan; 3],
            unfinished: vec![false; 3],
            weight: vec![1.0, 2.0, 5.0],
        };
        let mut report = IterationReport::unmeasured(1);
        gap.report_into(&mut report);
        assert!((report.spread_route - 1.0 / 3.0).abs() < 1e-15, "{}", report.spread_route);
        // (2 × 0.75 + 5 × 0) / 7.
        assert!((report.spread_mode - 1.5 / 7.0).abs() < 1e-15, "{}", report.spread_mode);
        // No trip measured: not measured.
        let none = GapTrips { p_ratio_set: vec![nan; 3], ..gap };
        none.report_into(&mut report);
        assert!(report.spread_route.is_nan());
    }

    #[test]
    fn the_probability_ratio_is_over_the_highest_of_the_range() {
        let p = [0.1, 0.6, 0.3, 0.9];
        assert!((p_ratio(&p, 0..3, Some(2)) - 0.5).abs() < 1e-15);
        assert!((p_ratio(&p, 0..3, Some(1)) - 1.0).abs() < 1e-15);
        assert!(p_ratio(&p, 0..3, None).is_nan(), "the taken one not offered");
        assert!(p_ratio(&[0.0, 0.0], 0..2, Some(0)).is_nan());
    }

    fn sums(paid: f64, least: f64, expected: f64) -> RouteGapSums {
        RouteGapSums { weight: 2.0, paid, least, expected, ..RouteGapSums::new(false) }
    }

    #[test]
    fn nothing_added_leaves_the_route_gap_as_it_was_bit_for_bit() {
        // S302: a run whose trips are all given their mode adds no sums of the trips choosing
        // theirs, and its gaps must not move by a bit.
        let given = sums(1_000.1, 900.3, 950.7);
        let both = given.plus(&RouteGapSums::new(false));
        let bits = |(a, b, c): (f64, f64, f64)| (a.to_bits(), b.to_bits(), c.to_bits());
        assert_eq!(bits(both.gaps()), bits(given.gaps()));
    }

    #[test]
    fn the_route_gap_of_two_groups_is_their_pooled_sums() {
        // 10 % over the least for one group, 30 % for the other, with the same least: 20 %.
        let (a, b) = (sums(110.0, 100.0, 105.0), sums(130.0, 100.0, 105.0));
        let (gap, expected, excess) = a.plus(&b).gaps();
        assert!((gap - 0.2).abs() < 1e-12 && (expected - 0.05).abs() < 1e-12);
        assert!((excess - 0.15).abs() < 1e-12);
        // Not measured: no probabilities in either, no trip at all.
        let blind = RouteGapSums { measured: false, ..b };
        assert!(a.plus(&blind).gaps().1.is_nan());
        assert!(RouteGapSums::new(false).gaps().0.is_nan());
        assert!(RouteGapSums::new(true).generalised_excess().is_nan());
    }
}
