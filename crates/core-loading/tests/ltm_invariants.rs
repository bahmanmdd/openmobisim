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

/// **Property (D1):** one vehicle of PCU `p` over a chain leaves exactly `p`
/// in and `p` out on every link — never a multiple of `p`.
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
            assert!(
                (sim.cumulative_in(l).get() - pcu).abs() < 1e-9
                    && (sim.cumulative_out(l).get() - pcu).abs() < 1e-9,
                "link {l:?} must carry exactly {pcu} PCU in and out, got in {:?} out {:?}",
                sim.cumulative_in(l),
                sim.cumulative_out(l)
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
        let firsts: Vec<LinkId> = vehicles.iter().map(|v| v.route[0]).collect();
        let done = run_checked(&mut sim, step, 3600.0, |s| {
            for idx in 0..net.link_count() {
                let l = LinkId::from_index(idx as usize);
                let (i, o, q) = (s.cumulative_in(l).get(), s.cumulative_out(l).get(), s.queued_pcu(l).get());
                if (i - o - q).abs() > 1e-6 || o > i + 1e-9 || o < 0.0 {
                    violation.get_or_insert((l, i, o, q));
                }
                // Storage holds on links nobody departs onto (departures arrive
                // from outside the network and are not admission-controlled).
                if !firsts.contains(&l) && i - o > net.storage(l).get() + pcu + 1e-6 {
                    overfull.get_or_insert((l, i - o, net.storage(l).get()));
                }
            }
        });
        prop_assert!(violation.is_none(), "curves and vehicles disagree: (link, in, out, queued) = {violation:?}");
        prop_assert!(overfull.is_none(), "a link holds more than its storage plus one vehicle: (link, held, storage) = {overfull:?}");
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
                    let got = sim.cumulative_out(a).get();
                    assert!(
                        (got - expected).abs() <= pcu + 1e-6,
                        "pcu {pcu}, step {step}, t {t}: discharged {got:.2} PCU, capacity allows {expected:.2}"
                    );
                }
            }
        }
    }
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
    let expected = (600.0 - ff) * rate + 1.0;
    let got = sim.cumulative_out(a).get();
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
        let (ol, oh) = (sl.cumulative_out(route[2]).get(), sh.cumulative_out(route[2]).get());
        assert!(
            (ol - oh).abs() <= 10.0 + 1e-6,
            "step {s}: light {ol} PCU vs heavy {oh} PCU past the short link"
        );
    }
    assert!(
        sh.cumulative_out(route[2]).get() >= 50.0 - 1e-6,
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
/// upstream link's discharge to its own rate, and never holds more than its
/// storage plus one vehicle.
#[test]
fn spillback_limits_upstream_discharge_to_the_bottleneck_rate() {
    // a -> b: 1 km; b -> c: 100 m, signalised at c; c -> d: long. Not shorter:
    // S90's control delay is time spent on the link, so a signalised link
    // shorter than about storage × delay / capacity holds its own throughput
    // below g/C capacity — a property of S90, recorded in S151, not this test's.
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
        let held = sim.cumulative_in(bc).get() - sim.cumulative_out(bc).get();
        assert!(held <= net.storage(bc).get() + 1.0 + 1e-6, "step {s}: bc holds {held} PCU");
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
