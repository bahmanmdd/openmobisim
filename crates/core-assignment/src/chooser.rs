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
//! **The switch share, its floor and the excess.** The switch share of an iteration is the
//! share of trips (weighted by traveller weight) whose chain after the choice differs from the
//! one before it (a trip with no chain before counts as switching). Its **floor** is what chance
//! alone would give at a stationary state, where the trips are already spread over their chains
//! as the model's probabilities say: a trip that re-chooses then lands on another chain with
//! probability `1 − Σ_k P_k²` (two independent draws from the same probabilities differ), so the
//! floor is `Σ_t w_t · α_t · (1 − Σ_k P_t,k²) / Σ_t w_t` with `α_t` the trip's re-choice
//! probability (1 for a trip that had no chain, which switches for certain), `P_t,k` the model's
//! probability of chain `k` for trip `t` at this iteration's costs, and `w_t` the traveller's
//! weight. Away from a stationary state the trips sit on chains the model would not put them
//! on, and the re-choosers switch more often than the floor says. The **excess** is that
//! difference per re-chooser: `(switch share − floor) / Σ_t w_t · α_t / Σ_t w_t`, the share of
//! the trips that reconsidered which moved beyond chance. Under a deterministic model the floor
//! is 0 and the excess is the share of trips not on their best chain. The floor is never built
//! from the chain a trip had before (that would make the switch share equal the floor by
//! construction, whatever the state).
//!
//! **Threads.** The re-choice draws and each set's path-size factors are worked out in
//! parallel, in fixed chunks of trips; every draw is addressed by identity and every factor
//! depends on its set alone, so the answers do not depend on the thread count. The choice model
//! is called once per stage with the whole batch, and the sums (switch share, floor) are taken
//! trip after trip in trip order, so their rounding is that of one thread.
//!
//! **Path sizes are kept.** A set's path-size factors depend only on its chains' links, which
//! never change, so they are computed when a set grows and kept between iterations
//! (`PathSizes`), not computed again for every batch.

use openmobisim_core_choice::model::first_argmax;
use openmobisim_core_choice::{ChoiceBatch, ChoiceError, ChoiceModel, Choices, Logit, Options};
use openmobisim_core_demand::{Travellers, Trips};
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_routes::path_sizes;
use openmobisim_core_types::ids::{EntityId, LinkId, TravellerId, TripId};
use openmobisim_core_types::rng::{DrawAddress, StreamRng};

use crate::attributes::Coefficients;
use crate::chain::Chain;
use crate::error::AssignError;
use crate::parallel;
use crate::state::State;

/// The trips a thread takes at a time in the chooser's parallel work.
const TRIPS_PER_CHUNK: usize = 512;

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

/// The logarithm of the path-size factor of every chain of every trip's set, kept between
/// iterations and computed again only for a set that grew (see the [module docs](self)).
#[derive(Debug, Default)]
pub(crate) struct PathSizes {
    ln: Vec<Vec<f64>>,
}

impl PathSizes {
    /// Bring the factors up to date with the sets of `state`.
    pub(crate) fn refresh(&mut self, network: &RoadNetwork, state: &State) {
        let trips = state.trips();
        self.ln.resize(trips.len(), Vec::new());
        let grown: Vec<usize> =
            (0..trips.len()).filter(|&t| self.ln[t].len() != trips[t].set.len()).collect();
        let fresh = parallel::map(
            &grown,
            TRIPS_PER_CHUNK,
            || (),
            |(), &t| ln_path_sizes(network, &trips[t].set),
        );
        for (t, ln) in grown.into_iter().zip(fresh) {
            self.ln[t] = ln;
        }
    }

    /// The factors of `trip`'s set, one per chain.
    fn of(&self, trip: TripId) -> &[f64] {
        &self.ln[trip.index()]
    }
}

/// The logarithm of each chain's path-size factor over `set`, by the length of its road links.
fn ln_path_sizes(network: &RoadNetwork, set: &[Chain]) -> Vec<f64> {
    let length = |l: u32| network.link_length(LinkId::new(l)).get();
    let links: Vec<Vec<u32>> = set.iter().map(Chain::road_links).collect();
    let routes: Vec<&[u32]> = links.iter().map(Vec::as_slice).collect();
    path_sizes(&routes, length).into_iter().map(f64::ln).collect()
}

