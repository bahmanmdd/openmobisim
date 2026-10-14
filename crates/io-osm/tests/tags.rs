//! Tag interpretation — the part of importing OpenStreetMap that is actually
//! hard, and the part most likely to be quietly wrong on real data.

#![allow(clippy::float_cmp, reason = "speed conversions are exact by construction")]

use openmobisim_core_graph::defaults::RoadClass;
use openmobisim_io_osm::tags::{
    self, Direction, KM_PER_MILE, Maxspeed, Rejection, WALKING_SPEED_KM_H,
};

fn tags(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
}

// --- classification -------------------------------------------------------

#[test]
fn a_road_is_classified_and_anything_else_is_refused_with_a_reason() {
    assert_eq!(tags::classify(&tags(&[("highway", "residential")]), 2), Ok(RoadClass::Residential));
    assert_eq!(tags::classify(&tags(&[("building", "yes")]), 5), Err(Rejection::NotAHighway));
    assert_eq!(
        tags::classify(&tags(&[("highway", "proposed")]), 2),
        Err(Rejection::UnknownHighwayClass)
    );
    assert_eq!(
        tags::classify(&tags(&[("highway", "pedestrian"), ("area", "yes")]), 4),
        Err(Rejection::IsAnArea)
    );
    assert_eq!(
        tags::classify(&tags(&[("highway", "service"), ("access", "private")]), 2),
        Err(Rejection::AccessDenied)
    );
    assert_eq!(
        tags::classify(&tags(&[("highway", "residential")]), 1),
        Err(Rejection::TooFewNodes)
    );
}

#[test]
fn access_yes_and_destination_do_not_exclude_a_way() {
    // `access=destination` is a residential street you may drive to but not
    // through. Excluding it would disconnect neighbourhoods; the restriction
    // belongs in routing, not in the network.
    for value in ["yes", "destination", "permissive", "customers"] {
        assert!(
            tags::classify(&tags(&[("highway", "residential"), ("access", value)]), 2).is_ok(),
            "access={value} should not exclude the way"
        );
    }
}

// --- direction ------------------------------------------------------------

#[test]
fn explicit_oneway_tags_win() {
    let c = RoadClass::Residential;
    for yes in ["yes", "true", "1"] {
        assert_eq!(tags::direction(&tags(&[("oneway", yes)]), c), Direction::Forward);
    }
    for back in ["-1", "reverse"] {
        assert_eq!(tags::direction(&tags(&[("oneway", back)]), c), Direction::Backward);
    }
    for no in ["no", "false", "0"] {
        assert_eq!(tags::direction(&tags(&[("oneway", no)]), c), Direction::Both);
    }
}

#[test]
fn roundabouts_and_motorways_are_one_way_by_convention() {
    assert_eq!(
        tags::direction(&tags(&[("junction", "roundabout")]), RoadClass::Secondary),
        Direction::Forward
    );
    assert_eq!(tags::direction(&[], RoadClass::Motorway), Direction::Forward);
    assert_eq!(tags::direction(&[], RoadClass::MotorwayLink), Direction::Forward);
    assert_eq!(tags::direction(&[], RoadClass::Primary), Direction::Both);
}

#[test]
fn an_explicit_oneway_no_overrides_the_conventions() {
    // A two-way motorway is unusual, but someone took the trouble to say so.
    assert_eq!(tags::direction(&tags(&[("oneway", "no")]), RoadClass::Motorway), Direction::Both);
    assert_eq!(
        tags::direction(
            &tags(&[("junction", "roundabout"), ("oneway", "no")]),
            RoadClass::Secondary
        ),
        Direction::Both
    );
}

#[test]
fn a_direction_nobody_modelled_stays_two_way() {
    // `reversible` and `alternating` describe a direction that changes over the
    // day; there is no v1 mechanism for that, so take the permissive reading
    // and keep the network connected.
    for value in ["reversible", "alternating"] {
        assert_eq!(
            tags::direction(&tags(&[("oneway", value)]), RoadClass::Primary),
            Direction::Both
        );
    }
}

// --- lanes ----------------------------------------------------------------

#[test]
fn explicit_directional_lane_counts_win() {
    let l = tags::lanes(
        &tags(&[("lanes", "5"), ("lanes:forward", "3"), ("lanes:backward", "2")]),
        Direction::Both,
    );
    assert_eq!(l.forward, Some(3));
    assert_eq!(l.backward, Some(2));
}

#[test]
fn a_lane_total_is_split_on_a_two_way_street_and_kept_on_a_one_way_one() {
    let two_way = tags::lanes(&tags(&[("lanes", "4")]), Direction::Both);
    assert_eq!(two_way.forward, Some(2));
    assert_eq!(two_way.backward, Some(2));

    let one_way = tags::lanes(&tags(&[("lanes", "3")]), Direction::Forward);
    assert_eq!(one_way.forward, Some(3));
    assert_eq!(one_way.backward, None);
}

