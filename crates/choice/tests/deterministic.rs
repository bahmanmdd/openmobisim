//! The deterministic model: the least cost, a tie to the lowest index, probability 1; and what
//! both models refuse.

mod common;

use common::{one_class, rows, traveller};
use openmobisim_choice::cost::{ChoiceError, Coefficients, Row};
use openmobisim_choice::model::{Choice, Deterministic, Logit};
use openmobisim_types::ids::{EntityId, UserClassId};
use openmobisim_types::mode::Mode;
use openmobisim_types::rng::RngKey;

#[test]
fn the_least_cost_is_taken_with_probability_one_and_a_tie_goes_to_the_lowest_index() {
    // Time in seconds weighed 1, money in euros weighed 100 per euro: costs by hand
    // 500 + 100 = 600, 400 + 200 = 600, 700, 450 + 150 = 600, 900. Least 600, first at row 0.
    let attributes: Vec<Vec<f64>> =
        [[500.0, 1.0], [400.0, 2.0], [700.0, 0.0], [450.0, 1.5], [900.0, 0.0]]
            .iter()
            .map(|r| r.to_vec())
            .collect();
    let model = Deterministic::new(one_class(&["time_s", "money_eur"], &[1.0, 100.0]));
    let who = traveller(3, 0, 2);
    let all = rows(&attributes, Mode::Car, &[0, 0, 0, 0, 0]);
    assert_eq!(model.choose(&who, &all), Ok(0));
    assert_eq!(model.probabilities(&who, &all), Ok(vec![1.0, 0.0, 0.0, 0.0, 0.0]));
    // Without row 0 the tie is between rows 1 and 3 of the rest: the lower, index 0 of four.
    let rest = rows(&attributes[1..], Mode::Bike, &[0, 0, 0, 0]);
    assert_eq!(model.choose(&who, &rest), Ok(0));
    // Without rows 0 and 1, row 3 (index 1 of three) is the least, alone.
    let fewer = rows(&attributes[2..], Mode::Walk, &[0, 0, 0]);
    assert_eq!(model.choose(&who, &fewer), Ok(1));
    assert_eq!(model.probabilities(&who, &fewer), Ok(vec![0.0, 1.0, 0.0]));
    // The same for any trip and iteration: no draw is made.
    assert_eq!(model.choose(&traveller(99, 0, 40), &fewer), Ok(1));
}

#[test]
fn coefficients_that_do_not_fit_the_run_are_refused_when_built() {
    let class = UserClassId::new;
    let new = |classes, by_class: &[(UserClassId, Vec<f64>)]| {
        Coefficients::new(&["time_s", "money_eur"], classes, by_class)
    };
    let refused = new(1, &[(class(0), vec![1.0])]);
    let expected = ChoiceError::CoefficientCount { class: class(0), columns: 2, coefficients: 1 };
    assert_eq!(refused, Err(expected));
    let refused = new(1, &[(class(0), vec![1.0, f64::NAN])]);
    assert!(matches!(refused, Err(ChoiceError::NotFinite { column, .. }) if column == "money_eur"));
    let refused = new(2, &[(class(1), vec![1.0, 0.0])]);
    assert_eq!(refused, Err(ChoiceError::MissingClass(class(0))));
    let refused = new(1, &[(class(0), vec![1.0, 0.0]), (class(1), vec![1.0, 0.0])]);
    assert_eq!(refused, Err(ChoiceError::UnknownClass(class(1))));
    let refused = new(1, &[(class(0), vec![1.0, 0.0]), (class(0), vec![2.0, 0.0])]);
    assert_eq!(refused, Err(ChoiceError::UnknownClass(class(0))));
    assert!(new(0, &[]).is_ok_and(|c| c.classes() == 0));
}

#[test]
fn a_nest_scale_outside_zero_to_one_is_refused_when_built() {
    let coefficients = one_class(&["time_s"], &[1.0]);
    for mu in [0.0, -0.5, 1.000_001, f64::NAN, f64::INFINITY] {
        let refused = Logit::new(coefficients.clone(), mu, RngKey::new(0));
        assert!(matches!(refused, Err(ChoiceError::NestScale(_))), "mu {mu}");
    }
    for mu in [1e-9, 0.5, 1.0] {
        assert!(Logit::new(coefficients.clone(), mu, RngKey::new(0)).is_ok(), "mu {mu}");
    }
}

#[test]
fn no_rows_a_short_row_an_unknown_class_and_a_cost_that_is_not_finite_are_refused_at_choice() {
    let logit = Logit::new(one_class(&["time_s"], &[1.0]), 0.5, RngKey::new(0)).expect("valid");
    let best = Deterministic::new(one_class(&["time_s"], &[1.0]));
    let models: [&dyn Choice; 2] = [&logit, &best];
    let who = traveller(0, 0, 0);
    let (one, none, infinite) = ([60.0], [], [f64::INFINITY]);
    let row = |attributes| Row { attributes, mode: Mode::Car, nest: 0 };
    for model in models {
        assert_eq!(model.choose(&who, &[]), Err(ChoiceError::NoRows));
        assert_eq!(model.probabilities(&who, &[]), Err(ChoiceError::NoRows));
        let short = ChoiceError::RowLength { row: 1, columns: 1, attributes: 0 };
        assert_eq!(model.choose(&who, &[row(&one), row(&none)]), Err(short));
        let stranger = traveller(0, 1, 0);
        let unknown = Err(ChoiceError::UnknownClass(UserClassId::new(1)));
        assert_eq!(model.choose(&stranger, &[row(&one)]), unknown);
        let not_finite = Err(ChoiceError::NotFiniteCost { row: 1 });
        assert_eq!(model.choose(&who, &[row(&one), row(&infinite)]), not_finite);
    }
}
