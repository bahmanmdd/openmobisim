//! The chooser: one choice over every trip's chains, with the averaging rule.
//!
//! **What the choice model sees.** For every trip with a non-empty choice set, the chooser
//! hands the run's choice model ([`ChoiceModel`], from `core-choice`) one situation whose
//! alternatives are the set's chains, each with five attributes, and a sixth for a nested logit:
//!
//! - `utility`: the chain's systematic utility by the trip's user class's coefficients, its
//!   mode's constant included ([`crate::Coefficients::chain_utility`]), in utility units;
//! - `ln_path_size`: the logarithm of the chain's path-size factor over the set (Ben-Akiva and
//!   Bierlaire, 1999; computed by `core-routes`' `path_sizes` over the links of the set's
//!   chains, each on its own layer, by length: a road link and a bike-layer link are different
//!   links even where they share a number): 0 for a set of one chain or for chains that share no
//!   link, negative for chains that overlap others;
//! - `indifference_band`: the run's indifference band ε ([`crate::Params::indifference_band`]),
//!   the same for every alternative;
//! - `chain_high` and `chain_low`: the upper and lower 32 bits of the chain's identity
//!   ([`Chain::identity`]), each exact in a double, so a model can tell which trips face the
//!   same chains;
//! - `nest` (only for a model that reads it): the chain's mode, as a number (the car 0, the bike
//!   1, walking 2, transit 3: `core-demand`'s mode numbers), so a nested logit puts each mode in
//!   a nest of its own.
//!
//! The logit weighs only `utility` and `ln_path_size`; the nested logit the same, in nests by
//! `nest`; the band and the identities are for the deterministic choice.
//!
//! **Mode choice is this choice.** A trip choosing its mode has chains of several modes in its
//! set ([`crate::modes`]), and the same model chooses among them as among a car trip's routes:
//! the deterministic choice takes the cheapest chain (ties within the band split evenly), the
//! logit weighs every chain of the set by `exp(V + ln PS)`, and the **nested logit** with nest
//! scale `μ` (`core-choice`'s `NestedLogit`, the alternatives nested by `nest`) takes mode `m`
//! with probability `exp(μ I_m) / Σ_n exp(μ I_n)`, `I_m = ln Σ_{k ∈ m} exp((V_k + ln PS_k) / μ)`,
//! and chain `k` of mode `m` with probability `exp((V_k + ln PS_k) / μ − I_m)`: μ = 1 is the
//! logit over the whole set; a smaller μ makes the routes of one mode closer substitutes for each
//! other than for the other modes, so a mode does not gain share from the number of its routes.
//! Chains of different modes share no link, so a bike or walking chain's path size is 1 when it
//! is its mode's only chain; the bike chains of a sampled set ([`crate::BikeRoutes::Sampled`])
//! share bike-layer links with one another, and their path sizes say by how much.
//!
//! An alternative's identity in the batch is its index in the set, which is stable across
//! iterations because sets only grow: the model's random draws, addressed by
//! (traveller, trip, iteration, identity), stay with the chain.
//!
//! **The two models of the kernel.** [`HighestUtility`], the deterministic choice with an
//! **indifference band**: the chains whose cost is within the band of the set's least cost,
//! `cost_k ≤ cost_best + ε · |cost_best|` (that is `cost_best · (1 + ε)` for a positive cost),
//! **tie** with the best, and the model gives each of the `k` tied chains probability `1 / k`
//! and every other chain 0. A re-choosing traveller takes one of the tied chains, assigned by
//! an **even split in a fixed order, with no random draw**: among the trips that choose in this
//! iteration, those facing the same tie (the same set of tied chains, by chain identity) are
//! ordered by trip id, and the n-th of them (from 0) takes the group's tied chain
//! `(n + i) mod k` in assignment iteration `i`, the tied chains taken in their order in the set
//! of the group's first trip. So of `N` trips facing one tie of `k` chains in one iteration,
//! each chain receives `⌊N / k⌋` or `⌈N / k⌉`; the iteration turns the order so that a tie
//! faced by a single re-chooser an iteration, as is common late in a run, is still split
//! evenly over the iterations rather than always sent to the same chain. The order is by trip id alone:
//! the split does not depend on link or node ids (only on which chains tie) nor on the number
//! of threads (the choice is made trip after trip on one thread). With `ε = 0` only chains of
//! exactly equal cost tie. [`logit`]: the multinomial logit on the same utility, with the
//! path-size term (`P_k = exp(V_k + ln PS_k) / Σ_j exp(V_j + ln PS_j)`), sampled exactly by the
//! Gumbel-max construction of `core-choice`'s `Logit`; the band does not enter it. The
//! utility's scale is the coefficients' own: a time-based run's logit weighs one minute as one
//! unit of utility.
//!
//! **The averaging rule** (the method of successive averages applied per traveller). In
//! assignment iteration `i ≥ 1`, each traveller re-chooses with probability
//! `α_i = 1 / (i + 1 + offset)` (offset 3 by default) and otherwise keeps every one of their
//! chains; in iteration 0 everyone chooses (`α_0 = 1`), and so does a trip that has no chain yet.
//! Whether a traveller re-chooses is a draw of the run's re-selection stream addressed by
//! (traveller, iteration), so the answer does not depend on the order or the number of threads
//! that compute it. That is the default allocation, [`Allocation::Draws`]; the quota allocations
//! ([`crate::quota`], [`crate::Params::allocation`]) replace the draws by the quota of each group of
//! trips facing the same choice: of who reconsiders (`⌊α N⌋` or `⌈α N⌉` of a group of `N`), and
//! of what the re-choosers take (the model's probabilities, `N P_k` of them on chain `k`), without
//! calling the model's own choice. They are worked out on one thread, group by group, in trip
//! order.
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
//! the trips that reconsidered which moved beyond chance. Under the deterministic model the
//! probabilities are those of the band (`1 / k` over the `k` tied chains), so the floor
//! counts the chance that a re-chooser facing a tie lands on another tied chain,
//! `α · (1 − 1 / k)`, and the excess is the **excess beyond the band**: the share of the
//! re-choosers that were on a chain dearer than the band allows, or that the even split moved
//! more often than chance would; at a stationary state it is 0 within the even split's rounding,
//! with no special case for the deterministic model.
//!
//! **The expected excess.** The realised excess is a draw: which travellers reconsider is
//! random, and late in a run so few do that its noise grows as `α` falls. Its expectation over
//! the whole population has no such noise: before the choice, with `P_t` the model's
//! probabilities for trip `t` at the costs the last loading produced and `c_t` the chain the
//! trip has, `Σ_t w_t (Σ_k P_t,k² − P_t,c_t) / Σ_t w_t`, a trip with no chain yet contributing
//! 0 (it moves for certain, as its floor says, as in the realised excess). It is the share of trips that would change chain beyond the stationary chance if
//! everyone reconsidered: a trip that reconsiders moves with probability `1 − P_t,c_t`, and at a
//! stationary state with probability `1 − Σ_k P_t,k²`. Under the deterministic choice with its
//! uniform ties it is the share of trips on a chain beyond the band of their best (a trip on a
//! tied chain contributes `1/k − 1/k = 0`, a trip off its best `1`, when the best is unique).
//! It can be negative for the logit, when the trips are more concentrated on their likely
//! chains than the probabilities say; the stopping rule reads its absolute value.
//!
//! The floor is never built
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

