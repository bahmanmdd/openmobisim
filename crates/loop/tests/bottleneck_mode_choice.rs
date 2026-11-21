//! The whole loop at its default parameters with congestion: logit mode choice against one
//! bottleneck, judged against a hand derivation solved by scipy (`bottleneck_reference.py`,
//! which writes `fixtures/bottleneck/reference.csv`; nothing of the simulator in it).
//!
//! Two nodes O and D. A street O→D for cars, 2 000 m at 50 km/h (144 s) with a capacity `c` at
//! its downstream end; a path O→D for bikes, 1 500 m (360 s at 15 km/h), never congested.
//! Travellers leave O at a constant rate `q = 3 c` for three hours, each open to car and bike,
//! choosing by the default model (the nested logit; one alternative per mode, so the nest scale
//! plays no part). Under the logit every traveller is offered a new choice at iteration `k` at
//! the plain step of the method of successive averages, `1 / (k + 1)`, whatever it holds.
//!
//! The default utilities here. Every trip starts and ends at a node, so no walk between door
//! and node (`walk_min` 0); the street is the one car route, so its path size is 1 and
//! `car_ln_path_size` 0; money is 0. The car's cost is `0.1 · (144 + w) / 60` (0.1 per minute
//! driving, constant 0) and the bike's `0.1 · 360 / 60 = 0.6` (0.1 per minute riding, constant
//! 0). 0.1 per minute is 1/600 per second, and the two constants are equal, so the difference of
//! the costs is `(144 + w − 360) / 600`: exactly what it was when the cost was time alone at
//! 1/600 per second. What changes with the sixteen-column utility: nothing in this test's
//! theory, its fixture or its tolerances; only the coefficients the test pins (0.1 per minute
//! on driving and on riding, constants 0 for car and bike) and the negative control's way of
//! setting 1/500 per second (0.12 per minute on both). What the new utility would change, the
//! walking, waiting and transfer weights and the walk and transit constants, plays no part.
//!
//! Theory. With `w(t)` the queue delay met by a car departing at `t`, the car share of the
//! departures at `t` is `p = 1 / (1 + exp((144 + w − 360) / 600))`, and first-in-first-out
//! gives `dw/dt = (q / c) p(w) − 1`, `w(0) = 0`: one trajectory, not a set of equilibria, since
//! a car's delay depends on the earlier departures only. It rises to the steady state where the
//! car inflow equals the capacity: car share `c / q = 1/3` and
//! `w* = 360 − 144 + 600 ln(q / c − 1) = 216 + 600 ln 2 = 631.9 s`, a car trip of 775.9 s.
//! The time constant near the steady state is `600 / (1 − c / q) = 900 s`; by the third hour
//! the queue is within 0.2 s of `w*`. The fixture holds, per 300 s of departures, the car share
//! and the cars' mean trip time.
//!
//! The theory is the fluid limit: it is invariant to scaling `q` and `c` together, which here
//! scales the population.
//!
//! For bikes: the bike is the uncongested alternative; its trips take 360 s exactly and its
//! share is one less the car's.

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
use openmobisim_graph::network::{LinkRow, Network};
use openmobisim_loop::parameters::{COLUMNS, Parameters};
use openmobisim_loop::run::{Run, run};
use openmobisim_types::ids::{EntityId, TripId};
use openmobisim_types::mode::{Mode, ModeSet};
use openmobisim_types::time::Second;
use openmobisim_types::units::Metres;

/// The columns the theory reads: driving and riding minutes, the car and bike constants.
const DRIVE_RIDE_CAR_BIKE: (&str, &str, &str, &str) =
    ("car_drive_min", "bike_ride_min", "is_car", "is_bike");
