//! The choice layer: exact probabilities by hand, sampling that follows them,
//! draws that belong to identities and not to positions, and a model added the
//! way a researcher would add one.

#![allow(
    clippy::float_cmp,
    reason = "draws and saturated probabilities are compared for exact equality on purpose"
)]

use openmobisim_core_choice::{
    ChoiceBatch, ChoiceError, ChoiceModel, Choices, DEFAULT_MODEL, Deterministic, Logit, Options,
    Registry, gumbel, model, sample_random_utility,
};
use openmobisim_core_types::rng::{RngKey, Stream, StreamRng};

const ATTRIBUTES: [&str; 3] = ["time_min", "ln_path_size", "length_km"];

fn rng(seed: u64) -> StreamRng {
    StreamRng::new(RngKey::from_seed(seed), Stream::Choice)
}

/// `(identity, time_min, path_size, length_km)`.
type Alt = (u32, f64, f64, f64);

fn batch_of(iteration: u32, situations: &[(u32, u32, Vec<Alt>)]) -> ChoiceBatch {
    let mut b = ChoiceBatch::new(iteration, &ATTRIBUTES);
    for (traveller, trip, alts) in situations {
        b.begin_situation(*traveller, *trip);
        for (id, time, ps, length) in alts {
            b.push_alternative(*id, &[*time, ps.ln(), *length]);
        }
    }
    b
}

/// Three routes: A and B tie on time but B shares half its length with the others; C is 2 minutes slower.
fn three_routes() -> Vec<Alt> {
    vec![(11, 10.0, 1.0, 5.0), (22, 10.0, 0.5, 5.0), (33, 12.0, 1.0, 6.0)]
}

fn logit() -> Logit {
    Logit::from_options(&Options::new()).expect("defaults")
}

// --- exact probabilities, by hand ---------------------------------------------------

#[test]
fn two_routes_three_minutes_apart_split_by_the_closed_form() {
    // U = -0.2 t with path size 1, and the slow route 3 minutes worse:
    // P(fast) = 1 / (1 + e^-0.6) and P(slow) = 1 / (1 + e^0.6).
    let b = batch_of(0, &[(1, 1, vec![(1, 10.0, 1.0, 5.0), (2, 13.0, 1.0, 5.0)])]);
    let p = logit().probabilities(&b).expect("probabilities");
    assert!((p[0][0] - 0.645_656_306_225_795_4).abs() < 1e-12, "{p:?}");
    assert!((p[0][1] - 0.354_343_693_774_204_6).abs() < 1e-12);
    assert!((p[0][0] + p[0][1] - 1.0).abs() < 1e-15);
}

#[test]
fn the_path_size_term_splits_the_share_of_routes_that_overlap() {
    // U = -0.2 t + ln PS: A = -2, B = -2 + ln 0.5, C = -2.4. Weights are e^U.
    let b = batch_of(0, &[(1, 1, three_routes())]);
    let p = logit().probabilities(&b).expect("probabilities");
    let expect = [0.460_761_536_910_938_5, 0.230_380_768_455_469_2, 0.308_857_694_633_592_25];
    for (got, want) in p[0].iter().zip(expect) {
        assert!((got - want).abs() < 1e-9, "{p:?}");
    }
    // Without the term the overlapping route is as likely as its twin: the logit's known flaw.
    let plain = Logit::from_options(&Options::from([("beta_ln_path_size".into(), 0.0)])).unwrap();
    let q = plain.probabilities(&b).expect("probabilities");
    assert!((q[0][0] - q[0][1]).abs() < 1e-15);
}

#[test]
fn a_huge_gap_cannot_overflow_the_logit() {
    let b = batch_of(0, &[(1, 1, vec![(1, 1.0, 1.0, 1.0), (2, 1e6, 1.0, 1.0)])]);
    let p = logit().probabilities(&b).expect("probabilities");
    assert_eq!(p[0][0], 1.0);
    assert_eq!(p[0][1], 0.0);
}

// --- deterministic -------------------------------------------------------------------

