//! The route-set cache (S178).
//!
//! What is defended, and how each is checked:
//!
//! * **the cache is invisible in the results**: a run that takes its sets from the cache is the run
//!   that generated them, bit for bit, and a hit does not call the generator;
//! * **it only hands back what a miss would have made**: another method, another option, another
//!   network, other pairs, or (for a method that reads the demand) other trips or weights is another entry;
//! * **a run that grows its sets does not change the cached ones**;
//! * **capacity**: the least recently used goes first, and `clear` forgets all.

use std::sync::Arc;

use openmobisim_core_demand::{ClassDefaults, Ownership, RawTrip, build_travellers};
use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::network::{LinkSpec, RoadNetwork, RoadNetworkBuilder};
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_loading::FidelityLevel;
use openmobisim_core_routes::{Options, RouteKey, RouteSets, Shortest, generator};
use openmobisim_core_sim::equilibration::strategy;
use openmobisim_core_sim::route_cache::{CacheStats, generation_key};
use openmobisim_core_sim::route_update;
use openmobisim_core_sim::{FlowMotor, RouteSetCache, Run, RunResult};
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::time::Second;
use openmobisim_core_types::units::Duration;

const O: (f64, f64) = (4.800, 45.700);
const D: (f64, f64) = (4.810, 45.700);

fn bottleneck(slow_metres: f64) -> RoadNetwork {
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
    slow.length_m = Some(slow_metres);
    b.add_link("slow1", "o", "m", slow);
    b.add_link("slow2", "m", "d", slow);
    b.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
        .expect("buildable")
}

fn trips(n: u32) -> Vec<RawTrip> {
    (0..n)
        .map(|i| RawTrip {
            traveller_id: format!("p{i}"),
            trip_seq: 0,
            origin: LonLat::new(O.0, O.1),
            destination: LonLat::new(D.0, D.1),
            departure_time: Second(i * 900 / n.max(1)),
            user_class: "commuter".to_string(),
            weight: None,
            mode: None,
        })
        .collect()
}

/// One scenario; `method`, its options, the trips' number and weight, the network's slow road and
/// the route update are what a route-set cache should tell apart.
struct Case {
    method: &'static str,
    options: Vec<(&'static str, f64)>,
    trips: u32,
    weight: u32,
    slow_metres: f64,
    update: &'static str,
}

impl Case {
    fn new() -> Self {
        Self {
            method: "penalty",
            options: Vec::new(),
            trips: 300,
            weight: 1,
            slow_metres: 700.0,
            update: "none",
        }
    }

