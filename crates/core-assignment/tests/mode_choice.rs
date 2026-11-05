//! Trips choosing their mode among the car, the bike and walking, through the one chooser,
//! checked against values derived by hand from each case's own constants (lengths, speeds, the
//! class's coefficients, the model's formula) and against properties that must hold; never
//! against a run's output.
//!
//! | Test | Case | What is derived |
//! |---|---|---|
//! | 1 | a car road of 10 min, a cycleway of 12 min, a footway of 40 min, free flow | the set holds one chain of each mode; the multinomial logit's and the nested logit's shares by formula (equal with one chain a nest), the model's probabilities to 1e-12, the shares drawn by 6 000 travellers within three standard deviations; a mode's constant moves them as its formula says |
//! | 2 | the same, the deterministic choice | every trip drives (10 < 12 · 0.99); with a band of 0.25 the car and the bike tie and split 3 000 / 3 000 |
//! | 3 | a second car road of 11 min seeded in every set (V-C2's closed form) | the nested logit's mode and route split at μ 0.5 and 1 by formula, probabilities to 1e-12, draws within 3σ |
//! | 4 | the car road with a bottleneck | **property**: more demand gives a bike share that does not fall; demand far below capacity gives the free-flow shares by formula |
//! | 5 | the bottleneck, cars only, with and without 300 stated bike trips | **the ruling**: the stopping shares and the iteration at which the run stops are unchanged, to the bit |
//! | 6 | the bottleneck with mode choice | **property**: the same run twice and on 1, 2 and every thread, to the bit; a state rebuilt from what the run wrote equals it |
//! | 7 | a grid of unequal blocks, every node and link renamed and the links listed in another order | **property**: every trip's mode and chain, street for street, and every report are the same |
//! | 8 | classes without a car, without a bike, with short limits | the modes left out of each set, and why, as constructed; a class offered one mode is a trip of that mode at any length |
//! | 9 | a cycleway against a street | the bike's cost of a kilometre on a track of its own chooses the route where its formula turns |
//! | 10 | test 1's three modes at a stationary state (every trip on a draw from its probabilities) | the expected change of each mode's share equals `P_m` less the share drawn and is 0 within three standard deviations; grouping the expected switch excess by the mode held would not be 0 (walkers: `Σ P² − P_walk` = 0.4525) |

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::float_cmp,
    clippy::too_many_lines,
    reason = "test fixtures: small counts, and exact comparisons of values that must be equal to the bit"
)]

use openmobisim_core_assignment::{
    CarPlanner, ClassModes, Coefficients, FreeFlow, HighestUtility, Inputs, IterationReport, Layer,
    Layers, LeftOut, Loader, NoHooks, Params, Place, PointQueue, SavedTrip, State, assign,
    left_out, logit, restore,
};
use openmobisim_core_choice::{ChoiceModel, NestedLogit, Options};
use openmobisim_core_demand::{
    ClassDefaults, Mode, Ownership, RawTrip, Travellers, Trips, build_travellers,
};
use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::layers::{StaticLayer, StaticLayerDefaults, StaticNetwork};
use openmobisim_core_graph::network::{LinkSpec, RoadNetwork, RoadNetworkBuilder};
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::{EntityId, LinkId, NodeId, TripId};
use openmobisim_core_types::rng::RngKey;
use openmobisim_core_types::time::Second;

// --- the cases' constants ------------------------------------------------------------------------

/// The car roads' speed, km/h: 7 500 m in 300 s.
const CAR_KM_H: f64 = 90.0;
/// A bike's speed on a cycleway, km/h (the shipped layer defaults).
const BIKE_DEDICATED_KM_H: f64 = 18.0;
/// The walking speed, km/h (the shipped layer defaults).
const WALK_KM_H: f64 = 4.8;
/// The time coefficient of the logit cases: 0.1 utility units a minute, in a vehicle, walking or
/// parking alike.
const BETA_PER_MIN: f64 = 0.1;

fn metres_per_second(km_h: f64) -> f64 {
    km_h / 3.6
}

// --- helpers --------------------------------------------------------------------------------

/// A road network of `nodes` (name, metres east, metres north) and `links` (name, from, to,
/// metres, class, km/h or 0 for the class's own speed, capacity veh/h or 0 for the class's,
/// lanes), each length stated.
#[allow(clippy::type_complexity, reason = "a table of links, written as a literal")]
fn network(
    nodes: &[(&str, f64, f64)],
    links: &[(&str, &str, &str, f64, RoadClass, f64, f64, u8)],
) -> RoadNetwork {
    let mut b = RoadNetworkBuilder::new();
    for &(n, x, y) in nodes {
        b.add_node(n, LonLat::new(4.8 + x / 77_800.0, 45.7 + y / 110_574.0));
    }
    for &(name, from, to, metres, class, km_h, veh_h, lanes) in links {
        let mut spec = LinkSpec::new(class);
        spec.length_m = Some(metres);
        if km_h > 0.0 {
            spec.maxspeed_km_h = Some(km_h);
        }
        if veh_h > 0.0 {
            spec.capacity_veh_h = Some(veh_h);
        }
        spec.lanes = Some(lanes);
        b.add_link(name, from, to, spec);
    }
    b.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
        .expect("buildable")
}

