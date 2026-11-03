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
//! | 5 | the diamond with two equal roads, 2 000 travellers | the switch share against its floor `α_i · ½`, the split ½, the deterministic tie rule |
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

/// The deterministic choice between two chains of exactly equal utility takes the first in the
/// set, the one found earliest, whichever road that is.
#[test]
fn the_deterministic_tie_goes_to_the_first_chain_of_the_set() {
    for (first, top_share) in [("top", 1.0), ("bottom", 0.0)] {
        let (c, initial) = two_equal_roads(first);
        let (state, reports, _) = run(&c, &HighestUtility, &exactly(2), 5, Some(initial));
        assert_eq!(on_top(&c, &state), top_share, "the first chain is the {first} road");
        assert!(state.trips().iter().all(|t| t.current == Some(0)));
        assert_eq!(reports[1].switch_share, 0.0);
        assert_eq!(
            reports[1].switch_floor, 0.0,
            "the deterministic model gives the kept chain probability 1"
        );
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
