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
    Search, SearchContext, Shortest, TripSnapper, generator,
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

/// **A search tree** finds, from one origin, a least-cost route to every destination at
/// once, connected and on allowed turns, the same cost as a search per pair; a bound leaves out
/// exactly the destinations beyond it; and `Shortest` generates by trees the same costs.
#[test]
fn a_search_tree_finds_a_least_cost_route_to_every_destination() {
    let (net, _) = manhattan_grid(8, 200.0, true);
    let turns = turns_of(&net);
    let ctx = SearchContext::new(&net, &turns);
    let mut search = Search::new(&ctx);
    let origin = node(&net, &node_name(2, 3));
    let destinations: Vec<NodeId> =
        (0..net.node_count()).map(NodeId::new).filter(|&d| d != origin).collect();
    let routes = search.shortest_tree(origin, &destinations, f64::INFINITY);
    assert_eq!(routes.len(), destinations.len());
    let mut costs = Vec::new();
    for (&d, route) in destinations.iter().zip(&routes) {
        let route = route.as_ref().expect("a grid is connected");
        let reference = reference_cost(&net, origin, d).expect("connected");
        assert!((route.cost - reference).abs() < 1e-3 * reference.max(1.0), "a least-cost route");
        let one = search.shortest(origin, d).expect("connected");
        assert!((route.cost - one.cost).abs() < 1e-9, "the same cost as a search for the pair");
        assert_eq!(net.link_from(route.links[0]), origin);
        assert_eq!(net.link_to(*route.links.last().expect("non-empty")), d);
        for w in route.links.windows(2) {
            assert!(turns.find(w[0], w[1]).is_some(), "an allowed turn");
        }
        costs.push(route.cost);
    }
    // Bounded at the median cost: exactly the destinations within it have a route.
    let mut sorted = costs.clone();
    sorted.sort_by(f64::total_cmp);
    let bound = sorted[sorted.len() / 2];
    let bounded = search.shortest_tree(origin, &destinations, bound);
    for (route, &cost) in bounded.iter().zip(&costs) {
        assert_eq!(route.is_some(), cost <= bound + 1e-9, "within the bound, and only then");
    }
    // Generated by trees, the shortest method's routes cost what a search per pair finds.
    let keys = grid_keys(&net, 8, 200, 3);
    let sets = RouteSets::generate(&net, &turns, &keys, &Shortest);
    for (i, key) in sets.keys().iter().enumerate() {
        let route = sets.routes(i).next().expect("connected");
        let one = search
            .shortest(NodeId::new(key.origin), NodeId::new(key.destination))
            .expect("connected");
        assert!((f64::from(route.cost) - one.cost).abs() < 1e-3, "the same least cost");
    }
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

/// `n` independent copies of a diamond whose two roads are **exactly** the same length (200 m
/// each way, unlike `diamond()`'s 200-vs-240), so their free-flow costs tie exactly rather than
/// approximately — the existing diamond never exercises the search's tie-break at all.
fn tied_diamonds(n: u32) -> (RoadNetwork, Vec<RouteKey>) {
    let mut b = RoadNetworkBuilder::new();
    for i in 0..n {
        let x0 = f64::from(i) * 1000.0;
        for (n, dx, dy) in
            [("a", 0.0, 0.0), ("b", 100.0, 60.0), ("c", 100.0, -60.0), ("d", 200.0, 0.0)]
        {
            b.add_node(
                format!("{n}{i}"),
                LonLat::new(4.8 + (x0 + dx) / 77_800.0, 45.7 + dy / 110_574.0),
            );
        }
        for (name, from, to) in
            [("ab", "a", "b"), ("bd", "b", "d"), ("ac", "a", "c"), ("cd", "c", "d")]
        {
            let mut spec = LinkSpec::new(RoadClass::Residential);
            spec.length_m = Some(100.0);
            b.add_link(format!("{name}{i}"), format!("{from}{i}"), format!("{to}{i}"), spec);
        }
    }
    let net = b
        .build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
        .expect("buildable");
    let keys = (0..n)
        .map(|i| RouteKey::new(node(&net, &format!("a{i}")), node(&net, &format!("d{i}"))))
        .collect();
    (net, keys)
}

#[test]
fn an_exact_cost_tie_is_broken_the_same_way_regardless_of_threads_or_key_order() {
    // `search.rs`'s `Entry` orders by link id once costs tie — an adversarial review
    // found no test exercising an exact tie at all (`diamond()`'s two roads, 200 m and 240 m,
    // never actually tie), and confirmed this one: 30 independent, geometrically identical tied
    // diamonds give the parallel driver real work to spread across keys and threads, and every
    // single search must still resolve its own tie the same way every time. Note (checked while
    // adding this): removing the explicit `.then_with(link cmp)` tie-break did **not** make this
    // test fail — `BinaryHeap`'s pop order is itself a deterministic function of push order on
    // this target, so today's stability comes from links being visited in the same fixed order
    // every time, not from the tie-break specifically. The tie-break's doc comment reads as a
    // guarantee against that assumption changing later (a different visitation order, a
    // different heap), which this test cannot isolate from outside — recorded here rather than
    // claimed as proven.
    let (net, keys) = tied_diamonds(30);
    let turns = turns_of(&net);
    let g = Shortest;
    let reference = RouteSets::generate(&net, &turns, &keys, &g);
    // Every diamond is geometrically identical (only translated), so whichever road the
    // tie-break favours, it must favour the *same one* — by link id, not by chance — in every
    // one of the 30 copies. "Which road" is read off the external id's own name (`ab`/`bd` for
    // the top road, `ac`/`cd` for the bottom), not the internal id, since ids are assigned by
    // sorted external id across the *whole* network, not per diamond.
    // links[0] is always "ab<i>" (top road) or "ac<i>" (bottom road): its second character.
    let road_of = |link: LinkId| -> char {
        net.link_external_ids().external_of(link).expect("named")[1..2].chars().next().unwrap()
    };
    let first_choice = road_of(LinkId::new(reference.routes(0).next().expect("a route").links[0]));
    for i in 0..keys.len() {
        let chosen = road_of(LinkId::new(reference.routes(i).next().expect("a route").links[0]));
        assert_eq!(chosen, first_choice, "diamond {i} broke its tie differently from diamond 0");
    }
    let mut shuffled = keys.clone();
    shuffled.reverse();
    shuffled.rotate_left(7);
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
    assert_eq!(names, ["penalty", "shortest", "montecarlo", "sampled"]);
    assert_eq!(DEFAULT_METHOD, "penalty");
    match generator("teleport", &no_options()).err() {
        Some(RouteError::UnknownMethod { name, known }) => {
            assert!(name == "teleport" && known.len() == 4)
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
    assert_eq!(registry.names(), ["penalty", "shortest", "montecarlo", "sampled", "my_method"]);
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

/// Three small, tight clusters of nodes far apart, with wide empty space between them — unlike
/// `manhattan_grid`'s uniform density, most grid cells here hold nothing at all, so a query in
/// the empty space genuinely exercises how many *empty* rings the search must cross before
/// finding a cluster (an adversarial review: the ring early-stop bound, off by one
/// cell either way, was not caught by `snapping_agrees_with_a_scan_of_every_node` above, whose
/// uniform grid structurally can't expose it).
fn clustered_network() -> RoadNetwork {
    let mut b = RoadNetworkBuilder::new();
    for (cluster, (cx, cy)) in [(0, (0.0, 0.0)), (1, (3000.0, 100.0)), (2, (1200.0, 2800.0))] {
        for i in 0..4 {
            #[allow(clippy::cast_precision_loss, reason = "a handful of nodes per cluster")]
            let offset = f64::from(i) * 8.0;
            b.add_node(
                format!("c{cluster}n{i}"),
                LonLat::new(4.8 + (cx + offset) / 77_800.0, 45.7 + cy / 110_574.0),
            );
        }
        for i in 0..3 {
            let mut spec = LinkSpec::new(RoadClass::Residential);
            spec.length_m = Some(8.0);
            b.add_link(
                format!("c{cluster}l{i}"),
                format!("c{cluster}n{i}"),
                format!("c{cluster}n{}", i + 1),
                spec,
            );
        }
    }
    b.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
        .expect("buildable")
}

#[test]
fn snapping_agrees_with_a_scan_across_wide_empty_gaps_between_clusters() {
    let net = clustered_network();
    let snapper = NodeSnapper::new(&net);
    assert_eq!(snapper.len() as u32, net.node_count());
    let mut state = 29u64;
    let mut next = |lo: f64, hi: f64| -> f64 {
        state =
            state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        lo + (hi - lo) * ((state >> 11) as f64 / (1u64 << 53) as f64)
    };
    for _ in 0..1_000 {
        // The whole bounding box, dominated by the empty space between clusters.
        let (x, y) = (next(-200.0, 3200.0), next(-200.0, 3000.0));
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

/// A motorway M0–M1–M2 running east along y = 0 (M1 is reached by a ramp only), a street S0–S1
/// 120 m north of it, parallel, and a ramp from S1 down to M1. M0 and M2 have only the motorway;
/// M1 has the motorway and a ramp; S0 and S1 have the street.
fn motorway_beside_a_street() -> RoadNetwork {
    let mut b = RoadNetworkBuilder::new();
    for (n, x, y) in [
        ("M0", 0.0, 0.0),
        ("M1", 500.0, 0.0),
        ("M2", 1000.0, 0.0),
        ("S0", 0.0, 120.0),
        ("S1", 500.0, 120.0),
    ] {
        b.add_node(n, LonLat::new(4.8 + x / 77_800.0, 45.7 + y / 110_574.0));
    }
    for (name, from, to, class, len) in [
        ("m01", "M0", "M1", RoadClass::Motorway, 500.0),
        ("m12", "M1", "M2", RoadClass::Motorway, 500.0),
        ("s01", "S0", "S1", RoadClass::Residential, 500.0),
        ("ramp", "S1", "M1", RoadClass::MotorwayLink, 130.0),
    ] {
        let mut spec = LinkSpec::new(class);
        spec.length_m = Some(len);
        b.add_link(name, from, to, spec);
    }
    b.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
        .expect("buildable")
}

#[test]
fn a_trip_end_joins_a_street_unless_it_lies_on_the_motorway() {
    let net = motorway_beside_a_street();
    let at = |x: f64, y: f64| LonLat::new(4.8 + x / 77_800.0, 45.7 + y / 110_574.0);
    let plain = NodeSnapper::new(&net);
    let trips = TripSnapper::new(&net, 50.0);
    // A point 30 m from the motorway's end and 90 m from the street: the plain snapper takes
    // the motorway node, the trip snapper the street (the point is farther than 50 m from M0
    // once it is more than 50 m away; here it is within, so it stays).
    assert_eq!(plain.nearest(&net, at(0.0, 30.0)), node(&net, "M0"));
    assert_eq!(trips.nearest(&net, at(0.0, 30.0)), node(&net, "M0"), "within 50 m: kept");
    assert_eq!(plain.nearest(&net, at(0.0, 55.0)), node(&net, "M0"));
    assert_eq!(trips.nearest(&net, at(0.0, 55.0)), node(&net, "S0"), "beyond 50 m: a street");
    // On the node itself (a gateway row): kept.
    assert_eq!(trips.nearest(&net, at(1000.0, 0.0)), node(&net, "M2"));
    // M1 carries a ramp but no street: motorway-only, so a point beside it goes to S1.
    assert!(!trips.on_street(node(&net, "M1")));
    assert_eq!(trips.nearest(&net, at(500.0, 60.0)), node(&net, "S1"));
    // A street node is always itself; 0 sends every trip to a street; infinity is the plain rule.
    assert_eq!(trips.nearest(&net, at(0.0, 125.0)), node(&net, "S0"));
    assert_eq!(TripSnapper::new(&net, 0.0).nearest(&net, at(1000.0, 0.0)), node(&net, "S1"));
    assert_eq!(
        TripSnapper::new(&net, f64::INFINITY).nearest(&net, at(0.0, 55.0)),
        node(&net, "M0")
    );
}

#[test]
fn a_network_of_motorways_only_snaps_to_them() {
    let mut b = RoadNetworkBuilder::new();
    b.add_node("A", LonLat::new(4.8, 45.7));
    b.add_node("B", LonLat::new(4.8 + 500.0 / 77_800.0, 45.7));
    let mut spec = LinkSpec::new(RoadClass::Motorway);
    spec.length_m = Some(500.0);
    b.add_link("ab", "A", "B", spec);
    let net = b
        .build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
        .expect("buildable");
    let trips = TripSnapper::new(&net, 50.0);
    assert_eq!(trips.nearest(&net, LonLat::new(4.8, 45.7 + 300.0 / 110_574.0)), node(&net, "A"));
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

#[test]
fn path_sizes_over_given_routes_equal_the_stores_over_the_whole_set_and_ignore_the_rest() {
    let net = shared_start();
    let (a, d) = (node(&net, "A"), node(&net, "D"));
    let sets = RouteSets::generate(
        &net,
        &turns_of(&net),
        &[RouteKey::new(a, d)],
        generator(DEFAULT_METHOD, &no_options()).expect("default").as_ref(),
    );
    let length = |l: u32| net.link_length(LinkId::new(l)).get();
    let both: Vec<&[u32]> = sets.routes(0).map(|r| r.links).collect();
    let stored = sets.attributes(&net).path_size;
    let over_both = openmobisim_core_routes::path_sizes(&both, length);
    assert_eq!(over_both.len(), 2);
    for (got, want) in over_both.iter().zip(&stored) {
        assert_eq!(got.to_bits(), want.to_bits(), "{over_both:?} against {stored:?}");
    }
    // One route alone shares with nobody; a repeated route splits with its twin by half.
    let alone = openmobisim_core_routes::path_sizes(&both[..1], length);
    assert_eq!(alone, [1.0]);
    let twins = openmobisim_core_routes::path_sizes(&[both[0], both[0]], length);
    assert_eq!(twins, [0.5, 0.5]);
    assert!(openmobisim_core_routes::path_sizes(&[], length).is_empty());
}

/// The counted path sizes are the sorted ones to the last bit, on sets with shared links, links
/// a route uses twice and links of no length, and leave their scratch all zeros (S294).
#[test]
fn counted_path_sizes_equal_the_sorted_ones_to_the_bit() {
    let mut state = 5_u64;
    let mut next = || {
        state =
            state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        state >> 33
    };
    #[allow(clippy::cast_precision_loss, reason = "small numbers")]
    let length = |l: u32| if l % 7 == 0 { 0.0 } else { 3.0 + f64::from(l % 13) * 17.25 };
    let mut users = Vec::new();
    for _ in 0..500 {
        let n = 1 + usize::try_from(next() % 6).unwrap();
        let routes: Vec<Vec<u32>> = (0..n)
            .map(|_| {
                let len = 1 + next() % 40;
                (0..len).map(|_| u32::try_from(next() % 60).unwrap()).collect()
            })
            .collect();
        let refs: Vec<&[u32]> = routes.iter().map(Vec::as_slice).collect();
        let sorted = openmobisim_core_routes::path_sizes(&refs, length);
        let counted = openmobisim_core_routes::path_sizes_counted(&refs, length, &mut users);
        let bits = |v: &[f64]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
        assert_eq!(bits(&sorted), bits(&counted));
        assert!(users.iter().all(|&u| u == 0), "the scratch is left all zeros");
    }
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

#[test]
fn the_fastest_route_is_the_route_the_fastest_time_prices() {
    // The same times as above; now the route itself is asked for, and it is the one whose
    // walk through those times takes the time the search reports.
    let net = diamond();
    let turns = turns_of(&net);
    let id = |n: &str| net.link_external_ids().typed_id_of::<LinkId>(n).expect("link");
    let (ab, bd, ac, cd) = (id("ab"), id("bd"), id("ac"), id("cd"));
    let (a, d) = (node(&net, "A"), node(&net, "D"));
    let ctx = openmobisim_core_routes::SearchContext::new(&net, &turns);
    let mut search = openmobisim_core_routes::Search::new(&ctx);
    let seconds = |l: u32, at: f64| {
        if l == ab.raw() {
            if at < 100.0 { 50.0 } else { 500.0 }
        } else if l == bd.raw() {
            25.0
        } else if l == ac.raw() || l == cd.raw() {
            30.0
        } else {
            f64::INFINITY
        }
    };
    let jam_bottom = |l: u32, at: f64| if l == ac.raw() { 400.0 } else { seconds(l, at) };
    let no_wait = |_: u32, _: f64| 0.0;
    let inf = f64::INFINITY;
    // Leaving at 0 s the bottom road (60 s) beats the top (75 s); with the bottom jammed, the top
    // wins (75 s); leaving at 200 s the top's first link has jammed and the bottom wins again.
    let mut check =
        |times: &dyn Fn(u32, f64) -> f64, departure: f64, expected: f64, via: [LinkId; 2]| {
            let (time, route) =
                search.fastest_route(a, d, departure, inf, &no_wait, times).expect("a route");
            assert!((time - expected).abs() < 1e-12, "{time}");
            assert_eq!(route, via.to_vec());
            assert_eq!(
                search.fastest_time(a, d, departure, inf, &no_wait, times),
                Some(time),
                "the two agree"
            );
        };
    check(&seconds, 0.0, 60.0, [ac, cd]);
    check(&jam_bottom, 0.0, 75.0, [ab, bd]);
    check(&seconds, 200.0, 60.0, [ac, cd]);
    // A bound nothing meets; the same place is no time and no links; and the search is left clean.
    assert_eq!(search.fastest_route(a, d, 0.0, 50.0, &no_wait, &seconds), None);
    assert_eq!(search.fastest_route(a, a, 0.0, 1.0, &no_wait, &seconds), Some((0.0, Vec::new())));
    assert_eq!(
        search.fastest_route(a, d, 0.0, 60.0, &no_wait, &seconds).map(|(_, r)| r),
        Some(vec![ac, cd]),
        "a bound of exactly the time still finds it"
    );
}

// --- growing a store (S176) -------------------------------------------------------------------------

fn added(links: &[u32], cost: f64) -> openmobisim_core_routes::Route {
    openmobisim_core_routes::Route {
        links: links.iter().map(|&l| LinkId::new(l)).collect(),
        cost,
        overlap: 0.25,
    }
}

#[test]
fn an_extended_store_keeps_every_route_and_puts_the_new_ones_last() {
    let (net, _) = manhattan_grid(6, 200.0, true);
    let turns = turns_of(&net);
    let keys = grid_keys(&net, 6, 40, 5);
    let old = RouteSets::generate(&net, &turns, &keys, &Penalty::default());
    let key_count = old.keys().len();
    assert!(key_count > 10);
    // Routes added to keys 2, 5 and the last one: one, three and two of them.
    let picks = [(2, 1), (5, 3), (key_count - 1, 2)];
    let links = net.link_count();
    // Made-up routes: the store holds what it is given, so only the link ids must exist.
    let additions: Vec<(usize, Vec<_>)> = picks
        .iter()
        .map(|&(k, n)| {
            let route =
                |i: u32| added(&[7, 8, (9 + i + 10 * k as u32) % links], 100.0 + f64::from(i));
            (k, (0..n).map(route).collect())
        })
        .collect();
    let (grown, shift) = old.extended(&additions, 4, "best_response;test");

    // Sizes and identity.
    assert_eq!(grown.keys(), old.keys());
    assert_eq!(grown.route_count(), old.route_count() + 6);
    assert_eq!((grown.method(), grown.descriptor()), (old.method(), old.descriptor()));
    assert_eq!((old.update(), grown.update()), ("", "best_response;test"));
    assert_ne!(old.identity(), grown.identity(), "grown sets are not the sets the method made");
    assert!(grown.matches(&net));
    // Every old route is where the shift says, with what it had.
    assert_eq!(shift.len(), key_count);
    for (k, &shifted) in shift.iter().enumerate() {
        let before: u32 =
            additions.iter().filter(|(a, _)| *a < k).map(|(_, r)| r.len() as u32).sum();
        assert_eq!(shifted, before, "key {k}");
        for r in old.route_range(k) {
            let (o, g) = (old.route(r), grown.route(r + shifted as usize));
            assert_eq!(
                (o.links, o.cost.to_bits(), o.overlap.to_bits()),
                (g.links, g.cost.to_bits(), g.overlap.to_bits())
            );
            assert_eq!(old.stamps()[r], grown.stamps()[r + shifted as usize]);
            assert_eq!(grown.key_of_route(r + shifted as usize), k);
        }
    }
    // The new routes follow the old ones of their key, in the order given, stamped, costed as given.
    for (k, routes) in &additions {
        let range = grown.route_range(*k);
        assert_eq!(range.len(), old.route_range(*k).len() + routes.len());
        for (i, route) in routes.iter().enumerate() {
            let r = range.start + old.route_range(*k).len() + i;
            let v = grown.route(r);
            let expected: Vec<u32> = route.links.iter().map(|l| l.raw()).collect();
            assert_eq!(v.links, expected.as_slice());
            assert!(
                (f64::from(v.cost) - route.cost).abs() < 1e-4 && (v.overlap - 0.25).abs() < 1e-6
            );
            assert_eq!(grown.stamps()[r], 4);
        }
    }
    assert_eq!(grown.stamps().iter().filter(|&&s| s == 4).count(), 6);
    // Keys nobody added to keep their sizes; the inverted index and the offsets are consistent.
    assert_eq!(*grown.set_start().last().unwrap() as usize, grown.route_count());
    assert_eq!(*grown.route_start().last().unwrap() as usize, grown.links().len());
    let index = grown.link_index(net.link_count());
    assert!(index.routes_using(7).len() >= 6, "the added routes are in the inverted index");
    // Adding nothing is a copy: equal, with the same identity.
    let (same, none) = old.extended(&[], 1, "");
    assert_eq!(same, old);
    assert_eq!(same.identity(), old.identity());
    assert!(none.iter().all(|&s| s == 0));
    // Growing twice is growing once with both: the second update's descriptor is the store's.
    let (twice, _) = grown.extended(&[(0, vec![added(&[1, 2], 10.0)])], 5, "best_response;other");
    assert_eq!(twice.update(), "best_response;other");
    assert_eq!(twice.route_count(), grown.route_count() + 1);
}

#[test]
#[should_panic(expected = "strictly ascending")]
fn additions_out_of_key_order_are_refused() {
    let net = diamond();
    let (a, d) = (node(&net, "A"), node(&net, "D"));
    let sets = RouteSets::generate(
        &net,
        &turns_of(&net),
        &[RouteKey::new(a, d), RouteKey::new(d, a)],
        &Shortest,
    );
    let _ = sets.extended(&[(1, vec![added(&[0], 1.0)]), (0, vec![added(&[0], 1.0)])], 1, "x");
}

#[test]
#[should_panic(expected = "key the store does not have")]
fn additions_to_a_key_that_does_not_exist_are_refused() {
    let net = diamond();
    let (a, d) = (node(&net, "A"), node(&net, "D"));
    let sets = RouteSets::generate(&net, &turns_of(&net), &[RouteKey::new(a, d)], &Shortest);
    let _ = sets.extended(&[(1, vec![added(&[0], 1.0)])], 1, "x");
}

#[test]
fn a_search_over_many_items_returns_its_results_in_the_order_of_the_items() {
    let (net, _) = manhattan_grid(8, 150.0, true);
    let turns = turns_of(&net);
    let ctx = openmobisim_core_routes::SearchContext::new(&net, &turns);
    let items = grid_keys(&net, 8, 200, 17);
    let work = |search: &mut openmobisim_core_routes::Search<'_>, k: &RouteKey| {
        search.shortest(NodeId::new(k.origin), NodeId::new(k.destination)).map(|r| r.links)
    };
    let sequential: Vec<_> = {
        let mut search = openmobisim_core_routes::Search::new(&ctx);
        items.iter().map(|k| work(&mut search, k)).collect()
    };
    assert_eq!(openmobisim_core_routes::search_map(&ctx, &items, work), sequential);
    #[cfg(feature = "parallel")]
    for threads in [1, 3, 8] {
        let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().expect("pool");
        assert_eq!(
            pool.install(|| openmobisim_core_routes::search_map(&ctx, &items, work)),
            sequential,
            "{threads} threads"
        );
    }
    assert!(openmobisim_core_routes::search_map(&ctx, &Vec::<RouteKey>::new(), work).is_empty());
}

/// One search to many targets finds, for each, what a search to it alone
/// finds: the same earliest arrival and the same route, nearest first; a target
/// beyond the bound is not found; the origin, if a target, is found with no links.
#[test]
fn one_search_to_many_targets_equals_one_search_to_each() {
    let (net, _) = manhattan_grid(7, 200.0, true);
    let turns = turns_of(&net);
    let ctx = SearchContext::new(&net, &turns);
    let mut search = Search::new(&ctx);
    // Time-dependent costs: every link 10% slower after 100 s, and a wait at the origin.
    let seconds = |l: u32, at: f64| {
        let base = net.free_flow_time(LinkId::new(l)).get();
        if at > 100.0 { base * 1.1 } else { base }
    };
    let wait = |_l: u32, _t: f64| 3.0;
    let origin = node(&net, &node_name(0, 0));
    let names =
        [node_name(0, 0), node_name(3, 4), node_name(6, 6), node_name(2, 1), node_name(5, 0)];
    let mut targets = vec![false; net.node_count() as usize];
    for n in &names {
        targets[node(&net, n).index()] = true;
    }
    let found = search.fastest_routes_to(origin, &targets, names.len(), 0.0, 1e9, &wait, &seconds);
    assert_eq!(found.len(), names.len());
    assert_eq!((found[0].0, found[0].1, found[0].2.len()), (origin, 0.0, 0));
    for w in found.windows(2) {
        assert!(w[0].1 <= w[1].1, "nearest first");
    }
    for (target, time, links) in &found[1..] {
        let (t, l) =
            search.fastest_route(origin, *target, 0.0, 1e9, &wait, &seconds).expect("reachable");
        assert!((t - time).abs() < 1e-9, "{t} vs {time}");
        assert_eq!(&l, links);
    }
    // A bound below the farthest target's time leaves it out.
    let farthest = found.last().expect("found").1;
    let near = search.fastest_routes_to(
        origin,
        &targets,
        names.len(),
        0.0,
        farthest - 1.0,
        &wait,
        &seconds,
    );
    assert_eq!(near.len(), names.len() - 1);
    // Asking for fewer stops early: the nearest two.
    let two = search.fastest_routes_to(origin, &targets, 2, 0.0, 1e9, &wait, &seconds);
    assert_eq!(
        two.iter().map(|f| f.0).collect::<Vec<_>>(),
        found[..2].iter().map(|f| f.0).collect::<Vec<_>>()
    );
}

/// An irregular network: nodes jittered off a grid, links curved (longer than the
/// straight line between their ends, as real streets are) and of mixed classes, so
/// free-flow speeds differ by a factor of four across the network.
fn irregular(n: u32, seed: u64) -> RoadNetwork {
    let mut state = seed;
    let mut unit = || -> f64 {
        state =
            state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        (state >> 11) as f64 / (1u64 << 53) as f64
    };
    let mut b = RoadNetworkBuilder::new();
    let mut pos = Vec::new();
    for i in 0..n {
        for j in 0..n {
            let (x, y) =
                (f64::from(i) * 150.0 + unit() * 90.0, f64::from(j) * 150.0 + unit() * 90.0);
            pos.push((x, y));
            b.add_node(format!("n{i}_{j}"), LonLat::new(4.8 + x / 77_800.0, 45.7 + y / 110_574.0));
        }
    }
    let classes =
        [RoadClass::Residential, RoadClass::Secondary, RoadClass::Primary, RoadClass::Motorway];
    let mut k = 0;
    for i in 0..n {
        for j in 0..n {
            for (di, dj) in [(1, 0), (0, 1)] {
                let (i2, j2) = (i + di, j + dj);
                if i2 >= n || j2 >= n {
                    continue;
                }
                let (a, c) = (pos[(i * n + j) as usize], pos[(i2 * n + j2) as usize]);
                let chord = ((a.0 - c.0).powi(2) + (a.1 - c.1).powi(2)).sqrt();
                let class = classes[(unit() * 4.0) as usize % 4];
                let len = chord * (1.0 + unit() * 0.5);
                for (from, to) in [
                    (format!("n{i}_{j}"), format!("n{i2}_{j2}")),
                    (format!("n{i2}_{j2}"), format!("n{i}_{j}")),
                ] {
                    let mut spec = LinkSpec::new(class);
                    spec.length_m = Some(len);
                    b.add_link(format!("l{k}"), &from, &to, spec);
                    k += 1;
                }
            }
        }
    }
    b.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
        .expect("buildable")
}

/// **The goal-directed search is exact**: on an irregular network with mixed
/// speeds and curved links, every route `Search::shortest` returns costs exactly what an
/// independent Dijkstra finds, and a pair with no route has none either way. The lower
/// bound is positive there, so A* is really in play.
#[test]
fn the_goal_directed_search_finds_a_least_cost_route_on_an_irregular_network() {
    let net = irregular(12, 7);
    let turns = turns_of(&net);
    let ctx = SearchContext::new(&net, &turns);
    assert!(ctx.lower_bound_per_metre() > 0.0, "a bound that does nothing would test nothing");
    let mut search = Search::new(&ctx);
    let count = net.node_count();
    let mut state = 99u64;
    let mut next = || {
        state =
            state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        ((state >> 33) % u64::from(count)) as u32
    };
    for _ in 0..300 {
        let (o, d) = (NodeId::new(next()), NodeId::new(next()));
        if o == d {
            continue;
        }
        let found = search.shortest(o, d).map(|r| r.cost);
        let reference = reference_cost(&net, o, d);
        match (found, reference) {
            (Some(a), Some(b)) => assert!((a - b).abs() <= 1e-9 * b.max(1.0), "{a} against {b}"),
            (None, None) => {}
            other => panic!("one search found a route, the other did not: {other:?}"),
        }
    }
}

/// An independent, node-based earliest-arrival search with no lower bound (time-dependent
/// Dijkstra), for reference: link times from `seconds`, no origin wait.
fn reference_fastest(
    net: &RoadNetwork,
    from: NodeId,
    to: NodeId,
    departure: f64,
    seconds: &dyn Fn(u32, f64) -> f64,
) -> Option<f64> {
    let mut best = vec![f64::INFINITY; net.node_count() as usize];
    let mut heap = BinaryHeap::new();
    best[from.index()] = departure;
    heap.push(core::cmp::Reverse((departure.to_bits(), from.raw())));
    while let Some(core::cmp::Reverse((bits, n))) = heap.pop() {
        let (t, n) = (f64::from_bits(bits), NodeId::new(n));
        if t > best[n.index()] {
            continue;
        }
        if n == to {
            return Some(t - departure);
        }
        for &l in net.out_links(n) {
            if !net.link_class(l).carries_motor_traffic() {
                continue;
            }
            let (m, nt) = (net.link_to(l), t + seconds(l.raw(), t));
            if nt < best[m.index()] {
                best[m.index()] = nt;
                heap.push(core::cmp::Reverse((nt.to_bits(), m.raw())));
            }
        }
    }
    None
}

/// **The goal-directed time-dependent search is exact**: with link times that
/// change through the hour, are never below free flow (as a loading's are) and keep FIFO,
/// the fastest time found agrees with an independent time-dependent Dijkstra, unbounded and
/// bounded.
#[test]
fn the_goal_directed_fastest_time_agrees_with_a_plain_time_dependent_search() {
    let net = irregular(12, 3);
    let turns = turns_of(&net);
    let ctx = SearchContext::new(&net, &turns);
    let mut search = Search::new(&ctx);
    // Congestion that comes and goes: each link slower by up to 3 times free flow, by a
    // pattern of knots every five minutes, interpolated in between, the same function of
    // (link, time) each call. It changes slowly enough that a later entry never means an
    // earlier exit (FIFO), the condition under which any earliest-arrival search is exact.
    let knot = |l: u32, k: u64| {
        let mut x = (u64::from(l) << 20) ^ k;
        x = x.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        1.0 + 2.0 * ((x >> 11) as f64 / (1u64 << 53) as f64)
    };
    let seconds = |l: u32, t: f64| {
        let k = (t / 300.0).floor();
        let f = t / 300.0 - k;
        let slow = knot(l, k as u64) * (1.0 - f) + knot(l, k as u64 + 1) * f;
        net.free_flow_time(LinkId::new(l)).get() * slow
    };
    let none = |_: u32, _: f64| 0.0;
    let count = net.node_count();
    let mut state = 5u64;
    let mut next = || {
        state =
            state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        ((state >> 33) % u64::from(count)) as u32
    };
    for i in 0..200 {
        let (o, d) = (NodeId::new(next()), NodeId::new(next()));
        if o == d {
            continue;
        }
        let departure = 7.0 * 3600.0 + f64::from(i) * 37.0;
        let reference = reference_fastest(&net, o, d, departure, &seconds);
        let found = search.fastest_time(o, d, departure, f64::INFINITY, &none, &seconds);
        match (found, reference) {
            (Some(a), Some(b)) => assert!((a - b).abs() <= 1e-9 * b.max(1.0), "{a} against {b}"),
            (None, None) => {}
            other => panic!("one search found a route, the other did not: {other:?}"),
        }
        // Bounded at the reference time: still found; just under it: not.
        if let Some(b) = reference {
            assert!(
                search.fastest_time(o, d, departure, b * (1.0 + 1e-9), &none, &seconds).is_some()
            );
            assert!(
                search.fastest_time(o, d, departure, b * (1.0 - 1e-6), &none, &seconds).is_none()
            );
        }
    }
}

/// **The search after a link (en-route rerouting):** from the end of a link, through its
/// legal turns only. On a grid, a vehicle on `(1,1)→(1,2)` heading back to `(1,1)` may not
/// U-turn (the turn table allows that only at a dead end), so it goes round a block: four
/// links, against the one-link reverse a search from the node would find. Elsewhere it agrees
/// with the search from the link's end wherever that one's first link is a legal turn.
#[test]
fn the_search_after_a_link_turns_only_where_the_link_allows() {
    let (net, _) = manhattan_grid(5, 200.0, false);
    let turns = turns_of(&net);
    let ctx = SearchContext::new(&net, &turns);
    let mut search = Search::new(&ctx);
    let link = |a: (u32, u32), b: (u32, u32)| {
        let (from, to) = (node(&net, &node_name(a.0, a.1)), node(&net, &node_name(b.0, b.1)));
        *net.out_links(from).iter().find(|&&l| net.link_to(l) == to).expect("grid link")
    };
    let free = |l: u32, _t: f64| ctx.link_cost(LinkId::new(l));
    let east = link((1, 1), (1, 2));
    let home = node(&net, &node_name(1, 1));
    let (time, route) =
        search.fastest_route_after(east, home, 0.0, f64::INFINITY, &free).expect("a way back");
    assert_eq!(route.len(), 3, "round the block, not back the way it came: {route:?}");
    assert!(time > ctx.link_cost(east) * 2.5);
    assert_ne!(route[0], link((1, 2), (1, 1)), "no U-turn");
    // Away from the U-turn, it agrees with the search from the link's end.
    let far = node(&net, &node_name(4, 4));
    let (t_after, r_after) =
        search.fastest_route_after(east, far, 0.0, f64::INFINITY, &free).unwrap();
    let from = net.link_to(east);
    let (t_node, _) =
        search.fastest_route(from, far, 0.0, f64::INFINITY, &|_, _| 0.0, &free).unwrap();
    assert!((t_after - t_node).abs() < 1e-9, "{t_after} vs {t_node}");
    assert_eq!(net.link_from(r_after[0]), from);
    // Bounded: nothing within half the time.
    assert!(search.fastest_route_after(east, far, 0.0, t_node / 2.0, &free).is_none());
}

/// **Property (X-70, S294):** a key whose pair has no route (its destination cannot be reached)
/// has an empty set, and a revision of the store, a full set making room elsewhere, leaves it
/// empty.
#[test]
fn a_revised_store_leaves_a_key_without_routes_empty() {
    // a -> b and c -> b: from a, c cannot be reached.
    let mut b = RoadNetworkBuilder::new();
    b.add_node("a", LonLat::new(4.800, 45.700));
    b.add_node("b", LonLat::new(4.805, 45.700));
    b.add_node("c", LonLat::new(4.810, 45.700));
    b.add_link("ab", "a", "b", LinkSpec::new(RoadClass::Primary));
    b.add_link("cb", "c", "b", LinkSpec::new(RoadClass::Primary));
    let net = b
        .build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
        .expect("buildable");
    let turns = turns_of(&net);
    let node = |n: &str| net.node_external_ids().typed_id_of::<NodeId>(n).unwrap();
    let reachable = RouteKey::new(node("a"), node("b"));
    let unreachable = RouteKey::new(node("a"), node("c"));
    let sets = RouteSets::generate(&net, &turns, &[reachable, unreachable], &Shortest);
    let k = sets.key_index(unreachable).expect("the key is kept");
    assert!(sets.route_range(k).is_empty(), "no route for a pair that cannot be reached");
    let other = sets.key_index(reachable).expect("the other key");
    assert_eq!(sets.route_range(other).len(), 1);
    // A revision that drops the other key's route for a new one: the empty key stays empty.
    let mut removed = vec![false; sets.route_count()];
    removed[sets.route_range(other).start] = true;
    let ab = net.link_external_ids().typed_id_of::<LinkId>("ab").unwrap().raw();
    let additions = vec![(other, vec![added(&[ab], 50.0)])];
    let (revised, map) = sets.revised(&additions, &removed, 3, "best_response;test");
    assert!(revised.route_range(k).is_empty());
    assert_eq!(revised.route_count(), sets.route_count());
    assert_eq!(map[sets.route_range(other).start], u32::MAX, "the dropped route is gone");
}

// --- extras: generalised cost (S298) ---------------------------------------------------------------

/// A per-link extra in seconds from a hash: 0 to 30 s, a pure function of the link.
fn hashed_extras(net: &RoadNetwork, seed: u64) -> Vec<f64> {
    (0..net.link_count())
        .map(|l| {
            let mut x = seed ^ u64::from(l).wrapping_mul(0x9E37_79B9_7F4A_7C15);
            x ^= x >> 29;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            30.0 * ((x >> 11) as f64 / (1u64 << 53) as f64)
        })
        .collect()
}

/// **Extras move the search and leave the route's time alone**: on the diamond an extra of
/// 60 s on the top road's first link makes the bottom road the cheapest, both for a pair and in
/// a tree; the route's `cost` is still its travel time; `generalised` adds the extra; extras of
/// nothing give the search without them, to the bit.
#[test]
fn extras_move_the_search_and_leave_the_route_s_time_alone() {
    let net = diamond();
    let turns = turns_of(&net);
    let ctx = SearchContext::new(&net, &turns);
    let mut search = Search::new(&ctx);
    let id = |n: &str| net.link_external_ids().typed_id_of::<LinkId>(n).expect("link");
    let (ab, bd, ac, cd) = (id("ab"), id("bd"), id("ac"), id("cd"));
    let (a, d) = (node(&net, "A"), node(&net, "D"));
    let top = search.shortest(a, d).expect("a route");
    assert_eq!(top.links, vec![ab, bd]);
    let bottom_time = net.free_flow_time(ac).get() + net.free_flow_time(cd).get();
    let mut extra = vec![0.0; net.link_count() as usize];
    extra[ab.index()] = 60.0;
    search.set_extras(Some(ctx.extras(extra)));
    let found = search.shortest(a, d).expect("a route");
    assert_eq!(found.links, vec![ac, cd], "the bottom road is the cheapest with the extra");
    assert!((found.cost - bottom_time).abs() < 1e-9, "cost is the travel time: {}", found.cost);
    assert!((search.generalised(&found.links) - bottom_time).abs() < 1e-9);
    assert!((search.generalised(&top.links) - (top.cost + 60.0)).abs() < 1e-9);
    // In a tree too.
    let tree = search.shortest_tree(a, &[d], f64::INFINITY);
    assert_eq!(tree[0].as_ref().map(|r| r.links.clone()), Some(vec![ac, cd]));
    // A bound in generalised cost: just under the bottom's cost there is nothing (the top costs
    // more once its extra counts); at it, the bottom.
    assert!(search.shortest_within(a, d, bottom_time * (1.0 - 1e-6)).is_none());
    assert!(search.shortest_within(a, d, bottom_time * (1.0 + 1e-9)).is_some());
    // Extras of nothing: the search as it was, to the bit.
    search.set_extras(Some(ctx.extras(vec![0.0; net.link_count() as usize])));
    let same = search.shortest(a, d).expect("a route");
    assert_eq!((same.links.clone(), same.cost.to_bits()), (top.links.clone(), top.cost.to_bits()));
    // Cleared with the rest of the route state.
    search.clear_route_state();
    assert_eq!(search.shortest(a, d).expect("a route").links, vec![ab, bd]);
}

/// **A bounded search under extras never passes its bound, in generalised cost**: on a grid with
/// hashed extras, for many pairs, a bound at the cheapest route's generalised cost finds it and
/// one just under finds nothing; the route found is the cheapest (against an enumeration-free
/// check: no tree route is cheaper).
#[test]
fn a_bounded_search_with_extras_never_passes_its_bound_in_generalised_cost() {
    let (net, _) = manhattan_grid(7, 200.0, true);
    let turns = turns_of(&net);
    let ctx = SearchContext::new(&net, &turns);
    let mut search = Search::new(&ctx);
    search.set_extras(Some(ctx.extras(hashed_extras(&net, 11))));
    for key in grid_keys(&net, 7, 60, 9) {
        let (o, d) = (NodeId::new(key.origin), NodeId::new(key.destination));
        let route = search.shortest(o, d).expect("a grid is connected");
        let g = search.generalised(&route.links);
        let at = search.shortest_within(o, d, g * (1.0 + 1e-9)).expect("at the bound: found");
        assert!((search.generalised(&at.links) - g).abs() < 1e-9);
        assert!(search.shortest_within(o, d, g * (1.0 - 1e-6)).is_none(), "under it: nothing");
        let tree = search.shortest_tree(o, &[d], f64::INFINITY);
        let t = tree[0].as_ref().expect("connected");
        assert!((search.generalised(&t.links) - g).abs() < 1e-9, "the tree agrees");
    }
}

/// **With nothing set, the cheapest tree is the fastest tree**: the same targets, the same
/// routes, the generalised cost equal to the time, on time-dependent costs with a wait.
#[test]
fn the_cheapest_tree_with_nothing_set_finds_the_fastest_routes() {
    let (net, _) = manhattan_grid(7, 200.0, true);
    let turns = turns_of(&net);
    let ctx = SearchContext::new(&net, &turns);
    let mut search = Search::new(&ctx);
    let seconds = |l: u32, at: f64| {
        let base = net.free_flow_time(LinkId::new(l)).get();
        if at > 100.0 { base * 1.1 } else { base }
    };
    let wait = |_l: u32, _t: f64| 3.0;
    let origin = node(&net, &node_name(0, 0));
    let names =
        [node_name(0, 0), node_name(3, 4), node_name(6, 6), node_name(2, 1), node_name(5, 0)];
    let mut targets = vec![false; net.node_count() as usize];
    for n in &names {
        targets[node(&net, n).index()] = true;
    }
    let fastest =
        search.fastest_routes_to(origin, &targets, names.len(), 50.0, 1e9, &wait, &seconds);
    let cheapest =
        search.cheapest_routes_to(origin, &targets, names.len(), 50.0, 1e9, &wait, &seconds);
    assert_eq!(cheapest.len(), fastest.len());
    assert_eq!((cheapest[0].0, cheapest[0].1, cheapest[0].2), (origin, 0.0, 0.0));
    for (c, f) in cheapest.iter().zip(&fastest) {
        assert_eq!(c.0, f.0, "the same target in the same order");
        assert!((c.2 - f.1).abs() < 1e-9, "the same time: {} against {}", c.2, f.1);
        assert!((c.1 - f.1).abs() < 1e-9, "generalised cost equals time without extras");
        assert_eq!(c.3, f.2, "the same route");
    }
    // Bounded under the farthest target's cost: it is left out.
    let farthest = cheapest.last().expect("found").1;
    let near = search.cheapest_routes_to(
        origin,
        &targets,
        names.len(),
        50.0,
        farthest - 1.0,
        &wait,
        &seconds,
    );
    assert_eq!(near.len(), names.len() - 1);
}

/// Every simple path from `from` to `to` over motor links, its generalised cost (static
/// seconds plus extras), the least kept: the brute-force reference for a small grid.
fn cheapest_simple_path(net: &RoadNetwork, from: NodeId, to: NodeId, extra: &[f64]) -> Option<f64> {
    fn go(
        net: &RoadNetwork,
        at: NodeId,
        to: NodeId,
        cost: f64,
        seen: &mut Vec<bool>,
        extra: &[f64],
        best: &mut f64,
    ) {
        if at == to {
            *best = best.min(cost);
            return;
        }
        for &l in net.out_links(at) {
            if !net.link_class(l).carries_motor_traffic() {
                continue;
            }
            let m = net.link_to(l);
            if seen[m.index()] {
                continue;
            }
            seen[m.index()] = true;
            go(
                net,
                m,
                to,
                cost + net.free_flow_time(l).get() + extra[l.index()],
                seen,
                extra,
                best,
            );
            seen[m.index()] = false;
        }
    }
    let mut seen = vec![false; net.node_count() as usize];
    seen[from.index()] = true;
    let mut best = f64::INFINITY;
    go(net, from, to, 0.0, &mut seen, extra, &mut best);
    best.is_finite().then_some(best)
}

/// **The cheapest tree agrees with an enumeration of every simple path** on a 3×3 grid (no
/// signals, so a link's time is its length over its speed), with hashed extras: for every
/// destination, the generalised cost found is the least over all simple paths.
#[test]
fn the_cheapest_tree_agrees_with_an_enumeration_of_every_simple_path() {
    let (net, _) = manhattan_grid(3, 200.0, false);
    let turns = turns_of(&net);
    let ctx = SearchContext::new(&net, &turns);
    let mut search = Search::new(&ctx);
    let extra = hashed_extras(&net, 5);
    search.set_extras(Some(ctx.extras(extra.clone())));
    let seconds = |l: u32, _at: f64| net.free_flow_time(LinkId::new(l)).get();
    let none = |_: u32, _: f64| 0.0;
    for o in 0..net.node_count() {
        let origin = NodeId::new(o);
        let targets: Vec<bool> = (0..net.node_count()).map(|n| n != o).collect();
        let found = search.cheapest_routes_to(
            origin,
            &targets,
            net.node_count() as usize - 1,
            0.0,
            f64::INFINITY,
            &none,
            &seconds,
        );
        assert_eq!(found.len(), net.node_count() as usize - 1, "a grid is connected");
        for (target, generalised, time, links) in &found {
            let reference = cheapest_simple_path(&net, origin, *target, &extra).expect("connected");
            assert!(
                (generalised - reference).abs() < 1e-9,
                "{o}→{}: {generalised} against {reference}",
                target.raw()
            );
            let walked: f64 = links.iter().map(|&l| net.free_flow_time(l).get()).sum();
            assert!((time - walked).abs() < 1e-9, "the time is the route's");
            assert!((search.generalised(links) - generalised).abs() < 1e-9);
        }
        // The static tree finds the same costs.
        let destinations: Vec<NodeId> = found.iter().map(|f| f.0).collect();
        let tree = search.shortest_tree(origin, &destinations, f64::INFINITY);
        for (f, t) in found.iter().zip(&tree) {
            let t = t.as_ref().expect("connected");
            assert!((search.generalised(&t.links) - f.1).abs() < 1e-9);
        }
    }
}

/// **Noise and extras in the cheapest tree are deterministic and bounded**: two calls agree; a
/// perturbed route's unperturbed generalised cost is never below the unperturbed cheapest's and
/// never past the bound; another seed may find another route.
#[test]
fn noise_and_extras_in_the_cheapest_tree_are_deterministic_and_bounded() {
    let (net, _) = manhattan_grid(7, 200.0, true);
    let turns = turns_of(&net);
    let ctx = SearchContext::new(&net, &turns);
    let mut search = Search::new(&ctx);
    let extras = ctx.extras(hashed_extras(&net, 2));
    let seconds = |l: u32, at: f64| {
        let base = net.free_flow_time(LinkId::new(l)).get();
        if at > 300.0 { base * 1.3 } else { base }
    };
    let none = |_: u32, _: f64| 0.0;
    let origin = node(&net, &node_name(0, 0));
    let targets: Vec<bool> = (0..net.node_count()).map(|n| NodeId::new(n) != origin).collect();
    let wanted = net.node_count() as usize - 1;
    search.set_extras(Some(extras.clone()));
    let plain =
        search.cheapest_routes_to(origin, &targets, wanted, 0.0, f64::INFINITY, &none, &seconds);
    let by_target = |found: &[(NodeId, f64, f64, Vec<LinkId>)]| {
        let mut v: Vec<(u32, f64, Vec<LinkId>)> =
            found.iter().map(|f| (f.0.raw(), f.1, f.3.clone())).collect();
        v.sort_by_key(|f| f.0);
        v
    };
    let plain = by_target(&plain);
    let bound = plain.iter().map(|f| f.1).fold(0.0, f64::max) * 1.5;
    let mut different = 0;
    for seed in [1_u64, 2, 3] {
        search.set_noise(seed, 0.5, None);
        search.set_extras(Some(extras.clone()));
        let a = by_target(
            &search.cheapest_routes_to(origin, &targets, wanted, 0.0, bound, &none, &seconds),
        );
        search.set_noise(seed, 0.5, None);
        search.set_extras(Some(extras.clone()));
        let b = by_target(
            &search.cheapest_routes_to(origin, &targets, wanted, 0.0, bound, &none, &seconds),
        );
        assert_eq!(a, b, "the same seed gives the same routes");
        for (p, n) in plain.iter().zip(&a) {
            assert_eq!(p.0, n.0);
            assert!(n.1 >= p.1 - 1e-9, "never cheaper than the unperturbed cheapest");
            assert!(n.1 <= bound + 1e-9, "never past the bound");
            if n.2 != p.2 {
                different += 1;
            }
        }
        search.clear_noise();
    }
    assert!(different > 0, "noise finds other routes");
    search.clear_route_state();
}
