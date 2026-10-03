//! Turn pockets in the loading (S217): on an approach of two lanes or more, a vehicle passes
//! vehicles ahead of it that wait for another movement, as long as they fit in their pockets.
//!
//! A fork: from `a`, link `ab` to `b`, then `bc`–`cd` (towards a one-lane service road, whose
//! queue fills `bc`) or `be`–`ed` (free). Traffic already queued on `bc` keeps it full; a few
//! vehicles from `ab` bound for `c` wait at the end of `ab`; vehicles behind them bound for `e`
//! pass them in their own lane — unless `ab` has one lane, or its pocket cannot hold the ones
//! waiting. Then a property on a two-lane grid: everything the loading guarantees still holds.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "test fixtures: vehicle counts, indices and bounds are small and non-negative"
)]

use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::network::{LinkSpec, RoadNetwork, RoadNetworkBuilder};
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_loading::{LtmNetwork, Trajectory, Vehicle};
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::{EntityId, LinkId, VehicleId};
use openmobisim_core_types::time::Second;
use openmobisim_core_types::units::{Duration, Pcu};
use proptest::prelude::*;

const M_PER_DEG_LON: f64 = 111_320.0 * 0.6981; // cos(45.7°)

fn at(x: f64, y: f64) -> LonLat {
    LonLat::new(4.8 + x / M_PER_DEG_LON, 45.7 + y / 111_320.0)
}

fn build(b: RoadNetworkBuilder) -> RoadNetwork {
    b.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
        .expect("buildable")
}

struct Fork {
    net: RoadNetwork,
    ab: LinkId,
    bc: LinkId,
    cd: LinkId,
    be: LinkId,
    ed: LinkId,
}

fn fork(ab_lanes: u8) -> Fork {
    let mut b = RoadNetworkBuilder::new();
    b.add_node("a", at(0.0, 0.0));
    b.add_node("b", at(300.0, 0.0));
    b.add_node("c", at(600.0, 150.0));
    b.add_node("e", at(600.0, -150.0));
    b.add_node("d", at(900.0, 0.0));
    let mut ab = LinkSpec::new(RoadClass::Primary);
    ab.lanes = Some(ab_lanes);
    b.add_link("ab", "a", "b", ab);
    b.add_link("bc", "b", "c", LinkSpec::new(RoadClass::Primary));
    b.add_link("cd", "c", "d", LinkSpec::new(RoadClass::Service));
    b.add_link("be", "b", "e", LinkSpec::new(RoadClass::Secondary));
    b.add_link("ed", "e", "d", LinkSpec::new(RoadClass::Secondary));
    let net = build(b);
    let id = |n: &str| net.link_external_ids().typed_id_of::<LinkId>(n).expect("link");
    let (ab, bc, cd, be, ed) = (id("ab"), id("bc"), id("cd"), id("be"), id("ed"));
    Fork { net, ab, bc, cd, be, ed }
}

/// 400 vehicles queued onto `bc` at the start, keeping it full; then `to_c` vehicles along `ab`
/// bound for `c`, one a second from second 100, and twenty bound for `e` right behind them, one
/// every three seconds (well within what `be` takes).
fn demand(f: &Fork, to_c: u32) -> (Vec<Vehicle>, std::ops::Range<u32>) {
    let mut v: Vec<Vehicle> = (0..400u32)
        .map(|n| Vehicle::new(VehicleId::new(n), vec![f.bc, f.cd], Pcu(1.0), Second(0)))
        .collect();
    for n in 0..to_c {
        v.push(Vehicle::new(
            VehicleId::new(400 + n),
            vec![f.ab, f.bc, f.cd],
            Pcu(1.0),
            Second(100 + n),
        ));
    }
    let first_e = 400 + to_c;
    for n in 0..20 {
        let dep = Second(100 + to_c + 3 * n);
        v.push(Vehicle::new(VehicleId::new(first_e + n), vec![f.ab, f.be, f.ed], Pcu(1.0), dep));
    }
    (v, first_e..first_e + 20)
}

fn load(f: &Fork, vehicles: &[Vehicle], pocket_m: Option<f64>) -> Vec<Trajectory> {
    let turns = TurnTable::build(&f.net, SignalDefaults::SHIPPED);
    let mut sim = LtmNetwork::new(&f.net, &turns);
    if let Some(m) = pocket_m {
        sim = sim.with_pockets(&turns, m);
    }
    vehicles.iter().for_each(|v| sim.depart(v));
    let mut done = Vec::new();
    for _ in 0..240 {
        done.extend(sim.step(Duration(60.0)));
    }
    assert_eq!(done.len(), vehicles.len(), "every trip completes");
    done
}

