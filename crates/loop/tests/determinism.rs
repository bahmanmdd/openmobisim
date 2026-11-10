//! Determinism, the one self-comparison, of the whole chain: a run with mode choice (car, bike,
//! walk, transit) under the nested logit on a loaded grid with three transit lines gives, on
//! one thread and on eight, and over two runs built from scratch, to the bit: the same demand
//! (every end, every access and egress list), the same sets (every route and journey found) and
//! the same three tables (every column but the stages' wall-clock seconds).
//!
//! The one thread-count comparison of the parallel stages a run goes through: the demand's ends
//! and stop lists (`Demand::build`), the searches by car, bike and walk on loaded readings
//! (`fastest_many`), the journeys by number of rides (`by_rides_many`), the choice, the loading
//! and the sums of the assessment. Each case those stages branch on is asserted to occur at
//! least once, or the comparison would compare little: every mode taken, a car slowed by a
//! queue, a trip offered two journeys by number of rides, a journey with a footpath between two
//! rides, and stop-list entries within `stop_walk_max` and beyond it (the nearest stop of an end
//! with none within).
//!
//! For bikes: bike routes are searched, chosen, loaded and counted on links like the other modes,
//! and compared to the bit with them.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "counts of trips and seconds of the window, far below 2^53 and above zero"
)]

mod common;

use common::{VALUES, at};
use openmobisim_demand::demand::Demand;
use openmobisim_demand::input::TripRow;
use openmobisim_graph::network::{LinkRow, Network, NetworkBuilder};
use openmobisim_loop::alternative::Leg;
use openmobisim_loop::parameters::Parameters;
use openmobisim_loop::run::{Run, run};
use openmobisim_transit::timetable::{Call, Timetable, TimetableBuilder};
use openmobisim_types::ids::{EntityId, StopId, TripId};
use openmobisim_types::mode::{Mode, ModeSet};
use openmobisim_types::time::Second;
use openmobisim_types::units::{Duration, Metres};

/// The longest walk to or from a stop the lists hold beside others, metres: short against the
/// grid's 2 700 m, so that ends far from both lines get their nearest stop beyond it.
const STOP_WALK_MAX_M: f64 = 600.0;

/// A 10 × 10 grid of two-way streets 300 m apart, one lane at 30 km/h and 600 veh/h, open to
/// every mode.
fn grid() -> Network {
    let n = 10;
    let mut b = NetworkBuilder::new();
    for r in 0..n {
        for c in 0..n {
            b.add_node(format!("n{r}_{c}"), at(f64::from(c) * 300.0, f64::from(r) * 300.0));
        }
    }
    let row = LinkRow {
        modes: ModeSet::of(&[Mode::Car, Mode::Bike, Mode::Walk]),
        lanes: Some(1),
        car_speed_km_h: Some(30.0),
        length_m: None,
        capacity_veh_h: Some(600.0),
        signal_at_downstream_end: false,
    };
    for r in 0..n {
        for c in 0..n {
            if c + 1 < n {
                b.add_link(
                    format!("e{r}_{c}"),
                    format!("n{r}_{c}"),
                    format!("n{r}_{}", c + 1),
                    row,
                );
                b.add_link(
                    format!("w{r}_{c}"),
                    format!("n{r}_{}", c + 1),
                    format!("n{r}_{c}"),
                    row,
                );
            }
            if r + 1 < n {
                b.add_link(
                    format!("s{r}_{c}"),
                    format!("n{r}_{c}"),
                    format!("n{}_{c}", r + 1),
                    row,
                );
                b.add_link(
                    format!("x{r}_{c}"),
                    format!("n{}_{c}", r + 1),
                    format!("n{r}_{c}"),
                    row,
                );
            }
        }
    }
    b.build(VALUES).expect("a grid")
}

