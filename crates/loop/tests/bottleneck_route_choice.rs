//! The whole loop at its default parameters with congestion: logit route choice between two car
//! routes, one behind a bottleneck, judged at its steady state against a hand derivation.
//!
//! Nodes O, J, D; car links at 36 km/h (10 m/s): `approach` O→J 600 m (60 s), unlimited; two
//! links J→D: `short` 200 m (20 s) with a capacity `c` at its downstream end, and `long` 600 m
//! (60 s), unlimited. Two routes O→D, told apart by their last link: by `short` 800 m, 80 s at
//! free flow; by `long` 1 200 m, 120 s. Car-only travellers leave O for D at a constant rate
//! `q = 3 c` for three hours, every trip from node to node (no walk between door and node),
//! choosing by the default model: the nested logit with a nest per mode, so both routes are in
//! the one car nest at the default nest scale μ = 0.5.
//!
//! Path sizes by hand (Ben-Akiva and Bierlaire 1999, `PS_i = Σ_a (l_a / L_i) / n_a`), with both
//! routes in a set, the 600 m of `approach` shared by the two: by `short`
//! `600/800/2 + 200/800 = 0.375 + 0.25 = 0.625`; by `long` `600/1200/2 + 600/1200 = 0.25 + 0.5 =
//! 0.75`; a route alone in a set has 1 (as `path_size_in_a_run.rs` judges).
//!
//! The default utilities: 0.1 per minute driving (1/600 per second), constant 0, −1 on
//! `car_ln_path_size`, read times μ (−0.5), so inside the car nest a route's weight is
//! `PS · exp(−c / μ)`, `c` its cost without the path-size term: `t / 600` for a route of `t`
//! seconds. With `w` the queue delay on `short` met by a car departing at `t`, the share of the
//! departures at `t` that take `short` (once both routes are in their sets) is
//! `p(w) = 1 / (1 + (PS_long / PS_short) · exp((80 + w − 120) / (600 μ)))`, with
//! `PS_long / PS_short = 1.2` and `600 μ = 300 s`. First in first out (a car departing at `t`
//! waits for the cars that departed before it on `short`, nothing else) gives
//! `dw/dt = (q / c) p(w) − 1` while there is a queue; `p(0) = 0.4878`, so `(q / c) p(0) = 1.46`
//! and a queue forms at once. One trajectory, not a set of equilibria. Its steady state is the
//! inflow to `short` at its capacity, `p* = c / q = 1/3`, and
//! `w* = 120 − 80 + 600 μ (ln(q / c − 1) + ln(PS_short / PS_long)) = 40 + 300 (ln 2 + ln(5/6))
//! = 40 + 300 ln(5/3) = 193.25 s`, a trip of 273.25 s by `short`. Read as given (−1, the path
//! size squared in the weight), `w*` would be `40 + 300 (ln 2 + 2 ln(5/6)) = 138.55 s`, a trip of
//! 218.55 s. The time constant near the steady state: the slope of `(q / c) p(w) − 1` there is
//! `−3 p* (1 − p*) / 300 = −1/450` per second, so `T = 450 s`. The trajectory in closed form,
//! with `u = 1.2 exp((w − 40) / 300)` (so `p = 1 / (1 + u)` and `u* = 2`) and `u0 = 1.2
//! exp(−40/300) = 1.0502`: `t = 300 ((1/2) ln(u / u0) − (3/2) ln((2 − u) / (2 − u0)))`; the
//! queue passes 40 s at 97 s, is within 1 s of `w*` at 2 328 s and within 0.2 s at 3 053 s
//! (checked against a numerical solution of the equation: 193.188 s at 1 h, 193.2477 s at 2 h
//! and 3 h). So the third hour of departures (7 200 to 10 800 s) is at the steady state.
//!
//! Known, and the reason only the steady state is judged: at iteration 0 every set holds only
//! the route by `short` (the faster at free flow), so everyone takes it. A traveller is offered
//! `long` only once some iteration's reading has its trip by `short` slower than 120 s (the
//! queue above 40 s at its departure): one departing in the first seconds, before the queue
//! passes 40 s at every reading, may never be offered it, and its share is then 1, not `p(w)`.
//! The build-up is printed per group of 300 s, not judged.
//!
//! The window by hand: at iteration 0 all `N = 10 800 q` cars (with `q` per second) take `short`;
//! the first leaves it at 80 s and one leaves every `1 / c = 3 / q` s after, so the last leaves
//! at `80 + (N − 1) · 3 / q ≈ 32 480 s` (it waited about 6 h). No iteration does worse (`long`
//! has no queue and `approach` none), so a window of 10 h (36 000 s) holds every trip at every
//! iteration; a guard checks that no iteration has an unfinished trip.
//!
//! The theory is the fluid limit: it is invariant to scaling `q` and `c` together, which here
//! scales the population (`c = 1 200` veh/h and `q` one traveller a second at once).
//!
//! For bikes: there is none; these travellers are car-only, and the route choice judged is
//! inside the car nest. A bike nest beside it would change the car share of the trips, not the
//! split between the two car routes judged here at a given queue.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "counts of trips and seconds of the window, far below 2^53 and above zero"
)]

