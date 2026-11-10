//! Stage 1, search: the best alternative per mode open to the traveller at the current costs
//! (for transit, at iteration 0 only, the earliest journey per number of rides, between any
//! stops in reach and from and to each of the nearest few alone), appended to the traveller's
//! set if new.

use openmobisim_loading::link_times::LinkTimes;
use openmobisim_search::search::{Query, Route, fastest_many};
use openmobisim_transit::raptor::{self, Journey, by_rides_many};
use openmobisim_types::ids::{StopId, TransitRunId};
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

/// The `n` stops of `list` with the least walk, nearest first, ties to the lower stop id; none
/// for a list of one stop, which the whole list's query already starts from alone.
fn nearest(list: &[(StopId, Duration)], n: usize) -> Vec<(StopId, Duration)> {
    if list.len() < 2 {
        return Vec::new();
    }
    let mut by_walk = list.to_vec();
    by_walk.sort_by(|a, b| a.1.get().total_cmp(&b.1.get()));
    by_walk.truncate(n);
    by_walk
}

/// Per trip, the journeys on the timetable by number of rides (`by_rides`: for each count of
/// rides that reaches the door earlier than every smaller count, its earliest journey, fewest
/// rides first): those between any stops of the trip's two lists, then those from each of the
/// `nearby_stops − 1` nearest stops of the origin alone, then those to each of the
/// `nearby_stops − 1` nearest stops of the destination alone; a journey found before is not repeated. Empty for a
/// trip without transit, without a stop in reach at either end, or without a journey.
fn journeys(ctx: &Context<'_>) -> Vec<Vec<Journey>> {
    let Some(timetable) = ctx.timetable else {
        return vec![Vec::new(); ctx.trip_count()];
    };
    let (n, max_rides) = (usize::from(ctx.parameters.nearby_stops) - 1, ctx.parameters.max_rides);
    let mut who = Vec::new();
    let mut queries = Vec::new();
    for i in 0..ctx.trip_count() {
        let trip = ctx.trip(i);
        let (access, egress) = (ctx.demand.access(trip), ctx.demand.egress(trip));
        if access.is_empty() || egress.is_empty() {
            continue;
        }
        let at = |&(stop, walk): &(StopId, Duration)| {
            (stop, ctx.start().saturating_add(whole(ctx.departure[i] + walk.get())))
        };
        let all: Vec<_> = access.iter().map(at).collect();
        let mut ask = |access: Vec<_>, egress: Vec<_>| {
            who.push(i);
            queries.push(raptor::Query { access, egress, max_rides });
        };
        ask(all.clone(), egress.to_vec());
        for stop in nearest(access, n) {
            ask(vec![at(&stop)], egress.to_vec());
        }
        for stop in nearest(egress, n) {
            ask(all.clone(), vec![stop]);
        }
    }
    let slack = Duration(f64::from(ctx.parameters.boarding_slack));
    let answers = by_rides_many(timetable, slack, &queries);
    let mut by_trip = vec![Vec::new(); ctx.trip_count()];
    for (i, answer) in who.into_iter().zip(answers) {
        for journey in answer {
            if !by_trip[i].contains(&journey) {
                by_trip[i].push(journey);
            }
        }
    }
    by_trip
}

/// The legs of each journey offered to trip `i`: of the journeys that ride the same runs in the
/// same order (they differ in the stop boarded or left at), only the cheapest for the
/// traveller, the first found at equal cost; in the order found.
fn journey_legs(
    ctx: &Context<'_>,
    times: &LinkTimes,
    i: usize,
    found: &[Journey],
) -> Vec<Vec<Leg>> {
    let access = ctx.demand.access(ctx.trip(i));
    let mut kept: Vec<(Vec<TransitRunId>, f64, Vec<Leg>)> = Vec::new();
    for journey in found {
        let walk_in =
            access.iter().find(|(s, _)| *s == journey.access.0).map_or(0.0, |w| w.1.get());
        let legs = vec![
            Leg::Walk(Duration(walk_in)),
            Leg::Journey(journey.clone()),
            Leg::Walk(journey.egress.1),
        ];
        let time = price(times, ctx.start(), ctx.departure[i], &legs);
        let priced = Alternative { mode: Mode::Transit, legs, time, ln_path_size: 0.0 };
        let cost = ctx.parameters.cost(ctx.class[i], &priced.attributes());
        let runs: Vec<TransitRunId> = journey.rides.iter().map(|r| r.run).collect();
        match kept.iter_mut().find(|k| k.0 == runs) {
            Some(k) if cost < k.1 => *k = (runs, cost, priced.legs),
            Some(_) => {}
            None => kept.push((runs, cost, priced.legs)),
        }
    }
    kept.into_iter().map(|k| k.2).collect()
}

/// What the search offers trip `i` that its `set` does not hold, in mode order: a route's
/// three legs, or each journey's (only to a set without transit).
fn candidates(
    ctx: &Context<'_>,
    times: &LinkTimes,
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
        let legs = journey_legs(ctx, times, i, &journeys[i]);
        out.extend(legs.into_iter().map(|legs| (Mode::Transit, legs)));
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
            let found = candidates(ctx, times, i, set, &routes, &journeys);
            offer(ctx, times, i, set, found)
        })
        .sum()
}
