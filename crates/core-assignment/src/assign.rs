//! The assignment loop: five stages per iteration and one stopping rule.
//!
//! [`assign`] runs the iterated dynamic traffic assignment. Every assignment iteration `i`
//! does, for every trip, in this order and nothing else:
//!
//! 1. **Search** (`search_stage`): the chain search at the current state (the link travel
//!    times of the last loading, free flow before the first); a chain not yet in the trip's
//!    choice set is added to it, at most one per trip per iteration. The trips something new
//!    was found for are counted. **The stopping check** runs here, from iteration 1 on, after
//!    the search and before anything is added: the loop stops when the share of trips with a
//!    new chain is at most [`Params::found_tolerance`], the last iteration's switch share
//!    exceeds its floor by at most [`Params::switch_tolerance`], and at least
//!    [`Params::min_iterations`] iterations are done. It always stops after
//!    [`Params::max_iterations`] iterations. When it stops, the state returned is the one the
//!    last completed iteration left, without the stopped search's chains.
//! 2. **Choose** (`choose_stage`): one choice over every trip's set by the run's choice
//!    model, with the averaging rule deciding who re-chooses ([`crate::chooser`]).
//! 3. **Load** (`load_stage`): every chosen chain's car legs become vehicles, which the
//!    run's [`Loader`] moves on the network.
//! 4. **Measure** (`measure_stage`): the loaded link travel times become the state's, and
//!    every chain of every set (taken or not) is priced again on them by the planner of each
//!    leg's mode, the car's driven link by link with [`LinkTimes::route_seconds`].
//! 5. **Assess** (`assess_stage`): the convergence measures of the [`IterationReport`].
//!
//! Each iteration finishes its own work: nothing of iteration `i` is computed after `i + 1` has
//! begun, and the next iteration starts from the state this one returned.
//!
//! **Reproducibility.** Every random draw is addressed by identity (the re-choice draws by
//! traveller and iteration, the choice model's by traveller, trip, iteration and chain), so
//! the same inputs and seed give the same states and reports, to the bit, on any machine of the
//! same platform. The loop runs on one thread.
//!
//! [`LinkTimes::route_seconds`]: openmobisim_core_loading::LinkTimes::route_seconds

use std::time::Instant;

use openmobisim_core_choice::ChoiceModel;
use openmobisim_core_demand::{Travellers, Trips};
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_loading::Vehicle;
use openmobisim_core_types::ids::{EntityId, TripId, VehicleId};
use openmobisim_core_types::rng::{RngKey, Stream, StreamRng};
use openmobisim_core_types::time::Second;
use openmobisim_core_types::units::Pcu;

use crate::attributes::Coefficients;
use crate::chain::Chain;
use crate::chooser::{self, Demand, Switching};
use crate::error::AssignError;
use crate::hooks::StageHook;
use crate::loader::{Loaded, Loader};
use crate::planner::{CarPlanner, CostSource};
use crate::report::{IterationReport, StageSeconds};
use crate::search::ChainSearch;
use crate::state::State;

/// The loop's parameters. The defaults are proposals, each one named so it is easy to change.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    /// The fewest iterations before the loop may stop (default 3).
    pub min_iterations: u32,
    /// The most iterations the loop runs (default 20).
    pub max_iterations: u32,
    /// How far the switch share may exceed its floor for the loop to stop (default 0.02).
    pub switch_tolerance: f64,
    /// The largest share of trips (weighted) the search may still find a new chain for when
    /// the loop stops (default 0: nothing new).
    pub found_tolerance: f64,
    /// The averaging rule's offset: in iteration `i ≥ 1` a traveller re-chooses with
    /// probability `1 / (i + 1 + step_offset)` (default 3).
    pub step_offset: u32,
    /// Where the chain search reads leg costs (default exact).
    pub cost_source: CostSource,
    /// How close, in metres, a trip end must lie to a node with only motorways or trunk roads
    /// to start or end there rather than on the nearest street (default 50).
    pub motorway_snap_metres: f64,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            min_iterations: 3,
            max_iterations: 20,
            switch_tolerance: 0.02,
            found_tolerance: 0.0,
            step_offset: 3,
            cost_source: CostSource::Exact,
            motorway_snap_metres: 50.0,
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
    share: f64,
}