const CAR_S: f64 = 144.0;
const BIKE_S: f64 = 360.0;
/// Demand over capacity.
const RATIO: f64 = 3.0;
/// The capacity at once the population, vehicles per hour; the demand is `RATIO` times it.
const CAPACITY_VEH_H: f64 = 1_200.0;
/// Seconds of demand, and of a group of departures.
const DEMAND_S: u32 = 3 * 3_600;
const GROUP_S: u32 = 300;
/// The steady state by hand.
const STEADY_SHARE: f64 = 1.0 / 3.0;
const STEADY_CAR_S: f64 = 775.888;

/// The scipy fixture: per group of departures, the car share and the cars' mean trip time.
fn reference() -> Vec<(f64, f64)> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/bottleneck/reference.csv");
    let text = std::fs::read_to_string(path).expect("the fixture");
    let mut lines = text.lines();
    let constants = lines.next().expect("the constants");
    // 0.1 per minute of driving or riding, per second (bit for bit 1/600).
    let beta = 0.1_f64 / 60.0;
    assert_eq!(
        constants,
        format!("constants,{CAR_S:?},{BIKE_S:?},{beta:?},{RATIO:?},3.0,{GROUP_S}"),
        "the fixture was solved for this test's constants"
    );
    let rows: Vec<(f64, f64)> = lines
        .map(|l| {
            let f: Vec<&str> = l.split(',').collect();
            (f[2].parse().expect("a share"), f[3].parse().expect("a time"))
        })
        .collect();
    assert_eq!(rows.len(), (DEMAND_S / GROUP_S) as usize);
    rows
}

/// What a run says, per group of departures: travellers, cars, the cars' mean trip time; per
/// group the sum over its travellers of the logit car share at the times each met (part 3);
/// part 1 on the street, and the network to judge it on.
struct Outcome {
    groups: Vec<(f64, f64, f64)>,
    logit: Vec<f64>,
    queue: QueueReplay,
    queue_text: String,
    net: Network,
    iterations: usize,
    stopped_by_rule: bool,
    seconds: f64,
}

impl Outcome {
    /// The car share and the cars' mean trip time over the groups `from..to`.
    fn over(&self, from: usize, to: usize) -> (f64, f64) {
        let (mut n, mut cars, mut time) = (0.0, 0.0, 0.0);
        for &(g_n, g_cars, g_time) in &self.groups[from..to] {
            n += g_n;
            cars += g_cars;
            time += g_cars * g_time;
        }
        (cars / n, time / cars)
    }
}

