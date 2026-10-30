//! The operator protocol on the fleet engine: a policy steps every few minutes, sees the state
//! read-only, and its decisions are checked, refused with a reason or carried out, and recorded.
//!
//! Two networks, each with closed-form times. **Docks**: a 5 × 5 two-way grid of 200 m blocks
//! with three docks on its first row, at 0, 200 and 600 m (A, B, C); rides at 4 m/s and vans at
//! 10 m/s, as the crow flies. **Taxis**: ten nodes on a line a kilometre apart, driven at 60 s a
//! kilometre. What is defended: a move by road with its closed-form arrival; moves refused
//! with their reasons; a policy's failure stopping the day; bikes brought to a full dock; a
//! refusal of each decision kind; a relocation and a charge with closed-form arrival and charge;
//! targets solved into moves; bikes in vans counted in the conservation; the same seed the same
//! decisions, and a copy continuing as the original; a policy deciding nothing changes nothing;
//! the built-in policies (`Reset`, `Threshold`, `ChargeRule`) making the moves and charges their
//! rules give, and named with their parameters; the view's queries (travel times equal to single
//! drives at 1 and 8 threads, expected legs by hand and unchanged by a seed's variation, nearest
//! docks with ties by index, zones stable across rebuilds).

#![allow(clippy::float_cmp, reason = "times here are exact sums of the networks' own times")]

use std::sync::{Arc, Mutex};

use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
use openmobisim_core_graph::geometry::{LonLat, ground_distance_metres};
use openmobisim_core_graph::layers::{BikeCost, StaticLayer, StaticLayerDefaults, StaticNetwork};
use openmobisim_core_graph::manhattan_grid;
use openmobisim_core_graph::network::{LinkSpec, RoadNetwork, RoadNetworkBuilder};
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_sim::services::decisions::{Decision, DecisionKind};
use openmobisim_core_sim::services::engine::{
    FleetDay, FleetEngine, Leg, NoLegs, ServiceLegs, ServiceRoads, ServiceState, VehicleState,
};
use openmobisim_core_sim::services::frozen::vary_legs;
use openmobisim_core_sim::services::outcome::{FleetOutcome, LegStatus};
use openmobisim_core_sim::services::policy::{
    ChargeRule, NoOp, Policy, PolicyDescription, Reset, Threshold, policy_builtin,
};
use openmobisim_core_sim::services::queries::{NearestKind, Zones};
use openmobisim_core_sim::services::roads::{FrozenRoads, RoadTimes, search_context};
use openmobisim_core_sim::services::view::View;
use openmobisim_core_sim::services::world::{FrozenWorld, ServiceWorld};
use openmobisim_core_sim::services::{
    ChargerRow, DockDefaults, DockRow, ServiceParts, ServiceSetup, TaxiDefaults, VehicleRow,
};
use openmobisim_core_sim::{LayerSetup, StaticLayers};
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::{EntityId, LinkId, NodeId};

// --- A policy that asks for fixed decisions and records what it saw -----------------------------

/// What a policy saw at one step.
#[derive(Clone, Debug, Default, PartialEq)]
struct Seen {
    t: f64,
    step: u32,
    rng_seed: u64,
    bikes: Vec<u32>,
    pickups: Vec<f64>,
    returns: Vec<f64>,
    moving: u64,
    taxi_state: Vec<VehicleState>,
    charger_in_use: Vec<u32>,
}

