//! The toy network's road part, checked against values derived by hand (I-m,
//! S161).
//!
//! Each case loads a small, fixed demand onto `toy_network()` and compares the
//! result with what the loading's own rules imply, worked out on paper from the
//! network's parameters — never read back from the code. **Expected values are
//! computed from the network's parameters, not written as literals**, so a
//! change to the defaults table moves the numbers and not the tests.
//!
//! Recorded times are floored to whole seconds, so every time is compared to
//! within one second. Steady-state rates are compared to within 3 %.
//!
//! Cases: `t1` free flow and the sub-vehicle chain; `t2` signal delay and
//! discharge; `t3` storage, spillback and the fidelity levels; `t4` a
//! saturated merge; `t5` a queue crossing the chain; `t6` roundabout give-way;
//! `t7` a jammed branch throttling a diverge; `t8` a vehicle of two PCU; `t9` a
//! signalised junction with two approaches; `t0` properties every case must
//! have (identical on repeat, under reversed input, and for any loading step).

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "test fixtures: vehicle counts and times are small and non-negative"
)]

use openmobisim_core_graph::defaults::SignalDefaults;
use openmobisim_core_graph::examples::toy_network;
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_loading::{FidelityLevel, LtmNetwork, Trajectory, Vehicle, load_level_0};
use openmobisim_core_types::ids::{EntityId, LinkId, VehicleId};
use openmobisim_core_types::time::Second;
use openmobisim_core_types::units::{Duration, Pcu};

/// One trip as compared across runs: vehicle, departure, and per link its id, entry and exit.
type TripShape = (u32, u32, Vec<(u32, u32, u32)>);

/// Whole-second recording: a recorded time is the floor of the exact one.
const TOL: f64 = 1.0;

struct Toy {
    net: RoadNetwork,
    turns: TurnTable,
}

impl Toy {
    fn new() -> Self {
        let (net, diagnostics) = toy_network();
        assert!(diagnostics.is_empty(), "the toy network builds without diagnostics");
        let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);
        Self { net, turns }
    }

    fn id(&self, name: &str) -> LinkId {
        self.net.link_external_ids().typed_id_of::<LinkId>(name).expect("a toy link")
    }

    fn route(&self, names: &[&str]) -> Vec<LinkId> {
        names.iter().map(|n| self.id(n)).collect()
    }

    /// Free-flow time of `link` in seconds, including any control delay.
    fn ff(&self, name: &str) -> f64 {
        self.net.free_flow_time(self.id(name)).get()
    }

    /// Free-flow time of `link` without its control delay (`L / v`).
    fn travel(&self, name: &str) -> f64 {
        let id = self.id(name);
        self.net.free_flow_time(id).get() - self.net.link_parameters(id).control_delay.get()
    }

    /// The saturation headway `1 / capacity` of a link, in seconds.
    fn headway(&self, name: &str) -> f64 {
        1.0 / self.net.link_parameters(self.id(name)).capacity.get()
    }

    /// The saturation headway of the signalised approach `from` into `to`:
    /// `1 / (capacity · g/C)`.
    fn signal_headway(&self, from: &str, to: &str) -> f64 {
        let (f, t) = (self.id(from), self.id(to));
        let green = f64::from(
            self.turns.capacity_fraction(self.turns.find(f, t).expect("a through movement")),
        );
        1.0 / (self.net.link_parameters(f).capacity.get() * green)
    }

    fn delay(&self, name: &str) -> f64 {
        self.net.link_parameters(self.id(name)).control_delay.get()
    }

    fn storage(&self, name: &str) -> f64 {
        self.net.storage(self.id(name)).get()
    }

    /// The backward-wave travel time of a link, in seconds.
    fn wave_time(&self, name: &str) -> f64 {
        let id = self.id(name);
        self.net.link_length(id).get() / self.net.link_parameters(id).wave_speed.get()
    }

    fn vehicle(&self, id: u32, route: &[&str], pcu: f64, departure: u32) -> Vehicle {
        Vehicle::new(VehicleId::new(id), self.route(route), Pcu(pcu), Second(departure))
    }

    /// Load `vehicles` in steps of `step` seconds up to `horizon`; at every step
    /// no link may count more than its storage (levels 3 and 4). Returns the
    /// finished trajectories in vehicle order.
    fn run(
        &self,
        vehicles: &[Vehicle],
        level: FidelityLevel,
        step: f64,
        horizon: f64,
    ) -> Vec<Trajectory> {
        let mut sim = LtmNetwork::new(&self.net, &self.turns).with_level(level);
        vehicles.iter().for_each(|v| sim.depart(v));
        let mut done = Vec::new();
        let mut t = 0.0;
        while t < horizon {
            done.extend(sim.step(Duration(step)));
            t += step;
            if level != FidelityLevel::PointQueue {
                for link in LinkId::iter_space(self.net.link_count()) {
                    let counted = sim.counted_pcu(link).get();
                    let storage = self.net.storage(link).get();
                    assert!(
                        counted <= storage + 1e-6,
                        "{link:?} counts {counted} PCU of {storage}"
                    );
                }
            }
        }
        done.sort_by_key(|t| t.vehicle.raw());
        done
    }

    /// As [`Self::run`], full fidelity, 300-s steps, and every vehicle must finish.
    fn run_full(&self, vehicles: &[Vehicle], horizon: f64) -> Vec<Trajectory> {
        let done = self.run(vehicles, FidelityLevel::Full, 300.0, horizon);
        assert_eq!(done.len(), vehicles.len(), "every vehicle finishes within {horizon} s");
        done
    }
}

