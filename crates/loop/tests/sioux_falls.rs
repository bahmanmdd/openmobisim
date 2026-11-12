//! Test 6, Sioux Falls (TNTP: 24 nodes, 76 links, 528 OD pairs), against the
//! capacity-constrained equilibrium (Larsson and Patriksson 1995; Cominetti, Correa and Olver
//! 2022), the linear program `equilibrium_lp.py` solves with scipy at 0.045 of the TNTP demand
//! per hour with the capacities at a tenth (the program is invariant to scaling flows and
//! capacities together; 0.45 of TNTP is 0.9 of the hourly demand version 1 used, where the
//! total time is unique, range 0.014 %): the mean trip 11.18 min, `Σ τ x` 151 080 veh·min per
//! hour, 20 links that must queue, each at its capacity. Three hours of constant demand; the
//! second hour's departures are read as the steady state. Two tiers. The regular tier, 48 814
//! trips at the stopping rule: 3 % on a time and on `Σ τ x`, 3 % of capacity on a queued link's
//! entries. The convergence tier (ignored in the debug suite; CI runs it in release), twice the
//! population (0.09 of TNTP per hour, capacities at a fifth, 97 362 trips) to a fixed 400
//! iterations: `Σ τ x`, the route pattern, converges to the program (measured −0.00 % at 400,
//! +0.02 % at four times the population) and is judged at 0.1 %; a queued link's entries at
//! 2 % of capacity (measured within 1.7 %); the mean trip only at 1 %, because the queues of a
//! point queue at constant demand drift along the window (measured by departure hour at four
//! times the population and 400 iterations: −0.8 % in hour 2, −0.5 % in hour 3; with five
//! hours of demand +0.7 % in hour 4), a residual that vanishes neither with iterations nor with
//! the population. The OD times' distance outside their ranges over the dual's optimal face is
//! reported.
//!
//! Bar-Gera's user-equilibrium flows (`SiouxFalls_flow.tntp`) are the equilibrium of the BPR
//! functions, a model without capacity or queue; they are not a point-queue equilibrium and are
//! not judged here.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "counts of trips and seconds of the window, far below 2^53 and above zero"
)]

mod benchmark;
mod common;

use benchmark::{Equilibrium, equilibrium, fixtures, link_hours, steady, streams};
use common::{
    all_links, at, car_link, demand, far_from_best, print_iterations, replay_queues, time_only,
};
use openmobisim_graph::geometry::{LonLat, Projection};
use openmobisim_graph::network::{Network, NetworkBuilder};
use openmobisim_loop::parameters::Parameters;
use openmobisim_loop::run::{Run, run};
use openmobisim_types::ids::{EntityId, LinkId, TripId};
use openmobisim_types::mode::Mode;
use openmobisim_types::time::Second;

/// The data rows of a TNTP file: those with at least `fields` fields starting with a number.
fn rows(path: &std::path::Path, fields: usize) -> Vec<Vec<String>> {
    std::fs::read_to_string(path)
        .expect("a TNTP file")
        .lines()
        .map(|line| {
            line.replace(';', "").split_whitespace().map(str::to_string).collect::<Vec<_>>()
        })
        .filter(|f| f.len() >= fields && f[0].parse::<u32>().is_ok())
        .collect()
}

/// The TNTP network: a free-flow minute as a kilometre at 60 km/h, the capacity at
/// `capacity_scale`, the nodes at their longitude and latitude projected to metres (UTM).
fn network(capacity_scale: f64) -> Network {
    let folder = fixtures("sioux_falls");
    let mut b = NetworkBuilder::new();
    let nodes: Vec<(String, LonLat)> = rows(&folder.join("SiouxFalls_node.tntp"), 3)
        .into_iter()
        .map(|f| (f[0].clone(), LonLat::new(f[1].parse().expect("x"), f[2].parse().expect("y"))))
        .collect();
    let projection = Projection::for_extent(nodes.iter().map(|n| n.1)).expect("a zone");
    for (id, p) in &nodes {
        let xy = projection.project(*p);
        b.add_node(id.as_str(), at(xy.x, xy.y));
    }
    for f in rows(&folder.join("SiouxFalls_net.tntp"), 10) {
        let (cap, min): (f64, f64) = (f[2].parse().expect("cap"), f[4].parse().expect("min"));
        let row = car_link(min * 1_000.0, 60.0, cap * capacity_scale);
        b.add_link(format!("{}-{}", f[0], f[1]), f[0].as_str(), f[1].as_str(), row);
    }
    b.build(common::VALUES).expect("a network")
}