/// Asks for `plan[k]` at its step `k`, every `step_s`; records what it saw.
#[derive(Clone)]
struct Fixed {
    step_s: f64,
    plan: Vec<Vec<Decision>>,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Fixed {
    fn boxed(step_s: f64, plan: Vec<Vec<Decision>>) -> (Box<dyn Policy>, Arc<Mutex<Vec<Seen>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        (Box::new(Self { step_s, plan, seen: Arc::clone(&seen) }), seen)
    }
}

impl Policy for Fixed {
    fn describe(&self) -> PolicyDescription {
        PolicyDescription { name: "fixed".into(), parameters: vec![("step_s".into(), self.step_s)] }
    }
    fn step_s(&self) -> f64 {
        self.step_s
    }
    fn step(&mut self, view: &View<'_>, out: &mut Vec<Decision>) -> Result<(), String> {
        let mut s = Seen { t: view.t, step: view.step, rng_seed: view.rng_seed, ..Seen::default() };
        if let Some(d) = view.docked(0) {
            s.bikes = d.bikes.to_vec();
            s.pickups = d.pickups.to_vec();
            s.returns = d.returns.to_vec();
            s.moving = d.moving;
        }
        if let Some(t) = view.taxi(0) {
            s.taxi_state = t.state.to_vec();
            s.charger_in_use.clone_from(&t.charger_in_use);
        }
        self.seen.lock().unwrap().push(s);
        out.extend(self.plan.get(view.step as usize).cloned().unwrap_or_default());
        Ok(())
    }
}

// --- Docks on a grid ----------------------------------------------------------------------------

/// A 5 × 5 two-way grid of 200 m blocks, and its bike and walk layers.
fn grid() -> (RoadNetwork, StaticLayers) {
    let road = manhattan_grid(5, 200.0, false).0;
    let d = StaticLayerDefaults::SHIPPED;
    let derive = |layer| {
        Arc::new(StaticNetwork::derive(&road, layer, d, &mut Diagnostics::new()).expect("derives"))
    };
    let layers = StaticLayers {
        bike: Some(LayerSetup::new(derive(StaticLayer::Bike), BikeCost::Dedicated, d)),
        walk: Some(LayerSetup::new(derive(StaticLayer::Walk), BikeCost::Dedicated, d)),
    };
    (road, layers)
}

fn at(road: &RoadNetwork, node: usize) -> LonLat {
    road.node_lonlat(NodeId::from_index(node))
}

/// Three docks on the grid's first row, at 0, 200 and 600 m (A, B, C), snapped to the roads
/// if `with_roads`; and the vans' roads.
fn three(bikes: [u32; 3], capacity: u32, with_roads: bool) -> (ServiceSetup, Vans) {
    let (road, layers) = grid();
    let row = |id: &str, node: usize, b: u32| DockRow {
        dock_id: id.to_string(),
        name: None,
        hub_id: None,
        position: at(&road, node),
        capacity,
        bikes: Some(b),
    };
    let rows = vec![row("C", 3, bikes[2]), row("A", 0, bikes[0]), row("B", 1, bikes[1])];
    let s = ServiceSetup::bike_docked(
        "bike_share",
        &rows,
        layers.walk.as_ref().expect("walk"),
        layers.bike.as_ref().expect("bike"),
        with_roads.then_some(&road),
        DockDefaults::SHIPPED,
    )
    .expect("set up");
    let vans = Vans((0..road.node_count() as usize).map(|i| at(&road, i)).collect());
    (s, vans)
}

fn positions(s: &ServiceSetup) -> Vec<LonLat> {
    match s.parts() {
        ServiceParts::BikeDocked(docks, _) => docks.position.clone(),
        ServiceParts::Taxi(..) => panic!("a docked service"),
    }
}

/// Vans at 10 m/s as the crow flies between road nodes; nothing within reach (no taxi).
#[derive(Debug)]
struct Vans(Vec<LonLat>);

impl ServiceRoads for Vans {
    fn drive(&self, from: NodeId, to: NodeId, _: f64) -> (f64, f64) {
        let m = ground_distance_metres(self.0[from.index()], self.0[to.index()]);
        (m / 10.0, m)
    }
    fn reach_to(&self, _: NodeId, _: f64) -> Vec<(NodeId, f64)> {
        Vec::new()
    }
    fn reach_from(&self, _: NodeId, _: f64) -> Vec<(NodeId, f64)> {
        Vec::new()
    }
}

/// Rides at 4 m/s as the crow flies between docks, and a fixed walk to every destination.
#[derive(Debug)]
struct Legs(Vec<LonLat>);

impl ServiceLegs for Legs {
    fn ride_s(&self, _: u16, from: u32, to: u32) -> f64 {
        ground_distance_metres(self.0[from as usize], self.0[to as usize]) / 4.0
    }
    fn walk_to_destination_s(&self, _: usize, _: u32) -> f64 {
        500.0
    }
}

/// A leg of trip `trip` at the pickup dock at `at_s`, planned from `pickup` to `ret`.
fn trip(trip: u32, at_s: f64, pickup: u32, ret: u32, s: &ServiceSetup) -> Leg {
    Leg {
        service: 0,
        trip,
        weight: 1,
        request_s: at_s,
        origin: NodeId::new(0),
        destination: NodeId::new(1),
        pickup_dock: pickup,
        return_dock: ret,
        ride_s: Legs(positions(s)).ride_s(0, pickup, ret),
        egress_s: 60.0,
    }
}

fn dock_engine(
    s: &ServiceSetup,
    vans: Vans,
    legs: &[Leg],
    window: f64,
    seed: u64,
    policy: Option<Box<dyn Policy>>,
) -> FleetEngine {
    let day = FleetDay { window: (0.0, window), legs: legs.to_vec(), seed };
    FleetEngine::new(
        Arc::new(vec![s.clone()]),
        Arc::new(day),
        Arc::new(Legs(positions(s))),
        Arc::new(vans),
        policy,
    )
}

/// One fleet day of the docks over 0..`window` s.
fn docks_day(
    s: &ServiceSetup,
    vans: Vans,
    legs: &[Leg],
    window: f64,
    policy: Option<Box<dyn Policy>>,
) -> Result<FleetOutcome, String> {
    let out = dock_engine(s, vans, legs, window, 1, policy).run_day()?;
    assert_eq!(out.invariant_violations, 0, "bikes are conserved");
    Ok(out)
}

const A: u32 = 0;
const B: u32 = 1;
const C: u32 = 2;

fn mv(from_dock: u32, to_dock: u32, n: u32) -> Decision {
    Decision::Move { service: 0, from_dock, to_dock, n }
}

fn sum(v: &[f64]) -> f64 {
    v.iter().sum()
}

// --- Moves, refusals, failure and overflow ------------------------------------------------------

#[test]
fn an_operator_moves_bikes_by_road_and_the_count_holds() {
    // A starts empty; the operator moves two bikes from C to A at once; a traveller arriving
    // at A after the van finds one there.
    let (s, vans) = three([0, 0, 3], 3, true);
    let (van_s, metres) = vans.drive(NodeId::from_index(3), NodeId::from_index(0), 0.0);
    let (op, seen) = Fixed::boxed(900.0, vec![vec![mv(C, A, 2)]]);
    let t = trip(0, 1000.0, A, B, &s);
    let out = docks_day(&s, vans, &[t], 3600.0, Some(op)).expect("runs");
    let m = &out.decisions[0];
    assert_eq!(
        (m.kind, m.moved, m.accepted, m.reason.clone()),
        (DecisionKind::Move, 2, true, None)
    );
    let d = DockDefaults::SHIPPED;
    assert_eq!(m.finished_s, van_s + 2.0 * d.handling_s, "60 s of drive and 2 × 30 s of handling");
    assert_eq!((m.executed_s, m.metres), (0.0, metres));
    assert_eq!(out.legs[0].picked, Some(A), "the van came first");
    let bins = out.docks[0].as_ref().unwrap();
    assert_eq!(sum(&bins.moved_out), 2.0);
    assert_eq!(sum(&bins.moved_in), 2.0);
    // Steps every 900 s from 0 until the hour ends; the second saw the bikes moved.
    let seen = seen.lock().unwrap();
    let times: Vec<f64> = seen.iter().map(|x| x.t).collect();
    assert_eq!(times, [0.0, 900.0, 1800.0, 2700.0]);
    assert_eq!(seen[0].bikes, [0, 0, 3]);
    assert_eq!(seen[1].bikes, [2, 0, 1]);
    // The counts since the last step: the pickup at 1000 s and its return (1110 s), then none.
    assert_eq!(
        (seen[2].pickups.clone(), seen[2].returns.clone()),
        (vec![1.0, 0.0, 0.0], vec![0.0, 1.0, 0.0])
    );
    assert_eq!(sum(&seen[3].pickups) + sum(&seen[3].returns), 0.0, "cleared after each step");
}

#[test]
fn a_task_that_cannot_be_done_is_refused_with_why_and_the_run_goes_on() {
    let (s, vans) = three([0, 1, 1], 3, true);
    let plan = vec![mv(A, B, 1), mv(B, B, 1), mv(B, 7, 1), mv(C, A, 0), mv(C, A, 5)];
    let (op, _) = Fixed::boxed(900.0, vec![plan]);
    let out = docks_day(&s, vans, &[], 3600.0, Some(op)).expect("runs");
    let why: Vec<Option<String>> = out.decisions.iter().map(|m| m.reason.clone()).collect();
    assert_eq!(why[0].as_deref(), Some("no bike at dock A"));
    assert_eq!(why[1].as_deref(), Some("dock B is both ends"));
    assert_eq!(why[2].as_deref(), Some("no dock 7"));
    assert_eq!(why[3].as_deref(), Some("no bike asked for"));
    assert_eq!((why[4].clone(), out.decisions[4].moved), (None, 1), "as many as there are");
    assert!(out.decisions[..4].iter().all(|r| !r.accepted && r.executed_s.is_nan()));
}

#[test]
fn an_operator_that_fails_stops_the_loading_with_its_message() {
    #[derive(Clone)]
    struct Broken;
    impl Policy for Broken {
        fn describe(&self) -> PolicyDescription {
            PolicyDescription { name: "broken".into(), parameters: Vec::new() }
        }
        fn step_s(&self) -> f64 {
            600.0
        }
        fn step(&mut self, _: &View<'_>, _: &mut Vec<Decision>) -> Result<(), String> {
            Err("out of fuel".to_string())
        }
    }
    let (s, vans) = three([1, 1, 1], 3, true);
    let err = docks_day(&s, vans, &[], 3600.0, Some(Box::new(Broken))).unwrap_err();
    assert_eq!(err, "out of fuel");
}

#[test]
fn bikes_brought_to_a_full_dock_go_to_the_nearest_with_room() {
    let (s, vans) = three([2, 0, 3], 3, true);
    // A has one free slot: two bikes from C, one into A and one into B.
    let (op, seen) = Fixed::boxed(900.0, vec![vec![mv(C, A, 2)]]);
    let out = docks_day(&s, vans, &[], 3600.0, Some(op)).expect("runs");
    assert_eq!(seen.lock().unwrap()[1].bikes, [3, 1, 1]);
    assert_eq!(out.overflow_max[0], 0.0);
}

// --- Docks: new ---------------------------------------------------------------------------------

#[test]
fn docks_refuse_moves_without_roads_targets_above_slots_and_unknown_services() {
    let (s, vans) = three([0, 1, 3], 3, false);
    let plan = vec![
        mv(C, A, 1),
        Decision::Target { service: 0, dock: A, level: 4 },
        Decision::Target { service: 0, dock: 9, level: 1 },
        Decision::Target { service: 0, dock: A, level: 2 },
        Decision::Target { service: 0, dock: A, level: 1 },
        Decision::Move { service: 3, from_dock: C, to_dock: A, n: 1 },
        Decision::Relocate { service: 0, vehicle: 0, node: NodeId::new(0) },
        Decision::Charge { service: 0, vehicle: 0, charger: 0, to_soc: 0.9 },
    ];
    let (op, _) = Fixed::boxed(900.0, vec![plan]);
    let out = docks_day(&s, vans, &[], 3600.0, Some(op)).expect("runs");
    let why: Vec<Option<&str>> = out.decisions.iter().map(|m| m.reason.as_deref()).collect();
    assert_eq!(
        why,
        [
            Some("no roads"),
            Some("level 4 is above dock A's 3 slots"),
            Some("no dock 9"),
            None,
            Some("a second target for dock A at this step"),
            Some("no service 3"),
            Some("service bike_share is not a taxi fleet"),
            Some("service bike_share is not a taxi fleet"),
        ]
    );
    // The accepted target of 2 at A, alone: the solver moves nothing (no dock gives without
    // a target of its own); without roads a move would be refused anyway.
    assert_eq!(out.decisions.len(), 8, "no move made");
}

#[test]
fn targets_are_solved_into_moves_from_the_nearest_spare() {
    // A wants 2, B wants 1 and C (3 bikes) wants 1: A, the shortest, is served first from its
    // nearest spare, C, then B gets nothing (C has given its spare 2).
    let (s, vans) = three([0, 0, 3], 3, true);
    let plan = vec![
        Decision::Target { service: 0, dock: A, level: 2 },
        Decision::Target { service: 0, dock: B, level: 1 },
        Decision::Target { service: 0, dock: C, level: 1 },
    ];
    let (op, seen) = Fixed::boxed(900.0, vec![plan]);
    let out = docks_day(&s, vans, &[], 3600.0, Some(op)).expect("runs");
    let made: Vec<_> = out.decisions.iter().filter(|r| r.from_target).collect();
    assert_eq!(made.len(), 1);
    assert_eq!(made[0].parameters, [("from_dock", 2.0), ("to_dock", 0.0), ("n", 2.0)]);
    assert_eq!((made[0].moved, made[0].accepted), (2, true));
    assert!(out.decisions[..3].iter().all(|r| r.accepted && r.kind == DecisionKind::Target));
    assert_eq!(seen.lock().unwrap()[1].bikes, [2, 0, 1]);
}

#[test]
fn bikes_in_vans_are_counted_in_the_conservation() {
    let (s, vans) = three([0, 0, 3], 3, true);
    let arrival = vans.drive(NodeId::from_index(3), NodeId::from_index(0), 0.0).0 + 60.0;
    let (op, _) = Fixed::boxed(900.0, vec![vec![mv(C, A, 2)]]);
    let mut e = dock_engine(&s, vans, &[], 3600.0, 1, Some(op));
    // The van leaves at 0 s and arrives at about 120 s (600 m at 10 m/s, 2 × 30 s handling).
    e.advance_to(60.0).unwrap();
    let ServiceState::BikeDocked { bikes, riding, moving, fleet } = &e.state().services[0] else {
        panic!("a docked service")
    };
    assert_eq!((bikes.clone(), *riding, *moving, *fleet), (vec![0, 0, 1], 0, 2, 3));
    assert!(e.state().services[0].conserved());
    assert_eq!(e.view().docked(0).unwrap().moving, 2, "the view sees the van");
    e.advance_to(arrival).unwrap();
    let ServiceState::BikeDocked { bikes, moving, .. } = &e.state().services[0] else {
        panic!("a docked service")
    };
    assert_eq!((bikes.clone(), *moving), (vec![2, 0, 1], 0));
    assert_eq!(e.decisions()[0].finished_s, arrival);
}

/// Moves bikes between docks chosen by its draws: from a dock with bikes to another.
#[derive(Clone)]
struct Random {
    steps: u32,
}

impl Policy for Random {
    fn describe(&self) -> PolicyDescription {
        PolicyDescription { name: "random".into(), parameters: Vec::new() }
    }
    fn step_s(&self) -> f64 {
        300.0
    }
    fn step(&mut self, view: &View<'_>, out: &mut Vec<Decision>) -> Result<(), String> {
        self.steps += 1;
        let d = view.docked(0).ok_or("a docked service")?;
        let r = view.rng_seed;
        let from = u32::try_from(r % 3).unwrap();
        let to = u32::try_from((r >> 8) % 3).unwrap();
        let n = u32::try_from((r >> 16) % 3).unwrap() + self.steps % 2;
        if d.bikes[from as usize] > 0 {
            out.push(mv(from, to, n));
        }
        Ok(())
    }
}

#[test]
fn the_same_seed_gives_the_same_decisions_and_a_copy_continues_as_the_original() {
    let (s, _) = three([2, 1, 3], 3, true);
    let legs: Vec<Leg> =
        (0..30).map(|k| trip(k, f64::from(k) * 117.0, k % 3, (k + 1) % 3, &s)).collect();
    let run = |seed: u64| {
        let (_, vans) = three([2, 1, 3], 3, true);
        let mut e = dock_engine(&s, vans, &legs, 3600.0, seed, Some(Box::new(Random { steps: 0 })));
        e.advance_to(1700.0).unwrap();
        let copy = e.clone();
        let a = e.run_day().unwrap();
        let b = copy.run_day().unwrap();
        // Printed, so the times not yet known (`NaN`) compare equal.
        assert_eq!(format!("{a:?}"), format!("{b:?}"), "a copy continues as the original");
        assert_eq!(a.invariant_violations, 0);
        a
    };
    let (a, b) = (run(1), run(1));
    assert_eq!(format!("{:?}", a.decisions), format!("{:?}", b.decisions));
    assert!(a.decisions.iter().any(|r| r.accepted), "something moved");
    let c = run(2);
    assert_ne!(format!("{:?}", a.decisions), format!("{:?}", c.decisions), "another seed");
}

#[test]
fn a_policy_that_decides_nothing_changes_nothing() {
    let (s, vans) = three([1, 0, 2], 2, true);
    let legs: Vec<Leg> =
        (0..20).map(|k| trip(k, f64::from(k) * 151.0, k % 3, (k + 2) % 3, &s)).collect();
    let none = docks_day(&s, vans, &legs, 3600.0, None).unwrap();
    let (_, vans) = three([1, 0, 2], 2, true);
    let noop = docks_day(&s, vans, &legs, 3600.0, Some(Box::new(NoOp::DEFAULT))).unwrap();
    assert_eq!((none.steps, noop.steps), (0, 4));
    assert_eq!(format!("{:?}", none.legs), format!("{:?}", noop.legs));
    assert_eq!(none.docks, noop.docks);
    assert!(noop.decisions.is_empty());
    // And for taxis.
    let net = line();
    let setup = taxi_fleet(&net, &[("v", 0, 0.8), ("w", 4, 0.3)], &[("c", 2, 1)]);
    let requests = [(100.0, 2, 5), (400.0, 6, 1), (900.0, 3, 8)];
    let none = taxi_day(&net, &setup, &requests, None).unwrap();
    let noop = taxi_day(&net, &setup, &requests, Some(Box::new(NoOp::DEFAULT))).unwrap();
    assert_eq!(format!("{:?}", none.legs), format!("{:?}", noop.legs));
    assert_eq!(format!("{:?}", none.taxis), format!("{:?}", noop.taxis));
}

// --- Taxis on a line ----------------------------------------------------------------------------

fn lonlat(i: u32) -> LonLat {
    LonLat::new(4.8 + 0.0128 * f64::from(i), 45.7)
}

/// Ten nodes on a parallel, a kilometre apart, joined both ways.
fn line() -> RoadNetwork {
    let mut b = RoadNetworkBuilder::new();
    for i in 0..10 {
        b.add_node(format!("n{i}"), lonlat(i));
    }
    for i in 0..9 {
        for (from, to) in [(i, i + 1), (i + 1, i)] {
            let mut spec = LinkSpec::new(RoadClass::Primary);
            spec.length_m = Some(1000.0);
            b.add_link(format!("l{from}_{to}"), format!("n{from}"), format!("n{to}"), spec);
        }
    }
    b.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
        .expect("buildable")
}

/// The roads of the line: 60 s and 1 000 m a node, whatever the time.
#[derive(Debug)]
struct Line {
    nodes: Vec<NodeId>,
}

impl Line {
    fn new(net: &RoadNetwork) -> Self {
        let ids = net.node_external_ids();
        let nodes =
            (0..10).map(|i| ids.typed_id_of::<NodeId>(&format!("n{i}")).expect("node")).collect();
        Self { nodes }
    }