fn close(actual: u32, expected: f64, what: &str) {
    assert!(
        (f64::from(actual) - expected).abs() <= TOL,
        "{what}: recorded {actual}, expected {expected:.2}"
    );
}

fn enter(t: &Trajectory, leg: usize) -> u32 {
    t.links[leg].enter.get()
}

fn exit(t: &Trajectory, leg: usize) -> u32 {
    t.links[leg].exit.get()
}

/// **T1 · Free flow, and the sub-vehicle chain.** A lone car takes the sum of
/// the free-flow times, control delay included, at every fidelity level — and
/// level 0 must agree: it may not lose the sub-second remainders of the three
/// 4-m pieces (F1).
#[test]
fn t1_a_lone_car_takes_free_flow_time_at_every_level() {
    let toy = Toy::new();
    let names = ["a1", "a2", "a3", "c1", "c2", "c3", "a4"];
    let car = [toy.vehicle(0, &names, 1.0, 0)];
    let mut cumulative = 0.0;
    let expected: Vec<f64> = names
        .iter()
        .map(|n| {
            cumulative += toy.ff(n);
            cumulative
        })
        .collect();

    let mut runs = vec![("level 0", load_level_0(&car, &toy.net))];
    for (label, level) in [
        ("level 2", FidelityLevel::PointQueue),
        ("level 3", FidelityLevel::SpatialQueue),
        ("level 4", FidelityLevel::Full),
    ] {
        runs.push((label, toy.run(&car, level, 300.0, 600.0)));
    }
    for (label, done) in &runs {
        assert_eq!(done.len(), 1, "{label}: the car finishes");
        for (leg, name) in names.iter().enumerate() {
            close(exit(&done[0], leg), expected[leg], &format!("{label}, exit of {name}"));
        }
    }
}