/// The longest time a vehicle bound for `e` spent on `ab`, beyond free flow.
fn worst_wait_on_ab(f: &Fork, done: &[Trajectory], to_e: &std::ops::Range<u32>) -> f64 {
    let ff = f.net.free_flow_time(f.ab).get();
    done.iter()
        .filter(|t| to_e.contains(&t.vehicle.raw()))
        .map(|t| {
            let on_ab = t.links.iter().find(|x| x.link == f.ab).expect("crossed ab");
            f64::from(on_ab.exit.get() - on_ab.enter.get()) - ff
        })
        .fold(f64::NEG_INFINITY, f64::max)
}

#[test]
fn on_two_lanes_a_vehicle_turning_free_passes_those_waiting_to_turn_into_a_full_street() {
    let f = fork(2);
    let (vehicles, to_e) = demand(&f, 4);
    let fifo = load(&f, &vehicles, None);
    let pockets = load(&f, &vehicles, Some(50.0));
    let (wait_fifo, wait_pockets) =
        (worst_wait_on_ab(&f, &fifo, &to_e), worst_wait_on_ab(&f, &pockets, &to_e));
    assert!(wait_fifo > 30.0, "without pockets, held up behind them: {wait_fifo} s");
    println!(
        "worst wait on ab beyond free flow: {wait_fifo} s without pockets, {wait_pockets} s with"
    );
    assert!(wait_pockets < 2.0, "with pockets, past them in its own lane: {wait_pockets} s");
    // Those bound for `c` keep their order, and still go only when `bc` has room.
    let c_exits = |done: &[Trajectory]| {
        let mut v: Vec<(u32, u32)> = done
            .iter()
            .filter(|t| (400..404).contains(&t.vehicle.raw()))
            .map(|t| (t.vehicle.raw(), t.links[0].exit.get()))
            .collect();
        v.sort_unstable();
        v
    };
    let c = c_exits(&pockets);
    assert!(c.windows(2).all(|w| w[0].1 <= w[1].1), "first in, first out per movement: {c:?}");
    assert_eq!(c, c_exits(&fifo), "the vehicles waiting for `bc` are not affected");
}

/// A vehicle's links, with the seconds it entered and left each.
type Crossings = Vec<(u32, u32, u32)>;

#[test]
fn on_one_lane_nothing_passes_and_pockets_change_nothing() {
    let f = fork(1);
    let (vehicles, to_e) = demand(&f, 4);
    let key = |t: &[Trajectory]| {
        let mut v: Vec<(u32, Crossings)> = t
            .iter()
            .map(|t| {
                let links = t.links.iter().map(|x| (x.link.raw(), x.enter.get(), x.exit.get()));
                (t.vehicle.raw(), links.collect())
            })
            .collect();
        v.sort_unstable();
        v
    };
    let fifo = load(&f, &vehicles, None);
    let pockets = load(&f, &vehicles, Some(50.0));
    assert!(worst_wait_on_ab(&f, &fifo, &to_e) > 30.0);
    assert_eq!(key(&fifo), key(&pockets));
}

#[test]
fn a_pocket_too_short_for_those_waiting_holds_up_the_vehicles_behind() {
    let f = fork(2);
    let (vehicles, to_e) = demand(&f, 4);
    // Ten metres hold about one car: the second one waiting stands in the shared lanes.
    let short = load(&f, &vehicles, Some(10.0));
    assert!(worst_wait_on_ab(&f, &short, &to_e) > 30.0);
    let (more, to_e) = demand(&f, 12);
    // Twelve waiting do not fit in fifty metres either.
    let full = load(&f, &more, Some(50.0));
    assert!(worst_wait_on_ab(&f, &full, &to_e) > 30.0);
}