use std::collections::BTreeMap;

use openmobisim_core_choice::model::first_argmax;
use openmobisim_core_choice::{ChoiceBatch, ChoiceError, ChoiceModel, Choices, Logit, Options};
use openmobisim_core_demand::{Mode, Travellers, Trips};
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_routes::path_sizes;
use openmobisim_core_types::ids::{EntityId, LinkId, TravellerId, TripId};
use openmobisim_core_types::rng::{DrawAddress, StreamRng};

use crate::attributes::Coefficients;
use crate::chain::Chain;
use crate::error::AssignError;
use crate::modes::TripModes;
use crate::parallel;
use crate::quota::{self, Allocation};
use crate::report::PerMode;
use crate::state::State;

/// The trips a thread takes at a time in the chooser's parallel work.
const TRIPS_PER_CHUNK: usize = 512;

/// The name of the batch attribute holding a chain's systematic utility.
pub const UTILITY: &str = "utility";
/// The name of the batch attribute holding the logarithm of a chain's path-size factor.
pub const LN_PATH_SIZE: &str = "ln_path_size";
/// The name of the batch attribute holding the run's indifference band (relative).
pub const INDIFFERENCE_BAND: &str = "indifference_band";
/// The name of the batch attribute holding the upper 32 bits of a chain's identity.
pub const CHAIN_HIGH: &str = "chain_high";
/// The name of the batch attribute holding the lower 32 bits of a chain's identity.
pub const CHAIN_LOW: &str = "chain_low";
/// The name of the batch attribute holding a chain's mode as a number: a nested logit's nest.
pub const NEST: &str = "nest";

