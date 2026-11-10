//! Bikes and walkers (test 9): trip times equal length over speed by hand, unchanged by any car.
//!
//! The build values (`tests/common/mod.rs`): bike_speed 15 km/h (4.1667 m/s), walk_speed
//! 4.8 km/h (1.3333 m/s). They are test values, not defaults.

mod common;

use common::{
    VALUES, car_link, close, cycle_path, line, link, row, two_nodes, vehicle, whole_line, window,
};
use openmobisim_graph::network::LinkRow;
use openmobisim_loading::link_times::LinkTimes;
use openmobisim_loading::point_queue::{Loading, load};
use openmobisim_types::ids::EntityId;
use openmobisim_types::mode::Mode;
use openmobisim_types::time::Second;

/// A row for every link mode, `length_m` long.
fn street(length_m: f64) -> LinkRow {
    LinkRow { length_m: Some(length_m), ..row(1, 50.0) }
}

#[test]
fn a_bike_rides_1500_m_at_15_km_h_in_360_s() {
    // 1 500 m / (15 000 / 3 600 m/s) = 360 s, door to door, loaded and read alike.
    assert!(close(VALUES.bike_speed, 15.0));
    let net = two_nodes(street(1_500.0)).build(VALUES).unwrap();
    let run = load(&net, &[vehicle(0, Mode::Bike, &[link(0)], 30)], window(300, 3_600)).unwrap();
    let loaded = run.finished.travel_time(0).get();
    assert!(close(loaded, 360.0), "{loaded} s");
    let read = LinkTimes::new(&net, &run.links).route_time(&[link(0)], Mode::Bike, Second(30));
    assert_eq!(read.get().to_bits(), loaded.to_bits());
}

#[test]
fn a_walker_covers_1000_m_at_4_8_km_h_in_750_s() {
    // 1 000 m (the straight line) / (4 800 / 3 600 m/s) = 750 s.
    assert!(close(VALUES.walk_speed, 4.8));
    let net = two_nodes(row(1, 50.0)).build(VALUES).unwrap();
    let run = load(&net, &[vehicle(0, Mode::Walk, &[link(0)], 0)], window(300, 3_600)).unwrap();
    assert!(close(run.finished.travel_time(0).get(), 750.0));
}

#[test]
fn a_three_link_route_takes_the_sum_of_its_links_and_is_counted_on_each() {
    // 1 500 m, 1 000 m and 500 m: by bike 360 + 240 + 120 = 720 s, on foot 1 125 + 750 + 375 =
    // 2 250 s. The bike enters the links at 0, 360 and 600 s: one bike in bins 0, 1 and 2 of
    // 300 s; the walker at 0, 1 125 and 1 875 s: bins 0, 3 and 6.
    let net = line(&[street(1_500.0), street(1_000.0), street(500.0)]);
    let trips =
        [vehicle(0, Mode::Bike, &whole_line(3), 0), vehicle(1, Mode::Walk, &whole_line(3), 0)];
    let run = load(&net, &trips, window(300, 3_600)).unwrap();
    assert!(close(run.finished.travel_time(0).get(), 720.0));
    assert!(close(run.finished.travel_time(1).get(), 2_250.0));
    let t = &run.links;
    let rows: Vec<(u32, u32, u32, u32, u32)> = (0..t.len())
        .map(|r| (t.links()[r].raw(), t.bins()[r].get(), t.cars()[r], t.bikes()[r], t.walkers()[r]))
        .collect();
    assert_eq!(
        rows,
        [(0, 0, 0, 1, 1), (1, 1, 0, 1, 0), (1, 3, 0, 0, 1), (2, 2, 0, 1, 0), (2, 6, 0, 0, 1)]
    );
    assert!(t.first_entry()[0].get().is_nan(), "a row without cars has no first car");
}

