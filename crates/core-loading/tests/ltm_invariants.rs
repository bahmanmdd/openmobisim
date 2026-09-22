//! Invariants of the LTM loading (levels 2–4) that S150's defects broke.
//!
//! Each test names the property it defends. They were written *before* the
//! hybrid loading (S147/S151) and run against the S141 prototype first, where
//! every one of D1–D4, G1 and G2 made at least one of them fail (S150/S151
//! record which). They assert properties, not the implementation: a future
//! loading scheme should pass them unchanged.
//!
//! Fixtures are hand-built chains and `manhattan_grid`s — mechanism fixtures,
//! not data.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "test fixtures: vehicle counts, indices and bounds are small and non-negative"
)]

use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
use openmobisim_core_graph::examples::{manhattan_grid, node_name};
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::network::{LinkSpec, RoadNetwork, RoadNetworkBuilder};
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_loading::ltm::MIN_PART;
use openmobisim_core_loading::{FidelityLevel, LtmNetwork, Trajectory, Vehicle, run_ltm};
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::{EntityId, LinkId, VehicleId};
use openmobisim_core_types::time::Second;
use openmobisim_core_types::units::{Duration, Pcu};
use proptest::prelude::*;

/// Metres per degree of longitude at the fixtures' latitude (45.7°).
const M_PER_DEG_LON: f64 = 111_320.0 * 0.698_415_6;

/// A straight chain of links with the given approximate lengths, all of
/// `class`. `signalised` lists interior node indices (1-based positions
/// between links) that carry a signal.
fn chain(lengths_m: &[f64], class: RoadClass, signalised: &[usize]) -> (RoadNetwork, Vec<LinkId>) {
    let mut b = RoadNetworkBuilder::new();
    let mut lon = 4.8;
    b.add_node("n0", LonLat::new(lon, 45.7));
    for (i, len) in lengths_m.iter().enumerate() {
        lon += len / M_PER_DEG_LON;
        b.add_node(format!("n{}", i + 1), LonLat::new(lon, 45.7));
    }
    for &s in signalised {
        b.mark_signalised(format!("n{s}"));
    }
    for i in 0..lengths_m.len() {
        b.add_link(format!("l{i}"), format!("n{i}"), format!("n{}", i + 1), LinkSpec::new(class));
    }
    let mut d = Diagnostics::new();
    let net =
        b.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut d).expect("buildable");
    let ids = (0..lengths_m.len())
        .map(|i| net.link_external_ids().typed_id_of::<LinkId>(&format!("l{i}")).expect("link"))
        .collect();
    (net, ids)
}

fn free_flow(net: &RoadNetwork, route: &[LinkId]) -> f64 {
    route.iter().map(|&l| net.free_flow_time(l).get()).sum()
}

