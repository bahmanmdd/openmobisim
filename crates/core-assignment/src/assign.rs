//! The assignment loop: five stages per iteration and one stopping rule.
//!
//! [`assign`] runs the iterated dynamic traffic assignment. Every assignment iteration `i`
//! does, for every trip, in this order and nothing else:
//!
//! 1. **Search** (`search_stage`): the chain search at the current state (the link travel
//!    times of the last loading, free flow before the first); a chain not yet in the trip's
//!    choice set is added to it, at most one per trip per iteration (the trips with an addition
//!    are counted as **added**). An addition counts as **found** only when it beats the
//!    cheapest chain already in the trip's set by more than the indifference band:
//!    `cost_new < cost_best − ε · |cost_best|` (`cost_best · (1 − ε)` for a positive cost),
//!    ε = [`Params::indifference_band`], both costs read on the same link travel times (a trip
//!    with an empty set counts as found). A chain within the band is still added, since a
//!    choice model wants near-equal alternatives in the set, but it is no evidence that the
//!    search missed a better route. **The stopping check** runs here, from iteration 1 on, after
//!    the search and before anything is added: the loop stops when the share of trips with a
//!    found chain is at most [`Params::found_tolerance`], the expected switch excess
//!    ([`IterationReport::switch_excess_expected`]), averaged over the last three iterations,
//!    is in absolute value at most [`Params::switch_tolerance`], and at least
//!    [`Params::min_iterations`] iterations are done. In the words of the field: the loop stops
//!    when, at the current costs, fewer than one trip in twenty (the default tolerance 0.05)
//!    would change chain beyond chance if everyone reconsidered, and the route search finds
//!    nothing better beyond the indifference band. It always stops after
//!    [`Params::max_iterations`] iterations. When it stops, the state returned is the one the
//!    last completed iteration left, without the stopped search's chains.
//! 2. **Choose** (`choose_stage`): one choice over every trip's set by the run's choice
//!    model, with the averaging rule deciding who re-chooses ([`crate::chooser`]): by seeded
//!    draws, or by the quota allocation of each group of trips facing the same choice in the
//!    model's proportions ([`Params::allocation`], [`crate::quota`]).
//! 3. **Load** (`load_stage`): every chosen chain's car legs become vehicles, which the
//!    run's [`Loader`] moves on the network.
//! 4. **Measure** (`measure_stage`): the loading's entry-time tables become the state's link
//!    travel times by time of day ([`Loaded::link_times`]: per link and bin of entry time, the
//!    mean time of the vehicles that entered then; free flow for a loading that records none),
//!    and every chain of every set (taken or not) is priced again on them by the planner of
//!    each leg's mode, the car's driven link by link with [`LinkTimes::route_seconds`] (the
//!    wait to get onto the first link included).
//! 5. **Assess** (`assess_stage`): the convergence measures of the [`IterationReport`].
//!
//! Each iteration finishes its own work: nothing of iteration `i` is computed after `i + 1` has
//! begun, and the next iteration starts from the state this one returned.
//!
//! **Iteration numbers.** A run from [`State::initial`] numbers its iterations from 0; a run
//! continued from a state another run returned ([`Inputs::initial`]) numbers its first iteration
//! [`State::iterations`], so the averaging rule goes on with the step it had reached. The
//! minimum and most iterations count the iterations of this run.
//!
//! **Reproducibility.** Every random draw is addressed by identity (the re-choice draws by
//! traveller and iteration, the choice model's by traveller, trip, iteration and chain; the
//! quota allocation draws nothing and works its groups on one thread, in trip order), so
//! the same inputs and seed give the same states and reports, to the bit, on any machine of the
//! same platform.
//!
//! **Threads.** The search, the choice's draws and path sizes, the re-pricing and the per-trip
//! terms of the convergence measures run on the threads of the calling thread's pool (rayon's
//! global pool, unless the caller installs one), in fixed chunks of trips or of groups of
//! trips, joined in order. Each trip's result depends only on the trip and the state, and every
//! sum over trips is taken trip after trip in trip order, so a run gives the same states and
//! reports, to the bit, on one thread or on many. The loading runs as its loader runs it.
//!
//! [`LinkTimes::route_seconds`]: openmobisim_core_loading::LinkTimes::route_seconds

