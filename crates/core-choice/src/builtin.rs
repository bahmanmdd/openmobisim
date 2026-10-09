//! The built-in choice models.
//!
//! | name | what it does |
//! |---|---|
//! | `deterministic` (the default, [`DEFAULT_MODEL`](crate::DEFAULT_MODEL)) | Everyone takes the alternative with the least `time_min`: all-or-nothing, for debugging and upper bounds |
//! | `logit` | A random utility model, linear in named attributes, sampled exactly by the Gumbel-max construction; with the path-size term it is the standard **path-size logit** for routes |
//! | `nested_logit` | The logit's utility with the alternatives in nests (the attribute `nest`; in mode choice, the mode), scale `mu`; sampled exactly with keyed draws (M5) |

use crate::batch::{ChoiceBatch, Choices};
use crate::model::{
    ChoiceError, ChoiceModel, Options, first_argmax, logit_probabilities, sample_random_utility,
};
use openmobisim_core_types::rng::StreamRng;

/// Everyone takes the alternative with the least `time_min`, the first of any
/// tie (so, in route choice, the store's best route: cost, then links).
///
/// All-or-nothing assignment: no random draw, no probability but 1. It is the
/// debugging rung and the upper bound of design §4, and it reproduces the
/// behaviour every run had before the choice layer existed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Deterministic;

impl Deterministic {
    /// Make the model from its options (it has none).
    ///
    /// # Errors
    ///
    /// [`ChoiceError::UnknownOption`] for any option.
    pub fn from_options(options: &Options) -> Result<Self, ChoiceError> {
        match options.keys().next() {
            Some(option) => Err(ChoiceError::UnknownOption {
                model: "deterministic".to_string(),
                option: option.clone(),
                known: "no options".to_string(),
            }),
            None => Ok(Self),
        }
    }
}

impl ChoiceModel for Deterministic {
    fn name(&self) -> &str {
        "deterministic"
    }

    fn descriptor(&self) -> String {
        "deterministic".to_string()
    }

    fn is_sampled(&self) -> bool {
        false
    }

    fn required_attributes(&self) -> Option<Vec<String>> {
        Some(vec!["time_min".to_string()])
    }

    fn choose(&self, batch: &ChoiceBatch, _rng: &StreamRng) -> Result<Choices, ChoiceError> {
        let time = batch.attribute("time_min").ok_or_else(|| ChoiceError::MissingAttribute {
            name: "time_min".to_string(),
            offered: batch.attribute_names().to_vec(),
        })?;
        let mut chosen = Vec::with_capacity(batch.situations());
        let mut scratch: Vec<f64> = Vec::new();
        for s in 0..batch.situations() {
            scratch.clear();
            scratch.extend(batch.range(s).map(|a| -time[a]));
            #[allow(clippy::cast_possible_truncation, reason = "an index within one situation")]
            chosen.push(first_argmax(&scratch) as u32);
        }
        let probability = vec![1.0; batch.situations()];
        Ok(Choices { chosen, probability })
    }

    fn probabilities(&self, batch: &ChoiceBatch) -> Result<Option<Vec<f64>>, ChoiceError> {
        // All the probability on the alternative `choose` would take.
        let chosen = self.choose(
            batch,
            &openmobisim_core_types::rng::StreamRng::new(
                openmobisim_core_types::rng::RngKey::from_seed(0),
                openmobisim_core_types::rng::Stream::Choice,
            ),
        )?;
        let mut p = vec![0.0; batch.alternatives()];
        for (s, &c) in chosen.chosen.iter().enumerate() {
            p[batch.range(s).start + c as usize] = 1.0;
        }
        Ok(Some(p))
    }
}