/// Step a loading to `until`, calling `check` after every step.
fn run_checked(
    sim: &mut LtmNetwork<'_>,
    step: f64,
    until: f64,
    mut check: impl FnMut(&LtmNetwork<'_>),
) -> Vec<Trajectory> {
    let mut done = Vec::new();
    let mut t = 0.0;
    while t < until {
        done.extend(sim.step(Duration(step)));
        t += step;
        check(sim);
    }
    done
}

/// An L-shaped route on a `manhattan_grid`, row first or column first.
fn grid_route(
    net: &RoadNetwork,
    n: u32,
    from: (u32, u32),
    to: (u32, u32),
    rows_first: bool,
) -> Vec<LinkId> {
    let link = |a: (u32, u32), z: (u32, u32)| {
        let ext = format!("l_{}__{}", node_name(a.0, a.1), node_name(z.0, z.1));
        net.link_external_ids().typed_id_of::<LinkId>(&ext).expect("grid link")
    };
    let _ = n;
    let (mut r, mut c) = from;
    let mut route = Vec::new();
    for phase in 0..2 {
        if (phase == 0) == rows_first {
            while r != to.0 {
                let nr = if to.0 > r { r + 1 } else { r - 1 };
                route.push(link((r, c), (nr, c)));
                r = nr;
            }
        } else {
            while c != to.1 {
                let nc = if to.1 > c { c + 1 } else { c - 1 };
                route.push(link((r, c), (r, nc)));
                c = nc;
            }
        }
    }
    route
}

// --- D1/D2: vehicles and curves agree -------------------------------------

/// **Property (D1):** one vehicle of PCU `p` over a chain — including a link
/// shorter than it — is counted once on every link: what enters a link leaves
/// it, never more than `p` is counted, and exactly `p` passes each link's end.
#[test]
fn a_vehicle_is_counted_once_on_every_link() {
    let (net, route) = chain(&[101.0, 101.0, 4.7, 101.0], RoadClass::Residential, &[]);
    let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);
    for pcu in [1.0, 10.0] {
        let v = Vehicle::new(VehicleId::new(0), route.clone(), Pcu(pcu), Second(0));
        let mut sim = LtmNetwork::new(&net, &turns);
        sim.depart(&v);
        let done = run_checked(&mut sim, 60.0, 600.0, |_| {});
        assert_eq!(done.len(), 1, "the vehicle completes (pcu {pcu})");
        for &l in &route {
            let (i, o) = (sim.cumulative_in(l).get(), sim.cumulative_out(l).get());
            assert!(
                (i - o).abs() < 1e-9 && i <= pcu + 1e-9 && i > 0.0,
                "link {l:?}: in {i} and out {o} must match and not exceed {pcu}"
            );
            assert!(
                (sim.discharged_pcu(l).get() - pcu).abs() < 1e-9,
                "exactly one vehicle passes {l:?}"
            );
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 24, .. ProptestConfig::default() })]

    /// **Property (D2):** at the end of every step, on every link, the PCU the
    /// curves say is on the link equals the PCU of the vehicles queued on it.
    /// No inflow without a vehicle, no vehicle without inflow.
    #[test]
    fn curves_always_match_the_vehicles_on_them(
        n in 3u32..5,
        block in prop::sample::select(vec![12.0, 100.0, 400.0]),
        signals in any::<bool>(),
        pcu in prop::sample::select(vec![1.0, 2.0, 10.0]),
        step in prop::sample::select(vec![30.0, 120.0, 300.0]),
        trips in prop::collection::vec((0u32..25, 0u32..25, any::<bool>(), 0u32..1800), 1..120),
    ) {
        let (net, _) = manhattan_grid(n, block, signals);
        let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);
        let vehicles: Vec<Vehicle> = trips
            .iter()
            .enumerate()
            .filter_map(|(i, &(a, z, rows_first, dep))| {
                let from = (a % n, (a / n) % n);
                let to = (z % n, (z / n) % n);
                (from != to).then(|| {
                    let route = grid_route(&net, n, from, to, rows_first);
                    Vehicle::new(VehicleId::new(i as u32), route, Pcu(pcu), Second(dep))
                })
            })
            .collect();
        let mut sim = LtmNetwork::new(&net, &turns);
        for v in &vehicles {
            sim.depart(v);
        }
        let mut violation = None;
        let mut overfull = None;
        let done = run_checked(&mut sim, step, 3600.0, |s| {
            for idx in 0..net.link_count() {
                let l = LinkId::from_index(idx as usize);
                let (i, o, q) = (s.cumulative_in(l).get(), s.cumulative_out(l).get(), s.queued_pcu(l).get());
                if (i - o - q).abs() > 1e-6 || o > i + 1e-9 || o < 0.0 {
                    violation.get_or_insert((l, i, o, q));
                }
                // Storage holds on every link, whatever its length against the
                // vehicles' (S153), departures included.
                if s.counted_pcu(l).get() > net.storage(l).get() + 1e-6 {
                    overfull.get_or_insert((l, s.counted_pcu(l).get(), net.storage(l).get()));
                }
            }
        });
        prop_assert!(violation.is_none(), "curves and vehicles disagree: (link, in, out, queued) = {violation:?}");
        prop_assert!(overfull.is_none(), "a link counts more than its storage: (link, counted, storage) = {overfull:?}");
        // No vehicle leaves a link before its free-flow time (one-second floor).
        for t in &done {
            for tr in &t.links {
                let ff = net.free_flow_time(tr.link).get();
                prop_assert!(f64::from(tr.exit.get() - tr.enter.get()) >= ff.floor() - 1.0,
                    "a vehicle left {:?} faster than free flow: {:?}, ff {ff}", tr.link, tr);
            }
        }
    }
}

// --- D3/D4/G1: discharge happens at capacity, and always happens ---------

/// **Property (D4, and S141's smear):** a lone vehicle on an empty network
/// takes its free-flow time on every link, to the one-second clock, for any
/// step length — and always completes.
#[test]
fn a_lone_vehicle_takes_free_flow_time_for_any_step() {
    let (net, route) = chain(&[101.0, 4.7, 97.0, 2000.0, 30.0], RoadClass::Residential, &[]);
    let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);
    for pcu in [1.0, 10.0] {
        for step in [1.0, 7.0, 60.0, 300.0, 900.0] {
            let v = Vehicle::new(VehicleId::new(0), route.clone(), Pcu(pcu), Second(3));
            let window = 3.0 + free_flow(&net, &route) + 2.0 * step + 10.0;
            let done =
                run_ltm(&net, &turns, &[v], Duration(window), Duration(step), FidelityLevel::Full);
            assert_eq!(done.len(), 1, "a lone vehicle must complete (pcu {pcu}, step {step})");
            for tr in &done[0].links {
                let ff = net.free_flow_time(tr.link).get();
                let took = f64::from(tr.exit.get() - tr.enter.get());
                assert!(
                    (took - ff).abs() <= 1.0,
                    "pcu {pcu}, step {step}: link {:?} took {took} s, free flow is {ff:.2} s",
                    tr.link
                );
            }
            assert_eq!(
                done[0].departure(),
                Second(3),
                "departure is not coarsened to the step grid"
            );
        }
    }
}

/// Every timing check elsewhere in this crate tolerates ±1 s (S88's own tolerance;
/// `a_lone_vehicle_takes_free_flow_time_for_any_step` above, and every case in
/// `toy_network.rs`), wide enough to hide flooring vs. rounding entirely — confirmed live in
/// checkpoint 8b's adversarial pass: floor-to-nearest-second changed to round-to-nearest
/// survived all 60 tests this crate had at the time. A length chosen so the free-flow time's
/// own fractional part is 0.6 makes floor and round disagree by a whole second, checked at a
/// tolerance tight enough (0.05 s) that only flooring — S88's documented rule — can pass.
#[test]
fn recorded_times_are_floored_not_rounded() {
    let (net, route) = chain(&[105.0], RoadClass::Residential, &[]);
    let ff = net.free_flow_time(route[0]).get();
    assert!(
        (ff.fract() - 0.6).abs() < 0.02,
        "fixture must land near a whole number plus 0.6 s to tell floor from round apart: {ff}"
    );
    let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);
    let v = Vehicle::new(VehicleId::new(0), route.clone(), Pcu(1.0), Second(0));
    let done =
        run_ltm(&net, &turns, &[v], Duration(ff + 10.0), Duration(60.0), FidelityLevel::Full);
    assert_eq!(done.len(), 1);
    let tr = &done[0].links[0];
    let took = f64::from(tr.exit.get() - tr.enter.get());
    assert!(
        (took - ff.floor()).abs() < 0.05,
        "took {took} s for a free-flow time of {ff} s: expected the floor ({}), not a round",
        ff.floor()
    );
}