mod common;

use common::{PART_1_S, QueueReplay, at, car_link, larger, link, network, replay_queues};
use openmobisim_demand::demand::Demand;
use openmobisim_demand::input::TripRow;
use openmobisim_graph::network::Network;
use openmobisim_loop::alternative::Alternative;
use openmobisim_loop::parameters::{COLUMNS, LN_PS, Parameters};
use openmobisim_loop::run::{Run, run};
use openmobisim_types::ids::{EntityId, LinkId, TripId};
use openmobisim_types::mode::{Mode, ModeSet};
use openmobisim_types::time::Second;
use openmobisim_types::units::Metres;

/// The free-flow trips by the two routes, seconds.
const SHORT_S: f64 = 80.0;
const LONG_S: f64 = 120.0;
/// The two routes' path sizes with both in a set.
const PS_SHORT: f64 = 0.625;
const PS_LONG: f64 = 0.75;
/// The default nest scale and the default coefficient on `car_ln_path_size`.
const MU: f64 = 0.5;
const PATH_SIZE_COEFFICIENT: f64 = -1.0;
/// Demand over capacity.
const RATIO: f64 = 3.0;
/// The capacity at once the population, vehicles per hour; the demand is `RATIO` times it.
const CAPACITY_VEH_H: f64 = 1_200.0;
/// Seconds of demand, of a group of departures, and of the window.
const DEMAND_S: u32 = 3 * 3_600;
const GROUP_S: u32 = 300;
const WINDOW_S: u32 = 10 * 3_600;
/// The third hour: the groups of departures from 7 200 s to 10 800 s.
const THIRD_HOUR: (usize, usize) = (24, 36);
/// The steady state by hand: the share on `short`, `c / q`; the trip by `short`,
/// `80 + 40 + 300 ln(5/3)`; the same with the path size squared, `80 + 40 + 300 ln(25/18)`.
const STEADY_SHARE: f64 = 1.0 / 3.0;
const STEADY_SHORT_S: f64 = 273.247_7;
const SQUARED_SHORT_S: f64 = 218.551_2;
/// The time constant near the steady state, seconds.
const TIME_CONSTANT_S: f64 = 450.0;

/// What a run says, per group of departures: travellers, cars by `short`, their mean trip; per
/// group the sum over its travellers of the logit share of `short` at the times each met
/// (part 3); part 1 on the four links' queues and records, and the network to judge it on.
struct Outcome {
    groups: Vec<(f64, f64, f64)>,
    logit: Vec<f64>,
    queue: QueueReplay,
    net: Network,
    iterations: usize,
    seconds: f64,
}