use std::time::Instant;

use openmobisim_core_choice::ChoiceModel;
use openmobisim_core_demand::{Travellers, Trips};
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_loading::Vehicle;
use openmobisim_core_types::ids::{EntityId, LinkId, TripId, VehicleId};
use openmobisim_core_types::rng::{RngKey, Stream, StreamRng};
use openmobisim_core_types::time::Second;
use openmobisim_core_types::units::Pcu;

use crate::attributes::Coefficients;
use crate::chain::{Chain, FixedTimes, Place};
use crate::chooser::{self, Demand, PathSizes, Switching};
use crate::error::AssignError;
use crate::hooks::StageHook;
use crate::loader::{Loaded, Loader};
use crate::parallel;
use crate::planner::{CarPlanner, CostSource};
use crate::quota::Allocation;
use crate::report::{IterationReport, StageSeconds};
use crate::search::ChainSearch;
use crate::state::{State, TripState};

/// The trips a thread takes at a time in the measure and assess stages.
const TRIPS_PER_CHUNK: usize = 512;

/// The loop's parameters. The defaults are proposals, each one named so it is easy to change.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    /// The fewest iterations before the loop may stop (default 3).
    pub min_iterations: u32,
    /// The most iterations the loop runs (default 50).
    pub max_iterations: u32,
    /// The indifference band ε, relative (default 0.01, an uncalibrated default): two chains
    /// whose costs differ by at most ε times the lesser cost count as equal. It is used in
    /// exactly three places: the deterministic choice ties every chain within the band of the
    /// best, `cost_k ≤ cost_best + ε · |cost_best|`, and splits the trips facing a tie evenly
    /// over its chains ([`crate::HighestUtility`]); a chain the search adds counts as found only
    /// when it beats the trip's cheapest chain by more than the band (the stopping rule's found
    /// share, [`IterationReport::found_share`]); and, through the deterministic choice's
    /// probabilities (`1 / k` over the `k` tied chains), the switch floor counts the chance of a
    /// tie, so the switch excess is the excess beyond the band. The logit does not read it. 0:
    /// only chains of exactly equal cost tie, and every chain cheaper than the set's best
    /// counts as found.
    pub indifference_band: f64,
    /// The largest expected switch excess ([`IterationReport::switch_excess_expected`]: the
    /// share of trips that would change chain beyond the stationary chance if everyone
    /// reconsidered, at the current costs), in absolute value and averaged over the last three
    /// iterations, at which the loop may stop (default 0.05: one trip in twenty).
    ///
    /// The rule reads an expectation over the whole population, not a realised share: the
    /// realised excess ([`IterationReport::switch_excess`]) and the switch share above its floor
    /// ([`IterationReport::switch_above_floor`]) are reported only. The realised excess is a
    /// share among the travellers who happened to reconsider, whose sampling noise grows as
    /// the averaging step falls, so a rule on it either stops on a low draw or, given a noise
    /// margin, cannot stop late in a run; the switch share above its floor shrinks with the
    /// step whether or not the assignment is stationary. The expected excess has neither
    /// fault: it is 0 exactly at a stationary state, whatever the step.
    ///
    /// **Under the deterministic choice** it is the share of trips on a chain beyond the
    /// indifference band of their best, which falls only as the averaging moves a bin's costs by
    /// less than the band; the travellers of one bin of departure time who reconsider read the
    /// same costs and move as one block. On a bottleneck with 150 cars a bin a quarter of the
    /// trips are still beyond the band after 200 iterations, so such a run goes to
    /// [`Self::max_iterations`]. A run that reaches it did not stop by the rule.
    pub switch_tolerance: f64,
    /// The largest share of trips (weighted) the search may still find a new chain for when
    /// the loop stops (default 0.005, uncalibrated: one trip in two hundred may still find a
    /// chain cheaper than its best by more than the indifference band; at 0, a city's run
    /// with the logit can meet every other condition and still not stop).
    pub found_tolerance: f64,
    /// The averaging rule's offset: in iteration `i ≥ 1` a traveller re-chooses with
    /// probability `1 / (i + 1 + step_offset)` (default 3).
    pub step_offset: u32,
    /// How the travellers who reconsider in an iteration are selected and allocated over their
    /// chains (default [`Allocation::Draws`]: by seeded draws, each with probability `α`, and the
    /// choice model's own choice). [`Allocation::ChoiceQuota`] allocates each group's re-choosers
    /// over its chains in the model's proportions, [`Allocation::Quota`] also selects who
    /// reconsiders by quota, `⌊α N⌋` or `⌈α N⌉` of a group of `N`; both need a model that gives
    /// its probabilities ([`crate::quota`]).
    pub allocation: Allocation,
    /// Where the chain search reads leg costs (default exact).
    pub cost_source: CostSource,
    /// How close, in metres, a trip end must lie to a node with only motorways or trunk roads
    /// to start or end there rather than on the nearest street (default 50).
    pub motorway_snap_metres: f64,
    /// The fixed times of every car leg that uses a link: reaching the car before the drive,
    /// parking and leaving it after (default [`FixedTimes::NONE`]; see [`crate::chain`]).
    pub car_fixed_times: FixedTimes,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            min_iterations: 3,
            max_iterations: 50,
            indifference_band: 0.01,
            switch_tolerance: 0.05,
            found_tolerance: 0.005,
            step_offset: 3,
            allocation: Allocation::Draws,
            cost_source: CostSource::Exact,
            motorway_snap_metres: 50.0,
            car_fixed_times: FixedTimes::NONE,
        }
    }
}

