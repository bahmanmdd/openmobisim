//! Test 3, the gaps by hand, and `stop_walk_max` at its default of 1 500 m (`PARAMETERS.md`):
//! a walker at 4.8 km/h covers a metre in 0.75 s. Within the limit an end's list holds every
//! stop within it; at an end with no stop within it, the list holds the one stop with the least
//! whole walk (end gap, network, stop gap), ties to the lower stop id; an end from which no stop
//! can be reached on foot has an empty list.

mod common;

use common::{FOOTWAY, VALUES, at, one_class, stops, trip};
use openmobisim_demand::demand::Demand;
use openmobisim_graph::network::{LinkRow, Network, NetworkBuilder};
use openmobisim_types::ids::{EntityId, StopId, TripId};
use openmobisim_types::mode::Mode;
use openmobisim_types::units::{Duration, Metres};

/// The default of `stop_walk_max`, in metres (`PARAMETERS.md`).
const STOP_WALK_MAX: Metres = Metres(1_500.0);

/// Two nodes `a` at (0, 0) and `b` at (`metres`, 0), a footway each way between them.
fn street(metres: f64) -> Network {
    let mut b = NetworkBuilder::new();
    b.add_node("a", at(0.0, 0.0));
    b.add_node("b", at(metres, 0.0));
    b.add_link("a-b", "a", "b", LinkRow { length_m: Some(metres), ..FOOTWAY });
    b.add_link("b-a", "b", "a", LinkRow { length_m: Some(metres), ..FOOTWAY });
    b.build(VALUES).expect("builds")
}

/// Nodes `(id, x, y)` in metres and footways `(upstream, downstream, metres)`, each one way.
fn footways(nodes: &[(&str, f64, f64)], links: &[(&str, &str, f64)]) -> Network {
    let mut b = NetworkBuilder::new();
    for &(id, x, y) in nodes {
        b.add_node(id, at(x, y));
    }
    for &(up, down, metres) in links {
        b.add_link(format!("{up}-{down}"), up, down, LinkRow { length_m: Some(metres), ..FOOTWAY });
    }
    b.build(VALUES).expect("builds")
}

/// The stop ids and the walking seconds of a list.
fn ids_and_seconds(list: &[(StopId, Duration)]) -> Vec<(u32, f64)> {
    list.iter().map(|&(s, d)| (s.raw(), d.get())).collect()
}

/// Whether `list` is exactly `stop` at `seconds` by hand, to a relative 1e-9.
fn only(list: &[(StopId, Duration)], stop: u32, seconds: f64) -> bool {
    matches!(ids_and_seconds(list)[..], [(s, d)] if s == stop && (d - seconds).abs() <= 1e-9 * seconds)
}

#[test]
fn an_end_30_m_off_and_a_stop_40_m_off_a_1000_m_street_walk_1070_m_in_802_5_s() {
    // By hand: 30 m + 1 000 m + 40 m = 1 070 m, at 4.8 km/h (0.75 s per metre) 802.5 s, to the
    // stop from the origin and from the stop to the destination alike.
    let net = street(1_000.0);
    let tt = stops(&[("s", 1_040.0, 0.0)]);
    let rows = [trip("t", at(-30.0, 0.0), at(0.0, -30.0), &[Mode::Transit])];
    let demand =
        Demand::build(&rows, &one_class(), &net, Some(&tt), STOP_WALK_MAX).expect("builds");
    let (t, s) = (TripId::new(0), StopId::new(0));
    assert_eq!(demand.stop_end(s).gap, Metres(40.0));
    assert_eq!(demand.origin_end(t, Mode::Transit).expect("linked").gap, Metres(30.0));
    for (what, list) in [("access", demand.access(t)), ("egress", demand.egress(t))] {
        assert_eq!(list.len(), 1, "{what}");
        assert_eq!(list[0].0, s, "{what}");
        let seconds = list[0].1.get();
        assert!((seconds - 802.5).abs() <= 1e-9 * 802.5, "{what}: {seconds} s, by hand 802.5 s");
    }
    assert_eq!(demand.trips_without_transit(), 0);
}

