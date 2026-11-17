//! The tag rules by hand, from the OpenStreetMap wiki pages they read (`Key:highway`,
//! `Key:access`, `Key:oneway`, `Key:lanes`, `Key:maxspeed`, `Key:cycleway`, "Default speed
//! limits") and version 1's rules: a way's road class, who may use it in which direction, a car
//! link's lanes and speed, the nodes that split a way, and what the reader repairs. Most cases
//! call the rule itself; the end-to-end ones read a two-node street (way 100 from node 1 to node
//! 2, about 69 m) with a two-way street back (way 900), so every layer can return. Directions:
//! `f` along the way's node order, `b` against it. Bikes: every bike rule has its own case.

#![allow(clippy::float_cmp, reason = "speeds are the tagged numbers, exact by construction")]

mod common;

use common::{close, direction, modes, read, street};
use openmobisim_io_osm::tags::{
    self, Car, CarNotes, Direction, IMPLICIT_SPEED_LIMITS_KM_H, KM_PER_MILE, RoadClass,
    WALKING_SPEED_KM_H, bike_way, maxspeed, road_class, walk_way,
};
use openmobisim_types::mode::Mode;

fn tags(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
}

/// The car fields each way of a way of `class` with `pairs`, and what was noted.
fn car(class: RoadClass, pairs: &[(&str, &str)]) -> ((Option<Car>, Option<Car>), CarNotes) {
    let t = tags(pairs);
    let mut notes = CarNotes::default();
    let road = tags::direction(&t, class);
    (tags::car_fields(&t, road, class.car().expect("a car class"), &mut notes), notes)
}

/// The lanes each way (0 where cars do not drive).
fn lanes(class: RoadClass, pairs: &[(&str, &str)]) -> (u8, u8) {
    let ((f, b), _) = car(class, pairs);
    (f.map_or(0, |c| c.lanes), b.map_or(0, |c| c.lanes))
}

/// The speeds each way, km/h.
fn speeds(class: RoadClass, pairs: &[(&str, &str)]) -> (Option<f64>, Option<f64>) {
    let ((f, b), _) = car(class, pairs);
    (f.map(|c| c.speed_km_h), b.map(|c| c.speed_km_h))
}

/// Whether bikes may ride a way with `pairs` (two nodes) along and against its node order.
fn bike(pairs: &[(&str, &str)]) -> Option<(bool, bool)> {
    bike_way(&tags(pairs), 2).map(|(_, d)| (d.has_forward(), d.has_backward()))
}

/// The modes each way of way 100 with `pairs`, end to end (`-` for none).
fn both_modes(pairs: &[(&str, &str)]) -> (String, String) {
    let (net, _) = read(&street(2, pairs)).expect("readable");
    (modes(&net, 100, "f"), modes(&net, 100, "b"))
}

fn pair(f: &str, b: &str) -> (String, String) {
    (f.to_owned(), b.to_owned())
}

// --- the road class (version 1's `classify`) ------------------------------------------------

#[test]
fn a_road_is_classed_and_anything_else_is_no_road() {
    let class = |pairs: &[(&str, &str)], n| road_class(&tags(pairs), n);
    assert_eq!(class(&[("highway", "residential")], 2), Some(RoadClass::Residential));
    assert_eq!(class(&[("highway", "road")], 2), Some(RoadClass::Unclassified));
    assert_eq!(class(&[("highway", "path")], 2), Some(RoadClass::Footway));
    assert_eq!(class(&[("building", "yes")], 5), None);
    assert_eq!(class(&[("highway", "proposed")], 2), None);
    assert_eq!(class(&[("highway", "pedestrian"), ("area", "yes")], 4), None);
    assert_eq!(class(&[("highway", "service"), ("access", "private")], 2), None);
    assert_eq!(class(&[("highway", "residential")], 1), None);
    assert_eq!(class(&[("route", "ferry")], 2), None, "a ferry is no road");
    // A restriction to a destination is on through traffic, not on the street.
    for value in ["yes", "destination", "permissive", "customers"] {
        let t = [("highway", "residential"), ("access", value)];
        assert_eq!(class(&t, 2), Some(RoadClass::Residential), "access={value}");
    }
}

