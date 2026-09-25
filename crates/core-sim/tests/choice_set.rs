//! What a traveller is offered, how a run starts, and who counts in the gap (S178).
//!
//! What is defended, and how each is checked (on a hand-made bottleneck, where the answer is worked out):
//!
//! * **a time-dependent choice set**: a route is offered only if its expected time is within the limit of
//!   the best's; at free flow the slow road is 1/6 dearer than the fast one, so a limit under 1/6 offers
//!   only the fast road (everyone takes it, with probability 1) and one over it offers both (the logit's
//!   split); the best route is always offered; a route left out keeps its place in the set and a traveller who
//!   is offered only the others is mapped to the right route number;
//! * **a warm-up**: the first loading is the point-queue model's, the last never is, and a warm-up longer
//!   than the run allows is cut;
//! * **who counts in the gap**: trips still under way when the window ends are left out and counted apart;
//!   with nobody finished there is no gap to speak of.

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
use openmobisim_core_loading::FidelityLevel;
use openmobisim_core_sim::equilibration::{Options, strategy};
use openmobisim_core_sim::{FlowMotor, Run, RunResult};
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::time::Second;
use openmobisim_core_types::units::Duration;

const O: (f64, f64) = (4.800, 45.700);
const D: (f64, f64) = (4.810, 45.700);

/// Two ways from `o` to `d`: a short one-lane residential road (`fast`, 72 s at free flow) and a longer
/// two-lane primary road through `m` (`slow`, 84 s), each link its own street.
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

fn trips(n: u32) -> Vec<RawTrip> {
    (0..n)
        .map(|i| RawTrip {
            traveller_id: format!("p{i}"),
            trip_seq: 0,
            origin: LonLat::new(O.0, O.1),
            destination: LonLat::new(D.0, D.1),
            departure_time: Second(i * 900 / n.max(1)),
            user_class: "commuter".to_string(),
            weight: None,
            mode: None,
        })
        .collect()
}

struct Setup {
    n: u32,
    /// The loading: `None` is free flow (level 0), otherwise the link transmission model at this level.
    level: Option<FidelityLevel>,
    window: u32,
    choice: &'static str,
    beta: f64,
    iterations: f64,
    warmup: f64,
    limit: Option<f64>,
}

impl Setup {
    fn free_flow(n: u32) -> Self {
        Self {
            n,
            level: None,
            window: 3600,
            choice: "logit",
            beta: -2.0,
            iterations: 1.0,
            warmup: 0.0,
            limit: None,
        }
    }

    fn jam(n: u32) -> Self {
        Self { level: Some(FidelityLevel::Full), iterations: 4.0, ..Self::free_flow(n) }
    }

    fn go(&self) -> RunResult {
        let network = Arc::new(bottleneck());
        let classes = ClassDefaults::new()
            .with_default("commuter", Ownership { car: true, ..Ownership::NONE });
        let (travellers, raw) =
            build_travellers(trips(self.n), Vec::new(), &classes, 1, &mut Diagnostics::new())
                .expect("buildable");
        let mut run =
            Run::new(network.clone(), Arc::new(travellers), Arc::new(raw), Second(self.window))
                .with_master_seed(1);
        if let Some(level) = self.level {
            let turns = Arc::new(TurnTable::build(&network, SignalDefaults::SHIPPED));
            run = run.with_flow_motor(FlowMotor::Ltm { turns, step: Duration(60.0), level });
        }
        let choice: ChoiceOptions =
            [("beta_time_min".to_string(), self.beta)].into_iter().collect();
        let equilibration: Options = [
            ("iterations".to_string(), self.iterations),
            ("warmup".to_string(), self.warmup),
            ("gap_sample".to_string(), 200.0),
        ]
        .into_iter()
        .collect();
        run = run
            .with_choice_model(Arc::from(model(self.choice, &choice).expect("built in")))
            .with_equilibration(Arc::from(strategy("msa", &equilibration).expect("built in")));
        if let Some(limit) = self.limit {
            run = run.with_choice_detour_limit(limit);
        }
        run.execute(&mut Diagnostics::new())
    }
}

/// How many trips took route `k` of the (only) pair's set.
fn took(result: &RunResult, k: usize) -> usize {
    let first = result.route_sets.as_ref().unwrap().route_range(0).start;
    let routes = &result.route_choices.as_ref().unwrap().route;
    routes.iter().filter(|&&r| r as usize == first + k).count()
}

