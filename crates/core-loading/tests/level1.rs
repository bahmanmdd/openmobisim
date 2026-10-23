//! Level 1: the static user-equilibrium assignment on networks small enough to solve by hand.
//!
//! One link: the BPR time of the whole demand, the gap 0 at once. Two parallel routes: the split
//! that equalises their times, found here by bisection on the equal-time condition, against
//! Frank–Wolfe (exact in one step on two routes) and the method of successive averages (close
//! after many); the route flows add up to the demand; an unreachable pair is reported and left
//! out; the solver's answer is the same for any number of threads because its work is split in
//! fixed chunks, which `core-routes` tests; here the flows are checked against the hand solution.

use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::network::{LinkSpec, RoadNetwork, RoadNetworkBuilder};
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_loading::{
    Bpr, StaticDemand, StaticOptions, StaticSolver, apportion, solve_static,
};
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::{EntityId, LinkId, NodeId};

/// `a → b` directly (one link, 1 000 PCU/h) and round by `c` (two links of 2 000 PCU/h each);
/// `z` an island no link reaches.
fn two_routes() -> RoadNetwork {
    let mut b = RoadNetworkBuilder::new();
    b.add_node("a", LonLat::new(4.800, 45.700));
    b.add_node("b", LonLat::new(4.810, 45.700));
    b.add_node("c", LonLat::new(4.805, 45.705));
    b.add_node("z", LonLat::new(4.820, 45.720));
    let spec = |capacity: f64| LinkSpec {
        capacity_veh_h: Some(capacity),
        length_m: Some(1000.0),
        maxspeed_km_h: Some(50.0),
        ..LinkSpec::new(RoadClass::Primary)
    };
    b.add_link("ab", "a", "b", spec(1000.0));
    b.add_link("ac", "a", "c", spec(2000.0));
    b.add_link("cb", "c", "b", spec(2000.0));
    let mut diagnostics = Diagnostics::new();
    b.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut diagnostics)
        .expect("buildable")
}

fn node(network: &RoadNetwork, name: &str) -> NodeId {
    network.node_external_ids().typed_id_of::<NodeId>(name).expect("known node")
}

fn link(network: &RoadNetwork, name: &str) -> LinkId {
    network.link_external_ids().typed_id_of::<LinkId>(name).expect("known link")
}

fn options(solver: StaticSolver, max_iterations: u32, tolerance: f64) -> StaticOptions {
    StaticOptions {
        bpr: Bpr::SHIPPED,
        solver,
        max_iterations,
        tolerance,
        capacity_factor: Vec::new(),
    }
}

fn bpr(t0: f64, flow: f64, capacity: f64) -> f64 {
    t0 * (1.0 + 0.15 * (flow / capacity).powi(4))
}

#[test]
fn one_route_gets_the_whole_demand_and_the_bpr_time_with_a_gap_of_zero() {
    let network = two_routes();
    let turns = TurnTable::build(&network, network.signals());
    // c → b is the only way from c to b.
    let demand =
        StaticDemand { origins: vec![(node(&network, "c"), vec![(node(&network, "b"), 1500.0)])] };
    let result =
        solve_static(&network, &turns, &demand, &options(StaticSolver::FrankWolfe, 50, 1e-6));
    let cb = link(&network, "cb");
    assert!((result.flows[cb.index()] - 1500.0).abs() < 1e-9);
    let t0 = network.free_flow_time(cb).get();
    let expected = bpr(t0, 1500.0, 2000.0);
    assert!(
        (result.times_s[cb.index()] - expected).abs() < 1e-9,
        "{} against {expected}",
        result.times_s[cb.index()]
    );
    assert_eq!(result.iterations.len(), 1, "the only route is the equilibrium at once");
    assert!(result.iterations[0].gap.abs() < 1e-12 && result.converged);
    assert_eq!(result.routes[0], vec![(vec![cb], 1500.0)]);
    assert!(result.unreachable.is_empty());
}

/// The flow on the direct link at which both routes take the same time, by bisection.
fn hand_split(network: &RoadNetwork, demand: f64) -> f64 {
    let (ab, ac, cb) = (link(network, "ab"), link(network, "ac"), link(network, "cb"));
    let t = |l: LinkId| network.free_flow_time(l).get();
    let direct = |x: f64| bpr(t(ab), x, 1000.0);
    let around = |x: f64| bpr(t(ac), demand - x, 2000.0) + bpr(t(cb), demand - x, 2000.0);
    let (mut lo, mut hi) = (0.0, demand);
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if direct(mid) < around(mid) { lo = mid } else { hi = mid }
    }
    0.5 * (lo + hi)
}

