//! The reading on a route a traveller did not take, against the loading run again with that
//! one car moved to it: the time the car then takes is what the reading must have said. The
//! loading itself is held to Vickrey's recursion by hand in `point_queue.rs`; here it is the
//! outside answer for the reading, which is another part of the code.
//!
//! Two routes from `o` to `d`, each of two links, meeting at `m` or not as a case needs:
//! `x1` then `x2`, the route the cars under test take, and `y1` then `y2`, the route each is
//! moved to. Every car link is one lane at 50 km/h: 1 000 m take 72 s by hand.
//!
//! The third test shows the one case the reading cannot settle from the record, and states it.
//! The last reports the run time of the reading's longest walk, by hand in a release build:
//!
//! `cargo test --release -p openmobisim-loading --test reload -- --ignored --nocapture`

mod common;

use std::time::Instant;

use common::{VALUES, at, car_link, vehicle, window};
use openmobisim_graph::network::{LinkRow, Network, NetworkBuilder};
use openmobisim_loading::input::Vehicle;
use openmobisim_loading::link_times::{LinkTimes, Reader};
use openmobisim_loading::point_queue::load;
use openmobisim_types::ids::LinkId;
use openmobisim_types::mode::Mode;

/// Nodes `o`, `p`, `q`, `m`, `d` and the links, by name, `(from, to, row)`.
fn network(links: &[(&str, &str, &str, LinkRow)]) -> Network {
    let mut b = NetworkBuilder::new();
    let nodes = [("o", 0.0, 0.0), ("p", 500.0, 500.0), ("q", 500.0, -500.0), ("m", 1_000.0, 0.0)];
    for (node, x, y) in nodes {
        b.add_node(node, at(x, y));
    }
    b.add_node("d", at(2_000.0, 0.0));
    for &(id, from, to, row) in links {
        b.add_link(id, from, to, row);
    }
    b.build(VALUES).expect("the network builds")
}

/// The links named `ids`, in order.
fn route(net: &Network, ids: &[&str]) -> Vec<LinkId> {
    ids.iter().map(|id| net.link_ids().typed_id_of(id).expect("a link")).collect()
}

/// For car `k` of `cars` and another route: the time the reading of the loading gives that
/// car on the route from its departure, and the time it takes when the loading is run again
/// with it moved there; both in seconds.
fn read_and_reloaded(net: &Network, cars: &[Vehicle], k: usize, other: &[LinkId]) -> (f64, f64) {
    let w = window(300, 7_200);
    let times = LinkTimes::new(net, &load(net, cars, w).expect("loads").links);
    let car = &cars[k];
    let read = times.route_time(other, Mode::Car, car.departure, Reader::of(car.trip)).get();
    let mut moved = cars.to_vec();
    moved[k].route = other.to_vec();
    let again = load(net, &moved, w).expect("loads again").finished;
    let i = again.trips().iter().position(|&t| t == car.trip).expect("the moved car finishes");
    (read, again.travel_time(i).get())
}

#[test]
fn on_a_route_apart_from_its_own_a_moved_car_reads_what_it_then_takes() {
    // The routes share no link. `y1` passes 1 200 veh/h (a 3 s headway) and `y2` 900 veh/h
    // (4 s): 200 cars on `y`, one every 2 s, queue on both. 200 cars on `x` leave at the same
    // seconds with a lower trip each, so a car moved to `y` enters `y1` at the instant a car
    // of `y` does, ahead of it, and takes its place on `y2` as well.
    let free = f64::INFINITY;
    let net = network(&[
        ("x1", "o", "p", car_link(1_000.0, free)),
        ("x2", "p", "d", car_link(1_000.0, free)),
        ("y1", "o", "q", car_link(1_000.0, 1_200.0)),
        ("y2", "q", "d", car_link(1_000.0, 900.0)),
    ]);
    let (x, y) = (route(&net, &["x1", "x2"]), route(&net, &["y1", "y2"]));
    let mut cars: Vec<Vehicle> = (0..200).map(|k| vehicle(k, Mode::Car, &x, 2 * k)).collect();
    cars.extend((0..200).map(|k| vehicle(200 + k, Mode::Car, &y, 2 * k)));
    let mut queued = 0;
    for k in 0..200 {
        let (read, reloaded) = read_and_reloaded(&net, &cars, k, &y);
        assert_eq!(read.to_bits(), reloaded.to_bits(), "car {k}: read {read}, takes {reloaded}");
        queued += usize::from(reloaded > 144.0 + 1.0);
    }
    assert!(queued >= 190, "only {queued} of 200 moved cars met the queues");
}