/// What an assignment is run on.
pub struct Inputs<'a> {
    /// The road network.
    pub network: &'a RoadNetwork,
    /// Its turns.
    pub turns: &'a TurnTable,
    /// The travellers.
    pub travellers: &'a Travellers,
    /// Their trips.
    pub trips: &'a Trips,
    /// Each user class's coefficients, by user class (one per class of `travellers`).
    pub classes: &'a [Coefficients],
    /// The money charged on each link of the network, zeros where nothing is charged.
    pub money_per_link: &'a [f64],
    /// The choice model ([`chooser::HighestUtility`], [`chooser::logit`] or any model that reads
    /// the batch attributes the chooser offers).
    pub model: &'a dyn ChoiceModel,
    /// The dynamic network loading.
    pub loader: &'a dyn Loader,
    /// The run's seed.
    pub seed: RngKey,
    /// The state to start from; `None` for [`State::initial`].
    pub initial: Option<State>,
}

/// The chains the search stage found: per trip, the first chain not yet in its set.
struct Found {
    /// `(trip, chain)` for every trip something new was found for, in trip order.
    additions: Vec<(TripId, Chain)>,
    /// The share of trips (weighted) with an addition.
    added_share: f64,
    /// How many additions beat the trip's cheapest chain by more than the band.
    found: u32,
    /// The share of trips (weighted) with such an addition: what the stopping rule reads.
    share: f64,
}

/// Whether a chain of cost `new` beats a set whose cheapest chain costs `best` by more than
/// the indifference `band`; any chain beats an empty set (`best` infinite).
fn beyond_band(new: f64, best: f64, band: f64) -> bool {
    best.is_infinite() || new < best - band * best.abs()
}