/// The deterministic choice with the indifference band: every chain within the band of the
/// best ties with it, each tied chain has probability `1 / k`, and the trips facing the same
/// tie are split evenly over its chains by trip id, with no random draw. See the
/// [module docs](self).
///
/// It reads `utility`, and `indifference_band`, `chain_high` and `chain_low` when the batch
/// carries them (the chooser's batches do); without them the band is 0 (only chains of exactly
/// equal utility tie) and a chain is known by its identity in the batch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HighestUtility;

/// What [`HighestUtility`] reads from a batch.
struct Columns<'a> {
    utility: &'a [f64],
    band: Option<&'a [f64]>,
    chain: Option<(&'a [f64], &'a [f64])>,
}

impl<'a> Columns<'a> {
    fn of(batch: &'a ChoiceBatch) -> Result<Self, ChoiceError> {
        let utility = batch.attribute(UTILITY).ok_or_else(|| ChoiceError::MissingAttribute {
            name: UTILITY.to_string(),
            offered: batch.attribute_names().to_vec(),
        })?;
        let band = batch.attribute(INDIFFERENCE_BAND);
        let chain = batch.attribute(CHAIN_HIGH).zip(batch.attribute(CHAIN_LOW));
        Ok(Self { utility, band, chain })
    }

    /// The alternatives of the situation at `range` that tie with its best, as offsets into
    /// the situation, in their order: those whose utility is within the band of the highest,
    /// `u_best − u_k ≤ ε · |u_best|` (the cost `−u` within `ε · |cost_best|` of the least).
    fn tied(&self, range: std::ops::Range<usize>) -> Vec<usize> {
        let utility = &self.utility[range.clone()];
        let best = utility[first_argmax(utility)];
        let band = self.band.map_or(0.0, |b| b[range.start].max(0.0));
        let slack = band * best.abs();
        (0..utility.len()).filter(|&k| best - utility[k] <= slack).collect()
    }

    /// The identity of alternative `a` (an index into the batch's alternatives): the chain's,
    /// or its identity in the batch for a batch without the chains' identities.
    fn identity(&self, batch: &ChoiceBatch, a: usize) -> u64 {
        match self.chain {
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "each half was written from a u32, exactly"
            )]
            Some((high, low)) => (u64::from(high[a] as u32) << 32) | u64::from(low[a] as u32),
            None => u64::from(batch.identities()[a]),
        }
    }
}

