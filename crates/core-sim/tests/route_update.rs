//! Route updates (S176): growing the choice sets between iterations.
//!
//! What is defended, and how each is checked:
//!
//! * **the case the update exists for**, on a hand-made bottleneck: a choice set that holds
//!   only the road that jams, and a second road that only pays under congestion. Before the
//!   update the gap against the set is zero however jammed the road is; after it the second
//!   road is in the set, travellers take it, and the gap against the whole network falls;
//! * **off means absent**: a run with no update is what it was, bit for bit, and light demand,
//!   where nothing queues and so nothing beats the set, adds nothing and changes nothing;
//! * **a route at least as good as the set's best is worth having**, and a route the set holds is
//!   never added again;
//! * **what is added is a route**: connected, loop-free, made of drivable links and allowed
//!   turns, from the pair's origin to its destination, new to its set, costed at free flow,
//!   stamped with the iteration that added it; the sets keep their order and their keys;
//! * **nobody else's draw moves**: a route that was in the set has the alternative identity it
//!   had, so the routes travellers already know are drawn as they were;
//! * **determinism**: the same run at one and at seven threads gives the same everything.

#![allow(
    clippy::cast_precision_loss,
    reason = "test fixtures: counts of a few thousand travellers"
)]

use std::sync::{Arc, Mutex};

use openmobisim_core_choice::{
    ChoiceBatch, ChoiceError, ChoiceModel, Choices, Options as ChoiceOptions, model,
};
use openmobisim_core_demand::{ClassDefaults, Ownership, RawTrip, build_travellers};
use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
use openmobisim_core_graph::examples::manhattan_grid;
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::network::{LinkSpec, RoadNetwork, RoadNetworkBuilder};
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_loading::FidelityLevel;
use openmobisim_core_routes::{Demand, Route, RouteSetGenerator, Search, Shortest};
use openmobisim_core_sim::equilibration::{GAP_ACCEPTABLE, Options, strategy};
use openmobisim_core_sim::route_update::{self, RouteUpdateError};
use openmobisim_core_sim::{
    Equilibration, FlowMotor, IterationReport, Msa, NO_ROUTE, Run, RunResult,
};
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::{EntityId, LinkId, NodeId};
use openmobisim_core_types::rng::StreamRng;
use openmobisim_core_types::time::Second;
use openmobisim_core_types::units::Duration;

const O: (f64, f64) = (4.800, 45.700);
const D: (f64, f64) = (4.810, 45.700);

/// Two ways from `o` to `d`: a short one-lane residential road (`fast`), and a longer
/// two-lane primary road through `m` (`slow`), each link its own street. At free flow `fast`
/// is 12 s quicker; with a queue on it `slow` is several times quicker.
fn bottleneck() -> RoadNetwork {
    let mut b = RoadNetworkBuilder::new();
    b.add_node("o", LonLat::new(O.0, O.1));
    b.add_node("m", LonLat::new(4.805, 45.7004));
    b.add_node("d", LonLat::new(D.0, D.1));
    let mut fast = LinkSpec::new(RoadClass::Residential);
    fast.lanes = Some(1);
    fast.length_m = Some(600.0);
    b.add_link("fast", "o", "d", fast);
    let mut slow = LinkSpec::new(RoadClass::Primary);
    slow.lanes = Some(2);
    slow.length_m = Some(700.0);
    b.add_link("slow1", "o", "m", slow);
    b.add_link("slow2", "m", "d", slow);
    b.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
        .expect("buildable")
}

fn trips(n: u32, spread_s: u32) -> Vec<RawTrip> {
    (0..n)
        .map(|i| RawTrip {
            traveller_id: format!("p{i}"),
            trip_seq: 0,
            origin: LonLat::new(O.0, O.1),
            destination: LonLat::new(D.0, D.1),
            departure_time: Second(i * spread_s / n.max(1)),
            user_class: "commuter".to_string(),
            weight: None,
        })
        .collect()
}

fn car_owning() -> ClassDefaults {
    ClassDefaults::new().with_default("commuter", Ownership { car: true, ..Ownership::NONE })
}

fn options(pairs: &[(&str, f64)]) -> route_update::Options {
    pairs.iter().map(|&(k, v)| (k.to_string(), v)).collect()
}

/// A run on the bottleneck: `n` trips over 15 minutes, the link transmission model, a logit,
/// `iterations` of the method of successive averages.
struct Setup {
    n: u32,
    iterations: f64,
    /// Route-set method: `shortest` makes the set hold only the road that jams.
    generator: &'static str,
    update: (&'static str, Vec<(&'static str, f64)>),
    choice: &'static str,
    seed: u64,
}

impl Setup {
    fn new(n: u32) -> Self {
        Self {
            n,
            iterations: 8.0,
            generator: "shortest",
            update: ("best_response", Vec::new()),
            choice: "logit",
            seed: 1,
        }
    }

    fn without_update(mut self) -> Self {
        self.update = ("none", Vec::new());
        self
    }

