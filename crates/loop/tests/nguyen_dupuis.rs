//! Test 5, Nguyen–Dupuis (1984): 13 nodes, 19 links, four OD pairs, against the
//! capacity-constrained equilibrium (Larsson and Patriksson 1995; Cominetti, Correa and Olver
//! 2022), the linear program `equilibrium_lp.py` solves with scipy at 0.9 of the case's demand,
//! where its total time is unique (range 0.003 %): the mean trip 36.10 min, the OD times 32,
//! 38, 37 and 34 min (each a point over the dual's optimal face), `Σ τ x` 60 610 veh·min per
//! hour of demand; links 5 and 8 must queue, 13 and 19 may; a link with a standing queue runs
//! at its capacity. Each OD pair runs at its constant rate for three hours; the second hour's
//! departures are the steady state, the links read over minutes 80 to 140. Tolerances, as
//! version 1's case set them: 3 % on a time and on `Σ τ x`, 2 % of capacity on a queued
//! link's entries.
//!
//! The network's numbers are the tables of Sensors 25(7) 2262 (2025, CC BY 4.0), made dynamic
//! as version 1 did: a free-flow minute is a kilometre at 60 km/h, capacities and demand ×10
//! per hour. No published equilibrium of this network is a point-queue equilibrium.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "counts of trips and seconds of the window, far below 2^53 and above zero"
)]

mod benchmark;
mod common;

use std::collections::HashMap;

use benchmark::{equilibrium, fixtures, link_hours, steady, streams};
use common::{at, car_link, demand, print_iterations, time_only};
use openmobisim_graph::network::{Network, NetworkBuilder};
use openmobisim_loop::run::run;
use openmobisim_types::ids::EntityId;
use openmobisim_types::time::Second;

fn network() -> Network {
    let folder = fixtures("nguyen_dupuis");
    let mut b = NetworkBuilder::new();
    for line in std::fs::read_to_string(folder.join("nodes.csv")).expect("nodes").lines().skip(1) {
        let f: Vec<&str> = line.split(',').collect();
        b.add_node(f[0], at(f[1].parse().expect("x"), f[2].parse().expect("y")));
    }
    for line in std::fs::read_to_string(folder.join("links.csv")).expect("links").lines().skip(1) {
        let f: Vec<&str> = line.split(',').collect();
        let (km, min, cap): (f64, f64, f64) =
            (f[3].parse().expect("km"), f[4].parse().expect("min"), f[5].parse().expect("cap"));
        b.add_link(f[0], f[1], f[2], car_link(km * 1_000.0, km / (min / 60.0), cap));
    }
    b.build(common::VALUES).expect("a network")
}

#[test]
fn the_steady_hour_is_the_capacity_constrained_equilibrium() {
    let e = equilibrium("nguyen_dupuis", "equilibrium.csv");
    assert_eq!(e.scale, (0.9, 1.0), "the fixture's scales");
    assert!((e.mean.1 - e.mean.0) / e.mean.0 < 0.001, "the mean trip is unique");
    let net = network();
    let rows = streams(&net, &e, 3.0);
    let dem = demand(&net, &rows);
    let result =
        run(&net, &dem, None, (Second(0), Second(5 * 3_600)), &time_only(200)).expect("a run");
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
    for ((o, d), _, lo, hi) in &e.od {
        let got = s.od_mean[&(o.clone(), d.clone())];
        println!(
            "  {o}→{d}: {:.2} min against {:.2} to {:.2} min",
            got / 60.0,
            lo / 60.0,
            hi / 60.0
        );
    }
    let hours = link_hours(&net, &result, 80 * 60, 140 * 60);
    let mut at_capacity: HashMap<String, (f64, f64, f64, f64)> = HashMap::new();
    for (link, h) in &hours {
        let id = net.link_ids().external(link.raw()).to_string();
        let (lp_flow, q_lo, q_hi, cap) = e.links[&id];
        println!(
            "  link {id:>2}: entries {:7.1} veh/h (program {lp_flow:7.1}), capacity {cap:5.0}, standing queue {}, mean delay {:4.0} s (program {q_lo:3.0} to {q_hi:3.0} s)",
            h.entries, h.standing, h.delay
        );
        if h.standing {
            at_capacity.insert(id, (h.entries, h.delay, q_hi, cap));
        }
    }
    assert!((s.mean - e.mean.0).abs() <= 0.03 * e.mean.0, "mean trip {}", s.mean);
    assert!((s.tau_x - e.tau_x.0).abs() <= 0.03 * e.tau_x.0, "Σ τ x {}", s.tau_x);
    for ((o, d), _, lo, hi) in &e.od {
        let got = s.od_mean[&(o.clone(), d.clone())];
        assert!(got >= 0.97 * lo && got <= 1.03 * hi, "{o}→{d}: {got} against {lo} to {hi}");
    }
    assert!(!at_capacity.is_empty(), "some link has a standing queue");
    for (id, (entries, delay, q_hi, cap)) in &at_capacity {
        println!(
            "  link {id} with a standing queue: entries {entries:.1} of capacity {cap:.1} veh/h, mean delay {delay:.0} s"
        );
        assert!(*q_hi > 0.0, "link {id} queues in the run but never in the program");
        assert!((entries / cap - 1.0).abs() <= 0.02, "link {id}: entries {entries} of {cap}");
    }
}
