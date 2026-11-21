//! The reading on a route a traveller did not take, against the loading run again with that
//! one car moved to it: the time the car then takes is what the reading must have said. The
//! loading itself is held to Vickrey's recursion by hand in `point_queue.rs`; here it is the
//! outside answer for the reading, which is another part of the code.
//!
//! The first two tests and the fourth: two routes from `o` to `d`, each of two links, meeting
//! at `m` or not as a case needs: `x1` then `x2`, the route the cars under test take, and `y1`
//! then `y2`, the route each is moved to. Every car link is one lane at 50 km/h: 1 000 m take
//! 72 s by hand. The third and the fifth: two approaches `a` and `b` to one link `l2`, every
//! link 720 m at 36 km/h, 72 s by hand.
//!
//! The third test is a traveller reaching a link at the instant its own car did, ahead of the
//! car whose place it took on the link before. The fourth and the fifth show, by hand, two
//! cases of the reading's limits (stated in `src/link_times.rs`): where another car's entry on
//! a link of the route changes when the traveller is moved, the record cannot show it. A trip
//! with no car in the loading is added in the fifth. The last reports the run time of the
//! reading's longest walk, by hand in a release build:
//!
//! `cargo test --release -p openmobisim-loading --test reload -- --ignored --nocapture`

mod common;

use std::time::Instant;

use common::{VALUES, at, car_link, row, vehicle, window};
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

