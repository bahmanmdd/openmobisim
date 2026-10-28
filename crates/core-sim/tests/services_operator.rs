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
//! decisions, and a copy continuing as the original; a policy deciding nothing changes nothing.

#![allow(clippy::float_cmp, reason = "times here are exact sums of the networks' own times")]

use std::sync::{Arc, Mutex};

use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
use openmobisim_core_graph::geometry::{LonLat, ground_distance_metres};
use openmobisim_core_graph::layers::{BikeCost, StaticLayer, StaticLayerDefaults, StaticNetwork};
use openmobisim_core_graph::manhattan_grid;
use openmobisim_core_graph::network::{LinkSpec, RoadNetwork, RoadNetworkBuilder};
use openmobisim_core_sim::services::decisions::{Decision, DecisionKind};
use openmobisim_core_sim::services::engine::{
    FleetDay, FleetEngine, Leg, NoLegs, ServiceLegs, ServiceRoads, ServiceState, VehicleState,
};
use openmobisim_core_sim::services::outcome::{FleetOutcome, LegStatus};
use openmobisim_core_sim::services::policy::{NoOp, Policy, PolicyDescription};
use openmobisim_core_sim::services::view::View;
use openmobisim_core_sim::services::{
    ChargerRow, DockDefaults, DockRow, ServiceParts, ServiceSetup, TaxiDefaults, VehicleRow,
};
use openmobisim_core_sim::{LayerSetup, StaticLayers};
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::{EntityId, NodeId};

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
) -> FleetEngine<'static> {
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
    assert_eq!(why[0].as_deref(), Some("no bike at the first dock"));
    assert_eq!(why[1].as_deref(), Some("the two docks are one"));
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
            Some("level 4 is above dock 0's 3 slots"),
            Some("no dock 9"),
            None,
            Some("a second target for dock 0 at this step"),
            Some("no service 3"),
            Some("service 0 is not a taxi fleet"),
            Some("service 0 is not a taxi fleet"),
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
    let setup = taxi_fleet(&net, &[("v", 0, 0.8)], &[]);
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
    assert_eq!(out.legs[0].status, LegStatus::Served);
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
            Some("vehicle 0 is not idle (relocating)"),
            Some("vehicle 0 is not idle (relocating)"),
            Some("no vehicle 9"),
            Some("vehicle 1 is already at node 4"),
            Some("vehicle 1 has not the charge for the drive and the reserve"),
            Some("to_soc 0.1 is not above vehicle 1's charge 0.16 and at most 1"),
            Some("no charger 5"),
            Some("service 0 is not a docked service"),
            Some("service 0 is not a docked service"),
        ]
    );
}
