//! Route sets, checked against things that hold whatever the method: routes
//! are loop-free and connected and use only allowed turns; the first is a
//! shortest route (against an independent search); the others stay inside the
//! detour and overlap limits; and the result never depends on threads, key
//! order or what else was generated. Hand values come from a two-road diamond.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    reason = "test fixtures: small counts and indices"
)]

use std::collections::BinaryHeap;

use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
use openmobisim_core_graph::examples::{manhattan_grid, node_name, toy_network};
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::network::{LinkSpec, RoadNetwork, RoadNetworkBuilder};
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_routes::{
    DEFAULT_METHOD, NodeSnapper, Options, Penalty, Registry, RouteError, RouteKey, RouteSets,
    Shortest, generator,
};
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::{EntityId, LinkId, NodeId};

fn turns_of(net: &RoadNetwork) -> TurnTable {
    TurnTable::build(net, SignalDefaults::SHIPPED)
}

fn node(net: &RoadNetwork, name: &str) -> NodeId {
    net.node_external_ids().typed_id_of::<NodeId>(name).expect("a node")
}

/// A, B, C, D: the top road A-B-D is 200 m, the bottom road A-C-D is 240 m.
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

fn no_options() -> Options {
    Options::new()
}

fn opts(pairs: &[(&str, f64)]) -> Options {
    pairs.iter().map(|&(k, v)| (k.to_string(), v)).collect()
}

/// An independent node-based Dijkstra over motor links, for the reference cost.
fn reference_cost(net: &RoadNetwork, from: NodeId, to: NodeId) -> Option<f64> {
    let mut best = vec![f64::INFINITY; net.node_count() as usize];
    let mut heap = BinaryHeap::new();
    best[from.index()] = 0.0;
    heap.push(core::cmp::Reverse((0.0f64.to_bits(), from.raw())));
    while let Some(core::cmp::Reverse((bits, n))) = heap.pop() {
        let (d, n) = (f64::from_bits(bits), NodeId::new(n));
        if d > best[n.index()] {
            continue;
        }
        if n == to {
            return Some(d);
        }
        for &l in net.out_links(n) {
            if !net.link_class(l).carries_motor_traffic() {
                continue;
            }
            let (m, nd) = (net.link_to(l), d + net.free_flow_time(l).get());
            if nd < best[m.index()] {
                best[m.index()] = nd;
                heap.push(core::cmp::Reverse((nd.to_bits(), m.raw())));
            }
        }
    }
    None
}

#[test]
fn the_diamond_gives_both_roads_with_hand_costs() {
    let net = diamond();
    let turns = turns_of(&net);
    let (a, d) = (node(&net, "A"), node(&net, "D"));
    let sets = RouteSets::generate(
        &net,
        &turns,
        &[RouteKey::new(a, d)],
        generator(DEFAULT_METHOD, &no_options()).expect("default").as_ref(),
    );
    let routes: Vec<_> = sets.routes(0).collect();
    let v = net.free_flow_time(LinkId::new(0)).get() / 100.0; // seconds per metre
    assert_eq!(routes.len(), 2, "the top road and the bottom road");
    assert!((f64::from(routes[0].cost) - 200.0 * v).abs() < 1e-3, "top road: 200 m");
    assert!((f64::from(routes[1].cost) - 240.0 * v).abs() < 1e-3, "bottom road: 240 m");
    assert!(routes[1].overlap.abs() < 1e-6, "the roads share nothing");
    assert_eq!(routes[0].links.len(), 2);
}

#[test]
fn a_tight_detour_bound_leaves_only_the_best_road() {
    let net = diamond();
    let turns = turns_of(&net);
    let (a, d) = (node(&net, "A"), node(&net, "D"));
    let g = generator("penalty", &opts(&[("max_detour", 1.1)])).expect("ok");
    let sets = RouteSets::generate(&net, &turns, &[RouteKey::new(a, d)], g.as_ref());
    assert_eq!(sets.routes(0).count(), 1, "240 m is 1.2 times 200 m, over the bound");
}