/// Stage 1 without the stopping check: the chain search of every trip at `state`.
fn search_stage(
    search: &ChainSearch<'_>,
    inputs: &Inputs<'_>,
    state: &State,
    params: &Params,
) -> Found {
    let (mut total, mut new) = (0.0, 0.0);
    let mut additions = Vec::new();
    for t in 0..inputs.trips.len() {
        let trip = TripId::new(t);
        let traveller = inputs.trips.traveller(trip);
        let w = f64::from(inputs.travellers.weight(traveller));
        total += w;
        let set = &state.trip(trip).set;
        let fresh = search
            .search(traveller, trip, state, params)
            .into_iter()
            .find(|c| !set.iter().any(|k| k.same_alternative(c)));
        if let Some(chain) = fresh {
            new += w;
            additions.push((trip, chain));
        }
    }
    Found { additions, share: if total > 0.0 { new / total } else { 0.0 } }
}

/// Whether the loop stops at the search stage of `iteration`, given what the search found and
/// the last iteration's report.
fn should_stop(
    iteration: u32,
    found: &Found,
    last: Option<&IterationReport>,
    params: &Params,
) -> bool {
    let Some(last) = last else { return false };
    iteration >= 1
        && iteration >= params.min_iterations
        && found.share <= params.found_tolerance
        && last.switch_share - last.switch_floor <= params.switch_tolerance
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
    rngs: &(StreamRng, StreamRng),
    iteration: u32,
    params: &Params,
) -> Result<Switching, AssignError> {
    let demand = demand(inputs);
    chooser::choose(&demand, state, inputs.model, &rngs.0, &rngs.1, iteration, params.step_offset)
}

