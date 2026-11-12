//! The point queue against answers from outside the simulator: Vickrey's (1969) deterministic
//! bottleneck, the free-flow time with a capacity that never binds, Newell's (1982) tandem of
//! two bottlenecks, the merge of two approaches (test 2), the window's end, determinism, and the errors that refuse bad vehicles.
//!
//! Every car link here is one lane at 50 km/h (13.889 m/s): 1 000 m take 72 s and 2 000 m take
//! 144 s by hand, both exact in floating point. Capacities are given in the rows.

mod common;

use common::{
    Draws, VALUES, at, car_link, cycle_path, grid, grid_route, line, link, vehicle, whole_line,
    window,
};
use openmobisim_graph::network::{LinkRow, Network, NetworkBuilder};
use openmobisim_loading::input::{LoadError, Vehicle};
use openmobisim_loading::point_queue::{Loading, load};
use openmobisim_types::ids::{EntityId, LinkId, TripId};
use openmobisim_types::mode::Mode;
use openmobisim_types::time::{Second, StepGrid};
use openmobisim_types::units::Duration;

/// 1 000 m at 50 km/h: 1 000 / (50 000 / 3 600) = 72 s.
const TAU_1KM: f64 = 72.0;
/// 2 000 m at 50 km/h: 144 s.
const TAU_2KM: f64 = 144.0;

/// A car on link 0 every 2 s from second 0: car `k` departs at `2k`.
fn every_two_seconds(cars: u32) -> Vec<Vehicle> {
    (0..cars).map(|k| vehicle(k, Mode::Car, &[link(0)], 2 * k)).collect()
}

/// The total car delay on `l` in veh·s: the table's sum of car crossing times on it, less its
/// cars times `tau`.
fn delay_on(run: &Loading, l: LinkId, tau: f64) -> f64 {
    let t = &run.links;
    (0..t.len())
        .filter(|&r| t.links()[r] == l)
        .map(|r| t.car_time()[r].get() - f64::from(t.cars()[r]) * tau)
        .sum()
}

#[test]
fn a_bottleneck_of_1200_veh_h_fed_a_car_every_2_s_delays_car_k_by_k_seconds() {
    // Vickrey (1969), by hand: arrivals λ = 0.5 veh/s for T = 600 s at a bottleneck passing
    // μ = 1 200 veh/h = 1/3 veh/s, a headway of 3 s. Car k (0 to 299) enters at 2k and leaves at
    // max(2k + τ, x_{k−1} + 3) = τ + 3k: a delay of k s, which is (λ − μ) t / μ = t / 2 at
    // t = 2k. The total delay is Σ k = 299 × 300 / 2 = 44 850 veh·s.
    let net = line(&[car_link(1_000.0, 1_200.0)]);
    let run = load(&net, &every_two_seconds(300), window(300, 3_600)).unwrap();
    assert!(run.unfinished.is_empty());
    let (lambda, mu) = (0.5, 1.0 / 3.0);
    let mut worst: f64 = 0.0;
    for k in 0..300_u32 {
        let (i, t) = (k as usize, 2.0 * f64::from(k));
        let exit = run.finished.arrivals()[i].get();
        worst = worst.max((exit - (TAU_1KM + 3.0 * f64::from(k))).abs());
        let delay = run.finished.travel_time(i).get() - TAU_1KM;
        worst = worst.max((delay - (lambda - mu) * t / mu).abs());
    }
    // Test 1 allows 1 s per car; the point queue is exact to rounding.
    assert!(worst <= 1e-9, "the worst car is {worst} s from Vickrey's");
    let total = delay_on(&run, link(0), TAU_1KM);
    assert!((total - 44_850.0).abs() <= 1e-6, "total delay {total} veh·s, by hand 44 850");
}

#[test]
fn the_bottleneck_queue_clears_at_lambda_t_over_mu_900_s() {
    // Vickrey (1969): the queue built by λ = 0.5 veh/s for T = 600 s at μ = 1/3 veh/s clears at
    // λT/μ = 900 s: a car entering at 900 s waits nothing, one entering at 899 s still waits 1 s
    // (the last of the 300 leaves at τ + 897 s, and the next may leave 3 s later).
    let net = line(&[car_link(1_000.0, 1_200.0)]);
    for (probe, wait) in [(899, 1.0), (900, 0.0)] {
        let mut cars = every_two_seconds(300);
        cars.push(vehicle(300, Mode::Car, &[link(0)], probe));
        let run = load(&net, &cars, window(300, 3_600)).unwrap();
        let delay = run.finished.travel_time(300).get() - TAU_1KM;
        assert!((delay - wait).abs() <= 1e-9, "a car entering at {probe} s waits {delay} s");
    }
}