// --- the `detour` route attribute -------------------------------------------------------------------

#[test]
fn the_detour_attribute_is_time_over_the_bests_within_the_set() {
    // Free flow: fast = 72 s (the set's best, detour 0), slow = 84 s (detour = 84/72 - 1 = 1/6).
    // Zeroing beta_time_min and driving the split with beta_detour alone makes the observed choices
    // a direct, hand-checkable readout of `detour`'s own value — found untested by checkpoint 8b's
    // adversarial pass (hardcoding `detour` to 0.0 broke nothing in the suite as it stood).
    let n = 6_000;
    let network = Arc::new(bottleneck());
    let classes =
        ClassDefaults::new().with_default("commuter", Ownership { car: true, ..Ownership::NONE });
    let (travellers, raw) =
        build_travellers(trips(n), Vec::new(), &classes, 1, &mut Diagnostics::new())
            .expect("buildable");
    let choice: ChoiceOptions =
        [("beta_time_min".to_string(), 0.0), ("beta_detour".to_string(), -2.0)]
            .into_iter()
            .collect();
    let mut run = Run::new(network, Arc::new(travellers), Arc::new(raw), Second(3600))
        .with_master_seed(1)
        .with_choice_model(Arc::from(model("logit", &choice).expect("built in")));
    let result = run.execute(&mut Diagnostics::new());
    // U_fast = -2 * 0 = 0; U_slow = -2 * 1/6 = -1/3; P(fast) = 1 / (1 + e^(-1/3)).
    let expected = 1.0 / (1.0 + (-1.0_f64 / 3.0).exp());
    let sigma = (expected * (1.0 - expected) * f64::from(n)).sqrt();
    assert!(
        (took(&result, 0) as f64 - expected * f64::from(n)).abs() < 5.0 * sigma,
        "took {} of {n} on the fast route, expected close to {}",
        took(&result, 0),
        expected * f64::from(n)
    );
}

// --- a time-dependent choice set ------------------------------------------------------------------

#[test]
fn a_route_more_than_the_limit_slower_than_the_best_is_not_offered() {
    // Free flow: the fast road takes 72 s, the slow 84: 1/6 more. A logit at -2 per minute puts
    // P = 1 / (1 + e^-0.4) = 0.60 on the fast road when both are offered.
    let n = 6_000;
    let both = Setup { limit: Some(0.2), ..Setup::free_flow(n) }.go();
    let all = Setup { limit: Some(0.0), ..Setup::free_flow(n) }.go();
    let default = Setup::free_flow(n).go();
    let sigma = (0.24 * f64::from(n)).sqrt();
    assert!((took(&both, 0) as f64 - 0.6 * f64::from(n)).abs() < 5.0 * sigma);
    // 0 offers every route, and a limit past the slow road's detour offers it too: the same choices.
    assert_eq!(all.route_choices, both.route_choices);
    // The default (0.5) offers the slow road too at free flow, where it is a sixth dearer.
    assert_eq!(default.route_choices, all.route_choices);
    // A limit under 1/6 offers the fast road alone: everyone takes it, and with probability 1.
    let only = Setup { iterations: 2.0, limit: Some(0.1), ..Setup::free_flow(n) }.go();
    assert_eq!((took(&only, 0), took(&only, 1)), (n as usize, 0));
    let rc = only.route_choices.as_ref().unwrap();
    assert!(rc.probability.iter().all(|&p| (p - 1.0).abs() < 1e-12), "one alternative: certain");
    assert!(rc.alternatives.iter().all(|&a| a == 2), "the set still holds both roads");
    // Everyone is on the best route and the model expects them to be: its gap is 0, and so is the observed.
    for it in &only.iterations {
        assert!(it.gap.abs() < 1e-12 && it.gap_expected.abs() < 1e-12, "{it:?}");
    }
    // The best route is always offered, however tight the limit.
    let tiny = Setup { limit: Some(1e-9), ..Setup::free_flow(n) }.go();
    assert_eq!(took(&tiny, 0), n as usize);
}