    fn at(&self, node: NodeId) -> i64 {
        i64::try_from(self.nodes.iter().position(|&n| n == node).expect("a node of the line"))
            .expect("small")
    }

    fn within(&self, from: NodeId, bound: f64) -> Vec<(NodeId, f64)> {
        let i = self.at(from);
        let mut out: Vec<(NodeId, f64)> = (0..10_i64)
            .filter_map(|j| {
                #[allow(clippy::cast_precision_loss, reason = "small")]
                let s = 60.0 * (i - j).abs() as f64;
                (s <= bound).then(|| (self.nodes[usize::try_from(j).expect("small")], s))
            })
            .collect();
        out.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.raw().cmp(&b.0.raw())));
        out
    }
}

impl ServiceRoads for Line {
    fn drive(&self, from: NodeId, to: NodeId, _t: f64) -> (f64, f64) {
        #[allow(clippy::cast_precision_loss, reason = "small")]
        let k = (self.at(from) - self.at(to)).abs() as f64;
        (60.0 * k, 1000.0 * k)
    }
    fn reach_to(&self, to: NodeId, bound: f64) -> Vec<(NodeId, f64)> {
        self.within(to, bound)
    }
    fn reach_from(&self, from: NodeId, bound: f64) -> Vec<(NodeId, f64)> {
        self.within(from, bound)
    }
}

