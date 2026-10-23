//! Flow level 1 in a run (S294): the static user-equilibrium assignment, its results in the
//! run's shape.
//!
//! What is defended, and how each is checked:
//!
//! * **one link, exact by hand**: every trip's time is the BPR time of the whole hourly flow,
//!   the gap 0 at once, the per-link results the flow over the period;
//! * **two routes**: Frank–Wolfe splits the demand where the two routes' times are equal (by
//!   hand, bisection), the method of successive averages close to it; each trip is given a
//!   route, the counts by route are the equilibrium's to the trip, every trip arrives, the
//!   reports are the solver's iterations;
//! * **the same run on 1 thread and 7** gives the same results and fingerprint;
//! * **what level 1 cannot run is refused** (mode choice), and its fingerprint is another
//!   level's.

use std::num::NonZeroUsize;
use std::sync::Arc;

use openmobisim_core_demand::{ClassDefaults, Mode, Ownership, RawTrip, build_travellers};
use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::network::{LinkSpec, RoadNetwork, RoadNetworkBuilder};
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_sim::equilibration::{Options, strategy};
use openmobisim_core_sim::{FlowMotor, LoadingOptions, Run, RunError, RunResult};
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::{EntityId, LinkId, TripId};
use openmobisim_core_types::time::Second;

const O: (f64, f64) = (4.800, 45.700);
const D: (f64, f64) = (4.810, 45.700);

/// `o → d` directly on a one-lane residential street (`fast`), and round by `m` on a two-lane
/// primary road (`slow1`, `slow2`).
fn two_routes() -> RoadNetwork {
    let mut b = RoadNetworkBuilder::new();
    b.add_node("o", LonLat::new(O.0, O.1));
    b.add_node("m", LonLat::new(4.805, 45.7004));
    b.add_node("d", LonLat::new(D.0, D.1));
    let mut fast = LinkSpec::new(RoadClass::Residential);
    fast.lanes = Some(1);
    fast.length_m = Some(600.0);
    b.add_link("fast", "o", "d", fast);
    let mut slow = LinkSpec::new(RoadClass::Primary);
    slow.lanes = Some(2);
    slow.length_m = Some(700.0);
    b.add_link("slow1", "o", "m", slow);
    b.add_link("slow2", "m", "d", slow);
    b.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
        .expect("buildable")
}

/// `o → d` on one link.
fn one_link() -> RoadNetwork {
    let mut b = RoadNetworkBuilder::new();
    b.add_node("o", LonLat::new(O.0, O.1));
    b.add_node("d", LonLat::new(D.0, D.1));
    let mut spec = LinkSpec::new(RoadClass::Primary);
    spec.capacity_veh_h = Some(1000.0);
    spec.length_m = Some(800.0);
    b.add_link("od", "o", "d", spec);
    b.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
        .expect("buildable")
}

fn link(network: &RoadNetwork, name: &str) -> LinkId {
    network.link_external_ids().typed_id_of::<LinkId>(name).expect("known link")
}

/// `n` car trips `o → d`, all at 07:00 (the demand as level 1 takes it, one departure instant).
fn trips(n: u32, mode: Option<Mode>) -> Vec<RawTrip> {
    (0..n)
        .map(|i| RawTrip {
            traveller_id: format!("p{i:04}"),
            trip_seq: 0,
            origin: LonLat::new(O.0, O.1),
            destination: LonLat::new(D.0, D.1),
            departure_time: Second(25_200),
            user_class: "commuter".to_string(),
            weight: None,
            mode,
        })
        .collect()
}

fn run_of(network: &Arc<RoadNetwork>, n: u32, solver: &str, options: &[(&str, f64)]) -> Run {
    let defaults =
        ClassDefaults::new().with_default("commuter", Ownership { car: true, ..Ownership::NONE });
    let (travellers, trips) =
        build_travellers(trips(n, None), Vec::new(), &defaults, 1, &mut Diagnostics::new())
            .expect("buildable");
    let network = network.clone();
    let turns = Arc::new(TurnTable::build(&network, network.signals()));
    let options: Options = options.iter().map(|&(k, v)| (k.to_string(), v)).collect();
    Run::new(network, Arc::new(travellers), Arc::new(trips), Second(36_000))
        .with_window_start(Second(25_200))
        .with_flow_motor(FlowMotor::Static { turns, period_s: 3600 })
        .with_equilibration(Arc::from(strategy(solver, &options).expect("a strategy")))
        .with_link_bins(900)
}

