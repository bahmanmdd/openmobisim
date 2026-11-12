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
//! Two tiers. The regular tier, 4 500 travellers at the stopping rule: the finite population's
//! residual with the deterministic choice (measured over seeds, rule off, 300 iterations: the
//! mean −1.4 % ± 0.7, the queues −2.5 % ± 3.8 and −2.9 % ± 3.9; under the present re-choice,
//! at the rule over seeds 0 to 19, −1.31 % ± 0.25 and −2.7 % ± 1.1 and −2.6 % ± 1.2) gives 2 %
//! on the mean trip, 0.03 on a route share, 8 % on each queue delay. The convergence tier
//! (ignored in the debug suite; CI runs it in release), 72 000 travellers (demand and
//! capacities scaled together, the equilibrium invariant) to a fixed 1 000 iterations: the
//! residual vanishes with the population, not with iterations (18 000 travellers stay −0.5 %
//! on the mean from 1 000 to 10 000 iterations; 72 000 reached −0.06 % and +0.02 % over two
//! seeds, the queues within 0.31 %, under the plain method of successive averages; under the
//! loop's present re-choice seed 0 gives −0.04 % and the queues within 0.69 %), so it is
//! judged at 0.5 % on the mean, 0.005 on a share, 1 % on each queue. The cases without a
//! binding bottleneck: 2 % of the trip and 5 % of a crossing time.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "counts of trips and seconds of the window, far below 2^53 and above zero"
)]

mod common;

use common::{
    car_link, demand, departed_in, far_from_best, link, link_cars, mean_time, network,
    print_iterations, replay_queues, share_via, stream, time_only,
};
use openmobisim_graph::network::Network;
use openmobisim_loop::parameters::Parameters;
use openmobisim_loop::run::{Run, run};
use openmobisim_types::time::Second;

/// The network with every capacity times `scale`, with or without a→b.
fn braess_at(with_ab: bool, scale: f64) -> Network {
    let mut links = vec![
        ("sa", "s", "a", car_link(1_000.0, 60.0, 1_000.0 * scale)),
        ("at", "a", "t", car_link(10_000.0, 60.0, 6_000.0 * scale)),
        ("sb", "s", "b", car_link(10_000.0, 60.0, 6_000.0 * scale)),
        ("bt", "b", "t", car_link(1_000.0, 60.0, 1_000.0 * scale)),
    ];
    if with_ab {
        links.push(("ab", "a", "b", car_link(1_000.0, 60.0, 6_000.0 * scale)));
    }
    network(
        &[("s", 0.0, 0.0), ("a", 5_000.0, 500.0), ("b", 5_000.0, -500.0), ("t", 10_000.0, 0.0)],
        &links,
    )
}

fn braess(with_ab: bool) -> Network {
    braess_at(with_ab, 1.0)
}

/// Three hours of `rate_h` veh/h from s to t, run with `parameters`.
fn steady_with(net: &Network, rate_h: f64, parameters: &Parameters) -> Run {
    let rows = stream(net, "c", "s", "t", rate_h, 3.0, Second(0));
    let dem = demand(net, &rows);
    let started = std::time::Instant::now();
    let result = run(net, &dem, None, (Second(0), Second(4 * 3_600)), parameters).expect("a run");
    println!(
        "{} trips, {} iterations in {:.1} s",
        rows.len(),
        result.iterations.len(),
        started.elapsed().as_secs_f64()
    );
    result
}

fn steady(net: &Network, rate_h: f64) -> Run {
    let result = steady_with(net, rate_h, &time_only(200));
    print_iterations(&result);
    result
}

/// The 19-minute case at `scale` times the population run with `parameters`, judged at
/// `mean_tol` of the trip, `share_tol` on a share and `queue_tol` of a queue.
fn nineteen_minutes(
    scale: f64,
    parameters: &Parameters,
    mean_tol: f64,
    share_tol: f64,
    queue_tol: f64,
) {
    let net = braess_at(true, scale);
    let result = steady_with(&net, 1_500.0 * scale, parameters);
    let hour2 = departed_in(&result, 3_600, 7_200);
    assert_eq!(hour2.len(), (1_500.0 * scale) as usize, "every second-hour trip finishes");
    let (ab, sb, at) = (link(&net, "ab"), link(&net, "sb"), link(&net, "at"));
    let (sa, bt) = (link(&net, "sa"), link(&net, "bt"));
    println!("{} iterations at {scale} times the population", result.iterations.len());

    // Part 1, the queue and the record: the two bottlenecks replayed car by car, every car
    // followed along its route.
    let queue = replay_queues(&net, &result, &[sa, bt]);
    println!("part 1, queue: {}", queue.describe(&net));

    // Part 2, flows: the three route shares, a third each.
    let shares = [at, sb, ab].map(|l| share_via(&result, &hour2, l));
    println!(
        "part 2, flows: shares s-a-t {:.3}, s-b-t {:.3}, s-a-b-t {:.3} against 1/3 each",
        shares[0], shares[1], shares[2]
    );

    // Part 3, choices: each traveller against the best of its own set.
    let _ = far_from_best(&result);

    // Mixed, following from parts 1 to 3: the mean trip, the route times, the queue delays.
    let mean = mean_time(&result, &hour2);
    let by_route = [at, sb, ab].map(|l| {
        let on: Vec<usize> =
            hour2.iter().copied().filter(|&i| common::took(&result, i, l)).collect();
        mean_time(&result, &on)
    });
    let (_, sa_time) = link_cars(&result, sa, 80 * 60, 140 * 60);
    let (_, bt_time) = link_cars(&result, bt, 80 * 60, 140 * 60);
    let (q_sa, q_bt) = (sa_time - 60.0, bt_time - 60.0);
    println!(
        "mixed: mean trip {mean:.1} s against 1 140 s; route times s-a-t {:.1}, s-b-t {:.1}, s-a-b-t {:.1} s",
        by_route[0], by_route[1], by_route[2]
    );
    println!(
        "mixed: queue delays s→a {q_sa:.1} s, b→t {q_bt:.1} s against 480 s each, {:.1} s against 960 s",
        q_sa + q_bt
    );

    queue.judge(&net, true);
    for share in shares {
        assert!((share - 1.0 / 3.0).abs() <= share_tol, "part 2, flows: share {share}");
    }
    assert!((mean - 1_140.0).abs() <= mean_tol * 1_140.0, "mixed: mean trip {mean}");
    assert!((q_sa - 480.0).abs() <= queue_tol * 480.0, "mixed: queue at s→a {q_sa}");
    assert!((q_bt - 480.0).abs() <= queue_tol * 480.0, "mixed: queue at b→t {q_bt}");
}