/// A logit whose utility is linear in named attributes:
/// `U = Σ β_name · attribute_name`, then the traveller takes the alternative with
/// the largest `U` plus a Gumbel error, which is an exact draw from the logit.
///
/// Coefficients are options named `beta_<attribute>`. The defaults are those of
/// the standard **path-size logit** for route choice:
///
/// | option | default | attribute (per route) |
/// |---|---|---|
/// | `beta_time_min` | −0.2 | travel time, minutes (door to door for an itinerary) |
/// | `beta_ln_path_size` | 1 | ln of the path-size factor: routes that share links with the others in the set split their share (Ben-Akiva and Bierlaire, 1999), which repairs the logit's blindness to overlap |
/// | `beta_walk_min` | −0.13 | minutes walking, **on top of** `time_min`: walking weighs about 1.65 times riding (M4, A9) |
/// | `beta_wait_min` | −0.09 | minutes waiting at stops, on top: about 1.47 times riding (M4, A9) |
/// | `beta_transfers` | −1 | each change of vehicle: about five minutes (M4, A9) |
/// | `beta_bike_mixed_km` | −0.16 | each kilometre of a bike leg in mixed traffic, **on top of** its time (S244, D-4) |
/// | `beta_cost_eur` | −1.2 | what the alternative costs, euros: 10 €/h at −0.2 per minute (D-1); see **Money** below |
///
/// Walking, waiting, changing and riding in mixed traffic are 0 for a car route, so they
/// change nothing there; for an itinerary (transit, park-and-ride, bike-and-ride) the first
/// three are the standard weights of walking, waiting and changing — the averages of
/// Wardman's meta-analysis of public transport values of time (2004) and the low end of the
/// transfer penalties Garcia-Martinez et al. (2018) review. CITATION OWED (the references to
/// check); uncalibrated, like the rest. The weight on mixed traffic is what the bike route
/// choice already assumes at the default time weight and speed (a minute in mixed traffic
/// counts 1.2: 0.8 minute more per km at 15 km/h, times −0.2), so a ride's route and its
/// mode are chosen alike.
///
/// Any other attribute on offer takes a coefficient the same way — for routes
/// `beta_length_km`, `beta_detour`, `beta_overlap`, `beta_n_links` — and one not on
/// offer is refused when the run starts, with the list of those that are. Set a
/// default to `0` to drop its term.
///
/// **Money** (S248): `beta_cost_eur` weighs what an alternative costs in euros: −1.2 by default
/// since `DEFAULTS_VERSION` 20 (D-1; 0 before, and 0 drops it). A value of time of `V` euros per
/// hour is `beta_cost_eur = beta_time_min · 60 / V`: −1.2 per euro at −0.2 per minute and 10 €/h;
/// a traveller class's own coefficient is its own value of time.
///
/// **Products** (S249): `beta_<a>*<b>` weighs the product of two attributes, so a value that is
/// the same for every alternative of a choice — the traveller's (`person_<name>`), the trip's
/// (`trip_<name>`, `trip_departure_h`) — can shift one mode (`beta_mode_car*person_age`) or
/// scale another attribute (`beta_cost_eur*person_income_inv`). The two names may come in
/// either order; they are kept sorted.
///
/// **The defaults are an assumption, not a calibration**: a time coefficient of
/// −0.2 per minute means a route four minutes slower is taken with about 45% of
/// the weight of one that is not, other things equal. Estimate your own and pass them.
#[derive(Clone, Debug, PartialEq)]
pub struct Logit {
    /// `(attribute, coefficient)`, sorted by attribute.
    pub betas: Vec<(String, f64)>,
    /// Per traveller class (S231), by class index: its name and its coefficients (the model's,
    /// with the class's overrides), sorted by attribute. Empty: every class weighs alike.
    pub classes: Vec<(String, Vec<(String, f64)>)>,
}

impl Default for Logit {
    fn default() -> Self {
        Self {
            betas: vec![
                ("bike_mixed_km".to_string(), -0.16),
                ("cost_eur".to_string(), -1.2),
                ("ln_path_size".to_string(), 1.0),
                ("time_min".to_string(), -0.2),
                ("transfers".to_string(), -1.0),
                ("wait_min".to_string(), -0.09),
                ("walk_min".to_string(), -0.13),
            ],
            classes: Vec::new(),
        }
    }
}

/// The two attributes of a product term `a*b` (S249), if `term` is one.
fn product(term: &str) -> Option<(&str, &str)> {
    term.split_once('*')
}

/// `attribute` as a model keeps it: a product's two names sorted (S249), so `b*a` is `a*b`.
fn canonical(attribute: &str) -> String {
    match product(attribute) {
        Some((a, b)) if b < a => format!("{b}*{a}"),
        _ => attribute.to_string(),
    }
}

/// One term of a utility on a batch's columns: a column, or the product of two (S249).
#[derive(Clone, Copy, Debug)]
enum Term {
    One(usize),
    Two(usize, usize),
}

impl Term {
    fn value(self, columns: &[&[f64]], a: usize) -> f64 {
        match self {
            Term::One(k) => columns[k][a],
            Term::Two(j, k) => columns[j][a] * columns[k][a],
        }
    }
}

/// A class's utility on a batch's columns: a coefficient per column (0 for one it does not
/// weigh), and its product terms (S249).
type Dense = (Vec<f64>, Vec<(Term, f64)>);