    fn run(&self) -> Run {
        let network = Arc::new(bottleneck());
        let (travellers, raw) = build_travellers(
            trips(self.n, 900),
            Vec::new(),
            &car_owning(),
            1,
            &mut Diagnostics::new(),
        )
        .expect("buildable");
        let turns = Arc::new(TurnTable::build(&network, SignalDefaults::SHIPPED));
        let choice: ChoiceOptions = [("beta_time_min".to_string(), -2.0)].into_iter().collect();
        let update_options = options(&self.update.1);
        let gap_options: Options =
            [("iterations".to_string(), self.iterations), ("gap_sample".to_string(), 400.0)]
                .into_iter()
                .collect();
        Run::new(network, Arc::new(travellers), Arc::new(raw), Second(3600))
            .with_flow_motor(FlowMotor::Ltm {
                turns,
                step: Duration(60.0),
                level: FidelityLevel::Full,
            })
            .with_master_seed(self.seed)
            .with_route_generator(Arc::from(
                openmobisim_core_routes::generator(self.generator, &Default::default())
                    .expect("built in"),
            ))
            .with_choice_model(Arc::from(model(self.choice, &choice).expect("built in")))
            .with_equilibration(Arc::from(strategy("msa", &gap_options).expect("built in")))
            .with_route_update(Arc::from(
                route_update::update(self.update.0, &update_options).expect("built in"),
            ))
    }

    fn go(&self) -> RunResult {
        self.run().execute(&mut Diagnostics::new())
    }
}

// --- the case the update exists for ----------------------------------------------------------

#[test]
fn a_road_that_only_pays_under_congestion_enters_the_set_and_the_gap_to_the_network_falls() {
    // The set holds the fast road alone ("shortest"), and 1 500 travellers jam it. The slow road
    // is the network's own answer: several times quicker with a queue on the other.
    let without = Setup::new(1_500).without_update().go();
    let with = Setup::new(1_500).go();

    // Without the update the set never changes, and the whole-network test says what that costs.
    let sets = without.route_sets.as_ref().expect("sets");
    assert_eq!(sets.route_count(), 1, "the fast road, and nothing else");
    assert!(without.iterations.iter().all(|it| it.routes_added == 0 && it.route_searches == 0));
    let stuck = without.iterations.last().unwrap();
    assert!(stuck.gap.abs() < 1e-12 && stuck.gap_network > 2.0, "{stuck:?}");

    // With it, the first loading shows the slow road to be worth having: one search (one pair,
    // one departure), one route, added after the loading for the next iteration to choose among.
    let first = &with.iterations[0];
    assert_eq!((first.route_searches, first.routes_added), (1, 1), "{first:?}");
    // The pair has both roads now, the fast one first, the slow one added at iteration 1.
    let sets = with.route_sets.as_ref().expect("sets");
    assert_eq!(sets.route_count(), 2);
    assert_eq!(sets.stamps(), &[0, 1], "made by the generator, and added for iteration 1");
    assert_eq!(sets.route(1).links.len(), 2, "the slow road: two links");
    assert_eq!(sets.route(0).links.len(), 1, "the fast road: one link");
    // Its cost is its free flow time (12 s more than the fast road's), and it shares nothing.
    assert!(
        sets.route(1).cost > sets.route(0).cost,
        "{} {}",
        sets.route(1).cost,
        sets.route(0).cost
    );
    assert!(sets.route(1).overlap.abs() < 1e-9, "the roads share no link");
    // Nothing is added once the set has it: the road is known, and no other exists.
    assert!(with.iterations[1..].iter().all(|it| it.routes_added == 0), "{:?}", with.iterations);
    // A search is made after every loading but the last, which no choice follows.
    let last = with.iterations.len() - 1;
    assert!(with.iterations[..last].iter().all(|it| it.route_searches == 1));
    assert_eq!(with.iterations[last].route_searches, 0);

    // Travellers use it, the queue eases, and the time everyone spends falls.
    let took_slow = with
        .route_choices
        .as_ref()
        .expect("choices")
        .route
        .iter()
        .filter(|&&r| r as usize == 1)
        .count();
    assert!(took_slow > 500, "{took_slow} of 1 500 on the slow road");
    assert!(
        with.total_travel_time.get() < 0.4 * without.total_travel_time.get(),
        "{} against {}",
        with.total_travel_time.get(),
        without.total_travel_time.get()
    );
    // And the whole network agrees: no faster route was missing.
    let gap = with.iterations.last().unwrap().gap_network;
    assert!(gap < GAP_ACCEPTABLE, "{gap}");
    // The gap against the set, measured against the grown set, starts far from equilibrium (the
    // jam) and settles as the travellers move: from about 17 to a few per cent.
    let (first, late) =
        (with.iterations[0].gap, with.iterations[5..].iter().map(|it| it.gap).sum::<f64>() / 3.0);
    assert!(first > 5.0 && late < GAP_ACCEPTABLE, "{first} at first, {late} late");
    // The choice model's alternatives are counted afresh: every trip knew of two routes at the end.
    let rc = with.route_choices.as_ref().unwrap();
    assert!(rc.alternatives.iter().all(|&a| a == 2), "alternatives follow the grown set");
    assert!(rc.route.iter().all(|&r| r != NO_ROUTE && r < 2));
    assert_eq!(sets.update(), "best_response;max_routes=10;searches=1;slack=0.02");
}