/// **Property (D3):** a saturated approach discharges at its capacity,
/// whatever the vehicles' PCU and the step length — within one vehicle.
#[test]
fn a_queue_discharges_at_capacity_for_any_vehicle_size() {
    let (net, route) = chain(&[101.0, 3000.0], RoadClass::Residential, &[]);
    let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);
    let a = route[0];
    let cap = net.link_parameters(a).capacity.get();
    let ff = net.free_flow_time(a).get();
    for (pcu, count) in [(1.0, 200u32), (10.0, 20)] {
        for step in [5.0, 60.0, 300.0] {
            let vehicles: Vec<Vehicle> = (0..count)
                .map(|i| Vehicle::new(VehicleId::new(i), route.clone(), Pcu(pcu), Second(0)))
                .collect();
            let mut sim = LtmNetwork::new(&net, &turns);
            for v in &vehicles {
                sim.depart(v);
            }
            let mut t = 0.0;
            while t < 300.0 {
                let _ = sim.step(Duration(step));
                t += step;
                if t >= ff {
                    let expected = ((t - ff) * cap + pcu).min(pcu * f64::from(count));
                    let got = sim.discharged_pcu(a).get();
                    assert!(
                        (got - expected).abs() <= pcu + 1e-6,
                        "pcu {pcu}, step {step}, t {t}: discharged {got:.2} PCU, capacity allows {expected:.2}"
                    );
                }
            }
        }
    }
}