/// Stage 1 without the stopping check: the chain search of every trip at `state`.
fn search_stage(
    search: &ChainSearch<'_>,
    inputs: &Inputs<'_>,
    state: &State,
    params: &Params,
) -> Found {
    let demand = demand(inputs);
    let (mut total, mut added, mut beyond) = (0.0, 0.0, 0.0);
    let mut found = 0_u32;
    let mut additions = Vec::new();
    for (t, fresh) in search.fresh_chains(state, params).into_iter().enumerate() {
        let trip = TripId::from_index(t);
        let traveller = inputs.trips.traveller(trip);
        let w = demand.weight(traveller);
        total += w;
        if let Some(chain) = fresh {
            let k = demand.coefficients(traveller);
            let best = state
                .trip(trip)
                .set
                .iter()
                .map(|c| k.cost(c.total()))
                .fold(f64::INFINITY, f64::min);
            if beyond_band(k.cost(chain.total()), best, params.indifference_band) {
                beyond += w;
                found += 1;
            }
            added += w;
            additions.push((trip, chain));
        }
    }
    let share = |x: f64| if total > 0.0 { x / total } else { 0.0 };
    Found { additions, added_share: share(added), found, share: share(beyond) }
}

/// The rule's stability term: the absolute value of the mean expected switch excess of the last
/// three reports (fewer at the start), `None` before any.
fn tail_excess(reports: &[IterationReport]) -> Option<f64> {
    let tail = &reports[reports.len().saturating_sub(3)..];
    if tail.is_empty() {
        return None;
    }
    #[allow(clippy::cast_precision_loss, reason = "at most three reports")]
    let n = tail.len() as f64;
    Some((tail.iter().map(|r| r.switch_excess_expected).sum::<f64>() / n).abs())
}

/// Whether the loop stops at a search stage, given what the search found and the stability
/// term of the last iterations; `done` is the number of iterations this run has completed.
fn should_stop(done: u32, found: &Found, last: Option<f64>, params: &Params) -> bool {
    let Some(last) = last else { return false };
    done >= 1
        && done >= params.min_iterations
        && found.share <= params.found_tolerance
        && last <= params.switch_tolerance
}

/// Stage 2: the choice, writing every trip's current chain and choice probabilities into
/// `state`; returns the switch share and its floor.
///
/// # Errors
///
/// [`AssignError::Choice`] if the choice model fails.
fn choose_stage(
    inputs: &Inputs<'_>,
    state: &mut State,
    sizes: &mut PathSizes,
    rngs: &(StreamRng, StreamRng),
    iteration: u32,
    params: &Params,
) -> Result<Switching, AssignError> {
    let demand = demand(inputs);
    sizes.refresh(inputs.network, state);
    chooser::choose(
        &demand,
        state,
        sizes,
        inputs.model,
        &rngs.0,
        &rngs.1,
        iteration,
        params.step_offset,
        params.indifference_band,
        params.allocation,
    )
}

/// Stage 3: the vehicles of every trip's current chain (one per car leg that uses a link,
/// weighing its traveller's weight, setting off at the leg's vehicle departure), moved by the
/// run's loader; returns what the loading returned and, per vehicle, its trip and its leg's
/// egress time.
///
/// # Errors
///
/// [`AssignError::Input`] if the loader returns other than one arrival per vehicle.
fn load_stage(
    inputs: &Inputs<'_>,
    state: &State,
    iteration: u32,
    hooks: &mut dyn StageHook,
) -> Result<(Loaded, Vec<(TripId, f64)>), AssignError> {
    let mut vehicles = Vec::new();
    let mut of_trip = Vec::new();
    for (t, entry) in state.trips().iter().enumerate() {
        let Some(chain) = entry.current_chain() else { continue };
        let trip = TripId::from_index(t);
        let weight = f64::from(inputs.travellers.weight(inputs.trips.traveller(trip)));
        for leg in chain.legs().iter().filter(|l| !l.links.is_empty()) {
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a departure in seconds after midnight, non-negative, within the u32 clock"
            )]
            let second = Second(leg.vehicle_departure().max(0.0).floor() as u32);
            let id = VehicleId::from_index(vehicles.len());
            vehicles.push(Vehicle::new(id, leg.links.clone(), Pcu(weight), second));
            of_trip.push((trip, leg.fixed.egress_seconds));
        }
    }
    hooks.before_load(iteration, state, &vehicles);
    let loaded = inputs.loader.load(&vehicles, inputs.network);
    if loaded.arrival.len() != vehicles.len() {
        return Err(AssignError::Input(format!(
            "the loading returned {} arrivals for {} vehicles",
            loaded.arrival.len(),
            vehicles.len()
        )));
    }
    hooks.after_load(iteration, &loaded);
    Ok((loaded, of_trip))
}