#[test]
fn a_way_closed_to_cars_by_its_most_specific_key_is_no_car_road() {
    for pairs in [
        &[("motor_vehicle", "no")][..],
        &[("motor_vehicle", "private")],
        &[("motorcar", "no")],
        &[("vehicle", "no")],
        &[("vehicle", "private")],
        &[("vehicle", "yes"), ("motor_vehicle", "no")],
    ] {
        let mut all = vec![("highway", "residential")];
        all.extend_from_slice(pairs);
        assert_eq!(road_class(&tags(&all), 2), None, "{pairs:?}");
    }
    for pairs in [
        &[("vehicle", "no"), ("motor_vehicle", "yes")][..],
        &[("motor_vehicle", "no"), ("motorcar", "yes")],
        &[("motor_vehicle", "destination")],
        &[("motorcar", "customers")],
    ] {
        let mut all = vec![("highway", "residential")];
        all.extend_from_slice(pairs);
        assert_eq!(road_class(&tags(&all), 2), Some(RoadClass::Residential), "{pairs:?}");
    }
    // Not a car class to begin with: nothing to close (a road, no car road).
    let footway = [("highway", "footway"), ("motor_vehicle", "no")];
    assert_eq!(road_class(&tags(&footway), 2), Some(RoadClass::Footway));
}

#[test]
fn parking_aisles_and_driveways_are_no_car_roads_but_alleys_and_plain_service_are() {
    for service in ["parking_aisle", "driveway", "drive-through", "emergency_access"] {
        let t = [("highway", "service"), ("service", service)];
        assert_eq!(road_class(&tags(&t), 2), None, "service={service}");
        assert_eq!(bike(&t), Some((true, true)), "bikes keep it: service={service}");
    }
    for pairs in [
        &[("highway", "service")][..],
        &[("highway", "service"), ("service", "alley")],
        &[("highway", "residential"), ("service", "driveway")],
    ] {
        assert!(road_class(&tags(pairs), 2).is_some(), "{pairs:?}");
    }
}

#[test]
fn the_class_table_reads_as_its_readme_states() {
    // The parameter osm_class_defaults (PARAMETERS.md): the README's table, (highway, km/h,
    // lanes per direction, veh/h per lane) for every car class. Speeds and lanes: version 1's.
    // Capacities, junctions counted in: 2 000 (ICTAVRU), 1 800 a carriageway, 1 550 a ramp
    // (Cerema, "Fondamentaux de la conception routière"); at signals 1 800 × the green share
    // (Cerema 2016): 0.60, 0.50, 0.45, 0.40, 1/3, 0.25 by class (the project's assumption).
    let table = [
        ("motorway", 110.0, 2, 2000.0),
        ("motorway_link", 60.0, 1, 1550.0),
        ("trunk", 90.0, 2, 1800.0),
        ("trunk_link", 50.0, 1, 1550.0),
        ("primary", 60.0, 2, 1080.0),
        ("primary_link", 40.0, 1, 900.0),
        ("secondary", 50.0, 1, 900.0),
        ("secondary_link", 40.0, 1, 900.0),
        ("tertiary", 50.0, 1, 810.0),
        ("tertiary_link", 30.0, 1, 720.0),
        ("unclassified", 40.0, 1, 720.0),
        ("road", 40.0, 1, 720.0),
        ("residential", 30.0, 1, 600.0),
        ("living_street", 20.0, 1, 450.0),
        ("service", 20.0, 1, 450.0),
    ];
    for (highway, km_h, n, per_lane) in table {
        let class = RoadClass::from_highway(highway).expect(highway);
        let d = class.car().expect(highway);
        assert_eq!((d.speed_km_h, d.lanes, d.capacity_per_lane), (km_h, n, per_lane), "{highway}");
        // End to end, both ways: the class's lanes, speed and lanes × capacity per lane.
        let (net, _) = read(&street(2, &[("highway", highway), ("oneway", "no")])).unwrap();
        for tag in ["f", "b"] {
            let c = direction(&net, Mode::Car, 100, tag).expect(highway);
            let want = (Some(n), Some(km_h), Some(f64::from(n) * per_lane));
            assert_eq!((c.lanes, c.speed_km_h, c.capacity_veh_h), want, "{highway} {tag}");
        }
    }
    let car_classes = RoadClass::ALL.iter().filter(|c| c.car().is_some()).count();
    assert_eq!(car_classes, table.len() - 1, "every car class is in the table (road is one)");
    for highway in ["construction", "proposed", "platform", "raceway", "bridleway"] {
        assert!(RoadClass::from_highway(highway).is_none(), "{highway} is no road");
    }
}