/// A fleet of `(id, node, soc)` vehicles and `(id, node, plugs)` chargers, shipped defaults.
fn taxi_fleet(
    net: &RoadNetwork,
    vehicles: &[(&str, u32, f64)],
    chargers: &[(&str, u32, u32)],
) -> ServiceSetup {
    taxi_fleet_with(net, vehicles, chargers, TaxiDefaults::SHIPPED)
}

fn taxi_fleet_with(
    net: &RoadNetwork,
    vehicles: &[(&str, u32, f64)],
    chargers: &[(&str, u32, u32)],
    d: TaxiDefaults,
) -> ServiceSetup {
    let vehicles: Vec<VehicleRow> = vehicles
        .iter()
        .map(|&(id, at, soc)| VehicleRow {
            vehicle_id: id.into(),
            position: lonlat(at),
            soc: Some(soc),
        })
        .collect();
    let chargers: Vec<ChargerRow> = chargers
        .iter()
        .map(|&(id, at, plugs)| ChargerRow {
            charger_id: id.into(),
            hub_id: None,
            position: lonlat(at),
            plugs,
            kw: None,
        })
        .collect();
    ServiceSetup::taxi("taxi", net, &vehicles, &chargers, d).expect("set up")
}

/// One day of `(second, from node, to node)` requests on the line over 0..3600 s.
fn taxi_day(
    net: &RoadNetwork,
    setup: &ServiceSetup,
    requests: &[(f64, usize, usize)],
    policy: Option<Box<dyn Policy>>,
) -> Result<FleetOutcome, String> {
    let line = Line::new(net);
    let legs = (0_u32..)
        .zip(requests)
        .map(|(k, &(t, from, to))| Leg {
            service: 0,
            trip: k,
            weight: 1,
            request_s: t,
            origin: line.nodes[from],
            destination: line.nodes[to],
            pickup_dock: u32::MAX,
            return_dock: u32::MAX,
            ride_s: f64::NAN,
            egress_s: f64::NAN,
        })
        .collect();
    let day = FleetDay { window: (0.0, 3600.0), legs, seed: 1 };
    let out = FleetEngine::new(
        Arc::new(vec![setup.clone()]),
        Arc::new(day),
        Arc::new(NoLegs),
        Arc::new(line),
        policy,
    )
    .run_day()?;
    assert_eq!(out.invariant_violations, 0, "vehicles are conserved");
    Ok(out)
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn a_relocated_taxi_drives_there_empty_and_waits_for_a_request() {
    let net = line();
    let nodes = Line::new(&net).nodes;
    // No dispatch delay: the vehicle waiting at the pickup is there at the request.
    let d = TaxiDefaults { dispatch_delay_s: 0.0, ..TaxiDefaults::SHIPPED };
    let setup = taxi_fleet_with(&net, &[("v", 0, 0.8)], &[], d);
    let relocate = Decision::Relocate { service: 0, vehicle: 0, node: nodes[5] };
    let (op, seen) = Fixed::boxed(900.0, vec![vec![relocate]]);
    let out = taxi_day(&net, &setup, &[(400.0, 5, 6)], Some(op)).unwrap();
    let r = &out.decisions[0];
    assert!(r.accepted);
    // 5 km at 60 s a km: there at 300 s.
    assert_eq!((r.executed_s, r.finished_s, r.metres), (0.0, 300.0, 5000.0));
    let o = out.taxis[0].as_ref().unwrap().requests[0];
    assert!(close(o.assigned_s, 400.0) && close(o.picked_s, 400.0), "the vehicle waits there");
    let v = out.taxis[0].as_ref().unwrap().vehicles[0];
    assert!(close(v.km_empty, 5.0) && close(v.km_occupied, 1.0));
    assert!(close(v.soc_end, 0.8 - 6.0 * 0.18 / 60.0));
    assert_eq!(seen.lock().unwrap()[0].taxi_state, [VehicleState::Idle]);
    assert_eq!(out.legs[0].status, LegStatus::Completed);
}

#[test]
fn a_taxi_sent_to_charge_drives_charges_to_the_share_asked_and_is_freed() {
    let net = line();
    let setup = taxi_fleet(&net, &[("v", 0, 0.5)], &[("c", 3, 1)]);
    let charge = Decision::Charge { service: 0, vehicle: 0, charger: 0, to_soc: 0.9 };
    let (op, seen) = Fixed::boxed(900.0, vec![vec![charge]]);
    let out = taxi_day(&net, &setup, &[], Some(op)).unwrap();
    let r = &out.decisions[0];
    assert!(r.accepted);
    // 3 km (180 s, 0.54 kWh of a 60 kWh battery), then (0.9 − 0.491) × 60 kWh at 50 kW.
    let soc = 0.5 - 0.54 / 60.0;
    let kwh = (0.9 - soc) * 60.0;
    assert!(close(r.finished_s, 180.0 + kwh / 50.0 * 3600.0), "{}", r.finished_s);
    let v = out.taxis[0].as_ref().unwrap().vehicles[0];
    assert!(close(v.kwh_charged, kwh) && close(v.soc_end, 0.9) && close(v.km_empty, 3.0));
    let seen = seen.lock().unwrap();
    assert_eq!(
        (seen[1].taxi_state.clone(), seen[1].charger_in_use.clone()),
        (vec![VehicleState::Charging], vec![1])
    );
    assert_eq!(seen[3].taxi_state, [VehicleState::Idle], "freed after the charge");
}

#[test]
fn taxi_decisions_are_refused_with_why() {
    let net = line();
    let nodes = Line::new(&net).nodes;
    // Charging below 0.15 (the reserve), so `w`, at 0.16, is not sent to charge at the start.
    let d = TaxiDefaults { charge_below: 0.15, ..TaxiDefaults::SHIPPED };
    let setup = taxi_fleet_with(&net, &[("v", 0, 0.5), ("w", 4, 0.16)], &[("c", 3, 1)], d);
    let plan = vec![
        Decision::Relocate { service: 0, vehicle: 0, node: nodes[2] },
        Decision::Relocate { service: 0, vehicle: 0, node: nodes[3] },
        Decision::Charge { service: 0, vehicle: 0, charger: 0, to_soc: 0.9 },
        Decision::Relocate { service: 0, vehicle: 9, node: nodes[3] },
        Decision::Relocate { service: 0, vehicle: 1, node: nodes[4] },
        Decision::Relocate { service: 0, vehicle: 1, node: nodes[9] },
        Decision::Charge { service: 0, vehicle: 1, charger: 0, to_soc: 0.1 },
        Decision::Charge { service: 0, vehicle: 1, charger: 5, to_soc: 0.9 },
        mv(0, 1, 1),
        Decision::Target { service: 0, dock: 0, level: 1 },
    ];
    let (op, _) = Fixed::boxed(900.0, vec![plan]);
    let out = taxi_day(&net, &setup, &[], Some(op)).unwrap();
    let why: Vec<Option<&str>> = out.decisions.iter().map(|m| m.reason.as_deref()).collect();
    assert_eq!(
        why,
        [
            None,
            Some("vehicle v is not idle (relocating)"),
            Some("vehicle v is not idle (relocating)"),
            Some("no vehicle 9"),
            Some("vehicle w is already there"),
            Some("vehicle w has not the charge for the drive and the reserve"),
            Some("to_soc 0.1 is not above vehicle w's charge 0.16 and at most 1"),
            Some("no charger 5"),
            Some("service taxi is not a docked service"),
            Some("service taxi is not a docked service"),
        ]
    );
}

// --- The built-in policies ----------------------------------------------------------------------

/// The moves the target solver made, as `(second, from, to, bikes)`.
fn solver_moves(out: &FleetOutcome) -> Vec<(f64, u32, u32, u32)> {
    out.decisions
        .iter()
        .filter(|r| r.from_target && r.accepted)
        .map(|r| {
            let p = |k: &str| {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "small")]
                let v = r.parameters.iter().find(|x| x.0 == k).expect("a parameter").1 as u32;
                v
            };
            (r.t_s, p("from_dock"), p("to_dock"), r.moved)
        })
        .collect()
}

