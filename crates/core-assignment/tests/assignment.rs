//! The assignment loop at free flow, checked against values derived by hand from each case's
//! own constants (lengths, speeds, the averaging rule's formula), never against a run's output.
//!
//! | Test | Case | What is derived |
//! |---|---|---|
//! | 1 | a 3 × 3 grid, 1 000 m blocks | the corner-to-corner route's time and that its links form a path |
//! | 1b | the toy network | the only route from `N2` to `D1`, its links and its time |
//! | 2 | the 3 × 3 grid, nine trips | a second iteration changes nothing: found 0, switch share 0, gaps 0, the same state |
//! | 3 | the toy network, 50 trips | departed = arrived + unfinished (weighted), who is unfinished at the loading's end |
//! | 4 | the grid, and a diamond of two roads | money on the links weighed 0 changes nothing; a toll that outweighs the detour moves the choice |
//! | 5 | the diamond with two equal roads, 2 000 travellers | the switch share against its floor `α_i · ½`, the split ½, the deterministic even split by trip id |
//! | 5c | two diamonds, three trips | the expected switch excess: one trip off its best beyond the band, two on a tie: ⅓ |
//! | 5b | batches built by hand | the indifference band's ties and probabilities, the even split by trip id, turned by the iteration |
//! | 6 | as 5 | the same seed gives the same run, another seed another logit choice |
//! | stop | the grid, nine trips | the loop stops at the first check its rule allows, or at the most iterations |

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::float_cmp,
    reason = "test fixtures: small counts, and exact comparisons of values that must be equal to the bit"
)]

use openmobisim_core_assignment::{
    CarPlanner, Chain, Coefficients, FreeFlow, HighestUtility, Inputs, IterationReport, Params,
    Place, StageHook, State, assign, logit,
};
use openmobisim_core_choice::ChoiceModel;
use openmobisim_core_demand::{
    ClassDefaults, Ownership, RawTrip, Travellers, Trips, build_travellers,
};
use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
use openmobisim_core_graph::examples::{manhattan_grid, node_name, toy_network};
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::network::{LinkSpec, RoadNetwork, RoadNetworkBuilder};
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::{EntityId, LinkId, NodeId, TripId};
use openmobisim_core_types::rng::RngKey;
use openmobisim_core_types::time::Second;

// --- the cases' constants ---------------------------------------------------------------

/// The free-flow speed of a secondary road in the shipped defaults, km/h (the grid's links).
const SECONDARY_KM_H: f64 = 50.0;
/// The free-flow speed of a residential street in the shipped defaults, km/h (the toy network's
/// and the diamond's links).
const RESIDENTIAL_KM_H: f64 = 30.0;
/// The grid's block length, metres.
const BLOCK_M: f64 = 1000.0;

fn metres_per_second(km_h: f64) -> f64 {
    km_h / 3.6
}

// --- helpers ------------------------------------------------------------------------------

/// A network and everything an assignment on it needs.
struct Case {
    net: RoadNetwork,
    turns: TurnTable,
    travellers: Travellers,
    trips: Trips,
    classes: Vec<Coefficients>,
    money: Vec<f64>,
    loader: FreeFlow,
}

/// A trip of one traveller between two nodes of `net`.
fn trip(net: &RoadNetwork, id: &str, from: &str, to: &str, departure: u32, weight: u32) -> RawTrip {
    let at = |name: &str| net.node_lonlat(node(net, name));
    RawTrip {
        traveller_id: id.to_string(),
        trip_seq: 0,
        origin: at(from),
        destination: at(to),
        departure_time: Second(departure),
        user_class: "everyone".to_string(),
        weight: Some(weight),
        mode: None,
    }
}

fn node(net: &RoadNetwork, name: &str) -> NodeId {
    net.node_external_ids().typed_id_of(name).expect("a node of the case")
}

fn link(net: &RoadNetwork, name: &str) -> LinkId {
    net.link_external_ids().typed_id_of(name).expect("a link of the case")
}

fn link_names(net: &RoadNetwork, links: &[LinkId]) -> Vec<String> {
    links.iter().map(|&l| net.link_external_ids().external(l.raw()).to_string()).collect()
}

fn case(net: RoadNetwork, trips: Vec<RawTrip>) -> Case {
    let owners = ClassDefaults::new().with_fallback(Ownership::VEHICLES);
    let (travellers, trips) =
        build_travellers(trips, Vec::new(), &owners, 1, &mut Diagnostics::new()).expect("demand");
    let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);
    let money = vec![0.0; net.link_count() as usize];
    Case {
        net,
        turns,
        travellers,
        trips,
        classes: vec![Coefficients::time_based()],
        money,
        loader: FreeFlow::default(),
    }
}