#[test]
fn two_routes_split_so_that_their_times_are_equal_under_either_solver() {
    let network = two_routes();
    let turns = TurnTable::build(&network, network.signals());
    let demand = 3000.0;
    let table =
        StaticDemand { origins: vec![(node(&network, "a"), vec![(node(&network, "b"), demand)])] };
    let (ab, ac, cb) = (link(&network, "ab"), link(&network, "ac"), link(&network, "cb"));
    let x_hand = hand_split(&network, demand);
    assert!(x_hand > 0.0 && x_hand < demand, "both routes are used at this demand: {x_hand}");

    let fw = solve_static(&network, &turns, &table, &options(StaticSolver::FrankWolfe, 100, 1e-9));
    assert!(fw.converged, "gap {}", fw.iterations.last().expect("one").gap);
    assert!(
        (fw.flows[ab.index()] - x_hand).abs() < 1e-3,
        "{} against {x_hand}",
        fw.flows[ab.index()]
    );
    assert!((fw.flows[ac.index()] - (demand - x_hand)).abs() < 1e-3);
    assert!(
        (fw.flows[ac.index()] - fw.flows[cb.index()]).abs() < 1e-9,
        "the round route's two links carry one flow"
    );
    assert!(
        (fw.times_s[ab.index()] - fw.times_s[ac.index()] - fw.times_s[cb.index()]).abs() < 1e-3,
        "equal times"
    );
    assert!(
        fw.iterations.len() <= 3,
        "exact line search on two routes: {} iterations",
        fw.iterations.len()
    );
    // The route flows: two routes, adding up to the demand, each carrying its link's flow.
    let routes = &fw.routes[0];
    assert_eq!(routes.len(), 2);
    let total: f64 = routes.iter().map(|(_, f)| f).sum();
    assert!((total - demand).abs() < 1e-9);
    for (links, flow) in routes {
        let on_link = fw.flows[links[0].index()];
        assert!((flow - on_link).abs() < 1e-9, "route flow {flow} is its link's {on_link}");
    }

    let msa = solve_static(&network, &turns, &table, &options(StaticSolver::Msa, 400, 1e-4));
    assert!(msa.converged, "gap {}", msa.iterations.last().expect("one").gap);
    assert!(
        (msa.flows[ab.index()] - x_hand).abs() < 0.01 * demand,
        "msa within 1% of the demand: {} against {x_hand}",
        msa.flows[ab.index()]
    );
    let msa_steps: Vec<f64> = msa.iterations.iter().take(4).map(|i| i.step).collect();
    assert_eq!(msa_steps, vec![1.0, 0.5, 1.0 / 3.0, 0.25], "the step 1/(k + 1)");
    assert!(msa.iterations[0].time_change.is_nan() && msa.iterations[1].time_change > 0.0);
    // Both solvers' totals agree to the tolerance's order.
    let (a, b) = (
        fw.iterations.last().expect("one").total_time,
        msa.iterations.last().expect("one").total_time,
    );
    assert!((a - b).abs() / a < 1e-3, "{a} against {b}");
}

#[test]
fn an_unreachable_pair_is_reported_and_its_flow_left_out_and_a_cap_of_one_is_the_free_flow_aon() {
    let network = two_routes();
    let turns = TurnTable::build(&network, network.signals());
    let table = StaticDemand {
        origins: vec![
            (
                node(&network, "a"),
                vec![
                    (node(&network, "z"), 100.0),
                    (node(&network, "b"), 500.0),
                    (node(&network, "a"), 7.0),
                ],
            ),
            (node(&network, "c"), vec![(node(&network, "b"), 200.0)]),
        ],
    };
    let result = solve_static(&network, &turns, &table, &options(StaticSolver::Msa, 1, 0.0));
    assert_eq!(result.unreachable, vec![0], "a → z is pair 0; a → a is skipped silently");
    assert_eq!(result.iterations.len(), 1);
    assert!(!result.converged, "a tolerance of 0 never says converged");
    let (ab, cb) = (link(&network, "ab"), link(&network, "cb"));
    // Everyone on the faster route at free flow (the direct link is the shorter route).
    assert!((result.flows[ab.index()] - 500.0).abs() < 1e-9);
    assert!((result.flows[cb.index()] - 200.0).abs() < 1e-9);
    assert!(result.routes[0].is_empty() && result.routes[2].is_empty());
    assert_eq!(result.routes[1].len(), 1);
    assert_eq!(result.routes.len(), 4);
}

#[test]
fn a_closed_link_is_not_used_and_a_narrowed_one_costs_more() {
    let network = two_routes();
    let turns = TurnTable::build(&network, network.signals());
    let table =
        StaticDemand { origins: vec![(node(&network, "a"), vec![(node(&network, "b"), 300.0)])] };
    let (ab, ac) = (link(&network, "ab"), link(&network, "ac"));
    let mut closed = options(StaticSolver::FrankWolfe, 20, 1e-6);
    closed.capacity_factor = vec![(ab, 0.0)];
    let result = solve_static(&network, &turns, &table, &closed);
    assert!(result.flows[ab.index()].abs() < f64::EPSILON, "nothing on the closed link");
    assert!((result.flows[ac.index()] - 300.0).abs() < 1e-9);
    // At 3 000 PCU/h both routes are used (the test above); half the direct link's capacity
    // sends more of them round.
    let busy =
        StaticDemand { origins: vec![(node(&network, "a"), vec![(node(&network, "b"), 3000.0)])] };
    let mut narrowed = options(StaticSolver::FrankWolfe, 20, 1e-6);
    narrowed.capacity_factor = vec![(ab, 0.5)];
    let open = solve_static(&network, &turns, &busy, &options(StaticSolver::FrankWolfe, 20, 1e-6));
    let narrow = solve_static(&network, &turns, &busy, &narrowed);
    assert!(narrow.flows[ab.index()] < open.flows[ab.index()], "less on the narrowed link");
}

#[test]
fn apportionment_over_a_pair_s_route_shares_fills_routes_in_order() {
    assert_eq!(apportion(&[600.0, 400.0], 5), vec![0, 0, 0, 1, 1]);
}

#[test]
fn a_link_bins_table_from_its_rows_is_in_bin_then_link_order() {
    // Level 1 writes its one period of flow straight into the table, rows in any order.
    let bins = openmobisim_core_loading::LinkBins::from_rows(
        900,
        vec![(29, 4, 2, 2.0, 120.0), (28, 7, 1, 1.5, 90.0), (28, 3, 3, 3.0, 30.0)],
    );
    assert_eq!(bins.bin_seconds(), 900);
    assert_eq!(bins.bins(), &[28, 28, 29]);
    assert_eq!(bins.links(), &[3, 7, 4]);
    assert_eq!(bins.crossings(), &[3, 1, 2]);
    assert!((bins.mean_seconds(2) - 60.0).abs() < 1e-12);
}
