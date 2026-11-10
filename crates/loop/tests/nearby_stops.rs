//! `nearby_stops` by hand: the journeys from and to the nearest stops tried alone. At 1 a trip
//! gets the one search between any stops in reach; at 2 the nearest stop of each end is also
//! tried alone.
//!
//! A walker covers a metre in 0.75 s (4.8 km/h). The boarding slack is the default 60 s. Costs
//! with the default coefficients: 0.165 per minute walked, 0.145 per minute waited, 0.1 per
//! minute in the vehicle, 0.25 for transit (`PARAMETERS.md`); time-only costs are the
//! door-to-door minutes. The choice is deterministic in both, so the journey taken is known.

mod common;

use common::{at, network, time_only};
use openmobisim_demand::demand::Demand;
use openmobisim_demand::input::TripRow;
use openmobisim_graph::network::{LinkRow, Network};
use openmobisim_loop::parameters::{ChoiceModel, Parameters};
use openmobisim_loop::run::{Run, run};
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

/// One transit-only trip from `from` to `to` (metres along the line) at second 0, run for one
/// iteration with `parameters`.
fn one_trip(net: &Network, tt: &Timetable, (from, to): (f64, f64), parameters: &Parameters) -> Run {
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
    run(net, &demand, Some(tt), (Second(0), Second(3_600)), parameters).expect("a run")
}

/// The default coefficients with the deterministic choice, one iteration, `nearby_stops` 2.
fn weighted() -> Parameters {
    Parameters {
        choice: ChoiceModel::Deterministic,
        nearby_stops: 2,
        link_bin: 60,
        iteration_cap: 1,
        ..Parameters::defaults(1)
    }
}

fn times(r: &Run) -> Vec<f64> {
    r.set(TripId::new(0)).iter().map(|a| a.time).collect()
}

#[test]
fn the_nearest_stop_tried_alone_offers_a_later_journey_with_675_s_less_walking() {
    // By hand. The door at 0 m, stop A at 100 m (75 s), stop B at 1 000 m (750 s), stop C at the
    // destination, 5 000 m. Run r0: B 1 000 s → C 1 300 s. Run r1: A 800 s → C 1 400 s.
    // Between any stops the earliest journey is r0 from B (there at 750 s): 1 300 s. From A
    // alone, the nearest stop: r1 (there at 75 s): 1 400 s.
    let net = line(&[("o", 0.0), ("a", 100.0), ("b", 1_000.0), ("c", 5_000.0)]);
    let tt = timetable(
        &[("A", 100.0), ("B", 1_000.0), ("C", 5_000.0)],
        &[&[("B", 1_000), ("C", 1_300)], &[("A", 800), ("C", 1_400)]],
    );
    let ends = (0.0, 5_000.0);
    // nearby_stops 1: the earliest journey alone, whatever the coefficients.
    let none = one_trip(&net, &tt, ends, &Parameters { nearby_stops: 1, ..time_only(1) });
    assert_eq!(times(&none), [1_300.0]);
    let none = one_trip(&net, &tt, ends, &Parameters { nearby_stops: 1, ..weighted() });
    assert_eq!(times(&none), [1_300.0]);
    // nearby_stops 2, time alone: both offered, the earlier taken.
    let timed = one_trip(&net, &tt, ends, &Parameters { nearby_stops: 2, ..time_only(1) });
    assert_eq!(times(&timed), [1_300.0, 1_400.0]);
    assert!((timed.trips.travel_time[0] - 1_300.0).abs() < 1e-9, "the earlier is taken");
    // nearby_stops 2, the default coefficients. From B: walk 750 s = 12.5 min × 0.165 = 2.0625,
    // wait 250 s × 0.145 / 60 = 0.60417, ride 300 s = 5 min × 0.1 = 0.5, transit 0.25: 3.41667.
    // From A: walk 75 s = 1.25 min × 0.165 = 0.20625, wait 725 s × 0.145 / 60 = 1.75208, ride
    // 600 s = 10 min × 0.1 = 1, transit 0.25: 3.20833, the cheaper: taken, 100 s later.
    let w = one_trip(&net, &tt, ends, &weighted());
    assert_eq!(times(&w), [1_300.0, 1_400.0]);
    assert!((w.trips.travel_time[0] - 1_400.0).abs() < 1e-9, "the cheaper is taken");
    let by_hand = 0.206_25 + 725.0 * 0.145 / 60.0 + 1.0 + 0.25;
    assert!((w.trips.cost[0] - by_hand).abs() < 1e-9, "{} against {by_hand}", w.trips.cost[0]);
}

#[test]
fn of_two_journeys_on_the_same_run_only_the_cheaper_for_the_traveller_is_offered() {
    // By hand. Stop A at the origin, 0 m; stop C at 5 000 m, the door at 5 400 m (400 m from C,
    // 300 s), stop D at 5 500 m (100 m from the door, 75 s). One run: A 600 s → C 1 300 s →
    // D 1 550 s. Between any stops the earliest is off at C: 1 300 + 300 = 1 600 s. To D alone,
    // the destination's nearest stop: the same run, 1 550 + 75 = 1 625 s. The same vehicle, so
    // one journey is offered, the traveller's cheaper.
    let net = line(&[("a", 0.0), ("c", 5_000.0), ("x", 5_400.0), ("d", 5_500.0)]);
    let tt = timetable(
        &[("A", 0.0), ("C", 5_000.0), ("D", 5_500.0)],
        &[&[("A", 600), ("C", 1_300), ("D", 1_550)]],
    );
    let ends = (0.0, 5_400.0);
    // Time alone: 1 600 s is less than 1 625 s.
    let timed = one_trip(&net, &tt, ends, &Parameters { nearby_stops: 2, ..time_only(1) });
    assert_eq!(times(&timed), [1_600.0]);
    // The default coefficients. Off at C: wait 600 s = 10 min × 0.145 = 1.45, ride 700 s × 0.1
    // / 60 = 1.16667, walk 300 s = 5 min × 0.165 = 0.825, transit 0.25: 3.69167. Off at D: 1.45,
    // ride 950 s × 0.1 / 60 = 1.58333, walk 75 s = 1.25 min × 0.165 = 0.20625, 0.25: 3.48958.
    let w = one_trip(&net, &tt, ends, &weighted());
    assert_eq!(times(&w), [1_625.0]);
    let by_hand = 1.45 + 950.0 * 0.1 / 60.0 + 0.206_25 + 0.25;
    assert!((w.trips.cost[0] - by_hand).abs() < 1e-9, "{} against {by_hand}", w.trips.cost[0]);
    // nearby_stops 1: D alone is never asked, so the earliest, off at C, is the one offered.
    let none = one_trip(&net, &tt, ends, &Parameters { nearby_stops: 1, ..weighted() });
    assert_eq!(times(&none), [1_600.0]);
}