#[test]
fn with_unlimited_capacity_every_exit_is_exactly_entry_plus_free_flow() {
    // The meanings table: the point queue with unlimited capacity reproduces the free-flow time
    // exactly. The row gives the capacity as infinity, a headway of 1 / ∞ = 0 s: every exit is
    // t + τ to the bit and every door-to-door time is 1 000 m / 13.889 m/s = 72 s, even for two
    // cars setting out at the same second (cars 300 and 301, at 1 000 s).
    let net = line(&[car_link(1_000.0, f64::INFINITY)]);
    let free = net.free_flow_time(link(0), Mode::Car);
    assert_eq!(free.get().to_bits(), (1_000.0 / (50_000.0 / 3_600.0_f64)).to_bits());
    let mut cars = every_two_seconds(300);
    cars.extend([300, 301].map(|trip| vehicle(trip, Mode::Car, &[link(0)], 1_000)));
    let run = load(&net, &cars, window(300, 3_600)).unwrap();
    assert_eq!(run.finished.len(), 302);
    for (i, v) in cars.iter().enumerate() {
        let t = v.departure.get();
        assert_eq!(run.finished.arrivals()[i].get().to_bits(), (t + free.get()).to_bits());
        assert_eq!(run.finished.travel_time(i).get().to_bits(), free.get().to_bits());
    }
    assert!(delay_on(&run, link(0), TAU_1KM).abs() <= 1e-9);
}

/// Newell's (1982) tandem: 2 km links of `first` then `second` veh/h, 2 000 veh/h for 30 min
/// (1 000 cars, car k departing at ⌊1.8 k⌋ s, whole seconds). The run, the departures' sum.
fn tandem(first: f64, second: f64) -> (Loading, f64) {
    let net = line(&[car_link(2_000.0, first), car_link(2_000.0, second)]);
    let cars: Vec<Vehicle> =
        (0..1_000).map(|k| vehicle(k, Mode::Car, &whole_line(2), 18 * k / 10)).collect();
    let departures = cars.iter().map(|v| v.departure.get()).sum();
    let run = load(&net, &cars, window(300, 7_200)).unwrap();
    assert!(run.unfinished.is_empty(), "every car arrives within two hours");
    (run, departures)
}

/// Whether `measured` is within a share `share` of `expected`.
fn within(measured: f64, expected: f64, share: f64) -> bool {
    (measured - expected).abs() <= share * expected.abs()
}

#[test]
fn a_tandem_of_1500_then_1000_veh_h_delays_83_3_then_166_7_vehicle_hours() {
    // Newell (1982), the fluid tandem: the first queue grows at λ − μ1 = 500 veh/h for 0.5 h and
    // clears at λT/μ1 = 2 400 s: ½ × (500/3 600) × 1 800 × 2 400 = 300 000 veh·s = 83.3 veh·h.
    // The second is fed at μ1 = 1 500 veh/h for 2 400 s and clears 3 600 s after it began:
    // ½ × (500/3 600) × 2 400 × 3 600 = 600 000 veh·s = 166.7 veh·h.
    // Vickrey's recursion by hand, car by car: the first queue is busy from car 0 (each gap
    // ≤ 2 s < 2.4 s), so car k leaves it at τ + 2.4k, a delay of 2.4k − d_k; the second (3.6 s
    // headway) at 2τ + 3.6k, a delay of 1.2k. In all 2.4 × 499 500 − Σ d_k and 1.2 × 499 500.
    let (run, departures) = tandem(1_500.0, 1_000.0);
    let (first, second) = (delay_on(&run, link(0), TAU_2KM), delay_on(&run, link(1), TAU_2KM));
    let sum_k = 499_500.0;
    assert!((first - (2.4 * sum_k - departures)).abs() <= 1e-3, "first queue {first} veh·s");
    assert!((second - 1.2 * sum_k).abs() <= 1e-3, "second queue {second} veh·s");
    // The fluid answer within 0.25 %: a discrete queue differs from it by about one car's share.
    assert!(within(first / 3_600.0, 300_000.0 / 3_600.0, 0.002_5), "{} veh·h", first / 3_600.0);
    assert!(within(second / 3_600.0, 600_000.0 / 3_600.0, 0.002_5), "{} veh·h", second / 3_600.0);
    for k in 0..1_000_u32 {
        let by_hand = 2.0 * TAU_2KM + 3.6 * f64::from(k);
        assert!((run.finished.arrivals()[k as usize].get() - by_hand).abs() <= 1.0);
    }
}