fn inputs<'a>(
    c: &'a Case,
    model: &'a dyn ChoiceModel,
    seed: u64,
    initial: Option<State>,
) -> Inputs<'a> {
    Inputs {
        network: &c.net,
        turns: &c.turns,
        travellers: &c.travellers,
        trips: &c.trips,
        classes: &c.classes,
        money_per_link: &c.money,
        model,
        loader: &c.loader,
        seed: RngKey::from_seed(seed),
        initial,
    }
}

/// Run exactly `iterations` iterations.
fn exactly(iterations: u32) -> Params {
    Params { min_iterations: iterations, max_iterations: iterations, ..Params::default() }
}

/// Every iteration's state after its measure stage.
#[derive(Default)]
struct Recorder {
    states: Vec<State>,
}

impl StageHook for Recorder {
    fn after_measure(&mut self, _iteration: u32, state: &State) {
        self.states.push(state.clone());
    }
}

fn run(
    c: &Case,
    model: &dyn ChoiceModel,
    params: &Params,
    seed: u64,
    initial: Option<State>,
) -> (State, Vec<IterationReport>, Vec<State>) {
    let mut recorder = Recorder::default();
    let (state, reports) =
        assign(&inputs(c, model, seed, initial), params, &mut recorder).expect("the run");
    (state, reports, recorder.states)
}

fn current(state: &State, trip: u32) -> &Chain {
    state.trip(TripId::new(trip)).current_chain().expect("a chain")
}

/// Whether `links` lead from `from` to `to`, each beginning where the last ended.
fn is_path(net: &RoadNetwork, links: &[LinkId], from: NodeId, to: NodeId) -> bool {
    !links.is_empty()
        && net.link_from(links[0]) == from
        && net.link_to(*links.last().expect("non-empty")) == to
        && links.windows(2).all(|w| net.link_to(w[0]) == net.link_from(w[1]))
}

// --- 1. the cheapest route by hand --------------------------------------------------------

/// The grid's north-west corner is row 2, column 0 (rows count north, columns east); the
/// south-east corner row 0, column 2. Every route between them has at least four links (two
/// south, two east), and a route of four links is a monotone staircase; any other route has at
/// least six links of at least 1 000 m, more than any staircase's four. So the cheapest route is
/// the staircase of least time, each link costing its stated length over 50 km/h.
#[test]
fn the_cheapest_route_on_a_grid_is_the_shortest_staircase() {
    let (net, _) = manhattan_grid(3, BLOCK_M, false);
    let c = case(
        net,
        vec![trip(
            &manhattan_grid(3, BLOCK_M, false).0,
            "t0",
            &node_name(2, 0),
            &node_name(0, 2),
            0,
            1,
        )],
    );
    let net = &c.net;
    // The least time over the staircases, by dynamic programming over the grid's nodes: a
    // node's least time is the lesser of arriving from the north or from the west.
    let v = metres_per_second(SECONDARY_KM_H);
    let time_of = |a: (u32, u32), z: (u32, u32)| {
        let l = link(net, &format!("l_{}__{}", node_name(a.0, a.1), node_name(z.0, z.1)));
        net.link_length(l).get() / v
    };
    let mut best = [[f64::INFINITY; 3]; 3];
    best[2][0] = 0.0;
    for r in (0..3).rev() {
        for col in 0..3 {
            if r < 2 {
                best[r][col] = best[r][col].min(
                    best[r + 1][col] + time_of((r as u32 + 1, col as u32), (r as u32, col as u32)),
                );
            }
            if col > 0 {
                best[r][col] = best[r][col].min(
                    best[r][col - 1] + time_of((r as u32, col as u32 - 1), (r as u32, col as u32)),
                );
            }
        }
    }
    let derived = best[0][2];

    let (state, reports, _) = run(&c, &HighestUtility, &exactly(1), 1, None);
    let chain = current(&state, 0);
    let links = &chain.legs()[0].links;
    let simulated = chain.total().in_vehicle_seconds;
    println!("grid corner to corner: derived {derived:.9} s, simulated {simulated:.9} s");
    assert!((simulated - derived).abs() < 1e-6, "derived {derived}, simulated {simulated}");
    assert_eq!(links.len(), 4, "a staircase of two links south and two east");
    assert!(is_path(net, links, node(net, &node_name(2, 0)), node(net, &node_name(0, 2))));
    // Against the Manhattan distance over the speed: the grid's links are 0.12 % to 0.50 %
    // longer than the block the fixture is asked for (its flat approximation of degrees, against
    // the network's projection), so the two agree to that and no closer.
    let manhattan = 4.0 * BLOCK_M / v;
    println!("Manhattan distance over the speed: {manhattan:.9} s");
    assert!((simulated / manhattan - 1.0).abs() < 0.006);
    assert_eq!(reports[0].on_cheapest_share, 1.0);
}