/// The fork with a major road `xb` joining at `b`, where `ab` is minor: `cd` lets through five
/// vehicles an hour, so a vehicle on `ab` bound for `c` waits there for the whole test; `ed` lets
/// through 900, so traffic on `xb` bound for `e` queues, and the vehicles behind `ab`'s front
/// bound for `e` give way to it.
fn fork_with_major_road() -> (Fork, LinkId) {
    let mut b = RoadNetworkBuilder::new();
    b.add_node("a", at(0.0, 0.0));
    b.add_node("x", at(300.0, 300.0));
    b.add_node("b", at(300.0, 0.0));
    b.add_node("c", at(600.0, 150.0));
    b.add_node("e", at(600.0, -150.0));
    b.add_node("d", at(900.0, 0.0));
    let mut ab = LinkSpec::new(RoadClass::Secondary);
    ab.lanes = Some(2);
    b.add_link("ab", "a", "b", ab);
    b.add_link("xb", "x", "b", LinkSpec::new(RoadClass::Primary));
    b.add_link("bc", "b", "c", LinkSpec::new(RoadClass::Primary));
    let mut cd = LinkSpec::new(RoadClass::Service);
    cd.capacity_veh_h = Some(5.0);
    b.add_link("cd", "c", "d", cd);
    b.add_link("be", "b", "e", LinkSpec::new(RoadClass::Primary));
    let mut ed = LinkSpec::new(RoadClass::Primary);
    ed.capacity_veh_h = Some(900.0);
    b.add_link("ed", "e", "d", ed);
    let net = build(b);
    let id = |n: &str| net.link_external_ids().typed_id_of::<LinkId>(n).expect("link");
    let (ab, bc, cd, be, ed, xb) = (id("ab"), id("bc"), id("cd"), id("be"), id("ed"), id("xb"));
    (Fork { net, ab, bc, cd, be, ed }, xb)
}

#[test]
fn under_priority_a_vehicle_past_a_waiting_front_goes_once_the_major_road_clears() {
    let (f, xb) = fork_with_major_road();
    let mut v: Vec<Vehicle> = (0..300u32)
        .map(|n| Vehicle::new(VehicleId::new(n), vec![f.bc, f.cd], Pcu(1.0), Second(0)))
        .collect();
    for n in 0..3 {
        let route = vec![f.ab, f.bc, f.cd];
        v.push(Vehicle::new(VehicleId::new(300 + n), route, Pcu(1.0), Second(100 + n)));
    }
    let to_e = 303..323u32;
    for n in to_e.clone() {
        let dep = Second(600 + 6 * (n - 303));
        v.push(Vehicle::new(VehicleId::new(n), vec![f.ab, f.be, f.ed], Pcu(1.0), dep));
    }
    let major = 400..700u32;
    for n in major.clone() {
        let dep = Second(100 + 2 * (n - 400));
        v.push(Vehicle::new(VehicleId::new(n), vec![xb, f.be, f.ed], Pcu(1.0), dep));
    }
    let turns = TurnTable::build(&f.net, SignalDefaults::SHIPPED);
    let run = |pocket_m: f64| {
        let mut sim = LtmNetwork::new(&f.net, &turns).with_priority();
        if pocket_m > 0.0 {
            sim = sim.with_pockets(&turns, pocket_m);
        }
        v.iter().for_each(|x| sim.depart(x));
        let mut done = Vec::new();
        // Half an hour: `bc` lets one vehicle in every twelve minutes, and the vehicles waiting
        // at its origin take turns with `ab`'s front.
        for _ in 0..30 {
            done.extend(sim.step(Duration(60.0)));
            assert_eq!(sim.lock_report().room_waits_with_room, 0);
        }
        done
    };
    let fifo = run(0.0);
    assert!(!fifo.iter().any(|t| to_e.contains(&t.vehicle.raw())), "stuck behind the front");
    let pockets = run(50.0);
    assert_eq!(pockets.iter().filter(|t| to_e.contains(&t.vehicle.raw())).count(), 20);
    let exit_of = |vehicles: &std::ops::Range<u32>, link: LinkId, pick: fn(u32, u32) -> u32| {
        pockets
            .iter()
            .filter(|t| vehicles.contains(&t.vehicle.raw()))
            .map(|t| t.links.iter().find(|x| x.link == link).expect("crossed").exit.get())
            .reduce(pick)
            .expect("some")
    };
    let last_major = exit_of(&major, xb, u32::max);
    let first_minor = exit_of(&to_e, f.ab, u32::min);
    // They give way while the major road's queue lasts, and go as soon as it has cleared:
    // woken when the major road's front moves on, not only when their own front does.
    assert!(first_minor + 2 >= last_major, "gave way: {first_minor} vs {last_major}");
    assert!(first_minor <= last_major + 15, "went once it cleared: {first_minor} vs {last_major}");
}