/// **T2 · Signal: control delay and discharge at `capacity · g/C`.** Cars
/// arrive faster than the signal discharges, so a queue forms (20 cars, well
/// inside the approach's storage: no spillback).
#[test]
fn t2_a_signal_queue_discharges_at_green_time_capacity() {
    let toy = Toy::new();
    let cars: Vec<Vehicle> = (0..20).map(|k| toy.vehicle(k, &["a1", "a2"], 1.0, 3 * k)).collect();
    let hs = toy.signal_headway("a1", "a2");
    assert!(hs > 3.0, "the fixture needs arrivals faster than the signal discharges");
    let first_pass = toy.travel("a1");
    let done = toy.run_full(&cars, 900.0);
    for (k, t) in done.iter().enumerate() {
        let exit_a1 = first_pass + hs * k as f64 + toy.delay("a1");
        close(exit(t, 0), exit_a1, &format!("car {k}, exit of a1"));
        close(exit(t, 1), exit_a1 + toy.ff("a2"), &format!("car {k}, arrival"));
    }
}

/// **T3 · Storage, spillback and the fidelity levels.** Cars depart faster
/// than a signal serves them; the approach fills. A full link admits what left
/// one wave-time earlier (level 4), at once (level 3), or never limits
/// (level 2).
#[test]
fn t3_a_full_approach_admits_what_left_one_wave_time_earlier() {
    let toy = Toy::new();
    let n = 60u32;
    let cars: Vec<Vehicle> = (0..n).map(|k| toy.vehicle(k, &["a1", "a2"], 1.0, 2 * k)).collect();
    let (hs, first_pass, lag) =
        (toy.signal_headway("a1", "a2"), toy.travel("a1"), toy.wave_time("a1"));
    let storage = toy.storage("a1");
    assert!(
        (storage - storage.round()).abs() < 1e-9,
        "the fixture's storage is a whole number of cars"
    );
    let storage = storage.round() as u32;
    // Car j passes the stop line at `first_pass + hs·j` while the queue stands.
    let pass = |j: u32| first_pass + hs * f64::from(j);
    // Car k needs `k − storage + 1` cars to have passed before it can enter.
    let entry = |k: u32, lag: f64| -> f64 {
        let departure = f64::from(2 * k);
        match (k + 1).checked_sub(storage).filter(|&needed| needed > 0) {
            Some(needed) => departure.max(pass(needed - 1) + lag),
            None => departure,
        }
    };
    let mut entries = Vec::new();
    for (label, level, lag) in [
        ("level 2", FidelityLevel::PointQueue, None),
        ("level 3", FidelityLevel::SpatialQueue, Some(0.0)),
        ("level 4", FidelityLevel::Full, Some(lag)),
    ] {
        let done = toy.run(&cars, level, 300.0, 1500.0);
        assert_eq!(done.len(), n as usize, "{label}: everyone finishes");
        let e: Vec<u32> = done.iter().map(|t| enter(t, 0)).collect();
        for (k, &recorded) in e.iter().enumerate() {
            let expected = lag.map_or(f64::from(2 * k as u32), |l| entry(k as u32, l));
            close(recorded, expected, &format!("{label}, entry of car {k}"));
        }
        entries.push(e);
    }
    let (l2, l3, l4) = (&entries[0], &entries[1], &entries[2]);
    for (k, ((a, b), c)) in l2.iter().zip(l3).zip(l4).enumerate() {
        assert!(a <= b && b <= c, "car {k}: level 2 ≤ 3 ≤ 4");
    }
    assert!(entries[2][storage as usize] > 2 * storage, "the approach really filled at level 4");
}