#[test]
fn deterministic_takes_the_least_time_and_the_first_of_a_tie() {
    let b = batch_of(
        0,
        &[
            (1, 1, vec![(1, 12.0, 1.0, 1.0), (2, 9.0, 1.0, 1.0), (3, 9.0, 1.0, 1.0)]),
            (2, 2, vec![(4, 5.0, 1.0, 1.0)]),
        ],
    );
    let c = Deterministic.choose(&b, &rng(1)).expect("choose");
    assert_eq!(c.chosen, [1, 0], "the first of the two nines; the only route");
    assert_eq!(c.probability, [1.0, 1.0]);
    c.validate(&b).expect("fits");
}

// --- sampling follows the probabilities ----------------------------------------------

#[test]
fn sampled_choices_follow_the_logit_probabilities() {
    let n = 60_000_u32;
    let situations: Vec<_> = (0..n).map(|i| (i, 0, three_routes())).collect();
    let b = batch_of(0, &situations);
    let model = logit();
    let c = model.choose(&b, &rng(7)).expect("choose");
    c.validate(&b).expect("fits");
    let p = &model.probabilities(&b).expect("probabilities")[0];
    let mut counts = [0_u32; 3];
    for &k in &c.chosen {
        counts[k as usize] += 1;
    }
    for i in 0..3 {
        let expected = f64::from(n) * p[i];
        let sigma = (f64::from(n) * p[i] * (1.0 - p[i])).sqrt();
        let diff = (f64::from(counts[i]) - expected).abs();
        assert!(diff < 5.0 * sigma, "route {i}: {} observed, {expected:.0} expected", counts[i]);
    }
    // The probability reported is that of the alternative taken.
    for (&k, &prob) in c.chosen.iter().zip(&c.probability).take(50) {
        assert!((prob - p[k as usize]).abs() < 1e-12);
    }
}

// --- draws belong to identities, not to positions -------------------------------------

#[test]
fn an_alternatives_draw_does_not_depend_on_who_else_is_offered() {
    let r = rng(42);
    let with = |alts: Vec<Alt>| batch_of(3, &[(412_002, 9, alts)]);
    let full = with(three_routes());
    let without_b = with(vec![three_routes()[0], three_routes()[2]]);
    let noise = |b: &ChoiceBatch| openmobisim_core_choice::gumbel_noise(b, &r);
    let (nf, nb) = (noise(&full), noise(&without_b));
    assert_eq!(nf[0], nb[0], "alternative 11 keeps its draw");
    assert_eq!(nf[2], nb[1], "alternative 33 keeps its draw");
    assert_eq!(nf[0], gumbel(&r, 412_002, 9, 3, 11));
}

#[test]
fn removing_an_alternative_nobody_took_changes_nobodys_choice() {
    let n = 5_000_u32;
    let full: Vec<_> = (0..n).map(|i| (i, 0, three_routes())).collect();
    let cut: Vec<_> = (0..n).map(|i| (i, 0, vec![three_routes()[0], three_routes()[2]])).collect();
    let model = logit();
    let a = model.choose(&batch_of(0, &full), &rng(5)).expect("choose");
    let b = model.choose(&batch_of(0, &cut), &rng(5)).expect("choose");
    let mut kept = 0;
    for i in 0..n as usize {
        // In the full set index 1 is alternative 22; in the cut set index 1 is alternative 33.
        if a.chosen[i] != 1 {
            let same = if a.chosen[i] == 0 { b.chosen[i] == 0 } else { b.chosen[i] == 1 };
            assert!(
                same,
                "traveller {i} changed their mind when an alternative they did not take went"
            );
            kept += 1;
        }
    }
    assert!(kept > 3_000, "most travellers did not take the removed one");
}

