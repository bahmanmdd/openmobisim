//! Equilibration (S170): the successive averages in traveller form, and the
//! numbers that say how close a run is to equilibrium.
//!
//! What is defended, and how each is checked:
//!
//! * the link times a loading leaves behind are read back correctly (hand-made
//!   tables);
//! * who chooses again at each iteration follows `1/(i + 1)`, keeps everyone else
//!   where they were, and does not depend on anything but the seed (counts, and
//!   the routes themselves against a run stopped one iteration earlier);
//! * **the gap is measured against what chance alone would give**: on a network
//!   whose costs never change, a freshly sampled assignment has an excess gap of
//!   zero, and one that is systematically wrong does not;
//! * on a hand-made **bottleneck**, iterating moves travellers off the road that
//!   jams and total travel time falls.

#![allow(
    clippy::cast_precision_loss,
    reason = "test fixtures: counts of a few thousand travellers"
)]

use std::sync::Arc;

use openmobisim_core_choice::{Options as ChoiceOptions, model};
use openmobisim_core_demand::{ClassDefaults, Ownership, RawTrip, build_travellers};
use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::network::{LinkSpec, RoadNetwork, RoadNetworkBuilder};
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_loading::{EntryTables, FidelityLevel, LinkBinRecorder, LinkBins};
use openmobisim_core_sim::equilibration::{
    GAP_ACCEPTABLE, GAP_GOOD, GapVerdict, Options, Registry, gap_verdict, strategy,
};
use openmobisim_core_sim::{
    Equilibration, FlowMotor, IterationReport, LinkTimes, Run, RunResult, relative_time_change,
};
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::{EntityId, LinkId};
use openmobisim_core_types::time::Second;
use openmobisim_core_types::units::Duration;

const O: (f64, f64) = (4.800, 45.700);
const D: (f64, f64) = (4.810, 45.700);

/// Two ways from `o` to `d`: a short one-lane residential road (`fast`), and a
/// longer two-lane primary road through `m` (`slow`), each link its own street.
fn bottleneck() -> RoadNetwork {
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

fn trips(n: u32, spread_s: u32) -> Vec<RawTrip> {
    (0..n)
        .map(|i| RawTrip {
            traveller_id: format!("p{i}"),
            trip_seq: 0,
            origin: LonLat::new(O.0, O.1),
            destination: LonLat::new(D.0, D.1),
            departure_time: Second(i * spread_s / n.max(1)),
            user_class: "commuter".to_string(),
            weight: None,
            mode: None,
        })
        .collect()
}

fn car_owning() -> ClassDefaults {
    ClassDefaults::new().with_default("commuter", Ownership { car: true, ..Ownership::NONE })
}

struct Setup {
    network: Arc<RoadNetwork>,
    level: Option<FidelityLevel>,
    n: u32,
    spread_s: u32,
    choice: (&'static str, Vec<(&'static str, f64)>),
    equilibration: (&'static str, Vec<(&'static str, f64)>),
    seed: u64,
    /// A route-set method by name; the default (`penalty`) if `None`.
    generator: Option<&'static str>,
}

impl Setup {
    fn bottleneck(n: u32) -> Self {
        Self {
            network: Arc::new(bottleneck()),
            level: Some(FidelityLevel::Full),
            n,
            spread_s: 900,
            choice: ("logit", vec![("beta_time_min", -2.0)]),
            equilibration: ("msa", vec![("iterations", 8.0)]),
            seed: 1,
            generator: None,
        }
    }

    fn run(&self) -> Run {
        let (travellers, raw) = build_travellers(
            trips(self.n, self.spread_s),
            Vec::new(),
            &car_owning(),
            1,
            &mut Diagnostics::new(),
        )
        .expect("buildable");
        let mut run =
            Run::new(self.network.clone(), Arc::new(travellers), Arc::new(raw), Second(3600))
                .with_master_seed(self.seed);
        if let Some(level) = self.level {
            let turns = Arc::new(TurnTable::build(&self.network, SignalDefaults::SHIPPED));
            run = run.with_flow_motor(FlowMotor::Ltm { turns, step: Duration(60.0), level });
        }
        let options = |pairs: &[(&str, f64)]| -> Options {
            pairs.iter().map(|&(k, v)| (k.to_string(), v)).collect()
        };
        let choice: ChoiceOptions =
            self.choice.1.iter().map(|&(k, v)| (k.to_string(), v)).collect();
        if let Some(name) = self.generator {
            let g =
                openmobisim_core_routes::generator(name, &Default::default()).expect("built in");
            run = run.with_route_generator(Arc::from(g));
        }
        // These tests defend the equilibration on the pair's whole set (the limit on what a traveller
        // is offered has its own, in `choice_set.rs`).
        run.with_choice_detour_limit(0.0)
            .with_choice_model(Arc::from(model(self.choice.0, &choice).expect("built in")))
            .with_equilibration(Arc::from(
                strategy(self.equilibration.0, &options(&self.equilibration.1)).expect("built in"),
            ))
    }