/// Stage 4: the link times the loading measured become the state's, and every chain of every
/// set is priced again on them, its legs timed one after the other from the trip's departure.
fn measure_stage(
    search: &ChainSearch<'_>,
    network: &RoadNetwork,
    state: &mut State,
    loaded: &Loaded,
) {
    state.set_link_times(loaded.link_times(network));
    // The sets are moved out while they are priced, so the planners read the rest of the state
    // (its link times) while every set is changed in place, trip by trip, in parallel.
    let mut trips: Vec<TripState> = std::mem::take(state.trips_mut());
    {
        let state = &*state;
        parallel::for_each_mut(&mut trips, TRIPS_PER_CHUNK, |entry| {
            for chain in &mut entry.set {
                let mut clock = None;
                for leg in chain.legs_mut() {
                    if let Some(at) = clock {
                        leg.departure = at;
                    }
                    if let Some(planner) = search.planner(leg.mode) {
                        planner.reprice(leg, state);
                    }
                    clock = Some(leg.departure + leg.attributes.total_seconds());
                }
                chain.refresh_total();
            }
        });
    }
    *state.trips_mut() = trips;
}

/// Stage 5: the convergence measures of `iteration`. A trip arrives when its last vehicle's leg
/// ends (the vehicle's arrival plus the leg's egress time), if that is by the loading's end
/// ([`Loader::end_seconds`]); otherwise it is unfinished.
///
/// # Errors
///
/// [`AssignError::Choice`] if the choice model fails to give its probabilities.
fn assess_stage(
    inputs: &Inputs<'_>,
    state: &State,
    sizes: &PathSizes,
    loaded: &Loaded,
    of_trip: &[(TripId, f64)],
    iteration: u32,
    band: f64,
) -> Result<IterationReport, AssignError> {
    let demand = demand(inputs);
    let weight = |trip: TripId| demand.weight(inputs.trips.traveller(trip));
    // Departed, arrived, unfinished and the trips' times, from the loading. Per trip: `None`
    // if none of its legs was loaded, `Some(None)` if one of its vehicles had not arrived,
    // `Some(Some(a))` with the last vehicle's arrival otherwise.
    let (mut departed, mut arrived, mut seconds) = (0.0, 0.0, 0.0);
    let end = inputs.loader.end_seconds();
    let mut last_arrival: Vec<Option<Option<f64>>> = vec![None; state.trips().len()];
    for (&(trip, egress), &at) in of_trip.iter().zip(&loaded.arrival) {
        // The leg ends once the vehicle is parked and left, if by the loading's end.
        let at = at.map(|a| a + egress).filter(|&a| a <= end);
        let slot = &mut last_arrival[trip.index()];
        *slot = Some(match (*slot, at) {
            (Some(None), _) | (_, None) => None,
            (Some(Some(a)), Some(b)) => Some(a.max(b)),
            (None, Some(b)) => Some(b),
        });
    }
    let mut without_chain = 0.0;
    for (t, entry) in state.trips().iter().enumerate() {
        let trip = TripId::from_index(t);
        let w = weight(trip);
        if entry.current.is_none() {
            without_chain += w;
            continue;
        }
        departed += w;
        let start = f64::from(inputs.trips.departure(trip).get());
        match last_arrival[t] {
            // A chain with no road link arrives as it sets out.
            None => arrived += w,
            Some(Some(a)) => {
                arrived += w;
                seconds += w * (a - start);
            }
            Some(None) => {}
        }
    }
    let unfinished = departed - arrived;
    // The gaps and the spread, on the re-priced sets.
    let situations = chooser::situations(state);
    let batch = chooser::batch(&demand, state, sizes, &situations, iteration, band);
    let probabilities = chooser::probabilities(inputs.model, &batch)?;
    // Per situation with a chain: the cost of the chain taken, the least cost of the set and the
    // expected cost under the model, unweighted; summed below in trip order.
    let index: Vec<usize> = (0..situations.len()).collect();
    let terms: Vec<Option<(f64, f64, f64)>> = parallel::map(
        &index,
        TRIPS_PER_CHUNK,
        || (),
        |(), &s| {
            let trip = situations[s];
            let entry = state.trip(trip);
            let current = entry.current?;
            let k = demand.coefficients(inputs.trips.traveller(trip));
            let cost = |c: &Chain| k.cost(c.total());
            let low = entry.set.iter().map(cost).fold(f64::INFINITY, f64::min);
            let mean: f64 =
                entry.set.iter().map(cost).zip(probabilities.of(s)).map(|(c, p)| c * p).sum();
            Some((cost(&entry.set[current]), low, mean))
        },
    );
    let (mut paid, mut least, mut expected, mut cheapest, mut chosen) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for (&trip, term) in situations.iter().zip(terms) {
        let Some((taken, low, mean)) = term else { continue };
        let w = weight(trip);
        paid += w * taken;
        least += w * low;
        expected += w * mean;
        chosen += w;
        if taken <= low {
            cheapest += w;
        }
    }
    let over = |x: f64, base: f64| if base > 0.0 { x / base } else { 0.0 };
    Ok(IterationReport {
        iteration,
        found: 0,
        found_share: 0.0,
        added: 0,
        added_share: 0.0,
        switch_share: 0.0,
        switch_floor: 0.0,
        switch_excess: 0.0,
        switch_above_floor: 0.0,
        switch_excess_expected: 0.0,
        departed,
        arrived,
        unfinished,
        unfinished_share: over(unfinished, departed),
        without_chain,
        duality_gap: over(paid - least, least),
        gap_deterministic: over(paid - expected, least),
        on_cheapest_share: over(cheapest, chosen),
        mean_trip_seconds: over(seconds, arrived),
        stage_seconds: StageSeconds::default(),
    })
}