/// Part 3's bearing on part 2, printed, not judged: the deviation of `Σ τ x` over the second
/// hour's departures from `reference` (veh·s), as taken and as if every traveller more than
/// 0.5 min from its best (`excess`, from `far_from_best`) had taken that best's route.
fn tau_x_if_on_best(result: &Run, net: &Network, excess: &[f64], reference: f64) {
    let free = |route: &[LinkId]| {
        route.iter().map(|&l| net.free_flow_time(l, Mode::Car).get()).sum::<f64>()
    };
    let (mut taken_tau_x, mut best_tau_x) = (0.0, 0.0);
    for (i, &over) in excess.iter().enumerate() {
        if over.is_nan() || !(3_600..7_200).contains(&result.trips.departure[i].get()) {
            continue;
        }
        let trip = TripId::from_index(i);
        let best = result.set(trip).iter().min_by(|a, b| a.time.total_cmp(&b.time));
        let best = best.expect("a finished trip has a set");
        let on_taken = free(result.taken(trip).unwrap_or(best).route().expect("a route"));
        taken_tau_x += on_taken;
        best_tau_x += if over > 0.5 { free(best.route().expect("a route")) } else { on_taken };
    }
    println!(
        "part 3, choices: Σ τ x of the second hour against the program {:+.3} % as taken, {:+.3} % with the travellers over 0.5 min on their best",
        100.0 * (taken_tau_x / reference - 1.0),
        100.0 * (best_tau_x / reference - 1.0)
    );
}

/// Part 2 on the links, printed: per link with a standing queue over minutes 80 to 140 its
/// entries and delay against the program; the cosine of all link flows against the program's.
/// Returns how many links had a standing queue and those off their capacity by more than
/// `tol_entries` or never queued by the program.
fn queued_links(
    net: &Network,
    result: &Run,
    e: &Equilibrium,
    tol_entries: f64,
) -> (usize, Vec<String>) {
    let (mut standing_links, mut off) = (0, Vec::new());
    let (mut sum_xy, mut sum_xx, mut sum_yy) = (0.0, 0.0, 0.0);
    for (link, h) in &link_hours(net, result, 80 * 60, 140 * 60) {
        let id = net.link_ids().external(link.raw()).to_string();
        let (lp_flow, q_lo, q_hi, cap) = e.links[&id];
        sum_xy += h.entries * lp_flow;
        sum_xx += h.entries * h.entries;
        sum_yy += lp_flow * lp_flow;
        if h.standing {
            standing_links += 1;
            println!(
                "  link {id:>5}: standing queue, entries {:6.1}, discharge {:6.1}, capacity {cap:6.1} veh/h, mean delay {:4.0} s (program: flow {lp_flow:6.1}, queue {q_lo:3.0} to {q_hi:3.0} s)",
                h.entries, h.discharge, h.delay
            );
            if (h.entries / cap - 1.0).abs() > tol_entries || q_hi <= 0.0 {
                off.push(id);
            }
        }
    }
    println!(
        "part 2, flows: cosine of the link flows against the program's {:.4}",
        sum_xy / (sum_xx * sum_yy).sqrt()
    );
    (standing_links, off)
}