#[test]
fn order_and_batch_size_do_not_change_who_takes_what() {
    let n = 2_000_u32;
    let situations: Vec<_> = (0..n).map(|i| (i, i % 7, three_routes())).collect();
    let model = logit();
    let whole = model.choose(&batch_of(0, &situations), &rng(9)).expect("choose");
    // In chunks of 137 situations.
    let mut chunked = Vec::new();
    for chunk in situations.chunks(137) {
        chunked.extend(model.choose(&batch_of(0, chunk), &rng(9)).expect("choose").chosen);
    }
    assert_eq!(whole.chosen, chunked);
    // With every situation's alternatives reversed: the same alternative is taken.
    let reversed: Vec<_> = situations
        .iter()
        .map(|(t, tr, alts)| (*t, *tr, alts.iter().rev().copied().collect::<Vec<_>>()))
        .collect();
    let back = model.choose(&batch_of(0, &reversed), &rng(9)).expect("choose");
    for i in 0..n as usize {
        assert_eq!(back.chosen[i], 2 - whole.chosen[i], "situation {i}");
    }
}

#[test]
fn the_seed_and_the_iteration_change_the_draws() {
    let situations: Vec<_> = (0..500).map(|i| (i, 0, three_routes())).collect();
    let model = logit();
    let base = model.choose(&batch_of(0, &situations), &rng(1)).expect("choose").chosen;
    let again = model.choose(&batch_of(0, &situations), &rng(1)).expect("choose").chosen;
    let other_seed = model.choose(&batch_of(0, &situations), &rng(2)).expect("choose").chosen;
    let other_iteration = model.choose(&batch_of(1, &situations), &rng(1)).expect("choose").chosen;
    assert_eq!(base, again);
    assert_ne!(base, other_seed);
    assert_ne!(base, other_iteration);
}

// --- options, errors -----------------------------------------------------------------

#[test]
fn the_descriptor_names_every_coefficient_and_changes_with_any() {
    assert_eq!(logit().descriptor(), "logit;beta_ln_path_size=1;beta_time_min=-0.2");
    let custom = Logit::from_options(&Options::from([
        ("beta_time_min".into(), -0.5),
        ("beta_length_km".into(), -0.1),
    ]))
    .expect("options");
    assert_eq!(
        custom.descriptor(),
        "logit;beta_length_km=-0.1;beta_ln_path_size=1;beta_time_min=-0.5"
    );
    assert_ne!(custom.descriptor(), logit().descriptor());
    assert!(logit().is_sampled() && !Deterministic.is_sampled());
}

#[test]
fn options_are_checked_when_the_model_is_made() {
    let bad = |o: Options| Logit::from_options(&o).unwrap_err();
    assert!(matches!(
        bad(Options::from([("time".into(), 1.0)])),
        ChoiceError::UnknownOption { .. }
    ));
    assert!(matches!(
        bad(Options::from([("beta_".into(), 1.0)])),
        ChoiceError::UnknownOption { .. }
    ));
    assert!(matches!(
        bad(Options::from([("beta_time_min".into(), f64::NAN)])),
        ChoiceError::BadOption { .. }
    ));
    let message = bad(Options::from([("scale".into(), 1.0)])).to_string();
    assert!(message.contains("beta_<attribute>"), "{message}");
    assert!(Deterministic::from_options(&Options::from([("x".into(), 1.0)])).is_err());
}

#[test]
fn a_coefficient_on_an_attribute_that_is_not_offered_says_what_is() {
    let b = batch_of(0, &[(1, 1, three_routes())]);
    let m = Logit::from_options(&Options::from([("beta_comfort".into(), 1.0)])).expect("made");
    let message = m.choose(&b, &rng(1)).unwrap_err().to_string();
    assert!(message.contains("comfort") && message.contains("time_min"), "{message}");
    // A coefficient of zero drops the term, so its attribute is not needed.
    let m0 = Logit::from_options(&Options::from([("beta_comfort".into(), 0.0)])).expect("made");
    m0.choose(&b, &rng(1)).expect("no term, no need");
    assert_eq!(m0.required_attributes().unwrap(), ["ln_path_size", "time_min"]);
}