// --- direction ------------------------------------------------------------------------------

#[test]
fn oneway_decides_else_roundabouts_and_motorways_are_one_way() {
    let d = |pairs: &[(&str, &str)], class| tags::direction(&tags(pairs), class);
    let r = RoadClass::Residential;
    for yes in ["yes", "true", "1"] {
        assert_eq!(d(&[("oneway", yes)], r), Direction::Forward);
    }
    for back in ["-1", "reverse"] {
        assert_eq!(d(&[("oneway", back)], r), Direction::Backward);
    }
    for no in ["no", "false", "0"] {
        assert_eq!(d(&[("oneway", no)], r), Direction::Both);
    }
    assert_eq!(d(&[("junction", "roundabout")], RoadClass::Secondary), Direction::Forward);
    assert_eq!(d(&[], RoadClass::Motorway), Direction::Forward);
    assert_eq!(d(&[], RoadClass::MotorwayLink), Direction::Forward);
    assert_eq!(d(&[], RoadClass::Primary), Direction::Both);
    assert_eq!(d(&[("oneway", "no")], RoadClass::Motorway), Direction::Both);
    assert_eq!(d(&[("junction", "roundabout"), ("oneway", "no")], r), Direction::Both);
    // A direction that changes over the day is read as both ways.
    for value in ["reversible", "alternating"] {
        assert_eq!(d(&[("oneway", value)], RoadClass::Primary), Direction::Both);
    }
}

// --- lanes ----------------------------------------------------------------------------------

#[test]
fn lanes_follow_the_wiki_with_one_side_given() {
    let p = RoadClass::Primary;
    assert_eq!(
        lanes(p, &[("lanes", "5"), ("lanes:forward", "3"), ("lanes:backward", "2")]),
        (3, 2)
    );
    // Bug fixed: total 4, forward 1 on a two-way street gives backward 4 − 1 = 3 (version 1: 4 / 2).
    assert_eq!(lanes(p, &[("lanes", "4"), ("lanes:forward", "1")]), (1, 3));
    assert_eq!(lanes(p, &[("lanes", "4"), ("lanes:backward", "1")]), (3, 1));
    // A centre lane belongs to neither side: 5 − 2 − 1 = 2.
    assert_eq!(
        lanes(p, &[("lanes", "5"), ("lanes:forward", "2"), ("lanes:both_ways", "1")]),
        (2, 2)
    );
    // The total alone is split, rounding down, at least 1: 4 gives 2 and 2; 3 and 1 give 1 and 1.
    assert_eq!(lanes(p, &[("lanes", "4")]), (2, 2));
    assert_eq!(lanes(p, &[("lanes", "3")]), (1, 1));
    assert_eq!(lanes(p, &[("lanes", "1")]), (1, 1));
    // One way: the total is the one direction's.
    assert_eq!(lanes(p, &[("oneway", "yes"), ("lanes", "3")]), (3, 0));
    // No lane tag, or one that is not a count: the class's (primary, 2 per direction).
    assert_eq!(lanes(p, &[]), (2, 2));
    assert_eq!(lanes(p, &[("lanes", "two")]), (2, 2));
}

// --- speed ----------------------------------------------------------------------------------

#[test]
fn the_ordinary_speed_formats_all_read() {
    assert_eq!(maxspeed("50"), Some(50.0));
    assert_eq!(maxspeed(" 50 "), Some(50.0));
    assert_eq!(maxspeed("50 km/h"), Some(50.0));
    assert_eq!(maxspeed("50km/h"), Some(50.0));
    assert_eq!(maxspeed("50 kph"), Some(50.0));
    assert_eq!(maxspeed("30 mph"), Some(30.0 * KM_PER_MILE));
    assert_eq!(maxspeed("30mph"), Some(30.0 * KM_PER_MILE));
    assert_eq!(maxspeed("10 knots"), Some(18.52));
    assert_eq!(maxspeed("walk"), Some(WALKING_SPEED_KM_H));
    // A list: the limit at the way's start.
    assert_eq!(maxspeed("50;30"), Some(50.0));
    // 30 mph is 30 × 1.609 344 = 48.280 32 km/h.
    assert!(close(30.0 * KM_PER_MILE, 48.280_32, 1e-12));
}