/// `o` to `d`: a car road through `c` (2 × 7 500 m at 90 km/h, 600 s), with a bottleneck of
/// `bottleneck_veh_h` on its second link; a second car road through `e` (2 × 8 250 m, 660 s) when
/// `second`; a cycleway through `a` (2 × 1 800 m at 18 km/h, 720 s); a footway through `w`
/// (2 × 1 600 m at 4.8 km/h, 2 400 s). The car roads are trunk roads (no bike, a pedestrian walks
/// them but the footway is far shorter); the cycleway carries no pedestrian and the footway no
/// bike or car.
fn three_modes(bottleneck_veh_h: f64, second: bool) -> RoadNetwork {
    let mut links = vec![
        ("oc", "o", "c", 7500.0, RoadClass::Trunk, CAR_KM_H, 7200.0, 2),
        ("cd", "c", "d", 7500.0, RoadClass::Trunk, CAR_KM_H, bottleneck_veh_h, 1),
        ("oa", "o", "a", 1800.0, RoadClass::Cycleway, 0.0, 0.0, 1),
        ("ad", "a", "d", 1800.0, RoadClass::Cycleway, 0.0, 0.0, 1),
        ("ow", "o", "w", 1600.0, RoadClass::Footway, 0.0, 0.0, 1),
        ("wd", "w", "d", 1600.0, RoadClass::Footway, 0.0, 0.0, 1),
    ];
    if second {
        links.push(("oe", "o", "e", 8250.0, RoadClass::Trunk, CAR_KM_H, 7200.0, 2));
        links.push(("ed", "e", "d", 8250.0, RoadClass::Trunk, CAR_KM_H, 7200.0, 2));
    }
    network(
        &[
            ("o", 0.0, 0.0),
            ("c", 1500.0, -1500.0),
            ("e", 1500.0, -2500.0),
            ("a", 1500.0, 800.0),
            ("w", 1500.0, 300.0),
            ("d", 3000.0, 0.0),
        ],
        &links,
    )
}

/// The run's inputs that live as long as the case.
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

/// A trip of traveller `id` of `class` from node `from` to node `to`, of `mode` (`None`: not
/// stated).
fn trip(
    net: &RoadNetwork,
    id: &str,
    class: &str,
    (from, to): (&str, &str),
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
        user_class: class.to_string(),
        weight: Some(1),
        mode,
    }
}

/// `n` trips of class `class` from `o` to `d`, departing evenly over `span` seconds from 7:00
/// (the i-th at `25 200 + ⌊i · span / n⌋`), ids `{prefix}{i}`.
fn trips_od(net: &RoadNetwork, n: u32, span: u32, class: &str, prefix: &str) -> Vec<RawTrip> {
    (0..n)
        .map(|i| {
            let at = 25_200 + i * span / n;
            trip(net, &format!("{prefix}{i:05}"), class, ("o", "d"), at, None)
        })
        .collect()
}

/// The case of `net` and `trips`, the classes owning `owners` (by class name; the rest a car and
/// a bike), every class weighing `k` and offered `modes` with no limit.
fn case(
    net: RoadNetwork,
    trips: Vec<RawTrip>,
    owners: &[(&str, Ownership)],
    k: Coefficients,
    modes: &[Mode],
) -> Case {
    let mut defaults = ClassDefaults::new().with_fallback(Ownership::VEHICLES);
    for &(class, own) in owners {
        defaults = defaults.with_default(class, own);
    }
    let (travellers, trips) =
        build_travellers(trips, Vec::new(), &defaults, 1, &mut Diagnostics::new()).expect("demand");
    let derive = |layer| {
        StaticNetwork::derive(&net, layer, StaticLayerDefaults::SHIPPED, &mut Diagnostics::new())
            .expect("a layer")
    };
    let (bike, walk) = (derive(StaticLayer::Bike), derive(StaticLayer::Walk));
    let turns_of = |l: &StaticNetwork| TurnTable::build(l.network(), l.network().signals());
    let classes = travellers.class_external_ids().count() as usize;
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
        classes: vec![k; classes],
        modes: vec![ClassModes::unlimited(modes); classes],
    }
}

fn layers(c: &Case) -> Layers<'_> {
    Layers {
        bike: Some(Layer { network: &c.bike, turns: &c.bike_turns }),
        walk: Some(Layer { network: &c.walk, turns: &c.walk_turns }),
    }
}

