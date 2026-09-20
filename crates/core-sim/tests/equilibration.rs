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
use openmobisim_core_sim::equilibration::{Options, Registry, strategy};
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
        run.with_choice_model(Arc::from(model(self.choice.0, &choice).expect("built in")))
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
fn where_costs_never_change_a_fresh_sample_has_no_excess_gap() {
    // Level 0: vehicles never interact, so every loading leaves the free-flow times. The
    // assignment at every iteration is then a sample from the model's probabilities, and
    // its gap to those probabilities is exactly what chance gives: the excess is zero.
    let n = 6_000;
    let result = setup_level0(n, "logit", &[("beta_time_min", -1.0)]).go();
    for it in &result.iterations {
        assert!(
            it.gap_flow > 0.0 && it.gap_flow < 0.03,
            "iteration {}: {}",
            it.iteration,
            it.gap_flow
        );
        assert!(
            it.gap_flow_excess.abs() < 0.01,
            "iteration {}: {}",
            it.iteration,
            it.gap_flow_excess
        );
        assert!(it.gap_cost.abs() < 0.005, "iteration {}: {}", it.iteration, it.gap_cost);
    }
    // ... while the same measure, with a systematically wrong assignment, is large: everyone on
    // the slower road, though the logit puts more on the faster.
    let sets_slower = |r: &RunResult| took(r, 1);
    let deterministic = Setup {
        equilibration: ("msa", vec![("iterations", 2.0)]),
        ..setup_level0(1_000, "deterministic", &[])
    }
    .go();
    assert_eq!(sets_slower(&deterministic), 0, "all-or-nothing takes the faster road");
    assert!(deterministic.iterations[1].gap_flow.abs() < 1e-12, "and needs no other");
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
    // Everyone took the fast road on free-flow costs; the queue it made costs them more than
    // the slow road, so nearly everyone is on the wrong one, and by all-or-nothing the
    // fresh sample would have been the argmin: no floor.
    assert!(first.gap_flow > 0.7, "{}", first.gap_flow);
    assert!(first.gap_flow_floor.abs() < 1e-12);
    assert!((first.gap_flow_excess - first.gap_flow).abs() < 1e-12);
    assert!(first.gap_cost > 0.1, "{}", first.gap_cost);
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
    // Far from equilibrium at first: everyone chose on free-flow costs, and the jam that made
    // is nowhere in them (the excess gap is the share who would choose differently now, less
    // what chance alone gives; the cost gap is the excess of the times paid over those expected).
    assert!(it[0].gap_flow_excess > 0.4, "{}", it[0].gap_flow_excess);
    assert!(it[0].gap_cost > 0.5, "{}", it[0].gap_cost);
    // Noisily, but surely, it settles: the late mean is a fraction of the early one.
    let early = mean(1, 4, &|r| r.gap_flow_excess);
    let late = mean(10, 20, &|r| r.gap_flow_excess);
    assert!(late < 0.5 * early && late < 0.1, "excess gap: {early:.3} early, {late:.3} late");
    assert!(mean(10, 20, &|r| r.gap_cost).abs() < 0.03);
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
fn a_tolerance_stops_the_run_only_once_the_excess_gap_has_stayed_small() {
    // Level 0: costs never change, so the excess gap is noise around zero from the start; the
    // run stops at the first moment it can judge (three iterations after the first).
    let stop = Setup {
        equilibration: ("msa", vec![("iterations", 10.0), ("gap_tolerance", 0.02)]),
        ..setup_level0(6_000, "logit", &[("beta_time_min", -1.0)])
    }
    .go();
    assert!(stop.converged);
    assert_eq!(stop.iterations.len(), 4);
    // On the bottleneck a tolerance that cannot be met runs every iteration.
    let never = Setup {
        equilibration: ("msa", vec![("iterations", 5.0), ("gap_tolerance", 1e-6)]),
        ..Setup::bottleneck(1_500)
    }
    .go();
    assert!(!never.converged);
    assert_eq!(never.iterations.len(), 5);
    // And tolerance 0 means never.
    let off = Setup {
        equilibration: ("msa", vec![("iterations", 6.0)]),
        ..setup_level0(6_000, "logit", &[("beta_time_min", -1.0)])
    }
    .go();
    assert!(!off.converged && off.iterations.len() == 6);
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
    // A model with no probabilities has no gap to report; the rest is measured.
    for it in &result.iterations {
        assert!(it.gap_flow.is_nan() && it.gap_cost.is_nan() && it.total_travel_time_s > 0.0);
    }
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
    assert_eq!(msa.equilibration_descriptor, "msa;cost_bin_s=300;gap_tolerance=0;iterations=10");
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
    assert!(err("msa", &[("steps", 3.0)]).contains("cost_bin_s, gap_tolerance, iterations"));
    for bad in [0.0, 1001.0, 2.5, f64::NAN] {
        assert!(
            err("msa", &[("iterations", bad)]).contains("whole number from 1 to 1000"),
            "{bad}"
        );
    }
    assert!(err("msa", &[("cost_bin_s", 0.0)]).contains("cost_bin_s"));
    assert!(err("msa", &[("gap_tolerance", 2.0)]).contains("from 0 to 1"));
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