/// The run at `scale` times the population (demand and capacity scaled together).
fn bottleneck(scale: usize, parameters: &Parameters) -> Outcome {
    let b = Parameters::DEFAULT_COEFFICIENTS;
    assert_eq!((COLUMNS[0], COLUMNS[1], COLUMNS[12], COLUMNS[13]), DRIVE_RIDE_CAR_BIKE);
    assert!(b[0].to_bits() == 0.1_f64.to_bits() && b[1].to_bits() == 0.1_f64.to_bits());
    assert!(b[12] == 0.0 && b[13] == 0.0, "car and bike constants 0");
    let path = LinkRow {
        modes: ModeSet::of(&[Mode::Bike]),
        lanes: None,
        car_speed_km_h: None,
        length_m: Some(1_500.0),
        capacity_veh_h: None,
        signal_at_downstream_end: false,
    };
    let street = car_link(2_000.0, 50.0, CAPACITY_VEH_H * scale as f64);
    let net = network(
        &[("O", 0.0, 0.0), ("D", 1_500.0, 0.0)],
        &[("street", "O", "D", street), ("path", "O", "D", path)],
    );
    // RATIO × 1 200 veh/h is one traveller a second at once the population.
    let rows: Vec<TripRow> = (0..DEMAND_S as usize * scale)
        .map(|i| TripRow {
            id: format!("t{i:07}"),
            origin: at(0.0, 0.0),
            destination: at(1_500.0, 0.0),
            departure: Second((i / scale) as u32),
            class: "default".to_string(),
            modes: ModeSet::of(&[Mode::Car, Mode::Bike]),
        })
        .collect();
    let dem = Demand::build(&rows, &["default".to_string()], &net, None, Metres(1_500.0))
        .expect("a demand");
    let started = std::time::Instant::now();
    // Six hours: at iteration 0 (free-flow shares) the last car waits about 2.3 h.
    let result: Run =
        run(&net, &dem, None, (Second(0), Second(6 * 3_600)), parameters).expect("a run");
    let seconds = started.elapsed().as_secs_f64();
    let queue = replay_queues(&net, &result, &[link(&net, "street")]);
    let mut groups = vec![(0.0, 0.0, 0.0); (DEMAND_S / GROUP_S) as usize];
    let mut logit = vec![0.0; groups.len()];
    for i in 0..rows.len() {
        assert!(result.trips.finished[i], "trip {i} did not finish");
        let k = (result.trips.departure[i].get() / GROUP_S) as usize;
        logit[k] += logit_car_share(&result, i);
        let g = &mut groups[k];
        g.0 += 1.0;
        match result.trips.mode[i] {
            Some(Mode::Car) => {
                g.1 += 1.0;
                g.2 += result.trips.travel_time[i];
            }
            Some(Mode::Bike) => {
                assert!((result.trips.travel_time[i] - BIKE_S).abs() < 1e-9, "a bike takes 360 s");
            }
            other => panic!("trip {i} took {other:?}"),
        }
    }
    for g in &mut groups {
        g.2 /= g.1;
    }
    Outcome {
        groups,
        logit,
        queue,
        queue_text: queue.describe(&net),
        net,
        iterations: result.iterations.len(),
        stopped_by_rule: result.stopped_by_rule,
        seconds,
    }
}

/// Part 3's model share: the logit car share of traveller `i` at the times it met, its set's
/// car and bike times as the last measure reads them (the car's at the delay it met or would
/// have met), `1 / (1 + exp((car − bike) / 600))`.
fn logit_car_share(result: &Run, i: usize) -> f64 {
    let time = |mode| {
        let set = result.set(TripId::from_index(i));
        set.iter().find(|a| a.mode == mode).expect("a car and a bike in every set").time
    };
    1.0 / (1.0 + ((time(Mode::Car) - time(Mode::Bike)) / 600.0).exp())
}

/// Print the four parts of a run, in order: part 1, the queue and the record; part 2, the third
/// hour's car share against the steady state's, capacity over demand; part 3, the car share
/// against the logit share at the times met; mixed, the car share of each group against the
/// fixture (it depends on the delay met in the build-up), and the car trip per group and in the
/// third hour against the fixture.
fn print_parts(o: &Outcome) {
    let want = reference();
    println!("part 1, queue and record: {}", o.queue_text);
    let (share3, time3) = o.over(24, 36);
    println!("part 2, flows: third hour, car share {share3:.4} against {STEADY_SHARE:.4}");
    let (mut worst, mut third_n, mut third_logit) = (0.0_f64, 0.0, 0.0);
    for (k, (&(n, cars, _), &logit)) in o.groups.iter().zip(&o.logit).enumerate() {
        worst = worst.max((cars / n - logit / n).abs());
        if (24..36).contains(&k) {
            (third_n, third_logit) = (third_n + n, third_logit + logit);
        }
    }
    println!(
        "part 3, choices (printed, not judged): third hour, car share {share3:.4} against the logit share at the times met {:.4}; the largest group difference {worst:.4}",
        third_logit / third_n
    );
    for (k, (&(n, cars, _), &(share, _))) in o.groups.iter().zip(&want).enumerate() {
        let from = k as u32 * GROUP_S;
        println!(
            "mixed: departures from {from:>5} s, car share {:.4} against {share:.4}",
            cars / n
        );
    }
    for (k, (&(_, _, time), &(_, car_s))) in o.groups.iter().zip(&want).enumerate() {
        let from = k as u32 * GROUP_S;
        println!("mixed: departures from {from:>5} s, car trip {time:7.1} s against {car_s:7.1} s");
    }
    println!("mixed: third hour, car trip {time3:.1} s against {STEADY_CAR_S} s");
}