/// Three lines: one along row 4 and one along column 4, crossing at node n4_4 (1 200 m,
/// 1 200 m) at a stop they share, so a transfer there needs no footpath; and one along row 7
/// with stops of its own, whose stop at node n7_4 has a footpath of 60 s each way to the column's
/// stop there. A stop at each node of a line (29 stops). A run each way every 300 s, 23 runs each
/// way, from second 0 on row 4, 120 on column 4 and 60 on row 7; 30 s between stops, and on row
/// 7 a dwell of 10 s at each stop besides.
fn timetable() -> Timetable {
    let mut b = TimetableBuilder::new();
    let at_m = |c: u32, r: u32| (Metres(f64::from(c) * 300.0), Metres(f64::from(r) * 300.0));
    for k in 0..10 {
        b.add_stop(format!("s4_{k}"), at_m(k, 4));
        b.add_stop(format!("t7_{k}"), at_m(k, 7));
        if k != 4 {
            b.add_stop(format!("s{k}_4"), at_m(4, k));
        }
    }
    b.add_footpath("t7_4", "s7_4", Duration(60.0));
    b.add_footpath("s7_4", "t7_4", Duration(60.0));
    let lines: [(&str, u32, u32, Vec<String>); 3] = [
        ("row4", 0, 0, (0..10).map(|c| format!("s4_{c}")).collect()),
        ("column4", 120, 0, (0..10).map(|r| format!("s{r}_4")).collect()),
        ("row7", 60, 10, (0..10).map(|c| format!("t7_{c}")).collect()),
    ];
    for (line, offset, dwell, stops) in lines {
        let back: Vec<String> = stops.iter().rev().cloned().collect();
        for (way, order) in [("out", &stops), ("back", &back)] {
            for j in 0..23_u32 {
                let calls = (0..)
                    .zip(order)
                    .map(|(k, stop): (u32, &String)| {
                        let t = offset + 300 * j + (30 + dwell) * k;
                        Call {
                            stop: stop.clone(),
                            arrival: Second(t),
                            departure: Second(t + dwell),
                        }
                    })
                    .collect();
                b.add_run(format!("{line}_{way}_{j}"), calls);
            }
        }
    }
    b.build().expect("a timetable")
}

/// The mode closed to trip `i`, by `i % 5`: none to one trip in five.
const CLOSED: [Option<Mode>; 5] =
    [Some(Mode::Car), Some(Mode::Bike), Some(Mode::Walk), Some(Mode::Transit), None];

/// 3 000 trips with ends drawn by a linear congruential generator over the grid's extent,
/// departing over one hour, two classes; every mode open to one trip in five, and to each of the
/// others every mode but one, car, bike, walk or transit in turn (so some trips have no ends by a
/// mode, and some no access or egress list).
fn trips() -> Vec<TripRow> {
    let mut x: u64 = 12_345;
    let mut draw = move || {
        x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        (x >> 11) as f64 / (1u64 << 53) as f64
    };
    (0..3_000)
        .map(|i| TripRow {
            id: format!("t{i:05}"),
            origin: at(draw() * 2_700.0, draw() * 2_700.0),
            destination: at(draw() * 2_700.0, draw() * 2_700.0),
            departure: Second((draw() * 3_600.0) as u32),
            class: if i % 2 == 0 { "a".to_string() } else { "b".to_string() },
            modes: Mode::ALL
                .into_iter()
                .filter(|&m| Some(m) != CLOSED[i % 5])
                .fold(ModeSet::EMPTY, ModeSet::with),
        })
        .collect()
}

fn one_run(network: &Network, demand: &Demand, timetable: &Timetable) -> Run {
    // Class a at the defaults; class b weighs every minute twice and a euro at 1.0.
    let a = Parameters::DEFAULT_COEFFICIENTS;
    let mut b = a;
    for x in &mut b[..9] {
        *x *= 2.0;
    }
    b[10] = 1.0;
    let parameters = Parameters {
        iteration_cap: 12,
        seed: 0,
        nest_scale: 0.5,
        coefficients: vec![a, b],
        ..Parameters::defaults(2)
    };
    let window = (Second(0), Second(2 * 3_600));
    run(network, demand, Some(timetable), window, &parameters).expect("a run")
}