#[test]
fn the_gap_against_the_set_is_that_of_the_set_as_grown() {
    // Iteration 0 chose on free flow: everyone is on the fast road, jammed. The update then adds
    // the slow road before the assessment, so the gap of iteration 0 is against a set that holds
    // it: large (the jam is several times the slow road's time), not the zero it was without.
    let with = Setup::new(1_500).go();
    let without = Setup::new(1_500).without_update().go();
    assert!(without.iterations[0].gap.abs() < 1e-12);
    assert!(with.iterations[0].gap > 1.0, "{}", with.iterations[0].gap);
}

// --- off means absent -------------------------------------------------------------------------

#[test]
fn no_update_is_the_run_without_one_bit_for_bit() {
    let plain = {
        // A run that never mentions the update.
        let mut setup = Setup::new(600);
        setup.generator = "penalty";
        let mut run = setup.run();
        run = run.with_route_update(Arc::new(route_update::NoRouteUpdate));
        run
    };
    let explicit = {
        let mut setup = Setup::new(600).without_update();
        setup.generator = "penalty";
        setup.run()
    };
    assert_eq!(plain.description(), explicit.description(), "the same fingerprint");
    assert_eq!(plain.description().route_update, "none");
    let (a, b) = (
        {
            let mut p = plain;
            p.execute(&mut Diagnostics::new())
        },
        {
            let mut e = explicit;
            e.execute(&mut Diagnostics::new())
        },
    );
    assert_eq!(a, b);
}

#[test]
fn an_update_is_part_of_what_a_run_was() {
    let base = Setup::new(300).without_update().run().description();
    let one = Setup::new(300).run().description();
    let two = Setup { update: ("best_response", vec![("searches", 2.0)]), ..Setup::new(300) }
        .run()
        .description();
    assert_ne!(base.fingerprint, one.fingerprint);
    assert_ne!(one.fingerprint, two.fingerprint, "its options too");
    assert_eq!(one.route_update, "best_response");
    assert_eq!(one.route_update_descriptor, "best_response;max_routes=10;searches=1;slack=0.02");
    assert_eq!(base.route_update_descriptor, "none");
}

#[test]
fn light_demand_adds_nothing_and_changes_nothing_but_the_fingerprint() {
    // Twenty travellers in fifteen minutes never queue: no route beats the fast road's, so no
    // search finds one, and the run is the run without the update. With the default slack the
    // update does not even look (the best route is at free flow: nothing to gain); with a slack of
    // 0 it looks after every loading but the last and finds nothing.
    let with = Setup::new(20).go();
    let without = Setup::new(20).without_update().go();
    assert!(with.iterations.iter().all(|it| it.routes_added == 0), "{:?}", with.iterations);
    assert!(with.iterations.iter().all(|it| it.route_searches == 0), "no search: nothing to gain");
    let looked = Setup { update: ("best_response", vec![("slack", 0.0)]), ..Setup::new(20) }.go();
    let last = looked.iterations.len() - 1;
    assert!(looked.iterations.iter().all(|it| it.routes_added == 0));
    assert!(
        looked.iterations[..last].iter().all(|it| it.route_searches == 1),
        "with no slack it looked, every time"
    );
    assert_eq!(looked.route_choices, with.route_choices);
    let (rw, ro) = (with.route_sets.as_ref().unwrap(), without.route_sets.as_ref().unwrap());
    assert_eq!(rw.route_count(), 1);
    assert_eq!(rw.stamps(), ro.stamps());
    assert_eq!(with.route_choices, without.route_choices);
    assert_eq!(with.events, without.events);
    assert_eq!(with.total_travel_time, without.total_travel_time);
    // The reports agree but for the searches made.
    for (a, b) in with.iterations.iter().zip(&without.iterations) {
        let mut a = *a;
        a.route_searches = 0;
        assert_eq!(a, *b);
    }
}

// --- the options --------------------------------------------------------------------------------

#[test]
fn a_pair_at_its_limit_is_not_searched_and_a_limit_is_kept() {
    // A limit of one route: the set already holds it, so no search is made and none added.
    let capped =
        Setup { update: ("best_response", vec![("max_routes", 1.0)]), ..Setup::new(1_500) }.go();
    assert!(capped.iterations.iter().all(|it| it.route_searches == 0 && it.routes_added == 0));
    assert_eq!(capped.route_sets.as_ref().unwrap().route_count(), 1);
    // A limit of two: the slow road fits, and nothing more could be added.
    let two =
        Setup { update: ("best_response", vec![("max_routes", 2.0)]), ..Setup::new(1_500) }.go();
    assert_eq!(two.route_sets.as_ref().unwrap().route_count(), 2);
    // Once full, a pair is no longer searched.
    assert!(two.iterations[1..].iter().all(|it| it.route_searches == 0), "{:?}", two.iterations);
}

