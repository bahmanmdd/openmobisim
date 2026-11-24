//! `nearby_stops` by hand: the journeys from and to the nearest stops tried alone. At 1 a trip
//! gets the searches between any stops in reach, the earliest journey and the cheapest for its
//! class; at 2 the nearest stop of each end is also tried alone for the earliest. With time as
//! the only cost the cheapest is the earliest, so the parameter's own effect shows there; with
//! the default coefficients the cheapest search already finds what the nearest stop offers.
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
    // nearby_stops 1, time alone: the earliest journey, which is also the cheapest.
    let none = one_trip(&net, &tt, ends, &Parameters { nearby_stops: 1, ..time_only(1) });
    assert_eq!(times(&none), [1_300.0]);
    // nearby_stops 1, the default coefficients: the earliest, and the cheapest between any
    // stops, which is r1 from A (3.20833 against 3.41667, derived below): taken.
    let none = one_trip(&net, &tt, ends, &Parameters { nearby_stops: 1, ..weighted() });
    assert_eq!(times(&none), [1_300.0, 1_400.0]);
    assert!((none.trips.travel_time[0] - 1_400.0).abs() < 1e-9, "the cheaper is taken");
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
    // nearby_stops 1, time alone: D alone is never asked, and off at C is the earliest and the
    // cheapest.
    let none = one_trip(&net, &tt, ends, &Parameters { nearby_stops: 1, ..time_only(1) });
    assert_eq!(times(&none), [1_600.0]);
    // nearby_stops 1, the default coefficients: the cheapest between any stops is off at D
    // (3.48958 against 3.69167), on the same run as the earliest: the one offered.
    let none = one_trip(&net, &tt, ends, &Parameters { nearby_stops: 1, ..weighted() });
    assert_eq!(times(&none), [1_625.0]);
    assert!(
        (none.trips.cost[0] - by_hand).abs() < 1e-9,
        "{} against {by_hand}",
        none.trips.cost[0]
    );
}

#[test]
fn max_transit_journeys_1_keeps_the_cheaper_of_the_two_journeys_and_2_keeps_both_in_order() {
    // The first test's case. With the default coefficients the two journeys cost 3.41667 (from
    // B, 1 300 s) and 3.20833 (from A, 1 400 s), derived there: a limit of 1 keeps the one from
    // A. With time alone they cost 21.667 and 23.333 minutes: a limit of 1 keeps the one from B.
    // A limit of 2 or more keeps both, in the order found.
    let net = line(&[("o", 0.0), ("a", 100.0), ("b", 1_000.0), ("c", 5_000.0)]);
    let tt = timetable(
        &[("A", 100.0), ("B", 1_000.0), ("C", 5_000.0)],
        &[&[("B", 1_000), ("C", 1_300)], &[("A", 800), ("C", 1_400)]],
    );
    let ends = (0.0, 5_000.0);
    let one = one_trip(&net, &tt, ends, &Parameters { max_transit_journeys: 1, ..weighted() });
    assert_eq!(times(&one), [1_400.0], "the cheaper for the traveller, from A");
    let timed = Parameters { nearby_stops: 2, max_transit_journeys: 1, ..time_only(1) };
    assert_eq!(times(&one_trip(&net, &tt, ends, &timed)), [1_300.0], "time alone: from B");
    for limit in [2, u32::MAX] {
        let both =
            one_trip(&net, &tt, ends, &Parameters { max_transit_journeys: limit, ..weighted() });
        assert_eq!(times(&both), [1_300.0, 1_400.0], "limit {limit}: both, in the order found");
    }
}

#[test]
fn of_three_journeys_a_limit_of_2_keeps_the_two_cheapest_in_the_order_found_the_cheapest_last() {
    // The first test's case and a third stop, F at 400 m (300 s), with run r2: F 400 s → C 1 500 s.
    // The default coefficients, nearby_stops 2. Found in this order: the earliest between any
    // stops, r0 from B (1 300 s); the nearest stop alone, r1 from A (1 400 s); the cheapest between
    // any stops, r2 from F (1 500 s). Costs: r0 3.41667 and r1 3.20833 (the first test); r2 walk
    // 5 min × 0.165 = 0.825, wait 100 s × 0.145 / 60 = 0.24167, ride 1 100 s × 0.1 / 60 = 1.83333,
    // transit 0.25: 3.15, the cheapest, found last. A limit of 2 keeps r1 and r2, in the order
    // found; 1 keeps r2; 3 keeps all three.
    let net = line(&[("o", 0.0), ("a", 100.0), ("f", 400.0), ("b", 1_000.0), ("c", 5_000.0)]);
    let tt = timetable(
        &[("A", 100.0), ("F", 400.0), ("B", 1_000.0), ("C", 5_000.0)],
        &[&[("B", 1_000), ("C", 1_300)], &[("A", 800), ("C", 1_400)], &[("F", 400), ("C", 1_500)]],
    );
    let ends = (0.0, 5_000.0);
    let with = |limit: u32| {
        one_trip(&net, &tt, ends, &Parameters { max_transit_journeys: limit, ..weighted() })
    };
    assert_eq!(times(&with(3)), [1_300.0, 1_400.0, 1_500.0], "all three, in the order found");
    assert_eq!(times(&with(2)), [1_400.0, 1_500.0], "the two cheapest, in the order found");
    assert_eq!(times(&with(1)), [1_500.0], "the cheapest");
    let by_hand = 0.825 + 100.0 * 0.145 / 60.0 + 1_100.0 * 0.1 / 60.0 + 0.25;
    let one = with(1);
    assert!((one.trips.cost[0] - by_hand).abs() < 1e-9, "{} against {by_hand}", one.trips.cost[0]);
}
