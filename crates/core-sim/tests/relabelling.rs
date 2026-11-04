//! A run's route choices must not depend on the ids of links and nodes (S312).
//!
//! **The case.** Two ways from `o` to `d`: a short one-lane residential road (`fast`, 600 m, 72 s
//! at free flow) and a longer two-lane primary road through `m` (`slow`, two links of 700 m, 84 s
//! in all). 2 000 car travellers leave over 15 minutes and choose by the logit (−2 per minute),
//! so about 60 % take the fast road (P = 1 / (1 + e^−0.4)). Each traveller's choice is a random
//! draw, addressed by (traveller, trip, iteration, route identity): the same traveller must make
//! the same draw for the same street whatever the street is called.
//!
//! **What is checked.** The network is built twice: the links and the nodes named as listed, then
//! with the names exchanged so that their ids, which follow the sorted names, come in another
//! order. Every traveller must take the same route, by its nodes, under both namings: at free flow
//! (the first choice alone) and through four assignment iterations of the point queue (level 2),
//! where the second choice reads the first loading's times. Before S312 a route's identity was a
//! hash of its links' ids, so renaming redrew every traveller's errors: 965 of the 2 000 took the
//! other road at free flow (independent draws differ with chance 2 · 0.6 · 0.4 = 48 %), 460 after
//! four iterations.

use std::sync::Arc;

use openmobisim_core_choice::{Options as ChoiceOptions, model};
use openmobisim_core_demand::{ClassDefaults, Ownership, RawTrip, build_travellers};
use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::network::{LinkSpec, RoadNetwork, RoadNetworkBuilder};
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_loading::FidelityLevel;
use openmobisim_core_sim::ModeDefaults;
use openmobisim_core_sim::equilibration::{Options, strategy};
use openmobisim_core_sim::{FlowMotor, Run};
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::{EntityId, LinkId, NodeId};
use openmobisim_core_types::time::Second;
use openmobisim_core_types::units::Duration;

const O: (f64, f64) = (4.800, 45.700);
const D: (f64, f64) = (4.810, 45.700);
const TRAVELLERS: u32 = 2_000;

/// The two roads, the links named `links` (fast, slow's first, slow's second) and the nodes
/// `nodes` (o, m, d).
fn network(links: [&str; 3], nodes: [&str; 3]) -> RoadNetwork {
    let mut b = RoadNetworkBuilder::new();
    b.add_node(nodes[0], LonLat::new(O.0, O.1));
    b.add_node(nodes[1], LonLat::new(4.805, 45.7004));
    b.add_node(nodes[2], LonLat::new(D.0, D.1));
    let mut fast = LinkSpec::new(RoadClass::Residential);
    fast.lanes = Some(1);
    fast.length_m = Some(600.0);
    b.add_link(links[0], nodes[0], nodes[2], fast);
    let mut slow = LinkSpec::new(RoadClass::Primary);
    slow.lanes = Some(2);
    slow.length_m = Some(700.0);
    b.add_link(links[1], nodes[0], nodes[1], slow);
    b.add_link(links[2], nodes[1], nodes[2], slow);
    b.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
        .expect("buildable")
}

fn trips() -> Vec<RawTrip> {
    (0..TRAVELLERS)
        .map(|i| RawTrip {
            traveller_id: format!("p{i:04}"),
            trip_seq: 0,
            origin: LonLat::new(O.0, O.1),
            destination: LonLat::new(D.0, D.1),
            departure_time: Second(25_200 + i * 900 / TRAVELLERS),
            user_class: "commuter".to_string(),
            weight: None,
            mode: None,
        })
        .collect()
}

/// Each trip's route as the position of its nodes (`o`, `m`, `d` as 0, 1, 2), in trip order,
/// from a run of `iterations` assignment iterations (free flow with `None`).
fn routes(
    net: RoadNetwork,
    nodes: [&str; 3],
    level: Option<FidelityLevel>,
    iterations: f64,
) -> Vec<Vec<usize>> {
    let network = Arc::new(net);
    let classes =
        ClassDefaults::new().with_default("commuter", Ownership { car: true, ..Ownership::NONE });
    let (travellers, raw) =
        build_travellers(trips(), Vec::new(), &classes, 1, &mut Diagnostics::new())
            .expect("buildable");
    let mut run = Run::new(network.clone(), Arc::new(travellers), Arc::new(raw), Second(32_400))
        .with_window_start(Second(25_200))
        .with_mode_defaults(ModeDefaults::SHIPPED.without_fixed_times())
        .with_master_seed(1);
    if let Some(level) = level {
        let turns = Arc::new(TurnTable::build(&network, SignalDefaults::SHIPPED));
        run = run.with_flow_motor(FlowMotor::Ltm { turns, step: Duration(60.0), level });
    }
    let choice: ChoiceOptions = [("beta_time_min".to_string(), -2.0)].into_iter().collect();
    let equilibration: Options =
        [("iterations".to_string(), iterations), ("warmup".to_string(), 0.0)].into_iter().collect();
    run = run
        .with_choice_model(Arc::from(model("logit", &choice).expect("built in")))
        .with_equilibration(Arc::from(strategy("msa", &equilibration).expect("built in")));
    let result = run.execute(&mut Diagnostics::new());
    let sets = result.route_sets.as_ref().expect("route sets");
    let position = |n: NodeId| {
        let name = network.node_external_ids().external_of(n).expect("named");
        nodes.iter().position(|&x| x == name).expect("a node of the case")
    };
    result
        .route_choices
        .as_ref()
        .expect("route choices")
        .route
        .iter()
        .map(|&r| {
            let links = sets.route(r as usize).links;
            let mut out = vec![position(network.link_from(LinkId::new(links[0])))];
            out.extend(links.iter().map(|&l| position(network.link_to(LinkId::new(l)))));
            out
        })
        .collect()
}

fn same_choices_under_both_namings(level: Option<FidelityLevel>, iterations: f64) {
    let (links, nodes) = (["fast", "slow1", "slow2"], ["o", "m", "d"]);
    // The links and the nodes named so that both sort the other way round.
    let (links2, nodes2) = (["z1", "y2", "x3"], ["c", "b", "a"]);
    let a = routes(network(links, nodes), nodes, level, iterations);
    let b = routes(network(links2, nodes2), nodes2, level, iterations);
    let fast = a.iter().filter(|r| r.len() == 2).count();
    if level.is_none() {
        assert!(
            (1_100..1_300).contains(&fast),
            "about 60 % on the fast road at free flow (1 200 of 2 000): {fast}"
        );
    } else {
        // The fast road's one lane queues, so the later choices move travellers to the slow one.
        assert!((200..1_800).contains(&fast), "both roads taken: {fast} on the fast one");
    }
    let differ = a.iter().zip(&b).filter(|(x, y)| x != y).count();
    assert_eq!(differ, 0, "{differ} of {TRAVELLERS} travellers take another road once renamed");
}

#[test]
fn the_logit_draws_follow_the_streets_not_the_ids_at_free_flow() {
    same_choices_under_both_namings(None, 1.0);
}

#[test]
fn the_logit_draws_follow_the_streets_not_the_ids_through_the_iterations() {
    same_choices_under_both_namings(Some(FidelityLevel::PointQueue), 4.0);
}