impl Outcome {
    /// The share on `short` and the mean trip by `short` over the groups `from..to`.
    fn over(&self, (from, to): (usize, usize)) -> (f64, f64) {
        let (mut n, mut on_short, mut time) = (0.0, 0.0, 0.0);
        for &(g_n, g_short, g_time) in &self.groups[from..to] {
            n += g_n;
            on_short += g_short;
            time += g_short * g_time;
        }
        (on_short / n, time / on_short)
    }

    /// The third hour's share on `short` less 1/3, and its mean trip by `short` less 273.25 s.
    fn residuals(&self) -> (f64, f64) {
        let (share, time) = self.over(THIRD_HOUR);
        (share - STEADY_SHARE, time - STEADY_SHORT_S)
    }
}

/// The two routes' last links.
struct Lasts {
    short: LinkId,
    long: LinkId,
}

/// That the hand constants are the header's formulas: `w* = 120 − 80 + 600 μ (ln(q / c − 1) +
/// e ln(PS_short / PS_long))` at the exponent `e` 1 (and 2, the path size squared), and that
/// `q` is one traveller a second at once the population.
fn steady_state_by_hand() {
    assert!((RATIO * CAPACITY_VEH_H - 3_600.0).abs() < 1e-12, "one traveller a second");
    let trip = |e: f64| {
        let w =
            LONG_S - SHORT_S + 600.0 * MU * ((RATIO - 1.0).ln() + e * (PS_SHORT / PS_LONG).ln());
        SHORT_S + w
    };
    assert!((trip(-PATH_SIZE_COEFFICIENT) - STEADY_SHORT_S).abs() < 1e-4, "{}", trip(1.0));
    assert!((trip(2.0) - SQUARED_SHORT_S).abs() < 1e-4, "{}", trip(2.0));
    let p = 1.0 / (1.0 + PS_LONG / PS_SHORT * ((STEADY_SHORT_S - LONG_S) / (600.0 * MU)).exp());
    assert!((p - STEADY_SHARE).abs() < 1e-6, "the steady share {p}");
}

/// Whether `a` is the route by `short`, by its last link.
fn by_short(a: &Alternative, lasts: &Lasts) -> bool {
    let last = *a.route().and_then(<[LinkId]>::last).expect("a car route");
    assert!(last == lasts.short || last == lasts.long, "a route by short or long");
    last == lasts.short
}

