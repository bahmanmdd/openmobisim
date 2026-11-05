//! The first-in-first-out reading against hand values of its rules, and its properties: it
//! equals a recorded entrant's own time, it is linear between two entrants, the exit it reads
//! never falls as the entry instant grows, it is never below free flow, and a route walked on it
//! takes the time a car took on it.
//!
//! Every car link here is one lane at 50 km/h: 1 000 m take 72 s by hand.

mod common;

use common::{Draws, car_link, close, line, link, vehicle, whole_line, window};
use openmobisim_loading::link_times::LinkTimes;
use openmobisim_loading::point_queue::load;
use openmobisim_types::mode::Mode;
use openmobisim_types::time::Second;
use openmobisim_types::units::Duration;

/// 1 000 m at 50 km/h: 72 s.
const TAU: f64 = 72.0;

/// The reading's time on `l` entering at `t`, in seconds.
fn at(times: &LinkTimes, l: u32, t: f64) -> f64 {
    times.time(link(l), Mode::Car, Duration(t)).get()
}

#[test]
fn each_rule_of_the_reading_gives_its_value_by_hand() {
    // Link 0, 1 200 veh/h (a 3 s headway), 300 s bins. Cars enter each second from 0 to 299 s:
    // car k leaves at max(k + 72, x_{k−1} + 3) = 72 + 3k, so bin 0's first entrant is (0, 72)
    // and its last (299, 969). Nobody enters in bin 1. One car enters at 650 s (bin 2) and
    // leaves at max(722, 969 + 3) = 972 s, a time of 322 s. By the rules of the reading:
    let net = line(&[car_link(1_000.0, 1_200.0), car_link(1_000.0, 1_200.0)]);
    let mut cars: Vec<_> = (0..300).map(|k| vehicle(k, Mode::Car, &[link(0)], k)).collect();
    cars.push(vehicle(300, Mode::Car, &[link(0)], 650));
    let run = load(&net, &cars, window(300, 3_600)).unwrap();
    let times = LinkTimes::new(&net, &run.links);
    let cases = [
        // (entry, time by hand, the rule)
        (0.0, 72.0, "a recorded entrant: its own time"),
        (299.0, 670.0, "a recorded entrant: its own time"),
        (149.5, 371.0, "between a bin's first and last entrant: on their line, 72 + 598 / 2"),
        (400.0, 569.0, "after the last entrant, the next bin empty: max(969 − 400, 72)"),
        (620.0, 349.0, "before the first entrant, the bin before empty: max(969 − 620, 322)"),
        (645.0, 324.0, "the same rule: max(969 − 645, 322)"),
        (650.0, 322.0, "a recorded entrant: its own time"),
        (700.0, 272.0, "after the last entrant, the next bin empty: max(972 − 700, 72)"),
        (1_000.0, 72.0, "past every exit: free flow"),
        (-5.0, 72.0, "before the first recorded bin: free flow"),
    ];
    for (t, by_hand, rule) in cases {
        assert!(close(at(&times, 0, t), by_hand), "t = {t}: {} s; {rule}", at(&times, 0, t));
    }
    assert!(close(at(&times, 1, 400.0), TAU), "a link nobody entered reads free flow");
    // A bike and a walker read free flow whatever the cars did: 1 000 m at 15 km/h is 240 s,
    // at 4.8 km/h 750 s.
    assert!(close(times.time(link(0), Mode::Bike, Duration(400.0)).get(), 240.0));
    assert!(close(times.time(link(0), Mode::Walk, Duration(400.0)).get(), 750.0));
}

