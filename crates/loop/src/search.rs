//! Stage 1, search: the best alternative per mode open to the traveller at the current costs
//! (for transit, at iteration 0 only, the earliest journey per number of rides), appended to
//! the traveller's set if new.

use openmobisim_loading::link_times::LinkTimes;
use openmobisim_search::search::{Query, Route, fastest_many};
use openmobisim_transit::raptor::{self, Journey, by_rides_many};
use openmobisim_types::mode::Mode;
use openmobisim_types::units::Duration;
use rayon::prelude::*;

use crate::alternative::{Alternative, Leg, price};
use crate::context::Context;
use crate::whole;

/// The network modes the search runs for at `iteration`: cars every iteration; bikes and
/// walkers at iteration 0 only, since their link times never change.
fn modes_at(iteration: u32) -> &'static [Mode] {
    if iteration == 0 { &[Mode::Car, Mode::Bike, Mode::Walk] } else { &[Mode::Car] }
}

/// Per trip, the fastest route by `mode` at the reading, or `None` for a trip without the mode
/// or without a route.
fn routes_by(ctx: &Context<'_>, times: &LinkTimes, mode: Mode) -> Vec<Option<Route>> {
    let mut who = Vec::new();
    let mut queries = Vec::new();
    for i in 0..ctx.trip_count() {
        let trip = ctx.trip(i);
        let (Some(o), Some(d)) =
            (ctx.demand.origin_end(trip, mode), ctx.demand.destination_end(trip, mode))
        else {
            continue;
        };
        let at = whole(ctx.departure[i] + ctx.gap_walk(o.gap).get());
        who.push(i);
        queries.push(Query {
            origin: o.node,
            destination: d.node,
            mode,
            departure: ctx.start().saturating_add(at),
        });
    }
    let answers = fastest_many(ctx.network, times, &queries);
    let mut by_trip = vec![None; ctx.trip_count()];
    for (i, answer) in who.into_iter().zip(answers) {
        by_trip[i] = answer;
    }
    by_trip
}

/// Per trip, the journeys on the timetable by number of rides (`by_rides`: for each count of
/// rides that reaches the door earlier than every smaller count, its earliest journey, fewest
/// rides first); empty for a trip without transit, without a stop in reach at either end, or
/// without a journey.
fn journeys(ctx: &Context<'_>) -> Vec<Vec<Journey>> {
    let Some(timetable) = ctx.timetable else {
        return vec![Vec::new(); ctx.trip_count()];
    };
    let mut who = Vec::new();
    let mut queries = Vec::new();
    for i in 0..ctx.trip_count() {
        let trip = ctx.trip(i);
        let (access, egress) = (ctx.demand.access(trip), ctx.demand.egress(trip));
        if access.is_empty() || egress.is_empty() {
            continue;
        }
        who.push(i);
        queries.push(raptor::Query {
            access: access
                .iter()
                .map(|&(stop, walk)| {
                    (stop, ctx.start().saturating_add(whole(ctx.departure[i] + walk.get())))
                })
                .collect(),
            egress: egress.to_vec(),
            max_rides: ctx.parameters.max_rides,
        });
    }
    let slack = Duration(f64::from(ctx.parameters.boarding_slack));
    let answers = by_rides_many(timetable, slack, &queries);
    let mut by_trip = vec![Vec::new(); ctx.trip_count()];
    for (i, answer) in who.into_iter().zip(answers) {
        by_trip[i] = answer;
    }
    by_trip
}

/// What the search offers trip `i` that its `set` does not hold, in mode order: a route's
/// three legs, or each journey's (only to a set without transit).
fn candidates(
    ctx: &Context<'_>,
    i: usize,
    set: &[Alternative],
    routes: &[(Mode, Vec<Option<Route>>)],
    journeys: &[Vec<Journey>],
) -> Vec<(Mode, Vec<Leg>)> {
    let trip = ctx.trip(i);
    let mut out = Vec::new();
    for (mode, by_trip) in routes {
        let Some(route) = &by_trip[i] else { continue };
        if set.iter().any(|a| a.is_route(*mode, &route.links)) {
            continue;
        }
        let (Some(o), Some(d)) =
            (ctx.demand.origin_end(trip, *mode), ctx.demand.destination_end(trip, *mode))
        else {
            continue;
        };
        let legs = vec![
            Leg::Walk(ctx.gap_walk(o.gap)),
            Leg::Route { mode: *mode, links: route.links.clone() },
            Leg::Walk(ctx.gap_walk(d.gap)),
        ];
        out.push((*mode, legs));
    }
    if !set.iter().any(|a| a.mode == Mode::Transit) {
        let access = ctx.demand.access(trip);
        for journey in &journeys[i] {
            let walk_in =
                access.iter().find(|(s, _)| *s == journey.access.0).map_or(0.0, |w| w.1.get());
            let legs = vec![
                Leg::Walk(Duration(walk_in)),
                Leg::Journey(journey.clone()),
                Leg::Walk(journey.egress.1),
            ];
            out.push((Mode::Transit, legs));
        }
    }
    out
}

/// Append the candidates to `set`; count 1 when one of them is cheaper, without the path-size
/// term, than everything the set held before.
fn offer(
    ctx: &Context<'_>,
    times: &LinkTimes,
    i: usize,
    set: &mut Vec<Alternative>,
    candidates: Vec<(Mode, Vec<Leg>)>,
) -> u64 {
    let class = ctx.class[i];
    let cheapest = set
        .iter()
        .map(|a| ctx.parameters.time_money_cost(class, &a.attributes()))
        .fold(f64::INFINITY, f64::min);
    let mut found = false;
    for (mode, legs) in candidates {
        let time = price(times, ctx.start(), ctx.departure[i], &legs);
        let alternative = Alternative { mode, legs, time, ln_path_size: 0.0 };
        if ctx.parameters.time_money_cost(class, &alternative.attributes()) < cheapest {
            found = true;
        }
        set.push(alternative);
    }
    u64::from(found)
}

/// The search stage: every trip's sets grown by what the search found at `times`; the count of
/// trips for which it found an alternative cheaper, without the path-size term, than their
/// set's cheapest (a trip with an empty set counts when it is offered anything).
pub(crate) fn search(
    ctx: &Context<'_>,
    times: &LinkTimes,
    sets: &mut [Vec<Alternative>],
    iteration: u32,
) -> u64 {
    let routes: Vec<(Mode, Vec<Option<Route>>)> =
        modes_at(iteration).iter().map(|&m| (m, routes_by(ctx, times, m))).collect();
    let journeys = if iteration == 0 { journeys(ctx) } else { vec![Vec::new(); ctx.trip_count()] };
    sets.par_iter_mut()
        .enumerate()
        .map(|(i, set)| {
            let found = candidates(ctx, i, set, &routes, &journeys);
            offer(ctx, times, i, set, found)
        })
        .sum()
}