/// **Property (S153, vehicles straddle links):** a queue passes through a link at
/// capacity whatever the link's length relative to the vehicles — including
/// links shorter than one vehicle and links just above any size. Each piece
/// of the chain is fed by a standing queue, so its entries are limited by
/// room, not by arrivals.
#[test]
fn a_queue_passes_links_of_any_length_at_capacity() {
    let mut failures = Vec::new();
    for len in [3.0, 6.0, 9.0, 12.0, 15.0, 18.0, 22.0, 30.0, 45.0, 70.0] {
        let (net, route) = chain(&[len, len, len, 3000.0], RoadClass::Residential, &[]);
        let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);
        let piece = route[2];
        let cap = net.link_parameters(piece).capacity.get();
        for pcu in [1.0, 10.0] {
            let count = (2.0 * cap * 900.0 / pcu).ceil() as u32 + 2;
            let vehicles: Vec<Vehicle> = (0..count)
                .map(|i| Vehicle::new(VehicleId::new(i), route.clone(), Pcu(pcu), Second(0)))
                .collect();
            let mut sim = LtmNetwork::new(&net, &turns);
            vehicles.iter().for_each(|v| sim.depart(v));
            let _ = sim.step(Duration(300.0));
            let before = sim.discharged_pcu(piece).get();
            let _ = sim.step(Duration(600.0));
            let passed = sim.discharged_pcu(piece).get() - before;
            let expected = cap * 600.0;
            if (passed - expected).abs() > 2.0 * pcu + 1e-6 {
                failures.push(format!(
                    "{len} m (storage {:.2}), pcu {pcu}: {passed:.1} PCU in 600 s, capacity {expected:.1}",
                    net.storage(piece).get()
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// **Property (G1):** a signalised approach discharges at capacity × g/C.
#[test]
fn a_signalised_approach_discharges_at_green_time_capacity() {
    let (net, route) = chain(&[101.0, 3000.0], RoadClass::Secondary, &[1]);
    let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);
    let a = route[0];
    let turn = turns.find(a, route[1]).expect("the through movement exists");
    let rate = net.link_parameters(a).capacity.get() * f64::from(turns.capacity_fraction(turn));
    assert!(f64::from(turns.capacity_fraction(turn)) < 1.0, "the fixture's node is signalised");
    let ff = net.free_flow_time(a).get();
    let vehicles: Vec<Vehicle> = (0..400)
        .map(|i| Vehicle::new(VehicleId::new(i), route.clone(), Pcu(1.0), Second(0)))
        .collect();
    let mut sim = LtmNetwork::new(&net, &turns);
    for v in &vehicles {
        sim.depart(v);
    }
    let _ = sim.step(Duration(300.0));
    let _ = sim.step(Duration(300.0));
    // Vehicles pass the stop line after the travel time, before the signal
    // delay (which is spent at the stop line, S153).
    let travel = ff - net.link_parameters(a).control_delay.get();
    let expected = (600.0 - travel) * rate + 1.0;
    let got = sim.discharged_pcu(a).get();
    assert!(
        (got - expected).abs() <= 1.0 + 1e-6,
        "discharged {got:.2} PCU in 600 s, g/C capacity allows {expected:.2}"
    );
}

// --- Short links and heavy vehicles (S148 b, c) ---------------------------

/// **Property:** ten 1-PCU vehicles and one 10-PCU vehicle cross a chain with
/// a link shorter than one vehicle, and their throughput differs by at most
/// one vehicle at every step.
#[test]
fn heavy_and_light_vehicles_have_the_same_throughput() {
    let (net, route) = chain(&[101.0, 4.7, 101.0, 3000.0], RoadClass::Residential, &[]);
    let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);
    assert!(net.storage(route[1]).get() < 1.0, "the fixture needs a link shorter than a vehicle");
    let light: Vec<Vehicle> = (0..50)
        .map(|i| Vehicle::new(VehicleId::new(i), route.clone(), Pcu(1.0), Second(0)))
        .collect();
    let heavy: Vec<Vehicle> = (0..5)
        .map(|i| Vehicle::new(VehicleId::new(i), route.clone(), Pcu(10.0), Second(0)))
        .collect();
    let mut sl = LtmNetwork::new(&net, &turns);
    let mut sh = LtmNetwork::new(&net, &turns);
    light.iter().for_each(|v| sl.depart(v));
    heavy.iter().for_each(|v| sh.depart(v));
    for s in 0..12 {
        let _ = sl.step(Duration(60.0));
        let _ = sh.step(Duration(60.0));
        let (ol, oh) = (sl.discharged_pcu(route[2]).get(), sh.discharged_pcu(route[2]).get());
        assert!(
            (ol - oh).abs() <= 10.0 + 1e-6,
            "step {s}: light {ol} PCU vs heavy {oh} PCU past the short link"
        );
    }
    assert!(
        sh.discharged_pcu(route[2]).get() >= 50.0 - 1e-6,
        "every heavy vehicle gets past the short link"
    );
}

// --- G2: memory ------------------------------------------------------------

/// **Property (G2):** what a link keeps for its receiving-flow lookups does
/// not grow with the length of the run.
#[test]
fn retained_history_is_bounded_by_the_wave_travel_time() {
    let (net, _) = manhattan_grid(4, 150.0, true);
    let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);
    let n = 4;
    let vehicles: Vec<Vehicle> = (0..2000u32)
        .map(|i| {
            let from = (i % n, (i / 3) % n);
            let to = ((i / 7 + 1) % n, (i / 11 + 2) % n);
            let to = if to == from { ((from.0 + 1) % n, from.1) } else { to };
            Vehicle::new(
                VehicleId::new(i),
                grid_route(&net, n, from, to, i % 2 == 0),
                Pcu(1.0),
                Second(i * 10),
            )
        })
        .collect();
    let mut sim = LtmNetwork::new(&net, &turns);
    vehicles.iter().for_each(|v| sim.depart(v));
    let step = 60.0;
    let mut worst = 0usize;
    let _ = run_checked(&mut sim, step, 6.0 * 3600.0, |s| {
        for idx in 0..net.link_count() {
            let l = LinkId::from_index(idx as usize);
            let p = net.link_parameters(l);
            let tau_w = net.link_length(l).get() / p.wave_speed.get();
            let bound = ((tau_w + 2.0 * step) * p.capacity.get()).ceil() as usize + 2;
            worst = worst.max(s.retained_history(l));
            assert!(
                s.retained_history(l) <= bound,
                "link {l:?} retains {} entries, bound {bound}",
                s.retained_history(l)
            );
        }
    });
    assert!(worst > 0, "the fixture must actually exercise the history");
}

// --- Determinism and FIFO ----------------------------------------------------

/// **Property (S77):** the result does not depend on the order vehicles are
/// handed to the loading.
#[test]
fn results_do_not_depend_on_input_order() {
    let (net, _) = manhattan_grid(5, 100.0, true);
    let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);
    let n = 5;
    let mut vehicles: Vec<Vehicle> = (0..600u32)
        .map(|i| {
            let from = (i % n, (i / 5) % n);
            let to = ((i / 3 + 2) % n, (i / 13 + 1) % n);
            let to = if to == from { ((from.0 + 1) % n, from.1) } else { to };
            Vehicle::new(
                VehicleId::new(i),
                grid_route(&net, n, from, to, i % 3 == 0),
                Pcu(1.0),
                Second(i % 900),
            )
        })
        .collect();
    let key = |mut t: Vec<Trajectory>| {
        t.sort_by_key(|x| x.vehicle.raw());
        t.into_iter().map(|x| (x.vehicle.raw(), x.links)).collect::<Vec<_>>()
    };
    let a = key(run_ltm(
        &net,
        &turns,
        &vehicles,
        Duration(3600.0),
        Duration(300.0),
        FidelityLevel::Full,
    ));
    vehicles.reverse();
    let b = key(run_ltm(
        &net,
        &turns,
        &vehicles,
        Duration(3600.0),
        Duration(300.0),
        FidelityLevel::Full,
    ));
    assert_eq!(a, b);
}

/// **Property:** on a single approach, vehicles leave every link in the order
/// they entered it.
#[test]
fn a_chain_is_first_in_first_out() {
    let (net, route) = chain(&[101.0, 10.0, 300.0, 50.0], RoadClass::Residential, &[2]);
    let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);
    let vehicles: Vec<Vehicle> = (0..300u32)
        .map(|i| {
            Vehicle::new(
                VehicleId::new(i),
                route.clone(),
                Pcu(1.0 + f64::from(i % 3)),
                Second((i * 7919) % 1200),
            )
        })
        .collect();
    let done =
        run_ltm(&net, &turns, &vehicles, Duration(7200.0), Duration(300.0), FidelityLevel::Full);
    assert_eq!(done.len(), vehicles.len(), "everything completes within two hours");
    for (k, &l) in route.iter().enumerate() {
        let mut legs: Vec<_> = done.iter().map(|t| t.links[k]).collect();
        assert!(legs.iter().all(|x| x.link == l));
        legs.sort_by_key(|x| (x.enter, x.exit));
        for w in legs.windows(2) {
            assert!(
                w[0].exit <= w[1].exit || w[0].enter == w[1].enter,
                "FIFO violated on link {l:?}: {:?}",
                w
            );
        }
    }
}

