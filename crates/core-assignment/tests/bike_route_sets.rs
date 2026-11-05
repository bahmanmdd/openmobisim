//! Sampled bike route sets ([`BikeRoutes::Sampled`]): a bike leg's set of routes per pair, made
//! once by `core-routes`' doubly stochastic generator, each route a chain of the trip's choice
//! set. Checked against values derived by hand from each case's constants and against
//! properties that must hold; never against a run's output.
//!
//! | Test | Case | What is derived |
//! |---|---|---|
//! | 1 | three routes from `o` to `d` that share no link: a 3.0 km cycleway, 2.6 km and 2.8 km of residential street | the set holds the three, best first; the logit's probabilities `exp(V_k) / Σ exp(V_j)` with `V = −0.2 · minutes − 0.16 · km in mixed traffic` (path size 1: no shared link), and the shares of 6 000 riders within three standard deviations of them |
//! | 2 | the same, trips choosing between the car and the bike | every bike route within the class's longest ride is a chain of the set, the slower one left out; a limit below every route leaves the bike out, counted as too long |
//! | 3 | cars on two roads, bikes with sets of routes on the cycleway and the roads | **property**: adding the bike trips changes no car result, to the bit |
//! | 4 | a 6 × 6 grid of equal blocks, bike, walking and car trips | **property**: the same run on 1, 2, 5 and every thread, and twice, to the bit |
//! | 5 | the grid with every node and link renamed and listed in another order | **property**: every bike trip's set, its order, the route taken and the probabilities are the same street for street |
//! | 6 | the three routes | a state rebuilt from what a run wrote equals it; the shortest rule is the default |

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::float_cmp,
    reason = "test fixtures: small counts, and exact comparisons of values that must be equal to the bit"
)]

use openmobisim_core_assignment::{
    BikeRoutes, ClassModes, Coefficients, FreeFlow, HighestUtility, Inputs, IterationReport, Layer,
    Layers, Loaded, Loader, NoHooks, Params, PointQueue, SampledRoutes, SavedTrip, StageHook,
    State, assign, bike_route_sets, left_out, logit, restore,
};
use openmobisim_core_choice::ChoiceModel;
use openmobisim_core_demand::{
    ClassDefaults, Mode, Ownership, RawTrip, Travellers, Trips, build_travellers,
};
use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::layers::{StaticLayer, StaticLayerDefaults, StaticNetwork};
use openmobisim_core_graph::network::{LinkSpec, RoadNetwork, RoadNetworkBuilder};
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_loading::Vehicle;
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::{EntityId, LinkId, TripId};
use openmobisim_core_types::rng::RngKey;
use openmobisim_core_types::time::Second;

// --- the cases' constants (the shipped layer defaults, `StaticLayerDefaults::SHIPPED`) ---------

/// A bike's speed on a cycleway (dedicated infrastructure), km/h.
const BIKE_DEDICATED_KM_H: f64 = 18.0;
/// A bike's speed in mixed traffic, km/h (below a residential street's 30 km/h, so not capped).
const BIKE_MIXED_KM_H: f64 = 15.0;
/// The logit's weight on a minute riding (its utility, so −0.2) and on a kilometre ridden in
/// mixed traffic on top of its time (−0.16): the other engine's logit defaults. As costs, the
/// opposite sign.
const COST_PER_MINUTE: f64 = 0.2;
const COST_PER_MIXED_KM: f64 = 0.16;

// --- helpers --------------------------------------------------------------------------------

/// A road network of `nodes` (name, metres east, metres north) and `links` (name, from, to,
/// metres, class), each link's length stated.
fn network(
    nodes: &[(&str, f64, f64)],
    links: &[(&str, &str, &str, f64, RoadClass)],
) -> RoadNetwork {
    let mut b = RoadNetworkBuilder::new();
    for &(n, x, y) in nodes {
        b.add_node(n, LonLat::new(4.8 + x / 77_800.0, 45.7 + y / 110_574.0));
    }
    for &(name, from, to, metres, class) in links {
        let mut spec = LinkSpec::new(class);
        spec.length_m = Some(metres);
        b.add_link(name, from, to, spec);
    }
    b.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
        .expect("buildable")
}

