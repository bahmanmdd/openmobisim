//! The loading ends at the window's end, whatever its step (X-38 #7).
//!
//! The loading runs in steps; a window the step does not divide used to be run to the next
//! whole step, so a trip finishing in the overshoot was neither a finished trip (it arrived
//! after the window) nor an unfinished one (it was no longer on the network), and the report
//! of what stood still described a moment after the window.

use openmobisim_core_graph::defaults::SignalDefaults;
use openmobisim_core_graph::examples::toy_network;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_loading::{FidelityLevel, Recording, Rules, Vehicle, run_ltm_chained};
use openmobisim_core_types::ids::{EntityId, LinkId, VehicleId};
use openmobisim_core_types::time::Second;
use openmobisim_core_types::units::{Duration, Pcu};

/// **Property:** a vehicle on its way when the window ends is counted on the network, and the
/// report is the same whether or not the step divides the window.
#[test]
fn a_trip_still_under_way_at_the_window_end_is_counted_on_the_network() {
    let (net, _) = toy_network();
    let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);
    let id = |n: &str| net.link_external_ids().typed_id_of::<LinkId>(n).expect("a toy link");
    let route = vec![id("a1"), id("a2"), id("a3")];
    let run = |departure: u32, window: f64, step: f64| {
        let car = [Vehicle::new(VehicleId::new(0), route.clone(), Pcu(1.0), Second(departure))];
        run_ltm_chained(
            &net,
            &turns,
            &car,
            &[],
            Duration(window),
            Duration(step),
            FidelityLevel::Full,
            Recording::Trajectories,
            Rules::default(),
            None,
        )
    };
    let alone = run(0, 3600.0, 300.0);
    let trip = alone.trajectories[0].arrival().get();
    assert!(trip > 20 && trip < 190, "the fixture needs a trip shorter than the overshoot");
    // It departs so that it arrives half its trip after the window of 1000 s, before 1200 s.
    let departure = 1000 - trip / 2;
    for step in [300.0, 250.0, 100.0] {
        let out = run(departure, 1000.0, step);
        assert!(out.trajectories.is_empty(), "step {step}: it has not arrived");
        assert_eq!(out.lock.on_network, 1, "step {step}: it is on the network at the window's end");
    }
}

/// **Property (S271):** a loading whose clock starts at the window's start finishes a trip
/// exactly as one started at midnight, and one still under way at the end stays unfinished.
#[test]
fn a_loading_started_at_the_window_start_finishes_trips_as_one_started_at_midnight() {
    let (net, _) = toy_network();
    let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);
    let id = |n: &str| net.link_external_ids().typed_id_of::<LinkId>(n).expect("a toy link");
    let route = vec![id("a1"), id("a2"), id("a3")];
    let run = |departure: u32, window: f64, start_s: f64| {
        let car = [Vehicle::new(VehicleId::new(0), route.clone(), Pcu(1.0), Second(departure))];
        run_ltm_chained(
            &net,
            &turns,
            &car,
            &[],
            Duration(window),
            Duration(300.0),
            FidelityLevel::Full,
            Recording::Trajectories,
            Rules { start_s, ..Rules::default() },
            None,
        )
    };
    let midnight = run(27_100, 30_600.0, 0.0);
    let started = run(27_100, 30_600.0, 27_000.0);
    assert_eq!(midnight.trajectories.len(), 1);
    assert_eq!(
        midnight.trajectories[0].arrival(),
        started.trajectories[0].arrival(),
        "the same arrival"
    );
    let trip = started.trajectories[0].arrival().get() - 27_100;
    let late = run(30_600 - trip / 2, 30_600.0, 27_000.0);
    assert!(late.trajectories.is_empty(), "under way at the end");
    assert_eq!(late.lock.on_network, 1);
}
