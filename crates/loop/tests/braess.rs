//! Test 4, the Braess network with capacities: the steady state of the point queue is the
//! capacity-constrained static equilibrium (Cominetti, Correa and Olver 2022, Theorems 1 and
//! 3; Braess 1968 for the paradox), a linear program solved by hand.
//!
//! Nodes s, a, b, t. Links s→a and b→t 1 km at 1 000 veh/h; a→t and s→b 10 km; a→b 1 km; all
//! at 60 km/h (1 km a minute), the ample links at 6 000 veh/h. The program minimises
//! `Σ τ·x` over route flows within the capacities. With a→b at 1 500 veh/h: f₁ (s-a-t), f₂
//! (s-b-t) and f₃ (s-a-b-t) with f₁ + f₃ ≤ 1 000, f₂ + f₃ ≤ 1 000, f₁ + f₂ + f₃ = 1 500,
//! minimising 11 f₁ + 11 f₂ + 3 f₃: f₃ = 500 and f₁ = f₂ = 500; the duals, the queue delays
//! at s→a and b→t, solve 11 + q = 3 + 2 q, so q = 8 min and every trip takes 19 min. Without
//! a→b: 750 on each route, no queue, 11 min. At 900 veh/h with a→b: all on s-a-b-t, 3 min.
//! The demand runs for three hours; the second hour's departures are the steady state.
//!
//! Tolerances, from the residual of the method of successive averages with the deterministic
//! choice on a bottleneck, measured before the tolerances were set (the rule off, the queues'
//! sum against 960 s: −4.5 % at 100 iterations, −1.5 % at 200, −1.1 % at 400, +2.5 % at
//! 800; the rule stops the run near 150, the sum then 4.1 % short): 2 % on the mean trip, 3 %
//! on a route's time (the equal-times condition, 19 min on every used route), 0.03 on a route
//! share. A single link's queue is the integral of the inflow's excess over the capacity: a
//! share residual of 0.01 is 15 veh/h on a 1 000 veh/h link, 54 s of queue over the hour read,
//! 11 % of the 8 min; so each queue alone, read from the links table, is judged at 10 % and
//! the two queues' sum at 5 %.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "counts of trips and seconds of the window, far below 2^53 and above zero"
)]

mod common;

use common::{
    car_link, demand, departed_in, link, link_cars, mean_time, network, print_iterations,
    share_via, stream, time_only,
};
use openmobisim_graph::network::Network;
use openmobisim_loop::run::{Run, run};
use openmobisim_types::time::Second;

fn braess(with_ab: bool) -> Network {
    let mut links = vec![
        ("sa", "s", "a", car_link(1_000.0, 60.0, 1_000.0)),
        ("at", "a", "t", car_link(10_000.0, 60.0, 6_000.0)),
        ("sb", "s", "b", car_link(10_000.0, 60.0, 6_000.0)),
        ("bt", "b", "t", car_link(1_000.0, 60.0, 1_000.0)),
    ];
    if with_ab {
        links.push(("ab", "a", "b", car_link(1_000.0, 60.0, 6_000.0)));
    }
    network(
        &[("s", 0.0, 0.0), ("a", 5_000.0, 500.0), ("b", 5_000.0, -500.0), ("t", 10_000.0, 0.0)],
        &links,
    )
}

fn steady(net: &Network, rate_h: f64) -> Run {
    let rows = stream(net, "c", "s", "t", rate_h, 3.0, Second(0));
    let dem = demand(net, &rows);
    let result =
        run(net, &dem, None, (Second(0), Second(4 * 3_600)), &time_only(200)).expect("a run");
    print_iterations(&result);
    result
}

#[test]
fn with_the_added_link_every_trip_takes_19_min_split_in_thirds_with_8_min_queues() {
    let net = braess(true);
    let result = steady(&net, 1_500.0);
    let hour2 = departed_in(&result, 3_600, 7_200);
    assert_eq!(hour2.len(), 1_500, "every second-hour trip finishes");
    let k = result.iterations.len() as f64;
    let (bound_s, bound_share) = (10_800.0 / k, 2.0 / k);
    println!("{k} iterations: a time within {bound_s:.1} s, a share within {bound_share:.4}");
    let mean = mean_time(&result, &hour2);
    let (ab, sb, at) = (link(&net, "ab"), link(&net, "sb"), link(&net, "at"));
    let via_ab = share_via(&result, &hour2, ab);
    let via_sb = share_via(&result, &hour2, sb);
    let via_at = share_via(&result, &hour2, at);
    let (_, sa_time) = link_cars(&result, link(&net, "sa"), 80 * 60, 140 * 60);
    let (_, bt_time) = link_cars(&result, link(&net, "bt"), 80 * 60, 140 * 60);
    let (q_sa, q_bt) = (sa_time - 60.0, bt_time - 60.0);
    let by_route: Vec<f64> = [(at, false), (sb, false), (ab, true)]
        .iter()
        .map(|&(l, _)| {
            let on: Vec<usize> =
                hour2.iter().copied().filter(|&i| common::took(&result, i, l)).collect();
            mean_time(&result, &on)
        })
        .collect();
    println!(
        "mean trip {mean:.1} s against 1 140 s; shares s-a-t {via_at:.3}, s-b-t {via_sb:.3}, s-a-b-t {via_ab:.3}"
    );
    println!(
        "route times s-a-t {:.1}, s-b-t {:.1}, s-a-b-t {:.1} s",
        by_route[0], by_route[1], by_route[2]
    );
    println!(
        "queue delays s→a {q_sa:.1} s, b→t {q_bt:.1} s against 480 s each, {:.1} s against 960 s",
        q_sa + q_bt
    );
    assert!(k >= 100.0, "{k} iterations: the step's bound would be loose");
    assert!((mean - 1_140.0).abs() <= bound_s, "mean trip {mean}");
    for t in by_route {
        assert!((t - 1_140.0).abs() <= bound_s, "a route's time {t}");
    }
    for share in [via_at, via_sb, via_ab] {
        assert!((share - 1.0 / 3.0).abs() <= bound_share, "share {share}");
    }
    assert!((q_sa - 480.0).abs() <= bound_s, "queue at s→a {q_sa}");
    assert!((q_bt - 480.0).abs() <= bound_s, "queue at b→t {q_bt}");
}

#[test]
fn without_the_added_link_every_trip_takes_11_min_with_no_queue() {
    let net = braess(false);
    let result = steady(&net, 1_500.0);
    let hour2 = departed_in(&result, 3_600, 7_200);
    let mean = mean_time(&result, &hour2);
    let (_, sa_time) = link_cars(&result, link(&net, "sa"), 80 * 60, 140 * 60);
    println!("mean trip {mean:.1} s against 660 s; s→a crossing {sa_time:.1} s against 60 s");
    assert!((mean - 660.0).abs() <= 0.02 * 660.0, "mean trip {mean}");
    assert!((sa_time - 60.0).abs() <= 0.05 * 60.0, "no queue at s→a: {sa_time}");
}

#[test]
fn at_900_veh_h_the_added_link_helps_everyone_3_min() {
    let net = braess(true);
    let result = steady(&net, 900.0);
    let hour2 = departed_in(&result, 3_600, 7_200);
    let mean = mean_time(&result, &hour2);
    let via_ab = share_via(&result, &hour2, link(&net, "ab"));
    println!("mean trip {mean:.1} s against 180 s; share s-a-b-t {via_ab:.3}");
    assert!((mean - 180.0).abs() <= 0.02 * 180.0, "mean trip {mean}");
    assert!(via_ab >= 0.97, "share {via_ab}");
}