/// The terms `betas` weigh, on `batch`'s columns, with their coefficients (zeros dropped).
fn terms(betas: &[(String, f64)], batch: &ChoiceBatch) -> Result<Vec<(Term, f64)>, ChoiceError> {
    let names = batch.attribute_names();
    let column = |name: &str| {
        names.iter().position(|n| n == name).ok_or_else(|| ChoiceError::MissingAttribute {
            name: name.to_string(),
            offered: names.to_vec(),
        })
    };
    let mut out = Vec::with_capacity(betas.len());
    for (attribute, beta) in betas {
        if *beta == 0.0 {
            continue;
        }
        let term = match product(attribute) {
            Some((a, b)) => Term::Two(column(a)?, column(b)?),
            None => Term::One(column(attribute)?),
        };
        out.push((term, *beta));
    }
    Ok(out)
}

impl Logit {
    /// Make the model from its options, defaults for those not given.
    ///
    /// # Errors
    ///
    /// [`ChoiceError::UnknownOption`] for an option that is not `beta_<attribute>`,
    /// [`ChoiceError::BadOption`] for a coefficient that is not a finite number.
    pub fn from_options(options: &Options) -> Result<Self, ChoiceError> {
        let mut m = Self::default();
        for (option, value) in options {
            let Some(attribute) = option.strip_prefix("beta_").filter(|a| !a.is_empty()) else {
                return Err(ChoiceError::UnknownOption {
                    model: "logit".to_string(),
                    option: option.clone(),
                    known: "beta_<attribute>, a coefficient per attribute, e.g. beta_time_min"
                        .to_string(),
                });
            };
            if !value.is_finite() {
                return Err(ChoiceError::BadOption {
                    model: "logit".to_string(),
                    option: option.clone(),
                    reason: format!("must be a finite number, got {value}"),
                });
            }
            if product(attribute)
                .is_some_and(|(a, b)| a.is_empty() || b.is_empty() || b.contains('*'))
            {
                return Err(ChoiceError::BadOption {
                    model: "logit".to_string(),
                    option: option.clone(),
                    reason: "a product is beta_<a>*<b>, two attributes".to_string(),
                });
            }
            let attribute = canonical(attribute);
            match m.betas.iter_mut().find(|(a, _)| *a == attribute) {
                Some(entry) => entry.1 = *value,
                None => m.betas.push((attribute, *value)),
            }
        }
        m.betas.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(m)
    }

    /// The same model with coefficients per traveller class (S231): `classes[c]` is class `c`'s
    /// name and its `beta_<attribute>` overrides of this model's coefficients.
    ///
    /// # Errors
    ///
    /// As [`Self::from_options`], for a class's options.
    pub fn with_classes(mut self, classes: &[(String, Options)]) -> Result<Self, ChoiceError> {
        self.classes = Vec::with_capacity(classes.len());
        for (name, options) in classes {
            // Checked as the model's own options are; only the names given override.
            Self::from_options(options).map_err(|e| match e {
                ChoiceError::UnknownOption { option, known, .. } => ChoiceError::UnknownOption {
                    model: format!("logit (class {name})"),
                    option,
                    known,
                },
                ChoiceError::BadOption { option, reason, .. } => ChoiceError::BadOption {
                    model: format!("logit (class {name})"),
                    option,
                    reason,
                },
                other => other,
            })?;
            let mut merged = self.betas.clone();
            for (option, value) in options {
                let attribute = canonical(option.strip_prefix("beta_").unwrap_or(option));
                match merged.iter_mut().find(|(a, _)| *a == attribute) {
                    Some(entry) => entry.1 = *value,
                    None => merged.push((attribute, *value)),
                }
            }
            merged.sort_by(|a, b| a.0.cmp(&b.0));
            self.classes.push((name.clone(), merged));
        }
        Ok(self)
    }

    /// The classes' coefficients that differ from the model's, as `;class:<name>.beta_<a>=<v>`
    /// terms in class-name order: what a descriptor adds for them (S231).
    fn class_terms(&self) -> String {
        let mut by_name: Vec<&(String, Vec<(String, f64)>)> = self.classes.iter().collect();
        by_name.sort_by(|a, b| a.0.cmp(&b.0));
        let mut out = String::new();
        for (name, betas) in by_name {
            for (attribute, beta) in betas {
                let own = self.betas.iter().find(|(a, _)| a == attribute).map_or(0.0, |(_, b)| *b);
                if beta.to_bits() != own.to_bits() {
                    out.push_str(&format!(";class:{name}.beta_{attribute}={beta}"));
                }
            }
        }
        out
    }