/// Stage 3: the vehicles of every trip's current chain (one per car leg that uses a link,
/// weighing its traveller's weight), moved by the run's loader; returns what the loading
/// returned and, per vehicle, its trip.
///
/// # Errors
///
/// [`AssignError::Input`] if the loader returns other than one arrival per vehicle.
fn load_stage(
    inputs: &Inputs<'_>,
    state: &State,
    iteration: u32,
    hooks: &mut dyn StageHook,
) -> Result<(Loaded, Vec<TripId>), AssignError> {
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
            let second = Second(leg.departure.max(0.0).floor() as u32);
            let id = VehicleId::from_index(vehicles.len());
            vehicles.push(Vehicle::new(id, leg.links.clone(), Pcu(weight), second));
            of_trip.push(trip);
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

/// Stage 4: the loaded link times become the state's, and every chain of every set is priced
/// again on them, its legs timed one after the other from the trip's departure.
fn measure_stage(search: &ChainSearch<'_>, state: &mut State, loaded: &Loaded) {
    state.set_link_times(loaded.link_times.clone());
    for t in 0..state.trips().len() {
        for c in 0..state.trips()[t].set.len() {
            // Priced on a copy, so the planner reads the whole state, then put back.
            let mut chain = state.trips()[t].set[c].clone();
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
            state.trips_mut()[t].set[c] = chain;
        }
    }
}

/// Stage 5: the convergence measures of `iteration`.
///
/// # Errors
///
/// [`AssignError::Choice`] if the choice model fails to give its probabilities.
fn assess_stage(
    inputs: &Inputs<'_>,
    state: &State,
    loaded: &Loaded,
    of_trip: &[TripId],
    iteration: u32,
) -> Result<IterationReport, AssignError> {
    let demand = demand(inputs);
    let weight = |trip: TripId| demand.weight(inputs.trips.traveller(trip));
    // Departed, arrived, unfinished and the trips' times, from the loading. Per trip: `None`
    // if none of its legs was loaded, `Some(None)` if one of its vehicles had not arrived,
    // `Some(Some(a))` with the last vehicle's arrival otherwise.
    let (mut departed, mut arrived, mut seconds) = (0.0, 0.0, 0.0);
    let mut last_arrival: Vec<Option<Option<f64>>> = vec![None; state.trips().len()];
    for (&trip, &at) in of_trip.iter().zip(&loaded.arrival) {
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
    let batch = chooser::batch(&demand, state, &situations, iteration);
    let probabilities = chooser::probabilities(inputs.model, &batch)?;
    let (mut paid, mut least, mut expected, mut cheapest, mut chosen) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for (s, &trip) in situations.iter().enumerate() {
        let entry = state.trip(trip);
        let Some(current) = entry.current else { continue };
        let w = weight(trip);
        let k = demand.coefficients(inputs.trips.traveller(trip));
        let costs: Vec<f64> = entry.set.iter().map(|c| k.cost(c.total())).collect();
        let low = costs.iter().copied().fold(f64::INFINITY, f64::min);
        let mean: f64 = costs.iter().zip(&probabilities[s]).map(|(c, p)| c * p).sum();
        paid += w * costs[current];
        least += w * low;
        expected += w * mean;
        chosen += w;
        if costs[current] <= low {
            cheapest += w;
        }
    }
    let over = |x: f64, base: f64| if base > 0.0 { x / base } else { 0.0 };
    Ok(IterationReport {
        iteration,
        found: 0,
        found_share: 0.0,
        switch_share: 0.0,
        switch_floor: 0.0,
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

fn demand<'a>(inputs: &Inputs<'a>) -> Demand<'a> {
    Demand {
        network: inputs.network,
        travellers: inputs.travellers,
        trips: inputs.trips,
        classes: inputs.classes,
    }
}

/// Check the inputs fit together before anything runs.
fn check(inputs: &Inputs<'_>, state: &State) -> Result<(), AssignError> {
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
    let car = CarPlanner::new(inputs.network, inputs.turns, inputs.classes, inputs.money_per_link)?;
    let search = ChainSearch::new(inputs.network, inputs.travellers, inputs.trips, car, params);
    let mut state = match &inputs.initial {
        Some(s) => s.clone(),
        None => State::initial(inputs.network, inputs.travellers, inputs.trips),
    };
    check(inputs, &state)?;
    let rngs = (
        StreamRng::new(inputs.seed, Stream::Choice),
        StreamRng::new(inputs.seed, Stream::MsaReselection),
    );
    hooks.before_loop(&state);
    let mut reports: Vec<IterationReport> = Vec::new();
    for iteration in 0..params.max_iterations {
        let clock = Instant::now();
        let found = search_stage(&search, inputs, &state, params);
        if should_stop(iteration, &found, reports.last(), params) {
            break;
        }
        let count = u32::try_from(found.additions.len()).expect("fewer than 2^32 trips");
        for (trip, chain) in found.additions {
            state.trip_mut(trip).add(chain);
        }
        let t_search = clock.elapsed().as_secs_f64();

        let clock = Instant::now();
        let switching = choose_stage(inputs, &mut state, &rngs, iteration, params)?;
        let t_choose = clock.elapsed().as_secs_f64();

        let clock = Instant::now();
        let (loaded, of_trip) = load_stage(inputs, &state, iteration, hooks)?;
        let t_load = clock.elapsed().as_secs_f64();

        let clock = Instant::now();
        measure_stage(&search, &mut state, &loaded);
        hooks.after_measure(iteration, &state);
        let t_measure = clock.elapsed().as_secs_f64();

        let clock = Instant::now();
        let mut report = assess_stage(inputs, &state, &loaded, &of_trip, iteration)?;
        report.found = count;
        report.found_share = found.share;
        report.switch_share = switching.share;
        report.switch_floor = switching.floor;
        report.stage_seconds = StageSeconds {
            search: t_search,
            choose: t_choose,
            load: t_load,
            measure: t_measure,
            assess: clock.elapsed().as_secs_f64(),
        };
        reports.push(report);
    }
    Ok((state, reports))
}