#[test]
fn a_traveller_offered_only_some_routes_is_mapped_to_the_right_one() {
    // The jam on the fast road makes the slow one several times quicker; from then on the fast road is
    // over any limit of 0.3, so a traveller who chooses again is offered the slow road alone: certain,
    // and route number 1 of the set, not number 0 (the first alternative offered is not the first route).
    let two = Setup { iterations: 2.0, limit: Some(0.3), ..Setup::jam(1_500) }.go();
    let rc = two.route_choices.as_ref().unwrap();
    let certain: Vec<usize> =
        (0..rc.route.len()).filter(|&t| (rc.probability[t] - 1.0).abs() < 1e-12).collect();
    assert!(certain.len() > 200, "many chose again on the jam: {}", certain.len());
    let first = two.route_sets.as_ref().unwrap().route_range(0).start;
    assert!(
        certain.iter().all(|&t| rc.route[t] as usize == first + 1),
        "offered the slow road alone, they are on it"
    );
    assert!(
        rc.route.iter().all(|&r| (r as usize) < first + 2),
        "every route is one of the pair's two"
    );
    // The gap is still measured against the whole set: the fast road, left out, is not the best now.
    assert!(two.iterations[1].gap.is_finite());
}

// --- a warm-up ---------------------------------------------------------------------------------------

#[test]
fn a_warmup_loads_the_first_iterations_with_the_point_queue_model_and_never_the_last() {
    let time = |r: &RunResult, i: usize| r.iterations[i].total_travel_time_s;
    let plain = Setup { iterations: 3.0, ..Setup::jam(1_500) }.go();
    let warm = Setup { iterations: 3.0, warmup: 1.0, ..Setup::jam(1_500) }.go();
    let point_queue =
        Setup { iterations: 1.0, level: Some(FidelityLevel::PointQueue), ..Setup::jam(1_500) }.go();
    // The first loading of the warm-up run is the point-queue model's, on the same choices: the same total time.
    assert_ne!(time(&warm, 0).to_bits(), time(&plain, 0).to_bits(), "the level matters on a jam");
    assert_eq!(time(&warm, 0).to_bits(), time(&point_queue, 0).to_bits());
    // The last loading, the run's result, is the full model's: a warm-up as long as the run is cut to
    // leave it, so one iteration is the run without one, and a longer one is cut to `iterations - 1`.
    let one = Setup { iterations: 1.0, warmup: 5.0, ..Setup::jam(1_500) }.go();
    let none = Setup { iterations: 1.0, ..Setup::jam(1_500) }.go();
    assert_eq!(one.total_travel_time, none.total_travel_time);
    assert_eq!(one.events, none.events);
    let cut = Setup { iterations: 3.0, warmup: 9.0, ..Setup::jam(1_500) }.go();
    let two = Setup { iterations: 3.0, warmup: 2.0, ..Setup::jam(1_500) }.go();
    assert_eq!(cut.iterations, two.iterations, "9 is cut to 2 of 3");
    assert_ne!(
        time(&cut, 2).to_bits(),
        time(&point_queue, 0).to_bits(),
        "the last is not the point queue's"
    );
    // Free flow has no interaction to warm up: the warm-up changes nothing.
    let free = Setup { iterations: 3.0, warmup: 1.0, ..Setup::free_flow(600) }.go();
    let free_plain = Setup { iterations: 3.0, ..Setup::free_flow(600) }.go();
    assert_eq!(free.iterations, free_plain.iterations);
}

// --- who counts in the gap ---------------------------------------------------------------------------

#[test]
fn trips_still_under_way_at_the_end_are_left_out_of_the_gap_and_counted_apart() {
    // A window of 240 s: the first vehicles finish (72 s), the many that follow on the jammed fast
    // road do not. The share left out is the share the loading reports as truncated.
    let n = 1_500_u32;
    let result = Setup { window: 240, ..Setup::jam(n) }.go();
    for it in &result.iterations {
        let expected = f64::from(it.truncated) / f64::from(n);
        assert!(
            (it.incomplete_share - expected).abs() < 1e-12,
            "{} against {expected}",
            it.incomplete_share
        );
    }
    let it = &result.iterations[0];
    assert!(it.incomplete_share > 0.5, "most did not finish: {}", it.incomplete_share);
    assert!(it.gap.is_finite() && it.gap >= 0.0, "the trips that finished have a gap: {}", it.gap);
    // Nobody finishes in one second: there is no gap to measure, and everyone is left out.
    let none = Setup { window: 1, ..Setup::jam(n) }.go();
    assert!(
        none.iterations
            .iter()
            .all(|it| it.gap.is_nan() && (it.incomplete_share - 1.0).abs() < 1e-12)
    );
    // With time to finish, nobody is left out.
    let all = Setup::jam(n).go();
    assert!(all.iterations.iter().all(|it| it.incomplete_share.abs() < 1e-12));
}
