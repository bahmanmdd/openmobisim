//! The alternative a traveller took costs what the trip took: after the measure stage, the
//! time of every taken alternative is the door-to-door time its trip was loaded with.
//!
//! The two routes of test 3 (`two_routes.rs`): a short one with a bottleneck of 1 200 veh/h and
//! a long one, 1 800 car trips an hour, each starting 13 m off the origin node (a walk of
//! 9.75 s, so route legs are entered between whole seconds). Eight iterations put cars on both
//! routes and in the bottleneck's queue. The property is the reading's: a traveller walking its
//! own route from its own departure reads the time it took.
//!
//! For bikes: a bike's alternative is its free-flow time, the same in the loading and in the
//! reading (`by_hand.rs`).

mod common;

use common::{at, car_link, demand, network, stream, time_only};
use openmobisim_demand::input::TripRow;
use openmobisim_loop::run::run;
use openmobisim_types::ids::{EntityId, TripId};
use openmobisim_types::time::Second;

#[test]
fn every_taken_alternative_costs_the_time_its_trip_took() {
    let net = network(
        &[("O", 0.0, 0.0), ("P", 2_000.0, 100.0), ("Q", 2_000.0, -100.0), ("D", 4_000.0, 0.0)],
        &[
            ("op", "O", "P", car_link(2_000.0, 50.0, 1_200.0)),
            ("pd", "P", "D", car_link(2_000.0, 50.0, 36_000.0)),
            ("oq", "O", "Q", car_link(3_200.0, 50.0, 36_000.0)),
            ("qd", "Q", "D", car_link(3_200.0, 50.0, 36_000.0)),
        ],
    );
    let rows: Vec<TripRow> = stream(&net, "c", "O", "D", 1_800.0, 1.0, Second(0))
        .into_iter()
        .map(|row| TripRow { origin: at(-13.0, 0.0), ..row })
        .collect();
    let dem = demand(&net, &rows);
    let result =
        run(&net, &dem, None, (Second(0), Second(3 * 3_600)), &time_only(8)).expect("a run");
    let (mut queued, mut long) = (0, 0);
    for i in 0..result.trips.finished.len() {
        assert!(result.trips.finished[i], "trip {i} finishes");
        let taken = result.taken(TripId::from_index(i)).expect("an alternative");
        let took = result.trips.travel_time[i];
        assert!(
            (taken.time - took).abs() < 1e-9,
            "trip {i}: priced {} s, took {took} s",
            taken.time
        );
        queued += usize::from(took > 288.0 + 9.75 + 1.0 && took < 460.0);
        long += usize::from(took > 460.0);
    }
    // Without a queue and both routes in use the property would hold trivially.
    assert!(queued >= 100, "only {queued} trips met the queue");
    assert!(long >= 100, "only {long} trips took the long route");
}