fn bpr(t0: f64, flow: f64, capacity: f64) -> f64 {
    t0 * (1.0 + 0.15 * (flow / capacity).powi(4))
}

fn mean_trip_s(result: &RunResult) -> f64 {
    result.total_travel_time.get() / f64::from(result.completion.completed)
}

#[test]
fn one_link_carries_the_whole_hour_at_its_bpr_time() {
    let network = Arc::new(one_link());
    let od = link(&network, "od");
    let mut run = run_of(&network, 1500, "frank_wolfe", &[]);
    let result = run.try_execute(&mut Diagnostics::new()).expect("runs");
    let capacity = network.link_parameters(od).capacity.get() * 3600.0;
    let t0 = network.free_flow_time(od).get();
    let expected = bpr(t0, 1500.0, capacity);
    assert_eq!(result.completion.completed, 1500);
    assert_eq!(result.completion.truncated, 0);
    // Each trip's time is the link's, its exit floored to the second as every loading records it.
    assert!(
        (mean_trip_s(&result) - expected).abs() < 1.0,
        "{} against {expected}",
        mean_trip_s(&result)
    );
    assert_eq!(result.iterations.len(), 1, "one route: the equilibrium at once");
    assert!(result.iterations[0].gap.abs() < 1e-12 && result.converged);
    assert!((result.iterations[0].total_travel_time_s - 1500.0 * expected).abs() < 1e-6);
    // The per-link results: the hour's 1 500 PCU over its four quarter hours, 375 each.
    let bins = result.link_bins.as_ref().expect("asked for");
    assert_eq!(bins.bins(), &[28, 29, 30, 31]);
    assert!(bins.pcu().iter().all(|&p| (p - 375.0).abs() < 1e-9), "{:?}", bins.pcu());
    assert_eq!(bins.crossings().iter().sum::<u32>(), 1500);
    assert!((bins.mean_seconds(0) - expected).abs() < 1e-9);
    let sets = result.route_sets.as_ref().expect("the run's sets");
    assert_eq!(sets.route_count(), 1);
    // A skim after the run reads the equilibrium's time, the same all day.
    let times = run.final_link_times().expect("level 1 leaves its times");
    for at in [0.0, 25_200.0, 50_000.0] {
        assert!((times.link_seconds(od.raw(), at) - expected).abs() < 1e-9);
    }
}

/// The flow on `fast` at which both routes take the same time, by bisection.
fn hand_split(network: &RoadNetwork, demand: f64) -> f64 {
    let c = |l: LinkId| network.link_parameters(l).capacity.get() * 3600.0;
    let t = |l: LinkId| network.free_flow_time(l).get();
    let (fast, s1, s2) = (link(network, "fast"), link(network, "slow1"), link(network, "slow2"));
    let direct = |x: f64| bpr(t(fast), x, c(fast));
    let around = |x: f64| bpr(t(s1), demand - x, c(s1)) + bpr(t(s2), demand - x, c(s2));
    let (mut lo, mut hi) = (0.0, demand);
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if direct(mid) < around(mid) { lo = mid } else { hi = mid }
    }
    0.5 * (lo + hi)
}