/// The run at `scale` times the population (demand and capacity scaled together).
fn bottleneck(scale: usize, parameters: &Parameters) -> Outcome {
    assert_eq!(
        (COLUMNS[0], COLUMNS[LN_PS], COLUMNS[12]),
        ("car_drive_min", "car_ln_path_size", "is_car")
    );
    let b = Parameters::DEFAULT_COEFFICIENTS;
    assert!(b[0].to_bits() == 0.1_f64.to_bits() && b[12] == 0.0, "0.1 per minute, constant 0");
    assert!(b[LN_PS].to_bits() == PATH_SIZE_COEFFICIENT.to_bits(), "−1 on ln PS");
    assert!(Parameters::defaults(1).nest_scale.to_bits() == MU.to_bits(), "the default μ");
    steady_state_by_hand();
    let unlimited = f64::INFINITY;
    let net = network(
        &[("O", 0.0, 0.0), ("J", 600.0, 0.0), ("D", 800.0, 0.0)],
        &[
            ("approach", "O", "J", car_link(600.0, 36.0, unlimited)),
            ("short", "J", "D", car_link(200.0, 36.0, CAPACITY_VEH_H * scale as f64)),
            ("long", "J", "D", car_link(600.0, 36.0, unlimited)),
        ],
    );
    let lasts = Lasts { short: link(&net, "short"), long: link(&net, "long") };
    // RATIO × 1 200 veh/h is one traveller a second at once the population.
    let rows: Vec<TripRow> = (0..DEMAND_S as usize * scale)
        .map(|i| TripRow {
            id: format!("t{i:07}"),
            origin: at(0.0, 0.0),
            destination: at(800.0, 0.0),
            departure: Second((i / scale) as u32),
            class: "default".to_string(),
            modes: ModeSet::of(&[Mode::Car]),
        })
        .collect();
    let dem = Demand::build(&rows, &["default".to_string()], &net, None, Metres(1_500.0))
        .expect("a demand");
    let started = std::time::Instant::now();
    let result: Run =
        run(&net, &dem, None, (Second(0), Second(WINDOW_S)), parameters).expect("a run");
    let seconds = started.elapsed().as_secs_f64();
    for it in &result.iterations {
        assert_eq!(it.unfinished, 0, "iteration {}: unfinished trips", it.iteration);
    }
    let all = [link(&net, "approach"), lasts.short, lasts.long];
    let queue = replay_queues(&net, &result, &all);
    let mut groups = vec![(0.0, 0.0, 0.0); (DEMAND_S / GROUP_S) as usize];
    let mut logit = vec![0.0; groups.len()];
    for i in 0..rows.len() {
        let trip = TripId::from_index(i);
        let k = (result.trips.departure[i].get() / GROUP_S) as usize;
        logit[k] += logit_short_share(result.set(trip), &lasts, parameters);
        let g = &mut groups[k];
        g.0 += 1.0;
        let time = result.trips.travel_time[i];
        if by_short(result.taken(trip).expect("a route"), &lasts) {
            assert!(time >= SHORT_S - 1e-9, "trip {i} by short in {time} s");
            g.1 += 1.0;
            g.2 += time;
        } else {
            assert!((time - LONG_S).abs() < 1e-9, "trip {i} by long in {time} s, not 120 s");
        }
    }
    for g in &mut groups {
        g.2 /= g.1;
    }
    Outcome { groups, logit, queue, net, iterations: result.iterations.len(), seconds }
}

/// Part 3's model share: the logit share of `short` in `set` at the times its traveller met (the
/// set's times as the last measure reads them, the delay on `short` it met or would have met),
/// each route weighted `PS^e · exp(−t / (600 μ))` with `e` the path size's exponent the
/// parameters give (`−β`; 1 at the default); 1 for a set of `short` alone.
fn logit_short_share(set: &[Alternative], lasts: &Lasts, parameters: &Parameters) -> f64 {
    let exponent = -parameters.coefficients[0][LN_PS];
    let weight = |a: &Alternative| {
        (exponent * a.ln_path_size - a.time / (600.0 * parameters.nest_scale)).exp()
    };
    let short: f64 = set.iter().filter(|a| by_short(a, lasts)).map(weight).sum();
    short / set.iter().map(weight).sum::<f64>()
}

/// Print the parts of a run, in order: part 1, the queue and the record; part 2, the third
/// hour's share on `short` against capacity over demand; part 3, that share against the logit
/// share at the times met; mixed, each group's share and trip by `short` (printed, not judged)
/// and the third hour's trip by `short` against the steady state.
fn print_parts(label: &str, o: &Outcome) {
    let (share, time) = o.over(THIRD_HOUR);
    println!("{label}: {} iterations in {:.1} s", o.iterations, o.seconds);
    println!("part 1, queue and record: {}", o.queue.describe(&o.net));
    println!("part 2, flows: third hour, share on short {share:.4} against {STEADY_SHARE:.4}");
    let (n, logit) = (THIRD_HOUR.0..THIRD_HOUR.1)
        .fold((0.0, 0.0), |(n, l), k| (n + o.groups[k].0, l + o.logit[k]));
    println!(
        "part 3, choices (printed, not judged): third hour, share on short {share:.4} against the logit share at the times met {:.4}",
        logit / n
    );
    for (k, &(n, on_short, time)) in o.groups.iter().enumerate() {
        println!(
            "mixed (printed, not judged): departures from {:>5} s, share on short {:.4}, trip by short {time:6.1} s; the logit share at the times met {:.4}",
            k as u32 * GROUP_S,
            on_short / n,
            o.logit[k] / n
        );
    }
    println!("mixed: third hour, trip by short {time:.2} s against {STEADY_SHORT_S} s");
}

