//! The generalised cost a route search minimises when it searches by what a user class values
//! (S298, `plans/58` §3.1): a link's travel time plus **extra seconds** for its length, its money
//! and the user's own link values, weighed by the class's coefficients, and **preference draws**
//! that perturb those coefficients per class and draw.
//!
//! **Composition.** A class's utility is linear in attributes, `U = β_time · minutes + Σ_k β_k ·
//! v_k`; its generalised cost in seconds is `−U / (−β_time / 60) = seconds + Σ_k (60 β_k / β_time)
//! · v_k`. For a link, every link-additive attribute `v_k` is a number per link (its kilometres,
//! its running cost, its toll, its kilometres in mixed traffic, a user column); the sum over the
//! route's links of the per-link extras is the route's. What is not link-additive (a parking fee,
//! a path size, a mode constant) is not part of a route search: it does not change which route of
//! a pair is cheapest. The extras are floored so that time plus extra is never negative
//! ([`openmobisim_core_routes::SearchContext::extras`] refuses a negative cost).
//!
//! **Which attributes.** `length_km`; `cost_running_eur` (kilometres times the vehicle's price
//! per kilometre) and `cost_toll_eur` (the road's `toll_eur` column), both also weighed by
//! `cost_eur`, the total's coefficient; `bike_mixed_km` on the bike layer; and the user's
//! `<layer>_<name>_km` and `<layer>_<name>_sum` columns. A coefficient on anything else is left
//! out here and weighs the alternative at the choice, as before.
//!
//! **Preference draws.** Draw `d` of class `c` multiplies each coefficient but the time
//! coefficient (the unit) by `1 + τ · v`, `v` uniform in [−1, 1) from a hash of (seed, class,
//! draw, attribute); the factor is clipped at 0, so a coefficient keeps its sign. Draw 0 is the
//! class's coefficients unchanged.
//!
//! **Cost.** One pass over the links per class and draw: a vector of 8 bytes per link each.
//! Nothing here runs for a run whose method does not ask for extras.

use std::sync::Arc;

use openmobisim_core_choice::ChoiceModel;
use openmobisim_core_demand::Travellers;
use openmobisim_core_graph::layers::{BikeInfrastructure, StaticNetwork};
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_routes::{ClassExtras, ExtrasRequest};
use openmobisim_core_types::ids::{EntityId, LinkId};

use crate::link_values::{PreparedLinkValues, ValueLayer};
use crate::prices::{Prices, TOLL_COLUMN};

/// The attribute whose coefficient is the unit of the generalised cost.
pub const TIME_ATTRIBUTE: &str = "time_min";

/// The link-additive terms of a layer: per attribute name, a value per link of the layer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LinkTerms {
    /// `(attribute, value per link)`, in a fixed order.
    pub terms: Vec<(String, Vec<f64>)>,
}

impl LinkTerms {
    /// The road's terms for car routes: `length_km`, `cost_running_eur` (at the car's price per
    /// kilometre), `cost_toll_eur` (the `toll_eur` column, 0 without one) and the user's road
    /// columns.
    #[must_use]
    pub(crate) fn road(
        network: &RoadNetwork,
        prices: &Prices,
        values: &PreparedLinkValues,
    ) -> Self {
        let km: Vec<f64> = network.link_lengths().iter().map(|m| m.get() / 1000.0).collect();
        let running: Vec<f64> = km.iter().map(|k| k * prices.car_eur_km).collect();
        let toll: Vec<f64> = match values.column(ValueLayer::Road, TOLL_COLUMN) {
            Some(column) => (0..km.len())
                .map(|l| {
                    values.total(column, crate::link_values::Aggregate::Sum, std::iter::once(l))
                })
                .collect(),
            None => vec![0.0; km.len()],
        };
        let mut terms = vec![
            ("length_km".to_string(), km),
            ("cost_running_eur".to_string(), running),
            ("cost_toll_eur".to_string(), toll),
        ];
        for (name, column) in values.columns_on(ValueLayer::Road) {
            terms.push((name, column.to_vec()));
        }
        Self { terms }
    }

