//! The per-link, per-time-bin results (S163), checked against things that are
//! true independently of how they are recorded: the loading's own cumulative
//! curves, free-flow times, and the fixture's hand values.

#![allow(
    clippy::cast_possible_truncation,
    reason = "test fixtures: a handful of vehicles, counts far below u32::MAX"
)]

use openmobisim_core_graph::defaults::SignalDefaults;
use openmobisim_core_graph::examples::toy_network;
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_loading::{
    FidelityLevel, LinkBins, LtmNetwork, Vehicle, load_level_0_binned, run_ltm, run_ltm_binned,
};
use openmobisim_core_types::ids::{EntityId, LinkId, VehicleId};
use openmobisim_core_types::time::Second;
use openmobisim_core_types::units::{Duration, Pcu};

struct Toy {
    net: RoadNetwork,
    turns: TurnTable,
}

impl Toy {
    fn new() -> Self {
        let (net, _) = toy_network();
        let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);
        Self { net, turns }
    }
    fn id(&self, name: &str) -> LinkId {
        self.net.link_external_ids().typed_id_of::<LinkId>(name).expect("a toy link")
    }
    fn car(&self, id: u32, route: &[&str], pcu: f64, departure: u32) -> Vehicle {
        let route = route.iter().map(|n| self.id(n)).collect();
        Vehicle::new(VehicleId::new(id), route, Pcu(pcu), Second(departure))
    }
}

/// A demand that exercises signals, a merge, the chain and a diverge.
fn mixed_demand(toy: &Toy) -> Vec<Vehicle> {
    let mut v = Vec::new();
    for k in 0..40 {
        v.push(toy.car(k, &["a1", "a2", "a3", "c1", "c2", "c3", "a4"], 1.0, 4 * k));
        v.push(toy.car(100 + k, &["m1", "a3", "c1", "c2", "c3", "a5", "r1", "e2"], 1.0, 3 * k));
        v.push(toy.car(200 + k, &["s1", "a2", "a3", "c1", "c2", "c3", "a4"], 2.0, 5 * k));
    }
    v
}

fn pcu_of(bins: &LinkBins, link: LinkId) -> f64 {
    (0..bins.len()).filter(|&r| bins.links()[r] == link.raw()).map(|r| bins.pcu()[r]).sum()
}

/// **Property:** each link's crossings and PCU in the table are exactly what
/// the demand's routes say (every vehicle finishes), and on links long enough
/// to hold a whole vehicle they also equal the loading's own curve.
#[test]
fn the_table_agrees_with_the_routes_and_the_curves() {
    let toy = Toy::new();
    let cars = mixed_demand(&toy);
    let mut sim = LtmNetwork::new(&toy.net, &toy.turns).with_link_bins(60, 20_000.0);
    cars.iter().for_each(|v| sim.depart(v));
    let mut finished = 0;
    for _ in 0..40 {
        finished += sim.step(Duration(300.0)).len();
    }
    assert_eq!(finished, cars.len(), "everyone finishes");
    let bins = sim.take_link_bins().expect("asked for");
    for link in LinkId::iter_space(toy.net.link_count()) {
        let using: Vec<&Vehicle> = cars.iter().filter(|v| v.route.contains(&link)).collect();
        let (n, pcu) = (using.len() as u32, using.iter().map(|v| v.pcu.get()).sum::<f64>());
        let rows = || (0..bins.len()).filter(|&r| bins.links()[r] == link.raw());
        assert_eq!(rows().map(|r| bins.crossings()[r]).sum::<u32>(), n, "{link:?}: crossings");
        assert!((pcu_of(&bins, link) - pcu).abs() < 1e-9, "{link:?}: PCU");
        if toy.net.storage(link).get() >= 2.0 {
            let curve = sim.cumulative_out(link).get();
            assert!((pcu - curve).abs() < 1e-6, "{link:?}: routes {pcu} PCU, curve {curve} PCU");
        }
    }
}