    fn go(&self) -> RunResult {
        self.run().execute(&mut Diagnostics::new())
    }
}

/// How many of the trips took route `k` of the (only) pair's set.
fn took(result: &RunResult, k: usize) -> usize {
    let sets = result.route_sets.as_ref().expect("sets");
    let first = sets.route_range(0).start;
    result
        .route_choices
        .as_ref()
        .expect("choices")
        .route
        .iter()
        .filter(|&&r| r as usize == first + k)
        .count()
}

fn setup_level0(n: u32, model_name: &'static str, options: &[(&'static str, f64)]) -> Setup {
    Setup {
        level: None,
        choice: (model_name, options.to_vec()),
        equilibration: ("msa", vec![("iterations", 4.0)]),
        ..Setup::bottleneck(n)
    }
}

// --- the link times a loading leaves behind ------------------------------------------------

/// The times of one link on a line `a -> b -> c`, and the network they belong to.
fn line() -> (RoadNetwork, u32, u32) {
    let mut b = RoadNetworkBuilder::new();
    for (n, x) in [("a", 4.800), ("b", 4.801), ("c", 4.802)] {
        b.add_node(n, LonLat::new(x, 45.7));
    }
    for (name, from, to) in [("ab", "a", "b"), ("bc", "b", "c")] {
        let mut spec = LinkSpec::new(RoadClass::Primary);
        spec.length_m = Some(100.0);
        b.add_link(name, from, to, spec);
    }
    let net = b
        .build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
        .expect("buildable");
    let id = |n: &str| net.link_external_ids().typed_id_of::<LinkId>(n).expect("link").raw();
    let (ab, bc) = (id("ab"), id("bc"));
    (net, ab, bc)
}

#[test]
fn a_route_is_walked_link_by_link_through_the_bins_it_meets() {
    let (net, ab, bc) = line();
    let free = |l: u32| net.free_flow_time(LinkId::new(l)).get();
    // 60-second bins. ab took 20 s in bin 0 and 50 s in bin 1; bc took 40 s in bin 1.
    let mut r = LinkBinRecorder::new(net.link_count() as usize, 60, 3600.0);
    r.record(LinkId::new(ab), 0.0, 20.0, 1.0); // exit 20 s: bin 0, 20 s
    r.record(LinkId::new(ab), 20.0, 70.0, 1.0); // exit 70 s: bin 1, 50 s
    r.record(LinkId::new(bc), 60.0, 100.0, 1.0); // exit 100 s: bin 1, 40 s
    // Somebody who set out at 130 s onto `ab` waited 25 s outside the network.
    let mut waits = LinkBinRecorder::new(net.link_count() as usize, 60, 3600.0).with_entry_bins();
    waits.record_origin_wait(LinkId::new(ab), 130.0, 155.0, 1.0);
    let tables = EntryTables {
        entry: r.finish(),
        origin_wait: waits.finish_with_entry().1.unwrap().origin_wait,
    };
    let times = LinkTimes::from_tables(&net, &tables);
    assert!((times.bin_seconds() - 60.0).abs() < 1e-12);
    assert!((times.origin_wait_seconds(ab, 130.0) - 25.0).abs() < 1e-12);
    assert!(times.origin_wait_seconds(ab, 10.0).abs() < 1e-12, "nobody waited then");
    // Setting out at 130 s: the 25 s wait, then ab from 155 s (bin 2: free flow), then bc.
    assert!((times.route_seconds(&[ab, bc], 130.0) - (25.0 + free(ab) + free(bc))).abs() < 1e-9);
    // Entering ab at 10 s: bin 0.
    assert!((times.link_seconds(ab, 10.0) - 20.0).abs() < 1e-12);
    // At 70 s: bin 1. At 130 s: bin 2, where nobody was recorded: free flow.
    assert!((times.link_seconds(ab, 70.0) - 50.0).abs() < 1e-12);
    assert!((times.link_seconds(ab, 130.0) - free(ab)).abs() < 1e-12);
    // Leaving at 50 s: ab (bin 0) takes 20 s, arriving at 70 s; bc at 70 s is in bin 1: 40 s.
    assert!((times.route_seconds(&[ab, bc], 50.0) - 60.0).abs() < 1e-12);
    // Leaving at 0 s: ab 20 s, bc at 20 s is bin 0 (nobody): free flow.
    assert!((times.route_seconds(&[ab, bc], 0.0) - (20.0 + free(bc))).abs() < 1e-12);
    // With no table at all, a route costs what it costs at free flow.
    let none = LinkTimes::free_flow(&net);
    assert!((none.route_seconds(&[ab, bc], 500.0) - (free(ab) + free(bc))).abs() < 1e-12);
}