#[test]
fn reset_brings_the_bikes_back_to_their_start_of_day_docks() {
    // A starts with three bikes, B and C empty; three travellers ride them from A to C at
    // 8:00, 8:01 and 8:02; from 9:00 the reset brings them back: one move, C to A, three bikes,
    // at 9:00, and nothing after (every dock at its count again).
    let (s, vans) = three([3, 0, 0], 3, true);
    let legs: Vec<Leg> =
        (0..3).map(|k| trip(k, 8.0 * 3600.0 + 60.0 * f64::from(k), A, C, &s)).collect();
    let reset = Reset::new(900.0, 9.0 * 3600.0);
    let out = docks_day(&s, vans, &legs, 10.0 * 3600.0, Some(Box::new(reset))).expect("runs");
    assert_eq!(solver_moves(&out), [(9.0 * 3600.0, C, A, 3)]);
    assert!(out.decisions.iter().all(|r| r.t_s >= 9.0 * 3600.0), "nothing before 9:00");
    // From the window's start (the default), it acts at each step: the first move at 8:15.
    let (_, vans) = three([3, 0, 0], 3, true);
    let out = docks_day(&s, vans, &legs, 10.0 * 3600.0, Some(Box::new(Reset::new(900.0, 0.0))))
        .expect("runs");
    assert_eq!(solver_moves(&out), [(8.25 * 3600.0, C, A, 3)]);
}