/// Print a run against the fixture and return its residuals: the largest share difference over
/// the groups, the largest relative car-time difference over the groups, and the third hour's
/// share and car time less the steady state's.
fn residuals(label: &str, o: &Outcome, print_groups: bool) -> (f64, f64, f64, f64) {
    let want = reference();
    let (mut worst_share, mut worst_time) = (0.0_f64, 0.0_f64);
    for (k, (&(n, cars, time), &(share, car_s))) in o.groups.iter().zip(&want).enumerate() {
        let got = cars / n;
        worst_share = worst_share.max((got - share).abs());
        worst_time = worst_time.max((time - car_s).abs() / car_s);
        if print_groups {
            println!(
                "  departures from {:>5} s: car share {got:.4} against {share:.4}, car trip {time:7.1} s against {car_s:7.1} s",
                k as u32 * GROUP_S
            );
        }
    }
    let (share3, time3) = o.over(24, 36);
    println!(
        "{label}: {} iterations (rule: {}) in {:.1} s; third hour: car share {share3:.4} ({:+.4} from 1/3), car trip {time3:.1} s ({:+.2} % from {STEADY_CAR_S}); worst group: share {worst_share:.4}, car trip {:.2} %",
        o.iterations,
        o.stopped_by_rule,
        o.seconds,
        share3 - STEADY_SHARE,
        (time3 / STEADY_CAR_S - 1.0) * 100.0,
        worst_time * 100.0
    );
    (worst_share, worst_time, share3 - STEADY_SHARE, time3 / STEADY_CAR_S - 1.0)
}

/// A measurement, not a test: the residuals by population, iteration count and link bin.
#[test]
#[ignore = "a measurement; run by hand in release with --ignored --nocapture"]
fn the_residuals_by_population_iterations_and_bin() {
    let defaults = Parameters::defaults(1);
    residuals("defaults, population x1", &bottleneck(1, &defaults), true);
    for scale in [1, 4, 16] {
        residuals(&format!("defaults, x{scale}"), &bottleneck(scale, &defaults), false);
        for count in [50, 200, 800] {
            for bin in [300, 60] {
                let fixed = Parameters {
                    iteration_cap: count,
                    iteration_floor: count,
                    link_bin: bin,
                    ..Parameters::defaults(1)
                };
                let label = format!("x{scale}, {count} iterations fixed, bin {bin} s");
                residuals(&label, &bottleneck(scale, &fixed), false);
            }
        }
    }
}

/// The standard deviation by hand of the third hour's mean car trip time, seconds, at `scale`
/// times the population. Each traveller takes the car with probability `p` independently, so
/// the cars joining the queue fluctuate: per second `q p (1 − p)` cars of variance, each adding
/// `1 / c` s of delay, a variance of `σ² = q p (1 − p) / c²` s² per second entering the queue
/// delay; at the steady state (`p = 1/3`, `q = scale` travellers a second, `c = scale / 3`)
/// `σ² = 2 / scale`. Near the steady state the delay returns with the time constant
/// `T = 900 s` (the slope of `(q / c) p(w) − 1` there is `−3 p (1 − p) / 600 = −1/900`), so it
/// is an Ornstein–Uhlenbeck process of stationary variance `σ² T / 2`, and the variance of its
/// mean over `L = 3 600 s` is `2 (σ² T / 2) (T / L) (1 − (T / L)(1 − e^(−L/T)))`. At 16 times
/// the population: 4.6 s.
fn third_hour_time_sd(scale: usize) -> f64 {
    let (t, l) = (900.0_f64, 3_600.0_f64);
    let stationary = (2.0 / scale as f64) * t / 2.0;
    (2.0 * stationary * t / l * (1.0 - t / l * (1.0 - (-l / t).exp()))).sqrt()
}

