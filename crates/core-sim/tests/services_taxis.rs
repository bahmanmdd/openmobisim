//! Electric taxis on the fleet engine: on a line of ten nodes a kilometre apart, driven at 60 s
//! a kilometre, every time, distance and charge has a closed form. What is defended: the times
//! of a request served at once and of one that waits; a request that waits too long fails; a
//! vehicle without charge enough is not sent; the default charging rule and a one-plug queue;
//! vehicles conserved at every bin; a leg of two people; the same inputs give the same day, and
//! a copy taken in the middle ends as the original; options and rows checked by name.

use std::sync::Arc;

use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::network::{LinkSpec, RoadNetwork, RoadNetworkBuilder};
use openmobisim_core_sim::services::engine::{
    FleetDay, FleetEngine, Leg, NoLegs, ServiceRoads, ServiceState, VehicleState,
};
use openmobisim_core_sim::services::outcome::{FleetOutcome, LegStatus, TaxiDay};
use openmobisim_core_sim::services::{
    ChargerRow, Options, ServiceError, ServiceSetup, TaxiDefaults, VehicleRow,
};
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::{EntityId, NodeId};

/// Ten nodes on a parallel, a kilometre apart (0.0128 degrees of longitude at latitude 45.7),
/// joined both ways.
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

fn lonlat(i: u32) -> LonLat {
    LonLat::new(4.8 + 0.0128 * f64::from(i), 45.7)
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

fn vehicle(id: &str, at: u32, soc: Option<f64>) -> VehicleRow {
    VehicleRow { vehicle_id: id.to_string(), position: lonlat(at), soc }
}

fn charger(id: &str, at: u32, plugs: u32) -> ChargerRow {
    ChargerRow { charger_id: id.to_string(), hub_id: None, position: lonlat(at), plugs, kw: None }
}

fn fleet(
    net: &RoadNetwork,
    vehicles: &[VehicleRow],
    chargers: &[ChargerRow],
    d: TaxiDefaults,
) -> ServiceSetup {
    ServiceSetup::taxi("taxi", net, vehicles, chargers, d).expect("set up")
}

/// A taxi leg of `weight` people asking at `at_s` to go from node `from` to node `to`.
fn leg(line: &Line, trip: u32, at_s: f64, from: usize, to: usize, weight: u32) -> Leg {
    Leg {
        service: 0,
        trip,
        weight,
        request_s: at_s,
        origin: line.nodes[from],
        destination: line.nodes[to],
        pickup_dock: u32::MAX,
        return_dock: u32::MAX,
        ride_s: f64::NAN,
        egress_s: f64::NAN,
    }
}

/// Legs of one person each, trip ids in the order given.
fn legs(line: &Line, requests: &[(f64, usize, usize)]) -> Vec<Leg> {
    (0_u32..).zip(requests).map(|(k, &(t, from, to))| leg(line, k, t, from, to, 1)).collect()
}

fn engine<'w>(setup: &ServiceSetup, legs: Vec<Leg>, window: f64, roads: Line) -> FleetEngine<'w> {
    let day = FleetDay { window: (0.0, window), legs, step_s: None, seed: 1 };
    FleetEngine::new(
        Arc::new(vec![setup.clone()]),
        Arc::new(day),
        Arc::new(NoLegs),
        Arc::new(roads),
    )
}

fn load(setup: &ServiceSetup, legs: Vec<Leg>, window: f64, roads: Line) -> FleetOutcome {
    let out = engine(setup, legs, window, roads).run_day();
    assert_eq!(out.invariant_violations, 0);
    out
}

fn taxis(out: &FleetOutcome) -> &TaxiDay {
    out.taxis[0].as_ref().expect("a taxi fleet")
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn a_request_is_served_at_once_with_closed_form_times_distances_and_charge() {
    let net = line();
    let roads = Line::new(&net);
    let setup = fleet(&net, &[vehicle("v", 0, None)], &[], TaxiDefaults::SHIPPED);
    let out = load(&setup, legs(&roads, &[(100.0, 2, 5)]), 3600.0, roads);
    let o = taxis(&out).requests[0];
    assert_eq!(o.vehicle, Some(0));
    // Sent at 100 s; 2 km to the pickup (120 s); boards for 60 s; 3 km to the drop-off (180 s).
    assert!(close(o.assigned_s, 100.0) && close(o.picked_s, 220.0) && close(o.arrival_s, 460.0));
    assert!(close(o.empty_km, 2.0) && close(o.trip_km, 3.0));
    let v = taxis(&out).vehicles[0];
    assert_eq!(v.served, 1);
    assert!(close(v.kwh_used, 5.0 * 0.18) && close(v.soc_end, 0.8 - 0.9 / 60.0));
    assert!(close(v.km_empty, 2.0) && close(v.km_occupied, 3.0));
    let l = out.legs[0];
    assert_eq!((l.status, l.vehicle), (LegStatus::Served, Some(0)));
    assert!(close(l.requested_s, 100.0) && close(l.assigned_s, 100.0));
    assert!(close(l.picked_s, 220.0) && close(l.arrived_s, 460.0) && close(l.waited_s, 120.0));
}

#[test]
fn a_second_request_waits_for_the_vehicle_and_one_that_waits_too_long_fails() {
    let net = line();
    let roads = Line::new(&net);
    let setup = fleet(&net, &[vehicle("v", 0, None)], &[], TaxiDefaults::SHIPPED);
    let both = [(100.0, 2, 5), (100.0, 2, 4)];
    let out = load(&setup, legs(&roads, &both), 3600.0, roads);
    // Freed at n5 at 460 s, 3 km (180 s) from the second's pickup: within its 600 s wait.
    let o = taxis(&out).requests[1];
    assert!(close(o.assigned_s, 460.0) && close(o.picked_s, 640.0) && close(o.arrival_s, 820.0));
    assert_eq!(out.legs[1].status, LegStatus::Served);
    assert!(close(out.legs[1].waited_s, 540.0));
    let d =
        TaxiDefaults::from_options(&Options::from([("max_wait_s".to_string(), 300.0)])).unwrap();
    let setup = fleet(&net, &[vehicle("v", 0, None)], &[], d);
    let roads = Line::new(&net);
    let out = load(&setup, legs(&roads, &both), 3600.0, roads);
    // Now it would reach the pickup at 640 s, past 100 + 300: the trip is not made.
    let o = taxis(&out).requests[1];
    assert_eq!(o.vehicle, None);
    assert!(o.failed && o.arrival_s.is_nan());
    let l = out.legs[1];
    assert_eq!((l.status, l.vehicle), (LegStatus::Failed, None));
    assert!(l.arrived_s.is_nan() && close(l.waited_s, 300.0));
}

#[test]
fn a_vehicle_without_charge_enough_is_not_sent() {
    let net = line();
    let roads = Line::new(&net);
    let d =
        TaxiDefaults::from_options(&Options::from([("charge_below".to_string(), 0.155)])).unwrap();
    // 5 km take 0.015 of the battery: from 0.16 the vehicle would end below the 0.15 reserve.
    let setup = fleet(&net, &[vehicle("v", 0, Some(0.16))], &[], d);
    let out = load(&setup, legs(&roads, &[(100.0, 2, 5)]), 3600.0, roads);
    assert_eq!(taxis(&out).requests[0].vehicle, None);
    assert_eq!(out.legs[0].status, LegStatus::Failed);
    assert!(close(taxis(&out).vehicles[0].soc_end, 0.16));
}

#[test]
fn below_the_threshold_a_vehicle_charges_and_a_second_waits_for_the_one_plug() {
    let net = line();
    let roads = Line::new(&net);
    let vehicles = [vehicle("a", 0, Some(0.21)), vehicle("b", 1, Some(0.21))];
    let setup = fleet(&net, &vehicles, &[charger("c", 9, 1)], TaxiDefaults::SHIPPED);
    // Each takes a passenger to n5 and falls below 0.2: both drive to the one-plug charger.
    let out = load(&setup, legs(&roads, &[(100.0, 1, 5), (100.0, 2, 5)]), 86_400.0, roads);
    let day = taxis(&out);
    for v in &day.vehicles {
        assert_eq!(v.served, 1);
        assert!(close(v.soc_end, 0.8), "charged to the target: {}", v.soc_end);
        assert!(v.kwh_charged > 0.0);
    }
    // b, at n1, takes the first: 0 km to n1, 4 km to n5 (dropped at 100 + 60 + 240 = 400 s),
    // 4 km to n9 at 640 s. a takes the second: 2 km to n2, 3 km to n5 (dropped at 100 + 120 +
    // 60 + 180 = 460 s), 4 km to n9 at 700 s: b charges first, a waits for the plug, so a's
    // time at the charger is the longer.
    assert!(day.vehicles[0].charging_s > day.vehicles[1].charging_s);
    let charge_b = (0.8 - (0.21 - 8.0 * 0.18 / 60.0)) * 60.0 / 50.0 * 3600.0;
    assert!(close(day.vehicles[1].charging_s, charge_b), "{}", day.vehicles[1].charging_s);
    // The fleet is whole in every bin; someone queued in one of them.
    assert_eq!(day.bins.states.len(), 96, "a day of 900 s bins");
    assert!(day.bins.states.iter().all(|s| s.iter().sum::<u32>() == 2));
    assert!(day.bins.states.iter().any(|s| s[VehicleState::QueuedAtCharger as usize] == 1));
}

#[test]
fn a_leg_of_two_is_two_unit_requests_and_arrives_with_the_last() {
    let net = line();
    let roads = Line::new(&net);
    let setup = fleet(&net, &[vehicle("v", 0, None)], &[], TaxiDefaults::SHIPPED);
    let out = load(&setup, vec![leg(&roads, 0, 100.0, 2, 5, 2)], 3600.0, roads);
    let day = taxis(&out);
    assert_eq!(day.requests.len(), 2);
    // The first as above (dropped at 460 s); the second waits, sent from n5 at 460 s: picked
    // at 640 s, dropped at 640 + 60 + 180 = 880 s.
    assert!(close(day.requests[0].arrival_s, 460.0));
    assert!(close(day.requests[1].assigned_s, 460.0) && close(day.requests[1].arrival_s, 880.0));
    let l = out.legs[0];
    assert_eq!((l.status, l.vehicle), (LegStatus::Served, Some(0)));
    assert!(close(l.assigned_s, 460.0) && close(l.arrived_s, 880.0) && close(l.waited_s, 540.0));
    assert_eq!(day.vehicles[0].served, 2);
    // Ended before the second's drop-off: under way, truncated.
    let roads = Line::new(&net);
    let out = load(&setup, vec![leg(&roads, 0, 100.0, 2, 5, 2)], 700.0, roads);
    assert_eq!(out.legs[0].status, LegStatus::Truncated);
}

#[test]
fn the_same_inputs_give_the_same_day() {
    let net = line();
    let vehicles = [vehicle("a", 0, None), vehicle("b", 9, Some(0.3)), vehicle("c", 4, None)];
    let setup = fleet(&net, &vehicles, &[charger("c1", 5, 1)], TaxiDefaults::SHIPPED);
    let requests: Vec<(f64, usize, usize)> = (0..30_u32)
        .map(|k| (f64::from(k) * 37.0, (k * 7 % 10) as usize, (k * 3 % 10) as usize))
        .collect();
    let a = load(&setup, legs(&Line::new(&net), &requests), 7200.0, Line::new(&net));
    let b = load(&setup, legs(&Line::new(&net), &requests), 7200.0, Line::new(&net));
    // Compared as printed (exact to the bit), since a failed request's times are NaN.
    assert_eq!(format!("{a:?}"), format!("{b:?}"));
    let day = taxis(&a);
    assert!(day.requests.iter().any(|o| o.vehicle.is_none()), "some fail: the test covers both");
    assert!(day.requests.iter().any(|o| o.vehicle.is_some()));
}

/// A small deterministic generator (a 64-bit linear congruential one), for the random day.
struct Lcg(u64);

impl Lcg {
    fn below(&mut self, n: u64) -> u64 {
        self.0 =
            self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) % n
    }
}

#[test]
fn a_random_day_conserves_vehicles_and_charge_replays_and_a_copy_ends_as_the_original() {
    let net = line();
    let vehicles = [vehicle("a", 0, Some(0.3)), vehicle("b", 5, Some(0.22)), vehicle("c", 9, None)];
    let setup = fleet(&net, &vehicles, &[charger("c1", 4, 1)], TaxiDefaults::SHIPPED);
    let mut rng = Lcg(7);
    let requests: Vec<(f64, usize, usize)> = (0..100)
        .map(|_| {
            #[allow(clippy::cast_precision_loss, reason = "small")]
            let t = rng.below(14_400) as f64;
            let from = usize::try_from(rng.below(10)).expect("small");
            let to = usize::try_from(rng.below(10)).expect("small");
            (t, from, to)
        })
        .collect();
    let day = |roads: &Line| legs(roads, &requests);
    let window = 18_000.0;
    // Advanced in steps, every charge checked at each.
    let mut e = engine(&setup, day(&Line::new(&net)), window, Line::new(&net));
    let mut copy = None;
    let mut t = 0.0;
    while !e.is_done() {
        t += 600.0;
        e.advance_to(t);
        let ServiceState::Taxi { soc, counts, .. } = &e.state().services[0] else {
            panic!("a taxi fleet")
        };
        assert!(soc.iter().all(|&c| (0.0..=1.0).contains(&c)), "{soc:?}");
        assert_eq!(counts.iter().sum::<u32>(), 3);
        if (t - 9000.0).abs() < 1e-9 {
            copy = Some(e.clone());
        }
    }
    let a = e.into_outcome();
    assert_eq!(a.invariant_violations, 0);
    let td = taxis(&a);
    assert!(td.vehicles.iter().all(|v| v.soc_end >= 0.0));
    assert!(td.bins.states.iter().all(|s| s.iter().sum::<u32>() == 3));
    assert!(td.vehicles.iter().any(|v| v.kwh_charged > 0.0), "someone charged");
    let served = a.legs.iter().filter(|l| l.status == LegStatus::Served).count();
    let failed = a.legs.iter().filter(|l| l.status == LegStatus::Failed).count();
    assert!(served > 0 && failed > 0, "served {served}, failed {failed}");
    let b = load(&setup, day(&Line::new(&net)), window, Line::new(&net));
    // Printed, so the times not yet known (`NaN`) compare equal.
    assert_eq!(format!("{a:?}"), format!("{b:?}"), "the same day twice");
    let c = copy.expect("a copy at 9000 s").run_day();
    assert_eq!(format!("{a:?}"), format!("{c:?}"), "a copy from the middle ends as the original");
}

#[test]
fn options_and_rows_are_checked() {
    let unknown = TaxiDefaults::from_options(&Options::from([("batery_kwh".to_string(), 1.0)]));
    assert!(matches!(unknown, Err(ServiceError::UnknownOption { .. })));
    assert!(unknown.unwrap_err().to_string().contains("battery_kwh"));
    for (name, value) in [("battery_kwh", 0.0), ("charge_to", 1.5), ("charge_below", 0.9)] {
        let bad = TaxiDefaults::from_options(&Options::from([(name.to_string(), value)]));
        assert!(matches!(bad, Err(ServiceError::BadOption { .. })), "{name} = {value}");
    }
    let net = line();
    let twice = [vehicle("v", 0, None), vehicle("v", 1, None)];
    assert!(ServiceSetup::taxi("taxi", &net, &twice, &[], TaxiDefaults::SHIPPED).is_err());
    // A vehicle a degree away is left out and counted; with none left, the fleet is refused.
    let far = VehicleRow { vehicle_id: "far".into(), position: LonLat::new(5.9, 45.7), soc: None };
    let setup = ServiceSetup::taxi(
        "taxi",
        &net,
        &[vehicle("v", 0, None), far.clone()],
        &[],
        TaxiDefaults::SHIPPED,
    )
    .unwrap();
    assert_eq!((setup.report().kept, setup.report().off_layer), (1, 1));
    assert!(ServiceSetup::taxi("taxi", &net, &[far], &[], TaxiDefaults::SHIPPED).is_err());
}