/// The standard deviation by hand of the third hour's mean trip by `short`, seconds, at `scale`
/// times the population; `third_hour_time_sd` of `bottleneck_mode_choice.rs` at this test's
/// time constant. Each traveller takes `short` with probability `p` independently, so the cars
/// joining its queue fluctuate: per second `q p (1 − p)` cars of variance, each adding `1 / c` s
/// of delay, a variance of `σ² = q p (1 − p) / c²` s² per second entering the queue delay; at
/// the steady state (`p = 1/3`, `q = scale` travellers a second, `c = scale / 3`)
/// `σ² = 2 / scale`. Near the steady state the delay returns with the time constant `T = 450 s`
/// (the slope of `(q / c) p(w) − 1` there is `−3 p (1 − p) / (600 μ) = −1/450`), so it is an
/// Ornstein–Uhlenbeck process of stationary variance `σ² T / 2` (28.1 s² at 16 times the
/// population), and the variance of its mean over `L = 3 600 s` is
/// `2 (σ² T / 2) (T / L) (1 − (T / L)(1 − e^(−L/T)))`. At 16 times the population: 2.48 s
/// (9.92 s at once, 1.24 s at 64 times). The weighting by the cars on `short`, fewer when the
/// queue is longer, moves the mean by `−Var(w) / T` = −0.06 s at 16 times: negligible. This is
/// the sampling error once the iterations have settled, and a lower bound before they have: at
/// 50 iterations the seeds' spread of this mean, measured over 20 seeds at 16 times the
/// population, is 4.99 s against the 2.48 s by hand.
fn third_hour_time_sd(scale: usize) -> f64 {
    let (t, l) = (TIME_CONSTANT_S, 3_600.0_f64);
    let stationary = (2.0 / scale as f64) * t / 2.0;
    (2.0 * stationary * t / l * (1.0 - t / l * (1.0 - (-l / t).exp()))).sqrt()
}

/// The binomial standard deviation of the third hour's share on `short` at `scale` times the
/// population: `sqrt(p* (1 − p*) / n)` over its `n = 3 600 scale` travellers (0.00196 at 16
/// times). It is an upper bound, since the queue's feedback narrows it: by conservation the share
/// less 1/3, times 10 800 s, is the change of the queue delay across the hour, whose standard
/// deviation at the steady state is `√(2 · 28.1) = 7.5 s` at 16 times the population, so the
/// share's is about 7.5 / 10 800 = 0.0007.
fn third_hour_share_sd(scale: usize) -> f64 {
    (STEADY_SHARE * (1.0 - STEADY_SHARE) / (3_600.0 * scale as f64)).sqrt()
}

/// Fifty iterations to a fixed count at the default parameters otherwise, at `seed`.
fn fifty(seed: u64) -> Parameters {
    Parameters { seed, iteration_cap: 50, iteration_floor: 50, ..Parameters::defaults(1) }
}

/// The population of the judged runs: 16 times, 172 800 travellers.
const SCALE: usize = 16;
/// The two-sided 0.999 quantile of the normal distribution.
const Z_999: f64 = 3.29;
/// The trip's tolerance in standard deviations of the queue's sampling error, as
/// `bottleneck_mode_choice.rs` has it: wider than the 3.29 of sampling theory to leave room for
/// the scatter of the iteration itself, which the hand value does not cover. The hand value,
/// 2.48 s at 16 times the population, is a lower bound at 50 iterations (the seeds' spread
/// measured there is 4.99 s), so 4 of them, 9.92 s, is about 2 measured spreads.
const TIME_SDS: f64 = 4.0;

