//! Waiting for the first of several lines, against the closed forms of uniform independent
//! phases (test 8 of the specification, its first half, and the one-line case of Osuna and
//! Newell 1972). A traveller every 10 s, from stop A to stop B, slack 0, at most one ride;
//! RAPTOR boards the first run that reaches B first.

mod common;

use common::{Rows, stop_to_stop};
use openmobisim_transit::raptor::{Journey, Raptor};
use openmobisim_types::ids::EntityId;
use openmobisim_types::units::Duration;

/// Stops: A and B.
const A: usize = 0;
const B: usize = 1;

/// The journeys of travellers at A every 10 s over `seconds` from second `from` to B.
fn travellers(rows: &Rows, from: u32, seconds: u32) -> (Vec<u32>, Vec<Journey>) {
    let tt = rows.build();
    let mut raptor = Raptor::new(&tt, Duration::ZERO);
    let times: Vec<u32> = (from..from + seconds).step_by(10).collect();
    let journeys = times
        .iter()
        .map(|&t| raptor.earliest(&stop_to_stop(A, t, B, 1)).expect("a run comes"))
        .collect();
    (times, journeys)
}

/// The share of journeys on runs of line `name`.
fn share(rows: &Rows, journeys: &[Journey], name: &str) -> f64 {
    let tt = rows.build();
    let on = journeys
        .iter()
        .filter(|j| tt.run_ids().external(j.rides[0].run.raw()).starts_with(name))
        .count();
    count(on) / count(journeys.len())
}

fn count(n: usize) -> f64 {
    f64::from(u32::try_from(n).unwrap())
}

fn mean(xs: impl Iterator<Item = f64>) -> f64 {
    let (sum, n) = xs.fold((0.0, 0.0), |(s, n), x| (s + x, n + 1.0));
    sum / n
}

#[test]
fn two_lines_of_equal_ride_wait_and_split_as_uniform_phases_say() {
    // Two lines A to B, 20 min each, every 600 s and every 870 s, the second 137 s later:
    // the first run to come is taken. With phases U1 ~ U(0, H1), U2 ~ U(0, H2), H1 <= H2, the
    // mean wait is E[min(U1, U2)] = H1/2 - H1^2/(6 H2), and line 1 is first with probability
    // P(U1 < U2) = 1 - H1/(2 H2).
    let (h1, h2): (f64, f64) = (600.0, 870.0);
    let mut rows = Rows { stops: 2, ..Rows::default() };
    rows.line("L1", &[A, B], &[1_200], 0, 600, 20_000);
    rows.line("L2", &[A, B], &[1_200], 137, 870, 20_000);
    let (_, journeys) = travellers(&rows, 0, 5 * 3_600);
    let wait = mean(journeys.iter().map(|j| j.wait().get()));
    let first = share(&rows, &journeys, "L1");
    let (wait_by_hand, first_by_hand) = (h1 / 2.0 - h1 * h1 / (6.0 * h2), 1.0 - h1 / (2.0 * h2));
    eprintln!(
        "mean wait {wait:.2} s (by hand {wait_by_hand:.2} s); line 1's share {first:.4} (by hand {first_by_hand:.4})"
    );
    // The timetable is discrete and the window finite: 5 h is 2.07 joint cycles of 600 and
    // 870 s, and the travellers' 10-s grid lands on line 1's departures. An independent
    // computation of these same timetables gives a mean wait of exactly 227.23 s and a share of
    // 0.6650; the gap to the closed forms (231.03 s, 0.6552) is the timetable's, not the code's.
    assert!((wait - 231.0).abs() <= 10.0 && (wait_by_hand - 231.0).abs() < 0.05);
    assert!((first - 0.655).abs() <= 0.02 && (first_by_hand - 0.655).abs() < 0.001);
}

/// `∫ f` over `[a, b]` by the midpoint rule in steps of 0.01 s.
fn integral(a: f64, b: f64, f: impl Fn(f64) -> f64) -> f64 {
    let mut steps = 0_u32;
    while f64::from(steps) * 0.01 < b - a - 1e-9 {
        steps += 1;
    }
    let h = (b - a) / f64::from(steps);
    (0..steps).map(|i| f(a + (f64::from(i) + 0.5) * h)).sum::<f64>() * h
}