#[test]
fn the_change_between_two_loadings_is_a_traffic_weighted_relative_difference() {
    let (net, ab, bc) = line();
    let free = |l: u32| net.free_flow_time(LinkId::new(l)).get();
    let table = |rows: &[(u32, f64, f64, f64)]| {
        // (link, exit second, seconds taken, pcu), 60-second bins.
        let mut r = LinkBinRecorder::new(net.link_count() as usize, 60, 3600.0);
        let mut sorted = rows.to_vec();
        sorted.sort_by(|a, b| a.1.total_cmp(&b.1));
        for (l, exit, took, pcu) in sorted {
            r.record(LinkId::new(l), exit - took, exit, pcu);
        }
        r.finish()
    };
    let none = || LinkBinRecorder::new(net.link_count() as usize, 60, 3600.0).finish();
    let tables = |entry: LinkBins| EntryTables { entry, origin_wait: none() };
    let before = tables(table(&[(ab, 30.0, 10.0, 2.0), (bc, 40.0, 20.0, 1.0)]));
    let now = tables(table(&[(ab, 30.0, 12.0, 2.0), (ab, 90.0, 30.0, 1.0)]));
    // Rows: (bin 0, ab): weight 2, |12 - 10| = 2; (bin 0, bc): only before, weight 1,
    // |free(bc) - 20|; (bin 1, ab): only now, weight 1, |30 - free(ab)|.
    let moved = 2.0 * 2.0 + (free(bc) - 20.0).abs() + (30.0 - free(ab)).abs();
    let base = 2.0 * 10.0 + 20.0 + free(ab);
    let got = relative_time_change(&net, &before, &now);
    assert!((got - moved / base).abs() < 1e-12, "{got} against {}", moved / base);
    assert!(relative_time_change(&net, &before, &before).abs() < 1e-15, "nothing moved");
}

// --- who chooses again ---------------------------------------------------------------------

#[test]
fn the_first_iteration_of_msa_is_the_run_without_it() {
    let with = Setup::bottleneck(300);
    let without = Setup { equilibration: ("none", vec![]), ..Setup::bottleneck(300) };
    let (m, n) = (with.go(), without.go());
    assert_eq!(n.iterations.len(), 1);
    let first = m.iterations[0];
    assert!((first.total_travel_time_s - n.total_travel_time.get()).abs() < 1e-9);
    assert_eq!(first.completed, n.completion.completed);
    assert!((first.reselected_share - 1.0).abs() < 1e-15, "everyone chooses at iteration 0");
    assert!(first.changed_share.is_nan() && first.time_change.is_nan());
    // Without equilibration nothing beyond the run's own numbers is measured.
    assert!(n.iterations[0].gap_flow.is_nan() && !n.converged);
}

#[test]
fn a_share_of_one_over_i_plus_one_chooses_again_at_iteration_i() {
    let n = 8_000;
    let result =
        setup_level0(n, "logit", &[("beta_time_min", 0.0), ("beta_ln_path_size", 0.0)]).go();
    for it in &result.iterations[1..] {
        let p = 1.0 / (f64::from(it.iteration) + 1.0);
        let sigma = (p * (1.0 - p) / f64::from(n)).sqrt();
        assert!(
            (it.reselected_share - p).abs() < 5.0 * sigma,
            "iteration {}: {} against {p}",
            it.iteration,
            it.reselected_share
        );
        // An indifferent traveller who chooses again lands on the other of two roads half
        // the time, so about half of those who chose again changed.
        assert!((it.changed_share - p / 2.0).abs() < 5.0 * sigma, "iteration {}", it.iteration);
    }
}

