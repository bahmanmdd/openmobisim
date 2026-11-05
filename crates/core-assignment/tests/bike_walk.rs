//! Bike and walking trips whose mode is stated, on their own layers, checked against values
//! derived by hand from each case's own constants (lengths, the layers' speeds, the fixed times,
//! the class's coefficients), and against properties that must hold; never against a run's
//! output.
//!
//! | Test | Case | What is derived |
//! |---|---|---|
//! | 1 | a 2 000 m cycleway | a bike trip's door-to-door time `60 + 2 000 / (18 / 3.6) + 60 = 520 s`, its attributes and cost, at levels 0 and 2; unfinished when the window ends before |
//! | 2 | a 1 800 m street | a walk's time `1 800 / (4.8 / 3.6) = 1 350 s`, no fixed time |
//! | 3 | a 2 000 m cycleway against a 1 800 m (and a 1 600 m) mixed street | the route each bike cost chooses, from the costs worked out by hand |
//! | 4 | cars on two roads, bikes on a cycleway | **property**: adding bike trips changes no car result (sets, choices, probabilities, link times, every vehicle and arrival), to the bit |
//! | 5 | a 6 × 6 grid of equal blocks, 600 bike, walk and car trips | **property**: the same run on 1, 2, 5 threads and twice gives the same state and reports, to the bit |
//! | 6 | the grid with every node and link renamed and listed in another order | **property**: every bike and walking route is the same street for street |
//! | 7 | the cycleway | a traveller without a bike has no chain; a bike trip without its layer is refused; a run rebuilt from its saved state equals it |

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::float_cmp,
    reason = "test fixtures: small counts, and exact comparisons of values that must be equal to the bit"
)]

use openmobisim_core_assignment::{
    Coefficients, FixedTimes, FreeFlow, HighestUtility, Inputs, IterationReport, Layer, Layers,
    Loaded, Loader, NoHooks, Params, PointQueue, SavedTrip, StageHook, State, assign, logit,
    restore,
};
use openmobisim_core_choice::ChoiceModel;
use openmobisim_core_demand::{
    ClassDefaults, Mode, Ownership, RawTrip, Travellers, Trips, build_travellers,
};
use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::layers::{BikeCost, StaticLayer, StaticLayerDefaults, StaticNetwork};
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
/// The walking speed, km/h.
const WALK_KM_H: f64 = 4.8;
/// The multiplier the `dedicated` bike cost puts on a minute in mixed traffic.
const MIXED_FACTOR: f64 = 1.2;
/// The bike's fixed times, seconds: reaching and unlocking it, parking and locking it.
const BIKE_ACCESS_S: f64 = 60.0;
const BIKE_EGRESS_S: f64 = 60.0;

fn metres_per_second(km_h: f64) -> f64 {
    km_h / 3.6
}

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

fn case_owning(net: RoadNetwork, trips: Vec<RawTrip>, owning: Ownership) -> Case {
    let owners = ClassDefaults::new().with_fallback(owning);
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
        classes: vec![Coefficients::time_based()],
    }
}

fn case(net: RoadNetwork, trips: Vec<RawTrip>) -> Case {
    case_owning(net, trips, Ownership::VEHICLES)
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
    seed: u64,
) -> Inputs<'a> {
    Inputs {
        network: &c.net,
        turns: &c.turns,
        layers,
        transit: None,
        travellers: &c.travellers,
        trips: &c.trips,
        classes: &c.classes,
        mode_choice: &[],
        money_per_link: &c.money,
        model,
        loader,
        seed: RngKey::from_seed(seed),
        initial: None,
    }
}