/// A measurement, not a test: over 20 seeds, the third hour's residuals; their mean (a bias
/// would show here) and their spread against the sampling error by hand.
#[test]
#[ignore = "a measurement over 20 seeds; run by hand in release with --ignored --nocapture"]
fn the_third_hour_over_twenty_seeds() {
    let seeds = 20_u64;
    for (scale, count) in [
        (1, 0),
        (1, 50),
        (1, 200),
        (4, 0),
        (4, 50),
        (4, 200),
        (16, 0),
        (16, 50),
        (16, 200),
        (64, 0),
        (64, 50),
    ] {
        let (mut shares, mut times, mut its) = (Vec::new(), Vec::new(), Vec::new());
        let started = std::time::Instant::now();
        for seed in 0..seeds {
            let p = if count == 0 {
                Parameters { seed, ..Parameters::defaults(1) }
            } else {
                Parameters {
                    seed,
                    iteration_cap: count,
                    iteration_floor: count,
                    ..Parameters::defaults(1)
                }
            };
            let o = bottleneck(scale, &p);
            let (share, time) = o.over(24, 36);
            shares.push(share - STEADY_SHARE);
            times.push(time - STEADY_CAR_S);
            its.push(o.iterations);
        }
        let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
        let sd = |v: &[f64]| {
            let m = mean(v);
            (v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (v.len() as f64 - 1.0)).sqrt()
        };
        let label = if count == 0 { "the rule".to_string() } else { format!("{count} fixed") };
        println!(
            "x{scale:<2} {label:>9}: iterations {:>3} to {:>3}; car trip residual mean {:+6.2} s, sd {:5.2} s (by hand {:5.2} s; mean's error {:4.2} s); car share residual mean {:+.4}, sd {:.4} (binomial {:.4}); {:.0} s",
            its.iter().min().unwrap(),
            its.iter().max().unwrap(),
            mean(&times),
            sd(&times),
            third_hour_time_sd(scale),
            third_hour_time_sd(scale) / (seeds as f64).sqrt(),
            mean(&shares),
            sd(&shares),
            (2.0 / 9.0 / (3_600.0 * scale as f64)).sqrt(),
            started.elapsed().as_secs_f64()
        );
    }
}

/// Fifty iterations to a fixed count at the default parameters otherwise, at `seed`.
fn fifty(seed: u64) -> Parameters {
    Parameters { seed, iteration_cap: 50, iteration_floor: 50, ..Parameters::defaults(1) }
}

/// The population of the two tests: 16 times, 172 800 travellers.
const SCALE: usize = 16;
/// The two-sided 0.999 quantile of the normal distribution, and the same spread over the 36
/// groups (0.999 over all of them: 1 − 0.001 / 72 a side).
const Z_999: f64 = 3.29;
const Z_999_OVER_36: f64 = 4.0;

/// One run, judged at its sampling error by hand (`third_hour_time_sd`): each group's car share at the binomial error of its travellers, the third hour's
/// car share at the binomial error of its travellers (an upper bound: the queue's feedback
/// narrows it), the third hour's car trip at `TIME_SDS` standard deviations of the queue's
/// sampling error. Provisional tolerances: the first two are sampling theory
/// alone; the third is wider than the 3.29 of sampling theory to leave room for the scatter
/// of the iteration itself, which the hand value does not cover.
#[test]
#[ignore = "release only: 27 s in debug, 2.6 s in release; CI runs it by name with --ignored"]
fn one_run_follows_the_trajectory_and_reaches_the_steady_state_within_sampling_error() {
    const TIME_SDS: f64 = 4.0;
    let o = bottleneck(SCALE, &fifty(0));
    residuals("x16, 50 iterations, seed 0", &o, false);
    print_parts(&o);
    o.queue.judge(&o.net, true);
    for (k, (&(n, cars, _), &(share, _))) in o.groups.iter().zip(&reference()).enumerate() {
        let sd = (share * (1.0 - share) / n).sqrt();
        let got = cars / n;
        assert!((got - share).abs() <= Z_999_OVER_36 * sd, "mixed: group {k}'s car share: {got}");
    }
    let (share, time) = o.over(24, 36);
    let share_sd = (STEADY_SHARE * (1.0 - STEADY_SHARE) / (3_600.0 * SCALE as f64)).sqrt();
    let share_off = (share - STEADY_SHARE).abs();
    assert!(share_off <= Z_999 * share_sd, "part 2, flows: third hour's car share {share}");
    let time_sd = third_hour_time_sd(SCALE);
    let time_off = (time - STEADY_CAR_S).abs();
    assert!(time_off <= TIME_SDS * time_sd, "mixed: third hour's car trip {time}");
}