#[test]
fn threshold_tops_up_the_empty_docks_from_the_full_ones() {
    // Three slots a dock, low 0.25, high 0.5: the middle is 1.125 bikes. A (3 bikes, above
    // 1.5) gives down to 2 (rounded up), B and C (none, below 0.75) want 1 (rounded down): A
    // gives one bike, to B (the tie between B and C goes to the lower index). At the later
    // steps A (2 bikes, still above 1.5) and C (none) are past a threshold again, but A is at
    // its target and has none to spare: C stays short. Equivalently, a dock past a threshold
    // gives `floor(bikes − middle)` bikes and takes `floor(middle − bikes)`.
    let (s, vans) = three([3, 0, 0], 3, true);
    let rule = Threshold::new(0.25, 0.5, 900.0).expect("valid");
    assert_eq!((rule.target(3, 3), rule.target(0, 3), rule.target(1, 3)), (Some(2), Some(1), None));
    let out = docks_day(&s, vans, &[], 3600.0, Some(Box::new(rule))).expect("runs");
    assert_eq!(solver_moves(&out), [(0.0, A, B, 1)]);
    let targets: Vec<_> = out.decisions.iter().filter(|r| r.kind == DecisionKind::Target).collect();
    assert_eq!(targets.len(), 3 + 3 * 2, "three docks past a threshold at 0 s, then A and C");
    assert!(Threshold::new(0.8, 0.2, 900.0).unwrap_err().contains("low < high"));
}

#[test]
fn charge_rule_sends_a_low_idle_taxi_to_its_nearest_charger() {
    let net = line();
    // `v` at node 0 with 30 % (above the fleet's own 20 %, below the rule's 50 %); `w` at
    // node 9 with 90 %. Chargers at nodes 4 (`far`) and 2 (`near`): `v` goes to `near`.
    let setup = taxi_fleet(&net, &[("v", 0, 0.3), ("w", 9, 0.9)], &[("far", 4, 1), ("near", 2, 1)]);
    let ServiceParts::Taxi(fleet, _) = setup.parts() else { panic!("a taxi fleet") };
    let near = u32::try_from(fleet.charger_ids.iter().position(|c| c == "near").unwrap()).unwrap();
    let rule = ChargeRule::new(0.5, 0.9, 900.0).expect("valid");
    let out = taxi_day(&net, &setup, &[], Some(Box::new(rule))).unwrap();
    assert_eq!(out.decisions.len(), 1, "one vehicle low, sent once (charging after)");
    let r = &out.decisions[0];
    assert_eq!(r.kind, DecisionKind::Charge);
    assert!(r.accepted);
    assert_eq!(r.parameters, [("vehicle", 0.0), ("charger", f64::from(near)), ("to_soc", 0.9)]);
    assert!(close(r.metres, 2000.0), "2 km to the near charger");
    assert!(ChargeRule::new(0.9, 0.5, 900.0).is_err());
}

#[test]
fn the_built_in_policies_are_named_with_their_parameters() {
    let none = std::collections::BTreeMap::new();
    let describe = |name: &str, p: &[(&str, f64)]| {
        let p = p.iter().map(|&(k, v)| (k.to_string(), v)).collect();
        policy_builtin(name, &p).map(|x| x.describe())
    };
    assert_eq!(
        describe("threshold", &[("low", 0.1)]).unwrap(),
        PolicyDescription {
            name: "threshold".into(),
            parameters: vec![("low".into(), 0.1), ("high".into(), 0.8), ("step_s".into(), 900.0)],
        }
    );
    assert_eq!(
        describe("reset", &[]).unwrap().parameters,
        [("step_s".into(), 900.0), ("from_s".into(), 0.0)]
    );
    assert_eq!(
        describe("charge_rule", &[("step_s", 300.0)]).unwrap().parameters,
        [("charge_below".into(), 0.2), ("charge_to".into(), 0.8), ("step_s".into(), 300.0)]
    );
    assert_eq!(policy_builtin("noop", &none).unwrap().describe().name, "noop");
    let unknown = describe("nope", &[]).unwrap_err();
    assert!(unknown.contains("noop, reset, threshold, charge_rule"), "{unknown}");
    let bad = describe("threshold", &[("lo", 0.1)]).unwrap_err();
    assert!(bad.contains("\"lo\"") && bad.contains("low, high, step_s"), "{bad}");
}

#[test]
fn a_reset_acting_while_its_van_is_on_the_road_moves_no_bike_twice() {
    // The reset of `reset_brings_the_bikes_back…`, every 30 s: shorter than the van's drive plus
    // its handling (3 × 30 s), so the van is still on the road at the next steps. The bikes
    // moved over the day are the three that went to C, once.
    let (s, vans) = three([3, 0, 0], 3, true);
    let legs: Vec<Leg> =
        (0..3).map(|k| trip(k, 8.0 * 3600.0 + 60.0 * f64::from(k), A, C, &s)).collect();
    let reset = Reset::new(30.0, 9.0 * 3600.0);
    let out = docks_day(&s, vans, &legs, 10.0 * 3600.0, Some(Box::new(reset))).expect("runs");
    let moves = solver_moves(&out);
    let moved: u32 = moves.iter().map(|m| m.3).sum();
    assert_eq!(moved, 3, "every bike moved once: {moves:?}");
    let last = out.docks[0].as_ref().expect("docks");
    let n = last.bins;
    assert!(
        (0..3).all(|k| last.bikes_mean[k * n + n - 1] >= 0.0),
        "counts stay whole: {:?}",
        last.bikes_mean
    );
}

#[test]
fn a_reset_with_a_small_van_counts_the_bikes_already_on_their_way() {
    // As above with vans of two bikes: the first step sends two of A's three back; at the next
    // steps, while that van is on the road, A still shows none. Bikes on their way must count,
    // or a second van brings two more and A ends above its start.
    let (road, layers) = grid();
    let row = |id: &str, node: usize, b: u32| DockRow {
        dock_id: id.to_string(),
        name: None,
        hub_id: None,
        position: at(&road, node),
        capacity: 5,
        bikes: Some(b),
    };
    let rows = vec![row("C", 3, 0), row("A", 0, 3), row("B", 1, 0)];
    let small = DockDefaults { van_bikes: 2.0, ..DockDefaults::SHIPPED };
    let s = ServiceSetup::bike_docked(
        "bike_share",
        &rows,
        layers.walk.as_ref().expect("walk"),
        layers.bike.as_ref().expect("bike"),
        Some(&road),
        small,
    )
    .expect("set up");
    let vans = Vans((0..road.node_count() as usize).map(|i| at(&road, i)).collect());
    let legs: Vec<Leg> =
        (0..3).map(|k| trip(k, 8.0 * 3600.0 + 60.0 * f64::from(k), A, C, &s)).collect();
    let reset = Reset::new(30.0, 9.0 * 3600.0);
    let out = docks_day(&s, vans, &legs, 10.0 * 3600.0, Some(Box::new(reset))).expect("runs");
    let moves = solver_moves(&out);
    let moved: u32 = moves.iter().map(|m| m.3).sum();
    assert_eq!(moved, 3, "three bikes back, none twice: {moves:?}");
}