    /// The bike layer's terms for bike routes: `length_km`, `cost_running_eur` (at the bike's
    /// price per kilometre), `bike_mixed_km` (the link's kilometres if it is in mixed traffic)
    /// and the user's bike columns.
    #[must_use]
    #[allow(dead_code, reason = "wired by the bike route sets (S298 P7a)")]
    pub(crate) fn bike(
        layer: &StaticNetwork,
        prices: &Prices,
        values: &PreparedLinkValues,
    ) -> Self {
        let network = layer.network();
        let km: Vec<f64> = network.link_lengths().iter().map(|m| m.get() / 1000.0).collect();
        let running: Vec<f64> = km.iter().map(|k| k * prices.bike_eur_km).collect();
        let mixed: Vec<f64> = km
            .iter()
            .enumerate()
            .map(|(l, k)| {
                if layer.infrastructure(LinkId::from_index(l)) == BikeInfrastructure::Mixed {
                    *k
                } else {
                    0.0
                }
            })
            .collect();
        let mut terms = vec![
            ("length_km".to_string(), km),
            ("cost_running_eur".to_string(), running),
            ("bike_mixed_km".to_string(), mixed),
        ];
        for (name, column) in values.columns_on(ValueLayer::Bike) {
            terms.push((name, column.to_vec()));
        }
        Self { terms }
    }
}

/// A uniform number in [−1, 1) that is a pure function of its arguments (SplitMix64's finaliser).
fn signed_uniform(seed: u64, class: u32, draw: u32, attribute: u32) -> f64 {
    let mut x = seed
        ^ (u64::from(class) << 40 | u64::from(draw) << 20 | u64::from(attribute))
            .wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    #[allow(clippy::cast_precision_loss, reason = "the top 53 bits of a u64 fit an f64 exactly")]
    let u = (x >> 11) as f64 / (1_u64 << 53) as f64;
    2.0 * u - 1.0
}

/// `coefficients` with the preference draw `draw` of class `class` applied: every coefficient
/// but the time coefficient times `1 + tau · v`, `v` uniform in [−1, 1) from (seed, class,
/// draw, the attribute's position), the factor clipped at 0. Draw 0, or `tau` 0, returns them
/// unchanged. The order is kept.
#[must_use]
pub fn drawn_coefficients(
    coefficients: &[(String, f64)],
    seed: u64,
    class: u32,
    draw: u32,
    tau: f64,
) -> Vec<(String, f64)> {
    if draw == 0 || tau == 0.0 {
        return coefficients.to_vec();
    }
    coefficients
        .iter()
        .enumerate()
        .map(|(k, (attribute, beta))| {
            if attribute == TIME_ATTRIBUTE {
                return (attribute.clone(), *beta);
            }
            #[allow(clippy::cast_possible_truncation, reason = "a handful of attributes")]
            let v = signed_uniform(seed, class, draw, k as u32);
            (attribute.clone(), beta * (1.0 + tau * v).max(0.0))
        })
        .collect()
}