/// The bias test: over 20 seeds the mean residual of every quantity is zero within its own
/// standard error (the seeds' spread over √20), so no tolerance is chosen: a systematic bias
/// of the loop shows as a mean many standard errors from zero. Judged: the third hour's car
/// share and car trip at 3.29 standard errors, each group's at 4.0 (0.999 over the 36 groups).
#[test]
#[ignore = "the convergence tier: 20 runs, about 50 s in release"]
fn over_twenty_seeds_no_quantity_is_biased() {
    let seeds = 20;
    let want = reference();
    let runs: Vec<Outcome> = (0..seeds).map(|s| bottleneck(SCALE, &fifty(s))).collect();
    // The mean and its standard error of one residual over the seeds.
    let judge = |residual: &dyn Fn(&Outcome) -> f64| {
        let v: Vec<f64> = runs.iter().map(residual).collect();
        let mean = v.iter().sum::<f64>() / v.len() as f64;
        let var = v.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (v.len() as f64 - 1.0);
        (mean, (var / v.len() as f64).sqrt())
    };
    let replay = runs.iter().map(|o| o.queue.largest_s).fold(0.0, larger);
    let gap = runs.iter().map(|o| o.queue.gap_s).fold(0.0, larger);
    let misrecorded = runs.iter().map(|o| o.queue.misrecorded).sum::<usize>();
    println!(
        "part 1, queue and record over the seeds: largest difference from the replayed point queue {replay:.2e} s; largest gap along the routes {gap:.2e} s; {misrecorded} records missing, twice or off every route"
    );
    // Each group's car share and car trip (mixed: they depend on the delay met in the
    // build-up), and the third hour's (the share part 2, flows: capacity over demand; the car
    // trip mixed).
    let (mut shares_off, mut trips_off) = (Vec::new(), Vec::new());
    for share in [true, false] {
        let (part, third_part, off) = if share {
            ("mixed: car share", "part 2, flows: car share", &mut shares_off)
        } else {
            ("mixed: car trip", "mixed: car trip", &mut trips_off)
        };
        let residual = |o: &Outcome, k: usize| {
            if share {
                o.groups[k].1 / o.groups[k].0 - want[k].0
            } else {
                o.groups[k].2 - want[k].1
            }
        };
        for k in 0..want.len() {
            let (d, e) = judge(&|o| residual(o, k));
            println!(
                "{part}, departures from {:>5} s: {d:+.4} ± {e:.4} ({:+.1})",
                k as u32 * GROUP_S,
                d / e
            );
            if (d / e).abs() > Z_999_OVER_36 {
                off.push(format!("group {k}: {:+.1}", d / e));
            }
        }
        let steady = if share { STEADY_SHARE } else { STEADY_CAR_S };
        let third = |o: &Outcome| if share { o.over(24, 36).0 } else { o.over(24, 36).1 };
        let (d, e) = judge(&|o| third(o) - steady);
        println!("{third_part}, third hour: {d:+.5} ± {e:.5} ({:+.1} standard errors)", d / e);
        if (d / e).abs() > Z_999 {
            off.push(format!("third hour: {:+.1}", d / e));
        }
    }
    // Part 3, choices: the third hour's car share less the logit share at the times met.
    let (d, e) = judge(&|o| {
        let (n, logit) = (24..36).fold((0.0, 0.0), |(n, l), k| (n + o.groups[k].0, l + o.logit[k]));
        o.over(24, 36).0 - logit / n
    });
    println!(
        "part 3, choices (printed, not judged): third hour, car share less the logit share at the times met {d:+.5} ± {e:.5}"
    );
    for o in &runs {
        o.queue.judge(&o.net, true);
    }
    assert!(replay <= PART_1_S, "part 1, queue: largest difference {replay} s");
    assert!(gap <= PART_1_S, "part 1, record: largest gap {gap} s");
    assert!(
        shares_off.is_empty(),
        "car share (each group mixed, the third hour part 2, flows): biased, in standard errors: {shares_off:#?}"
    );
    assert!(trips_off.is_empty(), "mixed: biased, in standard errors: {trips_off:#?}");
}