// --- Hand-checkable queueing -------------------------------------------------

/// **Property (the deterministic bottleneck):** vehicles arriving at a
/// bottleneck faster than it discharges leave exactly at its saturation
/// headway once the queue forms — the k-th vehicle's exit is
/// `max(arrival_k + free flow, exit_{k−1} + 1/capacity)`, to the one-second
/// clock.
#[test]
fn a_bottleneck_queue_matches_the_deterministic_queue_by_hand() {
    let (net, route) = chain(&[500.0, 3000.0], RoadClass::Residential, &[]);
    let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);
    let a = route[0];
    let headway = 1.0 / net.link_parameters(a).capacity.get();
    let ff = net.free_flow_time(a).get();
    // Arrivals every 2 s: faster than the ~2.57 s saturation headway.
    let vehicles: Vec<Vehicle> = (0..60)
        .map(|i| Vehicle::new(VehicleId::new(i), route.clone(), Pcu(1.0), Second(i * 2)))
        .collect();
    for level in [FidelityLevel::PointQueue, FidelityLevel::Full] {
        let done = run_ltm(&net, &turns, &vehicles, Duration(3600.0), Duration(300.0), level);
        let mut exits: Vec<(u32, Second)> =
            done.iter().map(|t| (t.vehicle.raw(), t.links[0].exit)).collect();
        exits.sort_unstable();
        let mut expected_prev = f64::NEG_INFINITY;
        for (k, exit) in exits {
            let expected = (f64::from(k * 2) + ff).max(expected_prev + headway);
            expected_prev = expected;
            assert!(
                (f64::from(exit.get()) - expected).abs() <= 1.0,
                "{level:?}: vehicle {k} left at {exit:?}, the deterministic queue says {expected:.2}"
            );
        }
    }
}

/// **Property (spillback):** a saturated short link downstream holds the
/// upstream link's discharge to its own rate, and never counts more than its
/// storage.
#[test]
fn spillback_limits_upstream_discharge_to_the_bottleneck_rate() {
    // a -> b: 1 km; b -> c: 100 m, signalised at c; c -> d: long.
    let (net, route) = chain(&[1000.0, 100.0, 3000.0], RoadClass::Secondary, &[2]);
    let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);
    let (ab, bc) = (route[0], route[1]);
    let bottleneck = net.link_parameters(bc).capacity.get()
        * f64::from(turns.capacity_fraction(turns.find(bc, route[2]).expect("through movement")));
    assert!(
        bottleneck < net.link_parameters(ab).capacity.get() * 0.6,
        "the fixture needs a real bottleneck"
    );
    let vehicles: Vec<Vehicle> = (0..3000)
        .map(|i| Vehicle::new(VehicleId::new(i), route.clone(), Pcu(1.0), Second(0)))
        .collect();
    let mut sim = LtmNetwork::new(&net, &turns);
    vehicles.iter().for_each(|v| sim.depart(v));
    let step = 300.0;
    let mut previous = 0.0;
    for s in 0..8 {
        let _ = sim.step(Duration(step));
        let out = sim.cumulative_out(ab).get();
        let held = sim.counted_pcu(bc).get();
        assert!(held <= net.storage(bc).get() + 1e-6, "step {s}: bc counts {held} PCU");
        if s >= 2 {
            let rate = (out - previous) / step;
            assert!(
                (rate - bottleneck).abs() <= 0.05 * bottleneck + 2.0 / step,
                "step {s}: ab discharges {rate:.4} PCU/s, the bottleneck allows {bottleneck:.4}"
            );
        }
        previous = out;
    }
}