    /// The utility of every alternative in `batch`.
    ///
    /// # Errors
    ///
    /// [`ChoiceError::MissingAttribute`] if a coefficient names an attribute the
    /// batch does not carry.
    pub fn utilities(&self, batch: &ChoiceBatch) -> Result<Vec<f64>, ChoiceError> {
        let columns: Vec<&[f64]> =
            (0..batch.attribute_names().len()).map(|k| batch.column(k)).collect();
        let own = terms(&self.betas, batch)?;
        let mut utility = vec![0.0; batch.alternatives()];
        if self.classes.is_empty() {
            for (term, beta) in own {
                for (a, u) in utility.iter_mut().enumerate() {
                    *u += beta * term.value(&columns, a);
                }
            }
            return Ok(utility);
        }
        // Each situation weighed by its class's coefficients (S231); a class index past the
        // classes given takes the model's own. Per class, a coefficient per batch attribute (0
        // for one it does not weigh), summed in column order, then its products (S249).
        let dense = |weighed: Vec<(Term, f64)>| {
            let mut plain = vec![0.0; columns.len()];
            let mut products = Vec::new();
            for (term, beta) in weighed {
                match term {
                    Term::One(k) => plain[k] = beta,
                    Term::Two(..) => products.push((term, beta)),
                }
            }
            (plain, products)
        };
        let own = dense(own);
        let by_class: Vec<Dense> = self
            .classes
            .iter()
            .map(|(_, b)| terms(b, batch).map(dense))
            .collect::<Result<_, _>>()?;
        for (s, &class) in batch.classes().iter().enumerate() {
            let (plain, products) = by_class.get(class as usize).unwrap_or(&own);
            for a in batch.range(s) {
                let linear: f64 = plain.iter().zip(&columns).map(|(b, c)| b * c[a]).sum();
                let more: f64 = products.iter().map(|(t, b)| b * t.value(&columns, a)).sum();
                utility[a] = linear + more;
            }
        }
        Ok(utility)
    }

    /// The logit probability of every alternative, for inspection and tests:
    /// one vector per situation.
    ///
    /// # Errors
    ///
    /// As [`Logit::utilities`].
    pub fn probabilities_by_situation(
        &self,
        batch: &ChoiceBatch,
    ) -> Result<Vec<Vec<f64>>, ChoiceError> {
        let utility = self.utilities(batch)?;
        Ok((0..batch.situations()).map(|s| logit_probabilities(&utility[batch.range(s)])).collect())
    }
}

impl ChoiceModel for Logit {
    fn name(&self) -> &str {
        "logit"
    }

    fn descriptor(&self) -> String {
        let terms: Vec<String> = self.betas.iter().map(|(a, b)| format!("beta_{a}={b}")).collect();
        format!("logit;{}{}", terms.join(";"), self.class_terms())
    }

    fn is_sampled(&self) -> bool {
        true
    }

    fn required_attributes(&self) -> Option<Vec<String>> {
        // Every attribute some class weighs (S231), the model's own among them.
        // A product reads both its attributes (S249).
        let mut names: Vec<String> = self
            .betas
            .iter()
            .chain(self.classes.iter().flat_map(|(_, b)| b.iter()))
            .filter(|(_, b)| *b != 0.0)
            .flat_map(|(a, _)| match product(a) {
                Some((x, y)) => vec![x.to_string(), y.to_string()],
                None => vec![a.clone()],
            })
            .collect();
        names.sort();
        names.dedup();
        Some(names)
    }

    fn choose(&self, batch: &ChoiceBatch, rng: &StreamRng) -> Result<Choices, ChoiceError> {
        Ok(sample_random_utility(batch, &self.utilities(batch)?, rng))
    }

    fn probabilities(&self, batch: &ChoiceBatch) -> Result<Option<Vec<f64>>, ChoiceError> {
        Ok(Some(self.probabilities_by_situation(batch)?.into_iter().flatten().collect()))
    }

    fn logsums(&self, batch: &ChoiceBatch) -> Result<Option<Vec<f64>>, ChoiceError> {
        let utility = self.utilities(batch)?;
        Ok(Some(
            (0..batch.situations())
                .map(|s| crate::model::log_sum_exp(utility[batch.range(s)].iter().copied()))
                .collect(),
        ))
    }
}