/// A measurement, not a test: the mean car-trip residual over 20 seeds in the build-up (the
/// departures from 300 s to 4 800 s) and in the third hour, by iteration count and link bin:
/// whether the small positive residual of the build-up at 50 iterations is the iteration
/// count's or the bin's.
#[test]
#[ignore = "a measurement over 20 seeds; run by hand in release with --ignored --nocapture"]
fn the_build_up_by_iterations_and_bin() {
    let want = reference();
    for (count, bin) in [(50, 300), (50, 60), (200, 300)] {
        let (mut build_up, mut third) = (Vec::new(), Vec::new());
        for seed in 0..20 {
            let p = Parameters {
                link_bin: bin,
                iteration_cap: count,
                iteration_floor: count,
                ..fifty(seed)
            };
            let o = bottleneck(SCALE, &p);
            build_up.push((1..16).map(|k| o.groups[k].2 - want[k].1).sum::<f64>() / 15.0);
            third.push(o.over(24, 36).1 - STEADY_CAR_S);
        }
        let stats = |v: &[f64]| {
            let m = v.iter().sum::<f64>() / v.len() as f64;
            let var = v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (v.len() as f64 - 1.0);
            (m, (var / v.len() as f64).sqrt())
        };
        let ((b, be), (t, te)) = (stats(&build_up), stats(&third));
        println!(
            "x16, {count} iterations, bin {bin} s: build-up car trip {b:+.2} ± {be:.2} s; third hour {t:+.2} ± {te:.2} s"
        );
    }
}

/// A negative control, not a test of the simulator: with a time coefficient of 1/500 per second
/// (0.12 per minute of driving and of riding, 60 / 500) the theory's steady car trip is
/// 144 + 216 + 500 ln 2 = 706.6 s, 69 s below the fixture's; the judgement of the one-run test
/// must refuse such a run.
#[test]
#[ignore = "a negative control; run by hand in release with --ignored --nocapture"]
fn a_run_at_another_time_coefficient_is_refused_by_the_tolerance() {
    let mut b = Parameters::DEFAULT_COEFFICIENTS;
    (b[0], b[1]) = (60.0 / 500.0, 60.0 / 500.0);
    let p = Parameters { coefficients: vec![b], ..fifty(0) };
    let o = bottleneck(SCALE, &p);
    let (share, time) = o.over(24, 36);
    println!(
        "at 1/500 per second: third hour's car share {share:.4}, car trip {time:.1} s against 706.6 s by hand"
    );
    assert!((time - 706.6).abs() <= 4.0 * third_hour_time_sd(SCALE), "the other theory holds");
    assert!((time - STEADY_CAR_S).abs() > 4.0 * third_hour_time_sd(SCALE), "and is refused");
}