#[test]
fn everyone_who_did_not_choose_again_keeps_their_route() {
    let setup = |iterations: f64| Setup {
        equilibration: ("msa", vec![("iterations", iterations)]),
        ..setup_level0(2_000, "logit", &[("beta_time_min", 0.0), ("beta_ln_path_size", 0.0)])
    };
    let (one, two) = (setup(1.0).go(), setup(2.0).go());
    let (a, b) = (one.route_choices.unwrap(), two.route_choices.unwrap());
    let differing = a.route.iter().zip(&b.route).filter(|(x, y)| x != y).count();
    // Same seed, same first choice: what differs is exactly who changed at iteration 1.
    let changed = f64::from(u32::try_from(differing).unwrap()) / 2_000.0;
    assert!((changed - two.iterations[1].changed_share).abs() < 1e-12);
    assert!(changed > 0.1 && changed < 0.4, "about a quarter: {changed}");
    assert!(changed <= two.iterations[1].reselected_share);
}

#[test]
fn iterating_is_repeatable_and_follows_the_seed() {
    let a = Setup::bottleneck(400).go();
    let again = Setup::bottleneck(400).go();
    assert_eq!(a.iterations, again.iterations);
    assert_eq!(a.route_choices, again.route_choices);
    let other = Setup { seed: 2, ..Setup::bottleneck(400) }.go();
    assert_ne!(a.route_choices, other.route_choices);
}

// --- the gap, against what chance alone gives ----------------------------------------------

#[test]
fn the_gap_is_the_relative_excess_over_the_cheapest_route_and_has_a_closed_form() {
    // Level 0: vehicles never interact, so the times never change, and the fast road (72 s) beats
    // the slow (84 s) by 12 s for everyone. A logit with a time coefficient b per minute sends
    // a share P = 1 / (1 + e^(0.2 |b|)) down the slow road, so the gap, the relative excess over
    // the cheapest route, is P x 12 / 72 exactly, at every iteration.
    for b in [-2.0_f64, -0.2, -1.0] {
        let slow_share = 1.0 / (1.0 + (0.2 * b.abs()).exp());
        let result = setup_level0(6_000, "logit", &[("beta_time_min", b)]).go();
        for it in &result.iterations {
            assert!(
                (it.gap - slow_share * 12.0 / 72.0).abs() < 0.006,
                "beta {b}, iteration {}: {} against {}",
                it.iteration,
                it.gap,
                slow_share * 12.0 / 72.0
            );
            // What the model expects is the same closed form, **exactly**: it is worked out from
            // the probabilities, not from who was sampled (S178). So what is left, the
            // disequilibrium, is only the sampling noise of 6 000 travellers.
            assert!(
                (it.gap_expected - slow_share * 12.0 / 72.0).abs() < 1e-9,
                "beta {b}: expected {} against {}",
                it.gap_expected,
                slow_share * 12.0 / 72.0
            );
            assert!((it.gap_excess - (it.gap - it.gap_expected)).abs() < 1e-12);
            assert!(
                it.gap_excess.abs() < 0.006,
                "beta {b}: the model explains the gap: {}",
                it.gap_excess
            );
            assert!(it.incomplete_share.abs() < 1e-12, "nobody is left out at level 0");
        }
    }
    // All-or-nothing: everyone on the cheapest road, so nothing to gain, and nothing expected.
    let deterministic = setup_level0(1_000, "deterministic", &[]).go();
    assert!(deterministic.iterations.iter().all(|it| it.gap.abs() < 1e-12));
    assert!(
        deterministic
            .iterations
            .iter()
            .all(|it| it.gap_expected.abs() < 1e-12 && it.gap_excess.abs() < 1e-12),
        "the disequilibrium of an all-or-nothing model is its gap"
    );
    assert_eq!(took(&deterministic, 1), 0, "all-or-nothing takes the faster road");
    // The stochastic model's own consistency, separate from the gap: a fresh sample has no excess.
    let result = setup_level0(6_000, "logit", &[("beta_time_min", -1.0)]).go();
    for it in &result.iterations {
        assert!(it.gap_flow > 0.0 && it.gap_flow < 0.03, "{}", it.gap_flow);
        assert!(
            it.gap_flow_excess.abs() < 0.01,
            "iteration {}: {}",
            it.iteration,
            it.gap_flow_excess
        );
    }
}

