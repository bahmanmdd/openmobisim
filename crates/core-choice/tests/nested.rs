//! The nested logit (M5): probabilities by hand, the flat logit at μ = 1, sampled shares that
//! follow the probabilities, draws that are the same every time, and the options.

use openmobisim_core_choice::{ChoiceBatch, ChoiceModel, Logit, NestedLogit, Options, model};
use openmobisim_core_types::rng::{RngKey, Stream, StreamRng};

const NAMES: [&str; 6] = ["time_min", "ln_path_size", "walk_min", "wait_min", "transfers", "nest"];

fn rng(seed: u64) -> StreamRng {
    StreamRng::new(RngKey::from_seed(seed), Stream::Choice)
}

/// One situation per traveller, each with the same alternatives `(identity, time_min, nest)`.
fn batch(travellers: u32, alts: &[(u32, f64, f64)]) -> ChoiceBatch {
    let mut b = ChoiceBatch::new(0, &NAMES);
    for t in 0..travellers {
        b.begin_situation(t, t);
        for &(id, time, nest) in alts {
            b.push_alternative(id, &[time, 0.0, 0.0, 0.0, 0.0, nest]);
        }
    }
    b
}

fn nested(mu: f64) -> NestedLogit {
    NestedLogit::from_options(&Options::from([("mu".into(), mu)])).expect("options")
}

/// Two car routes of 20 and 22 minutes (nest 0) and a bike of 25 (nest 1): by hand.
const ALTS: [(u32, f64, f64); 3] = [(11, 20.0, 0.0), (12, 22.0, 0.0), (21, 25.0, 1.0)];

fn by_hand(mu: f64) -> [f64; 3] {
    let v = ALTS.map(|(_, t, _)| -0.2 * t);
    let car = ((v[0] / mu).exp() + (v[1] / mu).exp()).ln();
    let bike = v[2] / mu;
    let (pc, pb) = {
        let (a, b) = ((mu * car).exp(), (mu * bike).exp());
        (a / (a + b), b / (a + b))
    };
    [pc * (v[0] / mu - car).exp(), pc * (v[1] / mu - car).exp(), pb]
}

#[test]
fn probabilities_follow_the_nested_logit_by_hand() {
    for mu in [0.3, 0.5, 0.8] {
        let p = nested(mu).probabilities(&batch(1, &ALTS)).expect("ok").expect("some");
        let hand = by_hand(mu);
        for (a, b) in p.iter().zip(hand) {
            assert!((a - b).abs() < 1e-12, "mu {mu}: {a} vs {b}");
        }
        assert!((p.iter().sum::<f64>() - 1.0).abs() < 1e-12);
    }
    // The nest makes the two car routes share: at mu 0.5 the bike keeps more than the flat logit gives it.
    let flat = Logit::default().probabilities(&batch(1, &ALTS)).unwrap().unwrap();
    let half = nested(0.5).probabilities(&batch(1, &ALTS)).unwrap().unwrap();
    assert!(half[2] > flat[2], "{} vs {}", half[2], flat[2]);
}

#[test]
fn mu_one_is_the_flat_logit() {
    let b = batch(1, &ALTS);
    let flat = Logit::default().probabilities(&b).unwrap().unwrap();
    let one = nested(1.0).probabilities(&b).unwrap().unwrap();
    for (a, c) in flat.iter().zip(&one) {
        assert!((a - c).abs() < 1e-12);
    }
}

#[test]
fn sampled_shares_follow_the_probabilities_and_the_draws_repeat() {
    let n = 40_000u32;
    let b = batch(n, &ALTS);
    let m = nested(0.5);
    let choices = m.choose(&b, &rng(7)).expect("choose");
    let p = by_hand(0.5);
    for (k, &pk) in p.iter().enumerate() {
        let count = choices.chosen.iter().filter(|&&c| c as usize == k).count();
        let share = f64::from(u32::try_from(count).expect("fits")) / f64::from(n);
        let sd = (pk * (1.0 - pk) / f64::from(n)).sqrt();
        assert!((share - pk).abs() < 5.0 * sd, "alternative {k}: {share} vs {pk}");
    }
    // The probability reported is the one of the alternative taken.
    for (s, &c) in choices.chosen.iter().enumerate().take(50) {
        assert!((choices.probability[s] - p[c as usize]).abs() < 1e-12);
    }
    assert_eq!(m.choose(&b, &rng(7)).unwrap().chosen, choices.chosen, "the same draws");
    assert_ne!(m.choose(&b, &rng(8)).unwrap().chosen, choices.chosen, "another seed");
}

#[test]
fn options_are_checked_and_the_model_is_selected_by_name() {
    assert!(NestedLogit::from_options(&Options::from([("mu".into(), 0.0)])).is_err());
    assert!(NestedLogit::from_options(&Options::from([("mu".into(), 1.2)])).is_err());
    let e =
        NestedLogit::from_options(&Options::from([("rho".into(), 0.5)])).unwrap_err().to_string();
    assert!(e.contains("rho") && e.contains("mu"), "{e}");
    let m =
        model("nested_logit", &Options::from([("beta_time_min".into(), -0.3)])).expect("by name");
    assert_eq!(m.name(), "nested_logit");
    assert!(m.descriptor().starts_with("nested_logit;mu=0.5;"), "{}", m.descriptor());
    assert!(m.required_attributes().unwrap().contains(&"nest".to_string()));
    // Without the nest attribute the model says so.
    let mut b = ChoiceBatch::new(0, &["time_min"]);
    b.begin_situation(0, 0);
    b.push_alternative(1, &[1.0]);
    assert!(nested(0.5).choose(&b, &rng(1)).is_err());
}

#[test]
fn the_logsums_are_by_hand_and_the_nested_one_is_the_flat_one_at_mu_1() {
    // S238: the expected utility of the best alternative, for accessibility.
    let v = ALTS.map(|(_, t, _)| -0.2 * t);
    let b = batch(2, &ALTS);
    let flat = Logit::default().logsums(&b).unwrap().unwrap();
    let by_hand_flat = v.iter().map(|x| x.exp()).sum::<f64>().ln();
    assert_eq!(flat.len(), 2, "one per situation");
    assert!((flat[0] - by_hand_flat).abs() < 1e-12);
    for mu in [0.3, 0.5, 0.8] {
        let car = ((v[0] / mu).exp() + (v[1] / mu).exp()).ln();
        let by_hand = ((mu * car).exp() + v[2].exp()).ln();
        let got = nested(mu).logsums(&b).unwrap().unwrap();
        assert!((got[1] - by_hand).abs() < 1e-12, "mu {mu}: {} vs {by_hand}", got[1]);
        assert!(got[0] > v[0], "above the best alternative's utility");
    }
    let one = nested(1.0).logsums(&b).unwrap().unwrap();
    assert!((one[0] - flat[0]).abs() < 1e-12);
    assert_eq!(openmobisim_core_choice::log_sum_exp(std::iter::empty()), f64::NEG_INFINITY);
}
