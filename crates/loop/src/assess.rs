//! Stage 5, assess: the iteration's measures, reported; and the stopping rule.

use openmobisim_choice::cost::ChoiceError;
use openmobisim_types::mode::Mode;
use openmobisim_types::reduce::fixed_order_map_sum;
use rayon::prelude::*;

use crate::alternative::Alternative;
use crate::choose::{Model, choice};
use crate::context::Context;
use crate::load::Realised;
use crate::parameters::ChoiceModel;
use crate::share;

/// One iteration's row of the iterations table.
#[derive(Clone, Debug)]
pub struct Iteration {
    /// The iteration, from 0.
    pub iteration: u32,
    /// The mean door-to-door travel time of the finished trips, seconds; `NaN` when none finished.
    pub mean_trip_time: f64,
    /// Trips that arrived before the window's end.
    pub finished: u64,
    /// Trips with an alternative still travelling at the window's end.
    pub unfinished: u64,
    /// Trips with no alternative in any mode open to them.
    pub without_alternative: u64,
    /// Trips with transit open to them and no journey in their set: transit is not offered.
    pub without_transit: u64,
    /// Of those, the trips with no stop within `stop_walk_max` of their origin or of their
    /// destination; the rest have stops at both ends and no journey between them.
    pub without_stop: u64,
    /// The share of trips with an alternative that were offered a new choice in this iteration's
    /// choice (a trip holding nothing chooses: 1 at iteration 0).
    pub offered_share: f64,
    /// The share of trips with an alternative that changed alternative in this iteration's choice.
    pub switch_share: f64,
    /// The share of trips with an alternative for which this iteration's search found one
    /// cheaper, without the path-size term, than every alternative of its mode in their set.
    pub found_share: f64,
    /// Under the logit, the share of trips with an alternative that the logit, choosing again
    /// over their own set at the costs this iteration measured, the set as it stands after this
    /// iteration, would put on another alternative than the one they hold: one that their own
    /// random numbers, fixed for the run, no longer pick at these costs. The next iteration's
    /// choice reads the same unless its search adds an alternative first. 0 means
    /// that nobody would change: the run is at the fixed point of its iterations, for the sets it
    /// has found. The logit puts a share of travellers on an alternative that is not their
    /// lowest-cost one, so the gap does not reach 0 and this share is the measure. `NaN` under
    /// the least-cost choice, where it is not computed: at the equilibrium the routes in use cost
    /// the same, so the share of travellers not on their lowest-cost alternative does not settle.
    pub would_change_share: f64,
    /// Under the least-cost choice, the relative duality gap at this iteration's measured costs:
    /// over trips with an alternative, the sum of the chosen alternative's cost less the set's
    /// least cost, over the sum of the least costs. It weighs each traveller by what it would
    /// save, so it falls to 0 where the routes in use cost the same: it is the measure there.
    /// `NaN` under the logit, whose own shares keep it above 0 at the fixed point.
    pub gap: f64,
    /// The seconds the five stages took: search, choose, load, measure, assess.
    pub stage_seconds: [f64; 5],
}

/// Two rows compare equal when each of their numbers has the same bits, so a measure that is not
/// reported (`NaN`) in both rows does not make them differ. The stages' wall-clock seconds are part
/// of the row, so two runs' rows compare equal only once a caller has set those equal.
impl PartialEq for Iteration {
    fn eq(&self, other: &Self) -> bool {
        let counts = |r: &Self| {
            let c = (r.finished, r.unfinished, r.without_alternative);
            (r.iteration, c, r.without_transit, r.without_stop)
        };
        let numbers = |r: &Self| {
            let shares = [r.offered_share, r.switch_share, r.found_share, r.would_change_share];
            let mut all = vec![r.mean_trip_time, r.gap];
            all.extend(shares.iter().chain(&r.stage_seconds));
            all.into_iter().map(f64::to_bits).collect::<Vec<u64>>()
        };
        counts(self) == counts(other) && numbers(self) == numbers(other)
    }
}

/// Who is counted: trips with at least one alternative.
fn with_alternative(sets: &[Vec<Alternative>]) -> u64 {
    sets.iter().filter(|s| !s.is_empty()).count() as u64
}

/// How many trips the model, choosing again over their own set at the costs the sets hold now,
/// would put on another alternative than the one they hold. It is the choose stage's own choice
/// ([`choice`]), whose result is fixed for given costs and a given set, so this is a count, not
/// an estimate. Each trip's choice is made on its own, in parallel; the first refusal in trip
/// order is returned.
fn would_change(
    ctx: &Context<'_>,
    model: &Model,
    sets: &[Vec<Alternative>],
    chosen: &[Option<u32>],
) -> Result<u64, ChoiceError> {
    let changes: Vec<Result<bool, ChoiceError>> = (0..sets.len())
        .into_par_iter()
        .map(|i| match chosen[i] {
            Some(held) => Ok(choice(ctx, model, i, &sets[i])? != held),
            None => Ok(false),
        })
        .collect();
    changes.into_iter().try_fold(0, |n, changed| Ok(n + u64::from(changed?)))
}