#[test]
fn all_or_nothing_on_a_jammed_road_is_far_from_equilibrium_and_the_gap_says_so() {
    let setup = Setup {
        choice: ("deterministic", vec![]),
        equilibration: ("msa", vec![("iterations", 3.0)]),
        ..Setup::bottleneck(1_500)
    };
    let result = setup.go();
    let first = result.iterations[0];
    // Everyone took the fast road on free-flow costs; the queue it made costs them several times
    // what the slow road does: the gap is the excess over the cheapest route, several hundred %.
    assert!(first.gap > 2.0, "{}", first.gap);
    assert_eq!(gap_verdict(first.gap), Some(GapVerdict::Poor));
    // And by the model's own probabilities (all on the argmin) nearly all are on the wrong road.
    assert!(first.gap_flow > 0.7 && first.gap_flow_floor.abs() < 1e-12);
}

#[test]
fn the_verdict_follows_five_and_fifteen_percent() {
    assert_eq!((GAP_GOOD, GAP_ACCEPTABLE), (0.05, 0.15));
    assert_eq!(gap_verdict(0.0), Some(GapVerdict::Good));
    assert_eq!(gap_verdict(0.049), Some(GapVerdict::Good));
    assert_eq!(gap_verdict(0.05), Some(GapVerdict::Acceptable));
    assert_eq!(gap_verdict(0.149), Some(GapVerdict::Acceptable));
    assert_eq!(gap_verdict(0.15), Some(GapVerdict::Poor));
    assert_eq!(gap_verdict(f64::NAN), None);
    assert_eq!(GapVerdict::Acceptable.as_str(), "acceptable");
}

#[test]
fn a_gap_against_the_choice_set_can_hide_a_route_the_network_has() {
    // Only the fast road is in the choice set ("shortest" makes one route per pair): everyone is
    // on the cheapest route *of the set*, so that gap is nothing, however jammed the road is. The
    // slow road, which the set does not hold, is several times faster: only the test against the
    // whole network sees it.
    let setup = Setup {
        generator: Some("shortest"),
        choice: ("deterministic", vec![]),
        equilibration: ("msa", vec![("iterations", 3.0), ("gap_sample", 200.0)]),
        ..Setup::bottleneck(1_500)
    };
    let result = setup.go();
    let last = result.iterations.last().unwrap();
    assert!(last.gap.abs() < 1e-12, "against the set: {}", last.gap);
    assert!(last.gap_network > 2.0, "against the network: {}", last.gap_network);
    // Only the last iteration is tested against the network; the others say nothing.
    assert!(result.iterations[..2].iter().all(|it| it.gap_network.is_nan()));
    // With the sample off, nothing is measured against the network.
    let off = Setup {
        equilibration: ("msa", vec![("iterations", 3.0), ("gap_sample", 0.0)]),
        ..Setup {
            generator: Some("shortest"),
            choice: ("deterministic", vec![]),
            ..Setup::bottleneck(300)
        }
    }
    .go();
    assert!(off.iterations.iter().all(|it| it.gap_network.is_nan()));
}

#[test]
fn iterating_moves_travellers_off_the_road_that_jams() {
    let n = 1_500;
    let once =
        Setup { equilibration: ("msa", vec![("iterations", 1.0)]), ..Setup::bottleneck(n) }.go();
    let result = Setup::bottleneck(n).go();
    // On free-flow costs the fast road is preferred: P(fast) = 1 / (1 + e^-0.4) = 0.60.
    let fast_first = took(&once, 0);
    assert!((fast_first as f64 - 0.6 * f64::from(n)).abs() < 5.0 * (0.24 * f64::from(n)).sqrt());
    // After iterating, the queue has pushed travellers to the slow road.
    let fast_last = took(&result, 0);
    assert!(fast_last + 100 < fast_first, "{fast_last} on the fast road, from {fast_first}");
    // And the time everyone spends falls.
    let first = result.iterations[0].total_travel_time_s;
    let last = result.iterations.last().unwrap().total_travel_time_s;
    assert!(last < 0.8 * first, "{last} against {first}");
    assert!(
        (result.total_travel_time.get() - last).abs() < 1e-9,
        "the result is the last loading's"
    );
}