/// The toy network's only route from `N2` to `D1` is `m1, a3, c1, c2, c3, a4`: lengths of 300,
/// 400, 3 × 4 and 300 m, 1 012 m of residential street at 30 km/h, no signal on the way (`M`
/// merges, `S` is the only signal), so 1 012 m / (30 km/h) = 121.44 s.
#[test]
fn the_toy_network_route_is_the_one_drawn() {
    let (net, _) = toy_network();
    let c = case(net, vec![trip(&toy_network().0, "t0", "N2", "D1", 100, 1)]);
    let derived = (300.0 + 400.0 + 3.0 * 4.0 + 300.0) / metres_per_second(RESIDENTIAL_KM_H);
    let (state, _, _) = run(&c, &HighestUtility, &exactly(1), 1, None);
    let leg = &current(&state, 0).legs()[0];
    println!(
        "toy N2 to D1: derived {derived:.9} s, simulated {:.9} s",
        leg.attributes.in_vehicle_seconds
    );
    assert_eq!(link_names(&c.net, &leg.links), ["m1", "a3", "c1", "c2", "c3", "a4"]);
    assert!((leg.attributes.in_vehicle_seconds - derived).abs() < 1e-9);
    assert!((leg.attributes.length_metres - 1012.0).abs() < 1e-9);
}

// --- 2. idempotence at free flow ----------------------------------------------------------

fn grid_trips(net: &RoadNetwork) -> Vec<RawTrip> {
    let pairs = [
        ((2, 0), (0, 2)),
        ((0, 0), (2, 2)),
        ((1, 0), (1, 2)),
        ((0, 1), (2, 1)),
        ((2, 2), (0, 0)),
        ((1, 1), (0, 2)),
        ((0, 2), (2, 0)),
        ((2, 1), (1, 0)),
        ((1, 2), (0, 1)),
    ];
    pairs
        .iter()
        .enumerate()
        .map(|(i, &((a, b), (y, z)))| {
            trip(
                net,
                &format!("t{i:02}"),
                &node_name(a, b),
                &node_name(y, z),
                60 * i as u32,
                1 + i as u32 % 3,
            )
        })
        .collect()
}

/// At free flow the link times never change, so the second iteration's search finds the routes
/// the first found, everyone keeps them, and the state is what it was.
#[test]
fn a_second_iteration_at_free_flow_changes_nothing() {
    let (net, _) = manhattan_grid(3, BLOCK_M, false);
    let c = case(net, grid_trips(&manhattan_grid(3, BLOCK_M, false).0));
    let (_, reports, states) = run(&c, &HighestUtility, &exactly(2), 7, None);
    assert_eq!(reports.len(), 2);
    let first = &reports[0];
    assert_eq!(first.found, 9, "every trip finds its route in iteration 0");
    assert_eq!(first.on_cheapest_share, 1.0, "everyone on their cheapest route at iteration 0");
    let second = &reports[1];
    println!("iteration 1: {second:?}");
    assert_eq!(second.found, 0);
    assert_eq!(second.switch_share, 0.0);
    assert_eq!(second.duality_gap, 0.0);
    assert_eq!(second.gap_deterministic, 0.0);
    assert_eq!(states[1], states[0], "the state after iteration 1 is the state after iteration 0");
}

// --- 3. conservation ----------------------------------------------------------------------

/// Fifty trips from `N2` to `D1` (121.44 s each, test 1b) departing at scattered seconds of an
/// hour, weighing 1 to 4, on a loading that ends at 1 800 s: a trip is unfinished exactly when
/// it departs after 1 800 − 121.44 s.
#[test]
fn every_trip_that_departs_arrives_or_is_unfinished() {
    let (net, _) = toy_network();
    let t = (300.0 + 400.0 + 3.0 * 4.0 + 300.0) / metres_per_second(RESIDENTIAL_KM_H);
    let end = 1800.0;
    let departures: Vec<(u32, u32)> = (0..50_u32).map(|i| ((i * 7919) % 3600, 1 + i % 4)).collect();
    let raw = departures
        .iter()
        .enumerate()
        .map(|(i, &(d, w))| trip(&net, &format!("t{i:02}"), "N2", "D1", d, w))
        .collect();
    let mut c = case(toy_network().0, raw);
    c.loader = FreeFlow { end_seconds: end };
    let departed: f64 = departures.iter().map(|&(_, w)| f64::from(w)).sum();
    let arrived: f64 = departures
        .iter()
        .filter(|&&(d, _)| f64::from(d) + t <= end)
        .map(|&(_, w)| f64::from(w))
        .sum();
    let (_, reports, _) = run(&c, &HighestUtility, &exactly(2), 3, None);
    for r in &reports {
        println!(
            "iteration {}: departed {}, arrived {}, unfinished {}",
            r.iteration, r.departed, r.arrived, r.unfinished
        );
        assert_eq!(r.departed, departed);
        assert_eq!(r.arrived, arrived);
        assert_eq!(r.unfinished, departed - arrived);
        assert_eq!(r.departed, r.arrived + r.unfinished);
        assert!((r.unfinished_share - (departed - arrived) / departed).abs() < 1e-12);
        assert!((r.mean_trip_seconds - t).abs() < 1e-9);
    }
    assert!(arrived > 0.0 && arrived < departed, "the case has trips of both kinds");
}