#[test]
fn an_odd_total_rounds_down_with_a_floor_of_one() {
    // Understating capacity is the safer error: an overstated capacity gives a
    // network that never congests, and that failure is silent.
    let l = tags::lanes(&tags(&[("lanes", "3")]), Direction::Both);
    assert_eq!(l.forward, Some(1));
    assert_eq!(l.backward, Some(1));

    let single = tags::lanes(&tags(&[("lanes", "1")]), Direction::Both);
    assert_eq!(single.forward, Some(1));
    assert_eq!(single.backward, Some(1));
}

#[test]
fn an_absent_or_unusable_lane_tag_leaves_it_to_the_defaults_table() {
    assert_eq!(tags::lanes(&[], Direction::Both), tags::Lanes::default());
    let nonsense = tags::lanes(&tags(&[("lanes", "two")]), Direction::Both);
    assert_eq!(nonsense, tags::Lanes::default());
}

// --- maxspeed -------------------------------------------------------------

#[test]
fn the_ordinary_speed_formats_all_parse() {
    let s = |v: &str| tags::maxspeed(&tags(&[("maxspeed", v)]));
    assert_eq!(s("50"), Maxspeed::Known(50.0));
    assert_eq!(s(" 50 "), Maxspeed::Known(50.0));
    assert_eq!(s("50 km/h"), Maxspeed::Known(50.0));
    assert_eq!(s("50km/h"), Maxspeed::Known(50.0));
    assert_eq!(s("50 kph"), Maxspeed::Known(50.0));
    assert_eq!(s("30 mph"), Maxspeed::Known(30.0 * KM_PER_MILE));
    assert_eq!(s("30mph"), Maxspeed::Known(30.0 * KM_PER_MILE));
    assert_eq!(s("walk"), Maxspeed::Known(WALKING_SPEED_KM_H));
}

#[test]
fn a_list_takes_the_limit_that_applies_at_the_start() {
    assert_eq!(tags::maxspeed(&tags(&[("maxspeed", "50;30")])), Maxspeed::Known(50.0));
}

#[test]
fn unlimited_and_uninterpretable_limits_fall_back_and_are_flagged() {
    let s = |v: &str| tags::maxspeed(&tags(&[("maxspeed", v)]));
    assert_eq!(s("none"), Maxspeed::Unlimited);
    // A code the implicit-limit table lacks, or a malformed country prefix.
    assert_eq!(s("DE:bicycle_road"), Maxspeed::Unparsed);
    assert_eq!(s("XX:signals"), Maxspeed::Unparsed);
    assert_eq!(s("de:urban"), Maxspeed::Unparsed);
    assert_eq!(s("signals"), Maxspeed::Unparsed);
    assert_eq!(s("-20"), Maxspeed::Unparsed);
    assert_eq!(s("0"), Maxspeed::Unparsed);

    assert!(s("none").is_noteworthy(), "an unlimited limit must be recorded");
    assert!(s("XX:signals").is_noteworthy());
    assert!(!s("50").is_noteworthy());
    assert!(!tags::maxspeed(&[]).is_noteworthy(), "an absent tag is the normal case, not news");

    // All of the above hand the defaults table nothing, so the class row applies.
    assert_eq!(s("none").value_km_h(), None);
    assert_eq!(s("XX:signals").value_km_h(), None);
    assert_eq!(s("50").value_km_h(), Some(50.0));
}

#[test]
fn an_absent_speed_limit_is_not_an_error() {
    assert_eq!(tags::maxspeed(&[]), Maxspeed::Absent);
    assert_eq!(tags::maxspeed(&tags(&[("maxspeed", "")])), Maxspeed::Absent);
}

// --- node tags ------------------------------------------------------------

#[test]
fn signals_and_barriers_interrupt_a_way() {
    assert!(tags::node_splits_way(&tags(&[("highway", "traffic_signals")])));
    assert!(tags::node_splits_way(&tags(&[("highway", "stop")])));
    assert!(tags::node_splits_way(&tags(&[("barrier", "gate")])));
    assert!(!tags::node_splits_way(&tags(&[("highway", "crossing")])));
    assert!(!tags::node_splits_way(&[]));

    assert!(tags::node_is_signalised(&tags(&[("highway", "traffic_signals")])));
    assert!(!tags::node_is_signalised(&tags(&[("highway", "stop")])));
}

#[test]
fn a_signal_says_which_approaches_it_stops() {
    // S265: a stop line's signal stops one direction; an emergency signal is green for traffic.
    use tags::SignalDirection::{Backward, Both, Forward};
    let signal = |extra: &[(&str, &str)]| {
        let mut t = vec![("highway", "traffic_signals")];
        t.extend_from_slice(extra);
        tags::node_signal(&tags(&t))
    };
    assert_eq!(signal(&[]), Some(Both));
    assert_eq!(signal(&[("traffic_signals:direction", "both")]), Some(Both));
    assert_eq!(signal(&[("traffic_signals:direction", "forward")]), Some(Forward));
    assert_eq!(signal(&[("traffic_signals:direction", "backward")]), Some(Backward));
    assert_eq!(signal(&[("traffic_signals", "emergency")]), None);
    assert!(!tags::node_is_signalised(&tags(&[
        ("highway", "traffic_signals"),
        ("traffic_signals", "emergency")
    ])));
    assert_eq!(tags::node_signal(&tags(&[("highway", "stop")])), None);
}