fn inputs<'a>(c: &'a Case, model: &'a dyn ChoiceModel, loader: &'a dyn Loader) -> Inputs<'a> {
    Inputs {
        network: &c.net,
        turns: &c.turns,
        layers: layers(c),
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

/// Exactly `iterations` iterations, no fixed time around the car or the bike.
fn exactly(iterations: u32) -> Params {
    Params { min_iterations: iterations, max_iterations: iterations, ..Params::default() }
}

/// The coefficients of the logit cases: [`BETA_PER_MIN`] a minute of any kind.
fn per_minute() -> Coefficients {
    Coefficients {
        in_vehicle_per_minute: BETA_PER_MIN,
        walk_per_minute: BETA_PER_MIN,
        wait_per_minute: BETA_PER_MIN,
        parking_per_minute: BETA_PER_MIN,
        ..Coefficients::time_based()
    }
}

/// The nested logit on the chooser's utility, nest scale `mu`.
fn nested(mu: f64) -> NestedLogit {
    let options: Options =
        [("beta_utility".to_string(), 1.0), ("mu".to_string(), mu)].into_iter().collect();
    NestedLogit::from_options(&options).expect("valid options")
}

/// How many trips of `state` take a chain of `mode`.
fn taking(state: &State, mode: Mode) -> usize {
    state.trips().iter().filter(|t| t.current_chain().is_some_and(|c| c.mode() == mode)).count()
}

/// The logit's probabilities of `utilities`, by formula.
fn logit_shares(utilities: &[f64]) -> Vec<f64> {
    let sum: f64 = utilities.iter().map(|v| v.exp()).sum();
    utilities.iter().map(|v| v.exp() / sum).collect()
}

/// `drawn` within three standard deviations of sampling `n` trips at probability `p`.
fn within_three_sigma(what: &str, drawn: usize, n: usize, p: f64) {
    let share = drawn as f64 / n as f64;
    let sigma = (p * (1.0 - p) / n as f64).sqrt();
    println!("{what}: drawn {share:.4}, formula {p:.4}, 3σ {:.4}", 3.0 * sigma);
    assert!((share - p).abs() <= 3.0 * sigma, "{what}: {share} against {p} ± {}", 3.0 * sigma);
}

/// The case's times are as stated: each link's time is its length over its speed.
fn check_times(c: &Case) {
    let road = |name: &str| {
        let l = c.net.link_external_ids().typed_id_of::<LinkId>(name).expect("a road link");
        c.net.free_flow_time(l).get()
    };
    let layer = |s: &StaticNetwork, name: &str| {
        let g = s.network();
        let l = g.link_external_ids().typed_id_of::<LinkId>(name).expect("a layer link");
        s.link_seconds()[l.index()]
    };
    assert_eq!(road("oc") + road("cd"), 15_000.0 / metres_per_second(CAR_KM_H));
    assert_eq!(road("oc") + road("cd"), 600.0, "the car's 10 minutes");
    let ride = layer(&c.bike, "oa") + layer(&c.bike, "ad");
    assert!((ride - 3600.0 / metres_per_second(BIKE_DEDICATED_KM_H)).abs() < 1e-9);
    assert!((ride - 720.0).abs() < 1e-9, "the bike's 12 minutes");
    let walk = layer(&c.walk, "ow") + layer(&c.walk, "wd");
    assert!((walk - 3200.0 / metres_per_second(WALK_KM_H)).abs() < 1e-9);
    assert!((walk - 2400.0).abs() < 1e-9, "the walk's 40 minutes");
}

// --- 1 and 2: three modes, one chain each ---------------------------------------------------

/// Derived: at 0.1 a minute the car costs 1.0, the bike 1.2 and the walk 4.0, so the logit's
/// shares are `e^{−1.0}`, `e^{−1.2}`, `e^{−4.0}` over their sum: 0.5352, 0.4382, 0.0266. With one
/// chain in each nest the nested logit's inclusive values are `V / μ`, its nests weigh
/// `e^{μ · V / μ} = e^V`: the same shares at every μ. A bike constant of −0.2 (a bike chain costs
/// 0.2 less) makes the bike's utility −1.0, the car's: car and bike 0.4879 each.
#[test]
fn the_logit_and_the_nested_logit_split_three_modes_by_formula() {
    let n = 6000;
    let net = three_modes(7200.0, false);
    let rows = trips_od(&net, n, n, "c", "t");
    let c = case(
        three_modes(7200.0, false),
        rows,
        &[],
        per_minute(),
        &[Mode::Car, Mode::Bike, Mode::Walk],
    );
    check_times(&c);
    let free_flow = FreeFlow::default();
    let formula = logit_shares(&[-1.0, -1.2, -4.0]);
    let the_logit = logit();
    let (half, one) = (nested(0.5), nested(1.0));
    for (name, model) in [
        ("logit", &the_logit as &dyn ChoiceModel),
        ("nested logit μ 0.5", &half),
        ("nested logit μ 1", &one),
    ] {
        let (state, reports) =
            assign(&inputs(&c, model, &free_flow), &exactly(1), &mut NoHooks).expect("the run");
        for t in state.trips() {
            let modes: Vec<Mode> =
                t.set.iter().map(openmobisim_core_assignment::Chain::mode).collect();
            assert_eq!(modes, [Mode::Car, Mode::Bike, Mode::Walk], "{name}: one chain a mode");
            for (p, f) in t.probabilities.iter().zip(&formula) {
                assert!((p - f).abs() < 1e-12, "{name}: probability {p} against {f}");
            }
        }
        for (mode, p) in [Mode::Car, Mode::Bike, Mode::Walk].into_iter().zip(&formula) {
            within_three_sigma(
                &format!("{name}, {}", mode.as_str()),
                taking(&state, mode),
                n as usize,
                *p,
            );
        }
        // Every trip chooses its mode and is counted in the stopping shares; in iteration 0 no
        // trip held a chain, so no mode's group has a share.
        let r = &reports[0];
        assert_eq!(r.choosing, f64::from(n));
        assert_eq!((r.found, r.added), (n, n));
        assert_eq!(r.switch_share, 1.0);
        assert_eq!(r.switch_share_by_mode.car, None);
        assert_eq!(r.mode_change_share, None);
        let split = r.mode_split;
        let total = split.car.unwrap() + split.bike.unwrap() + split.walk.unwrap();
        assert!((total - 1.0).abs() < 1e-12);
        assert_eq!(split.bike.unwrap(), taking(&state, Mode::Bike) as f64 / f64::from(n));
    }
    // The bike's constant: −0.2 utility units, so the bike's cost is 1.0 as the car's.
    let mut liked = c;
    liked.classes = vec![Coefficients { bike_constant: -0.2, ..per_minute() }; 1];
    let (state, _) = assign(&inputs(&liked, &the_logit, &free_flow), &exactly(1), &mut NoHooks)
        .expect("the run");
    let formula = logit_shares(&[-1.0, -1.0, -4.0]);
    for (p, f) in state.trip(TripId::new(0)).probabilities.iter().zip(&formula) {
        assert!((p - f).abs() < 1e-12, "with the bike's constant: {p} against {f}");
    }
    within_three_sigma(
        "logit, bike constant −0.2, bike",
        taking(&state, Mode::Bike),
        n as usize,
        formula[1],
    );
}

/// Derived: the deterministic choice on time-based costs (10, 12 and 40 minutes) takes the car,
/// 12 being beyond the band of 1 %: 12 > 10 · 1.01. With a band of 0.25 the bike ties with the
/// car (12 ≤ 10 · 1.25) and the walk does not (40 > 12.5): the 6 000 trips split evenly, 3 000
/// each, in trip order.
#[test]
fn the_deterministic_choice_takes_the_car_or_splits_a_tie_evenly() {
    let n = 6000;
    let net = three_modes(7200.0, false);
    let rows = trips_od(&net, n, n, "c", "t");
    let c = case(
        three_modes(7200.0, false),
        rows,
        &[],
        Coefficients::time_based(),
        &[Mode::Car, Mode::Bike, Mode::Walk],
    );
    let free_flow = FreeFlow::default();
    let (state, _) =
        assign(&inputs(&c, &HighestUtility, &free_flow), &exactly(1), &mut NoHooks).expect("run");
    assert_eq!(taking(&state, Mode::Car), n as usize, "every trip drives");
    let tie = Params { indifference_band: 0.25, ..exactly(1) };
    let (state, reports) =
        assign(&inputs(&c, &HighestUtility, &free_flow), &tie, &mut NoHooks).expect("run");
    assert_eq!(
        (taking(&state, Mode::Car), taking(&state, Mode::Bike), taking(&state, Mode::Walk)),
        (3000, 3000, 0),
        "the tie split evenly"
    );
    assert_eq!(reports[0].mode_split.car, Some(0.5));
}

// --- 3: V-C2's closed form -----------------------------------------------------------------

/// V-C2's closed form (Ben-Akiva and Lerman 1985; Train 2009, ch. 4) with a second car road of
/// 11 minutes in every set before the first choice (the search, at free flow, finds only the
/// shortest): `V = −1.0, −1.1, −1.2, −4.0` (car through `c`, car through `e`, bike, walk);
/// `I_car = ln(e^{−1.0/μ} + e^{−1.1/μ})`, `P(car) = e^{μ I_car} / (e^{μ I_car} + e^{−1.2} +
/// e^{−4.0})`, `P(c | car) = e^{−1.0/μ} / (e^{−1.0/μ} + e^{−1.1/μ})`. At μ = 0.5: P(car) 0.6083,
/// P(c | car) 0.5498; at μ = 1 (the logit over the four): 0.6868 and 0.5250. The routes share no
/// link (path size 1).
#[test]
fn the_nested_logit_splits_modes_and_routes_as_its_closed_form() {
    let n = 6000;
    let net = three_modes(7200.0, true);
    let rows = trips_od(&net, n, n, "c", "t");
    let c = case(
        three_modes(7200.0, true),
        rows,
        &[],
        per_minute(),
        &[Mode::Car, Mode::Bike, Mode::Walk],
    );
    let road = |names: &[&str]| -> Vec<LinkId> {
        names.iter().map(|x| c.net.link_external_ids().typed_id_of(x).expect("a link")).collect()
    };
    let node = |x: &str| -> NodeId { c.net.node_external_ids().typed_id_of(x).expect("a node") };
    let car = CarPlanner::new(&c.net, &c.turns, &c.classes, &c.money).expect("the car planner");
    let mut seeded = State::initial(&c.net, &c.travellers, &c.trips);
    for t in 0..c.trips.len() {
        let trip = TripId::new(t);
        let departure = f64::from(c.trips.departure(trip).get());
        for links in [road(&["oc", "cd"]), road(&["oe", "ed"])] {
            let leg = car.leg_along(
                Place::Road(node("o")),
                Place::Road(node("d")),
                departure,
                links,
                &seeded,
            );
            seeded.trip_mut(trip).add(openmobisim_core_assignment::Chain::new(vec![leg]));
        }
    }
    let free_flow = FreeFlow::default();
    for mu in [0.5, 1.0] {
        let model = nested(mu);
        let mut i = inputs(&c, &model, &free_flow);
        i.initial = Some(seeded.clone());
        let (state, _) = assign(&i, &exactly(1), &mut NoHooks).expect("the run");
        let (v1, v2, vb, vw) = (-1.0_f64, -1.1_f64, -1.2_f64, -4.0_f64);
        let inclusive = ((v1 / mu).exp() + (v2 / mu).exp()).ln();
        let p_car = (mu * inclusive).exp() / ((mu * inclusive).exp() + vb.exp() + vw.exp());
        let p_short = (v1 / mu).exp() / ((v1 / mu).exp() + (v2 / mu).exp());
        let p_bike = vb.exp() / ((mu * inclusive).exp() + vb.exp() + vw.exp());
        let formula = [p_car * p_short, p_car * (1.0 - p_short), p_bike, 1.0 - p_car - p_bike];
        for t in state.trips() {
            assert_eq!(t.set.len(), 4, "two car roads, a bike and a walk");
            for (p, f) in t.probabilities.iter().zip(&formula) {
                assert!((p - f).abs() < 1e-12, "μ {mu}: probability {p} against {f}");
            }
        }
        let cars = taking(&state, Mode::Car);
        within_three_sigma(&format!("μ {mu}, the car's share"), cars, n as usize, p_car);
        let short = state.trips().iter().filter(|t| t.current == Some(0)).count();
        within_three_sigma(
            &format!("μ {mu}, the short road's share of the cars"),
            short,
            cars,
            p_short,
        );
    }
}

// --- 4: congestion moves trips off the car ---------------------------------------------------

/// **Property**: with the car road's second link a bottleneck of 600 veh/h and the travellers
/// departing over an hour, the expected bike share (the mean over the trips of the bike chain's
/// probability at their last choice) does not fall as the demand grows: 60, 1 200, 2 400 and
/// 4 800 trips, 20 iterations at the point queue. At 60 trips (some 32 cars an hour against 600)
/// no queue forms, and each trip's probabilities are the free-flow ones by formula (test 1's).
#[test]
fn congestion_moves_trips_off_the_car() {
    let mut last = 0.0;
    for n in [60, 1200, 2400, 4800] {
        let net = three_modes(600.0, false);
        let rows = trips_od(&net, n, 3600, "c", "t");
        let c = case(
            three_modes(600.0, false),
            rows,
            &[],
            per_minute(),
            &[Mode::Car, Mode::Bike, Mode::Walk],
        );
        let loader = PointQueue::new(&c.turns, 25_200.0 + 4.0 * 3600.0);
        let model = nested(0.5);
        let (state, reports) =
            assign(&inputs(&c, &model, &loader), &exactly(20), &mut NoHooks).expect("the run");
        let bike: f64 =
            state.trips().iter().map(|t| t.probabilities[1]).sum::<f64>() / f64::from(n);
        let r = reports.last().expect("reports");
        println!(
            "{n} trips: expected bike share {bike:.4}, drawn {:.4}, car {:.4}, mean trip {:.0} s",
            r.mode_split.bike.unwrap(),
            r.mode_split.car.unwrap(),
            r.mean_trip_seconds
        );
        assert!(bike >= last - 1e-12, "{n} trips: bike share {bike} fell from {last}");
        if n == 60 {
            let formula = logit_shares(&[-1.0, -1.2, -4.0]);
            for t in state.trips() {
                for (p, f) in t.probabilities.iter().zip(&formula) {
                    assert!((p - f).abs() < 1e-12, "no queue: probability {p} against {f}");
                }
            }
        }
        last = bike;
    }
    assert!(last > logit_shares(&[-1.0, -1.2, -4.0])[1] + 0.05, "the bottleneck moved trips");
}

// --- 5: the ruling ---------------------------------------------------------------------------

/// **The ruling (2026-11-05)**: the stopping shares count only the trips that could change their
/// choice. 1 200 car trips on the bottleneck and its second road (the logit splits them, and the
/// queue moves them), the loop's own stopping rule with a tolerance of 0.004, with and without 300
/// stated bike trips (their travellers' ids sorted after the cars'): every stopping share, the
/// duality gap, its deterministic part, the spread and the iteration at which the run stops are
/// the same, to the bit.
#[test]
fn stated_bike_trips_change_no_stopping_share() {
    let net = three_modes(600.0, true);
    let cars = trips_od(&net, 1200, 3600, "c", "t");
    let mut plus = cars.clone();
    plus.extend((0..300).map(|i| {
        trip(&net, &format!("~bike{i:03}"), "c", ("o", "d"), 25_200 + 12 * i, Some(Mode::Bike))
    }));
    let model = logit();
    // A tolerance tight enough that the rule, not the minimum, stops the run.
    let params = Params { max_iterations: 60, switch_tolerance: 0.004, ..Params::default() };
    let mut runs = Vec::new();
    for rows in [cars, plus] {
        let mut c = case(three_modes(600.0, true), rows, &[], per_minute(), &[]);
        c.modes.clear();
        let loader = PointQueue::new(&c.turns, 25_200.0 + 4.0 * 3600.0);
        let (_, reports) =
            assign(&inputs(&c, &model, &loader), &params, &mut NoHooks).expect("run");
        runs.push(reports);
    }
    let (alone, with) = (&runs[0], &runs[1]);
    println!(
        "cars alone stop after {} iterations, with the bikes after {}",
        alone.len(),
        with.len()
    );
    assert!(alone.len() > 3 && alone.len() < 60, "the rule, not a limit, stopped the run");
    assert_eq!(alone.len(), with.len(), "the same iteration");
    let shares = |r: &IterationReport| {
        [
            r.found_share,
            r.added_share,
            r.choosing,
            r.switch_share,
            r.switch_floor,
            r.switch_excess,
            r.switch_above_floor,
            r.switch_excess_expected,
            r.duality_gap,
            r.gap_deterministic,
            r.on_cheapest_share,
        ]
        .map(f64::to_bits)
    };
    for (a, b) in alone.iter().zip(with) {
        assert_eq!(shares(a), shares(b), "iteration {}", a.iteration);
        assert_eq!((a.found, a.added), (b.found, b.added));
        assert_eq!(a.switch_share_by_mode, b.switch_share_by_mode);
        assert_eq!(a.mode_share_change_expected, b.mode_share_change_expected);
    }
    assert!(with[1].departed > alone[1].departed, "the bikes travel, counted among the departed");
}

// --- 6: threads, twice, rebuilt ----------------------------------------------------------------

/// **Property**: mode choice on the bottleneck (2 400 trips, the nested logit, 8 iterations at the
/// point queue) gives the same state and reports, to the bit, twice on one thread and on 2 and
/// every thread; and the state rebuilt from what the run wrote (every set, by each chain's mode
/// and links) equals the run's.
#[test]
fn mode_choice_is_the_same_on_any_thread_and_rebuilt() {
    let net = three_modes(600.0, false);
    let rows = trips_od(&net, 2400, 2400, "c", "t");
    let c = case(
        three_modes(600.0, false),
        rows,
        &[],
        per_minute(),
        &[Mode::Car, Mode::Bike, Mode::Walk],
    );
    let loader = PointQueue::new(&c.turns, 25_200.0 + 4.0 * 3600.0);
    let model = nested(0.5);
    let params = exactly(8);
    let runs: Vec<(State, Vec<IterationReport>)> = [1, 1, 2, 0]
        .iter()
        .map(|&threads| {
            let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().expect("pool");
            pool.install(|| {
                assign(&inputs(&c, &model, &loader), &params, &mut NoHooks).expect("run")
            })
        })
        .collect();
    let (one, rest) = runs.split_first().expect("runs");
    for (k, other) in rest.iter().enumerate() {
        assert!(one.0 == other.0, "the state differs (run {k})");
        let a: Vec<IterationReport> = one.1.iter().map(IterationReport::without_times).collect();
        let b: Vec<IterationReport> = other.1.iter().map(IterationReport::without_times).collect();
        assert_eq!(a, b, "the reports differ (run {k})");
    }
    assert!(taking(&one.0, Mode::Car) > 0 && taking(&one.0, Mode::Bike) > 0, "both modes taken");
    let saved: Vec<SavedTrip> = one.0.trips().iter().map(SavedTrip::of).collect();
    let i = inputs(&c, &model, &loader);
    let rebuilt = restore(&i, &params, &saved, one.0.iterations()).expect("rebuilt");
    assert!(rebuilt == one.0, "the rebuilt state equals the run's");
    // A chain without its mode is refused for a trip choosing its mode.
    let mut bare = saved.clone();
    bare[0].modes.clear();
    assert!(restore(&i, &params, &bare, 8).is_err(), "a chain of no mode");
}

// --- 7: relabelling ----------------------------------------------------------------------------

/// A 5 × 5 grid of residential streets, both ways, each block's length set by its place in the
/// grid (180 to 228 m, so no two routes cost exactly the same), the link between the second and
/// third row of the middle column a cycleway; nodes named by `node`, links by `link`, listed in
/// `order`.
fn grid(
    node: &dyn Fn(u32, u32) -> String,
    link: &dyn Fn(usize) -> String,
    order: &[usize],
) -> RoadNetwork {
    let n = 5;
    let mut b = RoadNetworkBuilder::new();
    for r in 0..n {
        for c in 0..n {
            let (x, y) = (f64::from(c) * 200.0, f64::from(r) * 200.0);
            b.add_node(node(r, c), LonLat::new(4.8 + x / 77_800.0, 45.7 + y / 110_574.0));
        }
    }
    let pairs = grid_pairs(n);
    for &i in order {
        let ((r1, c1), (r2, c2)) = pairs[i];
        let class = if (c1, c2) == (2, 2) && r1.min(r2) == 1 {
            RoadClass::Cycleway
        } else {
            RoadClass::Residential
        };
        let mut spec = LinkSpec::new(class);
        spec.length_m =
            Some(180.0 + f64::from(((r1 * 7 + c1 * 3 + r2 * 5 + c2 * 11) % 17) as u8) * 3.0);
        b.add_link(link(i), node(r1, c1), node(r2, c2), spec);
    }
    b.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
        .expect("buildable")
}

fn grid_pairs(n: u32) -> Vec<((u32, u32), (u32, u32))> {
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
    pairs
}

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

fn grid_case(
    node: &dyn Fn(u32, u32) -> String,
    link: &dyn Fn(usize) -> String,
    order: &[usize],
) -> Case {
    let net = grid(node, link, order);
    let rows: Vec<RawTrip> = (0..500_u32)
        .map(|i| {
            let (a, b) = ((i * 7) % 25, (i * 11 + 3) % 25);
            let at = |k: u32| node(k / 5, k % 5);
            trip(&net, &format!("g{i:03}"), "c", (&at(a), &at(b)), 25_200 + 4 * i, None)
        })
        .collect();
    let k = Coefficients { bike_mixed_per_kilometre: 0.16, ..per_minute() };
    case(grid(node, link, order), rows, &[], k, &[Mode::Car, Mode::Bike, Mode::Walk])
}

/// Every chain of every set: its mode and its links, each named by its ends' positions.
type Named = Vec<Vec<(Mode, Vec<String>)>>;

/// **Property**: renaming every node and link of the road network (and so of the layers derived
/// from it) and listing the links in another order changes no trip's mode, no chain (street for
/// street) and no report: the choice's draws are addressed by trip and by the chain's place in
/// the set, never by an id.
#[test]
fn renaming_the_networks_changes_no_mode_and_no_chain() {
    let links = grid_pairs(5).len();
    let plain = grid_case(
        &|r, c| format!("n{r}{c}"),
        &|i| format!("l{i:03}"),
        &(0..links).collect::<Vec<_>>(),
    );
    let renamed = grid_case(
        &|r, c| format!("node-{}", 99 - (r * 5 + c)),
        &|i| format!("x{}", 7919 * (i + 1)),
        &shuffled(links, 3),
    );
    let model = nested(0.5);
    let runs: Vec<(State, Vec<IterationReport>, Named)> = [&plain, &renamed]
        .iter()
        .map(|c| {
            let loader = PointQueue::new(&c.turns, 25_200.0 + 3.0 * 3600.0);
            let (state, reports) =
                assign(&inputs(c, &model, &loader), &exactly(6), &mut NoHooks).expect("run");
            // Each chain of each set, by mode and the names of its links in travel order, named as
            // the plain network names them (a link's place in the grid's list of pairs).
            let named = state
                .trips()
                .iter()
                .map(|t| {
                    t.set
                        .iter()
                        .map(|ch| {
                            let leg = &ch.legs()[0];
                            let graph = match leg.mode {
                                Mode::Bike => c.bike.network(),
                                Mode::Walk => c.walk.network(),
                                _ => &c.net,
                            };
                            let ends = leg
                                .links
                                .iter()
                                .map(|&l| {
                                    let at = |n: NodeId| graph.node_lonlat(n);
                                    let (a, b) = (at(graph.link_from(l)), at(graph.link_to(l)));
                                    format!("{:.6},{:.6}>{:.6},{:.6}", a.lon, a.lat, b.lon, b.lat)
                                })
                                .collect();
                            (leg.mode, ends)
                        })
                        .collect()
                })
                .collect();
            (state, reports, named)
        })
        .collect();
    let (a, b) = (&runs[0], &runs[1]);
    assert_eq!(a.2, b.2, "every set, mode and street, is the same");
    let currents = |s: &State| s.trips().iter().map(|t| t.current).collect::<Vec<_>>();
    assert_eq!(currents(&a.0), currents(&b.0), "every trip takes the same chain");
    let ra: Vec<IterationReport> = a.1.iter().map(IterationReport::without_times).collect();
    let rb: Vec<IterationReport> = b.1.iter().map(IterationReport::without_times).collect();
    assert_eq!(ra, rb, "the same reports");
    println!(
        "renamed grid: {} car, {} bike, {} walk, as on the plain one",
        taking(&b.0, Mode::Car),
        taking(&b.0, Mode::Bike),
        taking(&b.0, Mode::Walk)
    );
}

// --- 8: what is left out ------------------------------------------------------------------------

/// Classes built to leave modes out, each case's count derived from its construction: `nocar`
/// (no car: 10 trips, `car_no_vehicle`), `nobike` (no bike: 20 trips, `bike_no_vehicle`), `short`
/// (a walk limit of 1 800 s under the walk's 2 400 s and a ride limit of 600 s under the ride's
/// 720 s: 30 trips, `walk_too_long` and `bike_too_long`), `none` (no car, no bike, the short
/// limits: 40 trips with no chain at all, which do not travel), `walker` (offered walking alone:
/// 5 trips of that mode, at any length, outside the stopping shares).
#[test]
fn modes_that_cannot_serve_a_trip_are_left_out_and_counted() {
    let net = three_modes(7200.0, false);
    let mut rows = Vec::new();
    for (class, n) in [("nocar", 10), ("nobike", 20), ("short", 30), ("none", 40), ("walker", 5)] {
        rows.extend(trips_od(&net, n, 7 * n, class, class));
    }
    let none = Ownership { car: false, bike: false, transit_pass: false };
    let owners = [
        ("nocar", Ownership { car: false, ..Ownership::VEHICLES }),
        ("nobike", Ownership { bike: false, ..Ownership::VEHICLES }),
        ("none", none),
    ];
    let mut c = case(
        three_modes(7200.0, false),
        rows,
        &owners,
        per_minute(),
        &[Mode::Car, Mode::Bike, Mode::Walk],
    );
    let ids = c.travellers.class_external_ids();
    for k in 0..ids.count() {
        let class = &mut c.modes[k as usize];
        match ids.external(k) {
            "short" | "none" => {
                class.walk_max_seconds = 1800.0;
                class.bike_max_seconds = 600.0;
            }
            "walker" => {
                *class =
                    ClassModes { walk_max_seconds: 60.0, ..ClassModes::unlimited(&[Mode::Walk]) }
            }
            _ => {}
        }
    }
    let free_flow = FreeFlow::default();
    let model = logit();
    let i = inputs(&c, &model, &free_flow);
    let params = exactly(2);
    let (state, reports) = assign(&i, &params, &mut NoHooks).expect("the run");
    let left = left_out(&i, &params, &state).expect("the counts");
    assert_eq!(
        left,
        LeftOut {
            choosing: 100,
            no_chain: 40,
            car_no_vehicle: 50,
            car_no_route: 0,
            bike_no_vehicle: 60,
            bike_no_route: 0,
            bike_too_long: 30,
            walk_no_route: 0,
            walk_too_long: 70,
            transit_no_stop: 0,
            transit_no_journey: 0,
        }
    );
    let set_modes = |class: &str| -> Vec<Mode> {
        let t = (0..c.trips.len())
            .map(TripId::new)
            .find(|&t| ids.external(c.travellers.user_class(c.trips.traveller(t)).raw()) == class)
            .expect("a trip of the class");
        state.trip(t).set.iter().map(openmobisim_core_assignment::Chain::mode).collect()
    };
    assert_eq!(set_modes("nocar"), [Mode::Bike, Mode::Walk]);
    assert_eq!(set_modes("nobike"), [Mode::Car, Mode::Walk]);
    assert_eq!(set_modes("short"), [Mode::Car]);
    assert_eq!(set_modes("none"), []);
    assert_eq!(set_modes("walker"), [Mode::Walk], "one mode offered: walked at any length");
    let r = &reports[1];
    assert_eq!(r.without_chain, 40.0);
    assert_eq!(r.choosing, 100.0, "the walkers, of one mode, are not counted");
}

// --- 9: the bike's facilities in the one vocabulary --------------------------------------------

/// A cycleway (a track of its own, 2 × 1 000 m at 18 km/h: 400 s) against a residential street
/// (mixed traffic, 2 × 900 m at 15 km/h: 432 s), the utility bike cost, one minute riding costing
/// 1: the cycleway costs `400 / 60 + 2 · β_separated` = 6.667 + 2 β_separated, the street
/// `432 / 60 + 1.8 · β_mixed` = 7.2 (β_mixed 0). The routes turn at β_separated = 0.2667 per km:
/// 0.2 rides the cycleway (its 2 000 m counted as separated), 0.4 the street (1 800 m mixed).
#[test]
fn a_kilometre_on_a_track_of_its_own_is_weighed_by_its_coefficient() {
    let net = || {
        network(
            &[("o", 0.0, 0.0), ("a", 900.0, 500.0), ("b", 900.0, -300.0), ("d", 1700.0, 0.0)],
            &[
                ("oa", "o", "a", 1000.0, RoadClass::Cycleway, 0.0, 0.0, 1),
                ("ad", "a", "d", 1000.0, RoadClass::Cycleway, 0.0, 0.0, 1),
                ("ob", "o", "b", 900.0, RoadClass::Residential, 0.0, 0.0, 1),
                ("bd", "b", "d", 900.0, RoadClass::Residential, 0.0, 0.0, 1),
            ],
        )
    };
    let free_flow = FreeFlow::default();
    for (beta, expected, separated, mixed) in
        [(0.2, ["oa", "ad"], 2000.0, 0.0), (0.4, ["ob", "bd"], 0.0, 1800.0)]
    {
        let rows = vec![trip(&net(), "t", "c", ("o", "d"), 25_200, Some(Mode::Bike))];
        let k = Coefficients { bike_separated_per_kilometre: beta, ..Coefficients::time_based() };
        let c = case(net(), rows, &[], k, &[]);
        let (state, _) =
            assign(&inputs(&c, &HighestUtility, &free_flow), &exactly(1), &mut NoHooks)
                .expect("run");
        let chain = state.trip(TripId::new(0)).current_chain().expect("a chain");
        let leg = &chain.legs()[0];
        let graph = c.bike.network();
        let names: Vec<&str> =
            leg.links.iter().map(|&l| graph.link_external_ids().external(l.raw())).collect();
        assert_eq!(names, expected, "β_separated {beta}");
        let a = chain.total();
        assert_eq!(
            (a.bike_separated_metres, a.bike_mixed_metres, a.bike_lane_metres),
            (separated, mixed, 0.0)
        );
        let cost = c.classes[0].chain_cost(chain);
        let derived = a.in_vehicle_seconds / 60.0 + beta * separated / 1000.0;
        assert!((cost - derived).abs() < 1e-12, "cost {cost} against {derived}");
    }
}

// --- 10: the stationarity measure by mode -------------------------------------------------------

/// Test 1's case at free flow, the logit, 6 000 trips: iteration 0 puts every trip on a draw from
/// `P = (0.5352, 0.4382, 0.0266)` (car, bike, walk), and the costs never change, so iteration 1
/// starts at a stationary state. Derived: the expected change of mode `m`'s share is
/// `D_m = P_m − n_m / N` (`n_m` the trips holding mode `m` after iteration 0), mean 0 and
/// standard deviation `√(P_m (1 − P_m) / N)`; the three sum to 0. The expected switch excess
/// grouped by the mode held would read `Σ P² − P_m` for every trip of the group: 0.4525 for the
/// walkers, −0.0561 for the drivers, not 0 at this stationary state.
#[test]
fn the_expected_change_of_each_modes_share_is_zero_at_a_stationary_state() {
    let n = 6000;
    let net = three_modes(7200.0, false);
    let rows = trips_od(&net, n, n, "c", "t");
    let modes = [Mode::Car, Mode::Bike, Mode::Walk];
    let c = case(three_modes(7200.0, false), rows, &[], per_minute(), &modes);
    let free_flow = FreeFlow::default();
    let model = logit();
    let (after0, _) =
        assign(&inputs(&c, &model, &free_flow), &exactly(1), &mut NoHooks).expect("run");
    let (_, reports) =
        assign(&inputs(&c, &model, &free_flow), &exactly(2), &mut NoHooks).expect("run");
    let p = logit_shares(&[-1.0, -1.2, -4.0]);
    let d = reports[1].mode_share_change_expected;
    let mut sum = 0.0;
    for (k, m) in modes.into_iter().enumerate() {
        let value = d.of(m).expect("trips face a choice");
        let derived = p[k] - taking(&after0, m) as f64 / f64::from(n);
        let sigma = (p[k] * (1.0 - p[k]) / f64::from(n)).sqrt();
        println!("{}: D {value:.5}, derived {derived:.5}, 3σ {:.5}", m.as_str(), 3.0 * sigma);
        assert!((value - derived).abs() < 1e-12, "{}: {value} against {derived}", m.as_str());
        assert!(value.abs() <= 3.0 * sigma, "{}: {value} beyond 3σ {}", m.as_str(), 3.0 * sigma);
        sum += value;
    }
    assert!(sum.abs() < 1e-12, "the three sum to 0");
    let squares: f64 = p.iter().map(|x| x * x).sum();
    assert!((squares - p[2] - 0.452_48).abs() < 1e-5 && (squares - p[0] + 0.056_06).abs() < 1e-5);
    println!(
        "grouped by the mode held, the walkers' expected excess would read {:.4}",
        squares - p[2]
    );
}
