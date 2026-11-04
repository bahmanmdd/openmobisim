//! The car planner's shared searches and the stages' threads change no result: two properties
//! that must hold by construction, checked on a congested network.
//!
//! | Test | Property |
//! |---|---|
//! | 1 | **Invariance to sharing**: at free flow, at every state a run passes through, and at a table of times without the signals' delay (where some steps cost less than nothing), the chains the search finds for all trips at once ([`ChainSearch::search_all`], one search answering many trips) are, trip by trip and to the bit, those it finds for each trip alone ([`ChainSearch::search`], one search per trip) |
//! | 2 | **Invariance to the thread count**: the same run on 1, 2 and 5 threads gives the same states and convergence measures, to the bit |
//!
//! The case: a 6 × 6 grid of 200-m blocks (secondary roads at 50 km/h, 1 800 veh/h a link, a
//! signal at every interior junction), 6 000 car trips in an hour between junctions chosen by
//! a fixed formula, half of them into one interior junction (queues on its four approaches),
//! many from one junction within one departure window, so searches are shared; two user
//! classes, one weighing time only and one also weighing money (1 a unit), with a subsidy
//! (negative money) on the four signalised approaches to junction 14 worth all but a tenth of
//! each approach's free-flow time with its control delay; the point queue (level 2) ending 30 s
//! after the last departure, so the last trips are unfinished. The loading records every
//! signalised approach at its free-flow time or more, so on the states a run passes through no
//! step costs less than nothing; the third kind of state, every link at the time it takes to
//! drive it without a signal's delay, makes the subsidised approaches cost −0.9 times the delay
//! (no cycle of the grid costs less than nothing: it enters junction 14 once at most), the case
//! where a shared search must give way to one search per trip.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "test fixtures: small counts and seconds"
)]

use openmobisim_core_assignment::{
    CarPlanner, ChainSearch, Coefficients, HighestUtility, Inputs, IterationReport, Params,
    PointQueue, StageHook, State, assign, logit,
};
use openmobisim_core_choice::ChoiceModel;
use openmobisim_core_demand::{
    ClassDefaults, Ownership, RawTrip, Travellers, Trips, build_travellers,
};
use openmobisim_core_graph::examples::{manhattan_grid, node_name};
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_loading::LinkTimes;
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::{EntityId, LinkId, TripId};
use openmobisim_core_types::rng::RngKey;
use openmobisim_core_types::time::Second;

const SIDE: u32 = 6;
const TRIPS: u64 = 6000;

struct Case {
    net: RoadNetwork,
    turns: TurnTable,
    travellers: Travellers,
    trips: Trips,
    classes: Vec<Coefficients>,
    money: Vec<f64>,
}

/// The grid and its trips: trip `k` from junction `(7k + k/50) mod 36` (row-major) to junction
/// 14 (row 2, column 2) when `k` is even, else to `(11k + 5) mod 36`, leaving at
/// `⌊k · 3600 / 6000⌋ s`, of class "money" when `k` is a multiple of three, else "time". The
/// money per link: on a link whose free-flow time `f` exceeds its length over its speed `t₀` (a
/// signalised approach, by its control delay), `−(f − (f − t₀)/10)/60` units, which a class
/// paying 1 per unit and 1 per minute values at `−(f − (f − t₀)/10)` seconds; 0 elsewhere.
fn case() -> Case {
    let (net, _) = manhattan_grid(SIDE, 200.0, true);
    let cells = u64::from(SIDE * SIDE);
    let at = |cell: u64| {
        let (row, col) = ((cell / u64::from(SIDE)) as u32, (cell % u64::from(SIDE)) as u32);
        let name = node_name(row, col);
        net.node_lonlat(net.node_external_ids().typed_id_of(&name).expect("a grid junction"))
    };
    let raw: Vec<RawTrip> = (0..TRIPS)
        .map(|k| RawTrip {
            traveller_id: format!("t{k:05}"),
            trip_seq: 0,
            origin: at((7 * k + k / 50) % cells),
            destination: at(if k % 2 == 0 { 14 } else { (11 * k + 5) % cells }),
            departure_time: Second((k * 3600 / TRIPS) as u32),
            user_class: if k % 3 == 0 { "money" } else { "time" }.to_string(),
            weight: Some(1),
            mode: None,
        })
        .collect();
    let owners = ClassDefaults::new().with_fallback(Ownership::VEHICLES);
    let (travellers, trips) =
        build_travellers(raw, Vec::new(), &owners, 1, &mut Diagnostics::new()).expect("demand");
    let classes = travellers
        .class_external_ids()
        .iter()
        .map(|name| match name {
            "money" => Coefficients { per_money: 1.0, ..Coefficients::time_based() },
            _ => Coefficients::time_based(),
        })
        .collect();
    let turns = TurnTable::build(&net, net.signals());
    let centre = net.node_external_ids().typed_id_of(&node_name(2, 2)).expect("junction 14");
    let money = (0..net.link_count())
        .map(|l| {
            let link = LinkId::new(l);
            let free = net.free_flow_time(link).get();
            let driven = net.link_length(link).get() / (50.0 / 3.6);
            let into_centre = net.link_to(link) == centre;
            if into_centre && free > driven + 1e-6 {
                -(free - (free - driven) / 10.0) / 60.0
            } else {
                0.0
            }
        })
        .collect();
    Case { net, turns, travellers, trips, classes, money }
}