/// The draw space of the nests: the iteration field of their draws carries this bit, so a
/// nest's draw can never be an alternative's (whose iteration field is the plain iteration).
const NEST_KEY: u32 = 0x4000_0000;

/// A **nested logit** (design §4; M5): the [`Logit`]'s utility, linear in named
/// attributes with the same coefficients and defaults, and the alternatives grouped
/// in nests by the attribute **`nest`** (a number: in mode choice, the mode).
///
/// With nest scale `μ` (`mu`, 0 < μ ≤ 1): the inclusive value of nest `n` is
/// `Iₙ = ln Σ_{j∈n} exp(Vⱼ / μ)`; a nest is taken with probability
/// `exp(μ Iₙ) / Σₘ exp(μ Iₘ)`, and an alternative in it with `exp(Vⱼ / μ) / Σ_{k∈n}
/// exp(Vₖ / μ)`. **μ = 1 is the flat logit**; a smaller μ makes the alternatives of one
/// nest closer substitutes for each other than for the rest — so a mode with several
/// routes does not gain share from their number.
///
/// **Sampled exactly with keyed draws**: the nest by the Gumbel-max construction over
/// `μ Iₙ`, each nest's error keyed on (traveller, trip, iteration, nest) in a draw
/// space of its own; then the alternative by Gumbel-max over `Vⱼ / μ`, each error keyed
/// on its identity, as the logit's.
///
/// **Default `mu` = 0.5**: inside the 0.3–0.8 range reported for route-in-mode nests.
/// CITATION OWED; uncalibrated, like the coefficients.
#[derive(Clone, Debug, PartialEq)]
pub struct NestedLogit {
    /// The utility.
    pub logit: Logit,
    /// The nest scale.
    pub mu: f64,
}

impl Default for NestedLogit {
    fn default() -> Self {
        Self { logit: Logit::default(), mu: 0.5 }
    }
}

impl NestedLogit {
    /// Make the model from its options: `mu`, and the [`Logit`]'s `beta_<attribute>`.
    ///
    /// # Errors
    ///
    /// [`ChoiceError::BadOption`] for a `mu` outside (0, 1]; otherwise as
    /// [`Logit::from_options`].
    pub fn from_options(options: &Options) -> Result<Self, ChoiceError> {
        let mut rest = options.clone();
        let mu = rest.remove("mu").unwrap_or(0.5);
        if !(mu > 0.0 && mu <= 1.0) {
            return Err(ChoiceError::BadOption {
                model: "nested_logit".to_string(),
                option: "mu".to_string(),
                reason: format!("must be above 0 and at most 1, got {mu}"),
            });
        }
        let logit = Logit::from_options(&rest).map_err(|e| match e {
            ChoiceError::UnknownOption { option, .. } => ChoiceError::UnknownOption {
                model: "nested_logit".to_string(),
                option,
                known: "mu, and beta_<attribute>, a coefficient per attribute".to_string(),
            },
            other => other,
        })?;
        Ok(Self { logit, mu })
    }

    /// The nests of one situation: each alternative's nest, and the nests in order of
    /// first appearance.
    fn nests(nest: &[f64]) -> (Vec<usize>, Vec<f64>) {
        let mut ids: Vec<f64> = Vec::new();
        let of = nest
            .iter()
            .map(|&n| match ids.iter().position(|&x| x.to_bits() == n.to_bits()) {
                Some(i) => i,
                None => {
                    ids.push(n);
                    ids.len() - 1
                }
            })
            .collect();
        (of, ids)
    }

    /// Every alternative's probability in one situation, from its utilities and nests.
    fn situation_probabilities(&self, utility: &[f64], nest: &[f64]) -> Vec<f64> {
        let (of, ids) = Self::nests(nest);
        let scaled: Vec<f64> = utility.iter().map(|v| v / self.mu).collect();
        let mut inclusive = vec![f64::NEG_INFINITY; ids.len()];
        for (n, value) in inclusive.iter_mut().enumerate() {
            let members: Vec<f64> =
                scaled.iter().zip(&of).filter(|&(_, &m)| m == n).map(|(v, _)| *v).collect();
            let top = members.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            *value = top + members.iter().map(|v| (v - top).exp()).sum::<f64>().ln();
        }
        let nest_p =
            logit_probabilities(&inclusive.iter().map(|i| self.mu * i).collect::<Vec<_>>());
        scaled.iter().zip(&of).map(|(v, &n)| nest_p[n] * (v - inclusive[n]).exp()).collect()
    }