// --- 4. one code path for cost ------------------------------------------------------------

/// The same grid trips with money on every link, weighed 0, and with no money at all: the same
/// chains, probabilities and reports.
#[test]
fn money_weighed_zero_changes_nothing() {
    let (net, _) = manhattan_grid(3, BLOCK_M, false);
    let plain = case(net, grid_trips(&manhattan_grid(3, BLOCK_M, false).0));
    let mut tolled =
        case(manhattan_grid(3, BLOCK_M, false).0, grid_trips(&manhattan_grid(3, BLOCK_M, false).0));
    tolled.money = (0..tolled.net.link_count()).map(|l| 0.5 + f64::from(l % 5)).collect();
    tolled.classes = vec![Coefficients { per_money: 0.0, ..Coefficients::time_based() }];
    let model = logit();
    let (a, ra, _) = run(&plain, &model, &exactly(3), 11, None);
    let (b, rb, _) = run(&tolled, &model, &exactly(3), 11, None);
    let times =
        |r: &[IterationReport]| r.iter().map(IterationReport::without_times).collect::<Vec<_>>();
    assert_eq!(times(&ra), times(&rb));
    for (x, y) in a.trips().iter().zip(b.trips()) {
        assert_eq!(x.current, y.current);
        assert_eq!(x.probabilities, y.probabilities);
        assert_eq!(x.set.len(), y.set.len());
        for (p, q) in x.set.iter().zip(&y.set) {
            assert_eq!(p.identity(), q.identity());
            assert!(p.same_alternative(q));
            assert_eq!(p.total().in_vehicle_seconds, q.total().in_vehicle_seconds);
            assert_eq!(plain.classes[0].utility(p.total()), tolled.classes[0].utility(q.total()));
        }
    }
}

/// Two roads from `A` to `D`: the top one through `B`, 2 × `top_m`, the bottom one through `C`,
/// 2 × `bottom_m`, residential streets at 30 km/h.
fn diamond(top_m: f64, bottom_m: f64) -> RoadNetwork {
    let mut b = RoadNetworkBuilder::new();
    for (n, x, y) in [("A", 0.0, 0.0), ("B", 100.0, 60.0), ("C", 100.0, -60.0), ("D", 200.0, 0.0)] {
        b.add_node(n, LonLat::new(4.8 + x / 77_800.0, 45.7 + y / 110_574.0));
    }
    for (name, from, to, len) in [
        ("ab", "A", "B", top_m),
        ("bd", "B", "D", top_m),
        ("ac", "A", "C", bottom_m),
        ("cd", "C", "D", bottom_m),
    ] {
        let mut spec = LinkSpec::new(RoadClass::Residential);
        spec.length_m = Some(len);
        b.add_link(name, from, to, spec);
    }
    b.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
        .expect("buildable")
}

/// The top road is 200 m (24 s, 0.4 minute), the bottom one 600 m (72 s, 1.2 minutes); a toll of
/// 5 on `ab` at 1 per unit of money makes the top road cost 0.4 + 5 = 5.4, more than the bottom
/// road's 1.2: the bottom road is taken. Without the toll's weight, the top road is.
#[test]
fn a_toll_that_outweighs_the_detour_moves_the_choice() {
    let (top, bottom) = (100.0, 300.0);
    let minutes = |m: f64| 2.0 * m / metres_per_second(RESIDENTIAL_KM_H) / 60.0;
    let toll = 5.0;
    let per_money = 1.0;
    assert!(minutes(top) + per_money * toll > minutes(bottom), "the case is as described");
    let raw = |net: &RoadNetwork| vec![trip(net, "t0", "A", "D", 0, 1)];
    for (weight, expected) in [(per_money, ["ac", "cd"]), (0.0, ["ab", "bd"])] {
        let net = diamond(top, bottom);
        let mut c = case(diamond(top, bottom), raw(&net));
        c.money[link(&c.net, "ab").index()] = toll;
        c.classes = vec![Coefficients { per_money: weight, ..Coefficients::time_based() }];
        let (state, _, _) = run(&c, &HighestUtility, &exactly(1), 1, None);
        let leg = &current(&state, 0).legs()[0];
        assert_eq!(link_names(&c.net, &leg.links), expected, "money weighed {weight}");
        let cost = c.classes[0].cost(current(&state, 0).total());
        let derived = if weight > 0.0 { minutes(bottom) } else { minutes(top) };
        assert!((cost - derived).abs() < 1e-9, "cost {cost}, derived {derived}");
    }
}

