//! The chooser: one choice over every trip's chains, with the averaging rule.
//!
//! **What the choice model sees.** For every trip with a non-empty choice set, the chooser
//! hands the run's choice model ([`ChoiceModel`], from `core-choice`) one situation whose
//! alternatives are the set's chains, each with two attributes:
//!
//! - `utility`: the chain's systematic utility by the trip's user class's coefficients
//!   ([`crate::Coefficients::utility`]), in utility units;
//! - `ln_path_size`: the logarithm of the chain's path-size factor over the set (Ben-Akiva and
//!   Bierlaire, 1999; computed by `core-routes`' `path_sizes` over the road links of the
//!   set's chains, by length): 0 for a set of one chain or for chains that share no link,
//!   negative for chains that overlap others.
//!
//! An alternative's identity in the batch is its index in the set, which is stable across
//! iterations because sets only grow: the model's random draws, addressed by
//! (traveller, trip, iteration, identity), stay with the chain.
//!
//! **The two models of the kernel.** [`HighestUtility`], the deterministic choice: every
//! re-choosing traveller takes the chain of highest systematic utility (all-or-nothing); of
//! chains of exactly equal utility, **the first in the set** (the one found earliest), with
//! probability 1. [`logit`]: the multinomial logit on the same utility, with the path-size term
//! (`P_k = exp(V_k + ln PS_k) / Σ_j exp(V_j + ln PS_j)`), sampled exactly by the Gumbel-max
//! construction of `core-choice`'s `Logit`. The utility's scale is the coefficients' own: a
//! time-based run's logit weighs one minute as one unit of utility.
//!
//! **The averaging rule** (the method of successive averages applied per traveller). In
//! assignment iteration `i ≥ 1`, each traveller re-chooses with probability
//! `α_i = 1 / (i + 1 + offset)` (offset 3 by default) and otherwise keeps every one of their
//! chains; in iteration 0 everyone chooses (`α_0 = 1`), and so does a trip that has no chain yet.
//! Whether a traveller re-chooses is a draw of the run's re-selection stream addressed by
//! (traveller, iteration), so the answer does not depend on the order or the number of threads
//! that compute it.
//!
//! **The switch share and its floor.** The switch share of an iteration is the share of trips
//! (weighted by traveller weight) whose chain after the choice differs from the one before it
//! (a trip with no chain before counts as switching). Its **floor** is what chance alone would
//! give if nothing changed: `Σ_t w_t · α_t · (1 − P_t(current)) / Σ_t w_t`, where `P_t(current)`
//! is the probability the model now gives the chain the trip had before the choice (0 for a trip
//! that had none), `α_t` the trip's re-choice probability and `w_t` its traveller's weight. At a
//! stationary state the switch share is the floor plus sampling noise.
//!
//! The chooser runs on one thread; every draw is addressed by identity, so splitting it over
//! threads would change no result.

use openmobisim_core_choice::model::first_argmax;
use openmobisim_core_choice::{ChoiceBatch, ChoiceError, ChoiceModel, Choices, Logit, Options};
use openmobisim_core_demand::{Travellers, Trips};
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_routes::path_sizes;
use openmobisim_core_types::ids::{EntityId, LinkId, TravellerId, TripId};
use openmobisim_core_types::rng::{DrawAddress, StreamRng};

use crate::attributes::Coefficients;
use crate::error::AssignError;
use crate::state::State;

/// The name of the batch attribute holding a chain's systematic utility.
pub const UTILITY: &str = "utility";
/// The name of the batch attribute holding the logarithm of a chain's path-size factor.
pub const LN_PATH_SIZE: &str = "ln_path_size";

/// The deterministic choice: the alternative of highest `utility`, the first of any that tie,
/// with probability 1. See the [module docs](self).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HighestUtility;