#[test]
fn several_searches_a_pair_are_spread_over_its_departures() {
    // (A slack of 0, so that every departure is searched; the early ones are at free flow.)
    let three = Setup {
        update: ("best_response", vec![("searches", 3.0), ("slack", 0.0)]),
        ..Setup::new(1_500)
    }
    .go();
    assert_eq!(three.iterations[0].route_searches, 3, "three quantiles of the pair's departures");
    // Fewer trips than searches: one search each, at most.
    let few = Setup {
        update: ("best_response", vec![("searches", 16.0), ("slack", 0.0)]),
        ..Setup::new(4)
    }
    .go();
    assert_eq!(few.iterations[0].route_searches, 4);
}

#[test]
fn updates_are_chosen_by_name_and_bad_options_say_what_is_wrong() {
    let none = Default::default();
    assert_eq!(route_update::Registry::builtin().names(), vec!["none", "best_response"]);
    assert_eq!(route_update::DEFAULT_UPDATE, "none");
    assert!(!route_update::update("none", &none).unwrap().is_active());
    assert!(route_update::update("best_response", &none).unwrap().is_active());
    let e = route_update::update("nope", &none).err().expect("unknown");
    assert!(matches!(e, RouteUpdateError::UnknownUpdate { .. }));
    assert!(e.to_string().contains("none, best_response"), "{e}");
    let e = route_update::update("best_response", &options(&[("nope", 1.0)])).err().unwrap();
    assert!(e.to_string().contains("max_routes, searches"), "{e}");
    let e = route_update::update("none", &options(&[("searches", 1.0)])).err().unwrap();
    assert!(e.to_string().contains("no options"), "{e}");
    for (name, bad) in [
        ("searches", 0.0),
        ("searches", 17.0),
        ("searches", 1.5),
        ("max_routes", 0.0),
        ("max_routes", 33.0),
        ("max_routes", f64::NAN),
        ("slack", -0.1),
        ("slack", 1.5),
        ("slack", f64::NAN),
    ] {
        let e =
            route_update::update("best_response", &options(&[(name, bad)])).err().expect("refused");
        assert!(matches!(e, RouteUpdateError::BadOption { .. }), "{name}={bad}: {e}");
        assert!(e.to_string().contains(name), "{e}");
    }
    let ok = route_update::update(
        "best_response",
        &options(&[("searches", 16.0), ("max_routes", 32.0)]),
    )
    .unwrap();
    assert_eq!(ok.descriptor(), "best_response;max_routes=32;searches=16;slack=0.02");
}

// --- the stopping rule --------------------------------------------------------------------------

#[test]
fn a_tolerance_is_not_met_while_routes_are_still_being_added() {
    let msa = Msa { iterations: 10, gap_tolerance: 0.1, gap_sample: 0, cost_bin_s: 300, warmup: 0 };
    let report = |i: u32, added: u32| IterationReport {
        gap: 0.01,
        routes_added: added,
        ..IterationReport::unmeasured(i)
    };
    let quiet: Vec<_> = (0..5).map(|i| report(i, 0)).collect();
    assert!(msa.is_converged(&quiet), "a small gap, and nothing added");
    let mut growing = quiet.clone();
    growing[4] = report(4, 3);
    assert!(!msa.is_converged(&growing), "a small gap, but the last update added routes");
    // An earlier addition does not hold the run back once the last update added none.
    let mut earlier = quiet;
    earlier[2] = report(2, 3);
    assert!(msa.is_converged(&earlier));
}

// --- a route as good as the set's best is worth having ------------------------------------------------

/// Two roads of exactly the same length and class from `o` to `d`, each its own street.
fn twin_roads() -> RoadNetwork {
    let mut b = RoadNetworkBuilder::new();
    b.add_node("o", LonLat::new(O.0, O.1));
    b.add_node("a", LonLat::new(4.805, 45.7004));
    b.add_node("b", LonLat::new(4.805, 45.6996));
    b.add_node("d", LonLat::new(D.0, D.1));
    let mut spec = LinkSpec::new(RoadClass::Primary);
    spec.length_m = Some(400.0);
    for (name, from, to) in [("oa", "o", "a"), ("ad", "a", "d"), ("ob", "o", "b"), ("bd", "b", "d")]
    {
        b.add_link(name, from, to, spec);
    }
    b.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
        .expect("buildable")
}

/// A generator that makes one route per pair: the `pick`th of the shortest and the shortest
/// once the first is penalised (on twin roads, one road each).
struct OneOfTwo {
    pick: usize,
}