// --- 5. the averaging rule and its floor --------------------------------------------------

const TRAVELLERS: u32 = 2000;

/// The diamond with two equal roads of 2 × 100 m, 2 000 travellers from `A` to `D`, each trip's
/// set holding both roads in the order given (`first` then the other).
fn two_equal_roads(first: &str) -> (Case, State) {
    let net = diamond(100.0, 100.0);
    let raw =
        (0..TRAVELLERS).map(|i| trip(&net, &format!("t{i:04}"), "A", "D", i % 600, 1)).collect();
    let c = case(diamond(100.0, 100.0), raw);
    let mut state = State::initial(&c.net, &c.travellers, &c.trips);
    let planner = CarPlanner::new(&c.net, &c.turns, &c.classes, &c.money).expect("planner");
    let (a, d) = (Place::Road(node(&c.net, "A")), Place::Road(node(&c.net, "D")));
    let roads =
        if first == "top" { [["ab", "bd"], ["ac", "cd"]] } else { [["ac", "cd"], ["ab", "bd"]] };
    for t in 0..c.trips.len() {
        let trip = TripId::new(t);
        let departure = f64::from(c.trips.departure(trip).get());
        for road in roads {
            let links = road.iter().map(|n| link(&c.net, n)).collect();
            let leg = planner.leg_along(a, d, departure, links, &state);
            assert!(state.trip_mut(trip).add(Chain::new(vec![leg])));
        }
    }
    (c, state)
}

/// The share of trips on the top road.
fn on_top(c: &Case, state: &State) -> f64 {
    let ab = link(&c.net, "ab");
    let n = state
        .trips()
        .iter()
        .filter(|t| t.current_chain().expect("chosen").legs()[0].links[0] == ab)
        .count();
    n as f64 / f64::from(TRAVELLERS)
}

/// With both roads in every set and equal utility, the logit gives each probability ½. In
/// iteration `i ≥ 1` a traveller re-chooses with probability `α_i = 1 / (i + 4)` and then
/// switches with probability ½, so the switch share is binomial with mean `α_i / 2`, the floor
/// itself; the share on either road stays binomial with mean ½.
#[test]
fn the_switch_share_sits_on_its_floor_and_the_split_is_even() {
    let (c, initial) = two_equal_roads("top");
    let iterations = 6;
    let model = logit();
    let (_, reports, states) = run(&c, &model, &exactly(iterations), 2026, Some(initial));
    let n = f64::from(TRAVELLERS);
    for r in &reports {
        let share = on_top(&c, &states[r.iteration as usize]);
        let sigma_split = (0.25 / n).sqrt();
        println!(
            "iteration {}: on the top road {share:.4} (½ ± 3σ = {:.4}); switch share {:.4}, floor {:.4}",
            r.iteration,
            3.0 * sigma_split,
            r.switch_share,
            r.switch_floor
        );
        assert!(
            (share - 0.5).abs() <= 3.0 * sigma_split,
            "iteration {}: split {share}",
            r.iteration
        );
        assert_eq!(r.found, 0, "both roads are in every set from the start");
        if r.iteration == 0 {
            continue;
        }
        let alpha = 1.0 / (f64::from(r.iteration) + 1.0 + 3.0);
        let p = alpha * 0.5;
        assert!(
            (r.switch_floor - p).abs() < 1e-12,
            "iteration {}: floor {}, derived {p}",
            r.iteration,
            r.switch_floor
        );
        let sigma = (p * (1.0 - p) / n).sqrt();
        assert!(
            (r.switch_share - p).abs() <= 3.0 * sigma,
            "iteration {}: switch share {}, floor {p} ± {}",
            r.iteration,
            r.switch_share,
            3.0 * sigma
        );
        assert_eq!(r.duality_gap, 0.0, "two equal roads: the paid cost is the least");
        assert!(r.gap_deterministic.abs() < 1e-12);
    }
}