/// A case: the road network, its two layers derived from it, and the demand.
struct Case {
    net: RoadNetwork,
    turns: TurnTable,
    bike: StaticNetwork,
    walk: StaticNetwork,
    bike_turns: TurnTable,
    walk_turns: TurnTable,
    travellers: Travellers,
    trips: Trips,
    classes: Vec<Coefficients>,
    modes: Vec<ClassModes>,
    money: Vec<f64>,
}

/// A trip of traveller `id` from node `from` to node `to` of `net`, of `mode` (`None`: not
/// stated).
fn trip(
    net: &RoadNetwork,
    id: &str,
    from: &str,
    to: &str,
    departure: u32,
    mode: Option<Mode>,
) -> RawTrip {
    let at = |name: &str| {
        net.node_lonlat(net.node_external_ids().typed_id_of(name).expect("a node of the case"))
    };
    RawTrip {
        traveller_id: id.to_string(),
        trip_seq: 0,
        origin: at(from),
        destination: at(to),
        departure_time: Second(departure),
        user_class: "everyone".to_string(),
        weight: Some(1),
        mode,
    }
}

fn case(net: RoadNetwork, trips: Vec<RawTrip>) -> Case {
    let owners = ClassDefaults::new().with_fallback(Ownership::VEHICLES);
    let (travellers, trips) =
        build_travellers(trips, Vec::new(), &owners, 1, &mut Diagnostics::new()).expect("demand");
    let derive = |layer| {
        StaticNetwork::derive(&net, layer, StaticLayerDefaults::SHIPPED, &mut Diagnostics::new())
            .expect("a layer")
    };
    let (bike, walk) = (derive(StaticLayer::Bike), derive(StaticLayer::Walk));
    let turns_of = |l: &StaticNetwork| TurnTable::build(l.network(), l.network().signals());
    Case {
        turns: TurnTable::build(&net, SignalDefaults::SHIPPED),
        bike_turns: turns_of(&bike),
        walk_turns: turns_of(&walk),
        bike,
        walk,
        money: vec![0.0; net.link_count() as usize],
        net,
        travellers,
        trips,
        classes: vec![Coefficients {
            in_vehicle_per_minute: COST_PER_MINUTE,
            walk_per_minute: COST_PER_MINUTE,
            wait_per_minute: COST_PER_MINUTE,
            parking_per_minute: COST_PER_MINUTE,
            bike_mixed_per_kilometre: COST_PER_MIXED_KM,
            ..Coefficients::time_based()
        }],
        modes: Vec::new(),
    }
}

fn layers(c: &Case) -> Layers<'_> {
    Layers {
        bike: Some(Layer { network: &c.bike, turns: &c.bike_turns }),
        walk: Some(Layer { network: &c.walk, turns: &c.walk_turns }),
    }
}

fn inputs<'a>(
    c: &'a Case,
    layers: Layers<'a>,
    model: &'a dyn ChoiceModel,
    loader: &'a dyn Loader,
) -> Inputs<'a> {
    Inputs {
        network: &c.net,
        turns: &c.turns,
        layers,
        transit: None,
        travellers: &c.travellers,
        trips: &c.trips,
        classes: &c.classes,
        mode_choice: &c.modes,
        money_per_link: &c.money,
        model,
        loader,
        seed: RngKey::from_seed(11),
        initial: None,
    }
}

/// The generator's options of V-B1: 32 draws, a noise of 3 (`sigma`), the rest its defaults.
fn sampled() -> SampledRoutes {
    SampledRoutes { draws: 32, sigma: 3.0, ..SampledRoutes::default() }
}

/// Exactly `iterations` iterations, with sampled bike route sets.
fn params(iterations: u32) -> Params {
    Params {
        min_iterations: iterations,
        max_iterations: iterations,
        bike_routes: BikeRoutes::Sampled(sampled()),
        ..Params::default()
    }
}

/// What every loading was given and returned.
#[derive(Default)]
struct Recorder {
    vehicles: Vec<Vec<(Vec<LinkId>, u32)>>,
    arrivals: Vec<Vec<Option<f64>>>,
}

impl StageHook for Recorder {
    fn before_load(&mut self, _iteration: u32, _state: &State, vehicles: &[Vehicle]) {
        self.vehicles.push(vehicles.iter().map(|v| (v.route.clone(), v.departure.get())).collect());
    }

