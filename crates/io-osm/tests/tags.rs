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
    // Country codes need an implicit-limit table, which is a v2 addition.
    assert_eq!(s("DE:urban"), Maxspeed::Unparsed);
    assert_eq!(s("RO:motorway"), Maxspeed::Unparsed);
    assert_eq!(s("signals"), Maxspeed::Unparsed);
    assert_eq!(s("-20"), Maxspeed::Unparsed);
    assert_eq!(s("0"), Maxspeed::Unparsed);

    assert!(s("none").is_noteworthy(), "an unlimited limit must be recorded");
    assert!(s("DE:urban").is_noteworthy());
    assert!(!s("50").is_noteworthy());
    assert!(!tags::maxspeed(&[]).is_noteworthy(), "an absent tag is the normal case, not news");

    // All of the above hand the defaults table nothing, so the class row applies.
    assert_eq!(s("none").value_km_h(), None);
    assert_eq!(s("DE:urban").value_km_h(), None);
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