/// **T4 · A saturated merge shares the supply in proportion to capacity
/// (S48).** Both approaches always have a car waiting; the merged link takes
/// its own capacity. Approach capacities are 1 : 2 (`a2` one lane, `m1` two),
/// so `a2` gets a third.
#[test]
fn t4_a_saturated_merge_shares_in_proportion_to_capacity() {
    let toy = Toy::new();
    let via_a2 = ["a2", "a3", "c1", "c2", "c3", "a4"];
    let via_m1 = ["m1", "a3", "c1", "c2", "c3", "a4"];
    let cars = merge_demand(&toy, &via_a2, &via_m1);
    let (cap_a2, cap_m1, cap_a3) = {
        let q = |n: &str| toy.net.link_parameters(toy.id(n)).capacity.get();
        (q("a2"), q("m1"), q("a3"))
    };
    let expected_share = cap_a2 / (cap_a2 + cap_m1);
    for step in [60.0, 300.0, 3600.0] {
        let done = toy.run(&cars, FidelityLevel::Full, step, 3600.0);
        assert_eq!(done.len(), cars.len(), "step {step}: everyone finishes");
        let (lo, hi) = (200u32, 500u32);
        let in_window: Vec<&Trajectory> =
            done.iter().filter(|t| (lo..hi).contains(&enter(t, 1))).collect();
        let from_a2 = in_window.iter().filter(|t| t.vehicle.raw() < 300).count();
        let share = from_a2 as f64 / in_window.len() as f64;
        let rate = in_window.len() as f64 / f64::from(hi - lo);
        assert!(
            (rate - cap_a3).abs() <= 0.03 * cap_a3,
            "step {step}: the merge carries {rate:.4}/s, its capacity is {cap_a3:.4}/s"
        );
        assert!(
            (share - expected_share).abs() <= 0.03,
            "step {step}: a2 took {share:.3} of the merge, capacity share {expected_share:.3}"
        );
    }
}

/// Stream A (300 cars, one per 2 s, ids below 300) on `a`, stream B (600 cars,
/// one per second) on `b`: both stay backlogged for the first ten minutes.
fn merge_demand(toy: &Toy, a: &[&str], b: &[&str]) -> Vec<Vehicle> {
    let mut cars: Vec<Vehicle> = (0..300).map(|k| toy.vehicle(k, a, 1.0, 2 * k)).collect();
    cars.extend((0..600).map(|k| toy.vehicle(300 + k, b, 1.0, k)));
    cars
}

/// **T5 · A saturated queue crosses the sub-vehicle chain at full capacity.**
/// Fifty cars leave together; the chain of three 4-m pieces throttles nothing.
#[test]
fn t5_a_queue_crosses_the_short_chain_at_capacity() {
    let toy = Toy::new();
    let route = ["a3", "c1", "c2", "c3", "a4"];
    let cars: Vec<Vehicle> = (0..50).map(|k| toy.vehicle(k, &route, 1.0, 0)).collect();
    assert!(50.0 < toy.storage("a3"), "all fifty fit on a3 at once");
    let h = toy.headway("a3");
    let after_a3: f64 = ["c1", "c2", "c3", "a4"].iter().map(|n| toy.ff(n)).sum();
    for (k, t) in toy.run_full(&cars, 900.0).iter().enumerate() {
        close(
            t.arrival().get(),
            toy.ff("a3") + h * k as f64 + after_a3,
            &format!("car {k}, arrival"),
        );
    }
}

/// **T6 · Roundabout: entering traffic gives way (S155).** A circulating car
/// and an entering car meet at `r1`. The entering car arrives while the
/// circulating one is on `r4`; it waits for `r4` to clear and then for the
/// inflow headway on `r1`. The circulating car is not delayed at all.
#[test]
fn t6_an_entering_car_gives_way_to_the_circulating_one() {
    let toy = Toy::new();
    let (cv_departs, ev_departs) = (30u32, 34u32);
    let cars = [
        toy.vehicle(0, &["n3", "r4", "r1", "e2"], 1.0, cv_departs),
        toy.vehicle(1, &["a5", "r1", "e2"], 1.0, ev_departs),
    ];
    let done = toy.run_full(&cars, 600.0);
    // The circulating car, in free flow.
    let on_r4 = f64::from(cv_departs) + toy.ff("n3");
    let on_r1 = on_r4 + toy.ff("r4");
    let cv_arrives = on_r1 + toy.ff("r1") + toy.ff("e2");
    close(enter(&done[0], 1), on_r4, "circulating car enters r4");
    close(enter(&done[0], 2), on_r1, "circulating car enters r1");
    close(done[0].arrival().get(), cv_arrives, "circulating car arrives");
    // The entering car reaches the end of a5 while the other is on r4.
    let at_the_line = f64::from(ev_departs) + toy.ff("a5");
    assert!(on_r4 <= at_the_line && at_the_line <= on_r1, "the fixture makes them meet");
    let enters_r1 = at_the_line.max(on_r1 + toy.headway("r1"));
    close(enter(&done[1], 1), enters_r1, "entering car enters r1");
    close(done[1].arrival().get(), enters_r1 + toy.ff("r1") + toy.ff("e2"), "entering car arrives");
}

