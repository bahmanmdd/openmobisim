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
//! Tolerances. The step of the method of successive averages at 100 iterations bounds the
//! residual at 1 800/100 veh/h, 54 s of queue over the hour on the 1 200 veh/h bottleneck; the
//! two-route equilibrium corrects itself faster (a queue past `t2 − t1` sends the next
//! re-choosers to the long route), so the test demands 3 % of a time, a sixth of that bound,
//! and 0.03 on the share. `t*` is read from the count on the long route: the reading's knots
//! are the 60 s bins, so the switching instant is resolved to one bin, 60 s.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "counts of trips and seconds of the window, far below 2^53 and above zero"
)]

mod common;

use common::{
    car_link, demand, departed_in, link, mean_time, network, print_iterations, share_via, stream,
    time_only,
};
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
    let got = mean_time(&result, &all);
    println!("mean trip {got:.1} s against {mean:.1} s");
    // The split by capacity after t*: c1 / q on the short route.
    let after = departed_in(&result, (t_star as u32) + 60, 3_600);
    let short = share_via(&result, &after, link(&net, "op"));
    println!("short route's share after t*: {short:.3} against {:.3}", c1 / (q_h / 3_600.0));

    // Equal times on both used routes, at t2.
    let on_short: Vec<usize> =
        after.iter().copied().filter(|&i| common::took(&result, i, link(&net, "op"))).collect();
    let on_long: Vec<usize> =
        after.iter().copied().filter(|&i| common::took(&result, i, link(&net, "oq"))).collect();
    let (ts, tl) = (mean_time(&result, &on_short), mean_time(&result, &on_long));
    println!("after t*: short route {ts:.1} s, long route {tl:.1} s, t2 {t2:.1} s, t1 {t1:.1} s");

    // t* from the count on the long route: n_long = (q − c1)(T − t*).
    let n_long = all.iter().filter(|&&i| common::took(&result, i, link(&net, "oq"))).count();
    let t_star_sim = T - n_long as f64 / (q_h / 3_600.0 - c1);
    println!("t* from the long route's count: {t_star_sim:.1} s against {t_star:.1} s");
    assert!((got - mean).abs() <= 0.03 * mean, "mean trip {got} against {mean}");
    assert!((short - c1 / (q_h / 3_600.0)).abs() <= 0.03, "short route's share {short}");
    assert!((ts - tl).abs() <= 0.03 * t2, "equal times: {ts} against {tl}");
    assert!((tl - t2).abs() <= 1e-6, "the long route runs free: {tl} against {t2}");
    assert!((t_star_sim - t_star).abs() <= 60.0, "t* {t_star_sim} against {t_star}");
}

/// Iteration 0 puts everyone on the short route, whose queue then delays a departure at `t` by
/// `(q − c1) t / c1 = t / 2`; at iteration 1 the search finds the long route cheaper for the
/// trips departing after `t1 + t / 2 > t2`, that is `t > 345.6 s`: the departures at even
/// seconds from 346 s, `1 627 / 1 800 = 0.9039`, resolved to one 60 s bin (30 trips, 0.017).
/// Half of them re-choose at iteration 1 (the step 1/2) and all of those switch: a binomial
/// share of `0.452 ± 0.012` (one standard deviation over 1 627 draws), judged at three.
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
    assert!((one.switch_share - 0.452).abs() <= 3.0 * 0.012, "switch {}", one.switch_share);
}