#[test]
fn a_fast_and_a_slow_line_split_as_the_first_arrival_says() {
    // Fast: A to B in 15 min every 1 200 s. Slow: A to B in 25 min every 390 s, 137 s later.
    // The fast runs overtake slow ones, so they are patterns of their own.
    // The door time is min(X, Y), X = U1 + 900, Y = U2 + 1 500, U1 ~ U(0, 1 200),
    // U2 ~ U(0, 390); E[min(X, Y)] = ∫ P(X > z) P(Y > z) dz and the fast line's share is
    // P(X < Y) = ∫ f_X(x) P(Y > x) dx, both integrated numerically here.
    let x_after = |z: f64| ((2_100.0 - z) / 1_200.0).clamp(0.0, 1.0);
    let y_after = |z: f64| ((1_890.0 - z) / 390.0).clamp(0.0, 1.0);
    let door_by_hand = integral(0.0, 2_100.0, |z| x_after(z) * y_after(z));
    let fast_by_hand = integral(900.0, 2_100.0, |x| y_after(x) / 1_200.0);
    let mut rows = Rows { stops: 2, ..Rows::default() };
    rows.line("F", &[A, B], &[900], 0, 1_200, 22_000);
    rows.line("S", &[A, B], &[1_500], 137, 390, 22_000);
    let (times, journeys) = travellers(&rows, 0, 5 * 3_600);
    let door = mean(times.iter().zip(&journeys).map(|(&t, j)| f64::from(j.arrival.get() - t)));
    let fast = share(&rows, &journeys, "F");
    eprintln!(
        "mean door time {door:.2} s (integrated {door_by_hand:.2} s); fast share {fast:.4} (integrated {fast_by_hand:.4})"
    );
    // As above, 5 h is 1.15 joint cycles of 1 200 and 390 s: an independent computation of
    // these same timetables gives exactly 1 423.28 s and a fast share of 0.6550, against the
    // integrated 1 426.37 s and 0.6625.
    assert!((door - door_by_hand).abs() <= 10.0);
    assert!((fast - fast_by_hand).abs() <= 0.02);
}

/// Departures at 5 s past a multiple of 10, then after each of `headways`: travellers every
/// 10 s between the first departure and the last each wait their gap's remainder, which over a
/// gap of `h` averages `h/2` exactly, so the mean wait is `E[H^2]/(2 E[H])` over the gaps.
fn one_line_mean_wait(headways: &[u32]) -> (f64, f64) {
    let mut rows = Rows { stops: 2, ..Rows::default() };
    let mut t = 5;
    for (n, &h) in headways.iter().chain(&[0]).enumerate() {
        rows.runs.push((format!("R-{n:04}"), vec![(A, t, t), (B, t + 600, t + 600)]));
        t += h;
    }
    let span: u32 = headways.iter().sum();
    let (times, journeys) = travellers(&rows, 10, span);
    assert!(times.iter().all(|&t| t <= 5 + span));
    let wait = mean(journeys.iter().map(|j| j.wait().get()));
    let h = || headways.iter().map(|&h| f64::from(h));
    let osuna_newell = h().map(|h| h * h).sum::<f64>() / (2.0 * h().sum::<f64>());
    (wait, osuna_newell)
}

#[test]
fn one_line_regular_and_irregular_waits_are_osuna_and_newell() {
    let (regular, by_hand) = one_line_mean_wait(&[600; 30]);
    eprintln!("regular every 600 s: mean wait {regular:.2} s (E[H²]/(2E[H]) = {by_hand:.2} s)");
    assert!((by_hand - 300.0).abs() < 1e-9);
    assert!((regular - by_hand).abs() <= 10.0);
    // Irregular: forty gaps of 60 to 1 500 s, multiples of 10, drawn once.
    let mut draws = common::Draws::new(3);
    let gaps: Vec<u32> = (0..40).map(|_| 10 * draws.between(6, 150)).collect();
    let (irregular, by_hand) = one_line_mean_wait(&gaps);
    let mean_gap = f64::from(gaps.iter().sum::<u32>()) / 40.0;
    eprintln!(
        "irregular, mean gap {mean_gap:.1} s: mean wait {irregular:.2} s (E[H²]/(2E[H]) = {by_hand:.2} s)"
    );
    assert!(by_hand > mean_gap / 2.0 + 20.0, "the gaps are irregular enough to matter");
    assert!((irregular - by_hand).abs() <= 10.0);
}
