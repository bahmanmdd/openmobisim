//! Test 4: what the demand refuses (a position of another kind, `stop_walk_max`), naming the
//! trip at fault by its external id, and what it sets right or drops instead, each noted.

mod common;

use common::{CARS_ONLY, FOOTWAY, STREET, VALUES, at, one_class, stops, trip};
use openmobisim_demand::demand::Demand;
use openmobisim_demand::input::{DemandError, TripRow};
use openmobisim_graph::geometry::{LonLat, Position};
use openmobisim_graph::network::{LinkRow, Network, NetworkBuilder};
use openmobisim_transit::timetable::Timetable;
use openmobisim_types::ids::{EntityId, TripId};
use openmobisim_types::mode::{Mode, ModeSet};
use openmobisim_types::units::Metres;

/// Two nodes 1 km apart, in metres or in longitude/latitude near Delft, and a link each way.
fn pair(lon_lat: bool, row: LinkRow) -> Network {
    let mut b = NetworkBuilder::new();
    if lon_lat {
        b.add_node("a", Position::LonLat(LonLat::new(4.36, 52.0)));
        b.add_node("b", Position::LonLat(LonLat::new(4.37, 52.0)));
    } else {
        b.add_node("a", at(0.0, 0.0));
        b.add_node("b", at(1_000.0, 0.0));
    }
    b.add_link("a-b", "a", "b", row);
    b.add_link("b-a", "b", "a", row);
    b.build(VALUES).expect("builds")
}

fn lon_lat(lon: f64, lat: f64) -> Position {
    Position::LonLat(LonLat::new(lon, lat))
}

fn refused(
    rows: &[TripRow],
    classes: &[String],
    net: &Network,
    tt: Option<&Timetable>,
) -> DemandError {
    let e = Demand::build(rows, classes, net, tt, Metres(1_500.0)).expect_err("refused");
    eprintln!("{e}");
    e
}

const WALK: &[Mode] = &[Mode::Walk];