#[test]
fn implicit_country_codes_read_the_table_with_the_country_ignored() {
    assert_eq!(maxspeed("NL:urban"), Some(50.0));
    assert_eq!(maxspeed("FR:rural"), Some(80.0));
    assert_eq!(maxspeed("FR:motorway"), Some(130.0));
    assert_eq!(maxspeed("NL:trunk"), Some(100.0));
    assert_eq!(maxspeed("DE:living_street"), Some(15.0));
    assert_eq!(maxspeed("FR:zone30"), Some(30.0));
    assert_eq!(maxspeed("NL:zone20"), Some(20.0));
    for (kind, km_h) in IMPLICIT_SPEED_LIMITS_KM_H {
        assert_eq!(maxspeed(&format!("LU:{kind}")), Some(km_h), "{kind}");
    }
    assert_eq!(maxspeed("NL:urban;30"), Some(50.0));
}

#[test]
fn a_value_that_is_not_a_speed_reads_as_none() {
    // Germany's motorway has no limit; `none`; a code the table lacks; a malformed prefix.
    for value in ["DE:motorway", "none", "DE:bicycle_road", "XX:signals", "de:urban", "signals"] {
        assert_eq!(maxspeed(value), None, "{value}");
    }
    for value in ["-20", "0", "", "fast"] {
        assert_eq!(maxspeed(value), None, "{value}");
    }
}

#[test]
fn maxspeed_by_direction_is_read_first() {
    let s = RoadClass::Secondary;
    // Bug fixed: maxspeed:forward=30 and maxspeed:backward=70 give 30 and 70 km/h.
    let by_direction = [("maxspeed:forward", "30"), ("maxspeed:backward", "70")];
    assert_eq!(speeds(s, &by_direction), (Some(30.0), Some(70.0)));
    assert_eq!(
        speeds(s, &[("maxspeed", "50"), ("maxspeed:backward", "70")]),
        (Some(50.0), Some(70.0))
    );
    // No maxspeed: the class's speed (secondary, 50 km/h).
    assert_eq!(speeds(s, &[]), (Some(50.0), Some(50.0)));
    // A one-way street has no backward car field.
    assert_eq!(speeds(s, &[("oneway", "yes")]), (Some(50.0), None));
}

// --- nodes ----------------------------------------------------------------------------------

#[test]
fn signals_stops_and_barriers_split_a_way_and_a_signal_is_marked() {
    let t = |pairs: &[(&str, &str)]| tags(pairs);
    assert!(tags::splits(&t(&[("highway", "traffic_signals")])));
    assert!(tags::splits(&t(&[("highway", "stop")])));
    assert!(tags::splits(&t(&[("highway", "mini_roundabout")])));
    assert!(tags::splits(&t(&[("barrier", "gate")])));
    assert!(!tags::splits(&t(&[("highway", "crossing")])));
    assert!(!tags::splits(&[]));
    assert!(tags::signalised(&t(&[("highway", "traffic_signals")])));
    assert!(!tags::signalised(&t(&[("highway", "stop")])));
    // An emergency signal is green for traffic but when a fire engine turns out.
    let emergency = [("highway", "traffic_signals"), ("traffic_signals", "emergency")];
    assert!(!tags::signalised(&t(&emergency)) && tags::splits(&t(&emergency)));
}

// --- bikes (version 1's `bike_way`) ---------------------------------------------------------

#[test]
fn motorways_and_ways_closed_to_bikes_are_no_bike_ways() {
    for pairs in [
        &[("highway", "motorway")][..],
        &[("highway", "motorway_link")],
        &[("highway", "primary"), ("motorroad", "yes")],
        &[("highway", "primary"), ("bicycle", "no")],
        &[("highway", "primary"), ("bicycle", "use_sidepath")],
        &[("highway", "service"), ("access", "private")],
        &[("highway", "pedestrian"), ("area", "yes"), ("bicycle", "yes")],
        &[("highway", "busway")],
        &[("building", "yes")],
    ] {
        assert_eq!(bike(pairs), None, "{pairs:?}");
    }
    assert_eq!(bike_way(&tags(&[("highway", "residential")]), 1), None, "one node");
}

