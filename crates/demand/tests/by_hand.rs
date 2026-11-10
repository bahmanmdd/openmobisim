//! Test 3, the gaps by hand, and `stop_walk_max` at its default of 1 500 m (`PARAMETERS.md`):
//! a walker at 4.8 km/h covers a metre in 0.75 s.

mod common;

use common::{FOOTWAY, VALUES, at, one_class, stops, trip};
use openmobisim_demand::demand::Demand;
use openmobisim_graph::network::{LinkRow, Network, NetworkBuilder};
use openmobisim_types::ids::{EntityId, StopId, TripId};
use openmobisim_types::mode::Mode;
use openmobisim_types::units::Metres;

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
    // from a: no stop in reach of its origin, its destination's as `t`'s; `car` has no transit.
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
    assert!(demand.access(far).is_empty());
    assert_eq!(demand.egress(far).len(), 1);
    assert!(demand.access(car).is_empty() && demand.egress(car).is_empty(), "no transit");
    assert_eq!(demand.origins_without_stop(), 1);
    assert_eq!(demand.destinations_without_stop(), 0);
    assert_eq!(demand.trips_without_transit(), 1, "u-far, and not v-car");
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
