//! The link-time reading for the cars a loading recorded and for bikes and walkers: a recorded
//! car reads its own time on a link and on its whole route, a link nobody entered reads free
//! flow, the exit read never falls as the entry instant grows and is never below free flow.
//! The reading's values for a traveller entering between recorded cars are in
//! `reading_by_crossing.rs`.
//!
//! Every car link here is one lane at 50 km/h: 1 000 m take 72 s by hand.

mod common;

use common::{Draws, car_link, close, line, link, vehicle, whole_line, window};
use openmobisim_loading::link_times::{LinkTimes, Reader};
use openmobisim_loading::point_queue::load;
use openmobisim_types::ids::{EntityId, TripId};
use openmobisim_types::mode::Mode;
use openmobisim_types::units::Duration;

/// 1 000 m at 50 km/h: 72 s.
const TAU: f64 = 72.0;

/// The reading's time on `l` entering at `t`, in seconds: for recorded car `car` (its trip),
/// or for a traveller with no car in the loading.
fn at(times: &LinkTimes, l: u32, t: f64, car: Option<u32>) -> f64 {
    let reader = car.map_or_else(Reader::stranger, |k| Reader::of(TripId::new(k)));
    times.time(link(l), Mode::Car, Duration(t), reader).get()
}

#[test]
fn a_recorded_car_reads_its_own_time_and_an_empty_link_free_flow() {
    // Link 0, 1 200 veh/h (a 3 s headway). Cars enter each second from 0 to 299 s: car k
    // leaves at max(k + 72, x_{k−1} + 3) = 72 + 3k. One more car enters at 650 s and leaves at
    // max(722, 969 + 3) = 972 s, a time of 322 s.
    let net = line(&[car_link(1_000.0, 1_200.0), car_link(1_000.0, 1_200.0)]);
    let mut cars: Vec<_> = (0..300).map(|k| vehicle(k, Mode::Car, &[link(0)], k)).collect();
    cars.push(vehicle(300, Mode::Car, &[link(0)], 650));
    let run = load(&net, &cars, window(300, 3_600)).unwrap();
    let times = LinkTimes::new(&net, &run.links);
    let cases = [
        // (entry, the recorded car reading, time by hand, why)
        (0.0, Some(0), 72.0, "a recorded car: its own time"),
        (299.0, Some(299), 670.0, "a recorded car: its own time"),
        (650.0, Some(300), 322.0, "a recorded car: its own time"),
        (1_000.0, None, 72.0, "past every exit: free flow"),
        (-5.0, None, 72.0, "before the first car: free flow"),
    ];
    for (t, car, by_hand, why) in cases {
        let got = at(&times, 0, t, car);
        assert!(close(got, by_hand), "t = {t}: {got} s; {why}");
    }
    assert!(close(at(&times, 1, 400.0, None), TAU), "a link nobody entered reads free flow");
    // A bike and a walker read free flow whatever the cars did: 1 000 m at 15 km/h is 240 s,
    // at 4.8 km/h 750 s.
    let free = |mode| times.time(link(0), mode, Duration(400.0), Reader::stranger()).get();
    assert!(close(free(Mode::Bike), 240.0));
    assert!(close(free(Mode::Walk), 750.0));
}

#[test]
fn over_random_arrivals_each_car_reads_its_own_time_fifo_and_above_free_flow() {
    // Properties: 800 cars at random seconds in [0, 3 000) into 1 200 veh/h, queueing in
    // places, several in the same second (served in trip order).
    let net = line(&[car_link(1_000.0, 1_200.0)]);
    let mut draws = Draws::new(11);
    let cars: Vec<_> =
        (0..800).map(|k| vehicle(k, Mode::Car, &[link(0)], draws.below(3_000))).collect();
    let run = load(&net, &cars, window(300, 3_600)).unwrap();
    let times = LinkTimes::new(&net, &run.links);
    // Each car, at its own instant, reads the time it took, to the bit.
    let f = &run.finished;
    assert_eq!(f.len(), 800, "every car finishes");
    for i in 0..f.len() {
        let read = at(&times, 0, f.departures()[i].get(), Some(f.trips()[i].raw()));
        assert_eq!(read.to_bits(), f.travel_time(i).get().to_bits(), "trip {}", f.trips()[i]);
    }
    // For a traveller with no car in the loading: first-in-first-out, the exit t + time(t)
    // never falls; and the time is never below 72 s.
    let mut last_exit = f64::NEG_INFINITY;
    for i in -200..16_000 {
        let entry = 0.25 * f64::from(i);
        let time = at(&times, 0, entry, None);
        assert!(time >= TAU, "at {entry} s the reading is {time} s, below free flow");
        assert!(entry + time >= last_exit - 1e-9, "the exit falls at {entry} s");
        last_exit = entry + time;
    }
    assert!(last_exit > 0.0);
}

#[test]
fn a_route_walked_on_the_reading_takes_the_time_each_car_took() {
    // Two 1 000 m links of 1 200 then 1 000 veh/h, a car every 2 s for 600 s. Car k leaves the
    // first at 72 + 3k and the second at 144 + 3.6k (Vickrey's recursion by hand), a time of
    // 144 + 1.6k s. Its route walked on the reading by that car, from its departure, takes
    // the time it was loaded with, to the bit.
    let net = line(&[car_link(1_000.0, 1_200.0), car_link(1_000.0, 1_000.0)]);
    let cars: Vec<_> = (0..300).map(|k| vehicle(k, Mode::Car, &whole_line(2), 2 * k)).collect();
    let run = load(&net, &cars, window(300, 3_600)).unwrap();
    let times = LinkTimes::new(&net, &run.links);
    for i in 0..run.finished.len() {
        let k = u32::try_from(i).unwrap();
        let loaded = run.finished.travel_time(i).get();
        let by_hand = 144.0 + 3.6 * f64::from(k) - 2.0 * f64::from(k);
        assert!((loaded - by_hand).abs() <= 1e-9, "car {i}");
        let departure = Duration(f64::from(2 * k));
        let read = times
            .route_time(&whole_line(2), Mode::Car, departure, Reader::of(TripId::new(k)))
            .get();
        assert_eq!(read.to_bits(), loaded.to_bits(), "car {i}: read {read} s, loaded {loaded} s");
    }
}
