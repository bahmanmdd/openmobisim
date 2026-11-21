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
//!
//! The equilibrium test is judged over seeds 0 to 4: in every seed, part 1 (the queue and the
//! record) and the long route at free flow; on the mean over the five seeds, the short route's
//! share, `t*`, the mean trip and the equal times, since one seed scatters about as much as
//! their tolerances. The two iteration-1 tests, under the least cost and under the logit, are
//! judged on seed 0 against their hand probabilities.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "counts of trips and seconds of the window, far below 2^53 and above zero"
)]

mod common;

use common::{
    SEEDS, car_link, demand, departed_in, far_from_best, link, mean_se, mean_time, network,
    print_iterations, replay_queues, seed_mean, share_via, stream, time_only, took,
};
use openmobisim_loop::parameters::{ChoiceModel, Parameters};
use openmobisim_loop::run::run;
use openmobisim_types::ids::{EntityId, TripId};
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

/// One seed of the 1 800 veh/h case, its parameters otherwise. Judged in the seed: every trip
/// finishes; part 1, the queue and the record; the long route runs free. Returned, the
/// quantities judged on their mean over the seeds: the short route's share after `t*`, `t*`
/// from the long route's count, the mean trip, the mean times on the short and on the long
/// route of the trips leaving after `t*`.
fn equilibrium_at(seed: u64) -> [f64; 5] {
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
    let (t_star, _, _, t2, c1) = closed_form(q_h);
    let parameters = Parameters { seed, ..time_only(100) };
    let started = std::time::Instant::now();
    let result = run(&net, &dem, None, (Second(0), Second(3 * 3_600)), &parameters).expect("a run");
    println!("seed {seed}: {:.1} s", started.elapsed().as_secs_f64());
    print_iterations(&result);
    let all = departed_in(&result, 0, 3_600);
    assert_eq!(all.len(), 1_800, "seed {seed}: every trip finishes");
    let (op, oq) = (link(&net, "op"), link(&net, "oq"));
    let after = departed_in(&result, (t_star as u32) + 60, 3_600);
    let on = |l| after.iter().copied().filter(|&i| took(&result, i, l)).collect::<Vec<_>>();
    let (ts, tl) = (mean_time(&result, &on(op)), mean_time(&result, &on(oq)));

    // Part 1, the queue and the record: the bottleneck replayed car by car, every car followed
    // along its route; the long route runs free.
    let queue = replay_queues(&net, &result, &[op]);
    println!("seed {seed}, part 1, queue: {}", queue.describe(&net));
    println!("seed {seed}, part 1, queue: the long route after t* {tl:.1} s against {t2:.1} s");

    // Part 3, choices: each traveller against the best of its own set.
    let _ = far_from_best(&result);

    queue.judge(&net, true);
    assert!((tl - t2).abs() <= 1e-6, "seed {seed}, part 1, queue: the long route runs free: {tl}");
    let short = share_via(&result, &after, op);
    let n_long = all.iter().filter(|&&i| took(&result, i, oq)).count();
    let t_star_sim = T - n_long as f64 / (q_h / 3_600.0 - c1);
    [short, t_star_sim, mean_time(&result, &all), ts, tl]
}