#[test]
fn the_reversed_tandem_delays_250_vehicle_hours_then_none() {
    // Newell (1982): 1 000 veh/h first holds the whole queue, ½ × (1 000/3 600) × 1 800 × 3 600 =
    // 900 000 veh·s = 250 veh·h, and feeds the second at 1 000 veh/h, below its 1 500: no delay.
    // By hand: car k leaves the first at τ + 3.6k (a delay of 3.6k − d_k); the second passes
    // cars 3.6 s apart with a 2.4 s headway, so none waits.
    let (run, departures) = tandem(1_000.0, 1_500.0);
    let (first, second) = (delay_on(&run, link(0), TAU_2KM), delay_on(&run, link(1), TAU_2KM));
    assert!((first - (3.6 * 499_500.0 - departures)).abs() <= 1e-3, "first queue {first} veh·s");
    assert!(second.abs() <= 1e-6, "second queue {second} veh·s");
    assert!(within(first / 3_600.0, 250.0, 0.002_5), "{} veh·h", first / 3_600.0);
}

/// Two 1 000 m approaches, `l0` from `a` and `l1` from `b`, merging at `c` into `l2`, 1 000 m on
/// to `d`; every link 1 800 veh/h. The network and the ids of `l0`, `l1` and `l2`.
fn merge() -> (Network, [LinkId; 3]) {
    let mut b = NetworkBuilder::new();
    for (node, x, y) in
        [("a", 0.0, 0.0), ("b", 0.0, 1_000.0), ("c", 1_000.0, 0.0), ("d", 2_000.0, 0.0)]
    {
        b.add_node(node, at(x, y));
    }
    for (id, from, to) in [("l0", "a", "c"), ("l1", "b", "c"), ("l2", "c", "d")] {
        b.add_link(id, from, to, car_link(1_000.0, 1_800.0));
    }
    let net = b.build(VALUES).expect("the merge builds");
    let ids = ["l0", "l1", "l2"].map(|id| net.link_ids().typed_id_of(id).expect("a merge link"));
    (net, ids)
}

