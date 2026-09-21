//! The Monte Carlo method and the congestion propensity it is biased by (S176).
//!
//! What is defended, and how each is checked:
//!
//! * **the noise goes only where the bias lets it**: on a two-road diamond, noise on the road
//!   that is already the shortest finds the other road, noise on the longer one never does, and
//!   a bias of nothing everywhere leaves the shortest route alone (hand-derived);
//! * **what it makes is a set of routes**: the first is a shortest one, the others are new,
//!   connected, loop-free, turn-respecting, inside the detour and overlap limits;
//! * **the sets are a function of the pair, the seed and the options alone**: any order of keys,
//!   any thread count; another seed is another set;
//! * **the propensity to congest** is worked out by hand: the busiest half-hour's persons on
//!   the shortest route over what the links carry, clipped at 1, and independent of the order
//!   of the trips.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "test fixtures: small counts and indices"
)]

use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
use openmobisim_core_graph::examples::{manhattan_grid, node_name};
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::network::{LinkSpec, RoadNetwork, RoadNetworkBuilder};
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_routes::{
    Demand, MonteCarlo, Options, PROPENSITY_BIN_SECONDS, Penalty, RouteError, RouteKey,
    RouteSetGenerator, RouteSets, Shortest, TripDemand, congestion_propensity, generator,
};
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::{EntityId, LinkId, NodeId};

fn turns_of(net: &RoadNetwork) -> TurnTable {
    TurnTable::build(net, SignalDefaults::SHIPPED)
}

fn node(net: &RoadNetwork, name: &str) -> NodeId {
    net.node_external_ids().typed_id_of::<NodeId>(name).expect("a node")
}

fn link(net: &RoadNetwork, name: &str) -> LinkId {
    net.link_external_ids().typed_id_of::<LinkId>(name).expect("a link")
}

fn opts(pairs: &[(&str, f64)]) -> Options {
    pairs.iter().map(|&(k, v)| (k.to_string(), v)).collect()
}

/// A, B, C, D: the top road A-B-D is 200 m, the bottom road A-C-D is 240 m; one-way, so D has no
/// way back.
fn diamond() -> RoadNetwork {
    let mut b = RoadNetworkBuilder::new();
    for (n, x, y) in [("A", 0.0, 0.0), ("B", 100.0, 60.0), ("C", 100.0, -60.0), ("D", 200.0, 0.0)] {
        b.add_node(n, LonLat::new(4.8 + x / 77_800.0, 45.7 + y / 110_574.0));
    }
    for (name, from, to, len) in [
        ("ab", "A", "B", 100.0),
        ("bd", "B", "D", 100.0),
        ("ac", "A", "C", 120.0),
        ("cd", "C", "D", 120.0),
    ] {
        let mut spec = LinkSpec::new(RoadClass::Residential);
        spec.length_m = Some(len);
        b.add_link(name, from, to, spec);
    }
    b.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
        .expect("buildable")
}

fn grid_keys(net: &RoadNetwork, n: u32, count: usize, seed: u64) -> Vec<RouteKey> {
    let mut state = seed;
    let mut next = |bound: u32| -> u32 {
        state =
            state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        ((state >> 33) % u64::from(bound)) as u32
    };
    let mut keys = Vec::new();
    while keys.len() < count {
        let (a, b) = ((next(n), next(n)), (next(n), next(n)));
        if a != b {
            keys.push(RouteKey::new(
                node(net, &node_name(a.0, a.1)),
                node(net, &node_name(b.0, b.1)),
            ));
        }
    }
    keys
}

/// A bias of `value` on the named links and 0 on every other.
fn bias_on(net: &RoadNetwork, names: &[&str], value: f32) -> Vec<f32> {
    let mut bias = vec![0.0; net.link_count() as usize];
    for n in names {
        bias[link(net, n).index()] = value;
    }
    bias
}

// --- the noise goes only where the bias lets it -----------------------------------------------

