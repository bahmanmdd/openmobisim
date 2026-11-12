//! The link-time reading against the point queue's formula (Vickrey 1969), replayed here: a car
//! entering a link at `t` leaves at `max(t + τ, x + h)`, `x` the exit of the car ahead of it and
//! `h` the headway. The reading must give that for a traveller with no car in the loading
//! entering at any `t`, behind the cars that entered at or before `t`; and for a recorded car at
//! its own instant its own time, cars entering at one instant in trip order; whatever the bins
//! of the table and wherever the window starts. Every expected value below is that formula
//! worked by hand or replayed by the test from the entry times; none comes from the loader or
//! the reading.
//!
//! Every car link here is one lane at 50 km/h: 1 000 m take τ = 72 s by hand.

#![allow(clippy::cast_precision_loss, reason = "a count of cars, far below 2^53")]

mod common;

use common::{Draws, car_link, close, line, link, vehicle};
use openmobisim_loading::link_times::{LinkTimes, Reader};
use openmobisim_loading::point_queue::load;
use openmobisim_types::ids::{EntityId, TripId};
use openmobisim_types::mode::Mode;
use openmobisim_types::time::{Second, StepGrid};
use openmobisim_types::units::Duration;

const TAU: f64 = 72.0;

/// The reading's time on link 0 entering at `t`, in seconds: for recorded car `car` (the cars
/// are numbered in the order of `entries`), or for a traveller with no car in the loading.
fn at(times: &LinkTimes, t: f64, car: Option<u32>) -> f64 {
    let reader = car.map_or_else(Reader::stranger, |k| Reader::of(TripId::new(k)));
    times.time(link(0), Mode::Car, Duration(t), reader).get()
}

/// The reading of cars entering link 0 (capacity `veh_h`) at `entries`, seconds of the clock,
/// over a window from `origin` in bins of `bin` seconds.
fn reading(veh_h: f64, entries: &[u32], origin: u32, bin: u32) -> LinkTimes {
    let net = line(&[car_link(1_000.0, veh_h)]);
    let cars: Vec<_> = entries
        .iter()
        .enumerate()
        .map(|(k, &t)| vehicle(u32::try_from(k).unwrap(), Mode::Car, &[link(0)], t - origin))
        .collect();
    let grid = StepGrid::new(Second(origin), bin, 3_600).expect("a window");
    LinkTimes::new(&net, &load(&net, &cars, grid).unwrap().links)
}

#[test]
fn a_burst_inside_one_bin_reads_the_formula_whatever_the_bin() {
    // 36 veh/h is a headway of 100 s. Cars enter at 300, 301, 302, 303, 304 and 599 s of the
    // clock and leave, by the formula, at 372, 472, 572, 672, 772 and 872 s: times of 72, 171,
    // 270, 369, 468 and 273 s, each read by its own car. A traveller with no car in the
    // loading entering at 450 s is behind the fifth (out at 772 s): it leaves at
    // max(522, 872) = 872 s, 422 s later. The same at any bin length and wherever the window
    // starts.
    let cases = [
        (300, 72.0, Some(0)),
        (301, 171.0, Some(1)),
        (302, 270.0, Some(2)),
        (303, 369.0, Some(3)),
        (304, 468.0, Some(4)),
        (599, 273.0, Some(5)),
        (450, 422.0, None),
    ];
    for (origin, bin) in [(0, 300), (4, 300), (0, 1), (0, 60), (7, 900)] {
        let times = reading(36.0, &[300, 301, 302, 303, 304, 599], origin, bin);
        for (t, by_hand, car) in cases {
            let got = at(&times, f64::from(t - origin), car);
            assert!(close(got, by_hand), "origin {origin}, bin {bin}, t = {t}: {got} s");
        }
    }
}