impl RouteSetGenerator for OneOfTwo {
    fn name(&self) -> &str {
        "one_of_two"
    }
    fn descriptor(&self) -> String {
        format!("one_of_two;pick={}", self.pick)
    }
    fn generate(&self, search: &mut Search<'_>, origin: NodeId, destination: NodeId) -> Vec<Route> {
        search.clear_route_state();
        let first = search.shortest(origin, destination).expect("connected");
        for &l in &first.links {
            search.factors.multiply(l, 2.0);
        }
        let second = search.shortest(origin, destination).expect("connected");
        search.clear_route_state();
        vec![[first, second].into_iter().nth(self.pick).unwrap()]
    }
}

#[test]
fn an_equally_good_route_is_added_and_a_route_the_set_holds_never_is() {
    // Two roads that take exactly the same time. The set holds one; the search finds one of the
    // two (which, link ids settle). If it is the road the set holds, that is nothing new. If it is
    // the other, it is as good as the set's best and it is added: an equally good alternative is
    // one more for the travellers to spread over (on Seoul's grid of equal blocks, leaving such
    // ties out ended at a network gap of 0.57 instead of 0.12). Each road is tried as the one the
    // set holds, so that both cases happen.
    let mut sizes = Vec::new();
    for pick in [0, 1] {
        let network = Arc::new(twin_roads());
        let (travellers, raw) =
            build_travellers(trips(20, 900), Vec::new(), &car_owning(), 1, &mut Diagnostics::new())
                .unwrap();
        let turns = Arc::new(TurnTable::build(&network, SignalDefaults::SHIPPED));
        let mut run = Run::new(network, Arc::new(travellers), Arc::new(raw), Second(3600))
            .with_flow_motor(FlowMotor::Ltm {
                turns,
                step: Duration(60.0),
                level: FidelityLevel::Full,
            })
            .with_route_generator(Arc::new(OneOfTwo { pick }))
            .with_equilibration(Arc::from(
                strategy("msa", &[("iterations".to_string(), 3.0)].into_iter().collect()).unwrap(),
            ))
            .with_route_update(Arc::from(
                route_update::update("best_response", &options(&[("slack", 0.0)])).unwrap(),
            ));
        let result = run.execute(&mut Diagnostics::new());
        let sets = result.route_sets.as_ref().unwrap();
        let roads: Vec<Vec<u32>> = sets.routes(0).map(|r| r.links.to_vec()).collect();
        assert!(
            roads.iter().enumerate().all(|(i, r)| !roads[..i].contains(r)),
            "pick {pick}: a route is in its set once"
        );
        // The other road is added once, at the first loading, or not at all.
        let added: Vec<u32> = result.iterations.iter().map(|it| it.routes_added).collect();
        assert_eq!(added.iter().sum::<u32>() as usize, roads.len() - 1, "pick {pick}: {added:?}");
        assert!(added[1..].iter().all(|&a| a == 0), "pick {pick}: {added:?}");
        assert_eq!(result.iterations[0].route_searches, 1, "it looked");
        sizes.push(roads.len());
    }
    sizes.sort_unstable();
    assert_eq!(sizes, [1, 2], "the road the search prefers is not added again; the other is");
}

#[test]
fn a_run_stops_on_its_tolerance_only_after_the_routes_have_stopped_coming() {
    // A tolerance of 15% on the bottleneck: the jam shows the slow road at once, and the run stops
    // only after it is in the set, the travellers have moved onto it, and nothing more is added.
    let gap_options = |tolerance: f64| -> Options {
        [
            ("iterations".to_string(), 20.0),
            ("gap_tolerance".to_string(), tolerance),
            ("gap_sample".to_string(), 400.0),
        ]
        .into_iter()
        .collect()
    };
    let with_tolerance = |tolerance: f64| {
        Setup::new(1_500)
            .run()
            .with_equilibration(Arc::from(strategy("msa", &gap_options(tolerance)).unwrap()))
            .execute(&mut Diagnostics::new())
    };
    let tolerant = with_tolerance(0.15);
    assert!(tolerant.converged && tolerant.iterations.len() < 20, "{}", tolerant.iterations.len());
    assert_eq!(tolerant.iterations[0].routes_added, 1, "the slow road, at the first loading");
    assert_eq!(tolerant.iterations.last().unwrap().routes_added, 0);
    let never = with_tolerance(0.0);
    assert!(never.iterations.len() == 20 && !never.converged, "no tolerance: never stops early");
}

// --- a method that reads the demand ---------------------------------------------------------------------

/// One trip as a method was told of it: origin, destination, weight, departure, and how many
/// nodes the network it was told about has.
type Told = (u32, u32, u32, u32, u32);

/// A method that makes the shortest route and records the demand it was told about.
struct ReadsDemand {
    told: Arc<Mutex<Vec<Told>>>,
    prepared: bool,
}