#[test]
fn at_stop_walk_max_1500_m_a_walk_of_1500_m_is_in_and_one_of_1501_m_out() {
    // By hand on a 1 430 m street: origin 30 m off a; stop `in` 40 m off b (30 + 1 430 + 40 =
    // 1 500 m, in, 1 125 s); stop `out` 41 m off b (1 501 m, out). Trip `far` starts 2 000 m
    // from a: no stop within the limit of its origin, so its access list is the nearest stop,
    // `in` at 2 000 + 1 430 + 40 = 3 470 m (2 602.5 s), not `out` at 3 471 m; its destination's
    // list as `t`'s; `car` has no transit.
    let net = street(1_430.0);
    let tt = stops(&[("in", 1_470.0, 0.0), ("out", 1_430.0, 41.0)]);
    let rows = [
        trip("t", at(-30.0, 0.0), at(-30.0, 0.0), &[Mode::Transit]),
        trip("u-far", at(-2_000.0, 0.0), at(-30.0, 0.0), &[Mode::Transit, Mode::Walk]),
        trip("v-car", at(-2_000.0, 0.0), at(-2_000.0, 0.0), &[Mode::Walk]),
    ];
    let demand =
        Demand::build(&rows, &one_class(), &net, Some(&tt), STOP_WALK_MAX).expect("builds");
    let stop_in = tt.stop_ids().typed_id_of::<StopId>("in").expect("a stop");
    let t = TripId::new(0);
    assert_eq!(demand.access(t).len(), 1, "only `in` is in reach");
    assert_eq!(demand.access(t)[0].0, stop_in);
    assert!((demand.access(t)[0].1.get() - 1_125.0).abs() <= 1e-9 * 1_125.0);
    assert_eq!(demand.egress(t).iter().map(|e| e.0).collect::<Vec<_>>(), vec![stop_in]);
    let (far, car) = (TripId::new(1), TripId::new(2));
    assert!(only(demand.access(far), stop_in.raw(), 2_602.5), "{:?}", demand.access(far));
    assert!(only(demand.egress(far), stop_in.raw(), 1_125.0), "{:?}", demand.egress(far));
    assert!(demand.access(car).is_empty() && demand.egress(car).is_empty(), "no transit");
    assert_eq!(demand.origins_without_stop(), 0);
    assert_eq!(demand.destinations_without_stop(), 0);
    assert_eq!(demand.trips_without_transit(), 0, "u-far reaches a stop, v-car has no transit");
}

#[test]
fn with_no_stop_within_1500_m_the_one_stop_1501_m_away_is_offered_in_1125_75_s() {
    // By hand on a 1 431 m street: both ends 30 m off a, the stop 40 m off b: 30 + 1 431 + 40 =
    // 1 501 m each way, 1 m beyond the limit and no stop nearer; 1 501 × 0.75 = 1 125.75 s.
    let net = street(1_431.0);
    let tt = stops(&[("s", 1_471.0, 0.0)]);
    let rows = [trip("t", at(-30.0, 0.0), at(-30.0, 0.0), &[Mode::Transit])];
    let demand =
        Demand::build(&rows, &one_class(), &net, Some(&tt), STOP_WALK_MAX).expect("builds");
    let t = TripId::new(0);
    assert!(only(demand.access(t), 0, 1_125.75), "{:?}", demand.access(t));
    assert!(only(demand.egress(t), 0, 1_125.75), "{:?}", demand.egress(t));
    assert_eq!(demand.origins_without_stop(), 0);
    assert_eq!(demand.destinations_without_stop(), 0);
    assert_eq!(demand.trips_without_transit(), 0);
}

#[test]
fn of_two_stops_beyond_1500_m_only_the_nearer_by_whole_walk_gaps_counted() {
    // By hand: footways a (0, 0) – b (2 000, 0) – c (2 100, 0), each way. Both ends 30 m off a.
    // Stop `p` 150 m off b (b is its nearest node: c is √(100² + 150²) = 180.3 m off): 30 +
    // 2 000 + 150 = 2 180 m. Stop `q` on c: 30 + 2 100 + 0 = 2 130 m, the nearer although its
    // node is farther; 2 130 × 0.75 = 1 597.5 s. `p` sorts first: id 0, `q` id 1.
    let net = footways(
        &[("a", 0.0, 0.0), ("b", 2_000.0, 0.0), ("c", 2_100.0, 0.0)],
        &[("a", "b", 2_000.0), ("b", "a", 2_000.0), ("b", "c", 100.0), ("c", "b", 100.0)],
    );
    let tt = stops(&[("p", 2_000.0, 150.0), ("q", 2_100.0, 0.0)]);
    let rows = [trip("t", at(-30.0, 0.0), at(-30.0, 0.0), &[Mode::Transit])];
    let demand =
        Demand::build(&rows, &one_class(), &net, Some(&tt), STOP_WALK_MAX).expect("builds");
    let t = TripId::new(0);
    assert_eq!(demand.stop_end(StopId::new(0)).gap, Metres(150.0), "p's gap");
    assert!(only(demand.access(t), 1, 1_597.5), "{:?}", demand.access(t));
    assert!(only(demand.egress(t), 1, 1_597.5), "{:?}", demand.egress(t));
}