impl HighestUtility {
    fn picks(batch: &ChoiceBatch) -> Result<Vec<u32>, ChoiceError> {
        let utility = batch.attribute(UTILITY).ok_or_else(|| ChoiceError::MissingAttribute {
            name: UTILITY.to_string(),
            offered: batch.attribute_names().to_vec(),
        })?;
        Ok((0..batch.situations())
            .map(|s| {
                let pick = first_argmax(&utility[batch.range(s)]);
                u32::try_from(pick).expect("an index within one choice set")
            })
            .collect())
    }
}

impl ChoiceModel for HighestUtility {
    fn name(&self) -> &str {
        "deterministic"
    }

    fn descriptor(&self) -> String {
        "deterministic;utility".to_string()
    }

    fn is_sampled(&self) -> bool {
        false
    }

    fn required_attributes(&self) -> Option<Vec<String>> {
        Some(vec![UTILITY.to_string()])
    }

    fn choose(&self, batch: &ChoiceBatch, _rng: &StreamRng) -> Result<Choices, ChoiceError> {
        let chosen = Self::picks(batch)?;
        let probability = vec![1.0; chosen.len()];
        Ok(Choices { chosen, probability })
    }

    fn probabilities(&self, batch: &ChoiceBatch) -> Result<Option<Vec<f64>>, ChoiceError> {
        let mut p = vec![0.0; batch.alternatives()];
        for (s, c) in Self::picks(batch)?.into_iter().enumerate() {
            p[batch.range(s).start + c as usize] = 1.0;
        }
        Ok(Some(p))
    }
}

/// The multinomial logit on the chains' systematic utility with the path-size term:
/// `core-choice`'s `Logit` with a coefficient of 1 on `utility` and of 1 on `ln_path_size`.
///
/// # Panics
///
/// Never: the options are valid by construction.
#[must_use]
pub fn logit() -> Logit {
    let options: Options = [(format!("beta_{UTILITY}"), 1.0)].into_iter().collect();
    Logit::from_options(&options).expect("a coefficient on one named attribute is valid")
}

/// The probability that a traveller re-chooses in `iteration` under the averaging rule:
/// 1 in iteration 0, else `1 / (iteration + 1 + offset)`.
#[must_use]
pub fn rechoice_probability(iteration: u32, offset: u32) -> f64 {
    if iteration == 0 { 1.0 } else { 1.0 / (f64::from(iteration) + 1.0 + f64::from(offset)) }
}

/// Whether `traveller` re-chooses in `iteration`: a draw of the re-selection stream `rng` at
/// (traveller, iteration) below [`rechoice_probability`].
#[must_use]
pub fn rechooses(rng: &StreamRng, traveller: TravellerId, iteration: u32, offset: u32) -> bool {
    iteration == 0
        || rng.unit(DrawAddress::from_pair(traveller.raw(), iteration))
            < rechoice_probability(iteration, offset)
}

/// What the run's fixed inputs give the chooser.
pub(crate) struct Demand<'a> {
    pub network: &'a RoadNetwork,
    pub travellers: &'a Travellers,
    pub trips: &'a Trips,
    pub classes: &'a [Coefficients],
}

impl Demand<'_> {
    /// The traveller's weight.
    pub fn weight(&self, traveller: TravellerId) -> f64 {
        f64::from(self.travellers.weight(traveller))
    }

    /// The coefficients of `traveller`'s class.
    pub fn coefficients(&self, traveller: TravellerId) -> &Coefficients {
        &self.classes[self.travellers.user_class(traveller).index()]
    }
}

/// The trips that face a choice (a non-empty set), in trip order.
pub(crate) fn situations(state: &State) -> Vec<TripId> {
    state
        .trips()
        .iter()
        .enumerate()
        .filter(|(_, t)| !t.set.is_empty())
        .map(|(i, _)| TripId::from_index(i))
        .collect()
}