#[test]
fn a_malformed_batch_or_answer_is_refused() {
    let empty_situation = {
        let mut b = ChoiceBatch::new(0, &ATTRIBUTES);
        b.begin_situation(1, 1);
        b
    };
    assert!(matches!(empty_situation.validate(), Err(ChoiceError::BadBatch(_))));
    let duplicate = batch_of(0, &[(1, 1, vec![(5, 1.0, 1.0, 1.0), (5, 2.0, 1.0, 1.0)])]);
    assert!(duplicate.validate().unwrap_err().to_string().contains("identity 5"));
    let mut nan = ChoiceBatch::new(0, &ATTRIBUTES);
    nan.begin_situation(1, 1);
    nan.push_alternative(1, &[f64::NAN, 0.0, 0.0]);
    assert!(nan.validate().unwrap_err().to_string().contains("time_min"));
    let ok = batch_of(0, &[(1, 1, three_routes())]);
    ok.validate().expect("fine");
    let wrong = Choices { chosen: vec![3], probability: vec![0.5] };
    assert!(wrong.validate(&ok).unwrap_err().to_string().contains("only 3 were offered"));
    let short = Choices { chosen: vec![], probability: vec![] };
    assert!(short.validate(&ok).is_err());
}

// --- the registry, and a model a researcher adds ---------------------------------------

/// A traveller with a stronger aversion to routes that share links with the others than the
/// standard path-size logit assumes. Written the way the docs say a model is written.
struct AverseToSharing {
    weight: f64,
}

impl ChoiceModel for AverseToSharing {
    fn name(&self) -> &str {
        "averse_to_sharing"
    }
    fn descriptor(&self) -> String {
        format!("averse_to_sharing;weight={}", self.weight)
    }
    fn is_sampled(&self) -> bool {
        true
    }
    fn required_attributes(&self) -> Option<Vec<String>> {
        Some(vec!["time_min".into(), "ln_path_size".into()])
    }
    fn choose(&self, batch: &ChoiceBatch, rng: &StreamRng) -> Result<Choices, ChoiceError> {
        let time = batch.attribute("time_min").ok_or(ChoiceError::missing("time_min"))?;
        let ps = batch.attribute("ln_path_size").ok_or(ChoiceError::missing("ln_path_size"))?;
        let utility: Vec<f64> =
            time.iter().zip(ps).map(|(t, p)| -0.2 * t + self.weight * p).collect();
        Ok(sample_random_utility(batch, &utility, rng))
    }
}

#[test]
fn the_registry_lists_the_built_ins_and_says_what_exists_when_a_name_is_wrong() {
    let registry = Registry::builtin();
    assert_eq!(registry.names(), ["deterministic", "logit"]);
    assert_eq!(DEFAULT_MODEL, "deterministic");
    let message = registry.create("mnl", &Options::new()).err().expect("unknown").to_string();
    assert!(message.contains("deterministic") && message.contains("logit"), "{message}");
    assert_eq!(model("logit", &Options::new()).expect("built in").name(), "logit");
}

#[test]
fn a_researchers_model_is_selected_by_name_like_a_built_in() {
    let mut registry = Registry::builtin();
    registry.register("averse_to_sharing", |o| {
        Ok(Box::new(AverseToSharing { weight: o.get("weight").copied().unwrap_or(3.0) }))
    });
    assert_eq!(registry.names().last(), Some(&"averse_to_sharing"));
    let made = registry
        .create("averse_to_sharing", &Options::from([("weight".into(), 5.0)]))
        .expect("registered");
    assert_eq!(made.descriptor(), "averse_to_sharing;weight=5");
    let b = batch_of(0, &[(1, 1, three_routes())]);
    let c = made.choose(&b, &rng(3)).expect("choose");
    c.validate(&b).expect("fits");
    // Its probabilities differ from the logit's: the overlapping route is now much less likely.
    let heavy = AverseToSharing { weight: 5.0 };
    let n = 20_000_u32;
    let many: Vec<_> = (0..n).map(|i| (i, 0, three_routes())).collect();
    let chosen = heavy.choose(&batch_of(0, &many), &rng(3)).expect("choose").chosen;
    let took_b = u32::try_from(chosen.iter().filter(|&&k| k == 1).count()).expect("few");
    let share_b = f64::from(took_b) / f64::from(n);
    assert!(share_b < 0.05, "the overlapping route is avoided: {share_b}");
}