/// One trip's choice set as a run wrote it, to rebuild a state from ([`restore`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SavedTrip {
    /// Each chain of the set, in the order it was found: the road links of its one car leg,
    /// from the leg's upstream end to its downstream end (empty for a leg whose two ends are
    /// one node).
    pub chains: Vec<Vec<LinkId>>,
    /// The chain the traveller takes, as an index into [`Self::chains`]; `None` if none.
    pub current: Option<usize>,
    /// The probability the choice model gave each chain at the trip's last choice (as many as
    /// there were chains then, or none).
    pub probabilities: Vec<f64>,
}

impl SavedTrip {
    /// What `entry` would be written as: its chains' road links, current chain and
    /// probabilities.
    #[must_use]
    pub fn of(entry: &TripState) -> Self {
        Self {
            chains: entry
                .set
                .iter()
                .map(|c| c.legs().iter().flat_map(|l| l.links.iter().copied()).collect())
                .collect(),
            current: entry.current,
            probabilities: entry.probabilities.clone(),
        }
    }
}

/// The state a run left, rebuilt from what it wrote: `saved`, every trip's set, current chain
/// and probabilities (one per trip of `inputs.trips`, in trip order), and `iterations`, the count
/// of iterations it completed. Each chain becomes the car leg along its links from the trip's
/// origin to its destination at the trip's departure, with the run's fixed times
/// (`params.car_fixed_times`); then the current chains are loaded once by `inputs.loader` and
/// the link travel times measured, and every chain priced on them, as the measure stage does.
/// The loading is deterministic, so a state rebuilt from what a run wrote of its final state
/// equals that state, link times included, to the bit. `inputs.initial` is not read.
///
/// # Errors
///
/// [`AssignError::Input`] if `saved` is not one entry per trip, a chain's links do not run in
/// driving order from the trip's origin node to its destination node, a set holds a chain
/// twice, a current chain is not one of the set, there are more probabilities than chains, or
/// a traveller without a car has a chain; and the errors of [`assign`] for inputs that do not
/// fit together.
pub fn restore(
    inputs: &Inputs<'_>,
    params: &Params,
    saved: &[SavedTrip],
    iterations: u32,
) -> Result<State, AssignError> {
    let car = CarPlanner::new(inputs.network, inputs.turns, inputs.classes, inputs.money_per_link)?
        .with_fixed_times(params.car_fixed_times)?;
    let search = ChainSearch::new(inputs.network, inputs.travellers, inputs.trips, car, params);
    let mut state = State::initial(inputs.network, inputs.travellers, inputs.trips);
    check(inputs, &state, params)?;
    if saved.len() != inputs.trips.len() as usize {
        return Err(AssignError::Input(format!(
            "the saved state has {} trips, the demand {}",
            saved.len(),
            inputs.trips.len()
        )));
    }
    let network = inputs.network;
    let mut trips = Vec::with_capacity(saved.len());
    for (t, s) in saved.iter().enumerate() {
        let trip = TripId::from_index(t);
        let bad = |what: String| AssignError::Input(format!("trip {t} of the saved state: {what}"));
        let (from, to) = search.ends(trip);
        let (Place::Road(origin), Place::Road(destination)) = (from, to);
        if !s.chains.is_empty() && !state.has_car(inputs.trips.traveller(trip)) {
            return Err(bad("its traveller has no car, yet it has chains".to_string()));
        }
        let departure = f64::from(inputs.trips.departure(trip).get());
        let mut entry = TripState::default();
        for (c, links) in s.chains.iter().enumerate() {
            if links.iter().any(|l| l.raw() >= network.link_count()) {
                return Err(bad(format!("chain {c} has a link not of this network")));
            }
            let joined = match (links.first(), links.last()) {
                (Some(&a), Some(&z)) => {
                    network.link_from(a) == origin
                        && network.link_to(z) == destination
                        && links
                            .windows(2)
                            .all(|w| network.link_to(w[0]) == network.link_from(w[1]))
                }
                _ => origin == destination,
            };
            if !joined {
                return Err(bad(format!(
                    "chain {c}'s links do not run from the trip's origin node to its destination \
                     node (another network, or trip ends snapped otherwise)"
                )));
            }
            let leg = search.car().leg_along(from, to, departure, links.clone(), &state);
            if !entry.add(Chain::new(vec![leg])) {
                return Err(bad(format!("chain {c} is in its set twice")));
            }
        }
        if s.current.is_some_and(|c| c >= entry.set.len()) {
            return Err(bad("its current chain is not one of its set".to_string()));
        }
        if s.probabilities.len() > entry.set.len() {
            return Err(bad("it has more probabilities than chains".to_string()));
        }
        entry.current = s.current;
        entry.probabilities.clone_from(&s.probabilities);
        trips.push(entry);
    }
    *state.trips_mut() = trips;
    state.set_iterations(iterations);
    let (loaded, _) = load_stage(inputs, &state, iterations, &mut crate::hooks::NoHooks)?;
    measure_stage(&search, inputs.network, &mut state, &loaded);
    Ok(state)
}