#[test]
fn a_merge_of_1800_and_600_veh_h_into_1800_veh_h_queues_both_approaches_alike() {
    // Daganzo's (1995) merge with priorities in proportion to demand is the deterministic queue
    // on the merged link (Vickrey 1969). Approach A sends a car every 2 s (1 800 veh/h, exactly
    // its capacity, so it never waits on l0) and approach B one every 6 s (600 veh/h), both for
    // T = 900 s: λ = 2 400 veh/h = 2/3 veh/s into l2 against μ = 1 800 veh/h = 1/2 veh/s. The
    // fluid queue peaks at (λ − μ)T = 150 cars; the total delay is λ(λ − μ)T²/(2μ) = 90 000 veh·s
    // = 25 veh·h, a mean of (λ − μ)T/(2μ) = 150 s on each approach, at most 300 s.
    // Vickrey's recursion by hand, car by car: cars reach l2 at their departure + 72 s, in the
    // order of (instant, trip), so at a tie the A car (the lower trip) goes first. l2 is busy
    // from its first car, so the n-th car to reach it (from 0) leaves at 144 + 2n. A's car k
    // follows k A cars and ⌈k/3⌉ B cars: it arrives at 144 + 2(k + ⌈k/3⌉), a delay of 2⌈k/3⌉;
    // Σ⌈k/3⌉ = 33 825 over k < 450, so 67 650 veh·s, a mean of 150.33 s, at most 300 s (k = 449).
    // B's car j follows 3j + 1 A cars (k ≤ 3j, the tie included) and j B cars: it arrives at
    // 144 + 2(4j + 1), a delay of 2j + 2; 22 650 veh·s over 150 cars, a mean of 151 s. In all
    // 90 300 veh·s, within 0.5 % (one car's share) of the fluid 90 000. Under priorities in
    // proportion to capacity (Daganzo's usual choice) B would pass free and A bear all 25 veh·h:
    // the unequal demands are what tell the two rules apart.
    let (net, [l0, l1, l2]) = merge();
    let mut cars: Vec<Vehicle> =
        (0..450).map(|k| vehicle(k, Mode::Car, &[l0, l2], 2 * k)).collect();
    cars.extend((0..150).map(|j| vehicle(450 + j, Mode::Car, &[l1, l2], 6 * j)));
    let run = load(&net, &cars, window(300, 3_600)).unwrap();
    assert!(run.unfinished.is_empty());
    for trip in 0..600_u32 {
        let by_hand = if trip < 450 {
            144.0 + 2.0 * f64::from(trip + trip.div_ceil(3))
        } else {
            144.0 + 2.0 * f64::from(4 * (trip - 450) + 1)
        };
        let arrival = run.finished.arrivals()[trip as usize].get();
        assert!(
            (arrival - by_hand).abs() <= 1e-9,
            "trip {trip} at {arrival} s, by hand {by_hand} s"
        );
    }
    for (name, l) in [("l0", l0), ("l1", l1)] {
        let delay = delay_on(&run, l, TAU_1KM);
        assert!(delay <= 1e-6, "delay on {name} {delay} veh·s");
    }
    let merged = delay_on(&run, l2, TAU_1KM);
    assert!((merged - 90_300.0).abs() <= 1e-3, "delay on l2 {merged} veh·s, by hand 90 300");
    assert!(within(merged, 90_000.0, 0.005), "delay on l2 {merged} veh·s, fluid 90 000");
    let delays: Vec<f64> =
        (0..600).map(|i| run.finished.travel_time(i).get() - 2.0 * TAU_1KM).collect();
    let (a, b) = (delays[..450].iter().sum::<f64>(), delays[450..].iter().sum::<f64>());
    assert!((a - 67_650.0).abs() <= 1e-3, "approach A {a} veh·s, by hand 67 650");
    assert!((b - 22_650.0).abs() <= 1e-3, "approach B {b} veh·s, by hand 22 650");
    let (mean_a, mean_b) = (a / 450.0, b / 150.0);
    assert!((mean_a - 150.0).abs() <= 1.0, "approach A's mean delay {mean_a} s, fluid 150 s");
    assert!((mean_b - 150.0).abs() <= 1.0, "approach B's mean delay {mean_b} s, fluid 150 s");
    let worst = delays[..450].iter().copied().fold(0.0, f64::max);
    assert!((worst - 300.0).abs() <= 1e-9, "approach A's longest delay {worst} s, by hand 300 s");
    // l2 discharges at capacity: the 600 arrivals at d, 2 s apart, from 144 s to 1 342 s.
    let mut at_d: Vec<f64> = run.finished.arrivals().iter().map(|a| a.get()).collect();
    at_d.sort_by(f64::total_cmp);
    assert!((at_d[0] - 144.0).abs() <= 1e-9 && (at_d[599] - 1_342.0).abs() <= 1e-9);
    for w in at_d.windows(2) {
        assert!((w[1] - w[0] - 2.0).abs() <= 1e-9, "arrivals at {} s and {} s", w[0], w[1]);
    }
}

/// The table's rows on link 0 as (bin, cars, mean car time in s).
fn bins_of(run: &Loading) -> Vec<(u32, u32, f64)> {
    let t = &run.links;
    (0..t.len())
        .map(|r| (t.bins()[r].get(), t.cars()[r], t.car_time()[r].get() / f64::from(t.cars()[r])))
        .collect()
}