/// **T7 · A jammed branch throttles the whole diverge (FIFO).** In every ten
/// cars, positions 0, 3 and 6 go to `a4`, the rest to the slower `a5` branch.
/// A car for `a5` directly behind a car for `a4` cannot leave the chain sooner
/// than two chain headways after the `a5` car before them.
#[test]
fn t7_a_jammed_branch_throttles_the_diverge() {
    let toy = Toy::new();
    let to_a4 = ["a3", "c1", "c2", "c3", "a4"];
    let to_a5 = ["a3", "c1", "c2", "c3", "a5", "r1", "e2"];
    let cars: Vec<Vehicle> = (0..600u32)
        .map(|k| toy.vehicle(k, if matches!(k % 10, 0 | 3 | 6) { &to_a4 } else { &to_a5 }, 1.0, k))
        .collect();
    let (h_chain, h_branch) = (toy.headway("c3"), toy.headway("a5"));
    assert!(h_branch > h_chain && h_branch < 2.0 * h_chain, "the fixture's branch is the slow one");
    // Per ten cars: three a5 cars follow an a4 car, four follow an a5 car.
    let cycle = 3.0 * (2.0 * h_chain) + 4.0 * h_branch;
    let (total, to_a4_rate) = (10.0 / cycle, 3.0 / cycle);
    let done = toy.run_full(&cars, 3600.0);
    let (lo, hi) = (300u32, 600u32);
    let window: Vec<&Trajectory> = done.iter().filter(|t| (lo..hi).contains(&exit(t, 3))).collect();
    let span = f64::from(hi - lo);
    let rate = window.len() as f64 / span;
    let a4_rate = window.iter().filter(|t| t.links.len() == 5).count() as f64 / span;
    assert!(
        (rate - total).abs() <= 0.03 * total,
        "the diverge passes {rate:.4}/s, expected {total:.4}/s"
    );
    assert!(
        (a4_rate - to_a4_rate).abs() <= 0.03 * to_a4_rate + 1.0 / span,
        "a4 gets {a4_rate:.4}/s, expected {to_a4_rate:.4}/s"
    );
}

/// **T8 · Vehicle size.** A two-PCU bus needs twice the saturation headway.
#[test]
fn t8_a_bus_takes_twice_the_headway() {
    let toy = Toy::new();
    let buses: Vec<Vehicle> = (0..10).map(|k| toy.vehicle(k, &["a1", "a2"], 2.0, 3 * k)).collect();
    let hs = 2.0 * toy.signal_headway("a1", "a2");
    assert!(20.0 < toy.storage("a1"), "ten buses fit: no spillback");
    for (k, t) in toy.run_full(&buses, 900.0).iter().enumerate() {
        close(
            exit(t, 0),
            toy.travel("a1") + hs * k as f64 + toy.delay("a1"),
            &format!("bus {k}, exit of a1"),
        );
    }
}