/// Everything the demand holds, as bits: per trip its class, departure, modes, ends per mode
/// and both lists; per stop its node and gap; the three counts.
fn demand_bits(d: &Demand) -> Vec<u64> {
    let mut out = Vec::new();
    for t in TripId::iter_space(d.trip_count()) {
        out.push(u64::from(d.class(t).raw()));
        out.push(u64::from(d.departure(t).get()));
        out.extend(Mode::ALL.map(|m| u64::from(d.modes(t).contains(m))));
        for m in Mode::ALL {
            for end in [d.origin_end(t, m), d.destination_end(t, m)] {
                out.push(end.map_or(u64::MAX, |e| u64::from(e.node.raw())));
                out.push(end.map_or(u64::MAX, |e| e.gap.get().to_bits()));
            }
        }
        for list in [d.access(t), d.egress(t)] {
            out.push(list.len() as u64);
            for &(s, seconds) in list {
                out.extend([u64::from(s.raw()), seconds.get().to_bits()]);
            }
        }
    }
    for s in StopId::iter_space(d.stop_count()) {
        out.extend([u64::from(d.stop_end(s).node.raw()), d.stop_end(s).gap.get().to_bits()]);
    }
    out.extend(
        [d.origins_without_stop(), d.destinations_without_stop(), d.trips_without_transit()]
            .map(u64::from),
    );
    out
}

/// Every column of the three tables, as bits, but the stages' wall-clock seconds; and whether
/// the rule stopped the run.
fn table_bits(r: &Run) -> Vec<u64> {
    let t = &r.trips;
    let mut out: Vec<u64> = t.class.iter().map(|c| u64::from(c.raw())).collect();
    out.extend(t.departure.iter().map(|s| u64::from(s.get())));
    out.extend(t.mode.iter().map(|m| m.map_or(u64::MAX, |m| u64::from(m.as_u8()))));
    for column in [&t.arrival, &t.travel_time, &t.cost] {
        out.extend(column.iter().map(|x| x.to_bits()));
    }
    out.extend(t.finished.iter().map(|&f| u64::from(f)));
    out.extend(t.alternatives.iter().map(|&n| u64::from(n)));
    let l = &r.links;
    out.extend(l.links().iter().map(|x| u64::from(x.raw())));
    out.extend(l.bins().iter().map(|b| u64::from(b.get())));
    out.extend(l.cars().iter().chain(l.bikes()).chain(l.walkers()).map(|&c| u64::from(c)));
    for column in [l.car_time(), l.first_entry(), l.first_exit(), l.last_entry(), l.last_exit()] {
        out.extend(column.iter().map(|d| d.get().to_bits()));
    }
    for it in &r.iterations {
        out.extend([u64::from(it.iteration), it.finished, it.unfinished, it.without_alternative]);
        out.extend([it.mean_trip_time, it.switch_share, it.found_share, it.gap].map(f64::to_bits));
    }
    out.push(u64::from(r.stopped_by_rule));
    out
}

/// What one whole run gives: the demand's bits, the tables' bits, and per trip its set and the
/// alternative it took, written out in full (`{:?}` writes every number exactly).
struct Output {
    demand: Vec<u64>,
    tables: Vec<u64>,
    sets: Vec<String>,
}

/// The network, the timetable and the demand built and the run made on a pool of `threads`
/// threads; its output, the demand and the run.
fn whole_chain(threads: usize) -> (Output, Demand, Run) {
    let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().expect("a pool");
    pool.install(|| {
        let (network, tt, rows) = (grid(), timetable(), trips());
        let classes = ["a".to_string(), "b".to_string()];
        let demand = Demand::build(&rows, &classes, &network, Some(&tt), Metres(STOP_WALK_MAX_M))
            .expect("a demand");
        let r = one_run(&network, &demand, &tt);
        let sets = TripId::iter_space(demand.trip_count())
            .map(|t| format!("{:?} {:?}", r.set(t), r.taken(t)))
            .collect();
        let output = Output { demand: demand_bits(&demand), tables: table_bits(&r), sets };
        (output, demand, r)
    })
}

