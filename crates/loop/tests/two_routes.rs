//! Test 3, two routes under steady demand: the dynamic user equilibrium of two parallel routes
//! with a bottleneck on the short one (Vickrey 1969), by the closed form.
//!
//! The short route takes `t1` free with a bottleneck of capacity `c1` on its first link; the
//! long one takes `t2` free with ample capacity; a constant inflow `q` (above `c1`, below the
//! sum) for `T` seconds. Theory: everyone takes the short route until its queue costs
//! `t2 − t1`, at `t* = (t2 − t1) c1 / (q − c1)`; from then on both routes are used at equal
//! times, the short one at capacity, so the trips leaving after `t*` split by capacity: `c1 / q`
//! on the short route (Wardrop). The mean trip over the hour is
//! `[t1 t* + (q − c1) t*² / (2 c1) + (T − t*) t2] / T`.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "counts of trips and seconds of the window, far below 2^53 and above zero"
)]

mod common;

use common::{
    car_link, demand, departed_in, far_from_best, link, mean_time, network, print_iterations,
    replay_queues, share_via, stream, time_only, took,
};
use openmobisim_loop::parameters::Parameters;
use openmobisim_loop::run::run;
use openmobisim_types::time::Second;

/// The hour in seconds.
const T: f64 = 3_600.0;

/// The closed form for `q_h` vehicles per hour: `(t*, mean trip, t1, t2, c1)`.
fn closed_form(q_h: f64) -> (f64, f64, f64, f64, f64) {
    let (t1, t2) = (2.0 * 2_000.0 / (50.0 / 3.6), 2.0 * 3_200.0 / (50.0 / 3.6));
    let (q, c1) = (q_h / 3_600.0, 1_200.0 / 3_600.0);
    let t_star = (t2 - t1) * c1 / (q - c1);
    let mean = (t1 * t_star + (q - c1) * t_star * t_star / (2.0 * c1) + (T - t_star) * t2) / T;
    (t_star, mean, t1, t2, c1)
}

#[test]
fn equal_times_on_both_routes_and_the_split_by_capacity_at_1800_veh_h() {
    let net = network(
        &[("O", 0.0, 0.0), ("P", 2_000.0, 100.0), ("Q", 2_000.0, -100.0), ("D", 4_000.0, 0.0)],
        &[
            ("op", "O", "P", car_link(2_000.0, 50.0, 1_200.0)),
            ("pd", "P", "D", car_link(2_000.0, 50.0, 36_000.0)),
            ("oq", "O", "Q", car_link(3_200.0, 50.0, 36_000.0)),
            ("qd", "Q", "D", car_link(3_200.0, 50.0, 36_000.0)),
        ],
    );
    let q_h = 1_800.0;
    let rows = stream(&net, "c", "O", "D", q_h, 1.0, Second(0));
    let dem = demand(&net, &rows);
    let (t_star, mean, t1, t2, c1) = closed_form(q_h);
    assert!((t_star - 345.6).abs() < 1e-9 && (mean - 452.5).abs() < 0.1, "the hand values");
    let parameters = time_only(100);
    let result = run(&net, &dem, None, (Second(0), Second(3 * 3_600)), &parameters).expect("a run");
    print_iterations(&result);
    let all = departed_in(&result, 0, 3_600);
    assert_eq!(all.len(), 1_800, "every trip finishes");
    let (op, oq) = (link(&net, "op"), link(&net, "oq"));
    let after = departed_in(&result, (t_star as u32) + 60, 3_600);
    let on = |l| after.iter().copied().filter(|&i| took(&result, i, l)).collect::<Vec<_>>();
    let (ts, tl) = (mean_time(&result, &on(op)), mean_time(&result, &on(oq)));

    // Part 1, the queue and the record: the bottleneck replayed car by car, every car followed
    // along its route; the long route runs free.
    let queue = replay_queues(&net, &result, &[op]);
    println!("part 1, queue: {}", queue.describe(&net));
    println!("part 1, queue: the long route after t* {tl:.1} s against its free flow {t2:.1} s");

    // Part 2, flows: the split by capacity after t*, c1 / q on the short route; t* from the
    // count on the long route, n_long = (q − c1)(T − t*).
    let short = share_via(&result, &after, op);
    let n_long = all.iter().filter(|&&i| took(&result, i, oq)).count();
    let t_star_sim = T - n_long as f64 / (q_h / 3_600.0 - c1);
    println!(
        "part 2, flows: short route's share after t* {short:.3} against {:.3}",
        c1 / (q_h / 3_600.0)
    );
    println!(
        "part 2, flows: t* from the long route's count {t_star_sim:.1} s against {t_star:.1} s"
    );

    // Part 3, choices: each traveller against the best of its own set.
    let _ = far_from_best(&result);

    // Mixed, following from parts 1 to 3: the mean trip, equal times on both used routes.
    let got = mean_time(&result, &all);
    println!("mixed: mean trip {got:.1} s against {mean:.1} s");
    println!("mixed: after t*, short route {ts:.1} s, long route {tl:.1} s (t1 {t1:.1} s)");

    queue.judge(&net, true);
    assert!((tl - t2).abs() <= 1e-6, "part 1, queue: the long route runs free: {tl} against {t2}");
    assert!(
        (short - c1 / (q_h / 3_600.0)).abs() <= 0.03,
        "part 2, flows: short route's share {short}"
    );
    assert!((t_star_sim - t_star).abs() <= 60.0, "part 2, flows: t* {t_star_sim} against {t_star}");
    assert!((got - mean).abs() <= 0.03 * mean, "mixed: mean trip {got} against {mean}");
    assert!((ts - tl).abs() <= 0.03 * t2, "mixed: equal times: {ts} against {tl}");
}