impl HighestUtility {
    /// Per situation, the chosen alternative (an offset into the situation) and the number of
    /// chains it tied with, itself included: the even split of each tie over the trips facing
    /// it, by trip id.
    fn picks(batch: &ChoiceBatch) -> Result<Vec<(u32, usize)>, ChoiceError> {
        let columns = Columns::of(batch)?;
        let n = batch.situations();
        let tied: Vec<Vec<usize>> = (0..n).map(|s| columns.tied(batch.range(s))).collect();
        // The situations by trip id (the batch's order where the ids are equal).
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by_key(|&s| (batch.trips()[s], s));
        // Per tie (its chains' identities, sorted): how many trips took one of its chains so
        // far, and its chains in the order of the first trip's set.
        let mut ties: BTreeMap<Vec<u64>, (usize, Vec<u64>)> = BTreeMap::new();
        let mut picks = vec![(0_u32, 1_usize); n];
        // The iteration turns the order, so that a tie faced by one re-chooser an iteration
        // (late in a run, when few reconsider) is still split evenly over the iterations.
        let turn = batch.iteration() as usize;
        for s in order {
            let offsets = &tied[s];
            let start = batch.range(s).start;
            let k = offsets.len();
            let pick = if k == 1 {
                offsets[0]
            } else {
                let ids: Vec<u64> =
                    offsets.iter().map(|&o| columns.identity(batch, start + o)).collect();
                let mut key = ids.clone();
                key.sort_unstable();
                let (taken, chains) = ties.entry(key).or_insert_with(|| (0, ids.clone()));
                let wanted = chains[(*taken + turn) % k];
                *taken += 1;
                offsets[ids.iter().position(|&i| i == wanted).expect("the tie's own chain")]
            };
            picks[s] = (u32::try_from(pick).expect("an index within one choice set"), k);
        }
        Ok(picks)
    }
}

impl ChoiceModel for HighestUtility {
    fn name(&self) -> &str {
        "deterministic"
    }

    fn descriptor(&self) -> String {
        "deterministic;utility;indifference_band".to_string()
    }

    fn is_sampled(&self) -> bool {
        false
    }

    fn required_attributes(&self) -> Option<Vec<String>> {
        Some(vec![UTILITY.to_string()])
    }

    fn choose(&self, batch: &ChoiceBatch, _rng: &StreamRng) -> Result<Choices, ChoiceError> {
        let picks = Self::picks(batch)?;
        #[allow(clippy::cast_precision_loss, reason = "a handful of tied chains")]
        let probability = picks.iter().map(|&(_, k)| 1.0 / k as f64).collect();
        let chosen = picks.into_iter().map(|(c, _)| c).collect();
        Ok(Choices { chosen, probability })
    }

