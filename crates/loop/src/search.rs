//! Stage 1, search: the best alternative per mode open to the traveller at the current costs
//! (for transit, at iteration 0 only, the earliest journey per number of rides, between any
//! stops in reach and from and to each of the nearest few alone), appended to the traveller's
//! set if new: a car route new to its origin-destination pair only while the pair holds fewer
//! than `max_car_routes` routes (a traveller's first car route always), and of the journeys at
//! most `max_transit_journeys`, the cheapest.

use std::collections::HashSet;

use openmobisim_loading::link_times::LinkTimes;
use openmobisim_search::search::{Query, Route, fastest_many};
use openmobisim_transit::cheapest::{CostQuery, Weights, cheapest_by_rides_many};
use openmobisim_transit::raptor::{self, Journey, by_rides_many};
use openmobisim_types::ids::{LinkId, StopId, TransitRunId};
use openmobisim_types::mode::Mode;
use openmobisim_types::units::Duration;
use rayon::prelude::*;

use crate::alternative::{Alternative, Leg, path_sizes, price};
use crate::context::Context;
use crate::parameters::{
    TRANSIT_ACCESS_MIN, TRANSIT_EGRESS_MIN, TRANSIT_FIRST_WAIT_MIN, TRANSIT_IN_VEHICLE_MIN,
    TRANSIT_TRANSFER_WAIT_MIN, TRANSIT_TRANSFER_WALK_MIN, TRANSIT_TRANSFERS,
};
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
        let Some((o, d)) = ctx.ends(i, mode) else { continue };
        who.push(i);
        queries.push(Query {
            origin: o.node,
            destination: d.node,
            mode,
            departure: Duration(ctx.departure[i] + ctx.gap_walk(o.gap).get()),
            trip: Some(trip),
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

/// What a second of each part of a journey costs class `class`, and a transfer: the class's
/// coefficients per minute over 60.
fn weights(ctx: &Context<'_>, class: usize) -> Weights {
    let b = &ctx.parameters.coefficients[class];
    Weights {
        first_wait: b[TRANSIT_FIRST_WAIT_MIN] / 60.0,
        in_vehicle: b[TRANSIT_IN_VEHICLE_MIN] / 60.0,
        transfer_wait: b[TRANSIT_TRANSFER_WAIT_MIN] / 60.0,
        transfer_walk: b[TRANSIT_TRANSFER_WALK_MIN] / 60.0,
        transfer: b[TRANSIT_TRANSFERS],
    }
}

/// Per trip, the journeys on the timetable by number of rides. The earliest (`by_rides`: for
/// each count of rides that reaches the door earlier than every smaller count, its earliest
/// journey): those between any stops of the trip's two lists, then those from each of the
/// `nearby_stops − 1` nearest stops of the origin alone, then those to each of the
/// `nearby_stops − 1` nearest stops of the destination alone. Then the cheapest for the
/// traveller's class between any stops of the two lists (`Cheapest::by_rides`: for each count
/// that makes a journey cheaper than every smaller count, its cheapest, the walks to and from
/// the stops priced with it). A journey found before is not repeated. Empty for a trip without
/// transit, without a stop in reach at either end, or without a journey.
fn journeys(ctx: &Context<'_>) -> Vec<Vec<Journey>> {
    let Some(timetable) = ctx.timetable else {
        return vec![Vec::new(); ctx.trip_count()];
    };
    let (n, max_rides) = (usize::from(ctx.parameters.nearby_stops) - 1, ctx.parameters.max_rides);
    let (mut who, mut queries) = (Vec::new(), Vec::new());
    let (mut who_cost, mut cost_queries, mut same_question) = (Vec::new(), Vec::new(), Vec::new());
    for i in 0..ctx.trip_count() {
        let trip = ctx.trip(i);
        let (access, egress) = (ctx.demand.access(trip), ctx.demand.egress(trip));
        if !ctx.modes[i].contains(Mode::Transit) || access.is_empty() || egress.is_empty() {
            continue;
        }
        let there = |walk: Duration| ctx.departure[i] + walk.get();
        let at = |&(stop, walk): &(StopId, Duration)| {
            (stop, ctx.start().saturating_add(whole(there(walk))))
        };
        let all: Vec<_> = access.iter().map(at).collect();
        same_question.push(queries.len());
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
        // The cost of being at a stop of the origin's list: the walk to it, and the wait for
        // the whole second the traveller is there on.
        let (b, w) = (&ctx.parameters.coefficients[ctx.class[i]], weights(ctx, ctx.class[i]));
        let reach = |&(_, walk): &(StopId, Duration)| {
            let wait = f64::from(whole(there(walk))) - there(walk);
            b[TRANSIT_ACCESS_MIN] / 60.0 * walk.get() + w.first_wait * wait
        };
        who_cost.push(i);
        cost_queries.push(CostQuery {
            query: raptor::Query { access: all, egress: egress.to_vec(), max_rides },
            weights: w,
            access_cost: access.iter().map(reach).collect(),
            egress_cost: egress.iter().map(|e| b[TRANSIT_EGRESS_MIN] / 60.0 * e.1.get()).collect(),
        });
    }
    let slack = Duration(f64::from(ctx.parameters.boarding_slack));
    let earliest = by_rides_many(timetable, slack, &queries);
    // The cheapest search is bounded by the earliest journeys of its question, the first
    // asked for its trip: they are handed over, not searched for again.
    let known: Vec<&[Journey]> = same_question.iter().map(|&q| earliest[q].as_slice()).collect();
    let cheapest = cheapest_by_rides_many(timetable, slack, &cost_queries, &known);
    let mut by_trip = vec![Vec::new(); ctx.trip_count()];
    for (i, answer) in who.into_iter().zip(earliest).chain(who_cost.into_iter().zip(cheapest)) {
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
/// traveller, the first found at equal cost; of those, when there are more than
/// `max_transit_journeys`, that many, the cheapest for the traveller, ties to the first found;
/// in the order found.
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
        let time = price(times, ctx.start(), ctx.departure[i], &legs, ctx.trip(i));
        let priced = Alternative { mode: Mode::Transit, legs, time, ln_path_size: 0.0 };
        let cost = ctx.parameters.cost(ctx.class[i], &priced.attributes());
        let runs: Vec<TransitRunId> = journey.rides.iter().map(|r| r.run).collect();
        match kept.iter_mut().find(|k| k.0 == runs) {
            Some(k) if cost < k.1 => *k = (runs, cost, priced.legs),
            Some(_) => {}
            None => kept.push((runs, cost, priced.legs)),
        }
    }
    let limit = ctx.parameters.max_transit_journeys as usize;
    if kept.len() > limit {
        let mut by_cost: Vec<usize> = (0..kept.len()).collect();
        by_cost.sort_by(|&a, &b| kept[a].1.total_cmp(&kept[b].1).then(a.cmp(&b)));
        let mut keep = vec![false; kept.len()];
        for &j in &by_cost[..limit] {
            keep[j] = true;
        }
        let mut j = 0;
        kept.retain(|_| {
            j += 1;
            keep[j - 1]
        });
    }
    kept.into_iter().map(|k| k.2).collect()
}

/// Hold the car routes the search found (`by_trip`, per trip) to `max_car_routes` per
/// origin-destination pair (`Context::car_pair`). A pair holds the distinct car routes in its
/// travellers' `sets`. A route new to its traveller's set and to the pair is kept only while the
/// pair holds fewer than the limit, the trips taken in trip order; otherwise it is dropped here,
/// so it is neither offered nor counted as found. It never replaces a route. A traveller's first
/// car route is never dropped: the limit holds back further routes, never a car driver's only
/// one, so a pair whose travellers' first routes differ (routes of equal time, the tie broken
/// one way for some departures and the other way for others) can hold more than the limit. A
/// pair is looked into only when its sets' car routes, repeats counted, and the routes found
/// exceed the limit: within the limit, every route found fits, so nothing of the pair can be
/// dropped.
fn limit_car_routes(ctx: &Context<'_>, sets: &[Vec<Alternative>], by_trip: &mut [Option<Route>]) {
    let limit = ctx.parameters.max_car_routes as usize;
    let mut bound = vec![0_usize; ctx.car_pair_trips.len()];
    for (i, set) in sets.iter().enumerate() {
        if let Some(p) = ctx.car_pair[i] {
            let held = set.iter().filter(|a| a.mode == Mode::Car).count();
            bound[p] += held + usize::from(by_trip[i].is_some());
        }
    }
    let mut dropped = Vec::new();
    let looked_into = ctx.car_pair_trips.iter().enumerate().filter(|&(p, _)| bound[p] > limit);
    for (_, trips) in looked_into {
        let mut held: HashSet<&[LinkId]> = trips
            .iter()
            .flat_map(|&i| sets[i].iter().filter(|a| a.mode == Mode::Car))
            .filter_map(Alternative::route)
            .collect();
        for &i in trips {
            let Some(route) = &by_trip[i] else { continue };
            if held.contains(&route.links[..]) {
                continue;
            }
            let first = !sets[i].iter().any(|a| a.mode == Mode::Car);
            if held.len() < limit || first {
                held.insert(&route.links[..]);
            } else {
                dropped.push(i);
            }
        }
    }
    for i in dropped {
        by_trip[i] = None;
    }
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
    let mut out = Vec::new();
    for (mode, by_trip) in routes {
        let Some(route) = &by_trip[i] else { continue };
        if set.iter().any(|a| a.is_route(*mode, &route.links)) {
            continue;
        }
        let Some((o, d)) = ctx.ends(i, *mode) else { continue };
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
/// term, than every alternative of its mode the set held before (the first of a mode counts).
fn offer(
    ctx: &Context<'_>,
    times: &LinkTimes,
    i: usize,
    set: &mut Vec<Alternative>,
    candidates: Vec<(Mode, Vec<Leg>)>,
) -> u64 {
    let class = ctx.class[i];
    let cost = |a: &Alternative| ctx.parameters.time_money_cost(class, &a.attributes());
    let mut cheapest = [f64::INFINITY; 4];
    for a in set.iter() {
        let of_mode = &mut cheapest[usize::from(a.mode.as_u8())];
        *of_mode = of_mode.min(cost(a));
    }
    let mut found = false;
    for (mode, legs) in candidates {
        let time = price(times, ctx.start(), ctx.departure[i], &legs, ctx.trip(i));
        let alternative = Alternative { mode, legs, time, ln_path_size: 0.0 };
        if cost(&alternative) < cheapest[usize::from(mode.as_u8())] {
            found = true;
        }
        set.push(alternative);
    }
    u64::from(found)
}

/// The search stage: every trip's sets grown by what the search found at `times`, the path
/// sizes of a set that grew rewritten over its new members (the logit's choice that follows
/// reads them; a path size depends on who is in the set, not on the reading); the count of
/// trips for which
/// it found an alternative cheaper, without the path-size term, than every alternative of its
/// mode in their set (the first alternative of a mode counts): a route that beats the routes
/// of its mode changes what the traveller can do, whatever the other modes cost.
pub(crate) fn search(
    ctx: &Context<'_>,
    times: &LinkTimes,
    sets: &mut [Vec<Alternative>],
    iteration: u32,
) -> u64 {
    let mut routes: Vec<(Mode, Vec<Option<Route>>)> =
        modes_at(iteration).iter().map(|&m| (m, routes_by(ctx, times, m))).collect();
    if let Some((_, by_trip)) = routes.iter_mut().find(|(m, _)| *m == Mode::Car) {
        limit_car_routes(ctx, sets, by_trip);
    }
    let journeys = if iteration == 0 { journeys(ctx) } else { vec![Vec::new(); ctx.trip_count()] };
    sets.par_iter_mut()
        .enumerate()
        .map(|(i, set)| {
            let found = candidates(ctx, times, i, set, &routes, &journeys);
            let grew = !found.is_empty();
            let cheaper = offer(ctx, times, i, set, found);
            if grew {
                path_sizes(ctx.network, set);
            }
            cheaper
        })
        .sum()
}
