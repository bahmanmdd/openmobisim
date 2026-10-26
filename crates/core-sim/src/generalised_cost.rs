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
use openmobisim_core_graph::defaults::RoadClass;
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
    /// kilometre), `cost_toll_eur` (the `toll_eur` column, 0 without one), `n_links` (1 per
    /// link) and the user's road columns.
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
        let links = vec![1.0; km.len()];
        let mut terms = vec![
            ("length_km".to_string(), km),
            ("cost_running_eur".to_string(), running),
            ("cost_toll_eur".to_string(), toll),
            ("n_links".to_string(), links),
        ];
        for (name, column) in values.columns_on(ValueLayer::Road) {
            terms.push((name, column.to_vec()));
        }
        Self { terms }
    }

    /// The bike layer's terms for bike routes: `length_km`, `cost_running_eur` (at the bike's
    /// price per kilometre), `bike_mixed_km`, `bike_lane_km` and `bike_separated_km` (the link's
    /// kilometres in mixed traffic, on a painted lane, on a track of its own; a ferry counts in
    /// none), `n_links` (1 per link) and the user's bike columns.
    #[must_use]
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
            .map(|(l, k)| if layer.is_mixed_traffic(LinkId::from_index(l)) { *k } else { 0.0 })
            .collect();
        let on = |kind: BikeInfrastructure| -> Vec<f64> {
            km.iter()
                .enumerate()
                .map(|(l, k)| {
                    let link = LinkId::from_index(l);
                    let ferry = network.link_class(link) == RoadClass::Ferry;
                    if layer.infrastructure(link) == kind && !ferry { *k } else { 0.0 }
                })
                .collect()
        };
        let (lane, separated) = (on(BikeInfrastructure::Lane), on(BikeInfrastructure::Separated));
        let links = vec![1.0; km.len()];
        let mut terms = vec![
            ("length_km".to_string(), km),
            ("cost_running_eur".to_string(), running),
            ("bike_mixed_km".to_string(), mixed),
            ("bike_lane_km".to_string(), lane),
            ("bike_separated_km".to_string(), separated),
            ("n_links".to_string(), links),
        ];
        for (name, column) in values.columns_on(ValueLayer::Bike) {
            terms.push((name, column.to_vec()));
        }
        Self { terms }
    }

    /// The walk layer's terms for walks: `length_km`, `n_links` (1 per link) and the user's walk
    /// columns. The walk's own minutes (`walk_min`) are its time, weighed beside `time_min`: a
    /// walk's generalised cost scales its time, which changes no walk's ranking, so it is not a
    /// term here.
    #[must_use]
    pub(crate) fn walk(layer: &StaticNetwork, values: &PreparedLinkValues) -> Self {
        let network = layer.network();
        let km: Vec<f64> = network.link_lengths().iter().map(|m| m.get() / 1000.0).collect();
        let links = vec![1.0; km.len()];
        let mut terms = vec![("length_km".to_string(), km), ("n_links".to_string(), links)];
        for (name, column) in values.columns_on(ValueLayer::Walk) {
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

/// One distinct coefficient vector while a [`RouteCost`] is composed: its extras, the value of
/// time of its first class and the people it serves.
type Distinct = (Option<Arc<[f64]>>, f64, u64);

/// The default most distinct coefficient vectors a run searches by ([`RouteCost::compose`]).
pub const DEFAULT_MAX_COST_VECTORS: u32 = 2;

/// **What every route search of a run minimises, read from its choice model** (S299).
///
/// A **coefficient vector** is a user class's coefficients on the link-additive terms of a
/// layer ([`LinkTerms`]). Each distinct one is held as its **extras**, the seconds per link it
/// adds to the travel time ([`extra_seconds`]), or as `None` where the utility weighs time alone:
/// a search by `None` is a search by time, the code path of a run without prices, bit for bit.
/// Every user class maps to one vector. A route's **generalised cost** under a vector is its
/// travel time plus the extras of its links: minus its link-additive systematic utility over the
/// time coefficient, in seconds. What is not link-additive (a parking fee, the path size, a mode
/// constant, a product with a traveller's value) is one number per alternative and is left to
/// the choice.
///
/// **Distinct vectors, and how many.** Classes whose extras are equal to the bit share one
/// vector, so one set of search trees serves them. At most `max_vectors` are kept: those serving
/// the most people (their travellers' weights; the first class breaks a tie). Every other class
/// is **grouped** onto the kept vector nearest by value of time (`β_time_min / β_cost_eur`,
/// euros per minute; infinite without a cost coefficient; the first kept breaks a tie), and the
/// run says so ([`Self::grouped`]). The cost is about one search tree set per kept vector.
#[derive(Clone, Debug, PartialEq)]
pub struct RouteCost {
    /// The kept vectors' extras, in the order of the first class that has each.
    vectors: Vec<Option<Arc<[f64]>>>,
    /// Each class's vector, by class index.
    class_vector: Vec<u32>,
    /// The classes searched by another class's vector than their own, ascending.
    grouped: Vec<u32>,
}

impl Default for RouteCost {
    fn default() -> Self {
        Self::time_only()
    }
}

impl RouteCost {
    /// Time alone for every class: one vector, `None`.
    #[must_use]
    pub fn time_only() -> Self {
        Self { vectors: vec![None], class_vector: Vec::new(), grouped: Vec::new() }
    }

    /// The route cost of `travellers`' classes under `model` over `terms`, floored on `seconds`
    /// (the layer's travel times; [`extra_seconds`]), with at most `max_vectors` distinct vectors
    /// (at least 1).
    ///
    /// # Panics
    ///
    /// Panics if there are more than `u32::MAX` vectors (there are at most as many as classes).
    #[must_use]
    pub fn compose(
        model: &dyn ChoiceModel,
        travellers: &Travellers,
        terms: &LinkTerms,
        seconds: &[f64],
        max_vectors: usize,
    ) -> Self {
        let classes = travellers.class_external_ids();
        let count = classes.count() as usize;
        let mut people = vec![0_u64; count];
        for t in 0..travellers.len() {
            let traveller = openmobisim_core_types::ids::TravellerId::new(t);
            if let Some(p) = people.get_mut(travellers.user_class(traveller).index()) {
                *p += u64::from(travellers.weight(traveller));
            }
        }
        // Per class: its extras and its value of time.
        let rows: Vec<(Option<Arc<[f64]>>, f64)> = (0..count)
            .map(|c| {
                #[allow(clippy::cast_possible_truncation, reason = "class ids are u32")]
                let coefficients = model.coefficients(classes.external(c as u32));
                let extras = coefficients
                    .as_deref()
                    .and_then(|b| extra_seconds(b, terms, seconds))
                    .map(Arc::from);
                let beta = |name: &str| {
                    coefficients
                        .as_ref()
                        .and_then(|b| b.iter().find(|(a, _)| a == name).map(|(_, v)| *v))
                };
                let vot = match (beta(TIME_ATTRIBUTE), beta("cost_eur")) {
                    (Some(t), Some(c)) if c != 0.0 => t / c,
                    _ => f64::INFINITY,
                };
                (extras, vot)
            })
            .collect();
        let same = |a: &Option<Arc<[f64]>>, b: &Option<Arc<[f64]>>| match (a, b) {
            (None, None) => true,
            (Some(a), Some(b)) => {
                a.len() == b.len()
                    && a.iter().zip(b.iter()).all(|(x, y)| x.to_bits() == y.to_bits())
            }
            _ => false,
        };
        // The distinct vectors, in the order of their first class: (extras, value of time of that
        // class, people served).
        let mut distinct: Vec<Distinct> = Vec::new();
        let mut own: Vec<usize> = Vec::with_capacity(count);
        for (c, (extras, vot)) in rows.iter().enumerate() {
            let i = match distinct.iter().position(|d| same(&d.0, extras)) {
                Some(i) => i,
                None => {
                    distinct.push((extras.clone(), *vot, 0));
                    distinct.len() - 1
                }
            };
            distinct[i].2 += people[c];
            own.push(i);
        }
        if distinct.is_empty() {
            return Self::time_only();
        }
        // The kept: the most people served, the first breaking a tie; kept in their order.
        let max_vectors = max_vectors.max(1);
        let mut by_people: Vec<usize> = (0..distinct.len()).collect();
        by_people.sort_by(|&a, &b| distinct[b].2.cmp(&distinct[a].2).then(a.cmp(&b)));
        let mut kept: Vec<usize> = by_people.into_iter().take(max_vectors).collect();
        kept.sort_unstable();
        let distance = |a: f64, b: f64| {
            if a.is_infinite() && b.is_infinite() { 0.0 } else { (a - b).abs() }
        };
        let mut grouped = Vec::new();
        let class_vector: Vec<u32> = own
            .iter()
            .enumerate()
            .map(|(c, &i)| {
                let at = kept.iter().position(|&k| k == i).unwrap_or_else(|| {
                    #[allow(clippy::cast_possible_truncation, reason = "class ids are u32")]
                    grouped.push(c as u32);
                    let mine = distinct[i].1;
                    let mut best = 0;
                    for (j, &k) in kept.iter().enumerate() {
                        if distance(distinct[k].1, mine) < distance(distinct[kept[best]].1, mine) {
                            best = j;
                        }
                    }
                    best
                });
                u32::try_from(at).expect("few vectors")
            })
            .collect();
        let vectors = kept.iter().map(|&k| distinct[k].0.clone()).collect();
        Self { vectors, class_vector, grouped }
    }

    /// Whether every class searches by time alone: then every search is the time search, to
    /// the bit.
    #[must_use]
    pub fn is_time_only(&self) -> bool {
        self.vectors.iter().all(Option::is_none)
    }

    /// How many vectors are kept (at least 1).
    #[must_use]
    pub fn vector_count(&self) -> usize {
        self.vectors.len()
    }

    /// The vector class `class` (by index) searches by; 0 for a class the cost does not know.
    #[must_use]
    pub fn vector_of_class(&self, class: usize) -> u32 {
        self.class_vector.get(class).copied().unwrap_or(0)
    }

    /// Vector `v`'s extra seconds per link, `None` where it is time alone.
    #[must_use]
    pub fn extras(&self, v: u32) -> Option<&Arc<[f64]>> {
        self.vectors.get(v as usize).and_then(Option::as_ref)
    }

    /// Every vector's extras, in order.
    #[must_use]
    pub fn vectors(&self) -> &[Option<Arc<[f64]>>] {
        &self.vectors
    }

    /// The extra seconds of `links` (raw ids) under vector `v`: 0 where it is time alone.
    #[must_use]
    pub fn route_extra(&self, v: u32, links: &[u32]) -> f64 {
        self.extras(v).map_or(0.0, |e| links.iter().map(|&l| e[l as usize]).sum())
    }

    /// The classes searched by another class's vector than their own (beyond the most vectors
    /// a run keeps), ascending by class index.
    #[must_use]
    pub fn grouped(&self) -> &[u32] {
        &self.grouped
    }

    /// The cost as a canonical string for a fingerprint: `time` where it is time alone, else
    /// each class's vector and a hash of every vector's extras.
    #[must_use]
    pub fn descriptor(&self) -> String {
        if self.is_time_only() {
            return "time".to_owned();
        }
        let mut h = openmobisim_core_types::hash::Fnv1a::new();
        h.write_u64(self.vectors.len() as u64);
        for v in &self.vectors {
            match v {
                None => h.write_bool(false),
                Some(e) => {
                    h.write_bool(true);
                    h.write_u64(e.len() as u64);
                    e.iter().for_each(|x| h.write_u64(x.to_bits()));
                }
            }
        }
        let classes: Vec<String> = self.class_vector.iter().map(u32::to_string).collect();
        format!(
            "vectors={};classes={};extras=given:{:016x}",
            self.vectors.len(),
            classes.join(","),
            h.finish()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{LinkTerms, RouteCost, drawn_coefficients, extra_seconds};

    /// Travellers of classes `a`, `b` and `c`, `n[i]` of class `i` (one person each).
    fn travellers(n: [usize; 3]) -> openmobisim_core_demand::Travellers {
        use openmobisim_core_demand::{ClassDefaults, Ownership, RawTrip, build_travellers};
        use openmobisim_core_graph::LonLat;
        use openmobisim_core_types::time::Second;
        let mut rows = Vec::new();
        for (c, &count) in n.iter().enumerate() {
            for i in 0..count {
                rows.push(RawTrip {
                    traveller_id: format!("{c}-{i}"),
                    trip_seq: 0,
                    origin: LonLat::new(4.8, 52.0),
                    destination: LonLat::new(4.81, 52.0),
                    departure_time: Second(0),
                    user_class: ["a", "b", "c"][c].to_string(),
                    weight: None,
                    mode: None,
                });
            }
        }
        let own = Ownership { car: true, bike: true, transit_pass: false };
        let defaults = ClassDefaults::new()
            .with_default("a", own)
            .with_default("b", own)
            .with_default("c", own);
        build_travellers(
            rows,
            Vec::new(),
            &defaults,
            1,
            &mut openmobisim_core_types::diagnostics::Diagnostics::new(),
        )
        .expect("builds")
        .0
    }

    fn logit(costs: [f64; 3]) -> openmobisim_core_choice::Logit {
        let class = |name: &str, beta: f64| {
            (name.to_string(), [("beta_cost_eur".to_string(), beta)].into_iter().collect())
        };
        openmobisim_core_choice::Logit::default()
            .with_classes(&[class("a", costs[0]), class("b", costs[1]), class("c", costs[2])])
            .expect("valid")
    }

    /// **The run's route cost** (S299): time alone where nothing but time weighs; one vector per
    /// distinct class cost, shared by equal classes; beyond the most kept, a class grouped onto
    /// the kept vector nearest by value of time, and said so.
    #[test]
    fn classes_share_equal_vectors_and_are_grouped_by_value_of_time_beyond_the_limit() {
        let free = LinkTerms { terms: vec![("cost_running_eur".into(), vec![0.0, 0.0])] };
        let priced = LinkTerms { terms: vec![("cost_running_eur".into(), vec![0.5, 0.0])] };
        let seconds = [60.0, 60.0];
        let people = travellers([3, 2, 1]);
        // No price: every class's cost is its time.
        let none = RouteCost::compose(&logit([-1.2, -0.6, -0.5]), &people, &free, &seconds, 2);
        assert!(none.is_time_only());
        assert_eq!(none.descriptor(), "time");
        // Equal classes, one vector.
        let one = RouteCost::compose(&logit([-1.2, -1.2, -1.2]), &people, &priced, &seconds, 2);
        assert!(!one.is_time_only());
        assert_eq!(one.vector_count(), 1);
        assert!(one.grouped().is_empty());
        // 0.5 € at 10 €/h is 180 s.
        assert!((one.route_extra(0, &[0, 1]) - 180.0).abs() < 1e-9);
        // Three values of time, two kept: `a` (3 people) and `b` (2); `c`, at 0.4 €/min, is
        // nearest `b` (0.33 €/min), not `a` (0.17 €/min).
        let three = RouteCost::compose(&logit([-1.2, -0.6, -0.5]), &people, &priced, &seconds, 2);
        assert_eq!(three.vector_count(), 2);
        assert_eq!(three.grouped(), &[2]);
        assert_eq!(three.vector_of_class(2), three.vector_of_class(1));
        assert_ne!(three.vector_of_class(0), three.vector_of_class(1));
        // With room for three, each its own.
        let all = RouteCost::compose(&logit([-1.2, -0.6, -0.5]), &people, &priced, &seconds, 3);
        assert_eq!(all.vector_count(), 3);
        assert!(all.grouped().is_empty());
        assert_ne!(all.descriptor(), three.descriptor());
    }

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