#[test]
fn every_other_car_road_and_a_cycleway_are_bike_ways_both_ways() {
    for highway in ["primary", "secondary", "tertiary", "unclassified", "residential"]
        .into_iter()
        .chain(["living_street", "service", "cycleway"])
    {
        assert_eq!(bike(&[("highway", highway)]), Some((true, true)), "{highway}");
    }
    let (class, _) = bike_way(&tags(&[("highway", "cycleway")]), 2).unwrap();
    assert_eq!(class, RoadClass::Cycleway);
}

#[test]
fn a_trunk_road_or_busway_carries_bikes_only_where_a_tag_lets_them_on() {
    for highway in ["trunk", "trunk_link", "busway"] {
        assert_eq!(bike(&[("highway", highway)]), None, "plain {highway}");
        assert_eq!(bike(&[("highway", highway), ("bicycle", "dismount")]), None, "{highway}");
        for value in ["yes", "designated", "permissive"] {
            assert_eq!(bike(&[("highway", highway), ("bicycle", value)]), Some((true, true)));
        }
        assert_eq!(bike(&[("highway", highway), ("bicycle", "yes"), ("motorroad", "yes")]), None);
    }
    // A bike-only road behind access=no.
    for value in ["yes", "designated", "permissive"] {
        let t = [("highway", "service"), ("access", "no"), ("bicycle", value)];
        assert_eq!(bike(&t), Some((true, true)), "access=no, bicycle={value}");
    }
}

#[test]
fn footways_paths_tracks_steps_and_pedestrian_streets_need_a_bicycle_tag() {
    // Version 1's rule: a path or a track is a footway; a bike rides it only where tagged.
    for highway in ["footway", "path", "track", "pedestrian", "steps", "bridleway"] {
        assert_eq!(bike(&[("highway", highway)]), None, "{highway} without a bicycle tag");
        for value in ["yes", "designated", "permissive", "dismount"] {
            let t = [("highway", highway), ("bicycle", value)];
            assert_eq!(bike(&t), Some((true, true)), "{highway} bicycle={value}");
        }
    }
}

#[test]
fn bikes_follow_a_one_way_street_unless_a_tag_opens_it() {
    let r = ("highway", "residential");
    let one = ("oneway", "yes");
    assert_eq!(bike(&[r, one]), Some((true, false)));
    assert_eq!(bike(&[r, one, ("oneway:bicycle", "no")]), Some((true, true)));
    assert_eq!(bike(&[r, ("oneway:bicycle", "yes")]), Some((true, false)));
    assert_eq!(bike(&[r, ("oneway:bicycle", "-1")]), Some((false, true)));
    assert_eq!(bike(&[r, ("junction", "roundabout")]), Some((true, false)));
    assert_eq!(bike(&[("highway", "cycleway"), one]), Some((true, false)));
    // An `opposite…` value, on any cycleway key, opens the contraflow.
    for key in ["cycleway", "cycleway:both", "cycleway:left", "cycleway:right"] {
        for value in ["opposite", "opposite_lane", "opposite_track", "opposite_share_busway"] {
            assert_eq!(bike(&[r, one, (key, value)]), Some((true, true)), "{key}={value}");
        }
    }
    // A lane, track or shared bus lane whose own `oneway` runs against the street opens it.
    for value in ["lane", "track", "share_busway"] {
        let t = [r, one, ("cycleway:left", value), ("cycleway:left:oneway", "-1")];
        assert_eq!(bike(&t), Some((true, true)), "{value}");
    }
    // A lane alone serves the street's direction; mirrored on a street one-way the other way.
    assert_eq!(bike(&[r, one, ("cycleway:left", "lane")]), Some((true, false)));
    assert_eq!(bike(&[r, ("oneway", "-1"), ("cycleway", "opposite_lane")]), Some((true, true)));
    assert_eq!(bike(&[r, ("oneway", "-1"), ("oneway:bicycle", "yes")]), Some((false, true)));
}

#[test]
fn a_ferry_carries_bikes_and_walkers_unless_told_not_to() {
    let ferry = |pairs: &[(&str, &str)]| bike_way(&tags(pairs), 2);
    assert_eq!(ferry(&[("route", "ferry")]), Some((RoadClass::Ferry, Direction::Both)));
    assert_eq!(
        walk_way(&tags(&[("route", "ferry")]), 2),
        Some((RoadClass::Ferry, Direction::Both))
    );
    assert_eq!(ferry(&[("route", "ferry"), ("bicycle", "no")]), None);
    assert_eq!(walk_way(&tags(&[("route", "ferry"), ("foot", "no")]), 2), None);
    assert_eq!(
        ferry(&[("route", "ferry"), ("oneway", "yes")]),
        Some((RoadClass::Ferry, Direction::Forward))
    );
}