#[test]
fn the_shortest_method_is_the_penalty_methods_first_route() {
    let (net, _) = manhattan_grid(7, 200.0, true);
    let turns = turns_of(&net);
    let keys = grid_keys(&net, 7, 120, 5);
    let shortest = RouteSets::generate(&net, &turns, &keys, &Shortest);
    let penalty = RouteSets::generate(&net, &turns, &keys, &Penalty::default());
    assert_eq!(shortest.keys(), penalty.keys());
    for i in 0..shortest.keys().len() {
        assert_eq!(shortest.routes(i).count(), 1);
        assert_eq!(
            shortest.routes(i).next().expect("one").links,
            penalty.routes(i).next().expect("some").links
        );
    }
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

/// **Properties of every route of every set** on a two-way signalised grid.
#[test]
fn every_route_is_loop_free_connected_and_within_the_limits() {
    let (net, _) = manhattan_grid(8, 200.0, true);
    let turns = turns_of(&net);
    let m = Penalty::default();
    let keys = grid_keys(&net, 8, 200, 11);
    let sets = RouteSets::generate(&net, &turns, &keys, &m);
    let mut with_alternatives = 0;
    for (i, key) in sets.keys().iter().enumerate() {
        let (origin, destination) = (NodeId::new(key.origin), NodeId::new(key.destination));
        let routes: Vec<_> = sets.routes(i).collect();
        assert!(!routes.is_empty() && routes.len() <= m.max_paths);
        let reference = reference_cost(&net, origin, destination).expect("a grid is connected");
        assert!(
            (f64::from(routes[0].cost) - reference).abs() < 1e-3 * reference.max(1.0),
            "the first route is a shortest one"
        );
        for w in routes.windows(2) {
            assert!(w[0].cost <= w[1].cost, "best first");
        }
        for r in &routes {
            let links: Vec<LinkId> = r.links.iter().map(|&l| LinkId::new(l)).collect();
            assert_eq!(net.link_from(links[0]), origin);
            assert_eq!(net.link_to(*links.last().expect("non-empty")), destination);
            let mut seen = vec![origin];
            for (k, &l) in links.iter().enumerate() {
                if k > 0 {
                    assert_eq!(net.link_from(l), net.link_to(links[k - 1]), "connected");
                    assert!(turns.find(links[k - 1], l).is_some(), "an allowed turn");
                }
                let to = net.link_to(l);
                assert!(!seen.contains(&to), "no node twice");
                seen.push(to);
            }
            let total: f64 = links.iter().map(|&l| net.free_flow_time(l).get()).sum();
            assert!(
                (f64::from(r.cost) - total).abs() < 1e-3 * total.max(1.0),
                "cost is the links' sum"
            );
            assert!(
                f64::from(r.cost) <= m.max_detour * f64::from(routes[0].cost) + 1e-3,
                "within the detour bound"
            );
            assert!(f64::from(r.overlap) <= m.max_overlap + 1e-6);
        }
        // Every pair: the later-found of the two shares at most max_overlap, so
        // the smaller of the two shares is within it, whichever was later.
        for a in 0..routes.len() {
            for b in (a + 1)..routes.len() {
                let (sa, sb) = (
                    shared_share(&net, routes[a].links, routes[b].links),
                    shared_share(&net, routes[b].links, routes[a].links),
                );
                assert!(sa.min(sb) <= m.max_overlap + 1e-6, "pair overlap {sa} / {sb}");
                assert_ne!(routes[a].links, routes[b].links, "no duplicates");
            }
        }
        with_alternatives += usize::from(routes.len() > 1);
    }
    assert!(with_alternatives > keys.len() / 2, "a grid offers alternatives for most pairs");
}

/// The share of `a`'s cost that lies on links also in `b`.
fn shared_share(net: &RoadNetwork, a: &[u32], b: &[u32]) -> f64 {
    let cost = |l: u32| net.free_flow_time(LinkId::new(l)).get();
    let total: f64 = a.iter().map(|&l| cost(l)).sum();
    a.iter().filter(|l| b.contains(l)).map(|&l| cost(l)).sum::<f64>() / total
}

#[test]
fn a_key_with_no_route_is_kept_as_an_empty_set() {
    let (net, _) = toy_network();
    let turns = turns_of(&net);
    let (w, d1) = (node(&net, "W"), node(&net, "D1"));
    let sets = RouteSets::generate(
        &net,
        &turns,
        &[RouteKey::new(w, d1), RouteKey::new(d1, w), RouteKey::new(w, w)],
        &Penalty::default(),
    );
    assert_eq!(sets.keys().len(), 2, "W to W is dropped, the rest kept");
    let back = sets.key_index(RouteKey::new(d1, w)).expect("kept");
    assert_eq!(sets.routes(back).count(), 0, "the streets are one-way: no way back");
    assert!(sets.best(RouteKey::new(d1, w)).is_none());
    assert!(sets.best(RouteKey::new(w, d1)).is_some());
}

/// **Property:** the store is a function of network, keys and method alone.
#[test]
fn generation_does_not_depend_on_threads_or_key_order() {
    let (net, _) = manhattan_grid(9, 150.0, true);
    let turns = turns_of(&net);
    let keys = grid_keys(&net, 9, 300, 3);
    let mut shuffled = keys.clone();
    shuffled.reverse();
    shuffled.rotate_left(37);
    let g = Penalty::default();
    let reference = RouteSets::generate(&net, &turns, &keys, &g);
    assert_eq!(reference, RouteSets::generate(&net, &turns, &shuffled, &g), "key order");
    assert_eq!(reference, RouteSets::generate(&net, &turns, &keys, &g), "repeat");
    #[cfg(feature = "parallel")]
    for threads in [1, 3, 8] {
        let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().expect("pool");
        let made = pool.install(|| RouteSets::generate(&net, &turns, &keys, &g));
        assert_eq!(reference, made, "{threads} threads");
    }
}

#[test]
fn the_inverted_index_finds_exactly_the_routes_that_use_a_link() {
    let (net, _) = manhattan_grid(6, 200.0, false);
    let turns = turns_of(&net);
    let sets = RouteSets::generate(&net, &turns, &grid_keys(&net, 6, 80, 9), &Penalty::default());
    let index = sets.link_index(net.link_count());
    for link in 0..net.link_count() {
        let expected: Vec<u32> = (0..sets.route_count())
            .filter(|&r| sets.route(r).links.contains(&link))
            .map(|r| r as u32)
            .collect();
        assert_eq!(index.routes_using(link), expected.as_slice(), "link {link}");
    }
    for r in 0..sets.route_count() {
        let key = sets.key_of_route(r);
        assert!(sets.route_range(key).contains(&r));
    }
}

#[test]
fn a_store_knows_what_made_it() {
    let (net5, _) = manhattan_grid(5, 200.0, false);
    let (net6, _) = manhattan_grid(6, 200.0, false);
    let (t5, t6) = (turns_of(&net5), turns_of(&net6));
    let key5 =
        |net: &RoadNetwork| RouteKey::new(node(net, &node_name(0, 0)), node(net, &node_name(4, 4)));
    let base = RouteSets::generate(&net5, &t5, &[key5(&net5)], &Penalty::default());
    let other_option = Penalty::from_options(&opts(&[("max_paths", 3.0)])).expect("ok");
    let changed = RouteSets::generate(&net5, &t5, &[key5(&net5)], &other_option);
    let other_net = RouteSets::generate(&net6, &t6, &[key5(&net6)], &Penalty::default());
    let other_method = RouteSets::generate(&net5, &t5, &[key5(&net5)], &Shortest);
    assert_eq!(
        base.identity(),
        RouteSets::generate(&net5, &t5, &[key5(&net5)], &Penalty::default()).identity()
    );
    for (label, s) in [("option", &changed), ("network", &other_net), ("method", &other_method)] {
        assert_ne!(base.identity(), s.identity(), "a different {label} is a different store");
    }
    assert!(base.matches(&net5) && !base.matches(&net6));
    assert_eq!((base.method(), other_method.method()), ("penalty", "shortest"));
}

#[test]
fn methods_are_selected_by_name_and_bad_choices_say_what_is_wrong() {
    let names = Registry::builtin().names().into_iter().map(str::to_string).collect::<Vec<_>>();
    assert_eq!(names, ["penalty", "shortest"]);
    assert_eq!(DEFAULT_METHOD, "penalty");
    match generator("teleport", &no_options()).err() {
        Some(RouteError::UnknownMethod { name, known }) => {
            assert!(name == "teleport" && known.len() == 2)
        }
        other => panic!("{other:?}"),
    }
    for (option, value) in [
        ("max_paths", 0.0),
        ("max_paths", 2.5),
        ("max_paths", 33.0),
        ("max_detour", 0.9),
        ("max_overlap", 0.0),
        ("max_overlap", 1.5),
        ("penalty", 1.0),
    ] {
        assert!(
            matches!(
                generator("penalty", &opts(&[(option, value)])).err(),
                Some(RouteError::BadOption { .. })
            ),
            "{option} = {value} must be refused"
        );
    }
    assert!(matches!(
        generator("penalty", &opts(&[("colour", 1.0)])).err(),
        Some(RouteError::UnknownOption { .. })
    ));
    assert!(matches!(
        generator("shortest", &opts(&[("max_paths", 1.0)])).err(),
        Some(RouteError::UnknownOption { .. })
    ));
    let message =
        generator("penalty", &opts(&[("max_paths", 0.0)])).err().expect("an error").to_string();
    assert!(message.contains("max_paths") && message.contains("penalty"));
}

/// **The extension point:** a method a researcher writes is used like a built-in one.
#[test]
fn a_new_method_plugs_in_without_changing_anything_else() {
    let mut registry = Registry::builtin();
    registry.register("my_method", |options| Ok(Box::new(Shortest::from_options(options)?)));
    assert_eq!(registry.names(), ["penalty", "shortest", "my_method"]);
    let net = diamond();
    let turns = turns_of(&net);
    let key = RouteKey::new(node(&net, "A"), node(&net, "D"));
    let g = registry.create("my_method", &no_options()).expect("registered");
    let sets = RouteSets::generate(&net, &turns, &[key], g.as_ref());
    assert_eq!(sets.routes(0).count(), 1);
}

#[test]
fn snapping_agrees_with_a_scan_of_every_node() {
    let (net, _) = manhattan_grid(12, 90.0, false);
    let snapper = NodeSnapper::new(&net);
    assert_eq!(snapper.len() as u32, net.node_count());
    let mut state = 17u64;
    let mut next = |lo: f64, hi: f64| -> f64 {
        state =
            state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        lo + (hi - lo) * ((state >> 11) as f64 / (1u64 << 53) as f64)
    };
    for _ in 0..400 {
        // Inside the grid and well outside it.
        let (x, y) = (next(-400.0, 1500.0), next(-400.0, 1500.0));
        let brute = (0..net.node_count())
            .map(NodeId::new)
            .min_by(|&a, &b| {
                let d = |n: NodeId| {
                    let p = net.node_position(n);
                    (p.x - x).hypot(p.y - y)
                };
                d(a).total_cmp(&d(b)).then(a.cmp(&b))
            })
            .expect("nodes");
        assert_eq!(snapper.nearest_xy(x, y), brute, "at ({x:.1}, {y:.1})");
    }
}

#[test]
fn keys_are_eight_bytes() {
    assert_eq!(size_of::<RouteKey>(), 8);
}

/// A loose floor, enforced in release only (see `core-types`' `tests/perf.rs`).
#[test]
fn generation_is_fast_enough() {
    if cfg!(debug_assertions) {
        return;
    }
    let (net, _) = manhattan_grid(60, 150.0, true);
    let turns = turns_of(&net);
    let keys = grid_keys(&net, 60, 1500, 8);
    let start = std::time::Instant::now();
    let sets = RouteSets::generate(&net, &turns, &keys, &Penalty::default());
    let elapsed = start.elapsed();
    println!(
        "{} keys, {} routes, {} KB in {elapsed:?}",
        sets.keys().len(),
        sets.route_count(),
        sets.bytes() / 1024
    );
    assert!(elapsed.as_secs_f64() < 20.0, "1 500 keys on a 3 600-node grid took {elapsed:?}");
}

/// **Property of the bounded search:** it never returns a route past its bound,
/// and finds the routes inside it, penalties or not.
#[test]
fn a_bounded_search_never_returns_a_route_past_its_bound() {
    use openmobisim_core_routes::{Search, SearchContext};
    let net = diamond();
    let turns = turns_of(&net);
    let ctx = SearchContext::new(&net, &turns);
    let mut search = Search::new(&ctx);
    let (a, d) = (node(&net, "A"), node(&net, "D"));
    let per_metre = net.free_flow_time(LinkId::new(0)).get() / 100.0;
    let (top, bottom) = (200.0 * per_metre, 240.0 * per_metre);

    assert!((search.shortest(a, d).expect("a route").cost - top).abs() < 1e-9);
    assert!(search.shortest_within(a, d, top - 0.01).is_none(), "nothing costs that little");
    assert!((search.shortest_within(a, d, top + 0.01).expect("top").cost - top).abs() < 1e-9);

    // Penalise the top road hard: the bottom road wins if the bound allows it, and
    // nothing is returned if it does not - never the over-bound top road's twin.
    // Link ids follow the sorted names: ab 0, ac 1, bd 2, cd 3. The top road is ab + bd.
    for link in [LinkId::new(0), LinkId::new(2)] {
        search.factors.multiply(link, 10.0);
    }
    let allowed = search.shortest_within(a, d, bottom + 0.01).expect("bottom");
    assert!(
        (allowed.cost - bottom).abs() < 1e-9,
        "the penalised top road loses to the bottom road"
    );
    let tight = search.shortest_within(a, d, top + 0.01).expect("top is still within the bound");
    assert!((tight.cost - top).abs() < 1e-9, "only the top road fits a tight bound");
    search.factors.reset();
    assert!(
        (search.shortest(a, d).expect("clean again").cost - top).abs() < 1e-9,
        "the search leaves nothing behind"
    );
}

// --- route attributes: length and path size (S169) -----------------------------------------

/// A -> S (100 m, shared), then S -> B -> D (100 + 100) or S -> C -> D (120 + 120).
fn shared_start() -> RoadNetwork {
    let mut b = RoadNetworkBuilder::new();
    for (n, x, y) in [
        ("A", -100.0, 0.0),
        ("S", 0.0, 0.0),
        ("B", 100.0, 60.0),
        ("C", 100.0, -60.0),
        ("D", 200.0, 0.0),
    ] {
        b.add_node(n, LonLat::new(4.8 + x / 77_800.0, 45.7 + y / 110_574.0));
    }
    for (name, from, to, len) in [
        ("as", "A", "S", 100.0),
        ("sb", "S", "B", 100.0),
        ("bd", "B", "D", 100.0),
        ("sc", "S", "C", 120.0),
        ("cd", "C", "D", 120.0),
    ] {
        let mut spec = LinkSpec::new(RoadClass::Residential);
        spec.length_m = Some(len);
        b.add_link(name, from, to, spec);
    }
    b.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
        .expect("buildable")
}

#[test]
fn routes_that_share_no_link_each_have_a_path_size_of_one() {
    let net = diamond();
    let (a, d) = (node(&net, "A"), node(&net, "D"));
    let sets = RouteSets::generate(
        &net,
        &turns_of(&net),
        &[RouteKey::new(a, d)],
        generator(DEFAULT_METHOD, &no_options()).expect("default").as_ref(),
    );
    let attributes = sets.attributes(&net);
    assert_eq!(attributes.length_m, [200.0, 240.0]);
    assert!(attributes.path_size.iter().all(|&p| (p - 1.0).abs() < 1e-12), "{attributes:?}");
}

#[test]
fn a_shared_first_link_lowers_both_path_sizes_by_hand() {
    let net = shared_start();
    let (a, d) = (node(&net, "A"), node(&net, "D"));
    let sets = RouteSets::generate(
        &net,
        &turns_of(&net),
        &[RouteKey::new(a, d)],
        generator(DEFAULT_METHOD, &no_options()).expect("default").as_ref(),
    );
    assert_eq!(sets.routes(0).count(), 2);
    let attributes = sets.attributes(&net);
    // Route 0: 300 m, of which the shared 100 m counts half: (50 + 100 + 100) / 300.
    // Route 1: 340 m: (50 + 120 + 120) / 340.
    assert_eq!(attributes.length_m, [300.0, 340.0]);
    assert!((attributes.path_size[0] - 250.0 / 300.0).abs() < 1e-12);
    assert!((attributes.path_size[1] - 290.0 / 340.0).abs() < 1e-12);
}

#[test]
fn one_keys_links_do_not_leak_into_the_next_keys_path_size() {
    let net = shared_start();
    let (a, s, d) = (node(&net, "A"), node(&net, "S"), node(&net, "D"));
    let keys = [RouteKey::new(a, d), RouteKey::new(s, d)];
    let sets = RouteSets::generate(
        &net,
        &turns_of(&net),
        &keys,
        generator(DEFAULT_METHOD, &no_options()).expect("default").as_ref(),
    );
    let attributes = sets.attributes(&net);
    let second = sets.route_range(sets.key_index(keys[1]).expect("key"));
    assert_eq!(second.len(), 2);
    for r in second {
        assert!(
            (attributes.path_size[r] - 1.0).abs() < 1e-12,
            "route {r} shares nothing in its own set"
        );
    }
    // A lone route has a path size of one, and the attributes are the same on a second call.
    assert_eq!(attributes, sets.attributes(&net));
}

// --- the fastest way when times depend on when a link is entered (S171) ------------------

#[test]
fn the_fastest_time_follows_the_times_at_the_moment_each_link_is_entered() {
    let net = diamond();
    let turns = turns_of(&net);
    let id = |n: &str| net.link_external_ids().typed_id_of::<LinkId>(n).expect("link").raw();
    let (ab, bd, ac, cd) = (id("ab"), id("bd"), id("ac"), id("cd"));
    let (a, d) = (node(&net, "A"), node(&net, "D"));
    let ctx = openmobisim_core_routes::SearchContext::new(&net, &turns);
    let mut search = openmobisim_core_routes::Search::new(&ctx);
    // The top road's first link takes 50 s if entered before 100 s and 500 s after; every other
    // link takes its length in seconds over 4 (100 m: 25 s, 120 m: 30 s).
    let seconds = |l: u32, at: f64| {
        if l == ab {
            if at < 100.0 { 50.0 } else { 500.0 }
        } else if l == bd {
            25.0
        } else if l == ac || l == cd {
            30.0
        } else {
            f64::INFINITY
        }
    };
    let no_wait = |_: u32, _: f64| 0.0;
    // Leaving at 0 s: the top road takes 50 + 25 = 75 s, the bottom 30 + 30 = 60 s: the bottom wins.
    let t = search.fastest_time(a, d, 0.0, f64::INFINITY, &no_wait, &seconds).expect("a route");
    assert!((t - 60.0).abs() < 1e-12, "{t}");
    // Make the bottom road slow too: the top is then 75 s. And leaving at 200 s, after the top's
    // first link has jammed (500 s), the bottom road's 60 s is the fastest again.
    let jam_bottom = |l: u32, at: f64| if l == ac { 400.0 } else { seconds(l, at) };
    let t = search.fastest_time(a, d, 0.0, f64::INFINITY, &no_wait, &jam_bottom).expect("a route");
    assert!((t - 75.0).abs() < 1e-12, "{t}");
    let t = search.fastest_time(a, d, 200.0, f64::INFINITY, &no_wait, &seconds).expect("a route");
    assert!((t - 60.0).abs() < 1e-12, "{t}");
    // Waiting outside the network before the first link counts: 10 s before `ab`, but the bottom's
    // first link is free to enter, so the bottom road still costs 60 s and the top 10 + 50 + 25.
    let wait_top = |l: u32, _: f64| if l == ab { 10.0 } else { 0.0 };
    let t = search.fastest_time(a, d, 0.0, f64::INFINITY, &wait_top, &jam_bottom).expect("a route");
    assert!((t - 85.0).abs() < 1e-12, "{t}");
    // A bound: nothing beats 50 s, so the search says so; a bound of 60 finds the bottom road.
    assert_eq!(search.fastest_time(a, d, 0.0, 50.0, &no_wait, &seconds), None);
    assert!(search.fastest_time(a, d, 0.0, 60.0, &no_wait, &seconds).is_some());
    // The same place is no time at all, and the search leaves itself clean for the next.
    assert_eq!(search.fastest_time(a, a, 0.0, 1.0, &no_wait, &seconds), Some(0.0));
    assert_eq!(search.shortest(a, d).map(|r| r.links.len()), Some(2));
}
