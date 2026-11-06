//! Determinism and invariance, the one self-comparison: the choices of 10 000 travellers are
//! the same over two runs, in any order they are asked in, and on one thread or eight.

mod common;

use common::traveller;
use openmobisim_choice::cost::{Coefficients, Row};
use openmobisim_choice::model::{Choice, Logit};
use openmobisim_types::ids::{EntityId, UserClassId};
use openmobisim_types::mode::Mode;
use openmobisim_types::rng::RngKey;

/// How many travellers.
const TRAVELLERS: u32 = 10_000;

/// Trip `trip`'s attributes (time in seconds, money in euros): two car routes, a bike route
/// and a walk, varying with the trip.
fn attributes(trip: u32) -> [[f64; 2]; 4] {
    let t = f64::from(trip % 97);
    [[600.0 + t, 2.0], [640.0 + 2.0 * t, 1.0], [900.0 - t, 0.0], [1_800.0 - 3.0 * t, 0.0]]
}

/// Trip `trip`'s choice: the cars in nest 0, the bike in nest 1, the walk in nest 2; class
/// `trip % 2`, iteration 4.
fn choice(model: &Logit, trip: u32) -> usize {
    let a = attributes(trip);
    let modes = [Mode::Car, Mode::Car, Mode::Bike, Mode::Walk];
    let nests = [0, 0, 1, 2];
    let rows: Vec<Row<'_>> =
        (0..4).map(|j| Row { attributes: &a[j], mode: modes[j], nest: nests[j] }).collect();
    model.choose(&traveller(trip, trip % 2, 4), &rows).expect("a choice")
}

fn model(seed: u64) -> Logit {
    let coefficients = Coefficients::new(
        &["time_s", "money_eur"],
        2,
        &[(UserClassId::new(0), vec![0.005, 0.5]), (UserClassId::new(1), vec![0.004, 1.0])],
    )
    .expect("valid");
    Logit::new(coefficients, 0.6, RngKey::new(seed)).expect("valid")
}

/// Every traveller's choice, asked in trip order on one thread.
fn in_order(model: &Logit) -> Vec<usize> {
    (0..TRAVELLERS).map(|trip| choice(model, trip)).collect()
}

#[test]
fn two_runs_any_order_and_eight_threads_give_the_same_choices() {
    let first = in_order(&model(2_026));
    assert_eq!(first, in_order(&model(2_026)), "a second run");

    // Asked in a shuffled order: trip (7 919 i + 13) mod 10 000 at step i, a permutation since
    // 7 919 is prime to 10 000.
    let mut shuffled = vec![usize::MAX; TRAVELLERS as usize];
    let again = model(2_026);
    for i in 0..TRAVELLERS {
        let trip = (7_919 * i + 13) % TRAVELLERS;
        shuffled[trip as usize] = choice(&again, trip);
    }
    assert_eq!(first, shuffled, "a shuffled order");

    // On eight threads, each taking every eighth trip.
    let mut threaded = vec![usize::MAX; TRAVELLERS as usize];
    std::thread::scope(|s| {
        let parts: Vec<_> = (0..8)
            .map(|part| {
                let again = &again;
                s.spawn(move || {
                    (part..TRAVELLERS).step_by(8).map(|t| (t, choice(again, t))).collect::<Vec<_>>()
                })
            })
            .collect();
        for part in parts {
            for (trip, chosen) in part.join().expect("a choosing thread") {
                threaded[trip as usize] = chosen;
            }
        }
    });
    assert_eq!(first, threaded, "eight threads");

    // The draws matter: every row is taken by someone, and another seed moves some choices.
    assert!((0..4).all(|j| first.contains(&j)));
    assert_ne!(first, in_order(&model(2_027)));
}