#[test]
fn noise_on_the_shortest_road_finds_the_other_and_noise_on_the_other_never_does() {
    let net = diamond();
    let turns = turns_of(&net);
    let key = RouteKey::new(node(&net, "A"), node(&net, "D"));
    let sets = |bias: Vec<f32>, options: &[(&str, f64)]| {
        let mc = MonteCarlo::from_options(&opts(options)).expect("valid").with_bias(bias);
        RouteSets::generate(&net, &turns, &[key], &mc)
    };
    // The top road (200 m) is the shortest. Noise on its links raises its cost by up to
    // (1 + 4u) each, so on most draws the bottom road (240 m) is cheaper: both roads are found.
    let top = sets(bias_on(&net, &["ab", "bd"], 1.0), &[("sigma", 4.0)]);
    let roads: Vec<Vec<u32>> = top.routes(0).map(|r| r.links.to_vec()).collect();
    let id = |n: &str| link(&net, n).raw();
    assert_eq!(roads, vec![vec![id("ab"), id("bd")], vec![id("ac"), id("cd")]], "best first");
    // Noise on the bottom road only can only make it dearer: the top stays the cheapest, and it
    // is the only route found, on any draw.
    let bottom = sets(bias_on(&net, &["ac", "cd"], 1.0), &[("sigma", 4.0), ("draws", 200.0)]);
    assert_eq!(bottom.routes(0).count(), 1);
    // Nothing is perturbed where the bias is nothing: no bias anywhere, no alternative.
    let none = sets(vec![0.0; net.link_count() as usize], &[("draws", 200.0)]);
    assert_eq!(none.routes(0).count(), 1);
    // The bottom road is 1.2 times the top: over a detour bound of 1.1 it is not kept.
    let tight = sets(bias_on(&net, &["ab", "bd"], 1.0), &[("max_detour", 1.1)]);
    assert_eq!(tight.routes(0).count(), 1);
    // And zero draws is the shortest route, as the method's first step says.
    let zero = sets(bias_on(&net, &["ab", "bd"], 1.0), &[("draws", 0.0)]);
    assert_eq!(zero.routes(0).count(), 1);
    // The road it made is a real one: its cost is the links' free-flow times, unperturbed.
    let v = net.free_flow_time(link(&net, "ab")).get() / 100.0;
    assert!((f64::from(top.route(0).cost) - 200.0 * v).abs() < 1e-3);
    assert!((f64::from(top.route(1).cost) - 240.0 * v).abs() < 1e-3);
    assert!(top.route(1).overlap.abs() < 1e-6, "the roads share no link");
}

#[test]
fn a_bias_of_nothing_leaves_only_the_shortest_route_everywhere() {
    let (net, _) = manhattan_grid(7, 200.0, true);
    let turns = turns_of(&net);
    let keys = grid_keys(&net, 7, 120, 4);
    let mc = MonteCarlo::default().with_bias(vec![0.0; net.link_count() as usize]);
    let sets = RouteSets::generate(&net, &turns, &keys, &mc);
    let shortest = RouteSets::generate(&net, &turns, &keys, &Shortest);
    for i in 0..sets.keys().len() {
        assert_eq!(sets.routes(i).count(), 1);
        assert_eq!(sets.routes(i).next().unwrap().links, shortest.routes(i).next().unwrap().links);
    }
}

// --- what it makes is a set of routes ----------------------------------------------------------