/// The steady hour against the fixture `file`, solved at `scale`, run with `parameters`,
/// judged at `tol_time` on the mean trip, `tol_taux` on `Σ τ x`, `tol_entries` of capacity on a
/// queued link's entries. Reported in order: part 1, the queue and the record; part 2, the flows; part 3, the
/// choices; the mixed numbers that follow from them.
fn steady_hour(
    file: &str,
    scale: (f64, f64),
    parameters: &Parameters,
    tol_time: f64,
    tol_taux: f64,
    tol_entries: f64,
) {
    let e = equilibrium("sioux_falls", file);
    assert_eq!(e.scale, scale, "the fixture's scales");
    assert!((e.mean.1 - e.mean.0) / e.mean.0 < 0.001, "the mean trip is unique");
    let net = network(e.scale.1);
    let rows = streams(&net, &e, 3.0);
    let dem = demand(&net, &rows);
    let started = std::time::Instant::now();
    let result = run(&net, &dem, None, (Second(0), Second(4 * 3_600)), parameters).expect("a run");
    println!(
        "{} trips, {} iterations in {:.1} s",
        rows.len(),
        result.iterations.len(),
        started.elapsed().as_secs_f64()
    );
    print_iterations(&result);
    let s = steady(&net, &dem, &result, 3_600, 7_200);

    // Part 1, the queue and the record: every link replayed car by car, every car followed
    // along its route.
    let queue = replay_queues(&net, &result, &all_links(&net));
    println!("part 1, queue: {}", queue.describe(&net));

    // Part 2, flows: Σ τ x; the links with a standing queue at their capacity.
    println!(
        "part 2, flows: Σ τ x of the routes taken {:.0} veh·min/h against {:.0}",
        s.tau_x / 60.0,
        e.tau_x.0 / 60.0
    );
    let (standing_links, off) = queued_links(&net, &result, &e, tol_entries);

    // Part 3, choices: each traveller against the best of its own set.
    let excess = far_from_best(&result);
    tau_x_if_on_best(&result, &net, &excess, e.tau_x.0);

    // Mixed, following from parts 1 to 3: the mean trip; the OD times.
    println!(
        "mixed: second hour {} trips, mean {:.2} min against {:.2} min",
        s.count,
        s.mean / 60.0,
        e.mean.0 / 60.0
    );
    let (mut outside, mut weight) = (0.0, 0.0);
    for ((o, d), rate, lo, hi) in &e.od {
        // A pair of a few trips an hour may have none departing in the hour read.
        let Some(got) = s.od_mean.get(&(o.clone(), d.clone())) else { continue };
        outside += rate * (lo - got).max(got - hi).max(0.0);
        weight += rate;
    }
    println!(
        "mixed: OD times, demand-weighted mean distance outside the program's ranges {:.1} s of a {:.1} s mean trip",
        outside / weight,
        e.mean.0
    );

    queue.judge(&net, true);
    assert!(
        (s.tau_x - e.tau_x.0).abs() <= tol_taux * e.tau_x.0,
        "part 2, flows: Σ τ x {}",
        s.tau_x
    );
    assert!(standing_links > 0, "part 2, flows: some link has a standing queue");
    assert!(
        off.is_empty(),
        "part 2, flows: links with a standing queue off their capacity or never queued by the program: {off:?}"
    );
    assert!((s.mean - e.mean.0).abs() <= tol_time * e.mean.0, "mixed: mean trip {}", s.mean);
}

#[test]
fn the_steady_hour_is_the_capacity_constrained_equilibrium() {
    steady_hour("equilibrium.csv", (0.045, 0.1), &time_only(100), 0.03, 0.03, 0.03);
}

/// The convergence tier: twice the population to a fixed 400 iterations.
#[test]
#[ignore = "the convergence tier: about 25 s in release, run by CI's release step"]
fn at_twice_the_population_and_400_iterations_the_route_pattern_within_0_1_percent() {
    let fixed = Parameters { iteration_floor: 400, ..time_only(400) };
    steady_hour("equilibrium_x2.csv", (0.09, 0.2), &fixed, 0.01, 0.001, 0.02);
}