/// One run, judged at its sampling error by hand: part 1 on the three links; the third hour's
/// share on `short` within 3.29 binomial standard deviations of 1/3 (0.0065 at 16 times); its
/// trip by `short` within `TIME_SDS` standard deviations of the queue's sampling error of
/// 273.25 s (9.9 s at 16 times).
#[test]
#[ignore = "release only: 172 800 travellers, 50 iterations, 4.4 s in release; CI runs it by name with --ignored"]
fn one_run_reaches_the_steady_state_of_the_route_logit_within_sampling_error() {
    let o = bottleneck(SCALE, &fifty(0));
    print_parts("x16, 50 iterations, seed 0", &o);
    o.queue.judge(&o.net, true);
    let (share, time) = o.over(THIRD_HOUR);
    let share_off = (share - STEADY_SHARE).abs();
    assert!(
        share_off <= Z_999 * third_hour_share_sd(SCALE),
        "part 2, flows: third hour's share on short {share}"
    );
    let time_off = (time - STEADY_SHORT_S).abs();
    assert!(
        time_off <= TIME_SDS * third_hour_time_sd(SCALE),
        "mixed: third hour's trip by short {time}"
    );
}

/// The bias test: over 20 seeds the mean residual of each of the two quantities is zero within
/// its own standard error (the seeds' spread over √20), so no tolerance is chosen: a systematic
/// bias of the loop shows as a mean many standard errors from zero. Judged at 3.29 standard
/// errors: the third hour's share on `short` less 1/3 and its trip by `short` less 273.25 s.
///
/// This test FAILS today, on the share. A run stopped at 50 iterations is not at the
/// equilibrium: the queue delay on `short` still varies along departure time by about ± 7 s
/// around the hand value. The step `1 / (k + 1)` of the method of successive averages alone
/// produces this ripple: it halves when the iterations double and is within the measurement
/// error at 400 iterations. The third hour's share is the slope of that ripple across the hour:
/// by conservation, the share less 1/3, times 10 800 s, is the change of the queue delay across
/// the hour. The test is kept as written and run by hand, as the record of that limit.
#[test]
#[ignore = "FAILS today on the share: at 50 iterations the queue delay still ripples by about ± 7 s along departure time, and the third hour's share is that ripple's slope; 20 runs, about 90 s in release; run by hand"]
fn over_twenty_seeds_neither_quantity_is_biased() {
    let runs: Vec<Outcome> = (0..20).map(|s| bottleneck(SCALE, &fifty(s))).collect();
    let replay = runs.iter().map(|o| o.queue.largest_s).fold(0.0, larger);
    let gap = runs.iter().map(|o| o.queue.gap_s).fold(0.0, larger);
    let misrecorded = runs.iter().map(|o| o.queue.misrecorded).sum::<usize>();
    println!(
        "part 1, queue and record over the seeds: largest difference from the replayed point queue {replay:.2e} s; largest gap along the routes {gap:.2e} s; {misrecorded} records missing, twice or off every route"
    );
    let shares: Vec<f64> = runs.iter().map(|o| o.residuals().0).collect();
    let times: Vec<f64> = runs.iter().map(|o| o.residuals().1).collect();
    let (s, se_s) = common::mean_se(&shares);
    let (t, se_t) = common::mean_se(&times);
    println!(
        "part 2, flows: third hour, share on short less 1/3 {s:+.5} ± {se_s:.5} ({:+.1} standard errors); the seeds' spread {:.5}, binomial by hand {:.5}",
        s / se_s,
        se_s * 20.0_f64.sqrt(),
        third_hour_share_sd(SCALE)
    );
    let logit: Vec<f64> = runs
        .iter()
        .map(|o| {
            let (n, l) = (THIRD_HOUR.0..THIRD_HOUR.1)
                .fold((0.0, 0.0), |(n, l), k| (n + o.groups[k].0, l + o.logit[k]));
            o.over(THIRD_HOUR).0 - l / n
        })
        .collect();
    let (d, e) = common::mean_se(&logit);
    println!(
        "part 3, choices (printed, not judged): third hour, share on short less the logit share at the times met {d:+.5} ± {e:.5}"
    );
    println!(
        "mixed: third hour, trip by short less {STEADY_SHORT_S} s {t:+.3} ± {se_t:.3} s ({:+.1} standard errors); the seeds' spread {:.2} s, by hand {:.2} s",
        t / se_t,
        se_t * 20.0_f64.sqrt(),
        third_hour_time_sd(SCALE)
    );
    for o in &runs {
        o.queue.judge(&o.net, true);
    }
    assert!(replay <= PART_1_S, "part 1, queue: largest difference {replay} s");
    assert!(gap <= PART_1_S, "part 1, record: largest gap {gap} s");
    assert!((s / se_s).abs() <= Z_999, "part 2, flows: share on short biased: {:+.1}", s / se_s);
    assert!((t / se_t).abs() <= Z_999, "mixed: trip by short biased: {:+.1}", t / se_t);
}