#[test]
fn every_route_is_loop_free_connected_and_within_the_limits() {
    let (net, _) = manhattan_grid(8, 200.0, true);
    let turns = turns_of(&net);
    let keys = grid_keys(&net, 8, 200, 11);
    // Two biases: everywhere (the unbiased method) and on every other link.
    let half: Vec<f32> = (0..net.link_count()).map(|l| f32::from(l % 2 == 0)).collect();
    for (label, mc) in [
        ("unbiased", MonteCarlo::from_options(&opts(&[("biased", 0.0)])).unwrap()),
        ("half the links", MonteCarlo::default().with_bias(half)),
    ] {
        let sets = RouteSets::generate(&net, &turns, &keys, &mc);
        let shortest = RouteSets::generate(&net, &turns, &keys, &Shortest);
        let mut with_alternatives = 0;
        for (i, key) in sets.keys().iter().enumerate() {
            let (origin, destination) = (NodeId::new(key.origin), NodeId::new(key.destination));
            let routes: Vec<_> = sets.routes(i).collect();
            assert!(!routes.is_empty() && routes.len() <= mc.max_paths, "{label}");
            // The first is a shortest route (the same cost as the shortest method's).
            let best = f64::from(shortest.routes(i).next().unwrap().cost);
            assert!((f64::from(routes[0].cost) - best).abs() < 1e-3 * best.max(1.0), "{label}");
            for w in routes.windows(2) {
                assert!(w[0].cost <= w[1].cost, "best first");
            }
            for (k, r) in routes.iter().enumerate() {
                let links: Vec<LinkId> = r.links.iter().map(|&l| LinkId::new(l)).collect();
                assert_eq!(net.link_from(links[0]), origin);
                assert_eq!(net.link_to(*links.last().unwrap()), destination);
                let mut seen = vec![origin];
                for (j, &l) in links.iter().enumerate() {
                    if j > 0 {
                        assert_eq!(net.link_from(l), net.link_to(links[j - 1]), "connected");
                        assert!(turns.find(links[j - 1], l).is_some(), "an allowed turn");
                    }
                    assert!(!seen.contains(&net.link_to(l)), "no node twice");
                    seen.push(net.link_to(l));
                }
                let total: f64 = links.iter().map(|&l| net.free_flow_time(l).get()).sum();
                assert!((f64::from(r.cost) - total).abs() < 1e-3 * total.max(1.0), "true cost");
                assert!(f64::from(r.cost) <= mc.max_detour * best + 1e-3, "detour bound");
                assert!(f64::from(r.overlap) <= mc.max_overlap + 1e-6, "overlap limit");
                assert!(routes[..k].iter().all(|o| o.links != r.links), "distinct");
            }
            if routes.len() > 1 {
                with_alternatives += 1;
            }
        }
        assert!(
            with_alternatives > keys.len() / 3,
            "{label}: {with_alternatives} sets with alternatives"
        );
    }
}

// --- determinism -------------------------------------------------------------------------------------

#[test]
fn the_sets_are_a_function_of_the_pair_the_seed_and_the_options_alone() {
    let (net, _) = manhattan_grid(9, 150.0, true);
    let turns = turns_of(&net);
    let keys = grid_keys(&net, 9, 300, 3);
    let mut shuffled = keys.clone();
    shuffled.reverse();
    shuffled.rotate_left(37);
    let bias: Vec<f32> = (0..net.link_count()).map(|l| (l % 5) as f32 / 4.0).collect();
    let mc = MonteCarlo::default().with_bias(bias.clone());
    let reference = RouteSets::generate(&net, &turns, &keys, &mc);
    assert_eq!(reference, RouteSets::generate(&net, &turns, &shuffled, &mc), "key order");
    assert_eq!(reference, RouteSets::generate(&net, &turns, &keys, &mc), "repeat");
    // A key's routes do not depend on which other keys are generated.
    let alone = RouteSets::generate(&net, &turns, &keys[..1], &mc);
    let first = reference.key_index(keys[0]).unwrap();
    assert_eq!(
        alone.routes(0).map(|r| r.links.to_vec()).collect::<Vec<_>>(),
        reference.routes(first).map(|r| r.links.to_vec()).collect::<Vec<_>>()
    );
    #[cfg(feature = "parallel")]
    for threads in [1, 3, 8] {
        let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().expect("pool");
        assert_eq!(
            reference,
            pool.install(|| RouteSets::generate(&net, &turns, &keys, &mc)),
            "{threads} threads"
        );
    }
    // The seed decides the draws: the same seed is the same sets, another is other ones.
    let seeded = |seed: f64| {
        let mc =
            MonteCarlo::from_options(&opts(&[("seed", seed)])).unwrap().with_bias(bias.clone());
        RouteSets::generate(&net, &turns, &keys, &mc)
    };
    assert_eq!(seeded(7.0), seeded(7.0));
    // (The stores differ in their descriptors anyway: compare the routes themselves.)
    assert_ne!(seeded(7.0).links(), seeded(8.0).links(), "another seed, other draws");
    assert_eq!(seeded(0.0), reference, "0 is the default");
    // The seed is part of the store's identity.
    assert_ne!(seeded(7.0).identity(), seeded(8.0).identity());
}