/// The deterministic choice between two chains of equal utility splits the trips evenly, by
/// trip id: in iteration 0 every trip chooses, all 2 000 face the same tie, and the n-th by trip
/// id takes the tie's chain `n mod 2` in the order of the first trip's set (the order of every
/// set here). So trip `t` takes the set's first chain when `t` is even: exactly 1 000 trips on
/// each road, whichever road is first, each chain with probability ½. In iteration 1 a
/// traveller re-chooses with probability `α_1 = 1 / 5` and the floor is `α_1 · (1 − ¼ − ¼)`.
#[test]
fn the_deterministic_tie_is_split_evenly_by_trip_id() {
    for first in ["top", "bottom"] {
        let (c, initial) = two_equal_roads(first);
        let (_, reports, states) = run(&c, &HighestUtility, &exactly(2), 5, Some(initial));
        let after_0 = &states[0];
        for (t, entry) in after_0.trips().iter().enumerate() {
            assert_eq!(entry.current, Some(t % 2), "trip {t}: the {}-th of the tie", t % 2);
            assert_eq!(entry.probabilities, vec![0.5, 0.5]);
        }
        assert_eq!(on_top(&c, after_0), 0.5, "1 000 trips on each road");
        assert_eq!(reports[0].switch_floor, 1.0, "no trip had a chain: all switch");
        let alpha = 1.0 / (1.0 + 1.0 + 3.0);
        assert!((reports[1].switch_floor - alpha * 0.5).abs() < 1e-12);
        println!(
            "first {first}: iteration 1 switch share {:.4}, floor {:.4}, excess {:.4}; on the \
             top road after it {:.4}",
            reports[1].switch_share,
            reports[1].switch_floor,
            reports[1].switch_excess,
            on_top(&c, &states[1])
        );
    }
}

/// Two diamonds side by side: `A → D` by a top road of 2 × 100 m or a bottom one of 2 × 300 m,
/// and `E → H` by two equal roads of 2 × 100 m, all residential streets.
fn two_diamonds() -> RoadNetwork {
    let mut b = RoadNetworkBuilder::new();
    for (n, x, y) in [
        ("A", 0.0, 0.0),
        ("B", 100.0, 60.0),
        ("C", 100.0, -60.0),
        ("D", 200.0, 0.0),
        ("E", 0.0, 1000.0),
        ("F", 100.0, 1060.0),
        ("G", 100.0, 940.0),
        ("H", 200.0, 1000.0),
    ] {
        b.add_node(n, LonLat::new(4.8 + x / 77_800.0, 45.7 + y / 110_574.0));
    }
    for (name, from, to, len) in [
        ("ab", "A", "B", 100.0),
        ("bd", "B", "D", 100.0),
        ("ac", "A", "C", 300.0),
        ("cd", "C", "D", 300.0),
        ("ef", "E", "F", 100.0),
        ("fh", "F", "H", 100.0),
        ("eg", "E", "G", 100.0),
        ("gh", "G", "H", 100.0),
    ] {
        let mut spec = LinkSpec::new(RoadClass::Residential);
        spec.length_m = Some(len);
        b.add_link(name, from, to, spec);
    }
    b.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
        .expect("buildable")
}

/// The expected switch excess, `Σ w (Σ_k P_k² − P_current) / Σ w`, on three trips under the
/// deterministic choice: trip 0 (`A → D`) holds the bottom road, 1.2 min against the top
/// road's 0.4 (beyond the 1 % band), so `P = (1, 0)` and it contributes `1 − 0 = 1`; trips 1 and
/// 2 (`E → H`) face a two-way tie, `P = (½, ½)`, each on one of the tied roads, contributing
/// `½ − ½ = 0`. The excess is `(1 + 0 + 0) / 3 = ⅓`: the share of trips off their best
/// beyond the band.
#[test]
fn the_expected_excess_is_the_share_of_trips_off_their_best_beyond_the_band() {
    let net = two_diamonds();
    let raw = vec![
        trip(&net, "t0", "A", "D", 0, 1),
        trip(&net, "t1", "E", "H", 0, 1),
        trip(&net, "t2", "E", "H", 0, 1),
    ];
    let c = case(two_diamonds(), raw);
    let mut state = State::initial(&c.net, &c.travellers, &c.trips);
    let planner = CarPlanner::new(&c.net, &c.turns, &c.classes, &c.money).expect("planner");
    let roads: [(&str, &str, [[&str; 2]; 2], usize); 3] = [
        ("A", "D", [["ab", "bd"], ["ac", "cd"]], 1),
        ("E", "H", [["ef", "fh"], ["eg", "gh"]], 0),
        ("E", "H", [["ef", "fh"], ["eg", "gh"]], 1),
    ];
    for (t, (from, to, pair, held)) in roads.into_iter().enumerate() {
        let (o, d) = (Place::Road(node(&c.net, from)), Place::Road(node(&c.net, to)));
        let trip = TripId::from_index(t);
        for road in pair {
            let links = road.iter().map(|n| link(&c.net, n)).collect();
            let leg = planner.leg_along(o, d, 0.0, links, &state);
            assert!(state.trip_mut(trip).add(Chain::new(vec![leg])));
        }
        state.trip_mut(trip).current = Some(held);
    }
    let (_, reports, _) = run(&c, &HighestUtility, &exactly(1), 1, Some(state));
    let derived = 1.0 / 3.0;
    println!("expected switch excess {}, derived {derived}", reports[0].switch_excess_expected);
    assert!((reports[0].switch_excess_expected - derived).abs() < 1e-12);
}

