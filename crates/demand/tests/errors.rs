//! Test 4: what the demand refuses, each refusal naming the trip, class or stop at fault by
//! its external id; the first fault in id order whatever the order of the rows.

mod common;

use common::{CARS_ONLY, FOOTWAY, STREET, VALUES, at, one_class, stops, trip};
use openmobisim_demand::demand::Demand;
use openmobisim_demand::input::{DemandError, TripRow};
use openmobisim_graph::geometry::{LonLat, Position};
use openmobisim_graph::network::{LinkRow, Network, NetworkBuilder};
use openmobisim_transit::timetable::Timetable;
use openmobisim_types::ids::{EntityId, TripId};
use openmobisim_types::mode::Mode;
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

#[test]
fn a_trip_or_a_class_given_twice_is_refused() {
    let net = pair(false, STREET);
    let rows =
        [trip("t", at(0.0, 0.0), at(1.0, 0.0), WALK), trip("t", at(5.0, 0.0), at(1.0, 0.0), WALK)];
    let e = refused(&rows, &one_class(), &net, None);
    assert_eq!(e, DemandError::DuplicateTrip { trip: "t".into() });
    assert!(e.to_string().contains("trip t "));
    let classes = vec!["c".to_owned(), "d".to_owned(), "c".to_owned()];
    let e = refused(&rows[..1], &classes, &net, None);
    assert_eq!(e, DemandError::DuplicateClass { class: "c".into() });
}

#[test]
fn an_unknown_class_and_an_empty_mode_set_are_refused_first_in_id_order() {
    let net = pair(false, STREET);
    let mut lost = trip("b-lost", at(0.0, 0.0), at(1.0, 0.0), WALK);
    lost.class = "x".into();
    let idle = trip("a-idle", at(0.0, 0.0), at(1.0, 0.0), &[]);
    // Given in the other order: `a-idle` sorts first and is named.
    let e = refused(&[lost.clone(), idle], &one_class(), &net, None);
    assert_eq!(e, DemandError::NoMode { trip: "a-idle".into() });
    let e = refused(&[lost], &one_class(), &net, None);
    assert_eq!(e, DemandError::UnknownClass { trip: "b-lost".into(), class: "x".into() });
    assert!(e.to_string().contains("b-lost") && e.to_string().contains('x'));
}

#[test]
fn a_position_the_network_cannot_take_is_refused() {
    // Longitude/latitude on a network in metres (it has no projection), and below, metres on a
    // network built from longitude/latitude: the trips' kind must be the network's.
    let net = pair(false, STREET);
    let rows = [trip("t", lon_lat(4.36, 52.0), lon_lat(4.37, 52.0), WALK)];
    assert_eq!(
        refused(&rows, &one_class(), &net, None),
        DemandError::PositionNotTaken { trip: "t".into() }
    );
    // A coordinate that is not finite.
    let rows = [trip("t", at(0.0, 0.0), at(f64::NAN, 0.0), WALK)];
    assert_eq!(
        refused(&rows, &one_class(), &net, None),
        DemandError::PositionNotTaken { trip: "t".into() }
    );
    // Latitude beyond the projectable 84°, on a network in longitude/latitude.
    let net = pair(true, STREET);
    let rows = [trip("t", at(0.0, 0.0), at(1_000.0, 0.0), WALK)];
    let e = refused(&rows, &one_class(), &net, None);
    assert_eq!(
        e,
        DemandError::PositionNotTaken { trip: "t".into() },
        "metres on longitude/latitude"
    );
    let rows = [trip("t", lon_lat(4.36, 52.0), lon_lat(4.36, 89.0), WALK)];
    assert_eq!(
        refused(&rows, &one_class(), &net, None),
        DemandError::PositionNotTaken { trip: "t".into() }
    );
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

#[test]
fn a_mode_no_link_allows_is_refused_naming_the_trip_and_the_mode() {
    let net = pair(false, FOOTWAY);
    let rows = [trip("t", at(0.0, 0.0), at(1.0, 0.0), &[Mode::Walk, Mode::Bike])];
    let e = refused(&rows, &one_class(), &net, None);
    assert_eq!(e, DemandError::NoNode { trip: "t".into(), mode: Mode::Bike });
    assert!(e.to_string().contains("bike"));
    // Transit is linked by walking: on a car-only network it cannot be.
    let net = pair(false, CARS_ONLY);
    let rows = [trip("t", at(0.0, 0.0), at(1.0, 0.0), &[Mode::Transit])];
    let e = refused(&rows, &one_class(), &net, None);
    assert_eq!(e, DemandError::NoNode { trip: "t".into(), mode: Mode::Transit });
    assert!(e.to_string().contains("walking"));
}

#[test]
fn a_stop_without_a_walk_node_is_refused_naming_the_stop() {
    let net = pair(false, CARS_ONLY);
    let rows = [trip("t", at(0.0, 0.0), at(1.0, 0.0), &[Mode::Car])];
    let tt = stops(&[("s1", 0.0, 0.0)]);
    let e = refused(&rows, &one_class(), &net, Some(&tt));
    assert_eq!(e, DemandError::StopNoNode { stop: "s1".into() });
    assert!(e.to_string().contains("s1"));
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