    fn run(&self, cache: Option<&Arc<RouteSetCache>>) -> RunResult {
        let network = Arc::new(bottleneck(self.slow_metres));
        let classes = ClassDefaults::new()
            .with_default("commuter", Ownership { car: true, ..Ownership::NONE });
        let (travellers, raw) = build_travellers(
            trips(self.trips),
            Vec::new(),
            &classes,
            self.weight,
            &mut Diagnostics::new(),
        )
        .expect("buildable");
        let turns = Arc::new(TurnTable::build(&network, SignalDefaults::SHIPPED));
        let options: Options = self.options.iter().map(|&(k, v)| (k.to_string(), v)).collect();
        let mut run = Run::new(network, Arc::new(travellers), Arc::new(raw), Second(3600))
            .with_flow_motor(FlowMotor::Ltm {
                turns,
                step: Duration(60.0),
                level: FidelityLevel::Full,
            })
            .with_route_generator(Arc::from(generator(self.method, &options).expect("built in")))
            .with_master_seed(3)
            .with_choice_model(Arc::from(
                openmobisim_core_choice::model("logit", &Default::default()).expect("built in"),
            ))
            .with_equilibration(Arc::from(
                strategy("msa", &[("iterations".to_string(), 3.0)].into_iter().collect())
                    .expect("built in"),
            ))
            .with_route_update(Arc::from(
                route_update::update(self.update, &Default::default()).expect("built in"),
            ));
        if let Some(cache) = cache {
            run = run.with_route_cache(cache.clone());
        }
        run.execute(&mut Diagnostics::new())
    }
}

#[test]
fn a_run_that_takes_its_sets_from_the_cache_is_the_run_that_made_them() {
    let cache = Arc::new(RouteSetCache::default());
    let case = Case::new();
    let plain = case.run(None);
    let first = case.run(Some(&cache));
    assert_eq!(cache.stats(), CacheStats { hits: 0, misses: 1, held: 1 });
    let second = case.run(Some(&cache));
    assert_eq!(cache.stats(), CacheStats { hits: 1, misses: 1, held: 1 }, "the second one hit");
    // Results, sets and choices: the same with the cache, with a hit, and without any.
    for other in [&first, &second] {
        assert_eq!(other, &plain);
        assert_eq!(other.route_sets, plain.route_sets);
        assert_eq!(other.route_choices, plain.route_choices);
        assert_eq!(other.iterations, plain.iterations);
    }
}

#[test]
fn only_the_same_inputs_share_an_entry() {
    let cache = Arc::new(RouteSetCache::default());
    Case::new().run(Some(&cache));
    let misses = |case: Case, expect: u64| {
        let before = cache.stats().misses;
        case.run(Some(&cache));
        assert_eq!(cache.stats().misses - before, expect, "hit or miss");
    };
    // Same everything: a hit. What the sets do not depend on: the update, the weights, the number of
    // trips between the same pair (a method that does not read the demand sees only its pairs).
    misses(Case::new(), 0);
    misses(Case { update: "best_response", ..Case::new() }, 0);
    misses(Case { weight: 5, ..Case::new() }, 0);
    misses(Case { trips: 50, ..Case::new() }, 0);
    // What they do depend on: the method, its options, the network.
    misses(Case { method: "shortest", ..Case::new() }, 1);
    misses(Case { options: vec![("max_paths", 2.0)], ..Case::new() }, 1);
    misses(Case { slow_metres: 800.0, ..Case::new() }, 1);
    // A method that reads the demand also depends on it: another number of trips, another weight.
    let mc = || Case { method: "montecarlo", ..Case::new() };
    misses(mc(), 1);
    misses(mc(), 0);
    misses(Case { trips: 50, ..mc() }, 1);
    misses(Case { weight: 5, ..mc() }, 1);
}

#[test]
fn a_run_that_grows_its_sets_leaves_the_cached_ones_as_the_method_made_them() {
    let cache = Arc::new(RouteSetCache::default());
    // A jam on the fast road with only the shortest route in the set: the update adds the slow road.
    let growing = Case { method: "shortest", trips: 1_500, update: "best_response", ..Case::new() };
    let grown = growing.run(Some(&cache));
    assert_eq!(grown.route_sets.as_ref().unwrap().route_count(), 2, "it grew");
    // The same scenario without the update takes the cached sets, and they are the method's: one route.
    let plain = Case { update: "none", ..growing }.run(Some(&cache));
    assert_eq!(cache.stats().hits, 1);
    assert_eq!(plain.route_sets.as_ref().unwrap().route_count(), 1);
    assert_eq!(plain.route_sets.as_ref().unwrap().update(), "");
}

#[test]
fn the_cache_keeps_the_most_recently_used_and_forgets_on_request() {
    let cache = RouteSetCache::new(2);
    let network = bottleneck(700.0);
    let turns = TurnTable::build(&network, SignalDefaults::SHIPPED);
    let node = |n: &str| {
        network.node_external_ids().typed_id_of::<openmobisim_core_types::ids::NodeId>(n).unwrap()
    };
    let keys = [RouteKey::new(node("o"), node("d"))];
    let made = std::cell::Cell::new(0);
    let get = |key: u64| {
        cache.get_or_generate(key, || {
            made.set(made.get() + 1);
            RouteSets::generate(&network, &turns, &keys, &Shortest)
        })
    };
    get(1);
    get(2);
    get(1); // 1 is now the most recent
    assert_eq!(made.get(), 2, "the third was a hit");
    get(3); // over capacity: 2, the least recently used, goes
    assert_eq!(cache.stats().held, 2);
    get(1);
    assert_eq!(made.get(), 3, "1 stayed");
    get(2);
    assert_eq!(made.get(), 4, "2 was forgotten");
    cache.clear();
    assert_eq!(cache.stats().held, 0);
    get(1);
    assert_eq!(made.get(), 5);
    // A capacity of 0 is 1: a cache always holds the last.
    let one = RouteSetCache::new(0);
    one.get_or_generate(7, || RouteSets::generate(&network, &turns, &keys, &Shortest));
    assert_eq!(one.stats().held, 1);
}

#[test]
fn the_key_does_not_depend_on_the_order_of_the_pairs_only_on_which_they_are() {
    let network = bottleneck(700.0);
    let node = |n: &str| {
        network.node_external_ids().typed_id_of::<openmobisim_core_types::ids::NodeId>(n).unwrap()
    };
    let (a, b, c) = (
        RouteKey::new(node("o"), node("d")),
        RouteKey::new(node("o"), node("m")),
        RouteKey::new(node("m"), node("d")),
    );
    let g = Shortest;
    let key = |keys: &[RouteKey]| generation_key(&network, &g, keys, None);
    assert_eq!(key(&[a, b, c]), key(&[c, a, b, a, a]), "order and repeats do not matter");
    assert_ne!(key(&[a, b, c]), key(&[a, b]));
    // A pair from a node to itself is nothing to route and is not part of the key.
    let same = RouteKey::new(node("o"), node("o"));
    assert_eq!(key(&[a, b]), key(&[a, same, b]));
}