/// The deterministic model on batches built by hand, with the chooser's attributes.
mod deterministic_model {
    use openmobisim_core_assignment::HighestUtility;
    use openmobisim_core_assignment::chooser::{
        CHAIN_HIGH, CHAIN_LOW, INDIFFERENCE_BAND, LN_PATH_SIZE, UTILITY,
    };
    use openmobisim_core_choice::{ChoiceBatch, ChoiceModel};
    use openmobisim_core_types::rng::{RngKey, Stream, StreamRng};

    /// One situation per `(trip, chains)`, each chain `(identity, cost)`, in `iteration`.
    fn batch(iteration: u32, band: f64, situations: &[(u32, &[(u64, f64)])]) -> ChoiceBatch {
        let mut b = ChoiceBatch::new(
            iteration,
            &[UTILITY, LN_PATH_SIZE, INDIFFERENCE_BAND, CHAIN_HIGH, CHAIN_LOW],
        );
        for &(trip, chains) in situations {
            b.begin_situation(trip, trip);
            for (i, &(id, cost)) in chains.iter().enumerate() {
                let (high, low) = ((id >> 32) as f64, (id & 0xFFFF_FFFF) as f64);
                b.push_alternative(i as u32, &[-cost, 0.0, band, high, low]);
            }
        }
        b
    }

    fn rng() -> StreamRng {
        StreamRng::new(RngKey::from_seed(1), Stream::Choice)
    }

    /// Costs 10, 10.05 and 10.2 with a band of 1 %: the band reaches 10 · 1.01 = 10.1, so the
    /// first two tie and the third does not; each tied chain has probability ½. With a band of
    /// 3 % (10.3) all three tie, each ⅓; with no band only the least cost, probability 1.
    #[test]
    fn the_chains_within_the_band_tie_with_equal_probability() {
        let chains: &[(u64, f64)] = &[(7, 10.0), (8, 10.05), (9, 10.2)];
        for (band, derived) in [
            (0.01, vec![0.5, 0.5, 0.0]),
            (0.03, vec![1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0]),
            (0.0, vec![1.0, 0.0, 0.0]),
        ] {
            let p = HighestUtility.probabilities(&batch(0, band, &[(0, chains)])).unwrap().unwrap();
            assert_eq!(p, derived, "band {band}");
        }
    }