#[test]
fn two_routes_split_where_their_times_are_equal_and_each_trip_takes_one() {
    let network = Arc::new(two_routes());
    let n = 3000;
    let x_hand = hand_split(&network, f64::from(n));
    assert!(x_hand > 100.0 && x_hand < f64::from(n) - 100.0, "both routes used: {x_hand}");
    let fast = link(&network, "fast");

    let fw = run_of(&network, n, "frank_wolfe", &[("gap_tolerance", 1e-9), ("iterations", 200.0)])
        .try_execute(&mut Diagnostics::new())
        .expect("runs");
    assert!(fw.converged, "{:?}", fw.iterations.last().map(|r| r.gap));
    let sets = fw.route_sets.as_ref().expect("sets");
    assert_eq!(sets.keys().len(), 1);
    assert_eq!(sets.route_count(), 2, "the free-flow route and the other");
    let choices = fw.route_choices.as_ref().expect("choices");
    let on_fast = (0..n)
        .filter(|&i| {
            let r = choices.route[i as usize] as usize;
            sets.route(r).links == [fast.raw()]
        })
        .count();
    let on_fast = f64::from(u32::try_from(on_fast).expect("few trips"));
    assert!(
        (on_fast - x_hand).abs() <= 1.0,
        "{on_fast} trips on the direct street, {x_hand} by hand"
    );
    assert_eq!(fw.completion.completed, n, "every trip arrives");
    // The run's reports are the solver's iterations: the first the free-flow all-or-nothing.
    assert!((fw.iterations[0].reselected_share - 1.0).abs() < f64::EPSILON);
    assert!(fw.iterations[0].gap > 0.1, "everyone on the free-flow route is far from it");
    assert!(fw.iterations.last().expect("one").gap_excess < 1e-9);
    // Both routes' trips take about the same time: the mean trip is the routes' common time.
    let bins = fw.link_bins.as_ref().expect("asked for");
    let fast_pcu: f64 =
        (0..bins.len()).filter(|&r| bins.links()[r] == fast.raw()).map(|r| bins.pcu()[r]).sum();
    assert!((fast_pcu - x_hand).abs() < 1e-3, "{fast_pcu} on the direct street over the hour");

    let msa = run_of(&network, n, "msa", &[("gap_tolerance", 0.0), ("iterations", 1000.0)])
        .try_execute(&mut Diagnostics::new())
        .expect("runs");
    assert_eq!(msa.iterations.len(), 1000);
    let steps: Vec<f64> = msa.iterations.iter().take(3).map(|r| r.reselected_share).collect();
    assert_eq!(steps, vec![1.0, 0.5, 1.0 / 3.0], "the step 1/(k + 1)");
    let msa_fast: f64 = {
        let b = msa.link_bins.as_ref().expect("asked for");
        (0..b.len()).filter(|&r| b.links()[r] == fast.raw()).map(|r| b.pcu()[r]).sum()
    };
    assert!((msa_fast - x_hand).abs() < 0.01 * f64::from(n), "msa {msa_fast}, by hand {x_hand}");
    assert!(
        (mean_trip_s(&msa) - mean_trip_s(&fw)).abs() / mean_trip_s(&fw) < 0.01,
        "msa and Frank–Wolfe agree on the mean trip within 1%"
    );
}

#[test]
fn the_same_run_on_one_thread_and_on_seven_gives_the_same_results() {
    let network = Arc::new(two_routes());
    let mut one = run_of(&network, 2000, "frank_wolfe", &[])
        .with_threads(Some(NonZeroUsize::new(1).expect("1")));
    let mut seven = run_of(&network, 2000, "frank_wolfe", &[])
        .with_threads(Some(NonZeroUsize::new(7).expect("7")));
    assert_eq!(one.description().fingerprint, seven.description().fingerprint);
    let a = one.try_execute(&mut Diagnostics::new()).expect("runs");
    let b = seven.try_execute(&mut Diagnostics::new()).expect("runs");
    assert_eq!(a, b);
}

#[test]
fn mode_choice_is_refused_and_level_one_is_its_own_fingerprint() {
    let network = Arc::new(two_routes());
    let mut run = run_of(&network, 10, "msa", &[]).with_mode_choice(&[Mode::Car, Mode::Walk]);
    match run.try_execute(&mut Diagnostics::new()) {
        Err(RunError::Input(why)) => assert!(why.contains("mode choice"), "{why}"),
        other => panic!("refused, not {other:?}"),
    }
    let level1 = run_of(&network, 10, "msa", &[]);
    let level0 = run_of(&network, 10, "msa", &[]).with_flow_motor(FlowMotor::Level0);
    assert_ne!(level1.description().fingerprint, level0.description().fingerprint);
    assert_eq!(level1.description().flow_level, 1);
    // The volume-delay table is in level 1's fingerprint.
    let mut steeper = LoadingOptions::SHIPPED;
    steeper.bpr.beta[RoadClass::Primary as usize] = 5.0;
    let other = run_of(&network, 10, "msa", &[]).with_loading_options(steeper);
    assert_ne!(level1.description().fingerprint, other.description().fingerprint);
    let _ = TripId::new(0);
}