    fn after_load(&mut self, _iteration: u32, loaded: &Loaded) {
        self.arrivals.push(loaded.arrival.clone());
    }
}

fn run(
    c: &Case,
    layers: Layers<'_>,
    model: &dyn ChoiceModel,
    loader: &dyn Loader,
    params: &Params,
) -> (State, Vec<IterationReport>, Recorder) {
    let mut recorder = Recorder::default();
    let (state, reports) =
        assign(&inputs(c, layers, model, loader), params, &mut recorder).expect("the run");
    (state, reports, recorder)
}

/// The names of `links` on `graph`.
fn names(graph: &RoadNetwork, links: &[LinkId]) -> Vec<String> {
    links.iter().map(|&l| graph.link_external_ids().external(l.raw()).to_string()).collect()
}

/// The names of the links of every bike chain of `trip`'s set, in the set's order.
fn bike_routes(c: &Case, state: &State, trip: usize) -> Vec<Vec<String>> {
    state
        .trip(TripId::from_index(trip))
        .set
        .iter()
        .filter(|ch| ch.mode() == Mode::Bike)
        .map(|ch| names(c.bike.network(), &ch.legs()[0].links))
        .collect()
}

// --- three routes that share no link -------------------------------------------------------------

/// From `o` to `d`: **A**, a cycleway through `a` (2 × 1 500 m); **B**, a residential street
/// through `b` (2 × 1 300 m); **C**, a residential street through `c` (2 × 1 400 m). The streets
/// one way, so a car drives `o–b–d` or `o–c–d`.
fn three_routes() -> RoadNetwork {
    network(
        &[
            ("o", 0.0, 0.0),
            ("a", 1250.0, 900.0),
            ("b", 1250.0, -450.0),
            ("c", 1250.0, -1000.0),
            ("d", 2500.0, 0.0),
        ],
        &[
            ("oa", "o", "a", 1500.0, RoadClass::Cycleway),
            ("ad", "a", "d", 1500.0, RoadClass::Cycleway),
            ("ob", "o", "b", 1300.0, RoadClass::Residential),
            ("bd", "b", "d", 1300.0, RoadClass::Residential),
            ("oc", "o", "c", 1400.0, RoadClass::Residential),
            ("cd", "c", "d", 1400.0, RoadClass::Residential),
        ],
    )
}

/// The three routes' minutes riding and kilometres in mixed traffic, derived from the case:
/// A 3.0 km at 18 km/h = 10.0 min, none mixed; B 2.6 km at 15 km/h = 10.4 min, 2.6 km mixed;
/// C 2.8 km at 15 km/h = 11.2 min, 2.8 km mixed.
fn derived_routes() -> [(f64, f64); 3] {
    [
        (3.0 / BIKE_DEDICATED_KM_H * 60.0, 0.0),
        (2.6 / BIKE_MIXED_KM_H * 60.0, 2.6),
        (2.8 / BIKE_MIXED_KM_H * 60.0, 2.8),
    ]
}

/// The logit's probabilities of A, B and C: `exp(−cost_k) / Σ exp(−cost_j)`, the cost
/// `0.2 · minutes + 0.16 · km in mixed traffic` (A 2.000, B 2.496, C 2.688), the path size 1
/// (no shared link): P = 0.47359, 0.28840, 0.23802.
fn derived_probabilities() -> [f64; 3] {
    let e = derived_routes().map(|(m, km)| (-(COST_PER_MINUTE * m + COST_PER_MIXED_KM * km)).exp());
    let sum: f64 = e.iter().sum();
    e.map(|x| x / sum)
}

const RIDERS: usize = 6000;