// --- walkers (version 1's `walk_way`) -------------------------------------------------------

#[test]
fn walking_is_everywhere_but_motorways_busways_and_closed_ways_and_both_ways() {
    for highway in
        ["primary", "trunk", "residential", "footway", "path", "steps", "pedestrian", "cycleway"]
    {
        let t = tags(&[("highway", highway), ("oneway", "yes")]);
        assert_eq!(walk_way(&t, 2).map(|(_, d)| d), Some(Direction::Both), "{highway}");
    }
    for pairs in [
        &[("highway", "motorway")][..],
        &[("highway", "motorway_link")],
        &[("highway", "busway")],
        &[("highway", "cycleway"), ("foot", "no")],
        &[("highway", "primary"), ("foot", "no")],
        &[("highway", "primary"), ("foot", "use_sidepath")],
        &[("highway", "trunk"), ("motorroad", "yes")],
        &[("highway", "service"), ("access", "private")],
        &[("highway", "pedestrian"), ("area", "yes")],
    ] {
        assert_eq!(walk_way(&tags(pairs), 2), None, "{pairs:?}");
    }
    let open = [("highway", "service"), ("access", "no"), ("foot", "designated")];
    assert!(walk_way(&tags(&open), 2).is_some());
    assert!(walk_way(&tags(&[("highway", "busway"), ("foot", "yes")]), 2).is_some());
}

// --- end to end -----------------------------------------------------------------------------

#[test]
fn each_layer_gets_the_ways_its_rules_admit() {
    assert_eq!(both_modes(&[("highway", "residential")]), pair("cbw", "cbw"));
    assert_eq!(both_modes(&[("highway", "residential"), ("oneway", "yes")]), pair("cbw", "w"));
    assert_eq!(both_modes(&[("highway", "motorway"), ("oneway", "no")]), pair("c", "c"));
    assert_eq!(both_modes(&[("highway", "cycleway")]), pair("bw", "bw"));
    assert_eq!(both_modes(&[("highway", "footway")]), pair("w", "w"));
    assert_eq!(both_modes(&[("highway", "path")]), pair("w", "w"));
    assert_eq!(both_modes(&[("highway", "footway"), ("bicycle", "yes")]), pair("bw", "bw"));
    assert_eq!(both_modes(&[("highway", "trunk")]), pair("cw", "cw"));
    assert_eq!(both_modes(&[("highway", "residential"), ("access", "private")]), pair("-", "-"));
    assert_eq!(
        both_modes(&[("highway", "service"), ("service", "parking_aisle")]),
        pair("bw", "bw")
    );
    // A cycleway is never a car road, whatever its car keys say (mopeds on Dutch cycleways).
    let mopeds = [("highway", "cycleway"), ("motor_vehicle", "destination")];
    assert_eq!(both_modes(&mopeds), pair("bw", "bw"));
}

#[test]
fn what_every_rule_means_for_a_bike() {
    let ride = |pairs: &[(&str, &str)]| {
        let (f, b) = both_modes(pairs);
        (f.contains('b'), b.contains('b'))
    };
    let r = ("highway", "residential");
    assert_eq!(ride(&[r]), (true, true));
    assert_eq!(ride(&[r, ("bicycle", "no")]), (false, false));
    assert_eq!(ride(&[r, ("vehicle", "no")]), (true, true), "version 1: vehicle binds cars here");
    assert_eq!(ride(&[r, ("access", "private")]), (false, false));
    assert_eq!(ride(&[r, ("access", "no"), ("bicycle", "designated")]), (true, true));
    assert_eq!(ride(&[r, ("motor_vehicle", "no")]), (true, true), "a car key binds no bike");
    assert_eq!(ride(&[("highway", "footway"), ("bicycle", "dismount")]), (true, true));
    assert_eq!(ride(&[r, ("oneway", "yes"), ("oneway:bicycle", "no")]), (true, true));
    // A bike link carries no car field, and a bike rides it at bike_speed (15 km/h here).
    let (net, _) = read(&street(2, &[r, ("maxspeed", "30")])).unwrap();
    let b = direction(&net, Mode::Bike, 100, "f").unwrap();
    assert_eq!((b.lanes, b.speed_km_h, b.capacity_veh_h), (None, None, None));
    let seconds = net.free_flow_time(b.links[0], Mode::Bike).0;
    assert!(close(seconds, b.length_m / (15.0 / 3.6), 1e-9), "{seconds} s at 15 km/h");
}