    fn nest_column(batch: &ChoiceBatch) -> Result<&[f64], ChoiceError> {
        batch.attribute("nest").ok_or_else(|| ChoiceError::MissingAttribute {
            name: "nest".to_string(),
            offered: batch.attribute_names().to_vec(),
        })
    }
}

impl ChoiceModel for NestedLogit {
    fn name(&self) -> &str {
        "nested_logit"
    }

    fn descriptor(&self) -> String {
        let terms: Vec<String> =
            self.logit.betas.iter().map(|(a, b)| format!("beta_{a}={b}")).collect();
        format!("nested_logit;mu={};{}{}", self.mu, terms.join(";"), self.logit.class_terms())
    }

    fn is_sampled(&self) -> bool {
        true
    }

    fn required_attributes(&self) -> Option<Vec<String>> {
        let mut names = self.logit.required_attributes().unwrap_or_default();
        names.push("nest".to_string());
        Some(names)
    }

    fn choose(&self, batch: &ChoiceBatch, rng: &StreamRng) -> Result<Choices, ChoiceError> {
        let utility = self.logit.utilities(batch)?;
        let nest = Self::nest_column(batch)?;
        let noise = crate::model::gumbel_noise(batch, rng);
        let mut chosen = Vec::with_capacity(batch.situations());
        let mut probability = Vec::with_capacity(batch.situations());
        for s in 0..batch.situations() {
            let range = batch.range(s);
            let (u, n) = (&utility[range.clone()], &nest[range.clone()]);
            let (of, ids) = Self::nests(n);
            let p = self.situation_probabilities(u, n);
            // The nest: Gumbel-max over μ·Iₙ, each nest's error in the nests' own draw space.
            let mut best_nest = (f64::NEG_INFINITY, 0usize);
            for (k, id) in ids.iter().enumerate() {
                let members: Vec<f64> =
                    u.iter().zip(&of).filter(|&(_, &m)| m == k).map(|(v, _)| v / self.mu).collect();
                let top = members.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                let inclusive = top + members.iter().map(|v| (v - top).exp()).sum::<f64>().ln();
                #[allow(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "a nest number"
                )]
                let key = *id as i64 as u32;
                let g = crate::model::gumbel(
                    rng,
                    batch.travellers()[s],
                    batch.trips()[s],
                    batch.iteration() | NEST_KEY,
                    key,
                );
                let value = self.mu * inclusive + g;
                if value > best_nest.0 {
                    best_nest = (value, k);
                }
            }
            // The alternative inside it: Gumbel-max over Vⱼ / μ, keyed on identity.
            let mut best = (f64::NEG_INFINITY, 0usize);
            for (j, a) in range.clone().enumerate() {
                if of[j] != best_nest.1 {
                    continue;
                }
                let value = u[j] / self.mu + noise[a];
                if value > best.0 {
                    best = (value, j);
                }
            }
            #[allow(clippy::cast_possible_truncation, reason = "an index within one situation")]
            chosen.push(best.1 as u32);
            probability.push(p[best.1]);
        }
        Ok(Choices { chosen, probability })
    }

    fn probabilities(&self, batch: &ChoiceBatch) -> Result<Option<Vec<f64>>, ChoiceError> {
        let utility = self.logit.utilities(batch)?;
        let nest = Self::nest_column(batch)?;
        let mut out = Vec::with_capacity(batch.alternatives());
        for s in 0..batch.situations() {
            let range = batch.range(s);
            out.extend(self.situation_probabilities(&utility[range.clone()], &nest[range]));
        }
        Ok(Some(out))
    }

    /// `ln Σₙ exp(μ·Iₙ)`, `Iₙ = ln Σ_{j∈n} exp(Vⱼ/μ)`: the logsum of this nesting.
    fn logsums(&self, batch: &ChoiceBatch) -> Result<Option<Vec<f64>>, ChoiceError> {
        let utility = self.logit.utilities(batch)?;
        let nest = Self::nest_column(batch)?;
        let mut out = Vec::with_capacity(batch.situations());
        for s in 0..batch.situations() {
            let range = batch.range(s);
            let (u, n) = (&utility[range.clone()], &nest[range]);
            let (of, ids) = Self::nests(n);
            let tops = (0..ids.len()).map(|k| {
                let members = u.iter().zip(&of).filter(move |&(_, &m)| m == k);
                self.mu * crate::model::log_sum_exp(members.map(|(v, _)| v / self.mu))
            });
            out.push(crate::model::log_sum_exp(tops));
        }
        Ok(Some(out))
    }
}