/// The assess stage's measures, from the counts of the search and the choice (offered, switched)
/// and the loading's outcome. The stage seconds are the caller's.
///
/// # Errors
///
/// Under the logit, as the model's choice (a cost that is not finite), read again for the
/// share that would change.
#[allow(
    clippy::too_many_arguments,
    reason = "the stage reads what each of the four before it gave"
)]
pub(crate) fn assess(
    ctx: &Context<'_>,
    model: &Model,
    sets: &[Vec<Alternative>],
    chosen: &[Option<u32>],
    realised: &Realised,
    iteration: u32,
    found: u64,
    (offered, switches): (u64, u64),
) -> Result<Iteration, ChoiceError> {
    let n = (0..ctx.trip_count()).filter(|&i| ctx.runs(i)).count() as u64;
    let counted = with_alternative(sets);
    let finished = realised.finished.iter().filter(|&&f| f).count() as u64;
    let indices: Vec<usize> = (0..ctx.trip_count()).collect();
    let time_sum = fixed_order_map_sum(&indices, |&i| {
        if realised.finished[i] { realised.arrival[i] - ctx.departure[i] } else { 0.0 }
    });
    // Each model reports the one measure that can reach 0 (the field documentation).
    let (would_change_share, gap) = match ctx.parameters.choice {
        ChoiceModel::Logit => (share(would_change(ctx, model, sets, chosen)?, counted), f64::NAN),
        ChoiceModel::Deterministic => (f64::NAN, duality_gap(ctx, sets, chosen, &indices)),
    };
    let without_transit = (0..ctx.trip_count())
        .filter(|&i| {
            ctx.modes[i].contains(Mode::Transit) && !sets[i].iter().any(|a| a.mode == Mode::Transit)
        })
        .count() as u64;
    let without_stop = (0..ctx.trip_count())
        .filter(|&i| {
            let trip = ctx.trip(i);
            ctx.modes[i].contains(Mode::Transit)
                && (ctx.demand.access(trip).is_empty() || ctx.demand.egress(trip).is_empty())
        })
        .count() as u64;
    Ok(Iteration {
        iteration,
        mean_trip_time: time_sum / share(finished, 1),
        finished,
        unfinished: counted - finished,
        without_alternative: n - counted,
        without_transit,
        without_stop,
        offered_share: share(offered, counted),
        switch_share: share(switches, counted),
        found_share: share(found, counted),
        would_change_share,
        gap,
        stage_seconds: [0.0; 5],
    })
}

/// The relative duality gap over `indices`: the sum of the chosen alternatives' costs less the
/// sum of the least costs of their sets, over the latter; `NaN` when that sum is not above 0.
fn duality_gap(
    ctx: &Context<'_>,
    sets: &[Vec<Alternative>],
    chosen: &[Option<u32>],
    indices: &[usize],
) -> f64 {
    let paid = fixed_order_map_sum(indices, |&i| {
        chosen[i]
            .map_or(0.0, |j| ctx.parameters.cost(ctx.class[i], &sets[i][j as usize].attributes()))
    });
    let least = fixed_order_map_sum(indices, |&i| {
        if sets[i].is_empty() {
            return 0.0;
        }
        sets[i]
            .iter()
            .map(|a| ctx.parameters.cost(ctx.class[i], &a.attributes()))
            .fold(f64::INFINITY, f64::min)
    });
    if least > 0.0 { (paid - least) / least } else { f64::NAN }
}

/// The stopping rule, read after an iteration: stop when at most 0.5 % of trips found a cheaper
/// alternative in its search and the range of the mean trip time over the last five iterations
/// (this one included), greatest less least, is at most 0.3 % of this one's; never while one of
/// the five has no finished trip.
#[must_use]
pub fn stops(iterations: &[Iteration]) -> bool {
    let Some(last) = iterations.last() else { return false };
    if iterations.len() < 5
        || last.found_share.is_nan()
        || last.found_share > 0.005
        || !last.mean_trip_time.is_finite()
    {
        return false;
    }
    let tail = &iterations[iterations.len() - 5..];
    if !tail.iter().all(|it| it.mean_trip_time.is_finite()) {
        return false;
    }
    let (lo, hi) = tail.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), it| {
        (lo.min(it.mean_trip_time), hi.max(it.mean_trip_time))
    });
    hi - lo <= 0.003 * last.mean_trip_time
}