/// 6 000 stated bike trips from `o` to `d`, one a second, by the logit: the set holds A, B and
/// C, best first (A, then B, then C by the class's generalised cost); the model gives each the
/// derived probability (to 1e-12), and the riders' shares lie within three standard deviations
/// `3 √(P (1 − P) / 6 000)` of it. The sets the run made are those `bike_route_sets` gives.
#[test]
fn three_disjoint_routes_split_as_the_logit_says() {
    let net = three_routes();
    let rows: Vec<RawTrip> = (0..RIDERS)
        .map(|i| trip(&net, &format!("r{i:04}"), "o", "d", 25_200 + i as u32, Some(Mode::Bike)))
        .collect();
    let c = case(three_routes(), rows);
    let model = logit();
    let loader = FreeFlow::default();
    let (state, reports, recorder) = run(&c, layers(&c), &model, &loader, &params(1));
    let p = derived_probabilities();
    let expected: Vec<Vec<String>> = [["oa", "ad"], ["ob", "bd"], ["oc", "cd"]]
        .iter()
        .map(|r| r.iter().map(|s| (*s).to_string()).collect())
        .collect();
    let mut counts = [0_usize; 3];
    for t in 0..RIDERS {
        assert_eq!(bike_routes(&c, &state, t), expected, "trip {t}: A, B and C, best first");
        let entry = state.trip(TripId::from_index(t));
        for (k, (&got, want)) in entry.probabilities.iter().zip(p).enumerate() {
            assert!((got - want).abs() < 1e-12, "trip {t}, route {k}: {got} against {want}");
        }
        counts[entry.current.expect("a route")] += 1;
    }
    let mut lines = Vec::new();
    for k in 0..3 {
        let share = counts[k] as f64 / RIDERS as f64;
        let sigma = (p[k] * (1.0 - p[k]) / RIDERS as f64).sqrt();
        assert!((share - p[k]).abs() <= 3.0 * sigma, "route {k}: share {share}, derived {}", p[k]);
        lines.push(format!("{share:.4} (derived {:.4}, 3σ {:.4})", p[k], 3.0 * sigma));
    }
    assert!(recorder.vehicles.iter().all(Vec::is_empty), "a bike loads no road link");
    // A stated bike trip's choice probabilities never change, so it stays out of the stopping
    // shares and the gaps, as a trip of one chain.
    assert_eq!((reports[0].choosing, reports[0].added, reports[0].duality_gap), (0.0, 0, 0.0));
    // The sets the run made.
    let i = inputs(&c, layers(&c), &model, &loader);
    let sets = bike_route_sets(&i, &params(1)).expect("the sets").expect("sampled sets");
    assert_eq!((sets.keys().len(), sets.route_count()), (1, 3), "one pair, three routes");
    let first: Vec<Vec<String>> = sets
        .routes(0)
        .map(|r| {
            let links: Vec<LinkId> = r.links.iter().map(|&l| LinkId::new(l)).collect();
            names(c.bike.network(), &links)
        })
        .collect();
    assert_eq!(first, expected, "the run's sets, in their order");
    println!("shares on A, B, C: {}", lines.join(", "));
}

/// Trips choosing between the car and the bike (the deterministic choice: the costs decide).
/// With the longest ride 650 s, A (600 s) and B (624 s) are in the set and C (672 s) is not: the
/// set is the car's route, then A and B. With 590 s no route is short enough: the bike is left
/// out of the set, counted as too long.
#[test]
fn a_trip_choosing_its_mode_has_every_bike_route_within_its_limit() {
    let net = three_routes();
    let rows = vec![trip(&net, "m0", "o", "d", 25_200, None)];
    for (limit, bikes, too_long) in [(650.0, 2, 0), (590.0, 0, 1)] {
        let mut c = case(three_routes(), rows.clone());
        c.modes = vec![ClassModes {
            bike_max_seconds: limit,
            ..ClassModes::unlimited(&[Mode::Car, Mode::Bike])
        }];
        let loader = FreeFlow::default();
        let p = params(1);
        let (state, _, _) = run(&c, layers(&c), &HighestUtility, &loader, &p);
        let routes = bike_routes(&c, &state, 0);
        let want: Vec<Vec<String>> = [["oa", "ad"], ["ob", "bd"]][..bikes]
            .iter()
            .map(|r| r.iter().map(|s| (*s).to_string()).collect())
            .collect();
        assert_eq!(routes, want, "the bike routes within {limit} s");
        let set = &state.trip(TripId::new(0)).set;
        assert_eq!(set[0].mode(), Mode::Car, "the car's chain first");
        assert_eq!(set.len(), 1 + bikes);
        let left = left_out(&inputs(&c, layers(&c), &HighestUtility, &loader), &p, &state)
            .expect("counted");
        assert_eq!(left.bike_too_long, too_long, "left out as too long");
    }
}