impl RouteSetGenerator for ReadsDemand {
    fn name(&self) -> &str {
        if self.prepared { "reads_demand_prepared" } else { "reads_demand" }
    }
    fn descriptor(&self) -> String {
        "reads_demand".to_string()
    }
    fn generate(&self, search: &mut Search<'_>, origin: NodeId, destination: NodeId) -> Vec<Route> {
        Shortest.generate(search, origin, destination)
    }
    fn reads_demand(&self) -> bool {
        !self.prepared
    }
    fn with_demand(&self, demand: &Demand<'_>) -> Option<Box<dyn RouteSetGenerator>> {
        let mut told = self.told.lock().unwrap();
        for t in demand.trips {
            told.push((
                t.key.origin,
                t.key.destination,
                t.weight,
                t.departure,
                demand.network.node_count(),
            ));
        }
        Some(Box::new(Self { told: self.told.clone(), prepared: true }))
    }
}

#[test]
fn a_run_tells_a_method_that_reads_the_demand_every_trip_before_it_generates() {
    let told = Arc::new(Mutex::new(Vec::new()));
    // Trips carry the scenario's default weight: 3 people each.
    let (travellers, raw) =
        build_travellers(trips(40, 900), Vec::new(), &car_owning(), 3, &mut Diagnostics::new())
            .unwrap();
    let mut run =
        Run::new(Arc::new(bottleneck()), Arc::new(travellers), Arc::new(raw), Second(3600))
            .with_route_generator(Arc::new(ReadsDemand { told: told.clone(), prepared: false }));
    let description = run.description();
    let result = run.execute(&mut Diagnostics::new());
    let told = told.lock().unwrap();
    assert_eq!(told.len(), 40, "every trip, once");
    assert!(told.iter().all(|t| (t.0, t.1, t.2, t.4) == (told[0].0, told[0].1, 3, 3)));
    let mut departures: Vec<u32> = told.iter().map(|t| t.3).collect();
    departures.sort_unstable();
    assert_eq!(departures, (0..40).map(|i| i * 900 / 40).collect::<Vec<u32>>());
    // The sets were made by the method that was given the demand; the run's description is of
    // the method it was given.
    assert_eq!(result.route_sets.as_ref().unwrap().method(), "reads_demand_prepared");
    assert_eq!(description.route_method, "reads_demand");
    // A method that does not read the demand is never handed it: no cost, nothing built.
    let plain = Setup::new(40).without_update().go();
    assert_eq!(plain.route_sets.as_ref().unwrap().method(), "shortest");
}

#[test]
fn the_demand_decides_what_the_congestion_biased_method_finds() {
    // Heavy demand jams the fast road, so its links are the ones the noise moves and the slow
    // road, a fifth dearer at free flow, is found; light demand leaves the fast road's links
    // alone, and nothing moves them enough for the slow road to be cheaper on any draw.
    let sets = |n: u32| {
        let mut setup = Setup::new(n).without_update();
        setup.generator = "montecarlo";
        setup.iterations = 1.0;
        setup.go().route_sets.unwrap()
    };
    let heavy = sets(1_500);
    assert_eq!(heavy.method(), "montecarlo");
    assert_eq!(heavy.route_count(), 2, "the road that the demand will jam, and the one round it");
    assert!(heavy.descriptor().contains("bias=demand"));
    assert_eq!(
        sets(20).route_count(),
        1,
        "light demand: nothing likely to congest, nothing to avoid"
    );
}

// --- the slack: no search where there is nothing to gain -----------------------------------------------

#[test]
fn a_pair_within_the_slack_of_free_flow_is_not_searched_and_one_beyond_it_is() {
    use openmobisim_core_loading::{EntryTables, LinkBinRecorder};
    use openmobisim_core_routes::{RouteKey, RouteSets};
    use openmobisim_core_sim::LinkTimes;
    use openmobisim_core_sim::route_update::{BestResponse, RouteUpdate, UpdateContext};

    // The fast road takes 72 s at free flow (its set's only route); the slow road 84 s, where
    // nobody was recorded. Hand-made times: the fast road took `took` seconds for everyone.
    let network = bottleneck();
    let turns = TurnTable::build(&network, SignalDefaults::SHIPPED);
    let node = |n: &str| network.node_external_ids().typed_id_of::<NodeId>(n).unwrap();
    let key = RouteKey::new(node("o"), node("d"));
    let sets = RouteSets::generate(&network, &turns, &[key], &openmobisim_core_routes::Shortest);
    let free = f64::from(sets.route(0).cost);
    assert!((free - 72.0).abs() < 1.0, "the fast road: {free}");
    let (travellers, raw) =
        build_travellers(trips(40, 900), Vec::new(), &car_owning(), 1, &mut Diagnostics::new())
            .unwrap();
    let trip_keys = vec![key; raw.len() as usize];
    let fast = network.link_external_ids().typed_id_of::<LinkId>("fast").unwrap();
    let searches = |took: f64, slack: f64| {
        let mut recorder = LinkBinRecorder::new(network.link_count() as usize, 3_600, 3_600.0);
        recorder.record(fast, 0.0, took, 1.0);
        let empty = LinkBinRecorder::new(network.link_count() as usize, 3_600, 3_600.0).finish();
        let times = LinkTimes::from_tables(
            &network,
            &EntryTables { entry: recorder.finish(), origin_wait: empty },
        );
        let update = BestResponse { searches: 1, max_routes: 10, slack };
        let found = update.update(&UpdateContext {
            network: &network,
            turns: &turns,
            trips: &raw,
            trip_keys: &trip_keys,
            route_sets: &sets,
            times: &times,
            iteration: 1,
        });
        (found.searches, found.route_count())
    };
    let _ = travellers;
    // 10% over free flow: within a 12% slack (skipped), beyond a 5% one (searched, nothing better than the fast road).
    assert_eq!(searches(1.10 * free, 0.12), (0, 0));
    assert_eq!(searches(1.10 * free, 0.05), (1, 0));
    // No slack: searched even at free flow, where there is nothing to find.
    assert_eq!(searches(free, 0.0), (1, 0));
    // The bound: 39% over free flow (100 s) is beyond any slack under 0.39; the slow road (84 s) is then found.
    assert_eq!(searches(100.0, 0.3), (1, 1));
    assert_eq!(searches(100.0, 0.02), (1, 1));
    // A slack over the excess leaves it: at most `slack` was left to gain, and here more was (16 s of 100).
    assert_eq!(searches(100.0, 0.45), (0, 0));
}