/// **Property (S48, proportional to capacity):** two saturated approaches
/// merging into a bottleneck share it in proportion to their own discharge
/// capacities.
#[test]
fn saturated_merges_share_room_in_proportion_to_capacity() {
    let mut b = RoadNetworkBuilder::new();
    b.add_node("w", LonLat::new(4.790, 45.700));
    b.add_node("s", LonLat::new(4.800, 45.690));
    b.add_node("m", LonLat::new(4.800, 45.700));
    b.add_node("x", LonLat::new(4.8004, 45.700));
    b.add_node("e", LonLat::new(4.840, 45.700));
    b.mark_signalised("x");
    b.add_link("wm", "w", "m", LinkSpec::new(RoadClass::Primary));
    b.add_link("sm", "s", "m", LinkSpec::new(RoadClass::Residential));
    b.add_link("mx", "m", "x", LinkSpec::new(RoadClass::Residential));
    b.add_link("xe", "x", "e", LinkSpec::new(RoadClass::Residential));
    let mut d = Diagnostics::new();
    let net =
        b.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut d).expect("buildable");
    let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);
    let id = |e: &str| net.link_external_ids().typed_id_of::<LinkId>(e).expect("link");
    let (wm, sm, mx, xe) = (id("wm"), id("sm"), id("mx"), id("xe"));
    let mut vehicles = Vec::new();
    for i in 0..4000u32 {
        let route = if i % 2 == 0 { vec![wm, mx, xe] } else { vec![sm, mx, xe] };
        vehicles.push(Vehicle::new(VehicleId::new(i), route, Pcu(1.0), Second(0)));
    }
    let mut sim = LtmNetwork::new(&net, &turns);
    vehicles.iter().for_each(|v| sim.depart(v));
    for _ in 0..2 {
        let _ = sim.step(Duration(300.0));
    }
    let (w0, s0) = (sim.cumulative_out(wm).get(), sim.cumulative_out(sm).get());
    for _ in 0..4 {
        let _ = sim.step(Duration(300.0));
    }
    let (w, s) = (sim.cumulative_out(wm).get() - w0, sim.cumulative_out(sm).get() - s0);
    let (cw, cs) = (net.link_parameters(wm).capacity.get(), net.link_parameters(sm).capacity.get());
    let share = w / (w + s);
    let expected = cw / (cw + cs);
    assert!(w + s > 100.0, "the merge must be busy: {w} + {s}");
    assert!(
        (share - expected).abs() <= 0.05,
        "the primary approach took {share:.3} of the merge, capacity share {expected:.3}"
    );
}

// --- Stop lines, origins and waiting cycles (S153) -------------------------

/// **Property (S153, stop line):** a signalised approach shorter than its
/// queue's delay would need still discharges at capacity × g/C: the signal
/// delay is spent at the stop line, off the approach's storage. A lone
/// vehicle still takes the approach's free-flow time including the delay.
#[test]
fn a_short_signalised_approach_discharges_at_green_time_capacity() {
    let (net, route) = chain(&[200.0, 20.0, 3000.0], RoadClass::Residential, &[2]);
    let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);
    let short = route[1];
    let delay = net.link_parameters(short).control_delay.get();
    let cap = net.link_parameters(short).capacity.get();
    let rate =
        cap * f64::from(turns.capacity_fraction(turns.find(short, route[2]).expect("through")));
    assert!(delay > 0.0 && rate < cap, "the fixture's approach is signalised");
    assert!(
        net.storage(short).get() < delay * rate,
        "the fixture needs an approach too short to hold its queue's delay"
    );

    let lone = Vehicle::new(VehicleId::new(0), route.clone(), Pcu(1.0), Second(0));
    let done = run_ltm(&net, &turns, &[lone], Duration(900.0), Duration(60.0), FidelityLevel::Full);
    assert_eq!(done.len(), 1);
    for tr in &done[0].links {
        let took = f64::from(tr.exit.get() - tr.enter.get());
        let ff = net.free_flow_time(tr.link).get();
        assert!((took - ff).abs() <= 1.0, "link {:?} took {took} s, free flow {ff:.2} s", tr.link);
    }

    let vehicles: Vec<Vehicle> = (0..1000)
        .map(|i| Vehicle::new(VehicleId::new(i), route.clone(), Pcu(1.0), Second(0)))
        .collect();
    let mut sim = LtmNetwork::new(&net, &turns);
    vehicles.iter().for_each(|v| sim.depart(v));
    let _ = sim.step(Duration(300.0));
    let before = sim.discharged_pcu(short).get();
    let _ = sim.step(Duration(600.0));
    let passed = sim.discharged_pcu(short).get() - before;
    assert!(
        (passed - rate * 600.0).abs() <= 2.0,
        "passed {passed:.1} PCU in 600 s, g/C capacity allows {:.1}",
        rate * 600.0
    );
}

/// **Property (S153, stop line and spillback):** a full link ahead holds a
/// signalised approach back — the approach discharges at the bottleneck's
/// rate, its stop line holds no more than one control delay's worth of
/// vehicles, and the short link ahead never counts more than its storage. A
/// short link after a signal is not throttled by the
/// signal's delay.
#[test]
fn spillback_holds_back_a_signalised_approach() {
    // ab: 1 km primary, signalised at b; bc: 30 m residential, signalised at
    // c — the bottleneck; cd: long.
    let mut b = RoadNetworkBuilder::new();
    let at = |m: f64| LonLat::new(4.8 + m / M_PER_DEG_LON, 45.7);
    b.add_node("a", at(0.0));
    b.add_node("b", at(1000.0));
    b.add_node("c", at(1030.0));
    b.add_node("d", at(4030.0));
    b.mark_signalised("b");
    b.mark_signalised("c");
    b.add_link("ab", "a", "b", LinkSpec::new(RoadClass::Primary));
    b.add_link("bc", "b", "c", LinkSpec::new(RoadClass::Residential));
    b.add_link("cd", "c", "d", LinkSpec::new(RoadClass::Residential));
    let net = b
        .build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
        .expect("buildable");
    let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);
    let id = |e: &str| net.link_external_ids().typed_id_of::<LinkId>(e).expect("link");
    let (ab, bc, cd) = (id("ab"), id("bc"), id("cd"));
    let green = |l: LinkId, next: LinkId| {
        f64::from(turns.capacity_fraction(turns.find(l, next).expect("through movement")))
    };
    let upstream = net.link_parameters(ab).capacity.get() * green(ab, bc);
    let bottleneck = net.link_parameters(bc).capacity.get() * green(bc, cd);
    assert!(bottleneck < 0.8 * upstream, "the fixture needs a real bottleneck");
    let buffer = (upstream * net.link_parameters(ab).control_delay.get()).ceil() + 1.0;

    let vehicles: Vec<Vehicle> = (0..3000)
        .map(|i| Vehicle::new(VehicleId::new(i), vec![ab, bc, cd], Pcu(1.0), Second(0)))
        .collect();
    let mut sim = LtmNetwork::new(&net, &turns);
    vehicles.iter().for_each(|v| sim.depart(v));
    let bound = net.storage(bc).get();
    let step = 300.0;
    let mut previous = 0.0;
    for s in 0..6 {
        let _ = sim.step(Duration(step));
        assert!(sim.counted_pcu(bc).get() <= bound + 1e-6, "step {s}: bc counts too much");
        assert!(
            sim.waiting_at_stop_line(ab) as f64 <= buffer,
            "step {s}: {} vehicles wait at ab's stop line, one control delay is {buffer}",
            sim.waiting_at_stop_line(ab)
        );
        let out = sim.discharged_pcu(bc).get();
        if s >= 2 {
            let rate = (out - previous) / step;
            assert!(
                (rate - bottleneck).abs() <= 0.05 * bottleneck + 2.0 / step,
                "step {s}: bc passes {rate:.4} PCU/s, its g/C capacity is {bottleneck:.4}"
            );
        }
        previous = out;
    }
    assert!(sim.queue_len(ab) > 20, "the queue spills back onto ab");
}

