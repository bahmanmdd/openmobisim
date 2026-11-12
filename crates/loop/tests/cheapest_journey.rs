//! The transit journeys a traveller is offered hold the cheapest for its class, not only the
//! earliest: by hand on a line of footways, one transit-only trip at second 0.
//!
//! A walker covers a metre in 0.75 s (4.8 km/h). The boarding slack is the default 60 s. The
//! default coefficients: 0.165 per minute walked, 0.145 per minute waited, 0.1 per minute in a
//! vehicle, 0.5 per transfer, 0.25 for transit (`PARAMETERS.md`). The choice is deterministic,
//! so the journey taken is the cheapest offered.
//!
//! For bikes: nothing here is a bike; a transit journey is walked to and from.

mod common;

use common::{at, network};
use openmobisim_demand::demand::Demand;
use openmobisim_demand::input::TripRow;
use openmobisim_graph::network::{LinkRow, Network};
use openmobisim_loop::parameters::{ChoiceModel, Parameters};
use openmobisim_loop::run::{Run, RunError, run};
use openmobisim_transit::timetable::{Call, Timetable, TimetableBuilder};
use openmobisim_types::ids::{EntityId, TripId};
use openmobisim_types::mode::{Mode, ModeSet};
use openmobisim_types::time::Second;
use openmobisim_types::units::Metres;

/// Footways each way between the nodes of a line, `(id, x)` in metres.
fn line(nodes: &[(&str, f64)]) -> Network {
    let row = |length_m: f64| LinkRow {
        modes: ModeSet::of(&[Mode::Walk]),
        lanes: None,
        car_speed_km_h: None,
        length_m: Some(length_m),
        capacity_veh_h: None,
        signal_at_downstream_end: false,
    };
    let xy: Vec<(&str, f64, f64)> = nodes.iter().map(|&(id, x)| (id, x, 0.0)).collect();
    let ids: Vec<String> = (0..2 * (nodes.len() - 1)).map(|i| format!("l{i}")).collect();
    let mut links = Vec::new();
    for (i, pair) in nodes.windows(2).enumerate() {
        let ((u, xu), (v, xv)) = (pair[0], pair[1]);
        links.push((ids[2 * i].as_str(), u, v, row(xv - xu)));
        links.push((ids[2 * i + 1].as_str(), v, u, row(xv - xu)));
    }
    network(&xy, &links)
}

/// A timetable of stops `(id, x)` on the line and runs as `(stop, second)` calls.
fn timetable(stops: &[(&str, f64)], runs: &[&[(&str, u32)]]) -> Timetable {
    let mut b = TimetableBuilder::new();
    for &(id, x) in stops {
        b.add_stop(id, (Metres(x), Metres(0.0)));
    }
    for (i, calls) in runs.iter().enumerate() {
        let calls = calls
            .iter()
            .map(|&(stop, s)| Call {
                stop: stop.to_string(),
                arrival: Second(s),
                departure: Second(s),
            })
            .collect();
        b.add_run(format!("r{i}"), calls);
    }
    b.build().expect("a timetable")
}

/// One transit-only trip from `from` to `to` (metres along the line) at second 0, one
/// iteration at the default coefficients with the deterministic choice and `nearby_stops`.
fn one_trip(net: &Network, tt: &Timetable, (from, to): (f64, f64), nearby_stops: u8) -> Run {
    let rows = [TripRow {
        id: "t".to_string(),
        origin: at(from, 0.0),
        destination: at(to, 0.0),
        departure: Second(0),
        class: "default".to_string(),
        modes: ModeSet::of(&[Mode::Transit]),
    }];
    let demand = Demand::build(&rows, &["default".to_string()], net, Some(tt), Metres(1_500.0))
        .expect("a demand");
    let parameters = Parameters {
        choice: ChoiceModel::Deterministic,
        nearby_stops,
        iteration_cap: 1,
        ..Parameters::defaults(1)
    };
    run(net, &demand, Some(tt), (Second(0), Second(3_600)), &parameters).expect("a run")
}