/// An `n`×`n` grid of two-lane primary roads, `block` metres apart.
fn two_lane_grid(n: u32, block: f64) -> RoadNetwork {
    let name = |r: u32, c: u32| format!("n{r}_{c}");
    let mut b = RoadNetworkBuilder::new();
    for r in 0..n {
        for c in 0..n {
            b.add_node(name(r, c), at(f64::from(c) * block, f64::from(r) * block));
        }
    }
    let spec = LinkSpec::new(RoadClass::Primary);
    for r in 0..n {
        for c in 0..n {
            for (r2, c2) in [(r, c + 1), (r + 1, c)] {
                if r2 < n && c2 < n {
                    b.add_link(
                        format!("{}-{}", name(r, c), name(r2, c2)),
                        name(r, c),
                        name(r2, c2),
                        spec,
                    );
                    b.add_link(
                        format!("{}-{}", name(r2, c2), name(r, c)),
                        name(r2, c2),
                        name(r, c),
                        spec,
                    );
                }
            }
        }
    }
    build(b)
}

/// An L-shaped route on [`two_lane_grid`], row first or column first.
fn grid_route(
    net: &RoadNetwork,
    from: (u32, u32),
    to: (u32, u32),
    rows_first: bool,
) -> Vec<LinkId> {
    let link = |a: (u32, u32), z: (u32, u32)| {
        let ext = format!("n{}_{}-n{}_{}", a.0, a.1, z.0, z.1);
        net.link_external_ids().typed_id_of::<LinkId>(&ext).expect("grid link")
    };
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

proptest! {
    #![proptest_config(ProptestConfig { cases: 24, .. ProptestConfig::default() })]

    /// **Property:** with turn pockets, under any demand on a two-lane grid, with or without
    /// priority at merges, the curves match the vehicles on every link at every step, no link
    /// counts more than its storage, no front — nor any movement waiting behind one — waits for
    /// room that is there (traffic still stops only at jam density), no vehicle is lost, and the
    /// vehicles of one movement leave a link in the order they entered it.
    #[test]
    fn pockets_keep_every_guarantee_of_the_loading(
        n in 3u32..5,
        block in prop::sample::select(vec![15.0, 60.0, 250.0]),
        pcu in prop::sample::select(vec![1.0, 2.5]),
        priority in any::<bool>(),
        trips in prop::collection::vec((0u32..25, 0u32..25, any::<bool>(), 0u32..300), 200..800),
    ) {
        let net = two_lane_grid(n, block);
        let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);
        let vehicles: Vec<Vehicle> = trips
            .iter()
            .enumerate()
            .filter_map(|(i, &(a, z, rows_first, dep))| {
                let from = (a % n, (a / n) % n);
                let to = (z % n, (z / n) % n);
                (from != to).then(|| {
                    let route = grid_route(&net, from, to, rows_first);
                    Vehicle::new(VehicleId::new(i as u32), route, Pcu(pcu), Second(dep))
                })
            })
            .collect();
        let mut sim = LtmNetwork::new(&net, &turns).with_pockets(&turns, 50.0);
        if priority {
            sim = sim.with_priority();
        }
        vehicles.iter().for_each(|v| sim.depart(v));
        let mut done = Vec::new();
        for _ in 0..30 {
            done.extend(sim.step(Duration(120.0)));
            for idx in 0..net.link_count() {
                let l = LinkId::from_index(idx as usize);
                let (i, o, q) =
                    (sim.cumulative_in(l).get(), sim.cumulative_out(l).get(), sim.queued_pcu(l).get());
                prop_assert!((i - o - q).abs() < 1e-6, "{l:?}: in {i}, out {o}, queued {q}");
                prop_assert!(sim.counted_pcu(l).get() <= net.storage(l).get() + 1e-6);
            }
            // Nothing waits for room that is there, at any step's end.
            let waits = sim.lock_report().room_waits_with_room;
            prop_assert_eq!(waits, 0, "a wait for room that is there");
        }
        let lock = sim.lock_report();
        prop_assert_eq!(lock.room_waits_with_room, 0, "{:?}", lock);
        prop_assert_eq!(lock.on_network as usize + lock.outside as usize + done.len(), vehicles.len());
        // First in, first out per movement: on each link, among vehicles bound for the same next
        // link (or ending there), whoever entered first leaves no later.
        let mut by_movement: std::collections::BTreeMap<(u32, u32), Vec<(u32, u32)>> =
            std::collections::BTreeMap::new();
        for t in &done {
            for (k, x) in t.links.iter().enumerate() {
                let next = t.links.get(k + 1).map_or(u32::MAX, |y| y.link.raw());
                by_movement.entry((x.link.raw(), next)).or_default().push((x.enter.get(), x.exit.get()));
            }
        }
        for (movement, mut crossings) in by_movement {
            crossings.sort_unstable();
            for w in crossings.windows(2) {
                prop_assert!(w[0].0 == w[1].0 || w[0].1 <= w[1].1, "{movement:?}: {w:?}");
            }
        }
    }
}