    fn probabilities(&self, batch: &ChoiceBatch) -> Result<Option<Vec<f64>>, ChoiceError> {
        let columns = Columns::of(batch)?;
        let mut p = vec![0.0; batch.alternatives()];
        for s in 0..batch.situations() {
            let range = batch.range(s);
            let tied = columns.tied(range.clone());
            #[allow(clippy::cast_precision_loss, reason = "a handful of tied chains")]
            let each = 1.0 / tied.len() as f64;
            for k in tied {
                p[range.start + k] = each;
            }
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
    /// Per trip, the modes its chains may have.
    pub modes: &'a [TripModes],
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

    /// Whether `trip` could change its choice (counted in the stopping shares).
    pub fn could_change(&self, trip: TripId) -> bool {
        self.modes[trip.index()].could_change()
    }

    /// Whether `trip` chooses its mode.
    pub fn chooses_mode(&self, trip: TripId) -> bool {
        self.modes[trip.index()].chooses_mode()
    }
}

/// The slot of `mode` in sums by mode (car, bike, walking, transit); `None` for another mode.
pub(crate) fn mode_slot(mode: Mode) -> Option<usize> {
    match mode {
        Mode::Car => Some(0),
        Mode::Bike => Some(1),
        Mode::Walk => Some(2),
        Mode::Transit => Some(3),
        _ => None,
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
    pub(crate) fn refresh(&mut self, lengths: &LinkLengths<'_>, state: &State) {
        let trips = state.trips();
        self.ln.resize(trips.len(), Vec::new());
        let grown: Vec<usize> =
            (0..trips.len()).filter(|&t| self.ln[t].len() != trips[t].set.len()).collect();
        let fresh = parallel::map(
            &grown,
            TRIPS_PER_CHUNK,
            || (),
            |(), &t| ln_path_sizes(lengths, &trips[t].set),
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

/// The lengths of every layer's links, under one numbering: the road network's links first (by
/// their own numbers), then the bike layer's, then the walk layer's, so a link of one layer is
/// never taken for a link of another.
pub(crate) struct LinkLengths<'a> {
    road: &'a RoadNetwork,
    bike: Option<&'a RoadNetwork>,
    walk: Option<&'a RoadNetwork>,
}

impl<'a> LinkLengths<'a> {
    /// The lengths of the layers of `inputs`.
    pub(crate) fn of(inputs: &crate::assign::Inputs<'a>) -> Self {
        Self {
            road: inputs.network,
            bike: inputs.layers.bike.map(|l| l.network.network()),
            walk: inputs.layers.walk.map(|l| l.network.network()),
        }
    }

    fn links(network: Option<&RoadNetwork>) -> u32 {
        network.map_or(0, RoadNetwork::link_count)
    }

    /// The number of `link` of a `mode` leg under the one numbering.
    fn key(&self, mode: Mode, link: LinkId) -> u32 {
        match mode {
            Mode::Bike => self.road.link_count() + link.raw(),
            Mode::Walk => self.road.link_count() + Self::links(self.bike) + link.raw(),
            _ => link.raw(),
        }
    }

    /// The length in metres of the link numbered `key`.
    fn length(&self, key: u32) -> f64 {
        let road = self.road.link_count();
        let bike = Self::links(self.bike);
        let (network, link) = if key < road {
            (Some(self.road), key)
        } else if key < road + bike {
            (self.bike, key - road)
        } else {
            (self.walk, key - road - bike)
        };
        network.map_or(0.0, |n| n.link_length(LinkId::new(link)).get())
    }

    /// Every link of `chain`, leg after leg, under the one numbering.
    fn chain(&self, chain: &Chain) -> Vec<u32> {
        chain
            .legs()
            .iter()
            .flat_map(|leg| leg.links.iter().map(move |&l| self.key(leg.mode, l)))
            .collect()
    }
}

/// The logarithm of each chain's path-size factor over `set`: a car, bike or walking chain's
/// over the set's other such chains, by the length of its links; a transit chain's over the
/// set's transit chains, by its shared rides ([`crate::transit::ride_path_sizes`]). Chains of
/// different modes share nothing, so the two are worked out apart.
fn ln_path_sizes(lengths: &LinkLengths<'_>, set: &[Chain]) -> Vec<f64> {
    let is_transit = |c: &Chain| c.mode() == Mode::Transit;
    let transit: Vec<&Chain> = set.iter().filter(|c| is_transit(c)).collect();
    let links: Vec<Vec<u32>> =
        set.iter().filter(|c| !is_transit(c)).map(|c| lengths.chain(c)).collect();
    let routes: Vec<&[u32]> = links.iter().map(Vec::as_slice).collect();
    let mut by_links = path_sizes(&routes, |k| lengths.length(k)).into_iter().map(f64::ln);
    let mut by_rides = crate::transit::ride_path_sizes(&transit).into_iter();
    set.iter()
        .map(|c| if is_transit(c) { by_rides.next() } else { by_links.next() })
        .map(|ln| ln.expect("one factor per chain"))
        .collect()
}

/// The batch of `situations` at `iteration`: each set's chains with their utility at the
/// state's current attributes, their path size (`sizes`, up to date with the sets), the
/// indifference `band`, their identity and, with `nest`, their mode as a number.
pub(crate) fn batch(
    demand: &Demand<'_>,
    state: &State,
    sizes: &PathSizes,
    situations: &[TripId],
    iteration: u32,
    band: f64,
    nest: bool,
) -> ChoiceBatch {
    let names: &[&str] = if nest {
        &[UTILITY, LN_PATH_SIZE, INDIFFERENCE_BAND, CHAIN_HIGH, CHAIN_LOW, NEST]
    } else {
        &[UTILITY, LN_PATH_SIZE, INDIFFERENCE_BAND, CHAIN_HIGH, CHAIN_LOW]
    };
    let mut batch = ChoiceBatch::new(iteration, names);
    let mut values = [0.0; 6];
    let width = names.len();
    for &trip in situations {
        let traveller = demand.trips.traveller(trip);
        let class = demand.travellers.user_class(traveller);
        batch.begin_situation_in(traveller.raw(), trip.raw(), class.raw());
        let set = &state.trip(trip).set;
        let k = demand.coefficients(traveller);
        for (i, (chain, &ln_size)) in set.iter().zip(sizes.of(trip)).enumerate() {
            let identity = u32::try_from(i).expect("fewer than 2^32 chains in a set");
            let (high, low) = (chain.identity() >> 32, chain.identity() & 0xFFFF_FFFF);
            #[allow(clippy::cast_precision_loss, reason = "32 bits, exact in a double")]
            let (high, low) = (high as f64, low as f64);
            values[..5].copy_from_slice(&[k.chain_utility(chain), ln_size, band, high, low]);
            values[5] = f64::from(chain.mode() as u8);
            batch.push_alternative(identity, &values[..width]);
        }
    }
    batch
}

/// Whether `model` reads the `nest` attribute (a nested logit): the batch carries it only then.
pub(crate) fn wants_nest(model: &dyn ChoiceModel) -> bool {
    model.required_attributes().is_some_and(|names| names.iter().any(|n| n == NEST))
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

/// What the choose stage measured over the trips that could change: the switch share, its floor
/// and the excess per re-chooser; the same by the mode each trip held; the mode changes and the
/// mode split of the trips choosing their mode.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Switching {
    pub share: f64,
    pub floor: f64,
    pub excess: f64,
    /// The expected excess over the whole population (see the module docs).
    pub expected: f64,
    pub share_by_mode: PerMode,
    /// Per mode, the expected change of its share (see [`crate::IterationReport`]).
    pub share_change: PerMode,
    pub mode_change: Option<f64>,
    pub mode_split: PerMode,
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
    band: f64,
    allocation: Allocation,
) -> Result<Switching, AssignError> {
    let situations = situations(state);
    let batch = batch(demand, state, sizes, &situations, iteration, band, wants_nest(model));
    batch.validate()?;
    let probabilities = probabilities(model, &batch)?;
    let alpha = rechoice_probability(iteration, offset);
    // The quota's groups (see `crate::quota`), only when the allocation uses them.
    let groups = if allocation == Allocation::Draws {
        Vec::new()
    } else {
        if probabilities.flat.iter().any(|p| p.is_nan()) {
            return Err(AssignError::Input(format!(
                "the {} allocation needs a choice model that gives its probabilities; {} gives none",
                allocation.name(),
                model.name()
            )));
        }
        group_keys(demand, state, &situations)
    };
    // Per situation, whether its trip had no chain and whether it chooses now.
    let drawn: Vec<(bool, bool)> = if allocation.quota_selection() {
        reselection_quota(demand, state, &situations, &groups, alpha, iteration)
    } else {
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
    let mut picked: Vec<Option<usize>> = vec![None; situations.len()];
    if allocation.quota_choice() {
        choice_quota(
            state,
            &situations,
            &groups,
            &probabilities,
            &choosing,
            iteration,
            &mut picked,
        );
    } else {
        let offered = batch.subset(&choosing);
        let choices = model.choose(&offered, choice_rng)?;
        choices.validate(&offered)?;
        for (k, &s) in choosing.iter().enumerate() {
            picked[s] = Some(choices.chosen[k] as usize);
        }
    }
    let (mut total, mut switched, mut floor, mut reconsidering) = (0.0, 0.0, 0.0, 0.0);
    let mut expected = 0.0;
    // By the mode held before the choice: the trips and those that switched. And per mode, the
    // expected change of its share: Σ w (P_m − 1[the chain held is of mode m]).
    let (mut held, mut moved, mut change) = ([0.0; 4], [0.0; 4], [0.0; 4]);
    // The trips choosing their mode: with a chain before, and of those with another mode after;
    // with a chain after, by its mode.
    let (mut had, mut changed, mut after_total, mut after_mode) = (0.0, 0.0, 0.0, [0.0; 4]);
    for (s, &trip) in situations.iter().enumerate() {
        let w = demand.weight(demand.trips.traveller(trip));
        let counted = demand.could_change(trip);
        let entry = &mut state.trips_mut()[trip.index()];
        let before = entry.current;
        let after = picked[s].or(before);
        let mode_of = |c: Option<usize>| c.map(|c| entry.set[c].mode());
        let (mode_before, mode_after) = (mode_of(before), mode_of(after));
        // The sums over the trips that could change their choice only.
        if counted {
            // The chance that two independent draws from the model's probabilities differ.
            let differs = 1.0 - probabilities.of(s).iter().map(|p| p * p).sum::<f64>();
            total += w;
            reconsidering += w * chance[s];
            floor += w * chance[s] * if before.is_none() { 1.0 } else { differs };
            let slot = mode_before.and_then(mode_slot);
            // A trip with no chain yet moves for certain, as its floor says: no excess.
            if let Some(c) = before {
                expected += w * (1.0 - differs - probabilities.of(s)[c]);
                for (chain, p) in entry.set.iter().zip(probabilities.of(s)) {
                    if let Some(k) = mode_slot(chain.mode()) {
                        change[k] += w * p;
                    }
                }
                if let Some(k) = slot {
                    change[k] -= w;
                }
            }
            if let Some(k) = slot {
                held[k] += w;
                if after != before {
                    moved[k] += w;
                }
            }
            if after != before {
                switched += w;
            }
        }
        if demand.chooses_mode(trip) {
            if mode_before.is_some() {
                had += w;
                if mode_after != mode_before {
                    changed += w;
                }
            }
            if let Some(k) = mode_after.and_then(mode_slot) {
                after_total += w;
                after_mode[k] += w;
            }
        }
        entry.current = after;
        entry.probabilities.clear();
        entry.probabilities.extend_from_slice(probabilities.of(s));
    }
    let share = |x: f64| if total > 0.0 { x / total } else { 0.0 };
    let excess = if reconsidering > 0.0 { (switched - floor) / reconsidering } else { 0.0 };
    Ok(Switching {
        share: share(switched),
        floor: share(floor),
        excess,
        expected: share(expected),
        share_by_mode: PerMode::shares(moved, held),
        share_change: PerMode::shares(change, [total; 4]),
        mode_change: (had > 0.0).then(|| changed / had),
        mode_split: PerMode::shares(after_mode, [after_total; 4]),
    })
}

/// A quota group's key: the user class, the bin in which the vehicle of the set's first chain
/// sets off (in bins of the state's link travel times; [`crate::Leg::vehicle_departure`]) and
/// the identities of the set's chains, sorted. Equal keys, equal choice situations
/// (see [`crate::quota`]); the keys' own order decides nothing.
type GroupKey = (u32, u64, Vec<u64>);

/// Every situation's group key.
fn group_keys(demand: &Demand<'_>, state: &State, situations: &[TripId]) -> Vec<GroupKey> {
    let bin = state.link_times().bin_seconds();
    situations
        .iter()
        .map(|&trip| {
            let class = demand.travellers.user_class(demand.trips.traveller(trip)).raw();
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a departure in seconds after midnight over a positive bin length"
            )]
            let at = (setting_off(demand, state, trip) / bin).floor() as u64;
            let mut chains: Vec<u64> = state.trip(trip).set.iter().map(Chain::identity).collect();
            chains.sort_unstable();
            (class, at, chains)
        })
        .collect()
}

/// When the vehicle of `trip`'s first chain sets off (its first leg's vehicle departure); the
/// trip's departure for a trip with no chain.
fn setting_off(demand: &Demand<'_>, state: &State, trip: TripId) -> f64 {
    state.trip(trip).set.first().and_then(|c| c.legs().first()).map_or_else(
        || f64::from(demand.trips.departure(trip).get()),
        crate::chain::Leg::vehicle_departure,
    )
}

/// The members of each group among `members` (indices into the situations, in trip order), each
/// group's list in trip order; the groups in the keys' order, which decides nothing.
fn grouped<'a>(groups: &'a [GroupKey], members: &[usize]) -> BTreeMap<&'a GroupKey, Vec<usize>> {
    let mut out: BTreeMap<&GroupKey, Vec<usize>> = BTreeMap::new();
    for &s in members {
        out.entry(&groups[s]).or_default().push(s);
    }
    out
}

/// Form 2 (see [`crate::quota`]): per situation, whether its trip had no chain and whether it
/// chooses now. A traveller is counted in the quota by their first trip with a chain; their other
/// trips choose when it does; a trip with no chain always chooses.
fn reselection_quota(
    demand: &Demand<'_>,
    state: &State,
    situations: &[TripId],
    groups: &[GroupKey],
    alpha: f64,
    iteration: u32,
) -> Vec<(bool, bool)> {
    let none: Vec<bool> = situations.iter().map(|&t| state.trip(t).current.is_none()).collect();
    // Each traveller's first situation with a chain, in trip order.
    let mut first: Vec<usize> = vec![usize::MAX; demand.travellers.len() as usize];
    let mut deciding = Vec::new();
    for (s, &trip) in situations.iter().enumerate() {
        let traveller = demand.trips.traveller(trip).index();
        if !none[s] && first[traveller] == usize::MAX {
            first[traveller] = s;
            deciding.push(s);
        }
    }
    let mut decided = vec![false; situations.len()];
    for members in grouped(groups, &deciding).values() {
        let turn = quota::selection_turn(iteration, situations[members[0]].index());
        for (r, &s) in members.iter().enumerate() {
            decided[s] = quota::reselected(r, alpha, turn);
        }
    }
    situations
        .iter()
        .enumerate()
        .map(|(s, &trip)| {
            let at = first[demand.trips.traveller(trip).index()];
            (none[s], none[s] || (at != usize::MAX && decided[at]))
        })
        .collect()
}

/// Form 1 (see [`crate::quota`]): the chain of every re-chooser, by its group's choice quota,
/// written into `picked` as an index into the trip's own set.
fn choice_quota(
    state: &State,
    situations: &[TripId],
    groups: &[GroupKey],
    probabilities: &Probabilities,
    choosing: &[usize],
    iteration: u32,
    picked: &mut [Option<usize>],
) {
    let mut ordered: Vec<f64> = Vec::new();
    let mut position: Vec<usize> = Vec::new();
    for members in grouped(groups, choosing).values() {
        // The group's chains in the order of its first trip's set.
        let order: Vec<u64> =
            state.trip(situations[members[0]]).set.iter().map(Chain::identity).collect();
        let first = situations[members[0]].index();
        let ranks = quota::ranks(members.len(), iteration, first);
        let turn = quota::choice_turn(iteration, first);
        for (n, &s) in members.iter().enumerate() {
            let set = &state.trip(situations[s]).set;
            let own = probabilities.of(s);
            ordered.clear();
            position.clear();
            for id in &order {
                let k = set.iter().position(|c| c.identity() == *id).expect("the group's chain");
                position.push(k);
                ordered.push(own[k]);
            }
            picked[s] = Some(position[quota::pick(ranks[n], members.len(), turn, &ordered)]);
        }
    }
}