/// Repairs as (key, count, first ids).
type Noted = Vec<(&'static str, u64, Vec<String>)>;

/// The demand of `rows`, built, with its repairs as (key, count, first ids).
fn repaired(
    rows: &[TripRow],
    classes: &[String],
    net: &Network,
    tt: Option<&Timetable>,
) -> (Demand, Noted) {
    let d = Demand::build(rows, classes, net, tt, Metres(1_500.0)).expect("built");
    let kinds = d.repairs().kinds().iter().map(|k| (k.key, k.count, k.first.clone())).collect();
    (d, kinds)
}

fn ids(d: &Demand) -> Vec<&str> {
    d.trip_ids().iter().collect()
}

/// A trip id given twice keeps the first row given (its origin at 0 m, not 5 m); a class given
/// twice is read once.
#[test]
fn a_trip_or_a_class_given_twice_is_read_once_and_noted() {
    let net = pair(false, STREET);
    let rows =
        [trip("t", at(0.0, 0.0), at(1.0, 0.0), WALK), trip("t", at(5.0, 0.0), at(1.0, 0.0), WALK)];
    let (d, kinds) = repaired(&rows, &one_class(), &net, None);
    assert_eq!((ids(&d), d.trips_given()), (vec!["t"], 2));
    assert_eq!(d.origin(TripId::new(0)).x, 0.0, "the first row given");
    assert_eq!(kinds, [("trips_repeated", 1, vec!["t".to_owned()])]);
    let classes = vec!["c".to_owned(), "d".to_owned(), "c".to_owned()];
    let (d, kinds) = repaired(&rows[..1], &classes, &net, None);
    assert_eq!(d.class_count(), 2);
    assert_eq!(kinds, [("classes_repeated", 1, vec!["c".to_owned()])]);
}

/// A trip of a class not given and a trip with no mode are dropped, noted; the others kept.
#[test]
fn an_unknown_class_and_an_empty_mode_set_drop_the_trip() {
    let net = pair(false, STREET);
    let mut lost = trip("b-lost", at(0.0, 0.0), at(1.0, 0.0), WALK);
    lost.class = "x".into();
    let idle = trip("a-idle", at(0.0, 0.0), at(1.0, 0.0), &[]);
    let fine = trip("c-fine", at(0.0, 0.0), at(1.0, 0.0), WALK);
    let (d, kinds) = repaired(&[lost, fine, idle], &one_class(), &net, None);
    assert_eq!((ids(&d), d.trips_given()), (vec!["c-fine"], 3));
    assert_eq!(
        kinds,
        [
            ("trips_without_mode", 1, vec!["a-idle".to_owned()]),
            ("trips_of_unknown_class", 1, vec!["b-lost".to_owned()]),
        ]
    );
}

/// Refused: the trips' kind must be the network's (no projection converts them).
/// Dropped, noted: a coordinate that is not finite, a latitude beyond the projectable 84°.
#[test]
fn a_position_of_another_kind_is_refused_and_one_not_placed_drops_the_trip() {
    let net = pair(false, STREET);
    let rows = [trip("t", lon_lat(4.36, 52.0), lon_lat(4.37, 52.0), WALK)];
    assert_eq!(
        refused(&rows, &one_class(), &net, None),
        DemandError::PositionNotTaken { trip: "t".into() }
    );
    let rows = [
        trip("t", at(0.0, 0.0), at(f64::NAN, 0.0), WALK),
        trip("u", at(0.0, 0.0), at(1.0, 0.0), WALK),
    ];
    let (d, kinds) = repaired(&rows, &one_class(), &net, None);
    assert_eq!((ids(&d), kinds), (vec!["u"], vec![("trips_unplaced", 1, vec!["t".to_owned()])]));
    let net = pair(true, STREET);
    let rows = [trip("t", at(0.0, 0.0), at(1_000.0, 0.0), WALK)];
    let e = refused(&rows, &one_class(), &net, None);
    assert_eq!(
        e,
        DemandError::PositionNotTaken { trip: "t".into() },
        "metres on longitude/latitude"
    );
    let rows = [trip("t", lon_lat(4.36, 52.0), lon_lat(4.36, 89.0), WALK)];
    let (d, kinds) = repaired(&rows, &one_class(), &net, None);
    assert_eq!((d.trip_count(), kinds), (0, vec![("trips_unplaced", 1, vec!["t".to_owned()])]));
}

#[test]
fn mixed_position_kinds_are_refused() {
    let net = pair(true, STREET);
    let rows = [
        trip("a", lon_lat(4.36, 52.0), lon_lat(4.37, 52.0), WALK),
        trip("b", at(0.0, 0.0), lon_lat(4.37, 52.0), WALK),
    ];
    let e = refused(&rows, &one_class(), &net, None);
    assert_eq!(e, DemandError::MixedKinds { trip: "b".into() });
    let rows = [trip("a", lon_lat(4.36, 52.0), at(0.0, 0.0), WALK)];
    assert_eq!(
        refused(&rows, &one_class(), &net, None),
        DemandError::MixedKinds { trip: "a".into() }
    );
    // One kind throughout is taken, projected with the network's projection: 0.01° of
    // longitude at 52° N is about 686 m, so the two ends link to a and b with gaps under 1 m.
    let rows = [trip("a", lon_lat(4.36, 52.0), lon_lat(4.37, 52.0), WALK)];
    let demand = Demand::build(&rows, &one_class(), &net, None, Metres(1_500.0)).expect("builds");
    let t = TripId::new(0);
    let (o, d) = (
        demand.origin_end(t, Mode::Walk).expect("in"),
        demand.destination_end(t, Mode::Walk).expect("in"),
    );
    assert!(o.gap.get() < 1e-6 && d.gap.get() < 1e-6, "the trip's ends are the nodes");
    let metres = demand.origin(t).distance_to(demand.destination(t));
    assert!((680.0..690.0).contains(&metres), "{metres} m between the ends");
}

/// A mode no link allows is taken out of the trip's set, noted per mode; a trip left with none
/// is dropped. On footways, bike is taken out of {walk, bike}; on a car-only network, transit
/// (linked by walking) out of {transit}, which drops the trip.
#[test]
fn a_mode_no_link_allows_is_taken_out_of_the_set() {
    let net = pair(false, FOOTWAY);
    let rows = [trip("t", at(0.0, 0.0), at(1.0, 0.0), &[Mode::Walk, Mode::Bike])];
    let (d, kinds) = repaired(&rows, &one_class(), &net, None);
    assert_eq!(d.modes(TripId::new(0)), ModeSet::of(WALK));
    assert_eq!(kinds, [("trips_without_bike_network", 1, vec!["t".to_owned()])]);
    let net = pair(false, CARS_ONLY);
    let rows = [trip("t", at(0.0, 0.0), at(1.0, 0.0), &[Mode::Transit])];
    let (d, kinds) = repaired(&rows, &one_class(), &net, None);
    assert_eq!(d.trip_count(), 0);
    let keys: Vec<_> = kinds.iter().map(|k| (k.0, k.1)).collect();
    assert_eq!(keys, [("trips_without_transit_network", 1), ("trips_without_mode", 1)]);
}

/// With a timetable and no link allowing walking, no stop is linked and the build goes on: the
/// demand still counts the timetable's stops.
#[test]
fn a_stop_without_a_walk_node_is_not_linked() {
    let net = pair(false, CARS_ONLY);
    let rows = [trip("t", at(0.0, 0.0), at(1.0, 0.0), &[Mode::Car])];
    let tt = stops(&[("s1", 0.0, 0.0)]);
    let (d, kinds) = repaired(&rows, &one_class(), &net, Some(&tt));
    assert_eq!((d.trip_count(), d.stop_count(), kinds.len()), (1, 1, 0));
}

#[test]
fn a_stop_walk_max_below_zero_or_not_finite_is_refused() {
    let net = pair(false, STREET);
    let rows = [trip("t", at(0.0, 0.0), at(1.0, 0.0), WALK)];
    for metres in [-1.0, f64::NAN, f64::INFINITY] {
        let e =
            Demand::build(&rows, &one_class(), &net, None, Metres(metres)).expect_err("refused");
        assert!(matches!(e, DemandError::StopWalkMax { .. }), "{metres}: {e}");
    }
    assert!(Demand::build(&rows, &one_class(), &net, None, Metres(0.0)).is_ok(), "0 m is taken");
}