// --- bikes beside cars ----------------------------------------------------------------------------

/// Cars from `o` to `d` on two roads (`o–c–d`, a secondary road of 2 × 1 000 m, and `o–b–d`, a
/// residential street of 2 × 900 m), and bikes, whose sets hold the cycleway `o–a–d` and the
/// roads (a bike may ride both, in mixed traffic).
fn roads_and_cycleway() -> RoadNetwork {
    network(
        &[
            ("o", 0.0, 0.0),
            ("a", 1000.0, 600.0),
            ("b", 1000.0, -400.0),
            ("c", 1000.0, -900.0),
            ("d", 1800.0, 0.0),
        ],
        &[
            ("oa", "o", "a", 1000.0, RoadClass::Cycleway),
            ("ad", "a", "d", 1000.0, RoadClass::Cycleway),
            ("ob", "o", "b", 900.0, RoadClass::Residential),
            ("bd", "b", "d", 900.0, RoadClass::Residential),
            ("oc", "o", "c", 1000.0, RoadClass::Secondary),
            ("cd", "c", "d", 1000.0, RoadClass::Secondary),
        ],
    )
}

/// 1 200 cars over 20 minutes, and (with `bikes`) 300 bikes between them, the bikes' travellers
/// named to sort after the cars' so that every car trip keeps its index.
fn cars_and_bikes(bikes: bool) -> Case {
    let net = roads_and_cycleway();
    let mut rows: Vec<RawTrip> =
        (0..1200).map(|i| trip(&net, &format!("car{i:04}"), "o", "d", i, None)).collect();
    if bikes {
        rows.extend(
            (0..300)
                .map(|i| trip(&net, &format!("zbike{i:03}"), "o", "d", 4 * i, Some(Mode::Bike))),
        );
    }
    case(roads_and_cycleway(), rows)
}

/// **Property**: bikes with sets of routes change no car result when no trip chooses its mode.
/// The same cars with and without 300 bike trips, five iterations by the point queue, by the
/// deterministic choice and the logit: every car trip's set, chain and probabilities, the link
/// travel times, and every vehicle loaded and its arrival are equal to the bit, while each bike
/// trip has a set of more than one route.
#[test]
fn bikes_with_route_sets_change_no_car_result() {
    let (alone, with) = (cars_and_bikes(false), cars_and_bikes(true));
    let n = alone.trips.len() as usize;
    let the_logit = logit();
    for model in [&HighestUtility as &dyn ChoiceModel, &the_logit] {
        let (la, lw) =
            (PointQueue::new(&alone.turns, 7200.0), PointQueue::new(&with.turns, 7200.0));
        let (a, _, reca) = run(&alone, Layers::NONE, model, &la, &params(5));
        let (w, _, recw) = run(&with, layers(&with), model, &lw, &params(5));
        for t in 0..n {
            assert!(a.trip(TripId::from_index(t)) == w.trip(TripId::from_index(t)), "car trip {t}");
        }
        assert_eq!(a.link_times().rows(), w.link_times().rows(), "the link travel times");
        assert_eq!(reca.vehicles, recw.vehicles, "every vehicle loaded");
        assert_eq!(reca.arrivals, recw.arrivals, "every arrival");
        let sizes: Vec<usize> =
            (n..with.trips.len() as usize).map(|t| bike_routes(&with, &w, t).len()).collect();
        assert!(sizes.iter().all(|&k| k > 1), "every bike trip has a set of routes: {sizes:?}");
        println!("{}: car results equal; bike sets of {} routes", model.name(), sizes[0]);
    }
}

// --- the grid: threads and names ------------------------------------------------------------------

