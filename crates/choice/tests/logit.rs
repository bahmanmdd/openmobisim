//! The plain logit (test 7 of the specification, first part), the path-size logit by hand
//! (Ben-Akiva and Bierlaire 1999) on three bike routes, and two user classes over the same rows.

mod common;

use common::{CHI_SQUARE_999_DF5, chi_square, counts, largest_gap, one_class, rows, traveller};
use openmobisim_choice::cost::Coefficients;
use openmobisim_choice::model::{Choice, Deterministic, Logit};
use openmobisim_types::ids::{EntityId, UserClassId};
use openmobisim_types::mode::Mode;
use openmobisim_types::rng::RngKey;

/// The plain logit by its formula, `exp(−cost_j) / Σ_k exp(−cost_k)`, written out here.
fn logit_by_formula(costs: &[f64]) -> Vec<f64> {
    let total: f64 = costs.iter().map(|c| (-c).exp()).sum();
    costs.iter().map(|c| (-c).exp() / total).collect()
}

/// Six rows of one column `cost` weighed 1, each in its own nest.
fn six(costs: &[f64]) -> (Vec<Vec<f64>>, Vec<u32>) {
    (costs.iter().map(|&c| vec![c]).collect(), (0..6).collect())
}

#[test]
fn six_alternatives_take_the_closed_form_shares_and_sample_them() {
    // cost_j = ln(21 / (j + 1)): exp(−cost_j) = (j + 1) / 21, which sum to 1, so by hand
    // P_j = (j + 1) / 21.
    let costs: Vec<f64> = (1..=6).map(|k| (21.0 / f64::from(k)).ln()).collect();
    let by_hand: Vec<f64> = (1..=6).map(|k| f64::from(k) / 21.0).collect();
    let (attributes, nests) = six(&costs);
    let rows = rows(&attributes, Mode::Car, &nests);
    let model = Logit::new(one_class(&["cost"], &[1.0]), 1.0, RngKey::new(11)).expect("valid");
    let p = model.probabilities(&traveller(0, 0, 0), &rows).expect("probabilities");
    assert!(largest_gap(&p, &by_hand) < 1e-12, "{p:?} against {by_hand:?}");
    assert!(largest_gap(&p, &logit_by_formula(&costs)) < 1e-12);

    let n = counts(&model, &rows, 200_000, 0);
    let statistic = chi_square(&n, &by_hand);
    assert!(statistic < CHI_SQUARE_999_DF5, "chi-square {statistic} for counts {n:?}");
}

#[test]
fn arbitrary_costs_and_two_columns_take_the_formula() {
    // Time in seconds weighed 1/600 per second and money in euros weighed 0.4 per euro.
    let attributes: Vec<Vec<f64>> = [[300.0, 2.5], [420.0, 0.0], [900.0, 0.0], [240.0, 6.0]]
        .iter()
        .map(|r| r.to_vec())
        .collect();
    let costs: Vec<f64> = attributes.iter().map(|r| r[0] / 600.0 + 0.4 * r[1]).collect();
    let rows = rows(&attributes, Mode::Car, &[0, 1, 2, 3]);
    let model =
        Logit::new(one_class(&["time_s", "money_eur"], &[1.0 / 600.0, 0.4]), 1.0, RngKey::new(3))
            .expect("valid");
    let p = model.probabilities(&traveller(0, 0, 0), &rows).expect("probabilities");
    assert!(largest_gap(&p, &logit_by_formula(&costs)) < 1e-12);
    assert!((p.iter().sum::<f64>() - 1.0).abs() < 1e-12);
}

#[test]
fn a_cost_of_hours_in_seconds_neither_overflows_nor_underflows() {
    // exp(−7200) is 0 in floating point; the shares depend on the differences only, so two
    // rows 1 s apart take 1 / (1 + e^−1) and 1 / (1 + e) by hand.
    let attributes = [vec![7_200.0], vec![7_201.0]];
    let rows = rows(&attributes, Mode::Walk, &[0, 1]);
    let model = Logit::new(one_class(&["time_s"], &[1.0]), 1.0, RngKey::new(0)).expect("valid");
    let p = model.probabilities(&traveller(0, 0, 0), &rows).expect("probabilities");
    let e = 1.0_f64.exp();
    assert!(largest_gap(&p, &[e / (1.0 + e), 1.0 / (1.0 + e)]) < 1e-12, "{p:?}");
}

