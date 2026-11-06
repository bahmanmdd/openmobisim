//! The nested logit (test 7 of the specification, second part): three nests of three, one and
//! two rows against the closed form of Ben-Akiva and Lerman (1985, chapter 10), sampled shares
//! by chi-square, the plain logit at μ = 1, nest ids of any size; and a mode-and-route nest by
//! hand (two car routes and a walk).

mod common;

use common::{CHI_SQUARE_999_DF5, chi_square, counts, largest_gap, one_class, rows, traveller};
use openmobisim_choice::cost::Row;
use openmobisim_choice::model::{Choice, Logit};
use openmobisim_types::mode::Mode;
use openmobisim_types::rng::RngKey;

/// The rows' costs: rows 0, 2 and 5 in the first nest, row 3 alone in the second, rows 1 and 4
/// in the third (interleaved, so that a nest is its id, never its position).
const COSTS: [f64; 6] = [0.3, 0.8, 1.1, 0.5, 0.2, 0.6];

/// Which of the three nests each row is in.
const NEST_OF: [usize; 6] = [0, 2, 0, 1, 2, 0];

/// The nested logit by its formula, written out here: per nest `S_n = Σ_{j∈n} exp(−c_j/μ)`,
/// `P(n) = S_n^μ / Σ_m S_m^μ`, `P(j) = P(n) · exp(−c_j/μ) / S_n`.
fn nested_by_formula(mu: f64) -> Vec<f64> {
    let mut s = [0.0; 3];
    for (c, &n) in COSTS.iter().zip(&NEST_OF) {
        s[n] += (-c / mu).exp();
    }
    let total: f64 = s.iter().map(|s_n| s_n.powf(mu)).sum();
    COSTS
        .iter()
        .zip(&NEST_OF)
        .map(|(c, &n)| s[n].powf(mu) / total * (-c / mu).exp() / s[n])
        .collect()
}

/// The six rows with nest ids `ids[n]` for nest `n`.
fn with_ids(attributes: &[Vec<f64>], ids: [u32; 3]) -> Vec<Row<'_>> {
    let nests: Vec<u32> = NEST_OF.iter().map(|&n| ids[n]).collect();
    rows(attributes, Mode::Car, &nests)
}

fn attributes() -> Vec<Vec<f64>> {
    COSTS.iter().map(|&c| vec![c]).collect()
}

fn model(mu: f64, seed: u64) -> Logit {
    Logit::new(one_class(&["cost"], &[1.0]), mu, RngKey::new(seed)).expect("valid")
}

#[test]
fn three_nests_take_the_closed_form_and_sample_it_at_every_scale() {
    let attributes = attributes();
    let rows = with_ids(&attributes, [0, 1, 2]);
    for mu in [0.2, 0.5, 0.9, 1.0] {
        let expected = nested_by_formula(mu);
        let p = model(mu, 21).probabilities(&traveller(0, 0, 0), &rows).expect("probabilities");
        assert!(largest_gap(&p, &expected) < 1e-12, "mu {mu}: {p:?} against {expected:?}");
        let n = counts(&model(mu, 21), &rows, 20_000, 0);
        let statistic = chi_square(&n, &expected);
        assert!(statistic < CHI_SQUARE_999_DF5, "mu {mu}: chi-square {statistic}, counts {n:?}");
    }
}

#[test]
fn at_scale_one_the_nests_vanish_into_the_plain_logit() {
    let attributes = attributes();
    let rows = with_ids(&attributes, [0, 1, 2]);
    let total: f64 = COSTS.iter().map(|c| (-c).exp()).sum();
    let plain: Vec<f64> = COSTS.iter().map(|c| (-c).exp() / total).collect();
    let p = model(1.0, 0).probabilities(&traveller(0, 0, 0), &rows).expect("probabilities");
    assert!(largest_gap(&p, &plain) < 1e-12);
    // And every row in its own nest is the plain logit at any scale.
    let own = rows_in_own_nests(&attributes);
    let p = model(0.3, 0).probabilities(&traveller(0, 0, 0), &own).expect("probabilities");
    assert!(largest_gap(&p, &plain) < 1e-12);
}

