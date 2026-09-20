//! The built-in choice models.
//!
//! | name | what it does |
//! |---|---|
//! | `deterministic` (the default, [`DEFAULT_MODEL`](crate::DEFAULT_MODEL)) | Everyone takes the alternative with the least `time_min`: all-or-nothing, for debugging and upper bounds |
//! | `logit` | A random utility model, linear in named attributes, sampled exactly by the Gumbel-max construction; with the path-size term it is the standard **path-size logit** for routes |

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
/// | `beta_time_min` | −0.2 | travel time, minutes |
/// | `beta_ln_path_size` | 1 | ln of the path-size factor: routes that share links with the others in the set split their share (Ben-Akiva and Bierlaire, 1999), which repairs the logit's blindness to overlap |
///
/// Any other attribute on offer takes a coefficient the same way — for routes
/// `beta_length_km`, `beta_detour`, `beta_overlap`, `beta_n_links` — and one not on
/// offer is refused when the run starts, with the list of those that are. Set a
/// default to `0` to drop its term.
///
/// **The defaults are an assumption, not a calibration**: a time coefficient of
/// −0.2 per minute means a route four minutes slower is taken with about 45% of
/// the weight of one that is not, other things equal. Estimate your own and pass them.
#[derive(Clone, Debug, PartialEq)]
pub struct Logit {
    /// `(attribute, coefficient)`, sorted by attribute.
    pub betas: Vec<(String, f64)>,
}

impl Default for Logit {
    fn default() -> Self {
        Self { betas: vec![("ln_path_size".to_string(), 1.0), ("time_min".to_string(), -0.2)] }
    }
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
            match m.betas.iter_mut().find(|(a, _)| a == attribute) {
                Some(entry) => entry.1 = *value,
                None => m.betas.push((attribute.to_string(), *value)),
            }
        }
        m.betas.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(m)
    }

    /// The utility of every alternative in `batch`.
    ///
    /// # Errors
    ///
    /// [`ChoiceError::MissingAttribute`] if a coefficient names an attribute the
    /// batch does not carry.
    pub fn utilities(&self, batch: &ChoiceBatch) -> Result<Vec<f64>, ChoiceError> {
        let mut utility = vec![0.0; batch.alternatives()];
        for (attribute, beta) in &self.betas {
            if *beta == 0.0 {
                continue;
            }
            let column =
                batch.attribute(attribute).ok_or_else(|| ChoiceError::MissingAttribute {
                    name: attribute.clone(),
                    offered: batch.attribute_names().to_vec(),
                })?;
            for (u, v) in utility.iter_mut().zip(column) {
                *u += beta * v;
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
    pub fn probabilities(&self, batch: &ChoiceBatch) -> Result<Vec<Vec<f64>>, ChoiceError> {
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
        format!("logit;{}", terms.join(";"))
    }

    fn is_sampled(&self) -> bool {
        true
    }

    fn required_attributes(&self) -> Option<Vec<String>> {
        Some(self.betas.iter().filter(|(_, b)| *b != 0.0).map(|(a, _)| a.clone()).collect())
    }

    fn choose(&self, batch: &ChoiceBatch, rng: &StreamRng) -> Result<Choices, ChoiceError> {
        Ok(sample_random_utility(batch, &self.utilities(batch)?, rng))
    }
}