#[test]
fn crossings_recorded_out_of_order_come_out_by_link_then_bin() {
    // Links of 1 500 m and 1 000 m, bins of 300 s; the loading records each trip's crossings in
    // trip order. Trip 0, a bike on the second link from 900 s: bin 3 of link 1. Trip 1, a
    // walker on both from 0 s: link 0 at 0 s (bin 0), link 1 at 1 125 s (bin 3). Trip 2, a bike
    // on the second link from 0 s: bin 0 of link 1. Recorded as (1, 3), (0, 0), (1, 3), (1, 0);
    // the table holds (0, 0), (1, 0), (1, 3), the last with one bike and one walker.
    let net = line(&[street(1_500.0), street(1_000.0)]);
    let trips = [
        vehicle(0, Mode::Bike, &[link(1)], 900),
        vehicle(1, Mode::Walk, &whole_line(2), 0),
        vehicle(2, Mode::Bike, &[link(1)], 0),
    ];
    let t = load(&net, &trips, window(300, 3_600)).unwrap().links;
    let rows: Vec<(u32, u32, u32, u32, u32)> = (0..t.len())
        .map(|r| (t.links()[r].raw(), t.bins()[r].get(), t.cars()[r], t.bikes()[r], t.walkers()[r]))
        .collect();
    assert_eq!(rows, [(0, 0, 0, 0, 1), (1, 0, 0, 1, 0), (1, 3, 0, 1, 1)]);
}

#[test]
fn a_bike_on_a_cycle_path_rides_1500_m_in_360_s() {
    let net = line(&[LinkRow { length_m: Some(1_500.0), ..cycle_path() }]);
    let run = load(&net, &[vehicle(0, Mode::Bike, &[link(0)], 0)], window(300, 3_600)).unwrap();
    assert!(close(run.finished.travel_time(0).get(), 360.0));
}

/// The bikes' and walkers' results of a loading, bit by bit: trip, arrival and the counts.
fn active(run: &Loading) -> Vec<u64> {
    let f = &run.finished;
    let mut out: Vec<u64> = (0..f.len())
        .filter(|&i| f.modes()[i] != Mode::Car)
        .flat_map(|i| [u64::from(f.trips()[i].raw()), f.arrivals()[i].get().to_bits()])
        .collect();
    let t = &run.links;
    out.extend((0..t.len()).filter(|&r| t.bikes()[r] + t.walkers()[r] > 0).flat_map(|r| {
        [t.links()[r].raw(), t.bins()[r].get(), t.bikes()[r], t.walkers()[r]].map(u64::from)
    }));
    out
}

#[test]
fn a_bike_is_unchanged_by_1000_cars_queued_on_its_link() {
    // 1 000 cars set out at second 0 on a 1 500 m link of 1 800 veh/h (a 2 s headway): the last
    // leaves at 108 + 999 × 2 = 2 106 s by hand. Bikes and a walker on the same link take
    // 360 s and 1 125 s with and without them, to the bit.
    let net = line(&[car_link(1_500.0, 1_800.0)]);
    let others = [
        vehicle(1_000, Mode::Bike, &[link(0)], 0),
        vehicle(1_001, Mode::Bike, &[link(0)], 100),
        vehicle(1_002, Mode::Walk, &[link(0)], 50),
    ];
    let mut with_cars: Vec<_> = (0..1_000).map(|k| vehicle(k, Mode::Car, &[link(0)], 0)).collect();
    with_cars.extend(others.iter().cloned());
    let w = window(300, 3_600);
    let (queued, alone) = (load(&net, &with_cars, w).unwrap(), load(&net, &others, w).unwrap());
    assert!(close(queued.finished.arrivals()[999].get(), 2_106.0), "the queue is there");
    assert_eq!(active(&queued), active(&alone));
    assert!(close(alone.finished.travel_time(0).get(), 360.0));
    assert!(close(alone.finished.travel_time(2).get(), 1_125.0));
}

#[test]
fn a_bike_still_riding_at_the_end_is_unfinished_on_its_link_and_enters_nothing_after() {
    // The route of 1 500, 1 000 and 500 m in a 600 s window, setting out at 5 s: the bike enters
    // the second link at 365 s and would reach the third at 605 s, after the window's end, so
    // it is unfinished on the second link, entered at 365 s, and the third records nothing.
    let net = line(&[street(1_500.0), street(1_000.0), street(500.0)]);
    let run = load(&net, &[vehicle(0, Mode::Bike, &whole_line(3), 5)], window(300, 600)).unwrap();
    assert!(run.finished.is_empty());
    assert_eq!(run.unfinished.links(), &[link(1)]);
    assert!(close(run.unfinished.entered()[0].get(), 365.0));
    assert!(run.links.links().iter().all(|&l| l != link(2)));
}