/// Exactly `iterations` iterations, the bike's fixed times 60 s and 60 s, routes by `cost`.
fn params(iterations: u32, cost: BikeCost) -> Params {
    Params {
        min_iterations: iterations,
        max_iterations: iterations,
        bike_fixed_times: FixedTimes::new(BIKE_ACCESS_S, BIKE_EGRESS_S),
        bike_cost: cost,
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
        assign(&inputs(c, layers, model, loader, 7), params, &mut recorder).expect("the run");
    (state, reports, recorder)
}

/// The names of `links` on `graph`.
fn names(graph: &RoadNetwork, links: &[LinkId]) -> Vec<String> {
    links.iter().map(|&l| graph.link_external_ids().external(l.raw()).to_string()).collect()
}

/// The layer's time of its link `name`, after checking it is the link's length over `km_h`.
fn checked_seconds(layer: &StaticNetwork, name: &str, metres: f64, km_h: f64) -> f64 {
    let graph = layer.network();
    let l = graph.link_external_ids().typed_id_of::<LinkId>(name).expect("a link of the layer");
    let derived = metres / metres_per_second(km_h);
    let built = layer.link_seconds()[l.index()];
    assert!(
        (built - derived).abs() < 1e-9 && (graph.link_length(l).get() - metres).abs() < 1e-9,
        "{name} on the {} layer: {built} s, derived {derived} s",
        layer.layer().as_str()
    );
    built
}

// --- the cycleway and the street ----------------------------------------------------------------

/// `o` to `d` by a cycleway through `a` (2 × 1 000 m) and by a residential street through `b`
/// (2 × `street_m / 2`); the street both ways, so it can be walked from `o` to `d`.
fn cycleway_and_street(street_m: f64) -> RoadNetwork {
    let half = street_m / 2.0;
    network(
        &[("o", 0.0, 0.0), ("a", 1000.0, 600.0), ("b", 1000.0, -400.0), ("d", 1800.0, 0.0)],
        &[
            ("oa", "o", "a", 1000.0, RoadClass::Cycleway),
            ("ad", "a", "d", 1000.0, RoadClass::Cycleway),
            ("ob", "o", "b", half, RoadClass::Residential),
            ("bd", "b", "d", half, RoadClass::Residential),
            ("bo", "b", "o", half, RoadClass::Residential),
            ("db", "d", "b", half, RoadClass::Residential),
        ],
    )
}

/// One bike trip from `o` to `d` leaving at 1 000 s, at levels 0 and 2. Derived: the ride is
/// 2 × 1 000 m at 18 km/h = 400 s (the cycleway: dedicated infrastructure); the bike sets off at
/// 1 000 + 60 s, arrives at 1 460 s, and is parked and locked at 1 520 s: 520 s door to door.
/// Its attributes: 400 s in the vehicle, 60 s walking, 60 s parking, 2 000 m, none of it in mixed
/// traffic. Its cost at 1, 2 and 3 a minute in the vehicle, walking and parking:
/// 400/60 + 2 × 60/60 + 3 × 60/60 = 11.666…
#[test]
fn a_bike_trip_on_a_cycleway_takes_its_ride_and_its_fixed_times() {
    let net = cycleway_and_street(1800.0);
    let t = trip(&net, "t0", "o", "d", 1000, Some(Mode::Bike));
    let mut c = case(cycleway_and_street(1800.0), vec![t]);
    c.classes = vec![Coefficients {
        in_vehicle_per_minute: 1.0,
        walk_per_minute: 2.0,
        parking_per_minute: 3.0,
        ..Coefficients::time_based()
    }];
    let ride = checked_seconds(&c.bike, "oa", 1000.0, BIKE_DEDICATED_KM_H)
        + checked_seconds(&c.bike, "ad", 1000.0, BIKE_DEDICATED_KM_H);
    assert_eq!(ride, 400.0, "the case is as stated");
    let door = BIKE_ACCESS_S + ride + BIKE_EGRESS_S;
    let point_queue = PointQueue::new(&c.turns, 10_000.0);
    let free_flow = FreeFlow::default();
    for (level, loader) in [(0, &free_flow as &dyn Loader), (2, &point_queue)] {
        let (state, reports, recorder) =
            run(&c, layers(&c), &HighestUtility, loader, &params(2, BikeCost::Utility));
        let chain = state.trip(TripId::new(0)).current_chain().expect("a chain");
        let [leg] = chain.legs() else { panic!("one leg") };
        assert_eq!(leg.mode, Mode::Bike);
        assert_eq!(names(c.bike.network(), &leg.links), ["oa", "ad"]);
        let a = chain.total();
        assert_eq!(
            (a.in_vehicle_seconds, a.walk_seconds, a.parking_seconds),
            (ride, BIKE_ACCESS_S, BIKE_EGRESS_S)
        );
        assert_eq!((a.length_metres, a.bike_mixed_metres), (2000.0, 0.0));
        let cost = c.classes[0].cost(a);
        assert!((cost - (400.0 / 60.0 + 2.0 + 3.0)).abs() < 1e-12, "cost {cost}");
        assert_eq!(leg.vehicle_departure(), 1060.0, "the bike sets off once reached");
        // Nothing is loaded on the road network.
        assert!(recorder.vehicles.iter().all(Vec::is_empty), "a bike loads no road link");
        for r in &reports {
            assert_eq!((r.departed, r.arrived, r.unfinished), (1.0, 1.0, 0.0));
            assert_eq!(r.mean_trip_seconds, door, "level {level}: door to door");
        }
        // Iteration 0 offers the chain, iteration 1 finds nothing new and nobody switches. A
        // stated bike trip could never change its choice, so the stopping shares and their
        // counts leave it out: nothing is counted as added or found, even in iteration 0.
        assert_eq!((reports[0].added, reports[0].found, reports[0].choosing), (0, 0, 0.0));
        assert_eq!((reports[1].added, reports[1].found, reports[1].switch_share), (0, 0, 0.0));
        assert_eq!((reports[1].duality_gap, reports[1].gap_deterministic), (0.0, 0.0));
        println!(
            "level {level}: bike trip {} s door to door (derived {door} s)",
            reports[1].mean_trip_seconds
        );
    }
    // A window that ends at 1 500 s, before the bike is parked at 1 520 s: unfinished.
    let early = PointQueue::new(&c.turns, 1500.0);
    let (_, reports, _) =
        run(&c, layers(&c), &HighestUtility, &early, &params(1, BikeCost::Utility));
    assert_eq!((reports[0].arrived, reports[0].unfinished), (0.0, 1.0));
    let early = FreeFlow { end_seconds: 1500.0 };
    let (_, reports, _) =
        run(&c, layers(&c), &HighestUtility, &early, &params(1, BikeCost::Utility));
    assert_eq!((reports[0].arrived, reports[0].unfinished), (0.0, 1.0));
}

/// One walk from `o` to `d`: the cycleway is not walked, so the street, 2 × 900 m at 4.8 km/h:
/// 1 350 s, all of it walking, no fixed time.
#[test]
fn a_walk_takes_its_length_at_the_walking_speed() {
    let net = cycleway_and_street(1800.0);
    let t = trip(&net, "t0", "o", "d", 1000, Some(Mode::Walk));
    let c = case(cycleway_and_street(1800.0), vec![t]);
    let walk = checked_seconds(&c.walk, "ob", 900.0, WALK_KM_H)
        + checked_seconds(&c.walk, "bd", 900.0, WALK_KM_H);
    let derived = 1800.0 / metres_per_second(WALK_KM_H);
    assert!(
        (walk - derived).abs() < 1e-9 && (derived - 1350.0).abs() < 1e-9,
        "the case is as stated"
    );
    let loader = FreeFlow::default();
    let (state, reports, _) =
        run(&c, layers(&c), &HighestUtility, &loader, &params(1, BikeCost::Time));
    let chain = state.trip(TripId::new(0)).current_chain().expect("a chain");
    let [leg] = chain.legs() else { panic!("one leg") };
    assert_eq!((leg.mode, leg.fixed), (Mode::Walk, FixedTimes::NONE));
    assert_eq!(names(c.walk.network(), &leg.links), ["ob", "bd"]);
    let a = chain.total();
    assert_eq!((a.in_vehicle_seconds, a.parking_seconds, a.walk_seconds), (0.0, 0.0, walk));
    assert!((reports[0].mean_trip_seconds - 1350.0).abs() < 1e-9);
    println!("walk: {} s (derived 1 350 s)", reports[0].mean_trip_seconds);
}

/// The route of a bike trip from `o` to `d` under each bike cost, the cycleway (2 × 1 000 m at
/// 18 km/h: 400 s) against the street (`street_m` at 15 km/h).
fn chosen(street_m: f64, cost: BikeCost, k: Coefficients) -> Vec<String> {
    let net = cycleway_and_street(street_m);
    let t = trip(&net, "t0", "o", "d", 0, Some(Mode::Bike));
    let mut c = case(cycleway_and_street(street_m), vec![t]);
    c.classes = vec![k];
    checked_seconds(&c.bike, "ob", street_m / 2.0, BIKE_MIXED_KM_H);
    let loader = FreeFlow::default();
    let (state, _, _) = run(&c, layers(&c), &HighestUtility, &loader, &params(1, cost));
    let chain = state.trip(TripId::new(0)).current_chain().expect("a chain");
    names(c.bike.network(), &chain.legs()[0].links)
}

/// The costs by hand (`t` in seconds; the utility at 0.2 a minute riding and `β` a kilometre in
/// mixed traffic):
///
/// | street | time: cycleway / street | dedicated (× 1.2 in mixed traffic) | utility, β 0.16 | utility, β 0 |
/// |---|---|---|---|---|
/// | 1 800 m | 400 / 432 s: cycleway | 400 / 518.4: cycleway | 1.333 / 1.440 + 0.288 = 1.728: cycleway | 1.333 / 1.440: cycleway |
/// | 1 600 m | 400 / 384 s: **street** | 400 / 460.8: cycleway | 1.333 / 1.280 + 0.256 = 1.536: cycleway | 1.333 / 1.280: **street** |
///
/// At 1 600 m the utility turns at `β = (1.333 − 1.280) / 1.6 = 0.0333`: 0.02 rides the street,
/// 0.05 the cycleway.
#[test]
fn the_bike_cost_chooses_the_route_as_derived() {
    let k = |beta: f64| Coefficients {
        in_vehicle_per_minute: 0.2,
        walk_per_minute: 0.33,
        parking_per_minute: 0.2,
        bike_mixed_per_kilometre: beta,
        ..Coefficients::time_based()
    };
    let cycleway = ["oa", "ad"];
    let street = ["ob", "bd"];
    for (street_m, cost, beta, expected) in [
        (1800.0, BikeCost::Time, 0.16, &cycleway),
        (1800.0, BikeCost::Dedicated, 0.16, &cycleway),
        (1800.0, BikeCost::Utility, 0.16, &cycleway),
        (1800.0, BikeCost::Utility, 0.0, &cycleway),
        (1600.0, BikeCost::Time, 0.16, &street),
        (1600.0, BikeCost::Dedicated, 0.16, &cycleway),
        (1600.0, BikeCost::Utility, 0.16, &cycleway),
        (1600.0, BikeCost::Utility, 0.0, &street),
        (1600.0, BikeCost::Utility, 0.02, &street),
        (1600.0, BikeCost::Utility, 0.05, &cycleway),
    ] {
        assert_eq!(
            chosen(street_m, cost, k(beta)),
            expected,
            "street {street_m} m, bike cost {}, beta {beta}",
            cost.as_str()
        );
    }
    // The factor the dedicated cost reads is the shipped one the table assumes.
    assert_eq!(StaticLayerDefaults::SHIPPED.bike_mixed_cost_factor, MIXED_FACTOR);
}

// --- bikes beside cars ----------------------------------------------------------------------------

/// Cars from `o` to `d` on two roads (`o–c–d`, a secondary road of 2 × 1 000 m at 1 lane, and
/// `o–b–d`, a residential street of 2 × 900 m), bikes on a cycleway `o–a–d`.
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

/// **Property**: bikes on a separated path change no car result. The same cars with and without
/// 300 bike trips on the cycleway, five iterations by the point queue, by the deterministic choice
/// and the logit: every car trip's set, chain and probabilities, the link travel times, and every
/// vehicle loaded and its arrival are equal to the bit. (The convergence shares differ: they are
/// over every trip, and the bike trips never switch.)
#[test]
fn bikes_on_a_cycleway_change_no_car_result() {
    let (alone, with) = (cars_and_bikes(false), cars_and_bikes(true));
    let n = alone.trips.len() as usize;
    for t in 0..n {
        let trip = TripId::from_index(t);
        assert_eq!(
            alone.travellers.external_ids().external(alone.trips.traveller(trip).raw()),
            with.travellers.external_ids().external(with.trips.traveller(trip).raw()),
            "car trip {t} keeps its index"
        );
    }
    let the_logit = logit();
    for model in [&HighestUtility as &dyn ChoiceModel, &the_logit] {
        let (la, lw) =
            (PointQueue::new(&alone.turns, 7200.0), PointQueue::new(&with.turns, 7200.0));
        let (a, ra, reca) = run(&alone, Layers::NONE, model, &la, &params(5, BikeCost::Utility));
        let (w, rw, recw) = run(&with, layers(&with), model, &lw, &params(5, BikeCost::Utility));
        for t in 0..n {
            assert!(a.trip(TripId::from_index(t)) == w.trip(TripId::from_index(t)), "car trip {t}");
        }
        assert_eq!(a.link_times().rows(), w.link_times().rows(), "the link travel times");
        assert_eq!(reca.vehicles, recw.vehicles, "every vehicle loaded");
        assert_eq!(reca.arrivals, recw.arrivals, "every arrival");
        for t in n..with.trips.len() as usize {
            let chain = w.trip(TripId::from_index(t)).current_chain().expect("a bike chain");
            assert_eq!(names(with.bike.network(), &chain.legs()[0].links), ["oa", "ad"]);
        }
        let cars_on = |s: &State, link: &str| {
            let l = alone.net.link_external_ids().typed_id_of::<LinkId>(link).expect("a link");
            (0..n)
                .filter(|&t| {
                    s.trip(TripId::from_index(t))
                        .current_chain()
                        .is_some_and(|c| c.legs()[0].links.contains(&l))
                })
                .count()
        };
        println!(
            "{}: cars on o-c-d {} and o-b-d {}, the same with bikes; switch share in the last iteration {:.4} alone, {:.4} with 300 bikes",
            model.name(),
            cars_on(&a, "oc"),
            cars_on(&a, "ob"),
            ra.last().expect("reports").switch_share,
            rw.last().expect("reports").switch_share
        );
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
fn grid_trips(net: &RoadNetwork, node: &dyn Fn(u32, u32) -> String) -> Vec<RawTrip> {
    (0..600_u32)
        .map(|i| {
            let (a, b) = ((i * 7) % 36, (i * 13 + 5) % 36);
            let mode = [Some(Mode::Bike), Some(Mode::Walk), None][i as usize % 3];
            trip(net, &format!("g{i:03}"), &node(a / 6, a % 6), &node(b / 6, b % 6), 3 * i, mode)
        })
        .collect()
}

fn plain_node(r: u32, c: u32) -> String {
    format!("n{r}{c}")
}

fn plain_link(i: usize) -> String {
    format!("l{i:03}")
}

fn grid_case(
    node: &dyn Fn(u32, u32) -> String,
    link: &dyn Fn(usize) -> String,
    order: &[usize],
) -> Case {
    let net = grid(node, link, order);
    let trips = grid_trips(&net, node);
    case(grid(node, link, order), trips)
}

/// **Property**: the same inputs give the same run, to the bit, on 1, 2, 5 and every thread, and
/// twice on one: the bike and walking trips are planned in parallel chunks, each by its own
/// search.
#[test]
fn one_thread_and_several_give_the_same_run_with_bikes_and_walks() {
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
                let (s, r, _) =
                    run(&c, layers(&c), &the_logit, &loader, &params(4, BikeCost::Utility));
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
    let modes = |m: Mode| {
        one.0
            .trips()
            .iter()
            .filter(|t| t.current_chain().is_some_and(|c| c.legs()[0].mode == m))
            .count()
    };
    println!(
        "1, 1, 2, 5 and all threads agree; chains taken: {} car, {} bike, {} walk",
        modes(Mode::Car),
        modes(Mode::Bike),
        modes(Mode::Walk)
    );
}

/// **Property**: renaming every node and link of the network (and with it of both layers, which
/// take the road network's names) and listing the links in another order changes no bike or
/// walking route: each trip's route, read as the sequence of its links' end points, and its time
/// are the same, although the link numbers differ. The grid's equal blocks make many routes tie
/// exactly, so the tie rule decides, and it reads no id.
#[test]
fn renaming_the_layers_changes_no_route() {
    let order: Vec<usize> = (0..GRID_LINKS).collect();
    let plain = grid_case(&plain_node, &plain_link, &order);
    let perm = shuffled(GRID_LINKS, 11);
    let node_perm = shuffled(36, 5);
    let renamed_node = |r: u32, c: u32| format!("x{:02}", node_perm[(r * 6 + c) as usize]);
    let renamed_link = |i: usize| format!("k{:03}", perm[i]);
    let other_order = shuffled(GRID_LINKS, 3);
    let renamed = grid_case(&renamed_node, &renamed_link, &other_order);
    let loader = FreeFlow::default();
    let p = params(1, BikeCost::Dedicated);
    let (a, _, _) = run(&plain, layers(&plain), &HighestUtility, &loader, &p);
    let (b, _, _) = run(&renamed, layers(&renamed), &HighestUtility, &loader, &p);
    let ends = |c: &Case, mode: Mode, links: &[LinkId]| -> Vec<(i64, i64)> {
        let g = if mode == Mode::Bike { c.bike.network() } else { c.walk.network() };
        let at = |n| {
            let p = g.node_lonlat(n);
            ((p.lon * 1e7).round() as i64, (p.lat * 1e7).round() as i64)
        };
        let mut out: Vec<(i64, i64)> = links.iter().map(|&l| at(g.link_from(l))).collect();
        out.extend(links.last().map(|&l| at(g.link_to(l))));
        out
    };
    let (mut compared, mut differ) = (0, 0);
    for t in 0..plain.trips.len() as usize {
        let trip = TripId::from_index(t);
        let (Some(x), Some(y)) = (a.trip(trip).current_chain(), b.trip(trip).current_chain())
        else {
            continue;
        };
        let (lx, ly) = (&x.legs()[0], &y.legs()[0]);
        if lx.mode == Mode::Car {
            continue;
        }
        assert_eq!(lx.mode, ly.mode);
        assert_eq!(
            ends(&plain, lx.mode, &lx.links),
            ends(&renamed, ly.mode, &ly.links),
            "trip {t}"
        );
        assert_eq!(x.total(), y.total(), "trip {t}: the same attributes");
        compared += 1;
        if lx.links != ly.links {
            differ += 1;
        }
    }
    assert!(compared >= 380 && differ > 0, "{compared} compared, {differ} with other numbers");
    println!(
        "{compared} bike and walking routes the same street for street; {differ} of them numbered otherwise"
    );
}

// --- what is refused, and a run rebuilt -------------------------------------------------------------

/// A traveller without a bike has no bike chain (and so no trip); a bike trip in a run without a
/// bike layer is refused by name; a state rebuilt from what a run wrote equals it.
#[test]
fn ownership_layers_and_a_rebuilt_state() {
    let net = cycleway_and_street(1800.0);
    let rows = vec![
        trip(&net, "t0", "o", "d", 0, Some(Mode::Bike)),
        trip(&net, "t1", "o", "d", 0, Some(Mode::Walk)),
        trip(&net, "t2", "o", "d", 0, None),
    ];
    let nobody = case_owning(
        cycleway_and_street(1800.0),
        rows.clone(),
        Ownership { car: true, bike: false, transit_pass: false },
    );
    let loader = FreeFlow::default();
    let (state, reports, _) =
        run(&nobody, layers(&nobody), &HighestUtility, &loader, &params(1, BikeCost::Utility));
    assert!(state.trip(TripId::new(0)).current.is_none(), "no bike, no bike chain");
    assert!(state.trip(TripId::new(1)).current.is_some(), "walking needs nothing");
    assert_eq!(reports[0].without_chain, 1.0);

    let c = case(cycleway_and_street(1800.0), rows);
    let model = HighestUtility;
    let refused = assign(
        &inputs(&c, Layers::NONE, &model, &loader, 7),
        &params(1, BikeCost::Utility),
        &mut NoHooks,
    );
    let message = refused.expect_err("no bike layer").to_string();
    assert!(message.contains("bike"), "{message}");

    let p = params(3, BikeCost::Utility);
    let point_queue = PointQueue::new(&c.turns, 7200.0);
    let i = inputs(&c, layers(&c), &model, &point_queue, 7);
    let (state, _) = assign(&i, &p, &mut NoHooks).expect("the run");
    let saved: Vec<SavedTrip> = state.trips().iter().map(SavedTrip::of).collect();
    let rebuilt = restore(&i, &p, &saved, state.iterations()).expect("rebuilt");
    assert!(rebuilt == state, "the rebuilt state equals the run's");
}