/// Iteration 0 puts everyone on the short route, whose queue then delays a departure at `t` by
/// `(q − c1) t / c1 = t / 2`; at iteration 1 the search finds the long route cheaper for the
/// trips departing after `t1 + t / 2 > t2`, that is `t > 345.6 s`: the departures at even
/// seconds from 346 s, `1 627 / 1 800 = 0.9039`, resolved to one 60 s bin (30 trips, 0.017).
/// At iteration 1's choice the trip departing at `t = 2 n` holds the short route, which it
/// reads at `c = 288 + n` s, and its best route (both are car routes) is the long one at
/// `b = 460.8` s for `n` from 173 to 1 799. It is offered a new choice with probability
/// `max(1/2, s)`: the step 1/2, or the share it would save, `r = (c − b) / c =
/// (n − 172.8) / (n + 288)`, when that is above the band 0.2 (from `n = 289`). `r` passes 1/2
/// at `n > 633.6`, so the probability is 1/2 for `n` from 173 to 633 and `r` from 634 on (0.779
/// at `n = 1 799`); offered, it takes the long route. A trip departing before 346 s holds its
/// best: it is offered with the plain step 1/2 and stays. The switch share is the sum of the
/// probabilities of the 1 627 over 1 800 trips, 0.566, `± √Σ p (1 − p) / 1 800` (one standard
/// deviation), judged at three; so are the shares of the found trips in thirds in order of
/// departure (`n` below 715: 0.503; below 1 257: 0.633; the rest: 0.744). The offered share is
/// the sum of the chances of being offered over all 1 800 trips, over 1 800: 1/2 for each of
/// the 634 trips with `n` below 634 (those holding their best and those within `r ≤ 1/2`),
/// and `r` for each of the 1 166 from 634 on, `Σ r = 1 166 − 460.8 Σ_{m = 922}^{2 087} 1 / m ≈
/// 1 166 − 460.8 ln(2 087.5 / 921.5) = 789.2`; so `(317 + 789.2) / 1 800 = 0.6146`, at
/// `± √Σ p (1 − p) / 1 800 = 0.0112` (one standard deviation), judged at three.
#[test]
fn at_iteration_1_the_found_and_switch_shares_are_the_hand_values() {
    let net = network(
        &[("O", 0.0, 0.0), ("P", 2_000.0, 100.0), ("Q", 2_000.0, -100.0), ("D", 4_000.0, 0.0)],
        &[
            ("op", "O", "P", car_link(2_000.0, 50.0, 1_200.0)),
            ("pd", "P", "D", car_link(2_000.0, 50.0, 36_000.0)),
            ("oq", "O", "Q", car_link(3_200.0, 50.0, 36_000.0)),
            ("qd", "Q", "D", car_link(3_200.0, 50.0, 36_000.0)),
        ],
    );
    let rows = stream(&net, "c", "O", "D", 1_800.0, 1.0, Second(0));
    let dem = demand(&net, &rows);
    let result =
        run(&net, &dem, None, (Second(0), Second(3 * 3_600)), &time_only(2)).expect("a run");
    print_iterations(&result);
    let one = &result.iterations[1];
    assert!((result.iterations[0].found_share - 1.0).abs() < 1e-12, "everyone found at 0");
    assert!((one.found_share - 1_627.0 / 1_800.0).abs() <= 0.017, "found {}", one.found_share);
    // The hand probabilities of the text above, summed over the trips `from..to`: the mean
    // and the standard deviation of the share that switches.
    let by_hand = |from: usize, to: usize| {
        let p = |n: usize| {
            if n < 173 {
                return 0.0;
            }
            let r = (n as f64 - 172.8) / (n as f64 + 288.0);
            if r > 0.2 { r.max(0.5) } else { 0.5 }
        };
        let count = (to - from) as f64;
        let mean = (from..to).map(p).sum::<f64>() / count;
        let sd = (from..to).map(|n| p(n) * (1.0 - p(n))).sum::<f64>().sqrt() / count;
        (mean, sd)
    };
    let (want, sd) = by_hand(0, 1_800);
    assert!((want - 0.566).abs() < 5e-4, "the hand value {want}");
    assert!((one.switch_share - want).abs() <= 3.0 * sd, "switch {}", one.switch_share);
    // The chance of being offered, the same as `p` but 1/2 for a trip holding its best.
    let offer = |n: usize| {
        let r = if n < 173 { 0.0 } else { (n as f64 - 172.8) / (n as f64 + 288.0) };
        if r > 0.2 { r.max(0.5) } else { 0.5 }
    };
    let offered = (0..1_800).map(offer).sum::<f64>() / 1_800.0;
    let offered_sd = (0..1_800).map(|n| offer(n) * (1.0 - offer(n))).sum::<f64>().sqrt() / 1_800.0;
    assert!((offered - 0.6146).abs() < 1e-4, "the hand value {offered}");
    println!(
        "offered at iteration 1 {:.4} against {offered:.4} ± {offered_sd:.4}",
        one.offered_share
    );
    assert!(
        (one.offered_share - offered).abs() <= 3.0 * offered_sd,
        "offered {} against {offered} ± 3 × {offered_sd}",
        one.offered_share
    );
    let long = link(&net, "oq");
    let switched = |from: usize, to: usize| {
        (from..to).filter(|&n| took(&result, n, long)).count() as f64 / (to - from) as f64
    };
    assert!(switched(0, 173) == 0.0, "a trip holding its best stays");
    for (from, to) in [(173, 715), (715, 1_257), (1_257, 1_800)] {
        let ((want, sd), got) = (by_hand(from, to), switched(from, to));
        assert!(
            (got - want).abs() <= 3.0 * sd + 1e-12,
            "trips {from} to {to}: {got} against {want}"
        );
    }
}