fn rows_in_own_nests(attributes: &[Vec<f64>]) -> Vec<Row<'_>> {
    rows(attributes, Mode::Car, &[10, 11, 12, 13, 14, 15])
}

#[test]
fn nest_ids_seven_and_four_billion_give_the_shares_of_ids_zero_and_one() {
    // The nest id is a u32 used as it is: ids 7 and 4 000 000 000 are two nests like 0 and 1,
    // with the same closed form and each its own draw (a shared draw would correlate the two
    // nests and move the sampled shares off the closed form).
    let attributes = attributes();
    let small = with_ids(&attributes, [0, 1, 2]);
    let large = with_ids(&attributes, [7, 4_000_000_000, 2]);
    let mu = 0.5;
    let expected = nested_by_formula(mu);
    let who = traveller(0, 0, 0);
    let (m, n) = (model(mu, 4), 20_000);
    let p_small = m.probabilities(&who, &small).expect("probabilities");
    let p_large = m.probabilities(&who, &large).expect("probabilities");
    assert_eq!(p_small, p_large);
    for rows in [&small, &large] {
        let counts = counts(&m, rows, n, 3);
        let statistic = chi_square(&counts, &expected);
        assert!(statistic < CHI_SQUARE_999_DF5, "chi-square {statistic}, counts {counts:?}");
    }
}

#[test]
fn a_mode_and_route_nest_gives_the_walk_and_the_short_car_route_their_hand_shares() {
    // Car routes of 4 km and 4.8 km at 50 km/h: 288 s and 345.6 s; walking 4 km at 4.8 km/h:
    // 3 000 s. A time coefficient of 0.03 per minute is 0.0005 per second: costs 0.144, 0.1728
    // and 1.5. The cars in one nest, the walk in another. By hand, with
    // I_car = ln(e^(−0.144/μ) + e^(−0.1728/μ)):
    //   P(short | car) = 1 / (1 + e^(−0.0288/μ)),   P(walk) = e^−1.5 / (e^−1.5 + e^(μ I_car)).
    //   μ = 0.5: P(walk) = 0.155 988 883 368 583, P(short | car) = 0.514 396 020 008 460.
    //   μ = 1:   P(walk) = 0.115 592 090 776 084, P(short | car) = 0.507 199 502 377 275.
    let attributes = [vec![4_000.0 / (50.0 / 3.6)], vec![4_800.0 / (50.0 / 3.6)], vec![3_000.0]];
    let rows = vec![
        Row { attributes: &attributes[0], mode: Mode::Car, nest: 0 },
        Row { attributes: &attributes[1], mode: Mode::Car, nest: 0 },
        Row { attributes: &attributes[2], mode: Mode::Walk, nest: 1 },
    ];
    let cases = [
        (0.5, 0.155_988_883_368_583, 0.514_396_020_008_460),
        (1.0, 0.115_592_090_776_084, 0.507_199_502_377_275),
    ];
    for (mu, p_walk, p_short_car) in cases {
        let model =
            Logit::new(one_class(&["time_s"], &[0.03 / 60.0]), mu, RngKey::new(17)).expect("valid");
        let p = model.probabilities(&traveller(0, 0, 0), &rows).expect("probabilities");
        assert!((p[2] - p_walk).abs() < 1e-12, "mu {mu}: P(walk) {}", p[2]);
        assert!((p[0] / (p[0] + p[1]) - p_short_car).abs() < 1e-12, "mu {mu}: {p:?}");

        let n = counts(&model, &rows, 6_000, 1);
        let share = |k: u64, of: u64| {
            #[allow(clippy::cast_precision_loss, reason = "counts far below 2^53")]
            let (k, of) = (k as f64, of as f64);
            (k / of, of)
        };
        let (walk, all) = share(n[2], 6_000);
        let sigma = (p_walk * (1.0 - p_walk) / all).sqrt();
        assert!((walk - p_walk).abs() < 3.0 * sigma, "mu {mu}: walk share {walk}");
        let (short, cars) = share(n[0], n[0] + n[1]);
        let sigma = (p_short_car * (1.0 - p_short_car) / cars).sqrt();
        assert!((short - p_short_car).abs() < 3.0 * sigma, "mu {mu}: short share {short}");
    }
}