/// The door-to-door seconds of the journeys offered, in the order found.
fn times(r: &Run) -> Vec<f64> {
    r.set(TripId::new(0)).iter().map(|a| a.time).collect()
}

#[test]
fn a_slow_run_boarded_at_once_is_offered_beside_the_fast_run_waited_for() {
    // Stops A at the origin and B at the destination, 5 000 m apart. A slow run leaves A at
    // 60 s and reaches B at 720 s; a fast run leaves at 600 s and arrives at 660 s. The
    // earliest arrival is the fast run's: wait 10 min × 0.145 + ride 1 min × 0.1 + 0.25 =
    // 1.8. The slow run: wait 1 min × 0.145 + ride 11 min × 0.1 + 0.25 = 1.495, the cheaper.
    // Both are offered, at any `nearby_stops`; the cheaper is taken.
    let net = line(&[("a", 0.0), ("b", 5_000.0)]);
    let tt = timetable(
        &[("A", 0.0), ("B", 5_000.0)],
        &[&[("A", 60), ("B", 720)], &[("A", 600), ("B", 660)]],
    );
    for nearby_stops in [1, 3] {
        let r = one_trip(&net, &tt, (0.0, 5_000.0), nearby_stops);
        assert_eq!(times(&r), [660.0, 720.0], "the earliest, then the cheapest");
        assert!((r.trips.travel_time[0] - 720.0).abs() < 1e-9, "the cheaper is taken");
        assert!((r.trips.cost[0] - 1.495).abs() < 1e-9, "{}", r.trips.cost[0]);
    }
}

#[test]
fn a_nearer_stop_with_a_later_run_is_offered_without_asking_for_it_alone() {
    // The door at 0 m, stop A at 100 m (75 s), stop B at 1 000 m (750 s), stop C at the
    // destination, 5 000 m. Run r0: B 1 000 s → C 1 300 s. Run r1: A 800 s → C 1 400 s. The
    // earliest is r0 from B: walk 12.5 min × 0.165 = 2.0625, wait 250 s × 0.145 / 60 =
    // 0.60417, ride 5 min × 0.1 = 0.5, transit 0.25: 3.41667. From A: walk 1.25 min × 0.165 =
    // 0.20625, wait 725 s × 0.145 / 60 = 1.75208, ride 10 min × 0.1 = 1, transit 0.25:
    // 3.20833, the cheaper. With one search per end the cheaper is offered all the same.
    let net = line(&[("o", 0.0), ("a", 100.0), ("b", 1_000.0), ("c", 5_000.0)]);
    let tt = timetable(
        &[("A", 100.0), ("B", 1_000.0), ("C", 5_000.0)],
        &[&[("B", 1_000), ("C", 1_300)], &[("A", 800), ("C", 1_400)]],
    );
    let r = one_trip(&net, &tt, (0.0, 5_000.0), 1);
    assert_eq!(times(&r), [1_300.0, 1_400.0]);
    assert!((r.trips.travel_time[0] - 1_400.0).abs() < 1e-9, "the cheaper is taken");
    let by_hand = 0.206_25 + 725.0 * 0.145 / 60.0 + 1.0 + 0.25;
    assert!((r.trips.cost[0] - by_hand).abs() < 1e-9, "{} against {by_hand}", r.trips.cost[0]);
}