#[test]
fn where_the_routes_share_a_queue_its_own_car_is_taken_out_of_it() {
    // Both routes end on `md`, 900 veh/h (a 4 s headway); the links before it pass any flow.
    // 200 cars on `x`, one every 2 s, queue on `md`. The way round by `y1` is 3 000 m longer,
    // 216 s: a car moved to `y` reaches `md` behind its own recorded car, which is no longer
    // in the queue, so every car that was behind it has moved up one place.
    let free = f64::INFINITY;
    let net = network(&[
        ("x1", "o", "m", car_link(1_000.0, free)),
        ("y1", "o", "q", car_link(4_000.0, free)),
        ("y2", "q", "m", car_link(1_000.0, free)),
        ("md", "m", "d", car_link(1_000.0, 900.0)),
    ]);
    let (x, y) = (route(&net, &["x1", "md"]), route(&net, &["y1", "y2", "md"]));
    let cars: Vec<Vehicle> = (0..200).map(|k| vehicle(k, Mode::Car, &x, 2 * k)).collect();
    let mut queued = 0;
    for k in 0..200 {
        let (read, reloaded) = read_and_reloaded(&net, &cars, k, &y);
        assert_eq!(read.to_bits(), reloaded.to_bits(), "car {k}: read {read}, takes {reloaded}");
        queued += usize::from(reloaded > 432.0 + 1.0);
    }
    assert!(queued >= 100, "only {queued} of 200 moved cars met the queue");
}

#[test]
fn where_a_bottleneck_upstream_refills_the_queue_the_reading_is_one_headway_short() {
    // The known limit. `x1` passes 1 200 veh/h (3 s) into `md`, 900 veh/h (4 s): 300 cars on
    // `x`, one every 2 s; car j leaves `x1` at 72 + 3j and `md` at 144 + 4j. Car 10 is moved
    // to `y`, 300 s to `m` (4 166.67 m), where it arrives at 20 + 300 = 320 s.
    //
    // The reading takes its own car out of `md`'s queue and moves the cars behind it up one
    // place: the cars that entered before 320 s are cars 0 to 82 (72 + 3 · 82 = 318), the
    // last now leaving at 144 + 4 · 81 = 468 s, so it reads an exit at 472 s: 452 s in all.
    //
    // Loaded again, car 10 is gone from `x1` too: every car behind it leaves `x1` 3 s
    // earlier, so car 83 enters `md` at 318 s, ahead of the moved car: 83 cars are ahead as
    // before, the last leaving at 472 s, and the moved car at 476 s: 456 s in all. The queue
    // upstream has refilled the place the car left, which the record of `md` cannot show.
    let free = f64::INFINITY;
    let net = network(&[
        ("x1", "o", "m", car_link(1_000.0, 1_200.0)),
        ("y1", "o", "q", car_link(1_000.0, free)),
        ("y2", "q", "m", car_link(3_166.666_666_666_667, free)),
        ("md", "m", "d", car_link(1_000.0, 900.0)),
    ]);
    let (x, y) = (route(&net, &["x1", "md"]), route(&net, &["y1", "y2", "md"]));
    let cars: Vec<Vehicle> = (0..300).map(|k| vehicle(k, Mode::Car, &x, 2 * k)).collect();
    let (read, reloaded) = read_and_reloaded(&net, &cars, 10, &y);
    assert!((read - 452.0).abs() < 1e-9, "read {read} s, by hand 452 s");
    assert!((reloaded - 456.0).abs() < 1e-9, "takes {reloaded} s, by hand 456 s");
}

#[test]
#[ignore = "run time, by hand in a release build (see the module documentation)"]
fn the_run_time_of_reading_behind_its_own_car_in_a_queue_of_thousands() {
    // The reading's longest walk. 5 000 cars on `x`, one a second, into `md` at 900 veh/h (a
    // 4 s headway): the queue grows to 3 750 cars. The way round by `y` is 2 000 s longer, so
    // a car reading it reaches `md` with its own car and about 2 000 others ahead, every one
    // of which the reading moves up a place. Reported beside the same cars reading their own
    // route, where nothing is replayed.
    let free = f64::INFINITY;
    let net = network(&[
        ("x1", "o", "m", car_link(1_000.0, free)),
        ("y1", "o", "q", car_link(27_777.777_777_777_777, free)),
        ("y2", "q", "m", car_link(1_000.0, free)),
        ("md", "m", "d", car_link(1_000.0, 900.0)),
    ]);
    let (x, y) = (route(&net, &["x1", "md"]), route(&net, &["y1", "y2", "md"]));
    let cars: Vec<Vehicle> = (0..5_000).map(|k| vehicle(k, Mode::Car, &x, k)).collect();
    let run = load(&net, &cars, window(300, 28_800)).expect("loads");
    let t0 = Instant::now();
    let times = LinkTimes::new(&net, &run.links);
    let built = t0.elapsed().as_secs_f64();
    let read = |route: &[LinkId]| -> (f64, f64) {
        let t = Instant::now();
        let mut sum = 0.0;
        for _ in 0..20 {
            for car in &cars {
                sum +=
                    times.route_time(route, Mode::Car, car.departure, Reader::of(car.trip)).get();
            }
        }
        (sum / 20.0, t.elapsed().as_secs_f64() * 1e9 / (20.0 * 5_000.0))
    };
    let (own, own_ns) = read(&x);
    let (other, other_ns) = read(&y);
    let loaded: f64 = (0..run.finished.len()).map(|i| run.finished.travel_time(i).get()).sum();
    assert!((own - loaded).abs() < 1e-6 * loaded, "own routes read {own} s, loaded {loaded} s");
    eprintln!(
        "5 000 cars, a queue of 3 750: the reading built in {built:.4} s; own route {own_ns:.0} ns \
         per read; the way round behind its own car {other_ns:.0} ns per read (mean {:.1} s)",
        other / 5_000.0
    );
}