/// Part 1 and the long route's free flow in every seed; the rest on the mean over seeds 0 to 4.
#[test]
fn equal_times_on_both_routes_and_the_split_by_capacity_at_1800_veh_h() {
    let q_h = 1_800.0;
    let (t_star, mean, t1, t2, c1) = closed_form(q_h);
    assert!((t_star - 345.6).abs() < 1e-9 && (mean - 452.5).abs() < 0.1, "the hand values");
    let runs: Vec<[f64; 5]> = (0..SEEDS).map(equilibrium_at).collect();
    let column = |k: usize| runs.iter().map(|r| r[k]).collect::<Vec<_>>();

    // Part 2, flows: the split by capacity after t*, c1 / q on the short route; t* from the
    // count on the long route, n_long = (q − c1)(T − t*).
    let label =
        format!("part 2, flows: short route's share after t* against {:.3}", c1 / (q_h / 3_600.0));
    let short = seed_mean(&label, &column(0));
    let t_star_sim = seed_mean(
        &format!("part 2, flows: t* from the long route's count, s, against {t_star:.1}"),
        &column(1),
    );

    // Mixed, following from parts 1 to 3: the mean trip, equal times on both used routes.
    let got = seed_mean(&format!("mixed: mean trip, s, against {mean:.1}"), &column(2));
    let ts = seed_mean(&format!("mixed: after t*, short route, s (t1 {t1:.1})"), &column(3));
    let tl = seed_mean(&format!("mixed: after t*, long route, s (t2 {t2:.1})"), &column(4));

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
/// seconds from 346 s, `1 627 / 1 800 = 0.9039`, exactly: each traveller reads its own time on
/// the short route (the derivation is in `found_by_mode.rs`).
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
    assert!((one.found_share - 1_627.0 / 1_800.0).abs() < 1e-12, "found {}", one.found_share);
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

/// The iteration-1 case of the test above under the logit at the default parameters. Under the
/// least cost (above) a traveller whose best route would save more than 20 % of the cost of the
/// one it holds is offered a new choice at the rate `r` of that saving; under the logit it is
/// not: everyone is offered at the plain step. The two tests together pin both.
///
/// The default model: the nested logit with a nest per mode at μ = 0.5, 0.1 per minute driving
/// (1/600 per second), the car constant 0, −1 on `car_ln_path_size`. The trips start and end
/// at nodes (no walk between door and node), and the two routes share no link, so each route's
/// path size is 1 and its `ln PS` 0: a route's cost is its seconds over 600. Both routes are in
/// the one car nest, so the logit share of the long route is
/// `1 / (1 + exp((t_long − t_short) / (600 μ)))`, `600 μ = 300 s`.
///
/// Iteration 0: every set holds the short route alone (288 s against 460.8 s at free flow), so
/// everyone takes it, and, as above, the trip departing at `t = 2 n` reads it at `288 + n` s
/// (car `n` leaves the bottleneck at `144 + 3 n`). Iteration 1: the search finds the long route
/// for `n` from 173 to 1 799, the 1 627 trips whose short route reads above 460.8 s (found share
/// `1 627 / 1 800`, exactly, as above). Each of the 1 800 travellers is offered a new choice
/// with probability `1 / (1 + 1) = 1/2`, by its own draw: the offered share is a binomial share
/// of mean 1/2 and standard deviation `√(1/4 / 1 800) = 0.0118`, judged within 4 of them, as
/// `no_congestion.rs` judges its offered share. An offered traveller with `n` below 173 has
/// one route and keeps it; one with `n` from 173 takes the long route with the logit probability
/// at the times it reads, `P(n) = 1 / (1 + exp(−(288 + n − 460.8) / 300))
/// = 1 / (1 + exp(−(n − 172.8) / 300))` (0.5002 at `n = 173`, 0.9956 at `n = 1 799`). So it
/// switches with probability `p(n) = P(n) / 2`, and the switch share is
/// `Σ_{n = 173}^{1 799} p(n) / 1 800 ≈ 150 ln((1 + e^{(1 799.5 − 172.8) / 300}) /
/// (1 + e^{(172.5 − 172.8) / 300})) / 1 800 = 0.3945` (the sum itself: 0.39451), at
/// `± √Σ p (1 − p) / 1 800 = 0.0110` (one standard deviation), judged at three, as the test
/// above judges its own; so are the shares that switch in thirds in order of departure (`n` from
/// 173 to 714: 0.3501 ± 0.0204; to 1 256: 0.4653 ± 0.0214; the rest: 0.4939 ± 0.0215). With the
/// rule of the least cost the offered share would be 0.6146, 9.7 standard deviations of the
/// offered share (0.0118) above 1/2.
///
/// For bikes: these travellers are car-only; a bike holder, alone in its mode, is offered at the
/// plain step under either model.
#[test]
fn under_the_logit_at_iteration_1_everyone_is_offered_at_the_plain_step() {
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
    let parameters = Parameters { iteration_cap: 2, ..Parameters::defaults(1) };
    assert_eq!(parameters.choice, ChoiceModel::Logit, "the default model");
    assert!(parameters.nest_scale.to_bits() == 0.5_f64.to_bits(), "the default nest scale");
    let b = &parameters.coefficients[0];
    assert!(b[0].to_bits() == 0.1_f64.to_bits() && b[12] == 0.0, "0.1 per minute, constant 0");
    let started = std::time::Instant::now();
    let result = run(&net, &dem, None, (Second(0), Second(3 * 3_600)), &parameters).expect("a run");
    println!("the run {:.2} s", started.elapsed().as_secs_f64());
    print_iterations(&result);
    let one = &result.iterations[1];
    assert!((result.iterations[0].found_share - 1.0).abs() < 1e-12, "everyone found at 0");
    assert!((one.found_share - 1_627.0 / 1_800.0).abs() < 1e-12, "found {}", one.found_share);
    for i in 0..rows.len() {
        let set = result.set(TripId::from_index(i));
        let ln_ps: Vec<f64> = set.iter().map(|a| a.ln_path_size).collect();
        assert!(ln_ps.iter().all(|&x| x == 0.0), "trip {i}: path sizes 1: {ln_ps:?}");
    }
    let offered_sd = (0.25_f64 / 1_800.0).sqrt();
    println!("offered at iteration 1 {:.6} against 0.5 ± {offered_sd:.4}", one.offered_share);
    assert!(
        (one.offered_share - 0.5).abs() <= 4.0 * offered_sd,
        "offered {} against 0.5 ± 4 × {offered_sd}",
        one.offered_share
    );
    // The hand probabilities of the text above, summed over the trips `from..to`: the mean
    // and the standard deviation of the share that switches.
    let by_hand = |from: usize, to: usize| {
        let p = |n: usize| {
            if n < 173 { 0.0 } else { 0.5 / (1.0 + (-(n as f64 - 172.8) / 300.0).exp()) }
        };
        let count = (to - from) as f64;
        let mean = (from..to).map(p).sum::<f64>() / count;
        let sd = (from..to).map(|n| p(n) * (1.0 - p(n))).sum::<f64>().sqrt() / count;
        (mean, sd)
    };
    let (want, sd) = by_hand(0, 1_800);
    assert!((want - 0.3945).abs() < 5e-4, "the hand value {want}");
    println!("switched at iteration 1 {:.4} against {want:.4} ± {sd:.4}", one.switch_share);
    assert!((one.switch_share - want).abs() <= 3.0 * sd, "switch {}", one.switch_share);
    let long = link(&net, "oq");
    let switched = |from: usize, to: usize| {
        (from..to).filter(|&n| took(&result, n, long)).count() as f64 / (to - from) as f64
    };
    assert!(switched(0, 173) == 0.0, "a trip with one route keeps it");
    for (from, to) in [(173, 715), (715, 1_257), (1_257, 1_800)] {
        let ((want, sd), got) = (by_hand(from, to), switched(from, to));
        println!("trips {from} to {to}: switched {got:.4} against {want:.4} ± {sd:.4}");
        assert!(
            (got - want).abs() <= 3.0 * sd + 1e-12,
            "trips {from} to {to}: {got} against {want}"
        );
    }
}

/// The gap under the least cost, at every iteration, against the gap the test computes from the
/// run's outputs alone: each trip's set, the alternative it holds and each alternative's time.
/// This is an identity between a run's outputs, not a hand value: the reported gap is its
/// definition applied to the reported sets and costs. A
/// run returns those as they stand after its last iteration, so the case of the first test is
/// run to exactly `K` iterations for `K` from 1 to 10 and the last row of each run is judged.
/// Time is the only cost (1 per minute on each of the nine time columns, 0 on the rest) and
/// every trip starts and ends at a node, so an alternative's cost is its door-to-door time in
/// minutes, `time / 60`. The gap is `Σ (c_held − c_least) / Σ c_least` over these costs, within
/// 1e-12. The share that would change is not reported under the least cost: it is not a number
/// at every iteration (at the equilibrium the routes in use cost the same, so that share does
/// not settle; the gap is the measure).
///
/// For bikes: these travellers are car-only; the gap reads every alternative alike.
#[test]
fn under_the_least_cost_the_gap_is_the_one_over_the_sets_and_the_share_who_would_change_is_not_reported()
 {
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
    let started = std::time::Instant::now();
    for k in 1..=10_u32 {
        let parameters = Parameters { iteration_floor: k, ..time_only(k) };
        let result =
            run(&net, &dem, None, (Second(0), Second(3 * 3_600)), &parameters).expect("a run");
        assert_eq!(result.iterations.len(), k as usize, "exactly {k} iterations");
        let (mut paid, mut least) = (0.0, 0.0);
        for i in 0..rows.len() {
            let trip = TripId::from_index(i);
            let held = result.taken(trip).expect("an alternative").time / 60.0;
            paid += held;
            least += result.set(trip).iter().map(|a| a.time / 60.0).fold(f64::INFINITY, f64::min);
        }
        let last = result.iterations.last().expect("a row");
        let gap = (paid - least) / least;
        println!("iteration {}: gap {:.3e} against {gap:.3e}", last.iteration, last.gap);
        assert!((last.gap - gap).abs() <= 1e-12, "iteration {}: gap {}", last.iteration, last.gap);
        for it in &result.iterations {
            let (k, would) = (it.iteration, it.would_change_share);
            assert!(would.is_nan(), "iteration {k}: would change {would}");
        }
    }
    println!("the ten runs {:.1} s", started.elapsed().as_secs_f64());
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
