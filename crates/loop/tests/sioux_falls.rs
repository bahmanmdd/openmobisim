//! Test 6, Sioux Falls (TNTP: 24 nodes, 76 links, 528 OD pairs), against the
//! capacity-constrained equilibrium (Larsson and Patriksson 1995; Cominetti, Correa and Olver
//! 2022), the linear program `equilibrium_lp.py` solves with scipy at 0.045 of the TNTP demand
//! per hour with the capacities at a tenth (the program is invariant to scaling flows and
//! capacities together; 0.45 of TNTP is 0.9 of the hourly demand version 1 used, where the
//! total time is unique, range 0.014 %). Judged, at 3 % as test 5: the mean trip 11.18 min and
//! `Σ τ x` 151 080 veh·min per hour. The OD times are judged against their ranges over the
//! dual's optimal face (all but a few are points) as the demand-weighted mean of each pair's
//! distance outside its range, at the step's bound: a run of K iterations last moved a share
//! 1/K of travellers, so a link at capacity ν saw an inflow residual of ν/K and its queue
//! drifted by 3 600/K s over the hour read, which is what a pair's time crossing one queued
//! link can be off by (109 s at K = 33; a pair's hourly mean also rests on a few dozen trips).
//! As inclusions, every link with a standing queue is one the program may queue, its entries at
//! capacity within 3 %. Three hours of constant demand; the second hour's departures are the
//! steady state.
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

use benchmark::{equilibrium, fixtures, link_hours, steady, streams};
use common::{at, car_link, demand, print_iterations, time_only};
use openmobisim_graph::geometry::{LonLat, Projection};
use openmobisim_graph::network::{Network, NetworkBuilder};
use openmobisim_loop::run::run;
use openmobisim_types::ids::EntityId;
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

#[test]
fn the_steady_hour_is_the_capacity_constrained_equilibrium() {
    let e = equilibrium("sioux_falls");
    assert_eq!(e.scale, (0.045, 0.1), "the fixture's scales");
    assert!((e.mean.1 - e.mean.0) / e.mean.0 < 0.001, "the mean trip is unique");
    let net = network(e.scale.1);
    let rows = streams(&net, &e, 3.0);
    let dem = demand(&net, &rows);
    let started = std::time::Instant::now();
    let result =
        run(&net, &dem, None, (Second(0), Second(4 * 3_600)), &time_only(100)).expect("a run");
    println!(
        "{} trips, {} iterations in {:.1} s",
        rows.len(),
        result.iterations.len(),
        started.elapsed().as_secs_f64()
    );
    print_iterations(&result);
    let s = steady(&net, &dem, &result, 3_600, 7_200);
    println!(
        "second hour: {} trips, mean {:.2} min against {:.2} min",
        s.count,
        s.mean / 60.0,
        e.mean.0 / 60.0
    );
    println!(
        "Σ τ x of the routes taken {:.0} veh·min/h against {:.0}",
        s.tau_x / 60.0,
        e.tau_x.0 / 60.0
    );
    let hours = link_hours(&net, &result, 80 * 60, 140 * 60);
    let (mut standing_links, mut off) = (0, Vec::new());
    let (mut sum_xy, mut sum_xx, mut sum_yy) = (0.0, 0.0, 0.0);
    for (link, h) in &hours {
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
            if (h.entries / cap - 1.0).abs() > 0.03 || q_hi <= 0.0 {
                off.push(id);
            }
        }
    }
    println!(
        "cosine of the link flows against the program's: {:.4}",
        sum_xy / (sum_xx * sum_yy).sqrt()
    );
    let (mut outside, mut weight) = (0.0, 0.0);
    for ((o, d), rate, lo, hi) in &e.od {
        // A pair of a few trips an hour may have none departing in the hour read.
        let Some(got) = s.od_mean.get(&(o.clone(), d.clone())) else { continue };
        outside += rate * (lo - got).max(got - hi).max(0.0);
        weight += rate;
    }
    let bound = 3_600.0 / result.iterations.len() as f64;
    println!(
        "OD times: demand-weighted mean distance outside the program's ranges {:.1} s, the step's bound {bound:.1} s",
        outside / weight
    );
    assert!(outside / weight <= bound, "OD times outside their ranges by {} s", outside / weight);
    assert!((s.mean - e.mean.0).abs() <= 0.03 * e.mean.0, "mean trip {}", s.mean);
    assert!((s.tau_x - e.tau_x.0).abs() <= 0.03 * e.tau_x.0, "Σ τ x {}", s.tau_x);
    assert!(standing_links > 0, "some link has a standing queue");
    assert!(
        off.is_empty(),
        "links with a standing queue off their capacity or never queued by the program: {off:?}"
    );
}