#[test]
fn with_the_added_link_every_trip_takes_19_min_split_in_thirds_with_8_min_queues() {
    nineteen_minutes(1.0, &time_only(200), 0.02, 0.03, 0.08);
}

/// The convergence tier: 72 000 travellers to a fixed 1 000 iterations.
#[test]
#[ignore = "the convergence tier: about 45 s in release, run by CI's release step"]
fn at_16_times_the_population_and_1000_iterations_within_0_5_percent_and_1_percent_on_a_queue() {
    let fixed = Parameters { iteration_floor: 1_000, ..time_only(1_000) };
    nineteen_minutes(16.0, &fixed, 0.005, 0.005, 0.01);
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

/// The mean and its standard error (the standard deviation over `√n`) of `v`.
fn mean_se(v: &[f64]) -> (f64, f64) {
    let n = v.len() as f64;
    let mean = v.iter().sum::<f64>() / n;
    let var = v.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0);
    (mean, (var / n).sqrt())
}

/// The 19-minute case of 4 500 travellers at `seed`, its regular tier's parameters otherwise:
/// the residuals against theory of the second hour's mean trip, the times of the routes
/// s-a-t, s-b-t and s-a-b-t (1 140 s each), the queue delays at s→a and b→t (480 s each),
/// then the iterations and 1 when the rule stopped the run.
fn nineteen_minutes_at(seed: u64) -> [f64; 8] {
    let net = braess_at(true, 1.0);
    let result = steady_with(&net, 1_500.0, &Parameters { seed, ..time_only(200) });
    let hour2 = departed_in(&result, 3_600, 7_200);
    let route = |l| {
        let on: Vec<usize> =
            hour2.iter().copied().filter(|&i| common::took(&result, i, l)).collect();
        mean_time(&result, &on) - 1_140.0
    };
    let (_, sa_time) = link_cars(&result, link(&net, "sa"), 80 * 60, 140 * 60);
    let (_, bt_time) = link_cars(&result, link(&net, "bt"), 80 * 60, 140 * 60);
    [
        mean_time(&result, &hour2) - 1_140.0,
        route(link(&net, "at")),
        route(link(&net, "sb")),
        route(link(&net, "ab")),
        sa_time - 60.0 - 480.0,
        bt_time - 60.0 - 480.0,
        result.iterations.len() as f64,
        f64::from(u8::from(result.stopped_by_rule)),
    ]
}

/// The regular tier's 19-minute case over seeds 0 to 19: each quantity it compares with
/// theory as the residual against its theoretical value, mean ± standard error over the seeds.
#[test]
#[ignore = "a measurement over 20 seeds; run by hand in release with --ignored --nocapture"]
fn the_19_minute_case_over_twenty_seeds() {
    let started = std::time::Instant::now();
    let runs: Vec<[f64; 8]> = (0..20).map(nineteen_minutes_at).collect();
    let labels = [
        "mean trip residual, s",
        "route s-a-t time residual, s",
        "route s-b-t time residual, s",
        "route s-a-b-t time residual, s",
        "queue delay s→a residual, s",
        "queue delay b→t residual, s",
        "iterations",
    ];
    for (k, label) in labels.iter().enumerate() {
        let (mean, se) = mean_se(&runs.iter().map(|r| r[k]).collect::<Vec<_>>());
        println!("{label}: {mean:+.2} ± {se:.2}");
    }
    let by_rule = runs.iter().map(|r| r[7]).sum::<f64>();
    println!(
        "{by_rule:.0} of 20 seeds stopped by the rule; {:.0} s",
        started.elapsed().as_secs_f64()
    );
}