// --- what is added ---------------------------------------------------------------------------------

/// A model that records the identities of the alternatives of every situation it is given,
/// and takes the first.
struct Identities {
    seen: Mutex<Vec<(u32, u32, Vec<u32>)>>,
}

impl ChoiceModel for Identities {
    fn name(&self) -> &str {
        "identities"
    }
    fn descriptor(&self) -> String {
        "identities".to_string()
    }
    fn is_sampled(&self) -> bool {
        false
    }
    fn required_attributes(&self) -> Option<Vec<String>> {
        Some(Vec::new())
    }
    fn choose(&self, batch: &ChoiceBatch, _rng: &StreamRng) -> Result<Choices, ChoiceError> {
        let mut seen = self.seen.lock().unwrap();
        for s in 0..batch.situations() {
            let ids = batch.range(s).map(|a| batch.identities()[a]).collect();
            seen.push((batch.iteration(), batch.trips()[s], ids));
        }
        Ok(Choices {
            chosen: vec![0; batch.situations()],
            probability: vec![f64::NAN; batch.situations()],
        })
    }
}

#[test]
fn a_route_that_was_in_the_set_keeps_its_identity_when_another_is_added() {
    // Identity is what a draw is keyed on (design §11.1(4)): the routes travellers already know
    // must be drawn as they were. The model is shown each situation's alternatives' identities.
    let spy = Arc::new(Identities { seen: Mutex::new(Vec::new()) });
    let setup = Setup { iterations: 3.0, ..Setup::new(1_500) };
    let run = setup.run().with_choice_model(spy.clone()).with_choice_detour_limit(0.0);
    let mut run = run;
    let result = run.execute(&mut Diagnostics::new());
    assert_eq!(result.route_sets.as_ref().unwrap().route_count(), 2);
    let seen = spy.seen.lock().unwrap();
    let of = |iteration: u32, trip: u32| {
        seen.iter().find(|s| s.0 == iteration && s.1 == trip).map(|s| s.2.clone())
    };
    // Trip 0 was asked at iteration 0 (free flow: one route). Whoever is asked again later sees
    // the fast road under the identity it had, and the slow road beside it.
    let first = of(0, 0).expect("asked at iteration 0");
    assert_eq!(first.len(), 1);
    let later: Vec<Vec<u32>> =
        (1..1_500u32).filter_map(|t| of(1, t).or_else(|| of(2, t))).collect();
    assert!(!later.is_empty(), "some travellers chose again");
    for ids in later {
        assert_eq!(ids.len(), 2, "the grown set");
        assert_eq!(ids[0], first[0], "the fast road, under the identity it had");
        assert_ne!(ids[0], ids[1]);
    }
}