#[test]
fn a_tie_beyond_1500_m_goes_to_the_lower_stop_id() {
    // By hand: footways c (-1 600, 0) – a (0, 0) – b (1 600, 0), each way. Both ends 30 m off a,
    // at (0, 30). Stop `w` 40 m off c, stop `x` 40 m off b: 30 + 1 600 + 40 = 1 670 m both, the
    // same sum of the same numbers; the tie goes to `w`, id 0; 1 670 × 0.75 = 1 252.5 s.
    let net = footways(
        &[("a", 0.0, 0.0), ("b", 1_600.0, 0.0), ("c", -1_600.0, 0.0)],
        &[("a", "b", 1_600.0), ("b", "a", 1_600.0), ("a", "c", 1_600.0), ("c", "a", 1_600.0)],
    );
    let tt = stops(&[("w", -1_640.0, 0.0), ("x", 1_640.0, 0.0)]);
    let rows = [trip("t", at(0.0, 30.0), at(0.0, 30.0), &[Mode::Transit])];
    let demand =
        Demand::build(&rows, &one_class(), &net, Some(&tt), STOP_WALK_MAX).expect("builds");
    let t = TripId::new(0);
    assert!(only(demand.access(t), 0, 1_252.5), "{:?}", demand.access(t));
    assert!(only(demand.egress(t), 0, 1_252.5), "{:?}", demand.egress(t));
}

#[test]
fn on_a_one_way_walk_the_destination_gets_the_nearest_stop_walking_toward_it() {
    // By hand: footways one way round a (0, 0) → b (1 000, 0) → c (1 000, 1 000) → a, of 1 000,
    // 1 000 and 1 500 m. Both ends 20 m off a, at (-20, 0). Stop `s1` 40 m off b, `s2` 50 m off
    // c. Access, from the origin: s1 at 20 + 1 000 + 40 = 1 060 m (795 s), within the limit;
    // s2 at 20 + 2 000 + 50 = 2 070 m, out. Egress, to the destination: from s1, 40 + 1 000 +
    // 1 500 + 20 = 2 560 m; from s2, 50 + 1 500 + 20 = 1 570 m; none within the limit, so the
    // nearer, s2, in 1 570 × 0.75 = 1 177.5 s (walking the other way it would be s1, 1 060 m).
    let net = footways(
        &[("a", 0.0, 0.0), ("b", 1_000.0, 0.0), ("c", 1_000.0, 1_000.0)],
        &[("a", "b", 1_000.0), ("b", "c", 1_000.0), ("c", "a", 1_500.0)],
    );
    let tt = stops(&[("s1", 1_040.0, 0.0), ("s2", 1_000.0, 1_050.0)]);
    let rows = [trip("t", at(-20.0, 0.0), at(-20.0, 0.0), &[Mode::Transit])];
    let demand =
        Demand::build(&rows, &one_class(), &net, Some(&tt), STOP_WALK_MAX).expect("builds");
    let t = TripId::new(0);
    assert!(only(demand.access(t), 0, 795.0), "{:?}", demand.access(t));
    assert!(only(demand.egress(t), 1, 1_177.5), "{:?}", demand.egress(t));
}

#[test]
fn an_end_with_no_walk_to_any_stop_keeps_an_empty_list() {
    // By hand: two pieces of footway, a (0, 0) – b (100, 0) and d (5 000, 0) – e (5 100, 0),
    // each way, joined by nothing. The stop is 10 m off e; trip `cut` starts and ends 10 m off
    // a: no walk joins it to the stop, so both its lists are empty and it is counted; trip
    // `joined` starts 10 m off a and ends 10 m off d: its egress is the stop, 10 + 100 + 10 =
    // 120 m (90 s), and its access empty.
    let net = footways(
        &[("a", 0.0, 0.0), ("b", 100.0, 0.0), ("d", 5_000.0, 0.0), ("e", 5_100.0, 0.0)],
        &[("a", "b", 100.0), ("b", "a", 100.0), ("d", "e", 100.0), ("e", "d", 100.0)],
    );
    let tt = stops(&[("s", 5_100.0, 10.0)]);
    let rows = [
        trip("cut", at(0.0, 10.0), at(0.0, 10.0), &[Mode::Transit]),
        trip("joined", at(0.0, 10.0), at(5_000.0, 10.0), &[Mode::Transit]),
    ];
    let demand =
        Demand::build(&rows, &one_class(), &net, Some(&tt), STOP_WALK_MAX).expect("builds");
    let (cut, joined) = (TripId::new(0), TripId::new(1));
    assert!(demand.access(cut).is_empty() && demand.egress(cut).is_empty());
    assert!(demand.access(joined).is_empty());
    assert!(only(demand.egress(joined), 0, 90.0), "{:?}", demand.egress(joined));
    assert_eq!(demand.origins_without_stop(), 2);
    assert_eq!(demand.destinations_without_stop(), 1);
    assert_eq!(demand.trips_without_transit(), 2);
}

#[test]
fn without_a_timetable_every_transit_trip_is_without_a_stop() {
    let net = street(1_000.0);
    let rows = [trip("t", at(0.0, 0.0), at(1_000.0, 0.0), &[Mode::Transit, Mode::Walk])];
    let demand = Demand::build(&rows, &one_class(), &net, None, STOP_WALK_MAX).expect("builds");
    assert_eq!(demand.stop_count(), 0);
    assert_eq!((demand.origins_without_stop(), demand.destinations_without_stop()), (1, 1));
    assert_eq!(demand.trips_without_transit(), 1);
}