// --- repairs --------------------------------------------------------------------------------

/// The repairs of way 100 with `pairs`, as (key, count, first ids).
fn repairs(pairs: &[(&str, &str)]) -> Vec<(&'static str, u64, Vec<String>)> {
    let (_, report) = read(&street(2, pairs)).expect("readable");
    report.repairs.kinds().iter().map(|k| (k.key, k.count, k.first.clone())).collect()
}

#[test]
fn a_tag_the_reader_cannot_read_is_read_as_absent_and_noted() {
    let way = vec!["way 100".to_string()];
    let residential = |extra: (&'static str, &'static str)| [("highway", "residential"), extra];
    let car_both = |pairs: &[(&str, &str)]| {
        let (net, _) = read(&street(2, pairs)).unwrap();
        let of = |tag| direction(&net, Mode::Car, 100, tag).map(|d| (d.lanes, d.speed_km_h));
        (of("f"), of("b"))
    };
    let plain = Some((Some(1), Some(30.0)));
    for value in ["none", "DE:motorway", "XX:signals", "fast", "0", "-20"] {
        let t = residential(("maxspeed", value));
        assert_eq!(car_both(&t), (plain, plain), "{value}: the class's 30 km/h, 1 lane");
        assert_eq!(repairs(&t), [("speeds_defaulted", 1, way.clone())], "{value}");
    }
    // Version 1's readings, nothing noted: `walk` 7 km/h, `NL:urban` 50, a list its first.
    for (value, km_h) in [("walk", 7.0), ("NL:urban", 50.0), ("50;30", 50.0), ("50 km/h", 50.0)] {
        let t = residential(("maxspeed", value));
        assert_eq!(car_both(&t).0, Some((Some(1), Some(km_h))), "{value}");
        assert!(repairs(&t).is_empty(), "{value}");
    }
    let t = residential(("maxspeed:forward", "signals"));
    assert_eq!(car_both(&t).0, plain);
    assert_eq!(repairs(&t), [("speeds_defaulted", 1, way.clone())]);
    for extra in
        [("lanes", "two"), ("lanes", "2;3"), ("lanes:forward", "1.5"), ("oneway", "no;yes")]
    {
        assert_eq!(car_both(&residential(extra)), (plain, plain), "{extra:?}");
        assert_eq!(repairs(&residential(extra)), [("tags_unread", 1, way.clone())], "{extra:?}");
    }
    // Lane tags that leave a side no lane, or a total not the sum of the sides: repaired.
    let p = |extra: &[(&'static str, &'static str)]| {
        let mut t = vec![("highway", "primary")];
        t.extend_from_slice(extra);
        t
    };
    let side = |n| Some((Some(n), Some(60.0)));
    let full = p(&[("lanes", "2"), ("lanes:forward", "2")]);
    assert_eq!(
        (car_both(&full), repairs(&full)),
        ((side(2), side(1)), vec![("lanes_repaired", 1, way.clone())])
    );
    let sum = p(&[("lanes", "4"), ("lanes:forward", "2"), ("lanes:backward", "1")]);
    assert_eq!(
        (car_both(&sum), repairs(&sum)),
        ((side(2), side(1)), vec![("lanes_repaired", 1, way.clone())])
    );
    let none = p(&[("lanes:forward", "0")]);
    assert_eq!(
        (car_both(&none).0, repairs(&none)),
        (side(1), vec![("lanes_repaired", 1, way.clone())])
    );
    // `lanes=0` on a two-way road leaves each direction 0 lanes: 1 each, repaired once.
    let zero = p(&[("lanes", "0")]);
    assert_eq!(
        (car_both(&zero), repairs(&zero)),
        ((side(1), side(1)), vec![("lanes_repaired", 1, way)])
    );
    // A way cars do not drive: its car tags are not read, so not noted.
    assert!(repairs(&[("highway", "footway"), ("maxspeed", "none")]).is_empty());
    assert!(
        repairs(&[("highway", "residential"), ("motor_vehicle", "no"), ("lanes", "two")])
            .is_empty()
    );
}
