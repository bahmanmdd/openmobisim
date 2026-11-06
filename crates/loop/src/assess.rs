//! Stage 5, assess: the iteration's measures, reported; and the stopping rule.

use openmobisim_types::reduce::fixed_order_map_sum;

use crate::alternative::Alternative;
use crate::context::Context;
use crate::load::Realised;
use crate::share;

/// One iteration's row of the iterations table.
#[derive(Clone, Debug, PartialEq)]
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
    /// The share of trips with an alternative that changed alternative in this iteration's choice.
    pub switch_share: f64,
    /// The share of trips with an alternative for which this iteration's search found one
    /// cheaper on time and money than their set's cheapest.
    pub found_share: f64,
    /// The relative duality gap at this iteration's measured costs: over trips with an
    /// alternative, the sum of the chosen alternative's cost less the set's least cost, over
    /// the sum of the least costs.
    pub gap: f64,
    /// The seconds the five stages took: search, choose, load, measure, assess.
    pub stage_seconds: [f64; 5],
}

/// Who is counted: trips with at least one alternative.
fn with_alternative(sets: &[Vec<Alternative>]) -> u64 {
    sets.iter().filter(|s| !s.is_empty()).count() as u64
}

/// The assess stage's measures, from the counts of the search and the choice and the loading's
/// outcome. The stage seconds are the caller's.
pub(crate) fn assess(
    ctx: &Context<'_>,
    sets: &[Vec<Alternative>],
    chosen: &[Option<u32>],
    realised: &Realised,
    iteration: u32,
    found: u64,
    switches: u64,
) -> Iteration {
    let n = ctx.trip_count() as u64;
    let counted = with_alternative(sets);
    let finished = realised.finished.iter().filter(|&&f| f).count() as u64;
    let indices: Vec<usize> = (0..ctx.trip_count()).collect();
    let time_sum = fixed_order_map_sum(&indices, |&i| {
        if realised.finished[i] { realised.arrival[i] - ctx.departure[i] } else { 0.0 }
    });
    let paid = fixed_order_map_sum(&indices, |&i| {
        chosen[i]
            .map_or(0.0, |j| ctx.parameters.cost(ctx.class[i], &sets[i][j as usize].attributes()))
    });
    let least = fixed_order_map_sum(&indices, |&i| {
        if sets[i].is_empty() {
            return 0.0;
        }
        sets[i]
            .iter()
            .map(|a| ctx.parameters.cost(ctx.class[i], &a.attributes()))
            .fold(f64::INFINITY, f64::min)
    });
    Iteration {
        iteration,
        mean_trip_time: time_sum / share(finished, 1),
        finished,
        unfinished: counted - finished,
        without_alternative: n - counted,
        switch_share: share(switches, counted),
        found_share: share(found, counted),
        gap: if least > 0.0 { (paid - least) / least } else { f64::NAN },
        stage_seconds: [0.0; 5],
    }
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