/// Keeps the state after every measure stage.
#[derive(Default)]
struct States(Vec<State>);

impl StageHook for States {
    fn after_measure(&mut self, _iteration: u32, state: &State) {
        self.0.push(state.clone());
    }
}

fn run(
    c: &Case,
    model: &dyn ChoiceModel,
    iterations: u32,
    hook: &mut dyn StageHook,
) -> (State, Vec<IterationReport>) {
    let last = (0..c.trips.len()).map(|t| c.trips.departure(TripId::new(t)).get()).max();
    let loader = PointQueue::new(&c.turns, f64::from(last.expect("trips")) + 30.0);
    let inputs = Inputs {
        network: &c.net,
        turns: &c.turns,
        travellers: &c.travellers,
        trips: &c.trips,
        classes: &c.classes,
        money_per_link: &c.money,
        model,
        loader: &loader,
        seed: RngKey::from_seed(2026),
        initial: None,
    };
    let params =
        Params { min_iterations: iterations, max_iterations: iterations, ..Params::default() };
    assign(&inputs, &params, hook).expect("the run")
}

#[test]
fn shared_searches_answer_every_trip_as_its_own_search() {
    let c = case();
    let mut states = States::default();
    run(&c, &HighestUtility, 6, &mut states);
    let params = Params::default();
    let car = CarPlanner::new(&c.net, &c.turns, &c.classes, &c.money).expect("a planner");
    let search = ChainSearch::new(&c.net, &c.travellers, &c.trips, car, &params);
    let initial = State::initial(&c.net, &c.travellers, &c.trips);
    // Every link at the time it takes to drive it, without the control delay of a signal: on a
    // signalised approach the "money" class's step then costs −0.9 × the delay.
    let mut undelayed = initial.clone();
    undelayed.set_link_times(LinkTimes::constant(
        &c.net,
        (0..c.net.link_count())
            .map(|l| c.net.link_length(LinkId::new(l)).get() / (50.0 / 3.6))
            .collect(),
    ));
    let mut compared = 0;
    let mut routes = std::collections::BTreeSet::new();
    let all_states = std::iter::once(&initial).chain(&states.0).chain(std::iter::once(&undelayed));
    for (s, state) in all_states.enumerate() {
        let all = search.search_all(state, &params);
        assert_eq!(all.len(), c.trips.len() as usize);
        // The least recorded time less the link's free-flow time, seconds.
        let least = state
            .link_times()
            .rows()
            .iter()
            .map(|&(l, _, t)| t - c.net.free_flow_time(LinkId::new(l)).get())
            .fold(f64::INFINITY, f64::min);
        for (t, shared) in all.iter().enumerate() {
            let trip = TripId::from_index(t);
            let own = search.search(c.trips.traveller(trip), trip, state, &params);
            assert_eq!(shared, &own, "state {s}, trip {t}: the shared search differs");
            routes.extend(own.iter().map(|chain| chain.identity()));
            compared += 1;
        }
        println!(
            "state {s}: {} trips, every chain equal; binned link times {}, least recorded time less free flow {least:e} s",
            all.len(),
            state.link_times().rows().len(),
        );
    }
    println!(
        "{compared} trips compared over {} states, {} distinct routes",
        states.0.len() + 2,
        routes.len()
    );
}

#[test]
fn one_thread_and_several_give_the_same_run() {
    let c = case();
    let the_logit = logit();
    for model in [&HighestUtility as &dyn ChoiceModel, &the_logit] {
        let runs: Vec<(State, Vec<IterationReport>)> = [1, 2, 5]
            .iter()
            .map(|&threads| {
                let pool = rayon::ThreadPoolBuilder::new()
                    .num_threads(threads)
                    .build()
                    .expect("a thread pool");
                pool.install(|| run(&c, model, 5, &mut openmobisim_core_assignment::NoHooks))
            })
            .collect();
        let (one, rest) = runs.split_first().expect("three runs");
        for (k, other) in rest.iter().enumerate() {
            assert!(
                one.0 == other.0,
                "{}: the state differs from one thread's (run {k})",
                model.name()
            );
            let a: Vec<IterationReport> =
                one.1.iter().map(IterationReport::without_times).collect();
            let b: Vec<IterationReport> =
                other.1.iter().map(IterationReport::without_times).collect();
            assert_eq!(a, b, "{}: the reports differ from one thread's", model.name());
        }
        let last = one.1.last().expect("reports");
        println!(
            "{}: 1, 2 and 5 threads agree to the bit; last iteration: duality gap {:.5}, mean trip {:.1} s, unfinished {}",
            model.name(),
            last.duality_gap,
            last.mean_trip_seconds,
            last.unfinished
        );
    }
}
