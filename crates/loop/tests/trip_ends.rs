//! A trip beside a dead end still travels: its origin joins the nearest node it can leave.
//!
//! Nodes S at 0 m, A at 1 m and D at 100 m on a line; footways A→S (1 m) and A→D (99 m), one
//! way each, so S has no way out. A walker from 0 m to 100 m walks the 1 m gap to A (0.75 s at
//! 4.8 km/h), enters the footway then, and walks its 99 m in 74.25 s: 75 s by hand, door to
//! door.
//!
//! For bikes: the same rule on the links a bike may use (`crates/demand/tests/direction.rs`).

mod common;

use common::{at, demand, network};
use openmobisim_demand::input::TripRow;
use openmobisim_graph::network::LinkRow;
use openmobisim_loop::parameters::Parameters;
use openmobisim_loop::run::run;
use openmobisim_types::mode::{Mode, ModeSet};
use openmobisim_types::time::Second;

#[test]
fn a_walker_starting_beside_a_dead_end_walks_100_m_in_75_s() {
    let footway = |length_m: f64| LinkRow {
        modes: ModeSet::of(&[Mode::Walk]),
        lanes: None,
        car_speed_km_h: None,
        length_m: Some(length_m),
        capacity_veh_h: None,
        signal_at_downstream_end: false,
    };
    let net = network(
        &[("S", 0.0, 0.0), ("A", 1.0, 0.0), ("D", 100.0, 0.0)],
        &[("as", "A", "S", footway(1.0)), ("ad", "A", "D", footway(99.0))],
    );
    let rows = [TripRow {
        id: "t".to_string(),
        origin: at(0.0, 0.0),
        destination: at(100.0, 0.0),
        departure: Second(0),
        class: "default".to_string(),
        modes: ModeSet::of(&[Mode::Walk]),
    }];
    let dem = demand(&net, &rows);
    let result =
        run(&net, &dem, None, (Second(0), Second(600)), &Parameters::defaults(1)).expect("a run");
    assert_eq!(result.trips.alternatives[0], 1, "the walk is offered");
    assert!(result.trips.finished[0]);
    assert!((result.trips.travel_time[0] - 75.0).abs() < 1e-9, "{}", result.trips.travel_time[0]);
}