/// The extra seconds per link under `coefficients`: `Σ_k (60 β_k / β_time) · v_k(l)` over the
/// `terms` the coefficients name (`cost_eur` weighs `cost_running_eur` and `cost_toll_eur` too),
/// floored so that `seconds[l] + extra[l]` is never negative where `seconds[l]` is finite. `None`
/// if there is no time coefficient, it is not negative, or every extra is 0: the search is then
/// by time alone and costs nothing more.
#[must_use]
pub fn extra_seconds(
    coefficients: &[(String, f64)],
    terms: &LinkTerms,
    seconds: &[f64],
) -> Option<Vec<f64>> {
    let beta_time = coefficients.iter().find(|(a, _)| a == TIME_ATTRIBUTE).map(|(_, b)| *b)?;
    if !(beta_time.is_finite() && beta_time < 0.0) {
        return None;
    }
    let weight = |attribute: &str| -> f64 {
        let own = coefficients.iter().find(|(a, _)| a == attribute).map_or(0.0, |(_, b)| *b);
        let total = if attribute == "cost_running_eur" || attribute == "cost_toll_eur" {
            coefficients.iter().find(|(a, _)| a == "cost_eur").map_or(0.0, |(_, b)| *b)
        } else {
            0.0
        };
        own + total
    };
    let mut extra = vec![0.0; seconds.len()];
    let mut any = false;
    for (attribute, values) in &terms.terms {
        let w = weight(attribute);
        if w == 0.0 {
            continue;
        }
        let per_unit = 60.0 * w / beta_time;
        for (e, v) in extra.iter_mut().zip(values) {
            if *v != 0.0 {
                *e += per_unit * v;
                any = true;
            }
        }
    }
    if !any {
        return None;
    }
    for (e, &t) in extra.iter_mut().zip(seconds) {
        if t.is_finite() && t + *e < 0.0 {
            *e = -t;
        }
    }
    Some(extra)
}

/// The road's search cost per link as a car search has it: the free-flow time, infinite where no
/// car may go ([`openmobisim_core_routes::SearchContext::new`]); what the extras are floored on.
#[must_use]
pub fn road_seconds(network: &RoadNetwork) -> Vec<f64> {
    (0..network.link_count())
        .map(|i| {
            let link = LinkId::new(i);
            if network.link_class(link).carries_motor_traffic() {
                network.free_flow_time(link).get()
            } else {
                f64::INFINITY
            }
        })
        .collect()
}

