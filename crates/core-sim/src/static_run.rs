//! Flow level 1 in a run: the static user-equilibrium assignment ([`FlowMotor::Static`],
//! `core_loading::level1`; S294).
//!
//! A run at level 1 makes **one loading**, and the equilibrium is found before it, where the
//! other levels generate their route sets:
//!
//! 1. **The demand.** The car trips that will set off (a dry pass over each traveller's day with
//!    the vehicle-location rule the loading applies: a trip whose car is not at its origin does
//!    not drive), grouped by route key (origin node, destination node). A pair's hourly flow is
//!    its trips' PCU (each a car of its traveller's weight) over the period in hours
//!    ([`FlowMotor::Static::period_s`]).
//! 2. **The equilibrium**, by the strategy's solver: `frank_wolfe`, or the method of successive
//!    averages for `msa` and any other (`free_flow`'s one iteration is the all-or-nothing
//!    assignment on free-flow times), capped at the strategy's `max_iterations` and stopped at its
//!    [`crate::Equilibration::gap_tolerance`]. A road disruption known to the travellers is a
//!    capacity factor for the whole period.
//! 3. **The results in the run's own shape.** The equilibrium's routes are the run's route sets
//!    (each costed at its time at the equilibrium); each trip of a pair is given one of its
//!    routes, by the route's share of the pair's flow, the largest remainder in trip order
//!    ([`openmobisim_core_loading::apportion`]); the loading drives each car on its route at the
//!    equilibrium's link times, so every trip arrives (a static model has no end of the day: an
//!    arrival after the window's end still counts as completed); the per-link results are the
//!    equilibrium's flows and times spread evenly over the period's bins; and the solver's
//!    iterations are the run's convergence reports.
//!
//! **Bikes and walking** are not part of it: at level 1 they travel on their own layers as at
//! every level. Transit, parkings, mode choice and disruptions the travellers do not know of are
//! refused at level 1, with a message that says what does run them.

use std::collections::BTreeMap;

use openmobisim_core_demand::{Mode, Travellers, Trips, VehicleKind, VehicleLocations};
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_loading::{
    Bpr, LinkBins, StaticDemand, StaticOptions, StaticResult, StaticSolver, apportion, solve_static,
};
use openmobisim_core_routes::{Route, RouteKey, RouteSets};
use openmobisim_core_types::ids::{EntityId, LinkId, NodeId, TravellerId, TripId};

use crate::disruptions::Disruptions;
use crate::equilibration::{Equilibration, IterationReport};
use crate::route_choice::{NO_ROUTE, RouteChoices};
#[cfg(doc)]
use crate::run::FlowMotor;

/// What the static assignment found, in the run's shape.
pub(crate) struct StaticSolution {
    /// The solver's flows, times, routes and iterations.
    pub result: StaticResult,
    /// The equilibrium's routes as route sets, one key per pair that drives.
    pub route_sets: RouteSets,
    /// Each trip's route among them.
    pub route_choices: RouteChoices,
    /// Per link, the trips (unweighted) whose route uses it.
    trips_on_link: Vec<u32>,
    /// The period the hourly flows are of, in seconds.
    period_s: u32,
}

/// Why a run cannot be made at level 1, if it cannot.
pub(crate) fn refusal(
    transit: bool,
    parking: bool,
    mode_choice: bool,
    disruptions: &Disruptions,
) -> Option<String> {
    let what = if transit {
        "a timetable (transit)"
    } else if parking {
        "parkings (park-and-ride, bike-and-ride)"
    } else if mode_choice {
        "mode choice"
    } else if !disruptions.transit.is_empty() {
        "transit disruptions"
    } else if !disruptions.is_empty() && !disruptions.known {
        "disruptions the travellers do not know of"
    } else {
        return None;
    };
    Some(format!(
        "flow level 1 (the static assignment) loads car trips on one period's flows, with bike and \
         walk trips on their layers; it cannot simulate {what}. Run it at flow level 0, 2, 3 or 4."
    ))
}

