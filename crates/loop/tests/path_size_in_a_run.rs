//! The path size of every car route in every traveller's set at the end of a run, against the
//! formula of Ben-Akiva and Bierlaire (1999) by hand: `PS_i = Σ_a (l_a / L_i) / n_a`, over the
//! links `a` of route `i` of length `L_i`, `n_a` the car routes of the set using `a`. Nothing
//! else is judged: no share, no route proportion, no convergence.
//!
//! Nodes O, J, D; car links at 36 km/h (10 m/s): `approach` O→J 600 m, unlimited; three links
//! J→D: `short` 200 m at 1 200 veh/h, `middle` 400 m at 600 veh/h, `long` 600 m unlimited. The
//! routes are 800 m (80 s), 1 000 m (100 s) and 1 200 m (120 s) at free flow: no tie. 1 800
//! car-only travellers O→D, one a second from second 0, a window of 4 h (14 400 s), the
//! time-only least-cost choice, 4 iterations. The queues are only the means by which the search
//! finds the second and the third route; they are not judged. A route is told by its last link.
//!
//! The window by hand: at worst all 1 800 cars take `middle`, the link of the longest headway,
//! one every 6 s: the first enters it at 60 s and leaves at 100 s, car `k` (departing at `k` s)
//! leaves it no later than `100 + 6 k`, the last at `100 + 6 × 1 799 = 10 894` s, below
//! 14 400 s. So every trip
//! finishes at every iteration whatever the choices (a guard: no iteration has an unfinished
//! trip).
//!
//! By hand, the shared link `approach` (600 m) used by every car route of the set:
//! - three routes in the set: short `600/800/3 + 200/800 = 0.25 + 0.25 = 0.5`; middle
//!   `600/1000/3 + 400/1000 = 0.2 + 0.4 = 0.6`; long `600/1200/3 + 600/1200 = 1/6 + 1/2 = 2/3`;
//! - two routes, whichever two: short `600/800/2 + 200/800 = 0.375 + 0.25 = 0.625`; middle
//!   `600/1000/2 + 400/1000 = 0.3 + 0.4 = 0.7`; long `600/1200/2 + 600/1200 = 0.25 + 0.5 = 0.75`;
//! - one route: 1.
//!
//! Each car route's `ln PS` equals the logarithm of its hand value to 1e-12. A guard, not an
//! expected value: at least 100 travellers end with three car routes, or the three-route values
//! are not tested.
//!
//! For bikes: a bike has one route; its path size is 0 in a run (`ln PS`), judged in
//! `choice_row.rs`. These travellers are car-only.

#![allow(clippy::cast_precision_loss, reason = "counts of trips, far below 2^53")]

mod common;

use std::collections::BTreeMap;

use common::{car_link, demand, network, print_iterations, stream, time_only};
use openmobisim_loop::run::run;
use openmobisim_types::ids::{EntityId, TripId};
use openmobisim_types::mode::Mode;
use openmobisim_types::time::Second;

/// The hand value of the path size of the route ending on `last`, in a set of `routes` car
/// routes (the header's arithmetic).
fn by_hand(last: &str, routes: usize) -> f64 {
    match (last, routes) {
        (_, 1) => 1.0,
        ("short", 2) => 0.625,
        ("middle", 2) => 0.7,
        ("long", 2) => 0.75,
        ("short", 3) => 0.5,
        ("middle", 3) => 0.6,
        ("long", 3) => 2.0 / 3.0,
        other => panic!("no hand value for {other:?}"),
    }
}

#[test]
fn every_car_route_of_every_final_set_has_the_path_size_by_hand() {
    let unlimited = 1.0e9;
    let net = network(
        &[("O", 0.0, 0.0), ("J", 600.0, 0.0), ("D", 800.0, 0.0)],
        &[
            ("approach", "O", "J", car_link(600.0, 36.0, unlimited)),
            ("short", "J", "D", car_link(200.0, 36.0, 1_200.0)),
            ("middle", "J", "D", car_link(400.0, 36.0, 600.0)),
            ("long", "J", "D", car_link(600.0, 36.0, unlimited)),
        ],
    );
    let rows = stream(&net, "c", "O", "D", 3_600.0, 0.5, Second(0));
    assert_eq!(rows.len(), 1_800);
    let dem = demand(&net, &rows);
    let result =
        run(&net, &dem, None, (Second(0), Second(4 * 3_600)), &time_only(4)).expect("a run");
    print_iterations(&result);
    for it in &result.iterations {
        assert_eq!(it.unfinished, 0, "iteration {}: unfinished trips", it.iteration);
    }
    assert_eq!(result.iterations.len(), 4);

    let mut compositions: BTreeMap<Vec<&str>, usize> = BTreeMap::new();
    for i in 0..rows.len() {
        let set = result.set(TripId::from_index(i));
        let cars: Vec<_> = set.iter().filter(|a| a.mode == Mode::Car).collect();
        assert_eq!(cars.len(), set.len(), "trip {i}: car-only");
        let mut lasts = Vec::new();
        for a in &cars {
            let route = a.route().expect("a car route");
            let last = net.link_ids().external(route.last().expect("a link").raw());
            let want = by_hand(last, cars.len()).ln();
            assert!(
                (a.ln_path_size - want).abs() < 1e-12,
                "trip {i}, route by {last} of {} car routes: ln PS {}, by hand {want}",
                cars.len(),
                a.ln_path_size
            );
            lasts.push(last);
        }
        lasts.sort_unstable();
        *compositions.entry(lasts).or_default() += 1;
    }
    for (lasts, count) in &compositions {
        println!("{count} travellers end with the car routes by {}", lasts.join(", "));
    }
    let three: usize = compositions.iter().filter(|(k, _)| k.len() == 3).map(|(_, n)| n).sum();
    assert!(three >= 100, "only {three} travellers end with three car routes");
}