/// The batch of `situations` at `iteration`: each set's chains with their utility and path size
/// at the state's current attributes.
pub(crate) fn batch(
    demand: &Demand<'_>,
    state: &State,
    situations: &[TripId],
    iteration: u32,
) -> ChoiceBatch {
    let mut batch = ChoiceBatch::new(iteration, &[UTILITY, LN_PATH_SIZE]);
    let length = |l: u32| demand.network.link_length(LinkId::new(l)).get();
    for &trip in situations {
        let traveller = demand.trips.traveller(trip);
        let class = demand.travellers.user_class(traveller);
        batch.begin_situation_in(traveller.raw(), trip.raw(), class.raw());
        let set = &state.trip(trip).set;
        let links: Vec<Vec<u32>> = set.iter().map(crate::chain::Chain::road_links).collect();
        let routes: Vec<&[u32]> = links.iter().map(Vec::as_slice).collect();
        let sizes = path_sizes(&routes, length);
        let k = demand.coefficients(traveller);
        for (i, (chain, size)) in set.iter().zip(sizes).enumerate() {
            let identity = u32::try_from(i).expect("fewer than 2^32 chains in a set");
            batch.push_alternative(identity, &[k.utility(chain.total()), size.ln()]);
        }
    }
    batch
}

/// The model's probability of every alternative of `batch`, by situation; `NaN` everywhere if
/// the model has none to give.
pub(crate) fn probabilities(
    model: &dyn ChoiceModel,
    batch: &ChoiceBatch,
) -> Result<Vec<Vec<f64>>, AssignError> {
    let flat = model.probabilities(batch)?;
    Ok((0..batch.situations())
        .map(|s| match &flat {
            Some(p) => p[batch.range(s)].to_vec(),
            None => vec![f64::NAN; batch.range(s).len()],
        })
        .collect())
}

/// What the choose stage measured: the switch share and its floor.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Switching {
    pub share: f64,
    pub floor: f64,
}

/// The choose stage: every trip facing a choice is offered its set; the travellers the
/// averaging rule selects (and every trip without a chain) take the model's choice, the others
/// keep theirs. Writes the new current chains and the probabilities into `state`.
pub(crate) fn choose(
    demand: &Demand<'_>,
    state: &mut State,
    model: &dyn ChoiceModel,
    choice_rng: &StreamRng,
    reselection_rng: &StreamRng,
    iteration: u32,
    offset: u32,
) -> Result<Switching, AssignError> {
    let situations = situations(state);
    let batch = batch(demand, state, &situations, iteration);
    batch.validate()?;
    let probabilities = probabilities(model, &batch)?;
    let alpha = rechoice_probability(iteration, offset);
    let mut choosing: Vec<usize> = Vec::new();
    let mut chance: Vec<f64> = Vec::with_capacity(situations.len());
    for (s, &trip) in situations.iter().enumerate() {
        let traveller = demand.trips.traveller(trip);
        let none = state.trip(trip).current.is_none();
        let chooses = none || rechooses(reselection_rng, traveller, iteration, offset);
        chance.push(if none { 1.0 } else { alpha });
        if chooses {
            choosing.push(s);
        }
    }
    let offered = batch.subset(&choosing);
    let choices = model.choose(&offered, choice_rng)?;
    choices.validate(&offered)?;
    let mut picked: Vec<Option<usize>> = vec![None; situations.len()];
    for (k, &s) in choosing.iter().enumerate() {
        picked[s] = Some(choices.chosen[k] as usize);
    }
    let (mut total, mut switched, mut floor) = (0.0, 0.0, 0.0);
    for (s, &trip) in situations.iter().enumerate() {
        let w = demand.weight(demand.trips.traveller(trip));
        let entry = &mut state.trips_mut()[trip.index()];
        let before = entry.current;
        let after = picked[s].or(before);
        let p_before = before.map_or(0.0, |c| probabilities[s][c]);
        total += w;
        floor += w * chance[s] * (1.0 - p_before);
        if after != before {
            switched += w;
        }
        entry.current = after;
        entry.probabilities.clone_from(&probabilities[s]);
    }
    let share = |x: f64| if total > 0.0 { x / total } else { 0.0 };
    Ok(Switching { share: share(switched), floor: share(floor) })
}