/// **Property (S153, origins):** departures wait outside the network until
/// their first link has room, never overfilling it, and the wait is part of
/// the travel time.
#[test]
fn departures_wait_at_the_origin_and_the_wait_counts() {
    let (net, route) = chain(&[30.0, 3000.0], RoadClass::Residential, &[]);
    let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);
    let first = route[0];
    let vehicles: Vec<Vehicle> = (0..40)
        .map(|i| Vehicle::new(VehicleId::new(i), route.clone(), Pcu(1.0), Second(0)))
        .collect();
    let mut sim = LtmNetwork::new(&net, &turns);
    vehicles.iter().for_each(|v| sim.depart(v));
    let _ = sim.step(Duration(1.0));
    assert!(sim.counted_pcu(first).get() <= net.storage(first).get() + 1e-6);
    assert!(sim.waiting_at_origin(first) > 30, "most departures wait outside the network");
    let mut done = run_checked(&mut sim, 60.0, 1800.0, |s| {
        assert!(s.counted_pcu(first).get() <= net.storage(first).get() + 1e-6);
    });
    assert_eq!(done.len(), 40, "every departure completes");
    done.sort_by_key(|t| t.vehicle.raw());
    let headway = 1.0 / net.link_parameters(first).capacity.get();
    let ff = free_flow(&net, &route);
    for (k, t) in done.iter().enumerate() {
        assert_eq!(t.departure(), Second(0));
        let travel = f64::from(t.arrival().get() - t.departure().get());
        assert!(
            travel >= ff + (k as f64) * headway - 2.0,
            "vehicle {k}: travel {travel} s, but it cannot leave before {:.1} s",
            ff + (k as f64) * headway
        );
    }
    assert!(done[39].links[0].enter.get() > 0, "the last vehicle entered after waiting");
}

/// A one-way ring of pieces with the given lengths (metres), each ring node
/// with a 100-m entry and a 100-m exit: `ring[i]` runs from node `i` to node
/// `i + 1`, `entries[i]` ends at node `i`, `exits[i]` starts there. With
/// `tagged`, the ring is marked as a roundabout, so entries give way (S155).
fn roundabout(
    pieces: &[f64],
    tagged: bool,
) -> (RoadNetwork, Vec<LinkId>, Vec<LinkId>, Vec<LinkId>) {
    let k = pieces.len();
    let angle = |r: f64| pieces.iter().map(|l| 2.0 * (l / (2.0 * r)).min(1.0).asin()).sum::<f64>();
    let (mut lo, mut hi) = (pieces.iter().copied().fold(0.0, f64::max) / 2.0, 1e5);
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if angle(mid) > std::f64::consts::TAU { lo = mid } else { hi = mid }
    }
    let r = 0.5 * (lo + hi);
    let at = |x: f64, y: f64| LonLat::new(4.8 + x / M_PER_DEG_LON, 45.7 + y / 111_320.0);
    let mut b = RoadNetworkBuilder::new();
    let mut theta: f64 = 0.0;
    for (i, l) in pieces.iter().enumerate() {
        let (x, y) = (r * theta.cos(), r * theta.sin());
        b.add_node(format!("r{i}"), at(x, y));
        let out = (r + 100.0) / r;
        b.add_node(format!("a{i}"), at(x * out + 20.0 * theta.sin(), y * out - 20.0 * theta.cos()));
        b.add_node(format!("x{i}"), at(x * out - 20.0 * theta.sin(), y * out + 20.0 * theta.cos()));
        theta += 2.0 * (l / (2.0 * r)).asin();
    }
    for i in 0..k {
        b.add_link(
            format!("ring{i}"),
            format!("r{i}"),
            format!("r{}", (i + 1) % k),
            LinkSpec { roundabout: tagged, ..LinkSpec::new(RoadClass::Residential) },
        );
        b.add_link(
            format!("in{i}"),
            format!("a{i}"),
            format!("r{i}"),
            LinkSpec::new(RoadClass::Residential),
        );
        b.add_link(
            format!("out{i}"),
            format!("r{i}"),
            format!("x{i}"),
            LinkSpec::new(RoadClass::Residential),
        );
    }
    let net = b
        .build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
        .expect("buildable");
    let ids = |name: &str| -> Vec<LinkId> {
        (0..k)
            .map(|i| {
                net.link_external_ids().typed_id_of::<LinkId>(&format!("{name}{i}")).expect("link")
            })
            .collect()
    };
    let (ring, entries, exits) = (ids("ring"), ids("in"), ids("out"));
    (net, ring, entries, exits)
}