/// The mean and its standard error (the standard deviation over `√n`) of `v`.
fn mean_se(v: &[f64]) -> (f64, f64) {
    let n = v.len() as f64;
    let mean = v.iter().sum::<f64>() / n;
    let var = v.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0);
    (mean, (var / n).sqrt())
}

/// The first test's case over seeds 0 to 19, its parameters otherwise: each quantity it
/// compares with theory as the residual against its theoretical value, mean ± standard error
/// over the seeds.
#[test]
#[ignore = "a measurement over 20 seeds; run by hand in release with --ignored --nocapture"]
fn the_1800_veh_h_case_over_twenty_seeds() {
    let net = network(
        &[("O", 0.0, 0.0), ("P", 2_000.0, 100.0), ("Q", 2_000.0, -100.0), ("D", 4_000.0, 0.0)],
        &[
            ("op", "O", "P", car_link(2_000.0, 50.0, 1_200.0)),
            ("pd", "P", "D", car_link(2_000.0, 50.0, 36_000.0)),
            ("oq", "O", "Q", car_link(3_200.0, 50.0, 36_000.0)),
            ("qd", "Q", "D", car_link(3_200.0, 50.0, 36_000.0)),
        ],
    );
    let q_h = 1_800.0;
    let dem = demand(&net, &stream(&net, "c", "O", "D", q_h, 1.0, Second(0)));
    let (t_star, mean, _, _, c1) = closed_form(q_h);
    let (op, oq) = (link(&net, "op"), link(&net, "oq"));
    let (mut trip, mut gap, mut share, mut its) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut by_rule = 0;
    let started = std::time::Instant::now();
    for seed in 0..20 {
        let parameters = Parameters { seed, ..time_only(100) };
        let result =
            run(&net, &dem, None, (Second(0), Second(3 * 3_600)), &parameters).expect("a run");
        let all = departed_in(&result, 0, 3_600);
        trip.push(mean_time(&result, &all) - mean);
        let after = departed_in(&result, (t_star as u32) + 60, 3_600);
        share.push(share_via(&result, &after, op) - c1 / (q_h / 3_600.0));
        let on = |l| after.iter().copied().filter(|&i| took(&result, i, l)).collect::<Vec<_>>();
        gap.push(mean_time(&result, &on(op)) - mean_time(&result, &on(oq)));
        its.push(result.iterations.len() as f64);
        by_rule += usize::from(result.stopped_by_rule);
    }
    let [trip, gap, share, its] = [&trip, &gap, &share, &its].map(|v| mean_se(v));
    println!("mean trip residual {:+.2} ± {:.2} s (theory {mean:.1} s)", trip.0, trip.1);
    println!("short-route time minus long-route time {:+.2} ± {:.2} s (theory 0)", gap.0, gap.1);
    println!(
        "short route's share residual {:+.4} ± {:.4} (theory {:.3})",
        share.0,
        share.1,
        c1 / (q_h / 3_600.0)
    );
    println!(
        "iterations {:.1} ± {:.1}, {by_rule} of 20 seeds stopped by the rule; {:.0} s",
        its.0,
        its.1,
        started.elapsed().as_secs_f64()
    );
}
