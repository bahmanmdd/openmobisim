//! The per-link, per-time-bin results (S163), checked against things that are
//! true independently of how they are recorded: the loading's own cumulative
//! curves, free-flow times, and the fixture's hand values.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "test fixtures: a handful of vehicles, counts far below u32::MAX, and a positive storage"
)]

use openmobisim_core_graph::defaults::SignalDefaults;
use openmobisim_core_graph::examples::toy_network;
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_loading::{
    FidelityLevel, LinkBinRecorder, LinkBins, LtmNetwork, Vehicle, load_level_0_binned,
    load_level_0_recorded, run_ltm, run_ltm_binned, run_ltm_recorded,
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

// --- the tables an iterated run costs its routes from (S170) ---------------------------------

#[test]
fn the_entry_table_files_each_traversal_under_the_bin_it_entered_in() {
    let mut r = LinkBinRecorder::new(4, 60, 3600.0).with_entry_bins();
    let (a, b) = (LinkId::new(0), LinkId::new(1));
    // Entered in bin 0, left in bin 1: 20 s. Entered and left in bin 1: 10 s.
    r.record(a, 50.0, 70.0, 1.0);
    r.record(b, 80.0, 90.0, 2.0);
    // Entered in bin 1 and left in bin 2, on link a again: 50 s.
    r.record(a, 100.0, 150.0, 1.0);
    let (exit, tables) = r.finish_with_entry();
    let tables = tables.expect("asked for");
    // By exit: a's traversals are in bin 1 (the first) and bin 2 (the third).
    assert_eq!((exit.bins(), exit.links()), (&[1, 1, 2][..], &[0, 1, 0][..]));
    // By entry: bin 0 has a's first; bin 1 has b's and a's third.
    let entry = &tables.entry;
    assert_eq!((entry.bins(), entry.links()), (&[0, 1, 1][..], &[0, 0, 1][..]));
    assert_eq!(entry.crossings(), [1, 1, 1]);
    assert_eq!(entry.pcu(), [1.0, 1.0, 2.0]);
    assert_eq!(entry.pcu_seconds(), [20.0, 50.0, 20.0]);
    assert!((entry.mean_seconds(2) - 10.0).abs() < 1e-12);
    assert!(tables.origin_wait.is_empty(), "nobody set out from an origin");
    // Without asking, there is none.
    assert!(LinkBinRecorder::new(4, 60, 3600.0).finish_with_entry().1.is_none());
}

#[test]
fn the_entry_table_holds_the_same_traversals_as_the_exit_table() {
    let toy = Toy::new();
    let demand = mixed_demand(&toy);
    let (_, exit, tables) = run_ltm_recorded(
        &toy.net,
        &toy.turns,
        &demand,
        Duration(3600.0),
        Duration(300.0),
        FidelityLevel::Full,
        60,
    );
    // Every vehicle finishes inside the window, so the two tables count the same vehicles and the
    // same time on every link; only the bins differ.
    for l in 0..toy.net.link_count() {
        let link = LinkId::new(l);
        let entry_pcu = pcu_of(&tables.entry, link);
        assert!((entry_pcu - pcu_of(&exit, link)).abs() < 1e-9, "link {l}");
    }
    let time = |t: &LinkBins| t.pcu_seconds().iter().sum::<f64>();
    assert!((time(&tables.entry) - time(&exit)).abs() < 1e-6);
    // And the tables record no more than the plain run does: same exit table.
    let (_, plain) = run_ltm_binned(
        &toy.net,
        &toy.turns,
        &demand,
        Duration(3600.0),
        Duration(300.0),
        FidelityLevel::Full,
        60,
    );
    assert_eq!(plain, exit);
}

#[test]
fn waiting_at_an_origin_is_recorded_by_departure_and_grows_with_the_queue() {
    let toy = Toy::new();
    let mean_wait = |n: u32| {
        // n cars set out together onto a1: the k-th waits for the k cars ahead of it.
        let cars: Vec<Vehicle> = (0..n).map(|k| toy.car(k, &["a1"], 1.0, 0)).collect();
        let (_, _, tables) = run_ltm_recorded(
            &toy.net,
            &toy.turns,
            &cars,
            Duration(3600.0),
            Duration(300.0),
            FidelityLevel::Full,
            60,
        );
        let waits = tables.origin_wait;
        assert_eq!(waits.len(), 1, "all set out in bin 0, from one link");
        assert_eq!((waits.bins()[0], waits.links()[0]), (0, toy.id("a1").raw()));
        assert_eq!(waits.crossings()[0], n);
        waits.mean_seconds(0)
    };
    // Cars go straight onto the link while it has room, so nobody waits until its storage is full;
    // after that each car waits for the room the discharge frees, one headway apart.
    let storage = toy.net.storage(toy.id("a1")).get();
    let room = storage.floor() as u32;
    assert!(mean_wait(1).abs() < 1e-9 && mean_wait(room - 1).abs() < 1e-9, "no queue, no wait");
    let (some, more) = (mean_wait(room + 30), mean_wait(room + 60));
    assert!(some > 0.0, "the {}th car and after wait", room + 1);
    assert!(more > some, "a longer queue waits longer: {some} then {more}");
}

#[test]
fn vehicles_still_waiting_when_the_window_ends_are_counted_with_the_wait_so_far() {
    let toy = Toy::new();
    let n = toy.net.storage(toy.id("a1")).get().floor() as u32 + 60;
    let cars: Vec<Vehicle> = (0..n).map(|k| toy.car(k, &["a1"], 1.0, 0)).collect();
    let (_, _, tables) = run_ltm_recorded(
        &toy.net,
        &toy.turns,
        &cars,
        Duration(20.0),
        Duration(10.0),
        FidelityLevel::Full,
        60,
    );
    let waits = tables.origin_wait;
    // Every car is in the table once: those that got onto the link with the wait they had, the
    // rest with the 20 s they had waited when the window ended.
    assert_eq!(waits.crossings().iter().sum::<u32>(), n);
    assert!(waits.mean_seconds(0) <= 20.0 + 1e-9 && waits.mean_seconds(0) > 1.0);
    // The link entries too: those still on the link at the end have the time so far.
    assert!(tables.entry.crossings().iter().sum::<u32>() >= 1);
}

#[test]
fn level_zero_has_no_origin_wait_and_the_same_entry_times_as_its_exit_times() {
    let toy = Toy::new();
    let demand = mixed_demand(&toy);
    let (_, exit, tables) = load_level_0_recorded(&demand, &toy.net, 3600.0, 60);
    assert!(tables.origin_wait.is_empty());
    for l in 0..toy.net.link_count() {
        assert!(
            (pcu_of(&tables.entry, LinkId::new(l)) - pcu_of(&exit, LinkId::new(l))).abs() < 1e-9
        );
    }
    // Free flow: every traversal of a link takes the same time, so its mean is the same in any bin.
    for r in 0..tables.entry.len() {
        let l = LinkId::new(tables.entry.links()[r]);
        let free = toy.net.free_flow_time(l).get();
        assert!((tables.entry.mean_seconds(r) - free).abs() < 1e-6);
    }
}
