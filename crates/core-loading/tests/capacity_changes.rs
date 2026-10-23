//! Timed capacity changes (disruptions): a link closed for a while, or narrowed.
//!
//! The network: `A → B → C`, two residential links of 300 m: 36 s at 30 km/h, capacity
//! 1400/h, so fronts leave a link end 1/0.3889 = 2.571 s per PCU apart. Cars of 1 PCU leave
//! `A` for `C` every 10 s from 0 s; free, each takes 72 s.
//!
//! | Case | Hand value |
//! |---|---|
//! | no change | every car 72 s |
//! | `BC` closed from 100 s to 400 s (no car enters or leaves it) | none arrives between 100 and 400 s; the 4 cars on `BC` when it closed leave from **400 s**, one every 2.571 s; those held on `AB` enter at 400 s and arrive from **436 s** (400 + 36), one every 2.571 s |
//! | `BC` at a tenth of its capacity from 100 s | its cars leave 25.71 s apart (the tenth's headway) until the queue clears |

use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::network::{LinkSpec, RoadNetwork, RoadNetworkBuilder};
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_loading::{
    CapacityChange, FidelityLevel, Recording, Rules, Trajectory, Vehicle, run_ltm_chained,
};
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::LinkId;
use openmobisim_core_types::ids::{EntityId, VehicleId};
use openmobisim_core_types::time::Second;
use openmobisim_core_types::units::{Duration, Pcu};

fn line() -> (RoadNetwork, LinkId, LinkId) {
    let mut b = RoadNetworkBuilder::new();
    for (i, n) in ["A", "B", "C"].iter().enumerate() {
        #[allow(clippy::cast_precision_loss, reason = "three nodes")]
        b.add_node(*n, LonLat::new(4.9 + 0.005 * i as f64, 52.37));
    }
    for (id, from, to) in [("AB", "A", "B"), ("BC", "B", "C")] {
        let mut spec = LinkSpec::new(RoadClass::Residential);
        spec.length_m = Some(300.0);
        b.add_link(id, from, to, spec);
    }
    let net = b
        .build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
        .unwrap();
    let link = |id: &str| net.link_external_ids().typed_id_of::<LinkId>(id).unwrap();
    let (ab, bc) = (link("AB"), link("BC"));
    (net, ab, bc)
}

/// Each car's arrival at `C`, by car, under `changes`, in whole seconds (floored).
fn arrivals(changes: Vec<CapacityChange>) -> Vec<f64> {
    let (net, ab, bc) = line();
    let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);
    let cars: Vec<Vehicle> = (0..40)
        .map(|k| Vehicle::new(VehicleId::new(k), vec![ab, bc], Pcu(1.0), Second(10 * k)))
        .collect();
    let out = run_ltm_chained(
        &net,
        &turns,
        &cars,
        &[],
        Duration(3600.0),
        Duration(300.0),
        FidelityLevel::Full,
        Recording::Trajectories,
        Rules { capacity_changes: changes, ..Rules::default() },
        None,
    );
    let mut by_car: Vec<&Trajectory> = out.trajectories.iter().collect();
    by_car.sort_by_key(|t| t.vehicle.raw());
    by_car.iter().map(|t| f64::from(t.arrival().get())).collect()
}

#[test]
fn no_change_changes_nothing() {
    let free = arrivals(Vec::new());
    for (k, &a) in free.iter().enumerate() {
        #[allow(clippy::cast_precision_loss, reason = "a few cars")]
        let expected = 10.0 * k as f64 + 72.0;
        assert!((a - expected).abs() < 1e-6, "car {k}: {a} against {expected}");
    }
}

#[test]
fn a_closed_link_holds_its_cars_until_it_opens_and_then_discharges_at_capacity() {
    let (_, _, bc) = line();
    let closed = arrivals(vec![
        CapacityChange { time: 100.0, link: bc, factor: 0.0 },
        CapacityChange { time: 400.0, link: bc, factor: 1.0 },
    ]);
    assert!(closed.iter().all(|&a| !(100.0 < a && a < 400.0)), "nothing passes while closed");
    let held: Vec<f64> = closed.iter().copied().filter(|&a| a >= 400.0 - 1e-6).collect();
    let headway = 1.0 / (1400.0 / 3600.0);
    let on_bc = held.iter().filter(|&&a| a < 436.0).count();
    assert_eq!(on_bc, 4, "the cars on BC when it closed: {held:?}");
    assert!((held[0] - 400.0).abs() < 1e-6, "they leave when it opens");
    assert!((held[3] - held[0] - 3.0 * headway).abs() <= 1.0, "at capacity");
    assert!((held[4] - 436.0).abs() < 1e-6, "the held ones enter at 400 s, cross in 36 s");
    assert!((held[14] - held[4] - 10.0 * headway).abs() <= 1.0, "then at capacity: {held:?}");
    assert!(closed[..3].iter().zip([72.0, 82.0, 92.0]).all(|(a, e)| (a - e).abs() < 1e-6));
}

