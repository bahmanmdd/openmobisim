//! The sampled method: routes by each user class's generalised cost, in preference draws and
//! under attribute noise.
//!
//! What is defended, and how each is checked:
//!
//! * **draw 0 without extras is the anchor**: no draws and no extras give exactly the shortest
//!   method's routes and costs, pair by pair;
//! * **extras move the routes** on a two-road diamond, by hand: an extra on the shorter road
//!   brings in the longer one, kept with its travel time, unless the detour limit refuses it; a
//!   second class searching by time alone repeats nothing;
//! * **what it makes is a set of routes**: new, connected, loop-free, turn-respecting, inside
//!   the detour and overlap limits, the first a shortest one, and noise alone finds alternatives;
//! * **the sets are a function of the keys, the seed, the options and the extras alone**: any
//!   order of keys, any thread count; another seed is another set;
//! * **the descriptor names everything**, the extras by a hash of their values, and preparing for
//!   a context changes nothing but speed;
//! * **bad options are refused** with the range in the message.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "test fixtures: small counts and indices"
)]

use std::sync::Arc;

use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
use openmobisim_core_graph::examples::{manhattan_grid, node_name};
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::network::{LinkSpec, RoadNetwork, RoadNetworkBuilder};
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_routes::{
    ClassExtras, ExtrasRequest, Options, Registry, Route, RouteError, RouteKey, RouteSetGenerator,
    RouteSets, Sampled, Search, SearchContext, Shortest, generator,
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

/// A, B, C, D: the top road A-B-D is 200 m, the bottom road A-C-D is 240 m; one-way.
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

/// A per-link extra in seconds from a hash: 0 to 30 s, a pure function of the link and `seed`.
fn hashed_extras(net: &RoadNetwork, seed: u64) -> Arc<[f64]> {
    (0..net.link_count())
        .map(|l| {
            let mut x = seed ^ u64::from(l).wrapping_mul(0x9E37_79B9_7F4A_7C15);
            x ^= x >> 29;
            x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            30.0 * ((x >> 11) as f64 / (1u64 << 53) as f64)
        })
        .collect()
}

/// Two classes of `draws + 1` draws each: the first with extras in every draw, the second by
/// time alone in draw 0 and with extras in the others.
fn two_classes(net: &RoadNetwork, draws: u32) -> ClassExtras {
    let first = (0..=u64::from(draws)).map(|d| Some(hashed_extras(net, 10 + d))).collect();
    let second =
        (0..=u64::from(draws)).map(|d| (d > 0).then(|| hashed_extras(net, 20 + d))).collect();
    vec![first, second]
}

/// `Sampled` with `options`, given `extras`.
fn given(options: &[(&str, f64)], extras: ClassExtras) -> Box<dyn RouteSetGenerator> {
    Sampled::from_options(&opts(options)).expect("valid").with_extras(extras).expect("takes them")
}

/// Each key's routes as (links, cost bits, overlap bits).
fn routes_of(sets: &RouteSets, i: usize) -> Vec<(Vec<u32>, u32, u32)> {
    sets.routes(i).map(|r| (r.links.to_vec(), r.cost.to_bits(), r.overlap.to_bits())).collect()
}

/// The share of `a`'s free-flow time on links of `b`.
fn share(net: &RoadNetwork, a: &[u32], b: &[u32]) -> f64 {
    let time = |l: u32| net.free_flow_time(LinkId::new(l)).get();
    let total: f64 = a.iter().map(|&l| time(l)).sum();
    let shared: f64 = a.iter().filter(|l| b.contains(l)).map(|&l| time(l)).sum();
    if total > 0.0 { shared / total } else { 0.0 }
}

// --- draw 0 is the anchor --------------------------------------------------------------------

#[test]
fn no_draws_and_no_extras_give_exactly_the_shortest_routes() {
    let (net, _) = manhattan_grid(7, 200.0, true);
    let turns = turns_of(&net);
    let keys = grid_keys(&net, 7, 60, 5);
    let sampled = Sampled::from_options(&opts(&[("draws", 0.0)])).expect("valid");
    let sets = RouteSets::generate(&net, &turns, &keys, &sampled);
    let shortest = RouteSets::generate(&net, &turns, &keys, &Shortest::ONE);
    assert_eq!(sets.keys(), shortest.keys());
    for i in 0..sets.keys().len() {
        let routes = routes_of(&sets, i);
        assert_eq!(routes.len(), 1, "the anchor alone");
        assert_eq!(routes, routes_of(&shortest, i), "the same route, cost and overlap");
    }
    // One pair at a time, through `generate`, too.
    let ctx = SearchContext::new(&net, &turns);
    let mut search = Search::new(&ctx);
    for key in &keys[..10] {
        let (o, d) = (NodeId::new(key.origin), NodeId::new(key.destination));
        let one = sampled.generate(&mut search, o, d);
        let best = search.shortest_tree(o, &[d], f64::INFINITY);
        assert_eq!(one.len(), 1);
        assert_eq!(Some(&one[0]), best[0].as_ref());
    }
}

// --- the diamond with hand-made extras -------------------------------------------------------

#[test]
fn an_extra_on_the_shorter_road_brings_in_the_longer_one_within_the_detour_limit() {
    let net = diamond();
    let turns = turns_of(&net);
    let key = RouteKey::new(node(&net, "A"), node(&net, "D"));
    let id = |n: &str| link(&net, n).raw();
    let time = |n: &str| net.free_flow_time(link(&net, n)).get();
    let (top, bottom) = (time("ab") + time("bd"), time("ac") + time("cd"));
    assert!((bottom / top - 1.2).abs() < 1e-9, "the bottom road takes 1.2 times the top's time");
    assert!(top + 60.0 > bottom, "with 60 s on `ab`, the bottom road is the cheaper");
    let mut extra = vec![0.0; net.link_count() as usize];
    extra[link(&net, "ab").index()] = 60.0;
    let extra: Arc<[f64]> = extra.into();
    let routes = |g: &dyn RouteSetGenerator| {
        let sets = RouteSets::generate(&net, &turns, &[key], g);
        sets.routes(0).map(|r| (r.links.to_vec(), r.cost, r.overlap)).collect::<Vec<_>>()
    };

    let one = routes(given(&[("draws", 0.0)], vec![vec![Some(extra.clone())]]).as_ref());
    let links: Vec<Vec<u32>> = one.iter().map(|r| r.0.clone()).collect();
    // Ordered by the class's generalised cost: the bottom road (no extra) before the top one.
    assert_eq!(links, vec![vec![id("ac"), id("cd")], vec![id("ab"), id("bd")]], "cheapest first");
    assert!((f64::from(one[1].1) - top).abs() < 1e-3, "the anchor's travel time");
    assert!((f64::from(one[0].1) - bottom).abs() < 1e-3, "the bottom's travel time, no extra");
    assert_eq!((one[0].2, one[1].2), (0.0, 0.0), "the roads share no link");

    // Without the extra, draw 0 is time alone: the anchor only.
    let plain = Sampled::from_options(&opts(&[("draws", 0.0)])).expect("valid");
    assert_eq!(routes(&plain).len(), 1);
    // An extra on the bottom road only finds the top again: nothing new.
    let mut on_bottom = vec![0.0; net.link_count() as usize];
    on_bottom[link(&net, "ac").index()] = 60.0;
    assert_eq!(
        routes(given(&[("draws", 0.0)], vec![vec![Some(on_bottom.into())]]).as_ref()).len(),
        1
    );
    // Judged in the cost it was searched by (S299): the bottom takes 1.2 times the top's time,
    // past a detour limit of 1.1 in time, but costs less than the top with its extra, so it is
    // kept, with its share of the generalised cost in common with the top: none.
    let tight = given(&[("draws", 0.0), ("max_detour", 1.1)], vec![vec![Some(extra.clone())]]);
    assert_eq!(routes(tight.as_ref()), one);
    // A second class by time alone repeats nothing.
    let two = given(&[("draws", 0.0)], vec![vec![Some(extra.clone())], vec![None]]);
    assert_eq!(routes(two.as_ref()), one);
    // The set holds no more than `max_paths`.
    let single = given(&[("draws", 0.0), ("max_paths", 1.0)], vec![vec![Some(extra)]]);
    assert_eq!(routes(single.as_ref()).len(), 1);
}

// --- what it makes is a set of routes --------------------------------------------------------

#[test]
fn noise_finds_alternatives_and_every_route_is_a_route_within_the_limits() {
    let (net, _) = manhattan_grid(8, 200.0, true);
    let turns = turns_of(&net);
    let keys = grid_keys(&net, 8, 200, 11);
    let shortest = RouteSets::generate(&net, &turns, &keys, &Shortest::ONE);
    for (label, sampled) in [
        ("the defaults", Sampled::default()),
        ("max_overlap 0.5", Sampled::from_options(&opts(&[("max_overlap", 0.5)])).expect("valid")),
    ] {
        let sets = RouteSets::generate(&net, &turns, &keys, &sampled);
        let mut with_alternatives = 0;
        for (i, key) in sets.keys().iter().enumerate() {
            let (origin, destination) = (NodeId::new(key.origin), NodeId::new(key.destination));
            let routes: Vec<_> = sets.routes(i).collect();
            assert!(!routes.is_empty() && routes.len() <= sampled.max_paths, "{label}");
            let anchor = shortest.routes(i).next().expect("a grid is connected");
            assert_eq!(routes[0].links, anchor.links, "{label}: the first is the anchor");
            assert_eq!((routes[0].cost, routes[0].overlap), (anchor.cost, 0.0), "{label}");
            for w in routes.windows(2) {
                assert!(w[0].cost <= w[1].cost, "{label}: best first");
            }
            for (k, r) in routes.iter().enumerate() {
                let links: Vec<LinkId> = r.links.iter().map(|&l| LinkId::new(l)).collect();
                assert_eq!(net.link_from(links[0]), origin);
                assert_eq!(net.link_to(*links.last().expect("a link")), destination);
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
                assert!((f64::from(r.cost) - total).abs() < 1e-3 * total.max(1.0), "travel time");
                let limit = sampled.max_detour * f64::from(anchor.cost);
                assert!(f64::from(r.cost) <= limit * (1.0 + 1e-6), "{label}: detour limit");
                let overlap = f64::from(r.overlap);
                assert!(overlap <= sampled.max_overlap + 1e-6, "{label}: overlap limit");
                assert!(routes[..k].iter().all(|o| o.links != r.links), "{label}: new");
                // Whichever of two routes was kept later shares at most `max_overlap` with the
                // other.
                for o in &routes[..k] {
                    let least = share(&net, r.links, o.links).min(share(&net, o.links, r.links));
                    assert!(least <= sampled.max_overlap + 1e-6, "{label}: pairwise {least}");
                }
                // Every route but the anchor was kept after it, so its overlap counts the anchor;
                // and it is a share with one of the others.
                if k > 0 {
                    assert!(overlap >= share(&net, r.links, routes[0].links) - 1e-6, "{label}");
                    let most = routes
                        .iter()
                        .filter(|o| o.links != r.links)
                        .map(|o| share(&net, r.links, o.links))
                        .fold(0.0, f64::max);
                    assert!(overlap <= most + 1e-6, "{label}: {overlap} against {most}");
                }
            }
            if routes.len() > 1 {
                with_alternatives += 1;
            }
        }
        println!("{label}: {with_alternatives} of {} sets have alternatives", sets.keys().len());
        assert!(with_alternatives > 0, "{label}: noise alone finds alternatives");
    }
}

// --- determinism -----------------------------------------------------------------------------

#[test]
fn the_sets_do_not_depend_on_threads_or_key_order_and_the_seed_changes_them() {
    let (net, _) = manhattan_grid(9, 150.0, true);
    let turns = turns_of(&net);
    let keys = grid_keys(&net, 9, 300, 3);
    let mut shuffled = keys.clone();
    shuffled.reverse();
    shuffled.rotate_left(37);
    let draws = Sampled::default().draws;
    for (label, g) in [
        ("no extras", Box::new(Sampled::default()) as Box<dyn RouteSetGenerator>),
        ("two classes", given(&[], two_classes(&net, draws))),
    ] {
        let reference = RouteSets::generate(&net, &turns, &keys, g.as_ref());
        assert_eq!(
            reference,
            RouteSets::generate(&net, &turns, &shuffled, g.as_ref()),
            "{label}: key order"
        );
        assert_eq!(reference, RouteSets::generate(&net, &turns, &keys, g.as_ref()), "{label}");
        #[cfg(feature = "parallel")]
        for threads in [1, 7] {
            let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().expect("pool");
            let made = pool.install(|| RouteSets::generate(&net, &turns, &keys, g.as_ref()));
            assert_eq!(reference, made, "{label}: {threads} threads");
        }
    }
    let seeded = |seed: f64| {
        let g = Sampled::from_options(&opts(&[("seed", seed)])).expect("valid");
        RouteSets::generate(&net, &turns, &keys, &g)
    };
    assert_eq!(seeded(7.0), seeded(7.0));
    assert_ne!(seeded(0.0).links(), seeded(1.0).links(), "another seed, other draws");
    assert_ne!(seeded(0.0).identity(), seeded(1.0).identity());
}

// --- descriptor and identity -----------------------------------------------------------------

#[test]
fn the_descriptor_names_every_option_and_the_extras_by_their_values() {
    const DEFAULT: &str = "sampled;draws=4;extras=none;max_detour=1.5;max_overlap=0.9;\
                           max_paths=10;seed=0;sigma=0.5;tau=0.3";
    assert_eq!(Sampled::default().descriptor(), DEFAULT);
    assert_eq!(Sampled::from_options(&Options::new()).expect("valid").descriptor(), DEFAULT);
    assert_eq!(Sampled::default().name(), "sampled");
    assert_eq!(
        Sampled::default().extras_wanted(),
        Some(ExtrasRequest { draws: 4, tau: 0.3, seed: 0 })
    );
    let other = Sampled::from_options(&opts(&[("draws", 2.0), ("tau", 0.1), ("seed", 9.0)]))
        .expect("valid");
    assert_eq!(other.extras_wanted(), Some(ExtrasRequest { draws: 2, tau: 0.1, seed: 9 }));

    let net = diamond();
    let n = net.link_count() as usize;
    let field = |g: &dyn RouteSetGenerator| {
        let d = g.descriptor();
        let start = d.find("extras=").expect("an extras field") + "extras=".len();
        let end = start + d[start..].find(';').expect("a next field");
        (d[start..end].to_string(), d.replacen(&d[start..end], "none", 1))
    };
    let values = |v: f64| -> Arc<[f64]> { vec![v; n].into() };
    let a = given(&[], vec![vec![Some(values(1.0))]]);
    let (name_a, rest) = field(a.as_ref());
    assert!(name_a.starts_with("given:") && name_a.len() == "given:".len() + 16, "{name_a}");
    assert!(name_a["given:".len()..].chars().all(|c| c.is_ascii_hexdigit()), "{name_a}");
    assert_eq!(rest, DEFAULT, "only the extras field changes");
    assert_eq!(field(given(&[], vec![vec![Some(values(1.0))]]).as_ref()).0, name_a, "same values");
    let mut one_link = vec![1.0; n];
    one_link[n - 1] = 1.5;
    for (label, extras) in [
        ("another value", vec![vec![Some(Arc::<[f64]>::from(one_link))]]),
        ("time alone", vec![vec![None]]),
        ("another class", vec![vec![Some(values(1.0))], vec![None]]),
        ("another draw", vec![vec![Some(values(1.0)), None]]),
    ] {
        assert_ne!(field(given(&[], extras).as_ref()).0, name_a, "{label}");
    }

    // Preparing for a context changes nothing but speed.
    let (grid, _) = manhattan_grid(7, 200.0, true);
    let turns = turns_of(&grid);
    let ctx = SearchContext::new(&grid, &turns);
    let keys = grid_keys(&grid, 7, 80, 13);
    let g = given(&[], two_classes(&grid, 4));
    let prepared = g.with_context(&ctx).expect("it prepares");
    assert_eq!(prepared.descriptor(), g.descriptor());
    let store = RouteSets::generate_in(&ctx, &keys, g.as_ref());
    assert_eq!(store, RouteSets::generate_in(&ctx, &keys, prepared.as_ref()));
    // Directly on a search, one origin's keys at a time: the store's routes, prepared or not.
    let mut sorted = keys.clone();
    sorted.sort_unstable();
    sorted.dedup();
    let mut search = Search::new(&ctx);
    let mut direct: Vec<Vec<Route>> = Vec::new();
    for group in sorted.chunk_by(|a, b| a.origin == b.origin) {
        let destinations: Vec<NodeId> = group.iter().map(|k| NodeId::new(k.destination)).collect();
        let origin = NodeId::new(group[0].origin);
        let made = prepared.generate_from(&mut search, origin, &destinations);
        assert_eq!(made, g.generate_from(&mut search, origin, &destinations), "unprepared");
        direct.extend(made);
    }
    assert_eq!(direct.len(), store.keys().len());
    for (i, routes) in direct.iter().enumerate() {
        let stored: Vec<Vec<u32>> = store.routes(i).map(|r| r.links.to_vec()).collect();
        let made: Vec<Vec<u32>> =
            routes.iter().map(|r| r.links.iter().map(|l| l.raw()).collect()).collect();
        assert_eq!(made, stored);
    }
    // And the extras matter: the sets differ from those by time alone.
    let plain = RouteSets::generate_in(&ctx, &keys, &Sampled::default());
    assert_ne!(plain.links(), store.links());
}

// --- refusals --------------------------------------------------------------------------------

#[test]
fn the_method_is_chosen_by_name_and_bad_options_say_what_is_wrong() {
    assert_eq!(
        Registry::builtin().names(),
        ["penalty", "shortest", "montecarlo", "sampled"],
        "after montecarlo"
    );
    let g = generator("sampled", &opts(&[("draws", 64.0), ("max_paths", 32.0), ("seed", 42.0)]))
        .expect("built in");
    assert_eq!(g.name(), "sampled");
    assert_eq!(
        g.descriptor(),
        "sampled;draws=64;extras=none;max_detour=1.5;max_overlap=0.9;max_paths=32;seed=42;\
         sigma=0.5;tau=0.3"
    );
    let zero = generator("sampled", &opts(&[("draws", 0.0), ("sigma", 0.0), ("tau", 0.0)]))
        .expect("0 draws, no noise and no spread are allowed");
    assert!(zero.descriptor().contains("draws=0;"));
    let unknown = generator("sampled", &opts(&[("nope", 1.0)])).err().expect("refused");
    match &unknown {
        RouteError::UnknownOption { option, known, .. } => {
            assert_eq!(option, "nope");
            assert_eq!(
                known,
                &["draws", "max_detour", "max_overlap", "max_paths", "seed", "sigma", "tau"]
            );
        }
        other => panic!("{other:?}"),
    }
    assert!(unknown.to_string().contains("max_overlap"), "{unknown}");
    for (option, value, range) in [
        ("draws", 65.0, "from 0 to 64"),
        ("draws", -1.0, "from 0 to 64"),
        ("draws", 1.5, "from 0 to 64"),
        ("sigma", -1.0, "at least 0"),
        ("sigma", f64::NAN, "at least 0"),
        ("tau", -0.1, "at least 0"),
        ("max_detour", 0.5, "at least 1"),
        ("max_overlap", 0.0, "above 0 and at most 1"),
        ("max_overlap", 1.1, "above 0 and at most 1"),
        ("max_paths", 0.0, "from 1 to 32"),
        ("max_paths", 33.0, "from 1 to 32"),
        ("seed", -1.0, "from 0 to"),
    ] {
        match generator("sampled", &opts(&[(option, value)])).err() {
            Some(err @ RouteError::BadOption { .. }) => {
                let RouteError::BadOption { option: named, method, reason } = &err else {
                    unreachable!()
                };
                assert_eq!((named.as_str(), method.as_str()), (option, "sampled"), "{value}");
                assert!(reason.contains(range), "{option}={value}: {reason}");
            }
            other => panic!("{option}={value}: {other:?}"),
        }
    }
}

/// **A candidate is judged in the cost it was searched by** (S299): on the diamond, under
/// draws whose noise makes the top road dearer, the bottom road (1.2 times the top's time) is
/// kept when the class's extras on it leave its generalised cost within the detour of the
/// anchor's (1.4 times the top's time), and refused when they take it past (3.2 times), though
/// its travel time alone is within the detour either way.
#[test]
fn a_candidate_is_kept_by_its_generalised_cost_within_the_detour() {
    let net = diamond();
    let turns = turns_of(&net);
    let ctx = SearchContext::new(&net, &turns);
    let top_time =
        net.free_flow_time(link(&net, "ab")).get() + net.free_flow_time(link(&net, "bd")).get();
    let sets_with = |each: f64| {
        let mut extra = vec![0.0; net.link_count() as usize];
        extra[link(&net, "ac").index()] = each * top_time;
        extra[link(&net, "cd").index()] = each * top_time;
        let generator = Sampled::from_options(&opts(&[("draws", 16.0), ("sigma", 20.0)]))
            .expect("valid")
            .with_extras(vec![vec![Some(extra.into()); 17]])
            .expect("takes extras");
        RouteSets::generate_in(
            &ctx,
            &[RouteKey::new(node(&net, "A"), node(&net, "D"))],
            generator.as_ref(),
        )
    };
    assert_eq!(sets_with(0.1).route_count(), 2, "1.4 times the anchor's cost: kept");
    assert_eq!(sets_with(1.0).route_count(), 1, "3.2 times the anchor's cost: refused");
}