#[test]
fn across_a_bin_boundary_the_reading_follows_the_line_between_the_two_entrants() {
    // A car every 2 s into 1 200 veh/h: car 149 is bin 0's last entrant, (298, 72 + 447 = 519),
    // car 150 bin 1's first, (300, 522). At 299 s the line through them reads an exit of 520.5,
    // a time of 221.5 s by hand.
    let net = line(&[car_link(1_000.0, 1_200.0)]);
    let cars: Vec<_> = (0..300).map(|k| vehicle(k, Mode::Car, &[link(0)], 2 * k)).collect();
    let run = load(&net, &cars, window(300, 3_600)).unwrap();
    let times = LinkTimes::new(&net, &run.links);
    assert!(close(at(&times, 0, 299.0), 221.5), "{}", at(&times, 0, 299.0));
}

#[test]
fn over_random_arrivals_the_reading_is_its_entrants_own_time_linear_fifo_and_above_free_flow() {
    // Properties: 800 cars at random seconds in [0, 3 000) into 1 200 veh/h, queueing in places.
    let net = line(&[car_link(1_000.0, 1_200.0)]);
    let mut draws = Draws::new(11);
    let cars: Vec<_> =
        (0..800).map(|k| vehicle(k, Mode::Car, &[link(0)], draws.below(3_000))).collect();
    let run = load(&net, &cars, window(300, 3_600)).unwrap();
    let times = LinkTimes::new(&net, &run.links);
    let t = &run.links;
    for r in 0..t.len() {
        // At a recorded entrant, the entrant's own time.
        for (entry, exit) in
            [(t.first_entry()[r], t.first_exit()[r]), (t.last_entry()[r], t.last_exit()[r])]
        {
            let own = exit.get() - entry.get();
            assert!(close(at(&times, 0, entry.get()), own), "row {r}");
        }
        // Halfway between a bin's first and last entrant, halfway between their exits.
        let mid = 0.5 * (t.first_entry()[r].get() + t.last_entry()[r].get());
        let exit = 0.5 * (t.first_exit()[r].get() + t.last_exit()[r].get());
        assert!((mid + at(&times, 0, mid) - exit).abs() <= 1e-9, "row {r}");
    }
    // First-in-first-out: the exit t + time(t) never falls; and the time is never below 72 s.
    let mut last_exit = f64::NEG_INFINITY;
    for i in -200..16_000 {
        let entry = 0.25 * f64::from(i);
        let time = at(&times, 0, entry);
        assert!(time >= TAU, "at {entry} s the reading is {time} s, below free flow");
        assert!(entry + time >= last_exit - 1e-9, "the exit falls at {entry} s");
        last_exit = entry + time;
    }
    assert!(last_exit > 0.0);
}

#[test]
fn a_route_walked_on_the_reading_takes_the_time_each_car_took() {
    // Two 1 000 m links of 1 200 then 1 000 veh/h, a car every 2 s for 600 s. Car k leaves the
    // first at 72 + 3k and the second at 144 + 3.6k (Vickrey's recursion by hand): both exits
    // are straight lines in the entry, so the reading's straight lines through the recorded
    // entrants are exact, and the route walked on it at each car's departure takes that car's
    // loaded time, within rounding.
    let net = line(&[car_link(1_000.0, 1_200.0), car_link(1_000.0, 1_000.0)]);
    let cars: Vec<_> = (0..300).map(|k| vehicle(k, Mode::Car, &whole_line(2), 2 * k)).collect();
    let run = load(&net, &cars, window(300, 3_600)).unwrap();
    let times = LinkTimes::new(&net, &run.links);
    for i in 0..run.finished.len() {
        let loaded = run.finished.travel_time(i).get();
        let by_hand = 144.0 + 3.6 * f64::from(u32::try_from(i).unwrap())
            - 2.0 * f64::from(u32::try_from(i).unwrap());
        assert!((loaded - by_hand).abs() <= 1e-9, "car {i}");
        let read = times
            .route_time(&whole_line(2), Mode::Car, Second(2 * u32::try_from(i).unwrap()))
            .get();
        assert!((read - loaded).abs() <= 1e-6, "car {i}: read {read} s, loaded {loaded} s");
    }
}