    /// Seven trips face the same tie of chains `a` and `b`, given in a shuffled trip order and
    /// with the two chains in either order in their sets; an eighth trip has a chain cheaper
    /// beyond the band and takes it. In iteration 0 the n-th trip by id takes the tie's chain
    /// `n mod 2` in the order of the first trip's set (`a`, `b`): trips 0, 2, 4, 6 take `a`,
    /// trips 1, 3, 5 take `b`, four and three. In iteration 1 the order turns by one: trips 0,
    /// 2, 4, 6 take `b`. The split depends on which chains tie, not on their identities' values.
    #[test]
    fn the_trips_facing_one_tie_are_split_evenly_by_trip_id() {
        let (a, b) = (0xAAAA_0000_0001_u64, 0x0000_BBBB_0002_u64);
        let ab: &[(u64, f64)] = &[(a, 100.0), (b, 100.5)];
        let ba: &[(u64, f64)] = &[(b, 100.4), (a, 100.0)];
        let clear: &[(u64, f64)] = &[(a, 100.0), (b, 90.0)];
        let trips: [(u32, &[(u64, f64)]); 8] =
            [(5, ba), (0, ab), (3, ab), (7, clear), (6, ba), (1, ba), (4, ab), (2, ab)];
        for (iteration, even_takes) in [(0, a), (1, b), (2, a)] {
            let batch = batch(iteration, 0.01, &trips);
            let choices = HighestUtility.choose(&batch, &rng()).unwrap();
            let mut taken = [0_u32; 2];
            for (s, &(trip, chains)) in trips.iter().enumerate() {
                let id = chains[choices.chosen[s] as usize].0;
                if trip == 7 {
                    assert_eq!(id, b, "the chain cheaper beyond the band");
                    assert_eq!(choices.probability[s], 1.0);
                    continue;
                }
                let odd_takes = if even_takes == a { b } else { a };
                let wanted = if trip % 2 == 0 { even_takes } else { odd_takes };
                assert_eq!(id, wanted, "iteration {iteration}, trip {trip}");
                assert_eq!(choices.probability[s], 0.5);
                taken[usize::from(id == b)] += 1;
            }
            let (on_a, on_b) = (taken[0], taken[1]);
            assert_eq!(on_a.max(on_b), 4);
            assert_eq!(on_a.min(on_b), 3);
        }
        // The same chains under other identities: the same picks, by position.
        let (x, y) = (3_u64, 0xFFFF_FFFF_FFFF_u64);
        let renamed = |c: &[(u64, f64)]| -> Vec<(u64, f64)> {
            c.iter().map(|&(id, cost)| (if id == a { x } else { y }, cost)).collect()
        };
        let lists: Vec<(u32, Vec<(u64, f64)>)> =
            trips.iter().map(|&(t, c)| (t, renamed(c))).collect();
        let again: Vec<(u32, &[(u64, f64)])> =
            lists.iter().map(|(t, c)| (*t, c.as_slice())).collect();
        let before = HighestUtility.choose(&batch(0, 0.01, &trips), &rng()).unwrap();
        let after = HighestUtility.choose(&batch(0, 0.01, &again), &rng()).unwrap();
        assert_eq!(before.chosen, after.chosen);
    }

    /// A batch without the band and the identities (made by hand for another use): only equal
    /// utilities tie, and the alternatives are known by their identity in the batch.
    #[test]
    fn a_batch_without_the_band_ties_only_equal_utilities() {
        let mut b = ChoiceBatch::new(0, &[UTILITY]);
        for trip in 0..4 {
            b.begin_situation(trip, trip);
            b.push_alternative(0, &[-10.0]);
            b.push_alternative(1, &[-10.0]);
            b.push_alternative(2, &[-10.01]);
        }
        let choices = HighestUtility.choose(&b, &rng()).unwrap();
        assert_eq!(choices.chosen, vec![0, 1, 0, 1]);
        let p = HighestUtility.probabilities(&b).unwrap().unwrap();
        assert_eq!(&p[..3], &[0.5, 0.5, 0.0]);
    }
}

// --- 6. reproducibility -------------------------------------------------------------------

#[test]
fn the_same_seed_gives_the_same_run_and_another_seed_another_choice() {
    let model = logit();
    let (c, initial) = two_equal_roads("top");
    let (a, ra, _) = run(&c, &model, &exactly(4), 99, Some(initial.clone()));
    let (b, rb, _) = run(&c, &model, &exactly(4), 99, Some(initial.clone()));
    let times =
        |r: &[IterationReport]| r.iter().map(IterationReport::without_times).collect::<Vec<_>>();
    assert_eq!(times(&ra), times(&rb));
    assert_eq!(a, b);
    let (other, _, _) = run(&c, &model, &exactly(4), 100, Some(initial));
    let differ =
        a.trips().iter().zip(other.trips()).filter(|(x, y)| x.current != y.current).count();
    println!("trips choosing differently under another seed: {differ} of {TRAVELLERS}");
    assert!(differ > 0);
}

// --- the stopping rule --------------------------------------------------------------------

/// At free flow under the deterministic model nothing new is found after iteration 0 and nobody
/// switches, so the first check that may stop, at the search of iteration `min_iterations`,
/// does: `min_iterations` iterations are reported. A tolerance no share can meet runs to
/// `max_iterations`.
#[test]
fn the_loop_stops_at_the_first_check_the_rule_allows() {
    let (net, _) = manhattan_grid(3, BLOCK_M, false);
    let c = case(net, grid_trips(&manhattan_grid(3, BLOCK_M, false).0));
    let params = Params::default();
    let (_, reports, _) = run(&c, &HighestUtility, &params, 1, None);
    assert_eq!(reports.len(), params.min_iterations as usize);
    let never = Params { found_tolerance: -1.0, max_iterations: 7, ..Params::default() };
    let (_, reports, _) = run(&c, &HighestUtility, &never, 1, None);
    assert_eq!(reports.len(), 7);
}