#[test]
fn riding_one_stop_further_to_walk_less_is_offered_without_asking_for_that_stop_alone() {
    // Stop A at the origin; stop C at 5 000 m, the door at 5 400 m (300 s from C), stop D at
    // 5 500 m (75 s from the door). One run: A 600 s → C 1 300 s → D 1 550 s. Off at C: wait
    // 10 min × 0.145 = 1.45, ride 700 s × 0.1 / 60 = 1.16667, walk 5 min × 0.165 = 0.825,
    // transit 0.25: 3.69167, at the door at 1 600 s. Off at D: 1.45, ride 950 s × 0.1 / 60 =
    // 1.58333, walk 1.25 min × 0.165 = 0.20625, 0.25: 3.48958, at the door at 1 625 s. The
    // same vehicle, so one journey is offered: the traveller's cheaper, with one search per end.
    let net = line(&[("a", 0.0), ("c", 5_000.0), ("x", 5_400.0), ("d", 5_500.0)]);
    let tt = timetable(
        &[("A", 0.0), ("C", 5_000.0), ("D", 5_500.0)],
        &[&[("A", 600), ("C", 1_300), ("D", 1_550)]],
    );
    let r = one_trip(&net, &tt, (0.0, 5_400.0), 1);
    assert_eq!(times(&r), [1_625.0]);
    let by_hand = 1.45 + 950.0 * 0.1 / 60.0 + 0.206_25 + 0.25;
    assert!((r.trips.cost[0] - by_hand).abs() < 1e-9, "{} against {by_hand}", r.trips.cost[0]);
}

#[test]
fn a_direct_journey_and_an_earlier_one_with_a_transfer_are_both_offered() {
    // Stops A at the origin, X at 5 000 m, B at the destination, 10 000 m. A direct run A 600 s
    // → B 1 200 s; a run A 600 s → X 800 s and a run X 860 s → B 1 199 s. The two rides arrive
    // a second earlier. Direct: wait 10 min × 0.145 + ride 10 min × 0.1 + 0.25 = 2.7. With
    // the transfer: 1.45 + ride (200 + 339) s × 0.1 / 60 + wait 60 s × 0.145 / 60 + 0.5 +
    // 0.25 = 3.24333. Both are offered, the direct taken.
    let net = line(&[("a", 0.0), ("x", 5_000.0), ("b", 10_000.0)]);
    let tt = timetable(
        &[("A", 0.0), ("X", 5_000.0), ("B", 10_000.0)],
        &[&[("A", 600), ("B", 1_200)], &[("A", 600), ("X", 800)], &[("X", 860), ("B", 1_199)]],
    );
    let r = one_trip(&net, &tt, (0.0, 10_000.0), 3);
    assert_eq!(times(&r), [1_200.0, 1_199.0]);
    assert!((r.trips.cost[0] - 2.7).abs() < 1e-9, "{}", r.trips.cost[0]);
    let two = &r.set(TripId::new(0))[1];
    let cost = Parameters::defaults(1).cost(0, &two.attributes());
    let by_hand = 1.45 + 539.0 * 0.1 / 60.0 + 0.145 + 0.5 + 0.25;
    assert!((cost - by_hand).abs() < 1e-9, "{cost} against {by_hand}");
}

#[test]
fn a_negative_transit_coefficient_is_refused() {
    // The cheapest journey does not exist when a part of a journey is paid for: with a
    // timetable, a negative coefficient on a transit column is refused, naming the column.
    let net = line(&[("a", 0.0), ("b", 5_000.0)]);
    let tt = timetable(&[("A", 0.0), ("B", 5_000.0)], &[&[("A", 60), ("B", 720)]]);
    let rows = [TripRow {
        id: "t".to_string(),
        origin: at(0.0, 0.0),
        destination: at(5_000.0, 0.0),
        departure: Second(0),
        class: "default".to_string(),
        modes: ModeSet::of(&[Mode::Transit]),
    }];
    let demand = Demand::build(&rows, &["default".to_string()], &net, Some(&tt), Metres(1_500.0))
        .expect("a demand");
    let mut b = Parameters::DEFAULT_COEFFICIENTS;
    b[7] = -0.1;
    let parameters = Parameters { coefficients: vec![b], ..Parameters::defaults(1) };
    let window = (Second(0), Second(3_600));
    let refused = run(&net, &demand, Some(&tt), window, &parameters).map(|_| ());
    let want = RunError::TransitWeight { class: 0, column: "transit_transfer_wait_min" };
    assert_eq!(refused, Err(want));
    // Without a timetable the transit columns are not read.
    assert!(run(&net, &demand, None, window, &parameters).is_ok());
}