/// Which car trips set off: each traveller's day in order, a car trip driving only if the
/// traveller's car is at its origin, as the loading decides; a trip whose two ends snapped to
/// one node drives nowhere and is not counted.
fn drivers(travellers: &Travellers, trips: &Trips, keys: &[RouteKey]) -> Vec<bool> {
    let mut vehicles = VehicleLocations::at_first_trip_origin(travellers, trips);
    let mut out = vec![false; trips.len() as usize];
    for t in 0..travellers.len() {
        let traveller = TravellerId::new(t);
        for trip in travellers.trips_of(traveller) {
            if trips.mode(trip) != Mode::Car {
                continue;
            }
            if vehicles.is_at_origin(travellers, traveller, VehicleKind::Car, trips.origin(trip)) {
                vehicles.relocate(traveller, VehicleKind::Car, trips.destination(trip));
                let key = keys[trip.index()];
                out[trip.index()] = key.origin != key.destination;
            }
        }
    }
    out
}

/// The inputs of [`solve`].
pub(crate) struct Inputs<'a> {
    pub network: &'a RoadNetwork,
    pub turns: &'a TurnTable,
    pub travellers: &'a Travellers,
    pub trips: &'a Trips,
    /// Each trip's route key (a node to itself for a trip that is not a car trip).
    pub keys: &'a [RouteKey],
    pub bpr: Bpr,
    pub strategy: &'a dyn Equilibration,
    pub disruptions: &'a Disruptions,
    pub period_s: u32,
    /// When the period starts, seconds after midnight.
    pub start_s: u32,
}

/// The static assignment of the run's car trips. See the [module docs](self).
///
/// # Panics
///
/// Panics if `period_s` is 0.
pub(crate) fn solve(inputs: &Inputs<'_>) -> StaticSolution {
    let Inputs { network, turns, travellers, trips, keys, .. } = *inputs;
    assert!(inputs.period_s > 0, "a period has a length");
    let per_hour = 3600.0 / f64::from(inputs.period_s);
    // The pairs, sorted (as a store's keys are), each with its drivers in trip order.
    let driving = drivers(travellers, trips, keys);
    let mut pairs: BTreeMap<RouteKey, Vec<TripId>> = BTreeMap::new();
    for (i, &drives) in driving.iter().enumerate() {
        if drives {
            pairs.entry(keys[i]).or_default().push(TripId::from_index(i));
        }
    }
    let flow = |members: &[TripId]| -> f64 {
        members.iter().map(|&t| f64::from(travellers.weight(trips.traveller(t)))).sum::<f64>()
            * per_hour
    };
    let mut demand = StaticDemand::default();
    for (key, members) in &pairs {
        let origin = NodeId::new(key.origin);
        let entry = (NodeId::new(key.destination), flow(members));
        match demand.origins.last_mut() {
            Some((o, dests)) if *o == origin => dests.push(entry),
            _ => demand.origins.push((origin, vec![entry])),
        }
    }
    // A road disruption the travellers know of: its capacity factor for the whole period, if
    // it falls in the period.
    let (start, end) =
        (f64::from(inputs.start_s), f64::from(inputs.start_s) + f64::from(inputs.period_s));
    let capacity_factor: Vec<(LinkId, f64)> = inputs
        .disruptions
        .road
        .iter()
        .filter(|d| d.from_s < end && d.to_s > start)
        .flat_map(|d| d.links.iter().map(move |&l| (l, d.factor)))
        .collect();
    let solver = if inputs.strategy.name() == "frank_wolfe" {
        StaticSolver::FrankWolfe
    } else {
        StaticSolver::Msa
    };
    let options = StaticOptions {
        bpr: inputs.bpr,
        solver,
        max_iterations: inputs.strategy.max_iterations().max(1),
        tolerance: inputs.strategy.gap_tolerance(),
        capacity_factor,
    };
    let result = solve_static(network, turns, &demand, &options);

    // The route sets: each pair's routes, costed at the equilibrium's times.
    let time_of = |links: &[LinkId]| links.iter().map(|l| result.times_s[l.index()]).sum::<f64>();
    let sets: Vec<(RouteKey, Vec<Route>)> = pairs
        .keys()
        .zip(&result.routes)
        .map(|(&key, routes)| {
            let routes = routes
                .iter()
                .map(|(links, _)| Route {
                    links: links.clone(),
                    cost: time_of(links),
                    overlap: 0.0,
                })
                .collect();
            (key, routes)
        })
        .collect();
    let descriptor = format!("static;solver={solver:?}");
    let route_sets = RouteSets::from_routes(network, "static", &descriptor, sets);

    // Each driver's route, by its route's share of the pair's flow, in trip order.
    let total = trips.len() as usize;
    let mut route_choices = RouteChoices {
        route: vec![NO_ROUTE; total],
        alternatives: vec![0; total],
        probability: vec![f64::NAN; total],
        weight: (0..total)
            .map(|i| travellers.weight(trips.traveller(TripId::from_index(i))))
            .collect(),
    };
    let mut trips_on_link = vec![0_u32; network.link_count() as usize];
    for (pair, members) in pairs.values().enumerate() {
        let routes = &result.routes[pair];
        let shares: Vec<f64> = routes.iter().map(|(_, f)| *f).collect();
        let sum: f64 = shares.iter().sum();
        let first = route_sets.route_range(pair).start;
        let count = u32::try_from(routes.len()).expect("few routes");
        for (&trip, r) in members.iter().zip(apportion(&shares, members.len())) {
            let t = trip.index();
            route_choices.route[t] = u32::try_from(first + r).expect("routes fit u32");
            route_choices.alternatives[t] = count;
            route_choices.probability[t] = shares[r] / sum;
            for link in &routes[r].0 {
                trips_on_link[link.index()] += 1;
            }
        }
    }
    StaticSolution { result, route_sets, route_choices, trips_on_link, period_s: inputs.period_s }
}