/// The batch of `situations` at `iteration`: each set's chains with their utility at the
/// state's current attributes and their path size (`sizes`, up to date with the sets).
pub(crate) fn batch(
    demand: &Demand<'_>,
    state: &State,
    sizes: &PathSizes,
    situations: &[TripId],
    iteration: u32,
) -> ChoiceBatch {
    let mut batch = ChoiceBatch::new(iteration, &[UTILITY, LN_PATH_SIZE]);
    for &trip in situations {
        let traveller = demand.trips.traveller(trip);
        let class = demand.travellers.user_class(traveller);
        batch.begin_situation_in(traveller.raw(), trip.raw(), class.raw());
        let set = &state.trip(trip).set;
        let k = demand.coefficients(traveller);
        for (i, (chain, &ln_size)) in set.iter().zip(sizes.of(trip)).enumerate() {
            let identity = u32::try_from(i).expect("fewer than 2^32 chains in a set");
            batch.push_alternative(identity, &[k.utility(chain.total()), ln_size]);
        }
    }
    batch
}

/// The model's probability of every alternative of a batch, read by situation.
pub(crate) struct Probabilities {
    flat: Vec<f64>,
    offsets: Vec<u32>,
}

impl Probabilities {
    /// The probabilities of situation `s`'s alternatives.
    pub(crate) fn of(&self, s: usize) -> &[f64] {
        &self.flat[self.offsets[s] as usize..self.offsets[s + 1] as usize]
    }
}

/// The model's probability of every alternative of `batch`; `NaN` everywhere if the model has
/// none to give.
pub(crate) fn probabilities(
    model: &dyn ChoiceModel,
    batch: &ChoiceBatch,
) -> Result<Probabilities, AssignError> {
    let flat = model.probabilities(batch)?.unwrap_or_else(|| vec![f64::NAN; batch.alternatives()]);
    Ok(Probabilities { flat, offsets: batch.offsets().to_vec() })
}

/// What the choose stage measured: the switch share, its floor and the excess per re-chooser.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Switching {
    pub share: f64,
    pub floor: f64,
    pub excess: f64,
}

/// The choose stage: every trip facing a choice is offered its set; the travellers the
/// averaging rule selects (and every trip without a chain) take the model's choice, the others
/// keep theirs. Writes the new current chains and the probabilities into `state`.
#[allow(clippy::too_many_arguments, reason = "the stage's inputs, each named")]
pub(crate) fn choose(
    demand: &Demand<'_>,
    state: &mut State,
    sizes: &PathSizes,
    model: &dyn ChoiceModel,
    choice_rng: &StreamRng,
    reselection_rng: &StreamRng,
    iteration: u32,
    offset: u32,
) -> Result<Switching, AssignError> {
    let situations = situations(state);
    let batch = batch(demand, state, sizes, &situations, iteration);
    batch.validate()?;
    let probabilities = probabilities(model, &batch)?;
    let alpha = rechoice_probability(iteration, offset);
    // Per situation, whether its trip had no chain and whether it chooses now.
    let drawn: Vec<(bool, bool)> = {
        let state = &*state;
        parallel::map(
            &situations,
            TRIPS_PER_CHUNK,
            || (),
            |(), &trip| {
                let none = state.trip(trip).current.is_none();
                let traveller = demand.trips.traveller(trip);
                (none, none || rechooses(reselection_rng, traveller, iteration, offset))
            },
        )
    };
    let mut choosing: Vec<usize> = Vec::new();
    let mut chance: Vec<f64> = Vec::with_capacity(situations.len());
    for (s, &(none, chooses)) in drawn.iter().enumerate() {
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
    let (mut total, mut switched, mut floor, mut reconsidering) = (0.0, 0.0, 0.0, 0.0);
    for (s, &trip) in situations.iter().enumerate() {
        let w = demand.weight(demand.trips.traveller(trip));
        let entry = &mut state.trips_mut()[trip.index()];
        let before = entry.current;
        let after = picked[s].or(before);
        // The chance that two independent draws from the model's probabilities differ.
        let differs = 1.0 - probabilities.of(s).iter().map(|p| p * p).sum::<f64>();
        total += w;
        reconsidering += w * chance[s];
        floor += w * chance[s] * if before.is_none() { 1.0 } else { differs };
        if after != before {
            switched += w;
        }
        entry.current = after;
        entry.probabilities.clear();
        entry.probabilities.extend_from_slice(probabilities.of(s));
    }
    let share = |x: f64| if total > 0.0 { x / total } else { 0.0 };
    let excess = if reconsidering > 0.0 { (switched - floor) / reconsidering } else { 0.0 };
    Ok(Switching { share: share(switched), floor: share(floor), excess })
}