#[test]
fn what_is_added_is_a_route_of_the_network_and_new_to_its_set() {
    // A grid with many pairs, jammed, so that many routes are added; then every one is checked
    // against the network: connected, from the origin to the destination, loop-free, drivable,
    // turns allowed, distinct within its set, costed at free flow, stamped.
    let (network, _) = manhattan_grid(7, 200.0, true);
    let network = Arc::new(network);
    let turns = Arc::new(TurnTable::build(&network, SignalDefaults::SHIPPED));
    let n = network.node_count();
    let name = |i: u32, j: u32| format!("n{i}_{j}");
    let _ = (n, name);
    let raw: Vec<RawTrip> = grid_trips(&network, 7, 3_000);
    let (travellers, raw) =
        build_travellers(raw, Vec::new(), &car_owning(), 1, &mut Diagnostics::new()).unwrap();
    let mut run = Run::new(network.clone(), Arc::new(travellers), Arc::new(raw), Second(3600))
        .with_flow_motor(FlowMotor::Ltm {
            turns: turns.clone(),
            step: Duration(30.0),
            level: FidelityLevel::Full,
        })
        .with_master_seed(3)
        .with_route_generator(Arc::from(
            openmobisim_core_routes::generator("penalty", &Default::default()).unwrap(),
        ))
        .with_choice_model(Arc::from(model("logit", &Default::default()).unwrap()))
        .with_equilibration(Arc::from(
            strategy("msa", &[("iterations".to_string(), 5.0)].into_iter().collect()).unwrap(),
        ))
        .with_route_update(Arc::from(
            route_update::update("best_response", &Default::default()).unwrap(),
        ));
    let result = run.execute(&mut Diagnostics::new());
    let sets = result.route_sets.as_ref().expect("sets");
    let added: usize = result.iterations.iter().map(|it| it.routes_added as usize).sum();
    assert!(added > 20, "the test needs routes to check: {added} were added");
    assert_eq!(sets.stamps().iter().filter(|&&s| s > 0).count(), added);
    let mut checked = 0_usize;
    for k in 0..sets.keys().len() {
        let key = sets.keys()[k];
        let mut seen: Vec<&[u32]> = Vec::new();
        let mut last_stamp = 0;
        for r in sets.route_range(k) {
            let view = sets.route(r);
            let stamp = sets.stamps()[r];
            // The generator's routes first, then the added ones, in the order they were added.
            assert!(
                stamp >= last_stamp,
                "stamps only grow along a set: {stamp} after {last_stamp}"
            );
            last_stamp = stamp;
            assert!(!seen.contains(&view.links), "a route is in its set once");
            seen.push(view.links);
            if stamp == 0 {
                continue;
            }
            checked += 1;
            assert!(stamp < 5, "added for an iteration the run made: {stamp}");
            let links: Vec<LinkId> = view.links.iter().map(|&l| LinkId::new(l)).collect();
            assert_eq!(network.link_from(links[0]).raw(), key.origin, "starts at the origin");
            assert_eq!(network.link_to(*links.last().unwrap()).raw(), key.destination);
            let mut visited = vec![network.link_from(links[0]).raw()];
            let mut cost = 0.0;
            for w in links.windows(2) {
                assert_eq!(network.link_to(w[0]), network.link_from(w[1]), "connected");
                assert!(
                    turns.turns_from(w[0]).iter().any(|&t| turns.outgoing(t) == w[1]),
                    "the turn is allowed"
                );
            }
            for &l in &links {
                assert!(network.link_class(l).carries_motor_traffic());
                let to = network.link_to(l).raw();
                assert!(!visited.contains(&to), "loop-free");
                visited.push(to);
                cost += network.free_flow_time(l).get();
            }
            assert!((f64::from(view.cost) - cost).abs() < 1e-2 * cost.max(1.0), "free-flow cost");
            assert!((0.0..=1.0).contains(&view.overlap));
        }
    }
    assert_eq!(checked, added);
    // The choices are consistent with the grown sets.
    let rc = result.route_choices.as_ref().unwrap();
    for t in 0..rc.route.len() {
        if rc.route[t] == NO_ROUTE {
            continue;
        }
        let k = sets.key_of_route(rc.route[t] as usize);
        assert_eq!(rc.alternatives[t] as usize, sets.route_range(k).len());
    }
}

fn grid_trips(network: &RoadNetwork, side: u32, count: u32) -> Vec<RawTrip> {
    // Deterministic pseudo-random pairs of distinct nodes, departing within ten minutes.
    let nodes = network.node_count();
    let mut state = 0x9E37_79B9_u64;
    let mut next = |bound: u32| {
        state =
            state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        ((state >> 33) % u64::from(bound)) as u32
    };
    let _ = side;
    (0..count)
        .map(|i| {
            let (a, b) = (next(nodes), next(nodes));
            let (from, to) = (
                network.node_lonlat(openmobisim_core_types::ids::NodeId::new(a)),
                network.node_lonlat(openmobisim_core_types::ids::NodeId::new(b)),
            );
            RawTrip {
                traveller_id: format!("g{i}"),
                trip_seq: 0,
                origin: from,
                destination: to,
                departure_time: Second(next(600)),
                user_class: "commuter".to_string(),
                weight: None,
            }
        })
        .collect()
}

// --- determinism ------------------------------------------------------------------------------------

#[test]
fn an_adapting_run_is_the_same_at_one_thread_and_at_seven() {
    let at = |threads: usize| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .expect("a pool")
            .install(|| Setup { seed: 9, ..Setup::new(1_200) }.go())
    };
    let (one, seven) = (at(1), at(7));
    assert!(one.iterations.iter().map(|it| it.routes_added).sum::<u32>() > 0, "it adapted");
    assert_eq!(one, seven, "results");
    assert_eq!(one.route_sets, seven.route_sets, "sets");
    assert_eq!(one.route_choices, seven.route_choices, "choices");
    assert_eq!(one.iterations, seven.iterations, "reports");
    // And run again on the same pool: the same.
    assert_eq!(one, Setup { seed: 9, ..Setup::new(1_200) }.go());
}