/// The shares of three bike routes of 600 s each, A alone and B and C sharing a fraction `phi`
/// of their length, with `beta_ln_ps` on the `ln PS` column.
fn path_size_shares(phi: f64, beta_ln_ps: f64) -> Vec<f64> {
    // The path-size factor by hand: PS_i = Σ_{a∈i} (l_a / L_i) / (routes using a). A uses its
    // links alone: PS_A = 1. B's shared part (a fraction φ of its length) is used by two
    // routes, the rest by B alone: PS_B = φ/2 + (1 − φ) = 1 − φ/2, and PS_C likewise.
    let ps = [1.0, 1.0 - phi / 2.0, 1.0 - phi / 2.0];
    let attributes: Vec<Vec<f64>> = ps.iter().map(|p: &f64| vec![600.0, p.ln()]).collect();
    let rows = rows(&attributes, Mode::Bike, &[0, 1, 2]);
    let coefficients = one_class(&["time_s", "ln_path_size"], &[0.0005, beta_ln_ps]);
    let model = Logit::new(coefficients, 1.0, RngKey::new(9)).expect("valid");
    model.probabilities(&traveller(0, 0, 0), &rows).expect("probabilities")
}

#[test]
fn path_size_logit_on_three_bike_routes_gives_the_lone_route_one_over_three_minus_phi() {
    // P_A = PS_A / (PS_A + PS_B + PS_C) = 1 / (1 + 2 − φ) = 1 / (3 − φ): 1 / 2.2 = 0.4545… at
    // φ = 0.8 and 1 / 2.5 = 0.4 at φ = 0.5; B and C split the rest equally.
    for (phi, p_a) in [(0.8, 1.0 / 2.2), (0.5, 0.4)] {
        let p = path_size_shares(phi, -1.0);
        let p_b = (1.0 - p_a) / 2.0;
        assert!(largest_gap(&p, &[p_a, p_b, p_b]) < 1e-12, "phi {phi}: {p:?}");
    }
    assert!((path_size_shares(0.8, -1.0)[0] - 0.4545).abs() < 1e-4);
    // Without the overlap term (coefficient 0) it is the plain logit over three equal routes.
    let third = 1.0 / 3.0;
    assert!(largest_gap(&path_size_shares(0.8, 0.0), &[third, third, third]) < 1e-12);
}

#[test]
fn two_user_classes_over_the_same_rows_take_each_its_own_closed_form() {
    // Row A: 600 s, 0 €. Row B: 300 s, 3 €. Class 0 weighs time only (0.01 per second): costs
    // 6 and 3, P_A = 1 / (1 + e³) = 0.047 425 873 177 566 8. Class 1 weighs money only (1 per
    // euro): costs 0 and 3, P_A = 1 / (1 + e⁻³) = 0.952 574 126 822 433 2.
    let coefficients = Coefficients::new(
        &["time_s", "money_eur"],
        2,
        &[(UserClassId::new(1), vec![0.0, 1.0]), (UserClassId::new(0), vec![0.01, 0.0])],
    )
    .expect("valid");
    let attributes = [vec![600.0, 0.0], vec![300.0, 3.0]];
    let rows = rows(&attributes, Mode::Car, &[0, 1]);
    let logit = Logit::new(coefficients.clone(), 1.0, RngKey::new(5)).expect("valid");
    let best = Deterministic::new(coefficients);
    let e3 = 3.0_f64.exp();
    for (class, p_a, least) in [(0, 1.0 / (1.0 + e3), 1), (1, e3 / (1.0 + e3), 0)] {
        let who = traveller(0, class, 0);
        let p = logit.probabilities(&who, &rows).expect("probabilities");
        assert!(largest_gap(&p, &[p_a, 1.0 - p_a]) < 1e-12, "class {class}: {p:?}");
        assert_eq!(best.choose(&who, &rows), Ok(least), "class {class}");
        // 20 000 draws of the class: the share of A within three standard deviations.
        let draws = 20_000_u32;
        let took_a =
            (0..draws).filter(|&t| logit.choose(&traveller(t, class, 0), &rows) == Ok(0)).count();
        let share = f64::from(u32::try_from(took_a).expect("small")) / f64::from(draws);
        let sigma = (p_a * (1.0 - p_a) / f64::from(draws)).sqrt();
        assert!((share - p_a).abs() < 3.0 * sigma, "class {class}: {share} against {p_a}");
    }
}