#[test]
fn the_gap_and_the_link_times_settle_as_the_iterations_go_on() {
    let result =
        Setup { equilibration: ("msa", vec![("iterations", 20.0)]), ..Setup::bottleneck(1_500) }
            .go();
    let it = &result.iterations;
    let mean =
        |from: usize, to: usize, f: &dyn Fn(&openmobisim_core_sim::IterationReport) -> f64| {
            it[from..to].iter().map(f).sum::<f64>() / (to - from) as f64
        };
    // Far from equilibrium at first: everyone chose on free-flow costs, and the jam that made is
    // nowhere in them: the routes taken cost five times what the cheapest cost.
    assert!(it[0].gap > 1.0, "{}", it[0].gap);
    assert_eq!(gap_verdict(it[0].gap), Some(GapVerdict::Poor));
    // Noisily, but surely, it settles: the late mean is a fraction of the early one, and within
    // what is acceptable for a stochastic assignment (a logit sends some travellers down a
    // slower road at equilibrium, by design, so the gap does not go to zero).
    let early = mean(1, 4, &|r| r.gap);
    let late = mean(10, 20, &|r| r.gap);
    assert!(late < 0.5 * early, "the gap: {early:.3} early, {late:.3} late");
    assert!(late < GAP_ACCEPTABLE, "{late}");
    assert!(
        it[10..].iter().all(|r| r.gap < GAP_ACCEPTABLE),
        "{:?}",
        it[10..].iter().map(|r| r.gap).collect::<Vec<_>>()
    );
    // At the last iteration the whole network agrees: no faster route was missing.
    let last = it.last().unwrap();
    assert!(
        last.gap_network.is_finite() && last.gap_network < GAP_ACCEPTABLE,
        "{}",
        last.gap_network
    );
    // The link times move less and less.
    assert!(it[1].time_change > 5.0 * mean(10, 20, &|r| r.time_change));
    // The share that chooses again shrinks as 1/(i + 1) ...
    assert!(it[1].reselected_share > 5.0 * it[19].reselected_share);
    // ... and the time everyone spends, which fell by more than two thirds at once, holds.
    let times: Vec<f64> = it[10..].iter().map(|r| r.total_travel_time_s).collect();
    let average = times.iter().sum::<f64>() / times.len() as f64;
    assert!(average < 0.4 * it[0].total_travel_time_s);
    assert!(times.iter().all(|t| (t - average).abs() < 0.02 * average), "{times:?}");
}

#[test]
fn a_tolerance_stops_the_run_only_once_the_disequilibrium_has_stayed_below_it() {
    // Level 0, a logit at -1 per minute: the gap is 0.45 x 12 / 72 = 7.5% at every iteration, **and the
    // model itself expects exactly that** (S178): the disequilibrium, the part it does not explain, is
    // nothing. So a tolerance of 5% is met from the first moment the run can judge (three iterations
    // after the first), as is one of 10%; before S178 a 5% tolerance could never be met by a logit.
    let with = |tolerance: f64| {
        Setup {
            equilibration: ("msa", vec![("iterations", 8.0), ("gap_tolerance", tolerance)]),
            ..setup_level0(6_000, "logit", &[("beta_time_min", -1.0)])
        }
        .go()
    };
    for tolerance in [0.10, 0.05] {
        let stop = with(tolerance);
        assert!(stop.converged, "tolerance {tolerance}");
        assert_eq!(stop.iterations.len(), 4, "tolerance {tolerance}");
        let gap = stop.iterations.last().unwrap();
        assert!(
            (gap.gap - 0.075).abs() < 0.01 && (gap.gap_expected - 0.075).abs() < 0.01,
            "{gap:?}"
        );
        assert!(gap.gap_excess.abs() < 0.02, "the model explains it: {}", gap.gap_excess);
        // The whole-network test happens at the iteration the run stops at.
        assert!(gap.gap_network.is_finite() && gap.gap_network_excess.is_finite());
        assert!(stop.iterations[..3].iter().all(|it| it.gap_network.is_nan()));
    }
    // On the bottleneck a tolerance that cannot be met runs every iteration, and 0 means never.
    for tolerance in [1e-6, 0.0] {
        let run = Setup {
            equilibration: ("msa", vec![("iterations", 5.0), ("gap_tolerance", tolerance)]),
            ..Setup::bottleneck(1_500)
        }
        .go();
        assert!(!run.converged && run.iterations.len() == 5, "tolerance {tolerance}");
    }
    // A loose one is met on the bottleneck too, as soon as the jam is relieved.
    let loose = Setup {
        equilibration: ("msa", vec![("iterations", 20.0), ("gap_tolerance", 0.15)]),
        ..Setup::bottleneck(1_500)
    }
    .go();
    assert!(loose.converged && loose.iterations.len() < 12, "{}", loose.iterations.len());
}

// --- what the model is told, and how a strategy is chosen ----------------------------------

use std::sync::Mutex;