impl StaticSolution {
    /// Every link's time at the equilibrium, seconds (what the loading drives at).
    pub fn times_s(&self) -> &[f64] {
        &self.result.times_s
    }

    /// The solver's iterations as the run's reports: the step as the share that chose again,
    /// the relative gap as the gap and its disequilibrium (a deterministic equilibrium expects
    /// none), the total time of the flows over the period, and the trips the loading completed
    /// (`completed`, `people`), the same at every iteration.
    pub fn reports(&self, completed: u32, people: f64) -> Vec<IterationReport> {
        let hours = f64::from(self.period_s) / 3600.0;
        self.result
            .iterations
            .iter()
            .map(|it| {
                let mut r = IterationReport::unmeasured(it.iteration);
                r.reselected_share = it.step;
                r.total_travel_time_s = it.total_time * hours;
                r.completed = completed;
                r.completed_people = people;
                r.truncated = 0;
                r.time_change = it.time_change;
                r.gap = it.gap;
                r.gap_expected = 0.0;
                r.gap_excess = it.gap;
                r.incomplete_share = 0.0;
                r
            })
            .collect()
    }

    /// The per-link results: each link's equilibrium flow and time spread evenly over the
    /// bins of `bin_seconds` the period `[start_s, start_s + period)` covers (a bin it covers in
    /// part gets that part), its trips' count spread the same way and rounded so that it adds up
    /// to the trips on the link.
    pub fn link_bins(&self, bin_seconds: u32, start_s: u32) -> LinkBins {
        let end_s = start_s.saturating_add(self.period_s);
        let length = f64::from(end_s - start_s);
        let mut rows = Vec::new();
        for (l, &flow) in self.result.flows.iter().enumerate() {
            if flow <= 0.0 {
                continue;
            }
            let link = u32::try_from(l).expect("links fit u32");
            let time = self.result.times_s[l];
            let trips = f64::from(self.trips_on_link[l]);
            let mut given = 0_u32;
            let mut b = start_s / bin_seconds;
            while b.saturating_mul(bin_seconds) < end_s {
                let to = end_s.min((b + 1).saturating_mul(bin_seconds));
                let covered = f64::from(to - start_s.max(b * bin_seconds));
                let pcu = flow * covered / 3600.0;
                // The trips up to the end of this bin, rounded: the counts add up exactly.
                #[allow(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "a count of trips, rounded"
                )]
                let upto = (f64::from(to - start_s) / length * trips).round() as u32;
                rows.push((b, link, upto - given, pcu, pcu * time));
                given = upto;
                b += 1;
            }
        }
        LinkBins::from_rows(bin_seconds, rows)
    }
}