// --- the propensity to congest -------------------------------------------------------------------------

fn trip(net: &RoadNetwork, from: &str, to: &str, weight: u32, departure: u32) -> TripDemand {
    TripDemand { key: RouteKey::new(node(net, from), node(net, to)), weight, departure }
}

#[test]
fn the_propensity_is_the_busiest_half_hour_of_the_shortest_route_over_what_it_carries() {
    let net = diamond();
    let turns = turns_of(&net);
    let capacity = |name: &str| net.link_parameters(link(&net, name)).capacity.get();
    let bin = f64::from(PROPENSITY_BIN_SECONDS);
    let propensity = |trips: &[TripDemand]| {
        congestion_propensity(&Demand { network: &net, turns: &turns, trips })
    };
    let at = |p: &[f32], name: &str| f64::from(p[link(&net, name).index()]);

    // 5 + 7 = 12 persons leave in the first half hour, 20 in the second: the busier bin is 20.
    let trips = [
        trip(&net, "A", "D", 5, 100),
        trip(&net, "A", "D", 7, PROPENSITY_BIN_SECONDS - 1),
        trip(&net, "A", "D", 20, PROPENSITY_BIN_SECONDS),
        // Nothing to route, and a pair with no way (the streets are one-way): they count for nothing.
        trip(&net, "A", "A", 1_000, 0),
        trip(&net, "D", "A", 1_000, 0),
    ];
    let p = propensity(&trips);
    assert_eq!(p.len(), net.link_count() as usize);
    for road in ["ab", "bd"] {
        let expected = 20.0 / (bin * capacity(road));
        assert!(expected < 1.0, "the hand value is not clipped: {expected}");
        assert!(
            (at(&p, road) - expected).abs() < 1e-6 * expected.max(1.0),
            "{road}: {}",
            at(&p, road)
        );
    }
    // The bottom road is nobody's shortest route.
    assert_eq!((at(&p, "ac"), at(&p, "cd")), (0.0, 0.0));
    // The order the trips come in changes nothing.
    let mut reversed = trips;
    reversed.reverse();
    assert_eq!(p, propensity(&reversed));
    // A load of capacity or beyond is 1, and a link cannot be more likely than certain.
    let heavy = propensity(&[trip(&net, "A", "D", 1_000_000, 0)]);
    assert_eq!((at(&heavy, "ab"), at(&heavy, "bd")), (1.0, 1.0));
    // No demand: nothing is likely to congest.
    assert!(propensity(&[]).iter().all(|&b| b == 0.0));
}

#[test]
fn the_propensity_is_the_same_for_any_thread_count() {
    let (net, _) = manhattan_grid(9, 150.0, true);
    let turns = turns_of(&net);
    let keys = grid_keys(&net, 9, 400, 21);
    let trips: Vec<TripDemand> = keys
        .iter()
        .enumerate()
        .map(|(i, &key)| TripDemand {
            key,
            weight: 1 + (i % 3) as u32,
            departure: (i * 37 % 7200) as u32,
        })
        .collect();
    let demand = Demand { network: &net, turns: &turns, trips: &trips };
    let reference = congestion_propensity(&demand);
    assert!(
        reference.iter().any(|&b| b > 0.0) && reference.iter().all(|&b| (0.0..=1.0).contains(&b))
    );
    #[cfg(feature = "parallel")]
    for threads in [1, 4] {
        let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().expect("pool");
        assert_eq!(reference, pool.install(|| congestion_propensity(&demand)), "{threads} threads");
    }
}

// --- reading the demand, options, the registry ------------------------------------------------------------