/// For a traveller with no car in the loading of `cars`: the time the reading of that loading
/// gives `car` on its route from its departure, and the time it takes when the loading is run
/// again with it added; both in seconds.
fn read_and_added(net: &Network, cars: &[Vehicle], car: &Vehicle) -> (f64, f64) {
    let w = window(300, 7_200);
    let times = LinkTimes::new(net, &load(net, cars, w).expect("loads").links);
    let read = times.route_time(&car.route, Mode::Car, car.departure, Reader::of(car.trip)).get();
    let added = [cars, core::slice::from_ref(car)].concat();
    let again = load(net, &added, w).expect("loads again").finished;
    let i = again.trips().iter().position(|&t| t == car.trip).expect("the added car finishes");
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

/// Nodes `o`, `m`, `n`, `d` and `e`, and the links, by name, `(from, to, capacity in veh/h)`:
/// each one lane, 720 m at 36 km/h, 720 / (36 / 3.6) = 72 s by hand.
fn at_36_km_h(links: &[(&str, &str, &str, f64)]) -> Network {
    let mut b = NetworkBuilder::new();
    let nodes = [("o", 0.0, 0.0), ("m", 720.0, 0.0), ("n", 1_440.0, 0.0), ("d", 2_160.0, 0.0)];
    for (node, x, y) in nodes {
        b.add_node(node, at(x, y));
    }
    b.add_node("e", at(1_440.0, 720.0));
    for &(id, from, to, veh_h) in links {
        let link = LinkRow { length_m: Some(720.0), capacity_veh_h: Some(veh_h), ..row(1, 36.0) };
        b.add_link(id, from, to, link);
    }
    b.build(VALUES).expect("the network builds")
}

#[test]
fn where_it_reaches_a_link_with_its_own_car_the_car_whose_place_it_took_is_behind_it() {
    // `a` and `b` from `o` to `m`, 1 200 veh/h (a 3 s headway); `l2` from `m` to `n`, 900 veh/h
    // (4 s); `l3` from `n` to `d`, 600 veh/h (6 s); every link 72 s at free flow. Recorded:
    // cars 0 (second 0) and 1 (second 1) on `a`, `l2`: car 0 leaves `a` at 72 s, car 1 at
    // max(73, 72 + 3) = 75 s. Car 2 (second 0) on `b` only, leaving at 72 s. Car 5 (second 0)
    // on `b`, `l2`: behind car 2 it leaves `b` at 75 s. On `l2`, car 0 enters at 72 s and
    // leaves at 144 s; cars 1 and 5 enter together at 75 s, in the order of their trips: car 1
    // leaves at max(147, 148) = 148 s, car 5 at max(147, 152) = 152 s.
    //
    // Car 5 is moved to `a`, `l2` from second 0. The reading: on `a` it enters with car 0, a
    // lower trip, ahead; it leaves at max(72, 75) = 75 s, the instant car 1 did, so it has car
    // 1's place. On `l2` at 75 s its own recorded car entered at that same instant, behind car
    // 1; but car 1, whose place it took, is behind it, so only car 0 is ahead: it leaves at
    // max(147, 144 + 4) = 148 s. Loaded again: on `a` car 5 enters with car 0 and leaves at
    // 75 s, car 1 at 78 s; on `l2` car 5 leaves at 148 s, car 1 at max(150, 152) = 152 s.
    //
    // With `l3` after `l2` on every route but car 2's, recorded: car 0 enters `l3` at 144 s and
    // leaves at 216 s, car 1 at 148 s leaving at max(220, 222) = 222 s, car 5 at 152 s leaving
    // at max(224, 228) = 228 s. The reading of car 5 moved to `a`, `l2`, `l3`: on `l3` at 148 s
    // car 1 entered at that same instant and is behind it (it took car 1's place on `l2` too),
    // car 0 is ahead: max(220, 216 + 6) = 222 s. Loaded again: car 5 enters `l3` at 148 s
    // behind car 0 and leaves at 222 s, car 1 enters at 152 s. On its own route car 5 reads
    // 152 s and 228 s, the times it took.
    //
    // The case is exact because no other car moves in it: no car was queued behind car 5 on
    // `b`. Had one been, it would move up when car 5 is moved, enter `l2` at that very instant
    // with a lower trip than car 5, and be ahead of it, which the record cannot show (the first
    // of the reading's limits, at this very point).
    let net = at_36_km_h(&[
        ("a", "o", "m", 1_200.0),
        ("b", "o", "m", 1_200.0),
        ("l2", "m", "n", 900.0),
        ("l3", "n", "d", 600.0),
    ]);
    for (tail, moved_by_hand, own_by_hand) in
        [(&["l2"][..], 148.0, 152.0), (&["l2", "l3"][..], 222.0, 228.0)]
    {
        let by = |first: &str| -> Vec<LinkId> { route(&net, &[&[first][..], tail].concat()) };
        let (by_a, by_b) = (by("a"), by("b"));
        let b_only = route(&net, &["b"]);
        let cars = [
            vehicle(0, Mode::Car, &by_a, 0),
            vehicle(1, Mode::Car, &by_a, 1),
            vehicle(2, Mode::Car, &b_only, 0),
            vehicle(5, Mode::Car, &by_b, 0),
        ];
        let (read, reloaded) = read_and_reloaded(&net, &cars, 3, &by_a);
        let links = tail.len() + 1;
        assert_eq!(
            read.to_bits(),
            reloaded.to_bits(),
            "{links} links: read {read}, takes {reloaded}"
        );
        assert!(
            (read - moved_by_hand).abs() < 1e-9,
            "{links} links: read {read} s, by hand {moved_by_hand} s"
        );
        let run = load(&net, &cars, window(300, 7_200)).expect("loads");
        let times = LinkTimes::new(&net, &run.links);
        let own = times.route_time(&by_b, Mode::Car, cars[3].departure, Reader::of(cars[3].trip));
        let i = run.finished.trips().iter().position(|&t| t == cars[3].trip).expect("finishes");
        let took = run.finished.travel_time(i).get();
        assert_eq!(own.get().to_bits(), took.to_bits(), "{links} links: own route read {own:?}");
        assert!(
            (took - own_by_hand).abs() < 1e-9,
            "{links} links: took {took} s, by hand {own_by_hand} s"
        );
    }
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
fn where_two_cars_that_entered_together_part_ways_the_next_link_is_read_one_car_out() {
    // The second case of the reading's limits shown by hand. `a` and `b` from `o` to `m`, 1 200 veh/h (a 3 s headway); `l2`
    // from `m` to `n`, 900 veh/h (4 s); `l3` from `n` to `d` and `l3x` from `n` to `e`, 600
    // veh/h (6 s); every link 72 s at free flow. Recorded: car 0 (second 0) on `a`, `l2`, `l3`
    // leaves them at 72, 144 and 216 s. Car 5 (second 1) on `a`, `l2`, then `l3` or `l3x`,
    // leaves `a` at max(73, 75) = 75 s. Car 3 (second 0) on `b` only leaves it at 72 s; car 10
    // (second 0) on `b`, `l2`, then `l3x` or `l3`, behind it at 75 s. Cars 5 and 10 enter `l2`
    // together at 75 s, in the order of their trips: car 5 leaves at max(147, 148) = 148 s,
    // car 10 at max(147, 152) = 152 s.
    //
    // Trip 12, with no car in the loading, is read on `a`, `l2`, `l3` from second 0. On `a` it
    // enters with car 0, a lower trip, and leaves at 75 s, the instant car 5 did: it has car
    // 5's place. On `l2` at 75 s car 5 is behind it, car 10 ahead, moved up into car 5's place
    // (out at 148 s); it leaves at max(147, 148 + 4) = 152 s, the instant car 10 did, so the
    // reading carries car 10 as the car behind it on `l3`. Loaded again with trip 12 added: car
    // 12 leaves `a` at 75 s and car 5 at 78 s; on `l2` car 10 leaves at 148 s, car 12 at 152 s,
    // car 5 at max(150, 156) = 156 s. In truth car 10 is ahead and car 5 behind, the reverse of
    // what the reading carries. This needs car 10's trip to lie between car 5's and the
    // traveller's (5 < 10 < 12): with a trip below 5, car 10 would have been recorded ahead of
    // car 5, the traveller would leave `l2` at car 5's recorded instant, and the reading would
    // hold both cars on their right sides. On `l3` from 152 s, car 0 out at 216 s, the next link is read one car out where
    // the two part ways:
    //   car 5 by `l3`, car 10 by `l3x`: recorded, car 5 enters `l3` at 148 s and leaves at
    //     max(220, 222) = 222 s; the reading counts it ahead, max(224, 228) = 228 s; loaded
    //     again car 5 is behind, max(224, 222) = 224 s. 4 s late;
    //   car 5 by `l3x`, car 10 by `l3`: recorded, car 10 enters at 152 s, the same instant,
    //     the reading counts it behind, max(224, 222) = 224 s; loaded again car 10 enters at
    //     148 s and leaves at 222 s, ahead: max(224, 228) = 228 s. 4 s early;
    //   both by `l3`: one of the two ahead either way, 228 s read and loaded again;
    //   both by `l3x`: neither, 224 s read and loaded again.
    // On `l3` one car on the wrong side costs at most one headway of `l3`, 6 s; here 4 s, as the
    // free-flow time of 224 s binds on one side. Queues on links further on could add to it.
    let net = at_36_km_h(&[
        ("a", "o", "m", 1_200.0),
        ("b", "o", "m", 1_200.0),
        ("l2", "m", "n", 900.0),
        ("l3", "n", "d", 600.0),
        ("l3x", "n", "e", 600.0),
    ]);
    for (five, ten, read_by_hand, taken_by_hand) in [
        ("l3", "l3x", 228.0, 224.0),
        ("l3x", "l3", 224.0, 228.0),
        ("l3", "l3", 228.0, 228.0),
        ("l3x", "l3x", 224.0, 224.0),
    ] {
        let cars = [
            vehicle(0, Mode::Car, &route(&net, &["a", "l2", "l3"]), 0),
            vehicle(5, Mode::Car, &route(&net, &["a", "l2", five]), 1),
            vehicle(3, Mode::Car, &route(&net, &["b"]), 0),
            vehicle(10, Mode::Car, &route(&net, &["b", "l2", ten]), 0),
        ];
        let traveller = vehicle(12, Mode::Car, &route(&net, &["a", "l2", "l3"]), 0);
        let (read, taken) = read_and_added(&net, &cars, &traveller);
        let case = format!("car 5 by {five}, car 10 by {ten}");
        assert!(
            (read - read_by_hand).abs() < 1e-9,
            "{case}: read {read} s, by hand {read_by_hand} s"
        );
        assert!(
            (taken - taken_by_hand).abs() < 1e-9,
            "{case}: takes {taken} s, by hand {taken_by_hand} s"
        );
    }
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