#[test]
fn the_formula_by_hand_in_a_queue_behind_the_last_car_and_at_free_flow() {
    // 1 200 veh/h (a 3 s headway). Cars enter each second from 0 to 299 s: car k leaves at
    // 72 + 3k. One more enters at 650 s and leaves at max(722, 969 + 3) = 972 s.
    let mut entries: Vec<u32> = (0..300).collect();
    entries.push(650);
    let times = reading(1_200.0, &entries, 0, 300);
    // A case with a car is read by that recorded car; the others by a traveller with no car
    // in the loading.
    let cases = [
        (0.0, Some(0), 72.0, "the first car: nobody ahead"),
        (299.0, Some(299), 670.0, "car 299, behind car 298 (out at 966): max(371, 969) − 299"),
        (149.5, None, 372.5, "behind car 149 (out at 519): max(221.5, 522) − 149.5"),
        (400.0, None, 572.0, "behind car 299 (out at 969): max(472, 972) − 400"),
        (620.0, None, 352.0, "behind car 299: max(692, 972) − 620"),
        (645.0, None, 327.0, "behind car 299: max(717, 972) − 645"),
        (650.0, Some(300), 322.0, "the car at 650 s itself: max(722, 972) − 650"),
        (700.0, None, 275.0, "behind the car of 650 s (out at 972): max(772, 975) − 700"),
        (903.0, None, 72.0, "behind it still: max(975, 975) − 903, the queue just clear"),
        (1_000.0, None, 72.0, "past every exit: free flow"),
        (-5.0, None, 72.0, "before the first car: free flow"),
    ];
    for (t, car, by_hand, why) in cases {
        let got = at(&times, t, car);
        assert!(close(got, by_hand), "t = {t}: {got} s; {why}");
    }
}

#[test]
fn over_random_arrivals_the_reading_is_the_formula_at_every_instant() {
    // 800 cars at random whole seconds in [0, 3 000) into 1 200 veh/h (h = 3 s), queueing in
    // places, several at the same second. The formula replayed here gives every car's exit.
    let (h, n) = (3.0, 800);
    let mut draws = Draws::new(11);
    let mut entries: Vec<u32> = (0..n).map(|_| draws.below(3_000)).collect();
    entries.sort_unstable();
    let times = reading(1_200.0, &entries, 0, 300);
    let mut exits = Vec::new();
    let mut ahead = f64::NEG_INFINITY;
    for &a in &entries {
        ahead = (f64::from(a) + TAU).max(ahead + h);
        exits.push(ahead);
    }
    // Every recorded car at its own instant: its own exit (the cars are numbered in the order
    // they entered, so cars of one second are in trip order).
    for (k, &a) in entries.iter().enumerate() {
        let read = f64::from(a) + at(&times, f64::from(a), Some(u32::try_from(k).unwrap()));
        assert!(close(read, exits[k]), "car {k}: {read} against {}", exits[k]);
    }
    // A traveller with no car in the loading entering at `t`: behind the cars that entered at
    // or before `t`.
    let formula = |t: f64| {
        let ahead = entries.partition_point(|&a| f64::from(a) <= t);
        ahead.checked_sub(1).map_or(t + TAU, |k| (t + TAU).max(exits[k] + h))
    };
    // At every sixteenth of a second (the seconds cars entered included).
    let mut last_exit = f64::NEG_INFINITY;
    for i in -800..64_000 {
        let t = 0.062_5 * f64::from(i);
        let read = t + at(&times, t, None);
        assert!(close(read, formula(t)), "at {t} s: {read} against {}", formula(t));
        assert!(read - t >= TAU - 1e-9, "at {t} s the reading is below free flow");
        assert!(read >= last_exit - 1e-9, "the exit falls at {t} s");
        last_exit = read;
    }
}

#[test]
fn cars_entering_at_one_instant_read_each_its_own_time() {
    // A headway of 100 s. Three cars enter at 5 s and leave, in trip order, at 77, 177 and
    // 277 s: times of 72, 172 and 272 s, each read by its own car. A traveller with no car in
    // the loading entering at 5 s is behind all three: max(77, 377) − 5 = 372 s; at 5.3 s
    // the same: max(77.3, 377) − 5.3 = 371.7 s. A fourth car enters at 20 s and leaves at
    // 377 s: 357 s.
    let times = reading(36.0, &[5, 5, 5, 20], 0, 300);
    for (car, by_hand) in [(0, 72.0), (1, 172.0), (2, 272.0)] {
        let got = at(&times, 5.0, Some(car));
        assert!(close(got, by_hand), "car {car} of the three: {got}");
    }
    assert!(close(at(&times, 5.0, None), 372.0), "behind the three: {}", at(&times, 5.0, None));
    assert!(close(at(&times, 4.9, None), 72.0), "before the first car: free flow");
    assert!(close(at(&times, 5.3, None), 371.7), "{}", at(&times, 5.3, None));
    assert!(close(at(&times, 20.0, Some(3)), 357.0), "the fourth's own time");
}