#[test]
fn a_queue_outlasting_the_window_is_recorded_at_its_full_length() {
    // A car every 2 s for 3 600 s into 1 200 veh/h: car k enters at t = 2k and takes τ + k =
    // τ + t/2 (Vickrey). In 300 s bins, bin b holds cars 150b to 150b + 149, of mean time
    // τ + 150b + 74.5 s by hand. With a window of 3 600 s the last cars leave long after it
    // (car 1 799 at τ + 5 397 s); each bin, the last included, reads the same as with a window
    // of 14 400 s, in which every car arrives: no crossing is filed short.
    let net = line(&[car_link(1_000.0, 1_200.0)]);
    let cars = every_two_seconds(1_800);
    let short = load(&net, &cars, window(300, 3_600)).unwrap();
    let long = load(&net, &cars, window(300, 14_400)).unwrap();
    assert!(long.unfinished.is_empty());
    let (s, l) = (bins_of(&short), bins_of(&long));
    assert_eq!(s.len(), 12);
    for (b, (&(bin, cars, mean), &(long_bin, long_cars, long_mean))) in s.iter().zip(&l).enumerate()
    {
        assert_eq!((bin, cars), (long_bin, long_cars));
        assert_eq!(mean.to_bits(), long_mean.to_bits(), "bin {bin}");
        let by_hand = TAU_1KM + 150.0 * f64::from(u32::try_from(b).unwrap()) + 74.5;
        assert!((mean - by_hand).abs() <= 1e-9, "bin {bin}: {mean} s, by hand {by_hand} s");
        assert_eq!(cars, 150);
    }
    // Car k is unfinished when τ + 3k ≥ 3 600, k ≥ 1 176: 624 cars, each on link 0, entered at 2k.
    assert_eq!(short.unfinished.len(), 624);
    assert_eq!(short.finished.len(), 1_176);
    for (i, &trip) in short.unfinished.trips().iter().enumerate() {
        assert_eq!(short.unfinished.links()[i], link(0));
        let entered = short.unfinished.entered()[i].get();
        assert_eq!(entered.to_bits(), (2.0 * f64::from(trip.raw())).to_bits());
    }
}

#[test]
fn an_arrival_at_the_window_end_is_unfinished_and_nothing_enters_at_the_end() {
    // The window is half-open: a car arriving at its end is not finished. Two 1 000 m links,
    // 72 s each, window 144 s: a car departing at 0 reaches the second link's end at 144 s, and
    // one departing at 72 s reaches the first link's end at 144 s, and never enters the second.
    let net = line(&[car_link(1_000.0, f64::INFINITY), car_link(1_000.0, f64::INFINITY)]);
    let cars =
        [vehicle(0, Mode::Car, &whole_line(2), 0), vehicle(1, Mode::Car, &whole_line(2), 72)];
    let run = load(&net, &cars, window(72, 144)).unwrap();
    assert!(run.finished.is_empty());
    assert_eq!(run.unfinished.links(), &[link(1), link(0)]);
    let rows: Vec<(LinkId, u32, u32)> = (0..run.links.len())
        .map(|r| (run.links.links()[r], run.links.bins()[r].get(), run.links.cars()[r]))
        .collect();
    assert_eq!(rows, [(link(0), 0, 1), (link(0), 1, 1), (link(1), 1, 1)]);
}

/// Every value of a loading, bit by bit, in a fixed order.
fn fingerprint(run: &Loading) -> Vec<u64> {
    let f = &run.finished;
    let u = &run.unfinished;
    let t = &run.links;
    let mut out = Vec::new();
    out.extend(f.trips().iter().map(|x| u64::from(x.raw())));
    out.extend(f.modes().iter().map(|m| u64::from(m.as_u8())));
    out.extend(f.departures().iter().map(|s| s.get().to_bits()));
    out.extend(f.arrivals().iter().map(|a| a.get().to_bits()));
    out.extend(u.trips().iter().map(|x| u64::from(x.raw())));
    out.extend(u.links().iter().map(|x| u64::from(x.raw())));
    out.extend(u.entered().iter().map(|a| a.get().to_bits()));
    out.extend(t.links().iter().map(|x| u64::from(x.raw())));
    out.extend(t.bins().iter().map(|b| u64::from(b.get())));
    out.extend(t.cars().iter().chain(t.bikes()).chain(t.walkers()).map(|&c| u64::from(c)));
    for column in [t.car_time(), t.first_entry(), t.first_exit(), t.last_entry(), t.last_exit()] {
        out.extend(column.iter().map(|d| d.get().to_bits()));
    }
    out
}