/// The cases the comparison must reach, each counted on one run.
#[derive(Debug)]
struct Cases {
    /// Trips by the mode taken: car, bike, walk, transit.
    modes: [usize; 4],
    /// Link bins whose cars took over 1 s longer than free flow on average: a queue.
    queued: usize,
    /// Trips offered two journeys or more by number of rides.
    by_rides: usize,
    /// Trips offered a journey with a footpath between two rides.
    footpath: usize,
    /// Stop-list entries (access and egress) within `stop_walk_max`.
    within: usize,
    /// Stop-list entries beyond it: the nearest stop of an end with none within.
    beyond: usize,
}

fn cases(network: &Network, demand: &Demand, r: &Run) -> Cases {
    let modes = Mode::ALL.map(|m| r.trips.mode.iter().filter(|&&x| x == Some(m)).count());
    let l = &r.links;
    let queued = (0..l.len())
        .filter(|&i| l.cars()[i] > 0)
        .filter(|&i| {
            let free = network.free_flow_time(l.links()[i], Mode::Car).get();
            l.car_time()[i].get() / f64::from(l.cars()[i]) > free + 1.0
        })
        .count();
    let trips = || TripId::iter_space(demand.trip_count());
    let transit = |t| r.set(t).iter().filter(|a| a.mode == Mode::Transit);
    let by_rides = trips().filter(|&t| transit(t).count() >= 2).count();
    let walked =
        |leg: &Leg| matches!(leg, Leg::Journey(j) if j.footpaths.iter().any(Option::is_some));
    let footpath = trips().filter(|&t| transit(t).any(|a| a.legs.iter().any(walked))).count();
    let limit = (Metres(STOP_WALK_MAX_M) / network.walk_speed()).get();
    let entries = || trips().flat_map(|t| [demand.access(t), demand.egress(t)]).flatten();
    let beyond = entries().filter(|e| e.1.get() > limit).count();
    Cases { modes, queued, by_rides, footpath, within: entries().count() - beyond, beyond }
}

#[test]
fn the_same_tables_at_1_and_8_threads_and_over_two_runs() {
    let (one, demand, r) = whole_chain(1);
    let (eight, ..) = whole_chain(8);
    let (again, ..) = whole_chain(8);
    common::print_iterations(&r);
    let c = cases(&grid(), &demand, &r);
    println!("{c:?}");
    assert!(c.modes.iter().all(|&n| n > 0), "every mode is taken by some: {:?}", c.modes);
    assert!(c.queued > 0, "a car met a queue");
    assert!(c.by_rides > 0, "a trip was offered two journeys by number of rides");
    assert!(c.footpath > 0, "a trip was offered a journey with a footpath");
    assert!(c.within > 0 && c.beyond > 0, "stop-list entries within and beyond the limit");
    let first_difference = |a: &[u64], b: &[u64]| a.iter().zip(b).position(|(x, y)| x != y);
    for (other, what) in [(&eight, "1 and 8 threads"), (&again, "two runs")] {
        assert_eq!(one.demand.len(), other.demand.len(), "{what}: demand lengths");
        assert_eq!(first_difference(&one.demand, &other.demand), None, "{what}: demand differs");
        assert_eq!(one.tables.len(), other.tables.len(), "{what}: table lengths");
        assert_eq!(first_difference(&one.tables, &other.tables), None, "{what}: tables differ");
        assert_eq!(one.sets.len(), other.sets.len(), "{what}: set counts");
        let set = one.sets.iter().zip(&other.sets).position(|(x, y)| x != y);
        assert_eq!(set, None, "{what}: the set of this trip differs");
    }
}