#[test]
fn a_threshold_rule_counts_the_bikes_already_on_their_way() {
    // Docks of 10 slots: A empty, B and C full; low 0.25, high 0.75: A's target is the middle,
    // 5. Vans of two bikes, a step every 30 s, shorter than a van's trip: while the first vans
    // are on the road A still shows none. Counting the bikes on their way, A ends at its 5.
    let (road, layers) = grid();
    let row = |id: &str, node: usize, b: u32| DockRow {
        dock_id: id.to_string(),
        name: None,
        hub_id: None,
        position: at(&road, node),
        capacity: 10,
        bikes: Some(b),
    };
    let rows = vec![row("C", 3, 10), row("A", 0, 0), row("B", 1, 10)];
    let small = DockDefaults { van_bikes: 2.0, ..DockDefaults::SHIPPED };
    let s = ServiceSetup::bike_docked(
        "bike_share",
        &rows,
        layers.walk.as_ref().expect("walk"),
        layers.bike.as_ref().expect("bike"),
        Some(&road),
        small,
    )
    .expect("set up");
    let vans = Vans((0..road.node_count() as usize).map(|i| at(&road, i)).collect());
    let rule = Threshold::new(0.25, 0.75, 30.0).expect("a rule");
    let out = docks_day(&s, vans, &[], 600.0, Some(Box::new(rule))).expect("runs");
    let to_a: u32 = solver_moves(&out).iter().filter(|m| m.2 == A).map(|m| m.3).sum();
    assert_eq!(to_a, 5, "A brought to its middle once: {:?}", solver_moves(&out));
}

// --- The view's queries: travel times, expected legs, nearest, zones ----------------------------

/// The grid's city, owned, for `setups` and `legs`, at `times` (free flow if `None`).
fn grid_world(setups: &[ServiceSetup], legs: &[Leg], times: Option<RoadTimes>) -> FrozenWorld {
    let (road, layers) = grid();
    let road = Arc::new(road);
    let turns = Arc::new(TurnTable::build(&road, road.signals()));
    let times = times.unwrap_or_else(|| RoadTimes::free_flow(&road));
    FrozenWorld::new(
        Arc::clone(&road),
        turns,
        Arc::new(layers.walk.expect("walk")),
        Arc::new(layers.bike.expect("bike")),
        times,
        Arc::new(setups.to_vec()),
        legs,
    )
}

/// Road times on the grid that change by the bin: every third link slower in bins 30 to 33.
fn grid_times(road: &RoadNetwork) -> RoadTimes {
    let rows: Vec<(u32, u32, f64)> = (0..road.link_count())
        .step_by(3)
        .flat_map(|l| {
            let ff = road.free_flow_time(LinkId::new(l)).get();
            (30..34).map(move |b| (l, b, ff * f64::from(1 + (l + b) % 4)))
        })
        .collect();
    RoadTimes::from_rows(road, 900.0, &rows).expect("valid rows")
}

#[test]
fn travel_times_equal_single_drives_at_any_number_of_threads() {
    let (road, _) = grid();
    let turns = TurnTable::build(&road, road.signals());
    let ctx = search_context(&road, &turns);
    let roads = FrozenRoads::new(&road, &ctx, grid_times(&road));
    let from: Vec<NodeId> = (0..10).map(|i| NodeId::from_index(2 * i)).collect();
    let to: Vec<NodeId> = (0..10).map(|i| NodeId::from_index(24 - 2 * i)).collect();
    // Leaving inside the slow bins, so the times are time-dependent.
    let t = 30.0 * 900.0 + 400.0;
    let single: Vec<f64> = from
        .iter()
        .flat_map(|&a| to.iter().map(move |&b| (a, b)))
        .map(|(a, b)| roads.drive(a, b, t).0)
        .collect();
    assert!(single.iter().all(|s| s.is_finite()), "every pair connected");
    assert_ne!(
        single,
        FrozenRoads::new(&road, &ctx, RoadTimes::free_flow(&road)).travel_times(
            &from,
            &to,
            t,
            f64::INFINITY
        ),
        "the road times matter"
    );
    for threads in [1, 8] {
        let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().expect("a pool");
        let runs = 20;
        let clock = std::time::Instant::now();
        let mut m = Vec::new();
        for _ in 0..runs {
            m = pool.install(|| roads.travel_times(&from, &to, t, f64::INFINITY));
        }
        let per = clock.elapsed().as_secs_f64() / f64::from(runs);
        println!(
            "travel_times 10 x 10 nodes on the 5 x 5 grid, {threads} threads: {:.1} µs a query",
            per * 1e6
        );
        assert_eq!(m, single, "value for value, {threads} threads");
    }
    // Past max_s: infinite; within it, the same.
    let mut sorted = single.clone();
    sorted.sort_by(f64::total_cmp);
    let max_s = sorted[sorted.len() / 2];
    let bounded = roads.travel_times(&from, &to, t, max_s);
    for (b, s) in bounded.iter().zip(&single) {
        assert_eq!(*b, if *s <= max_s { *s } else { f64::INFINITY });
    }
    assert!(bounded.iter().any(|s| s.is_infinite()) && bounded.iter().any(|s| s.is_finite()));
}

#[test]
fn a_view_s_travel_times_are_the_world_s_drives_at_the_step_and_cached() {
    let (s, _) = three([1, 1, 1], 3, true);
    let (road, _) = grid();
    let world = Arc::new(grid_world(std::slice::from_ref(&s), &[], Some(grid_times(&road))));
    let day = FleetDay { window: (30.0 * 900.0, 34.0 * 900.0), legs: Vec::new(), seed: 1 };
    let mut engine = FleetEngine::in_world(Arc::new(vec![s]), Arc::new(day), world.clone(), None);
    engine.advance_to(30.0 * 900.0 + 400.0).expect("runs");
    let view = engine.view();
    let from = [NodeId::from_index(0), NodeId::from_index(12)];
    let to = [NodeId::from_index(24), NodeId::from_index(3), NodeId::from_index(0)];
    let m = view.queries.travel_times(&from, &to, f64::INFINITY).expect("a query");
    let turns = TurnTable::build(&road, road.signals());
    let ctx = search_context(&road, &turns);
    let roads = FrozenRoads::new(&road, &ctx, grid_times(&road));
    for (i, &a) in from.iter().enumerate() {
        for (j, &b) in to.iter().enumerate() {
            assert_eq!(m[i * 3 + j], roads.drive(a, b, view.t).0);
        }
    }
    let again = view.queries.travel_times(&from, &to, f64::INFINITY).expect("a query");
    assert!(Arc::ptr_eq(&m, &again), "cached within the step");
    assert!(view.queries.travel_times(&from, &to, -1.0).is_err());
}