use openmobisim_core_choice::{ChoiceBatch, ChoiceError, ChoiceModel, Choices};
use openmobisim_core_types::rng::StreamRng;

/// A model that takes the shorter road and remembers what it was told each iteration: the
/// times of the two routes (in seconds), for the trips it was asked about.
struct Spy {
    seen: Mutex<Vec<(u32, f64, f64)>>,
}

impl ChoiceModel for Spy {
    fn name(&self) -> &str {
        "spy"
    }
    fn descriptor(&self) -> String {
        "spy".to_string()
    }
    fn is_sampled(&self) -> bool {
        false
    }
    fn required_attributes(&self) -> Option<Vec<String>> {
        Some(vec!["time_min".to_string(), "length_km".to_string()])
    }
    fn choose(&self, batch: &ChoiceBatch, _rng: &StreamRng) -> Result<Choices, ChoiceError> {
        let time = batch.attribute("time_min").expect("asked for");
        let length = batch.attribute("length_km").expect("asked for");
        let mut chosen = Vec::new();
        for s in 0..batch.situations() {
            let r = batch.range(s);
            assert_eq!(r.len(), 2, "the two roads");
            // The fast road is the short one.
            let (fast, slow) = if length[r.start] < length[r.start + 1] {
                (r.start, r.start + 1)
            } else {
                (r.start + 1, r.start)
            };
            self.seen.lock().expect("lock").push((
                batch.iteration(),
                time[fast] * 60.0,
                time[slow] * 60.0,
            ));
            chosen.push(u32::try_from(fast - r.start).expect("small"));
        }
        let probability = vec![f64::NAN; chosen.len()];
        Ok(Choices { chosen, probability })
    }
}

#[test]
fn from_the_second_iteration_the_model_is_told_the_times_the_last_loading_gave() {
    let spy = Arc::new(Spy { seen: Mutex::new(Vec::new()) });
    let mut setup = Setup::bottleneck(600);
    setup.equilibration = ("msa", vec![("iterations", 3.0)]);
    let result = setup.run().with_choice_model(spy.clone()).execute(&mut Diagnostics::new());
    let seen = spy.seen.lock().expect("lock");
    // Iteration 0 is asked about every trip, on the store's free-flow times: 72 s and 84 s.
    let first: Vec<_> = seen.iter().filter(|s| s.0 == 0).collect();
    assert_eq!(first.len(), 600);
    assert!(first.iter().all(|s| (s.1 - 72.0).abs() < 0.01 && (s.2 - 84.0).abs() < 0.01));
    // Later, a share of the trips is asked again, and the fast road's time is no longer 72 s: the
    // queue it made is in it, longer for those who leave later.
    let later: Vec<_> = seen.iter().filter(|s| s.0 == 1).collect();
    assert!(!later.is_empty() && later.len() < 600);
    assert!(later.iter().any(|s| s.1 > 300.0), "some see a long queue on the fast road");
    assert!(later.iter().all(|s| (s.2 - 84.0).abs() < 1.0), "the slow road is not loaded");
    // A model with no probabilities has no consistency check to report, but the gap does not need
    // its probabilities: it is what the routes taken cost over the cheapest, and it is measured.
    for it in &result.iterations {
        assert!(it.gap_flow.is_nan() && it.gap.is_finite() && it.total_travel_time_s > 0.0);
    }
    assert!(result.iterations[0].gap > 1.0, "the queue the spy made costs it dear");
    // The spy always takes the fast road, so nothing moves: the same assignment loads to the very
    // same link times, and the change between two loadings is exactly zero.
    assert!(result.iterations[1].time_change.abs() < 1e-12);
    assert!(result.iterations[1].changed_share.abs() < 1e-12);
}