fn demand<'a>(inputs: &Inputs<'a>) -> Demand<'a> {
    Demand { travellers: inputs.travellers, trips: inputs.trips, classes: inputs.classes }
}

/// Check the inputs fit together before anything runs.
fn check(inputs: &Inputs<'_>, state: &State, params: &Params) -> Result<(), AssignError> {
    let band = params.indifference_band;
    if !(band.is_finite() && band >= 0.0) {
        return Err(AssignError::Input(format!(
            "the indifference band must be a finite number of at least 0, not {band}"
        )));
    }
    let classes = inputs.travellers.class_external_ids().count() as usize;
    if inputs.classes.len() < classes {
        return Err(AssignError::Input(format!(
            "{} sets of coefficients for {classes} user classes",
            inputs.classes.len()
        )));
    }
    if state.trips().len() != inputs.trips.len() as usize {
        return Err(AssignError::Input(format!(
            "the state has {} trips, the demand {}",
            state.trips().len(),
            inputs.trips.len()
        )));
    }
    Ok(())
}

/// Run the assignment: iterations of the five stages from `inputs.initial` (or the initial
/// state) until the stopping rule of `params` says stop; `hooks` are called at their stages.
/// Returns the final state and one report per completed iteration. See the
/// [module docs](self).
///
/// # Errors
///
/// [`AssignError`] if the inputs do not fit together, a class's coefficients cannot price a
/// route, the choice model fails, or the loader returns other than one arrival per vehicle.
///
/// # Panics
///
/// Never in practice: counts of trips and chains fit `u32`.
pub fn assign(
    inputs: &Inputs<'_>,
    params: &Params,
    hooks: &mut dyn StageHook,
) -> Result<(State, Vec<IterationReport>), AssignError> {
    let car = CarPlanner::new(inputs.network, inputs.turns, inputs.classes, inputs.money_per_link)?
        .with_fixed_times(params.car_fixed_times)?;
    let search = ChainSearch::new(inputs.network, inputs.travellers, inputs.trips, car, params);
    let mut state = match &inputs.initial {
        Some(s) => s.clone(),
        None => State::initial(inputs.network, inputs.travellers, inputs.trips),
    };
    check(inputs, &state, params)?;
    let rngs = (
        StreamRng::new(inputs.seed, Stream::Choice),
        StreamRng::new(inputs.seed, Stream::MsaReselection),
    );
    hooks.before_loop(&state);
    let mut sizes = PathSizes::default();
    let mut reports: Vec<IterationReport> = Vec::new();
    let first = state.iterations();
    for done in 0..params.max_iterations {
        let iteration = first + done;
        let clock = Instant::now();
        let found = search_stage(&search, inputs, &state, params);
        if should_stop(done, &found, tail_excess(&reports), params) {
            break;
        }
        let added = u32::try_from(found.additions.len()).expect("fewer than 2^32 trips");
        let (count, found_share, added_share) = (found.found, found.share, found.added_share);
        for (trip, chain) in found.additions {
            state.trip_mut(trip).add(chain);
        }
        let t_search = clock.elapsed().as_secs_f64();

        let clock = Instant::now();
        let switching = choose_stage(inputs, &mut state, &mut sizes, &rngs, iteration, params)?;
        let t_choose = clock.elapsed().as_secs_f64();

        let clock = Instant::now();
        let (loaded, of_trip) = load_stage(inputs, &state, iteration, hooks)?;
        let t_load = clock.elapsed().as_secs_f64();

        let clock = Instant::now();
        measure_stage(&search, inputs.network, &mut state, &loaded);
        hooks.after_measure(iteration, &state);
        let t_measure = clock.elapsed().as_secs_f64();

        let clock = Instant::now();
        let mut report = assess_stage(
            inputs,
            &state,
            &sizes,
            &loaded,
            &of_trip,
            iteration,
            params.indifference_band,
        )?;
        report.found = count;
        report.found_share = found_share;
        report.added = added;
        report.added_share = added_share;
        report.switch_share = switching.share;
        report.switch_floor = switching.floor;
        report.switch_excess = switching.excess;
        report.switch_above_floor = switching.share - switching.floor;
        report.switch_excess_expected = switching.expected;
        report.stage_seconds = StageSeconds {
            search: t_search,
            choose: t_choose,
            load: t_load,
            measure: t_measure,
            assess: clock.elapsed().as_secs_f64(),
        };
        reports.push(report);
        state.complete_iteration();
    }
    Ok((state, reports))
}