// --- ways closed to cars (G-17) ---------------------------------------------

#[test]
fn a_way_closed_to_cars_is_not_a_car_link() {
    for pairs in [
        &[("motor_vehicle", "no")][..],
        &[("motor_vehicle", "private")],
        &[("motorcar", "no")],
        &[("vehicle", "no")],
        &[("vehicle", "private")],
        // The most specific key wins: cars are named, so `vehicle=yes` does not reopen the way.
        &[("vehicle", "yes"), ("motor_vehicle", "no")],
    ] {
        let mut all = vec![("highway", "residential")];
        all.extend_from_slice(pairs);
        assert_eq!(tags::classify(&tags(&all), 2), Err(Rejection::AccessDenied), "{pairs:?}");
    }
}

#[test]
fn a_more_specific_yes_reopens_a_way_and_destination_access_stays_drivable() {
    for pairs in [
        &[("vehicle", "no"), ("motor_vehicle", "yes")][..],
        &[("motor_vehicle", "no"), ("motorcar", "yes")],
        &[("motor_vehicle", "destination")],
        &[("motor_vehicle", "delivery")],
        &[("vehicle", "destination")],
        &[("motorcar", "customers")],
    ] {
        let mut all = vec![("highway", "residential")];
        all.extend_from_slice(pairs);
        assert_eq!(tags::classify(&tags(&all), 2), Ok(RoadClass::Residential), "{pairs:?}");
    }
}

#[test]
fn a_way_closed_to_cars_but_open_to_buses_is_a_busway_and_a_footway_is_unchanged() {
    for key in ["motor_vehicle", "motorcar", "vehicle"] {
        assert_eq!(
            tags::classify(&tags(&[("highway", "residential"), (key, "no"), ("bus", "yes")]), 2),
            Ok(RoadClass::Busway),
            "{key}=no, bus=yes"
        );
    }
    // Not a car class to begin with: nothing to close.
    assert_eq!(
        tags::classify(&tags(&[("highway", "footway"), ("motor_vehicle", "no")]), 2),
        Ok(RoadClass::Footway)
    );
}

// --- service roads (G-18) -----------------------------------------------------

#[test]
fn parking_aisles_and_driveways_are_not_car_links_but_alleys_and_plain_service_are() {
    for service in ["parking_aisle", "driveway", "drive-through", "emergency_access"] {
        assert_eq!(
            tags::classify(&tags(&[("highway", "service"), ("service", service)]), 2),
            Err(Rejection::ServiceNotAStreet),
            "service={service}"
        );
    }
    for pairs in [
        &[("highway", "service")][..],
        &[("highway", "service"), ("service", "alley")],
        &[("highway", "service"), ("service", "spur")],
        // `service=driveway` on another class is not read.
        &[("highway", "residential"), ("service", "driveway")],
    ] {
        assert!(tags::classify(&tags(pairs), 2).is_ok(), "{pairs:?}");
    }
}

// --- implicit speed limits (G-21) ---------------------------------------------

#[test]
fn implicit_country_codes_read_the_table_with_the_country_ignored() {
    let s = |v: &str| tags::maxspeed(&tags(&[("maxspeed", v)]));
    assert_eq!(s("NL:urban"), Maxspeed::Known(50.0));
    assert_eq!(s("FR:rural"), Maxspeed::Known(80.0));
    assert_eq!(s("BE:urban"), Maxspeed::Known(50.0));
    assert_eq!(s("FR:motorway"), Maxspeed::Known(130.0));
    assert_eq!(s("NL:trunk"), Maxspeed::Known(100.0));
    assert_eq!(s("DE:living_street"), Maxspeed::Known(15.0));
    assert_eq!(s("FR:zone30"), Maxspeed::Known(30.0));
    assert_eq!(s("NL:zone20"), Maxspeed::Known(20.0));
    // Every row of the table, under an arbitrary country.
    for (kind, km_h) in tags::IMPLICIT_SPEED_LIMITS_KM_H {
        assert_eq!(s(&format!("LU:{kind}")), Maxspeed::Known(km_h), "{kind}");
    }
    // A list takes its first entry, as ever.
    assert_eq!(s("NL:urban;30"), Maxspeed::Known(50.0));
    // Germany's motorway has no limit: the class default applies, and it is recorded.
    assert_eq!(s("DE:motorway"), Maxspeed::Unlimited);
    assert_eq!(s("DE:motorway").value_km_h(), None);
    assert_eq!(s("NL:urban").value_km_h(), Some(50.0));
    assert!(!s("NL:urban").is_noteworthy());
}