/// **T9 · A signalised junction: two approaches, one exit.** Each approach
/// discharges at its own `capacity · g/C`, independently; `a2` carries the sum.
#[test]
fn t9_a_signalised_junction_gives_each_approach_its_green_time() {
    let toy = Toy::new();
    let mut cars: Vec<Vehicle> =
        (0..600).map(|k| toy.vehicle(k, &["a1", "a2"], 1.0, 2 * k)).collect();
    cars.extend((0..600).map(|k| toy.vehicle(600 + k, &["s1", "a2"], 1.0, 2 * k)));
    let per_approach = 1.0 / toy.signal_headway("a1", "a2");
    assert!(
        2.0 * per_approach < toy.net.link_parameters(toy.id("a2")).capacity.get(),
        "a2 can carry both"
    );
    let (lo, hi) = (300u32, 900u32);
    let span = f64::from(hi - lo);
    for step in [60.0, 300.0, 3600.0] {
        let done = toy.run(&cars, FidelityLevel::Full, step, 7200.0);
        let rate = |from: u32, to: u32| {
            done.iter()
                .filter(|t| (from..to).contains(&t.vehicle.raw()) && (lo..hi).contains(&exit(t, 0)))
                .count() as f64
                / span
        };
        for (name, r) in [("a1", rate(0, 600)), ("s1", rate(600, 1200))] {
            assert!(
                (r - per_approach).abs() <= 0.03 * per_approach,
                "step {step}: {name} discharges {r:.4}/s, expected {per_approach:.4}/s"
            );
        }
    }
}

/// **T0 · Properties every case has.** Identical on repeat and under reversed
/// input order (K26), and identical for any loading step: the step is
/// bookkeeping only (S84, K2).
#[test]
fn t0_every_case_is_repeatable_order_free_and_step_free() {
    let toy = Toy::new();
    let cases: Vec<(&str, Vec<Vehicle>, f64)> = vec![
        ("t2", (0..20).map(|k| toy.vehicle(k, &["a1", "a2"], 1.0, 3 * k)).collect(), 900.0),
        ("t3", (0..60).map(|k| toy.vehicle(k, &["a1", "a2"], 1.0, 2 * k)).collect(), 1500.0),
        (
            "t4",
            merge_demand(
                &toy,
                &["a2", "a3", "c1", "c2", "c3", "a4"],
                &["m1", "a3", "c1", "c2", "c3", "a4"],
            ),
            3600.0,
        ),
        (
            "t5",
            (0..50).map(|k| toy.vehicle(k, &["a3", "c1", "c2", "c3", "a4"], 1.0, 0)).collect(),
            900.0,
        ),
        (
            "t6",
            vec![
                toy.vehicle(0, &["n3", "r4", "r1", "e2"], 1.0, 30),
                toy.vehicle(1, &["a5", "r1", "e2"], 1.0, 34),
            ],
            600.0,
        ),
        ("t8", (0..10).map(|k| toy.vehicle(k, &["a1", "a2"], 2.0, 3 * k)).collect(), 900.0),
    ];
    let shape = |done: &[Trajectory]| -> Vec<TripShape> {
        done.iter()
            .map(|t| {
                let legs =
                    t.links.iter().map(|l| (l.link.raw(), l.enter.get(), l.exit.get())).collect();
                (t.vehicle.raw(), t.departure.get(), legs)
            })
            .collect()
    };
    for (name, cars, horizon) in cases {
        let reference = shape(&toy.run(&cars, FidelityLevel::Full, 300.0, horizon));
        assert_eq!(reference.len(), cars.len(), "{name}: everyone finishes");
        assert_eq!(
            reference,
            shape(&toy.run(&cars, FidelityLevel::Full, 300.0, horizon)),
            "{name}: repeat"
        );
        let reversed: Vec<Vehicle> = cars.iter().rev().cloned().collect();
        assert_eq!(
            reference,
            shape(&toy.run(&reversed, FidelityLevel::Full, 300.0, horizon)),
            "{name}: reversed input"
        );
        for step in [60.0, 3600.0] {
            assert_eq!(
                reference,
                shape(&toy.run(&cars, FidelityLevel::Full, step, horizon)),
                "{name}: step {step}"
            );
        }
    }
}