/// Random cars, bikes and walkers on rows, columns and L-shaped routes of a 6 × 6 grid.
fn mixed_traffic(net: &Network, draws: &mut Draws) -> Vec<Vehicle> {
    (0..1_500)
        .map(|trip| {
            let (r, c) = (draws.below(6), draws.below(6));
            let mut nodes: Vec<(u32, u32)> = (0..6).map(|x| (r, x)).collect();
            nodes.extend((r + 1..6).map(|y| (y, 5)));
            if c % 2 == 1 {
                nodes = (0..6).map(|y| (y, c)).collect();
            }
            let mode =
                [Mode::Car, Mode::Car, Mode::Car, Mode::Bike, Mode::Walk][draws.below(5) as usize];
            vehicle(trip, mode, &grid_route(net, &nodes), draws.below(1_500))
        })
        .collect()
}

#[test]
fn a_loading_is_the_same_over_two_runs_and_under_any_input_order() {
    // The one self-comparison the rules allow: determinism. 1 500 random trips, some still
    // travelling at the end of a 1 800 s window; the same loading, bit by bit, run twice and
    // with the vehicles shuffled. Nothing in the loading is parallel, so there is no thread
    // count to vary.
    let net = grid(6);
    let mut draws = Draws::new(7);
    let mut vehicles = mixed_traffic(&net, &mut draws);
    let w: StepGrid = window(300, 1_800);
    let first = load(&net, &vehicles, w).unwrap();
    let again = load(&net, &vehicles, w).unwrap();
    assert!(!first.unfinished.is_empty() && !first.finished.is_empty());
    assert_eq!(fingerprint(&first), fingerprint(&again));
    for i in (1..vehicles.len()).rev() {
        let j = draws.below(u32::try_from(i + 1).unwrap()) as usize;
        vehicles.swap(i, j);
    }
    assert_eq!(fingerprint(&first), fingerprint(&load(&net, &vehicles, w).unwrap()));
}

#[test]
fn each_refused_vehicle_names_its_trip() {
    let net = line(&[
        car_link(1_000.0, 1_200.0),
        car_link(1_000.0, 1_200.0),
        LinkRow { length_m: Some(10.0), ..cycle_path() },
    ]);
    // A window of 1 000 s from second 100 of the clock; departures are counted from its start.
    let w = StepGrid::new(Second(100), 100, 1_000).unwrap();
    let ok = vehicle(1, Mode::Car, &whole_line(2), 0);
    let refuse = |bad: Vehicle| load(&net, &[ok.clone(), bad], w).unwrap_err();
    let trip = TripId::new(9);
    let departing =
        |at: f64| Vehicle { departure: Duration(at), ..vehicle(9, Mode::Car, &[link(0)], 0) };
    assert_eq!(
        refuse(departing(-1.0)),
        LoadError::DepartureOutsideWindow {
            trip,
            departure: Duration(-1.0),
            length: Duration(1_000.0)
        }
    );
    assert!(matches!(refuse(departing(1_000.0)), LoadError::DepartureOutsideWindow { .. }));
    assert_eq!(refuse(vehicle(9, Mode::Bike, &[], 100)), LoadError::EmptyRoute(trip));
    assert_eq!(refuse(vehicle(9, Mode::Transit, &[link(0)], 100)), LoadError::Transit(trip));
    assert_eq!(
        refuse(vehicle(9, Mode::Car, &[link(1), link(0)], 100)),
        LoadError::NotConsecutive { trip, from: link(1), to: link(0) }
    );
    assert_eq!(
        refuse(vehicle(9, Mode::Car, &whole_line(3), 100)),
        LoadError::ModeNotAllowed { trip, link: link(2), mode: Mode::Car }
    );
    assert_eq!(
        refuse(vehicle(9, Mode::Walk, &[link(7)], 100)),
        LoadError::UnknownLink { trip, link: link(7) }
    );
    assert_eq!(
        refuse(vehicle(1, Mode::Bike, &[link(2)], 100)),
        LoadError::DuplicateTrip(TripId::new(1))
    );
    // The same faults in any order: the first in trip order is named.
    let (a, b) = (vehicle(4, Mode::Car, &[], 100), vehicle(3, Mode::Car, &[link(2)], 100));
    let named = LoadError::ModeNotAllowed { trip: TripId::new(3), link: link(2), mode: Mode::Car };
    assert_eq!(load(&net, &[a.clone(), b.clone()], w).unwrap_err(), named);
    assert_eq!(load(&net, &[b, a], w).unwrap_err(), named);
    assert!(refuse(departing(-1.0)).to_string().contains("trip 9"));
}