/// **Property (S153, traffic stops only at jam density):** a roundabout
/// whose pieces span the whole range of lengths — shorter than one vehicle,
/// just above one, one and a half, two — under demand far above its capacity,
/// with cars, with vehicles aggregated 10 to a unit, and with a mix of cars and
/// buses: no piece ever counts more than its storage, and wherever vehicles
/// wait on one another in a loop, every link of the loop is full.
#[test]
fn an_overloaded_roundabout_stops_only_at_jam_density() {
    let pieces = [3.0, 4.5, 6.0, 7.5, 8.5, 10.0, 11.5, 13.0, 15.5, 18.0, 22.0, 30.0];
    let (net, ring, entries, exits) = roundabout(&pieces, false);
    let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);
    let k = pieces.len();
    for (i, &l) in ring.iter().enumerate() {
        assert!((net.link_length(l).get() - pieces[i]).abs() < 0.5, "piece {i} length");
    }
    for (label, every, size) in [
        ("cars", 5u32, (|_: usize| 1.0) as fn(usize) -> f64),
        ("aggregates of 10", 50, |_| 10.0),
        ("one bus in five", 5, |n| if n % 5 == 0 { 2.5 } else { 1.0 }),
    ] {
        let mut vehicles = Vec::new();
        for i in 0..k {
            for n in 0..(1800 / every) {
                let id = vehicles.len() as u32;
                let hops = 1 + ((n as usize + 3 * i) % (k - 1));
                let mut route = vec![entries[i]];
                route.extend((0..hops).map(|h| ring[(i + h) % k]));
                route.push(exits[(i + hops) % k]);
                let pcu = size(n as usize + i);
                vehicles.push(Vehicle::new(VehicleId::new(id), route, Pcu(pcu), Second(n * every)));
            }
        }
        let mut sim = LtmNetwork::new(&net, &turns);
        vehicles.iter().for_each(|v| sim.depart(v));
        let mut stops = 0;
        let done = run_checked(&mut sim, 300.0, 6.0 * 3600.0, |s| {
            for idx in 0..net.link_count() {
                let l = LinkId::from_index(idx as usize);
                assert!(
                    s.counted_pcu(l).get() <= net.storage(l).get() + 1e-6,
                    "{label}: {l:?} counts {} on {} of storage",
                    s.counted_pcu(l).get(),
                    net.storage(l).get()
                );
            }
            for cycle in s.waiting_cycles() {
                stops += 1;
                for &l in &cycle {
                    assert!(
                        s.counted_pcu(l).get() >= net.storage(l).get() - MIN_PART,
                        "{label}: a waiting loop {cycle:?} stands with room on {l:?}: {} of {}",
                        s.counted_pcu(l).get(),
                        net.storage(l).get()
                    );
                }
            }
        });
        println!(
            "{label}: {} of {} trips completed; waiting loops seen at {stops} step ends",
            done.len(),
            vehicles.len()
        );
    }
}

/// **Property (S155, give way at roundabouts):** the same overloaded
/// roundabout, tagged as one, keeps moving — entries wait for circulating
/// traffic instead of filling the ring — and every trip completes once demand
/// ends, for cars and for a mix of cars and buses.
#[test]
fn a_tagged_roundabout_gives_way_and_keeps_moving() {
    let pieces = [3.0, 4.5, 6.0, 7.5, 8.5, 10.0, 11.5, 13.0, 15.5, 18.0, 22.0, 30.0];
    let (net, ring, entries, exits) = roundabout(&pieces, true);
    assert!(ring.iter().all(|&l| net.is_roundabout(l)) && !net.is_roundabout(entries[0]));
    let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);
    let k = pieces.len();
    for (label, size) in [
        ("cars", (|_: usize| 1.0) as fn(usize) -> f64),
        ("one bus in five", |n| if n % 5 == 0 { 2.5 } else { 1.0 }),
    ] {
        let mut vehicles = Vec::new();
        for i in 0..k {
            for n in 0..360u32 {
                let hops = 1 + ((n as usize + 3 * i) % (k - 1));
                let mut route = vec![entries[i]];
                route.extend((0..hops).map(|h| ring[(i + h) % k]));
                route.push(exits[(i + hops) % k]);
                let id = VehicleId::new(vehicles.len() as u32);
                vehicles.push(Vehicle::new(id, route, Pcu(size(n as usize + i)), Second(n * 5)));
            }
        }
        let mut sim = LtmNetwork::new(&net, &turns);
        vehicles.iter().for_each(|v| sim.depart(v));
        let done = run_checked(&mut sim, 300.0, 6.0 * 3600.0, |s| {
            for &l in &ring {
                assert!(s.counted_pcu(l).get() <= net.storage(l).get() + 1e-6, "{label}: overfull");
            }
        });
        assert!(sim.waiting_cycles().is_empty(), "{label}: a loop is left standing");
        assert_eq!(done.len(), vehicles.len(), "{label}: every trip completes");
    }
}