/// A 6 × 6 grid of 200 m residential blocks, two-way, its nodes named by `node` and its links by
/// `link` (the link's index in a fixed listing), added in the order `order` gives.
fn grid(
    node: &dyn Fn(u32, u32) -> String,
    link: &dyn Fn(usize) -> String,
    order: &[usize],
) -> RoadNetwork {
    let n = 6;
    let mut b = RoadNetworkBuilder::new();
    for r in 0..n {
        for c in 0..n {
            let (x, y) = (f64::from(c) * 200.0, f64::from(r) * 200.0);
            b.add_node(node(r, c), LonLat::new(4.8 + x / 77_800.0, 45.7 + y / 110_574.0));
        }
    }
    let mut pairs = Vec::new();
    for r in 0..n {
        for c in 0..n {
            if c + 1 < n {
                pairs.push(((r, c), (r, c + 1)));
                pairs.push(((r, c + 1), (r, c)));
            }
            if r + 1 < n {
                pairs.push(((r, c), (r + 1, c)));
                pairs.push(((r + 1, c), (r, c)));
            }
        }
    }
    for &i in order {
        let ((r1, c1), (r2, c2)) = pairs[i];
        let mut spec = LinkSpec::new(RoadClass::Residential);
        spec.length_m = Some(200.0);
        b.add_link(link(i), node(r1, c1), node(r2, c2), spec);
    }
    b.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
        .expect("buildable")
}

/// The number of links of the grid.
const GRID_LINKS: usize = 2 * 2 * 6 * 5;

/// A fixed shuffle of `0..n` (a linear congruential walk; any permutation would do).
fn shuffled(n: usize, seed: u64) -> Vec<usize> {
    let mut v: Vec<usize> = (0..n).collect();
    let mut x = seed;
    for i in (1..n).rev() {
        x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        v.swap(i, (x >> 33) as usize % (i + 1));
    }
    v
}

/// 600 trips on the grid between nodes taken in a fixed pattern: bikes, walks and cars by turns.
fn grid_case(
    node: &dyn Fn(u32, u32) -> String,
    link: &dyn Fn(usize) -> String,
    order: &[usize],
) -> Case {
    let net = grid(node, link, order);
    let trips = (0..600_u32)
        .map(|i| {
            let (a, b) = ((i * 7) % 36, (i * 13 + 5) % 36);
            let mode = [Some(Mode::Bike), Some(Mode::Walk), None][i as usize % 3];
            trip(&net, &format!("g{i:03}"), &node(a / 6, a % 6), &node(b / 6, b % 6), 3 * i, mode)
        })
        .collect();
    case(grid(node, link, order), trips)
}

fn plain_node(r: u32, c: u32) -> String {
    format!("n{r}{c}")
}

fn plain_link(i: usize) -> String {
    format!("l{i:03}")
}

/// **Property**: the same inputs give the same run, to the bit, on 1, 2, 5 and every thread, and
/// twice on one, with sampled bike sets (made in parallel per origin) and the logit.
#[test]
fn one_thread_and_several_give_the_same_run_with_route_sets() {
    let order: Vec<usize> = (0..GRID_LINKS).collect();
    let c = grid_case(&plain_node, &plain_link, &order);
    let loader = PointQueue::new(&c.turns, 7200.0);
    let the_logit = logit();
    let runs: Vec<(State, Vec<IterationReport>)> = [1, 1, 2, 5, 0]
        .iter()
        .map(|&threads| {
            let pool =
                rayon::ThreadPoolBuilder::new().num_threads(threads).build().expect("a pool");
            pool.install(|| {
                let (s, r, _) = run(&c, layers(&c), &the_logit, &loader, &params(4));
                (s, r)
            })
        })
        .collect();
    let (one, rest) = runs.split_first().expect("runs");
    for (k, other) in rest.iter().enumerate() {
        assert!(one.0 == other.0, "the state differs from one thread's (run {k})");
        let a: Vec<IterationReport> = one.1.iter().map(IterationReport::without_times).collect();
        let b: Vec<IterationReport> = other.1.iter().map(IterationReport::without_times).collect();
        assert_eq!(a, b, "the reports differ from one thread's (run {k})");
    }
    let sizes: Vec<usize> = (0..600).step_by(3).map(|t| bike_routes(&c, &one.0, t).len()).collect();
    let mean = sizes.iter().sum::<usize>() as f64 / sizes.len() as f64;
    assert!(sizes.iter().any(|&k| k > 1), "some bike trip has a set of routes");
    println!("1, 1, 2, 5 and all threads agree; {mean:.2} bike routes per bike trip");
}