/// **Property:** a lone vehicle's mean traversal time on every link is that
/// link's free-flow time, exactly — at every level, and at level 0 too.
#[test]
fn a_lone_vehicle_records_free_flow_times() {
    let toy = Toy::new();
    let names = ["a1", "a2", "a3", "c1", "c2", "c3", "a4"];
    let car = [toy.car(0, &names, 1.0, 0)];
    let mut tables = vec![load_level_0_binned(&car, &toy.net, 3600.0, 60).1];
    for level in [FidelityLevel::PointQueue, FidelityLevel::SpatialQueue, FidelityLevel::Full] {
        tables.push(
            run_ltm_binned(
                &toy.net,
                &toy.turns,
                &car,
                Duration(3600.0),
                Duration(300.0),
                level,
                60,
            )
            .1,
        );
    }
    for (k, bins) in tables.iter().enumerate() {
        assert_eq!(bins.len(), names.len(), "table {k}: one row per link");
        for row in 0..bins.len() {
            let link = LinkId::from_index(bins.links()[row] as usize);
            let expected = toy.net.free_flow_time(link).get();
            assert!(
                (bins.mean_seconds(row) - expected).abs() < 1e-6,
                "table {k}, {link:?}: mean {} s, free flow {expected} s",
                bins.mean_seconds(row)
            );
            assert_eq!(bins.crossings()[row], 1);
        }
    }
}

/// **Property:** traversals that finish inside the window are counted even
/// when their vehicle does not finish its trip — the reason the table is
/// recorded inside the loading rather than built from finished trips.
#[test]
fn vehicles_still_on_their_way_are_counted() {
    let toy = Toy::new();
    let car = [toy.car(0, &["a1", "a2", "a3", "c1", "c2", "c3", "a4"], 1.0, 0)];
    // Free flow to the end of a3 is 56.55 + 24 + 48 = 128.55 s; stop at 100 s.
    let (done, bins) = run_ltm_binned(
        &toy.net,
        &toy.turns,
        &car,
        Duration(100.0),
        Duration(50.0),
        FidelityLevel::Full,
        50,
    );
    assert!(done.is_empty(), "the trip does not finish inside the window");
    let recorded: Vec<u32> = bins.links().to_vec();
    assert!(recorded.contains(&toy.id("a1").raw()) && recorded.contains(&toy.id("a2").raw()));
    assert!(!recorded.contains(&toy.id("a3").raw()), "a3 is not finished by 100 s");
}

/// **Property:** recording changes nothing about the run, and the table does
/// not depend on the loading step (S84: the step is bookkeeping only).
#[test]
fn recording_is_free_and_step_independent() {
    let toy = Toy::new();
    let cars = mixed_demand(&toy);
    let plain = run_ltm(
        &toy.net,
        &toy.turns,
        &cars,
        Duration(20_000.0),
        Duration(300.0),
        FidelityLevel::Full,
    );
    let mut reference: Option<LinkBins> = None;
    for step in [60.0, 300.0, 3600.0] {
        let (done, bins) = run_ltm_binned(
            &toy.net,
            &toy.turns,
            &cars,
            Duration(20_000.0),
            Duration(step),
            FidelityLevel::Full,
            120,
        );
        let key = |t: &openmobisim_core_loading::Trajectory| {
            (
                t.vehicle.raw(),
                t.links.iter().map(|l| (l.enter.get(), l.exit.get())).collect::<Vec<_>>(),
            )
        };
        let (mut a, mut b): (Vec<_>, Vec<_>) =
            (plain.iter().map(key).collect(), done.iter().map(key).collect());
        a.sort();
        b.sort();
        if let Some(i) = (0..a.len().min(b.len())).find(|&i| a[i] != b[i]) {
            panic!("step {step}: vehicle {} differs: {:?} against {:?}", a[i].0, a[i].1, b[i].1);
        }
        assert_eq!(a.len(), b.len(), "step {step}: a different number finished");
        match &reference {
            None => reference = Some(bins),
            Some(r) => assert_eq!(r, &bins, "step {step}: the table depends on the step"),
        }
    }
}

/// **Property:** a level-0 table does not depend on the order the vehicles are given.
#[test]
fn a_level_0_table_does_not_depend_on_input_order() {
    let toy = Toy::new();
    let cars = mixed_demand(&toy);
    let reversed: Vec<Vehicle> = cars.iter().rev().cloned().collect();
    let a = load_level_0_binned(&cars, &toy.net, 3600.0, 30).1;
    let b = load_level_0_binned(&reversed, &toy.net, 3600.0, 30).1;
    assert_eq!(a, b);
    assert!(!a.is_empty());
}