/// `extras[class][draw]` for every user class of `travellers`, as a method or update that asked
/// for them wants them ([`ExtrasRequest`]): the class's coefficients from `model`
/// ([`ChoiceModel::coefficients`]; a model without them gives a row of `None`: time alone), drawn
/// per [`drawn_coefficients`] for draws `1..=request.draws` (draw 0 unperturbed), composed over
/// `terms` and floored on `seconds` by [`extra_seconds`].
#[must_use]
pub fn class_extras(
    request: ExtrasRequest,
    model: &dyn ChoiceModel,
    travellers: &Travellers,
    terms: &LinkTerms,
    seconds: &[f64],
) -> ClassExtras {
    let classes = travellers.class_external_ids();
    (0..classes.count().max(1))
        .map(|c| {
            let coefficients =
                if c < classes.count() { model.coefficients(classes.external(c)) } else { None };
            (0..=request.draws)
                .map(|d| {
                    let drawn = drawn_coefficients(
                        coefficients.as_deref()?,
                        request.seed,
                        c,
                        d,
                        request.tau,
                    );
                    extra_seconds(&drawn, terms, seconds).map(Arc::from)
                })
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{LinkTerms, drawn_coefficients, extra_seconds};

    fn betas(pairs: &[(&str, f64)]) -> Vec<(String, f64)> {
        pairs.iter().map(|(a, b)| ((*a).to_string(), *b)).collect()
    }

    #[test]
    fn length_and_money_become_seconds_by_the_value_of_time() {
        // −0.2 per minute and −1.2 per euro: one euro is 6 minutes; −0.1 per km: half a minute.
        let terms = LinkTerms {
            terms: vec![
                ("length_km".into(), vec![1.0, 2.0, 0.0]),
                ("cost_running_eur".into(), vec![0.5, 0.0, 0.0]),
                ("cost_toll_eur".into(), vec![0.0, 0.0, 1.0]),
            ],
        };
        let c = betas(&[("cost_eur", -1.2), ("length_km", -0.1), ("time_min", -0.2)]);
        let extra = extra_seconds(&c, &terms, &[100.0, 100.0, 100.0]).expect("extras");
        // link 0: 1 km → 30 s, 0.5 € → 180 s; link 1: 2 km → 60 s; link 2: a 1 € toll → 360 s.
        assert!((extra[0] - 210.0).abs() < 1e-9, "{extra:?}");
        assert!((extra[1] - 60.0).abs() < 1e-9);
        assert!((extra[2] - 360.0).abs() < 1e-9);
        // A part's own coefficient adds to the total's.
        let c = betas(&[("cost_eur", -1.2), ("cost_toll_eur", -1.2), ("time_min", -0.2)]);
        let extra = extra_seconds(&c, &terms, &[100.0; 3]).expect("extras");
        assert!((extra[2] - 720.0).abs() < 1e-9);
    }

    #[test]
    fn the_defaults_and_time_only_models_give_no_extras() {
        let terms = LinkTerms { terms: vec![("length_km".into(), vec![1.0, 2.0])] };
        // No coefficient on any term: nothing to add.
        assert_eq!(extra_seconds(&betas(&[("time_min", -0.2)]), &terms, &[1.0, 1.0]), None);
        // No time coefficient, or a positive one: time alone.
        assert_eq!(extra_seconds(&betas(&[("length_km", -0.1)]), &terms, &[1.0, 1.0]), None);
        assert_eq!(
            extra_seconds(&betas(&[("length_km", -0.1), ("time_min", 0.2)]), &terms, &[1.0, 1.0]),
            None
        );
    }

    #[test]
    fn a_liked_attribute_is_floored_so_that_no_link_costs_less_than_nothing() {
        // A positive coefficient on length: longer is better, but a link never costs below 0.
        let terms = LinkTerms { terms: vec![("length_km".into(), vec![1.0, 1.0, 1.0])] };
        let c = betas(&[("length_km", 1.0), ("time_min", -0.2)]);
        let extra = extra_seconds(&c, &terms, &[100.0, 500.0, f64::INFINITY]).expect("extras");
        assert!((extra[0] - -100.0).abs() < 1e-9, "floored at minus the time: {extra:?}");
        assert!((extra[1] - -300.0).abs() < 1e-9, "unfloored: 60 · 1 / −0.2 = −300");
        assert!((extra[2] - -300.0).abs() < 1e-9, "an unusable link is left as computed");
    }

    #[test]
    fn preference_draws_keep_the_time_coefficient_the_sign_and_the_order() {
        let c = betas(&[("cost_eur", -1.2), ("length_km", -0.1), ("time_min", -0.2)]);
        assert_eq!(drawn_coefficients(&c, 7, 0, 0, 0.3), c, "draw 0 is unchanged");
        assert_eq!(drawn_coefficients(&c, 7, 0, 3, 0.0), c, "tau 0 is unchanged");
        let d = drawn_coefficients(&c, 7, 1, 3, 0.3);
        assert_eq!(
            d.iter().map(|(a, _)| a.as_str()).collect::<Vec<_>>(),
            ["cost_eur", "length_km", "time_min"]
        );
        assert_eq!(d[2].1.to_bits(), (-0.2_f64).to_bits(), "the unit is not drawn");
        for ((_, b), (_, o)) in d.iter().zip(&c).take(2) {
            assert!(b <= &0.0, "the sign is kept");
            assert!((b / o - 1.0).abs() <= 0.3 + 1e-12, "within tau of the mean");
        }
        assert_eq!(drawn_coefficients(&c, 7, 1, 3, 0.3), d, "a pure function of its arguments");
        assert_ne!(drawn_coefficients(&c, 8, 1, 3, 0.3), d, "another seed, another draw");
        assert_ne!(drawn_coefficients(&c, 7, 2, 3, 0.3), d, "another class, another draw");
        // A tau above 1 would flip a sign: the factor is clipped at 0 instead.
        let big = drawn_coefficients(&c, 1, 0, 1, 5.0);
        assert!(big.iter().take(2).all(|(_, b)| *b <= 0.0));
    }
}