#[test]
fn expected_legs_count_the_world_s_legs_by_dock_and_bin_whatever_the_seed_varies() {
    let (s, _) = three([1, 1, 1], 3, true);
    let mut legs = vec![
        trip(0, 100.0, A, B, &s),
        trip(1, 200.0, A, C, &s),
        trip(2, 1000.0, B, A, &s),
        trip(3, 1900.0, C, A, &s),
        trip(4, 950.0, A, B, &s),
        trip(5, 4000.0, A, B, &s),
    ];
    legs[4].weight = 3;
    let world: Arc<dyn ServiceWorld> = Arc::new(grid_world(std::slice::from_ref(&s), &legs, None));
    let setups = Arc::new(vec![s.clone()]);
    let at = |day_legs: Vec<Leg>| {
        let day = FleetDay { window: (0.0, 3600.0), legs: day_legs, seed: 1 };
        let engine =
            FleetEngine::in_world(Arc::clone(&setups), Arc::new(day), Arc::clone(&world), None);
        let e = engine.view().queries.expected_legs(0, 900.0).expect("a query");
        (*e).clone()
    };
    let e = at(legs.clone());
    assert_eq!((e.rows, e.bins, e.by_zone), (3, 4, false));
    // By hand: A asks 2 people in the first quarter hour and 3 in the second; B one in the
    // second; C one in the third; the leg after the window is not counted.
    assert_eq!(e.values, [2.0, 3.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
    // A day the seed thinned and shifted: the expectation is still the world's legs.
    let varied = vary_legs(&legs, 7, 0.5, 300.0, 3600.0);
    assert_ne!(varied, legs, "the seed varied the day");
    assert_eq!(at(varied), e, "the base legs, not the day's");
    assert!(at(Vec::new()).values.iter().sum::<f64>() > 0.0);
    let day = FleetDay { window: (0.0, 3600.0), legs: legs.clone(), seed: 1 };
    let engine = FleetEngine::in_world(Arc::clone(&setups), Arc::new(day), world, None);
    assert!(engine.view().queries.expected_legs(0, 0.0).is_err());
    assert!(engine.view().queries.expected_legs(1, 900.0).is_err());
}

#[test]
fn nearest_docks_by_free_flow_time_ties_by_index() {
    // Docks "x" and "y" both at node 1, "w" at node 3, "v" at node 24; from node 2 the two at
    // node 1 tie, broken by index.
    let (road, layers) = grid();
    let row = |id: &str, node: usize| DockRow {
        dock_id: id.to_string(),
        name: None,
        hub_id: None,
        position: at(&road, node),
        capacity: 3,
        bikes: Some(1),
    };
    let rows = vec![row("y", 1), row("v", 24), row("x", 1), row("w", 3)];
    let s = ServiceSetup::bike_docked(
        "bike_share",
        &rows,
        layers.walk.as_ref().expect("walk"),
        layers.bike.as_ref().expect("bike"),
        Some(&road),
        DockDefaults::SHIPPED,
    )
    .expect("set up");
    let world = Arc::new(grid_world(std::slice::from_ref(&s), &[], None));
    let day = FleetDay { window: (0.0, 3600.0), legs: Vec::new(), seed: 1 };
    let engine = FleetEngine::in_world(Arc::new(vec![s.clone()]), Arc::new(day), world, None);
    let view = engine.view();
    let ServiceParts::BikeDocked(docks, _) = s.parts() else { panic!("docks") };
    let index =
        |id: &str| u32::try_from(docks.external.iter().position(|d| d == id).unwrap()).unwrap();
    let from = NodeId::from_index(2);
    let n = view.queries.nearest(0, NearestKind::Docks, from, 4).expect("a query");
    let tt = view.queries.travel_times(&[from], &docks.road_node, f64::INFINITY).expect("q");
    let mut by_hand: Vec<(u32, f64)> = (0_u32..).zip(tt.iter().copied()).collect();
    by_hand.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
    assert_eq!(n, by_hand, "at free flow: the travel times, ordered");
    let order: Vec<u32> = n.iter().map(|p| p.0).collect();
    let (kx, ky) = (
        order.iter().position(|&i| i == index("x")).unwrap(),
        order.iter().position(|&i| i == index("y")).unwrap(),
    );
    assert_eq!(n[kx].1, n[ky].1, "a tie");
    assert_eq!(kx.abs_diff(ky), 1);
    assert_eq!((kx < ky), (index("x") < index("y")), "ties by index");
    assert_eq!(order[3], index("v"), "the far corner last");
    assert_eq!(view.queries.nearest(0, NearestKind::Docks, from, 1).expect("q").len(), 1);
    assert!(view.queries.nearest(0, NearestKind::Vehicles, from, 1).is_err(), "no vehicles");
}

#[test]
fn zones_are_a_grid_over_the_nodes_stable_across_rebuilds() {
    let (road, _) = grid();
    let z = Zones::grid(&road, 300.0).expect("a grid").expect("zones");
    // Nodes 0, 200, …, 800 m along each axis fall in cells 0, 0, 1, 2, 2: 3 × 3 zones.
    assert_eq!(z.len(), 9);
    assert_eq!(z, Zones::grid(&grid().0, 300.0).expect("a grid").expect("zones"), "stable");
    let mut seen = [false; 9];
    for i in 0..road.node_count() as usize {
        let k = z.of_node(NodeId::from_index(i)).expect("a zone") as usize;
        seen[k] = true;
        assert_eq!(
            z.of_point(road.node_lonlat(NodeId::from_index(i))),
            Some(u32::try_from(k).unwrap())
        );
    }
    assert!(seen.iter().all(|&s| s), "every zone holds a node");
    assert_eq!(z.of_node(NodeId::from_index(0)), Some(0), "the south-west corner first");
    assert!(Zones::grid(&road, 0.0).is_err());
    let (s, _) = three([1, 1, 1], 3, true);
    let one = grid_world(std::slice::from_ref(&s), &[], None);
    assert_eq!(one.zones().expect("zones").len(), 1, "1 000 m over an 800 m grid: one zone");
    let fine = grid_world(&[s], &[], None).with_zone_grid_m(300.0).expect("a grid");
    assert_eq!(fine.zones(), Some(&z));
}