#[test]
fn a_method_that_reads_the_demand_is_given_it_once_and_says_so_in_its_descriptor() {
    let net = diamond();
    let turns = turns_of(&net);
    let trips = [trip(&net, "A", "D", 900, 0)];
    let demand = Demand { network: &net, turns: &turns, trips: &trips };
    let mc = MonteCarlo::default();
    assert!(mc.reads_demand() && mc.bias().is_none());
    let prepared = mc.with_demand(&demand).expect("it reads the demand");
    assert_eq!(
        prepared.descriptor(),
        mc.descriptor(),
        "the run's description is the same either way"
    );
    assert!(mc.descriptor().starts_with("montecarlo;bias=demand;draws=16;"), "{}", mc.descriptor());
    assert!(!prepared.reads_demand(), "once is enough");
    assert!(prepared.with_demand(&demand).is_none());
    // The prepared method's sets are biased; the unprepared one's are not (bias everywhere).
    let key = RouteKey::new(node(&net, "A"), node(&net, "D"));
    let made = RouteSets::generate(&net, &turns, &[key], prepared.as_ref());
    assert_eq!(made.method(), "montecarlo");
    // Not reading the demand: nothing to be told, and nothing done.
    for m in [Box::new(Penalty::default()) as Box<dyn RouteSetGenerator>, Box::new(Shortest)] {
        assert!(!m.reads_demand() && m.with_demand(&demand).is_none());
    }
    let unbiased = MonteCarlo::from_options(&opts(&[("biased", 0.0)])).unwrap();
    assert!(!unbiased.reads_demand() && unbiased.descriptor().contains("bias=none"));
    let given = MonteCarlo::default().with_bias(vec![0.5; net.link_count() as usize]);
    assert!(!given.reads_demand() && given.descriptor().contains("bias=given"));
    assert_eq!(given.bias().map(<[f32]>::len), Some(net.link_count() as usize));
}

#[test]
fn the_method_is_chosen_by_name_and_bad_options_say_what_is_wrong() {
    let g = generator("montecarlo", &Options::new()).expect("built in");
    assert_eq!(g.name(), "montecarlo");
    assert_eq!(
        g.descriptor(),
        "montecarlo;bias=demand;draws=16;max_detour=2;max_overlap=0.9;max_paths=10;seed=0;sigma=4"
    );
    match generator("montecarlo", &opts(&[("nope", 1.0)])).err() {
        Some(RouteError::UnknownOption { known, .. }) => {
            assert_eq!(
                known,
                ["biased", "draws", "max_detour", "max_overlap", "max_paths", "seed", "sigma"]
            );
        }
        other => panic!("{other:?}"),
    }
    for (option, value) in [
        ("draws", -1.0),
        ("draws", 1.5),
        ("draws", 1001.0),
        ("max_paths", 0.0),
        ("max_paths", 33.0),
        ("max_detour", 0.9),
        ("max_overlap", 0.0),
        ("max_overlap", 1.1),
        ("sigma", 0.0),
        ("sigma", f64::NAN),
        ("seed", -1.0),
        ("seed", 0.5),
        ("biased", 2.0),
    ] {
        match generator("montecarlo", &opts(&[(option, value)])).err() {
            Some(RouteError::BadOption { option: named, .. }) => {
                assert_eq!(named, option, "{value}")
            }
            other => panic!("{option}={value}: {other:?}"),
        }
    }
    let ok =
        generator("montecarlo", &opts(&[("draws", 1000.0), ("max_paths", 32.0), ("seed", 42.0)]))
            .unwrap();
    assert!(ok.descriptor().contains("draws=1000") && ok.descriptor().contains("seed=42"));
}

/// A loose floor, enforced in release only (see `core-types`' `tests/perf.rs`).
#[test]
fn monte_carlo_generation_is_fast_enough() {
    if cfg!(debug_assertions) {
        return;
    }
    let (net, _) = manhattan_grid(60, 150.0, true);
    let turns = turns_of(&net);
    let keys = grid_keys(&net, 60, 1500, 8);
    let trips: Vec<TripDemand> =
        keys.iter().map(|&key| TripDemand { key, weight: 1, departure: 0 }).collect();
    let mc = MonteCarlo::default();
    let start = std::time::Instant::now();
    let prepared = mc.with_demand(&Demand { network: &net, turns: &turns, trips: &trips }).unwrap();
    let sets = RouteSets::generate(&net, &turns, &keys, prepared.as_ref());
    let elapsed = start.elapsed();
    println!("{} keys, {} routes in {elapsed:?}", sets.keys().len(), sets.route_count());
    assert!(elapsed.as_secs_f64() < 30.0, "1 500 keys on a 3 600-node grid took {elapsed:?}");
}