#[test]
fn a_narrowed_link_discharges_at_its_share_of_capacity() {
    let (_, _, bc) = line();
    let narrow = arrivals(vec![CapacityChange { time: 100.0, link: bc, factor: 0.1 }]);
    let late: Vec<f64> = narrow.iter().copied().filter(|&a| a > 200.0).collect();
    let headway = 10.0 / (1400.0 / 3600.0);
    for w in late.windows(2).take(5) {
        assert!((w[1] - w[0] - headway).abs() <= 1.0, "{} apart against {headway}", w[1] - w[0]);
    }
}

/// One car leaving `A` for `C` at `depart` seconds, under `changes`: its arrival, whole seconds.
fn one_car(depart: u32, changes: Vec<CapacityChange>) -> f64 {
    one_car_at(depart, changes, FidelityLevel::Full, false).0
}

/// [`one_car`] at `level`, with the inflow headway at nodes kept at the point queue or not: its
/// arrival and when it entered `BC`, whole seconds.
fn one_car_at(
    depart: u32,
    changes: Vec<CapacityChange>,
    level: FidelityLevel,
    node_inflow: bool,
) -> (f64, f64) {
    let (net, ab, bc) = line();
    let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);
    let car = [Vehicle::new(VehicleId::new(0), vec![ab, bc], Pcu(1.0), Second(depart))];
    let out = run_ltm_chained(
        &net,
        &turns,
        &car,
        &[],
        Duration(3600.0),
        Duration(300.0),
        level,
        Recording::Trajectories,
        Rules { capacity_changes: changes, node_inflow, ..Rules::default() },
        None,
    );
    let t = &out.trajectories[0];
    (f64::from(t.arrival().get()), f64::from(t.links[1].enter.get()))
}

#[test]
fn a_closed_link_holds_even_a_first_vehicle() {
    // No vehicle has passed `BC` before it closes, so no headway runs from an earlier one: the
    // closure itself must hold the car, entering and leaving alike.
    let (_, _, bc) = line();
    let closed = |from: f64| {
        vec![
            CapacityChange { time: from, link: bc, factor: 0.0 },
            CapacityChange { time: 400.0, link: bc, factor: 1.0 },
        ]
    };
    assert!((one_car(10, Vec::new()) - 82.0).abs() < 1e-6, "free: 10 + 72 s");
    // Closed before it reaches `B`: it waits there and enters at 400 s, then 36 s on `BC`.
    let held_at_b = one_car(10, closed(0.0));
    assert!((held_at_b - 436.0).abs() < 1e-6, "entering a closed link: {held_at_b}");
    // On `BC` (it entered at 36 s) when `BC` closes at 50 s: it leaves when it opens.
    let held_on_bc = one_car(0, closed(50.0));
    assert!((held_on_bc - 400.0).abs() < 1e-6, "leaving a closed link: {held_on_bc}");
}

/// **Property (S293):** at the point queue a closed link refuses entry under either rule of entry
/// at nodes: the car waits at `B`, at the end of `AB`, enters `BC` when it opens and arrives
/// 36 s later. Under the classic point queue (no inflow headway at nodes) it entered the closed
/// link at 46 s before, crossed it while it was closed and left it the second it opened.
#[test]
fn at_the_point_queue_a_closed_link_refuses_entry_too() {
    let (_, _, bc) = line();
    let closed = vec![
        CapacityChange { time: 0.0, link: bc, factor: 0.0 },
        CapacityChange { time: 400.0, link: bc, factor: 1.0 },
    ];
    for node_inflow in [false, true] {
        let (arrival, entered) =
            one_car_at(10, closed.clone(), FidelityLevel::PointQueue, node_inflow);
        assert!((entered - 400.0).abs() < 1e-6, "node_inflow {node_inflow}: entered at {entered}");
        assert!((arrival - 436.0).abs() < 1e-6, "node_inflow {node_inflow}: arrived at {arrival}");
    }
    // Narrowed, not closed: at the classic point queue the car enters at once and waits at the
    // narrowed link's own exit.
    let narrowed = vec![CapacityChange { time: 0.0, link: bc, factor: 0.5 }];
    let (_, entered) = one_car_at(10, narrowed, FidelityLevel::PointQueue, false);
    assert!((entered - 46.0).abs() < 1e-6, "a narrowed link takes it at 10 + 36 s: {entered}");
}