#[test]
fn the_description_names_the_strategy_and_the_streams_it_draws_from() {
    let run = |eq: (&'static str, Vec<(&'static str, f64)>), choice: &'static str| {
        Setup { equilibration: eq, choice: (choice, vec![]), ..Setup::bottleneck(10) }
            .run()
            .description()
    };
    let none = run(("none", vec![]), "deterministic");
    assert_eq!(none.equilibration, "none");
    assert_eq!((none.max_iterations, none.live_streams.len()), (1, 0));
    let msa = run(("msa", vec![]), "deterministic");
    assert_eq!((msa.equilibration.as_str(), msa.max_iterations), ("msa", 10));
    assert_eq!(msa.live_streams, ["msa_reselection"]);
    assert_eq!(
        msa.equilibration_descriptor,
        "msa;cost_bin_s=300;gap_sample=300;gap_tolerance=0;iterations=10;warmup=0"
    );
    assert_eq!(run(("msa", vec![]), "logit").live_streams, ["choice", "msa_reselection"]);
    // One iteration has nobody to re-select.
    assert!(run(("msa", vec![("iterations", 1.0)]), "deterministic").live_streams.is_empty());
    // Every setting is an input.
    let fingerprint = |o: Vec<(&'static str, f64)>| run(("msa", o), "logit").fingerprint;
    let all = [
        fingerprint(vec![]),
        fingerprint(vec![("iterations", 5.0)]),
        fingerprint(vec![("gap_tolerance", 0.1)]),
        fingerprint(vec![("cost_bin_s", 60.0)]),
    ];
    for (i, a) in all.iter().enumerate() {
        for b in &all[i + 1..] {
            assert_ne!(a, b);
        }
    }
    assert_ne!(none.fingerprint, msa.fingerprint);
}

#[test]
fn strategies_are_chosen_by_name_and_bad_options_say_what_is_wrong() {
    assert_eq!(Registry::builtin().names(), ["none", "msa"]);
    let opts = |pairs: &[(&str, f64)]| -> Options {
        pairs.iter().map(|&(k, v)| (k.to_string(), v)).collect()
    };
    let err = |name: &str, o: &[(&str, f64)]| {
        strategy(name, &opts(o)).err().expect("refused").to_string()
    };
    assert!(err("replanning", &[]).contains("none, msa"));
    assert!(err("none", &[("iterations", 3.0)]).contains("no options"));
    assert!(
        err("msa", &[("steps", 3.0)]).contains("cost_bin_s, gap_sample, gap_tolerance, iterations")
    );
    for bad in [0.0, 1001.0, 2.5, f64::NAN] {
        assert!(
            err("msa", &[("iterations", bad)]).contains("whole number from 1 to 1000"),
            "{bad}"
        );
    }
    assert!(err("msa", &[("cost_bin_s", 0.0)]).contains("cost_bin_s"));
    assert!(err("msa", &[("gap_tolerance", 2.0)]).contains("from 0 to 1"));
    assert!(err("msa", &[("gap_sample", -1.0)]).contains("gap_sample"));
    assert!(err("msa", &[("gap_sample", 100_001.0)]).contains("whole number from 0 to 100000"));
    assert!(strategy("msa", &opts(&[("iterations", 1000.0)])).is_ok());
}

/// A strategy written outside the crate: everybody chooses again at every iteration, three times.
struct Everyone;

impl Equilibration for Everyone {
    fn name(&self) -> &str {
        "everyone"
    }
    fn descriptor(&self) -> String {
        "everyone".to_string()
    }
    fn max_iterations(&self) -> u32 {
        3
    }
    fn cost_bin_seconds(&self) -> u32 {
        300
    }
    fn draws_reselection(&self) -> bool {
        false
    }
    fn reselects(&self, _rng: &StreamRng, _traveller: u32, _iteration: u32) -> bool {
        true
    }
}

#[test]
fn a_strategy_from_outside_the_crate_drives_the_run() {
    let mut registry = Registry::builtin();
    registry.register("everyone", |_| Ok(Box::new(Everyone)));
    assert_eq!(registry.names().last(), Some(&"everyone"));
    let made = registry.create("everyone", &Options::new()).expect("registered");
    let result = Setup::bottleneck(400)
        .run()
        .with_equilibration(Arc::from(made))
        .execute(&mut Diagnostics::new());
    assert_eq!(result.iterations.len(), 3);
    let shares: Vec<f64> = result.iterations.iter().map(|r| r.reselected_share).collect();
    assert_eq!(shares, [1.0, 1.0, 1.0], "everyone chose again every time");
    // Everybody at once is the flip-flop MSA exists to avoid: a large share changes roads.
    assert!(result.iterations[1].changed_share > 0.2, "{}", result.iterations[1].changed_share);
}

#[test]
fn a_run_without_the_strategy_reports_one_iteration_and_no_table_it_was_not_asked_for() {
    let mut setup = Setup::bottleneck(200);
    setup.equilibration = ("none", vec![]);
    let result = setup.go();
    assert_eq!(result.iterations.len(), 1);
    let only: &IterationReport = &result.iterations[0];
    assert_eq!((only.iteration, only.completed), (0, result.completion.completed));
    assert!(result.link_bins.is_none(), "no per-link table unless it was asked for");
}