/// **Property**: renaming every node and link (both layers take the road network's names) and
/// listing the links in another order changes no bike trip's set: its routes, read as the
/// sequence of their links' end points, in the set's order, the route taken and the logit's
/// probabilities are the same, although the link numbers differ. The noise on a link is drawn
/// by its coordinates and length, a set's draws by its origin's coordinates, and equal routes
/// are ordered by the streets, so nothing reads an id.
#[test]
fn renaming_the_layers_changes_no_route_set() {
    let order: Vec<usize> = (0..GRID_LINKS).collect();
    let plain = grid_case(&plain_node, &plain_link, &order);
    let perm = shuffled(GRID_LINKS, 11);
    let node_perm = shuffled(36, 5);
    let renamed_node = |r: u32, c: u32| format!("x{:02}", node_perm[(r * 6 + c) as usize]);
    let renamed_link = |i: usize| format!("k{:03}", perm[i]);
    let other_order = shuffled(GRID_LINKS, 3);
    let renamed = grid_case(&renamed_node, &renamed_link, &other_order);
    let loader = FreeFlow::default();
    let the_logit = logit();
    let p = params(1);
    let (a, _, _) = run(&plain, layers(&plain), &the_logit, &loader, &p);
    let (b, _, _) = run(&renamed, layers(&renamed), &the_logit, &loader, &p);
    let ends = |c: &Case, links: &[LinkId]| -> Vec<(i64, i64)> {
        let g = c.bike.network();
        let at = |n| {
            let p = g.node_lonlat(n);
            ((p.lon * 1e7).round() as i64, (p.lat * 1e7).round() as i64)
        };
        let mut out: Vec<(i64, i64)> = links.iter().map(|&l| at(g.link_from(l))).collect();
        out.extend(links.last().map(|&l| at(g.link_to(l))));
        out
    };
    let (mut routes, mut differ) = (0, 0);
    for t in (0..plain.trips.len() as usize).step_by(3) {
        let trip = TripId::from_index(t);
        let (x, y) = (a.trip(trip), b.trip(trip));
        assert_eq!(x.set.len(), y.set.len(), "trip {t}: the set's size");
        for (cx, cy) in x.set.iter().zip(&y.set) {
            let (lx, ly) = (&cx.legs()[0].links, &cy.legs()[0].links);
            assert_eq!(ends(&plain, lx), ends(&renamed, ly), "trip {t}: the set's order");
            routes += 1;
            if lx != ly {
                differ += 1;
            }
        }
        assert_eq!(x.current, y.current, "trip {t}: the route taken");
        assert_eq!(x.probabilities, y.probabilities, "trip {t}: the probabilities");
    }
    assert!(routes > 200 && differ > 0, "{routes} routes compared, {differ} numbered otherwise");
    println!(
        "{routes} bike routes the same street for street and in the same order; {differ} numbered otherwise"
    );
}

// --- a run rebuilt, and the default -------------------------------------------------------------------

/// A state rebuilt from what a run with sampled sets wrote equals it; one route per pair is the
/// default.
#[test]
fn a_rebuilt_state_with_route_sets_equals_the_run() {
    assert_eq!(Params::default().bike_routes, BikeRoutes::Shortest, "one route: the default");
    let net = three_routes();
    let rows: Vec<RawTrip> = (0..40)
        .map(|i| trip(&net, &format!("r{i:02}"), "o", "d", 25_200 + i, Some(Mode::Bike)))
        .chain((0..40).map(|i| trip(&net, &format!("s{i:02}"), "o", "d", 25_200 + i, None)))
        .collect();
    let c = case(three_routes(), rows);
    let model = logit();
    let p = params(3);
    let point_queue = PointQueue::new(&c.turns, 36_000.0);
    let i = inputs(&c, layers(&c), &model, &point_queue);
    let (state, _) = assign(&i, &p, &mut NoHooks).expect("the run");
    assert_eq!(state.trip(TripId::new(0)).set.len(), 3, "a bike trip's three routes");
    let saved: Vec<SavedTrip> = state.trips().iter().map(SavedTrip::of).collect();
    let rebuilt = restore(&i, &p, &saved, state.iterations()).expect("rebuilt");
    assert!(rebuilt == state, "the rebuilt state equals the run's");
}
