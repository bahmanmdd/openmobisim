//! Determinism, the one self-comparison: a run with mode choice (car, bike, walk) under the
//! nested logit on a loaded grid gives the same three tables, to the bit, on one thread and on
//! eight, and over two runs.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "counts of trips and seconds of the window, far below 2^53 and above zero"
)]

mod common;

use common::{VALUES, at};
use openmobisim_demand::demand::Demand;
use openmobisim_demand::input::TripRow;
use openmobisim_graph::network::{LinkRow, Network, NetworkBuilder};
use openmobisim_loop::parameters::Parameters;
use openmobisim_loop::run::{Run, run};
use openmobisim_types::mode::{Mode, ModeSet};
use openmobisim_types::time::Second;
use openmobisim_types::units::Metres;

/// A 10 × 10 grid of two-way streets 300 m apart, one lane at 30 km/h and 600 veh/h, open to
/// every mode.
fn grid() -> Network {
    let n = 10;
    let mut b = NetworkBuilder::new();
    for r in 0..n {
        for c in 0..n {
            b.add_node(format!("n{r}_{c}"), at(f64::from(c) * 300.0, f64::from(r) * 300.0));
        }
    }
    let row = LinkRow {
        modes: ModeSet::of(&[Mode::Car, Mode::Bike, Mode::Walk]),
        lanes: Some(1),
        car_speed_km_h: Some(30.0),
        length_m: None,
        capacity_veh_h: Some(600.0),
        signal_at_downstream_end: false,
    };
    for r in 0..n {
        for c in 0..n {
            if c + 1 < n {
                b.add_link(
                    format!("e{r}_{c}"),
                    format!("n{r}_{c}"),
                    format!("n{r}_{}", c + 1),
                    row,
                );
                b.add_link(
                    format!("w{r}_{c}"),
                    format!("n{r}_{}", c + 1),
                    format!("n{r}_{c}"),
                    row,
                );
            }
            if r + 1 < n {
                b.add_link(
                    format!("s{r}_{c}"),
                    format!("n{r}_{c}"),
                    format!("n{}_{c}", r + 1),
                    row,
                );
                b.add_link(
                    format!("x{r}_{c}"),
                    format!("n{}_{c}", r + 1),
                    format!("n{r}_{c}"),
                    row,
                );
            }
        }
    }
    b.build(VALUES).expect("a grid")
}

/// 3 000 trips with ends drawn by a linear congruential generator over the grid's extent,
/// departing over one hour, two classes, every mode open.
fn trips() -> Vec<TripRow> {
    let mut x: u64 = 12_345;
    let mut draw = move || {
        x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        (x >> 11) as f64 / (1u64 << 53) as f64
    };
    (0..3_000)
        .map(|i| TripRow {
            id: format!("t{i:05}"),
            origin: at(draw() * 2_700.0, draw() * 2_700.0),
            destination: at(draw() * 2_700.0, draw() * 2_700.0),
            departure: Second((draw() * 3_600.0) as u32),
            class: if i % 2 == 0 { "a".to_string() } else { "b".to_string() },
            modes: ModeSet::of(&[Mode::Car, Mode::Bike, Mode::Walk]),
        })
        .collect()
}

fn one_run(network: &Network, demand: &Demand) -> Run {
    let parameters = Parameters {
        iteration_cap: 12,
        seed: 0,
        nest_scale: 0.5,
        coefficients: vec![[1.0 / 600.0, 0.6, -1.0], [1.0 / 300.0, 1.0, -1.0]],
        ..Parameters::defaults(2)
    };
    run(network, demand, None, (Second(0), Second(2 * 3_600)), &parameters).expect("a run")
}

fn same(a: &Run, b: &Run) {
    assert_eq!(a.trips, b.trips);
    assert_eq!(a.iterations.len(), b.iterations.len());
    for (x, y) in a.iterations.iter().zip(&b.iterations) {
        assert_eq!(x.mean_trip_time.to_bits(), y.mean_trip_time.to_bits());
        assert_eq!(
            (x.finished, x.unfinished, x.switch_share.to_bits(), x.found_share.to_bits()),
            (y.finished, y.unfinished, y.switch_share.to_bits(), y.found_share.to_bits())
        );
        assert_eq!(x.gap.to_bits(), y.gap.to_bits());
    }
    assert_eq!(a.links.links(), b.links.links());
    assert_eq!(a.links.cars(), b.links.cars());
    assert_eq!(a.links.bikes(), b.links.bikes());
    assert_eq!(a.links.walkers(), b.links.walkers());
    assert_eq!(a.links.car_time(), b.links.car_time());
}

#[test]
fn the_same_tables_at_1_and_8_threads_and_over_two_runs() {
    let network = grid();
    let rows = trips();
    let demand =
        Demand::build(&rows, &["a".to_string(), "b".to_string()], &network, None, Metres(1_500.0))
            .expect("a demand");
    let pool = |n| rayon::ThreadPoolBuilder::new().num_threads(n).build().expect("a pool");
    let one = pool(1).install(|| one_run(&network, &demand));
    let eight = pool(8).install(|| one_run(&network, &demand));
    let again = pool(8).install(|| one_run(&network, &demand));
    common::print_iterations(&one);
    let modes: Vec<usize> = [Mode::Car, Mode::Bike, Mode::Walk]
        .iter()
        .map(|&m| one.trips.mode.iter().filter(|&&x| x == Some(m)).count())
        .collect();
    println!(
        "mode counts car, bike, walk: {modes:?}; cars loaded: {}",
        one.links.cars().iter().sum::<u32>()
    );
    assert!(modes.iter().all(|&n| n > 100), "every mode is taken by some: {modes:?}");
    assert!(one.links.car_time().iter().any(|t| t.get() > 0.0), "cars were loaded");
    same(&one, &eight);
    same(&one, &again);
}