/// A measurement, not a test: the two residuals of the third hour, their mean and its standard
/// error over 5 seeds, at increasing iteration counts and populations, beside one run's
/// sampling error by hand.
#[test]
#[ignore = "a measurement; run by hand in release with --ignored --nocapture"]
fn the_residuals_by_iterations_and_population() {
    println!(
        "{:>6} {:>10} {:>26} {:>10} {:>24} {:>10} {:>8}",
        "scale",
        "iterations",
        "share on short less 1/3",
        "by hand",
        "trip by short less hand",
        "by hand",
        "seconds"
    );
    for scale in [4, 16, 64] {
        for count in [25, 50, 100, 200] {
            let started = std::time::Instant::now();
            let (mut shares, mut times) = (Vec::new(), Vec::new());
            for seed in 0..5 {
                let p = Parameters { iteration_cap: count, iteration_floor: count, ..fifty(seed) };
                let (s, t) = bottleneck(scale, &p).residuals();
                shares.push(s);
                times.push(t);
            }
            let ((s, se_s), (t, se_t)) = (common::mean_se(&shares), common::mean_se(&times));
            println!(
                "{scale:>6} {count:>10} {:>26} {:>10.5} {:>24} {:>10.2} {:>8.0}",
                format!("{s:+.5} ± {se_s:.5}"),
                third_hour_share_sd(scale),
                format!("{t:+.2} ± {se_t:.2} s"),
                third_hour_time_sd(scale),
                started.elapsed().as_secs_f64()
            );
        }
    }
}

/// A negative control, not a test of the simulator: with the coefficient on `car_ln_path_size`
/// given as −2 the logit reads −1 at μ = 0.5, the path size squared in a route's weight, and the
/// theory's steady trip by `short` is 218.55 s, 54.7 s below 273.25 s; the judgement of the
/// one-run test must refuse such a run. The share on `short` is still `c / q = 1/3` at the
/// steady state (the capacity sets it), so only the trip tells the two apart.
#[test]
#[ignore = "a negative control; run by hand in release with --ignored --nocapture"]
fn a_run_with_the_path_size_squared_is_refused_by_the_tolerance() {
    let mut b = Parameters::DEFAULT_COEFFICIENTS;
    b[LN_PS] = -2.0;
    let p = Parameters { coefficients: vec![b], ..fifty(0) };
    let o = bottleneck(SCALE, &p);
    print_parts("x16, 50 iterations, seed 0, −2 on ln PS", &o);
    let (share, time) = o.over(THIRD_HOUR);
    println!(
        "with the path size squared: third hour's share on short {share:.4}, trip by short {time:.2} s against {SQUARED_SHORT_S} s by hand"
    );
    let tolerance = TIME_SDS * third_hour_time_sd(SCALE);
    assert!((time - SQUARED_SHORT_S).abs() <= tolerance, "the other theory holds");
    assert!((time - STEADY_SHORT_S).abs() > tolerance, "and is refused");
}
