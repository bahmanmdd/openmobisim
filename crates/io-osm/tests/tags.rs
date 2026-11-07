//! The tags against the OpenStreetMap wiki, by hand: who may use a way and in which direction,
//! the lanes and the free-flow speed of each car direction, and the values the reader refuses.
//!
//! Each case is a two-node street (way 100 from node 1 to node 2, about 69 m) with the tags
//! the wiki page names, and a two-way street back (way 900) so that cars can always return; the expected modes, lanes and speeds are read off the page. Directions:
//! `f` along the way's node order, `b` against it. Bikes: every rule says what it means for a
//! bike (the `b` in the mode letters).

#![allow(clippy::float_cmp, reason = "speeds are the tagged numbers, rounded to 1e-9 km/h")]

mod common;

use common::{close, direction, read, street};
use openmobisim_io_osm::OsmError;
use openmobisim_io_osm::car::KM_PER_MILE;
use openmobisim_io_osm::class::{CLASSES, class_of};

/// The modes of each direction of a two-node street with `tags` (`-` for no link).
fn modes(tags: &[(&str, &str)]) -> (String, String) {
    let (net, _) = read(&street(2, tags)).expect("readable");
    let of = |t| direction(&net, 100, t).map_or("-".to_owned(), |d| d.modes);
    (of("f"), of("b"))
}

fn pair(f: &str, b: &str) -> (String, String) {
    (f.to_owned(), b.to_owned())
}

/// A car direction's lanes and free-flow speed in km/h.
type Car = (u8, f64);

/// The car's (lanes, km/h) each way on a two-node street with `tags`. The graph holds speeds
/// in m/s; read back in km/h they are rounded to 1e-9 km/h.
fn car(tags: &[(&str, &str)]) -> (Option<Car>, Option<Car>) {
    let (net, _) = read(&street(2, tags)).expect("readable");
    let of = |t| {
        let d = direction(&net, 100, t)?;
        Some((d.lanes?, (d.speed_km_h? * 1e9).round() / 1e9))
    };
    (of("f"), of("b"))
}

#[test]
fn a_cycleway_is_bike_and_walk_with_no_car_field() {
    // Wiki, access defaults, Netherlands: cycleway motorcar no, bicycle designated, foot yes.
    assert_eq!(modes(&[("highway", "cycleway")]), pair("bw", "bw"));
    let (net, _) = read(&street(2, &[("highway", "cycleway")])).unwrap();
    let f = direction(&net, 100, "f").unwrap();
    assert_eq!((f.lanes, f.speed_km_h), (None, None), "no car field on a cycle path");
}

#[test]
fn a_motorway_is_car_only_and_one_way() {
    // Motorway: motorcar designated, bicycle no, foot no; one-way by the Vienna Convention.
    assert_eq!(modes(&[("highway", "motorway")]), pair("c", "-"));
    assert_eq!(modes(&[("highway", "motorway_link")]), pair("c", "-"));
    assert_eq!(modes(&[("highway", "motorway"), ("oneway", "no")]), pair("c", "c"));
}

#[test]
fn a_street_is_car_bike_and_walk_both_ways() {
    for class in ["trunk", "primary", "secondary", "tertiary", "unclassified", "residential"] {
        assert_eq!(modes(&[("highway", class)]), pair("cbw", "cbw"), "{class}");
    }
    assert_eq!(modes(&[("highway", "living_street")]), pair("cbw", "cbw"));
    assert_eq!(modes(&[("highway", "service")]), pair("cbw", "cbw"));
}

#[test]
fn bicycle_no_removes_the_bike_and_leaves_the_rest() {
    assert_eq!(modes(&[("highway", "residential"), ("bicycle", "no")]), pair("cw", "cw"));
    assert_eq!(modes(&[("highway", "residential"), ("bicycle", "use_sidepath")]), pair("cw", "cw"));
    assert_eq!(modes(&[("highway", "residential"), ("foot", "no")]), pair("cb", "cb"));
}

#[test]
fn footways_paths_and_steps_follow_the_wiki() {
    // Footway: bicycle dismount (a bike pushed is a walker), foot designated.
    assert_eq!(modes(&[("highway", "footway")]), pair("w", "w"));
    assert_eq!(modes(&[("highway", "footway"), ("bicycle", "yes")]), pair("bw", "bw"));
    assert_eq!(modes(&[("highway", "footway"), ("bicycle", "dismount")]), pair("w", "w"));
    assert_eq!(modes(&[("highway", "pedestrian")]), pair("w", "w"));
    assert_eq!(modes(&[("highway", "steps")]), pair("w", "w"));
    // Path: bicycle yes, foot yes. Track: bike and walk (cars: the reader's one departure).
    assert_eq!(modes(&[("highway", "path")]), pair("bw", "bw"));
    assert_eq!(modes(&[("highway", "track")]), pair("bw", "bw"));
    assert_eq!(modes(&[("highway", "bridleway"), ("bicycle", "yes")]), pair("b", "b"));
}

#[test]
fn the_most_specific_access_key_decides() {
    // Key:access: access < vehicle < motor_vehicle < motorcar; access < vehicle < bicycle.
    let r = ("highway", "residential");
    assert_eq!(modes(&[r, ("access", "private")]), ("-".into(), "-".into()));
    assert_eq!(modes(&[r, ("access", "no"), ("bicycle", "yes")]), pair("b", "b"));
    assert_eq!(modes(&[r, ("vehicle", "no")]), pair("w", "w"));
    assert_eq!(modes(&[r, ("vehicle", "no"), ("motor_vehicle", "yes")]), pair("cw", "cw"));
    assert_eq!(modes(&[r, ("motor_vehicle", "no")]), pair("bw", "bw"));
    assert_eq!(modes(&[r, ("motor_vehicle", "destination")]), pair("cbw", "cbw"));
    assert_eq!(
        modes(&[r, ("motor_vehicle", "unknown"), ("access", "no")]),
        ("-".into(), "-".into())
    );
    // A motorroad closes the way to bikes and walkers.
    assert_eq!(modes(&[("highway", "trunk"), ("motorroad", "yes")]), pair("c", "c"));
    // A parking aisle is not a street for cars; bikes and walkers keep it.
    assert_eq!(modes(&[("highway", "service"), ("service", "parking_aisle")]), pair("bw", "bw"));
    // A cycleway is never a car street, whatever its car keys say (mopeds on Dutch cycleways).
    assert_eq!(
        modes(&[("highway", "cycleway"), ("motor_vehicle", "destination")]),
        pair("bw", "bw")
    );
}

#[test]
fn oneway_binds_cars_and_bikes_and_not_walkers() {
    // Key:oneway: it "only applies to vehicles"; oneway:bicycle=no opens the street to bikes
    // both ways; oneway:foot binds walkers.
    let r = ("highway", "residential");
    assert_eq!(modes(&[r, ("oneway", "yes")]), pair("cbw", "w"));
    assert_eq!(modes(&[r, ("oneway", "-1")]), pair("w", "cbw"));
    assert_eq!(modes(&[r, ("oneway", "yes"), ("oneway:bicycle", "no")]), pair("cbw", "bw"));
    assert_eq!(modes(&[("highway", "cycleway"), ("oneway", "yes")]), pair("bw", "w"));
    assert_eq!(modes(&[("highway", "footway"), ("oneway:foot", "yes")]), pair("w", "-"));
    assert_eq!(modes(&[r, ("junction", "roundabout")]), pair("cbw", "w"));
    assert_eq!(modes(&[r, ("oneway", "reversible")]), pair("cbw", "cbw"));
}

#[test]
fn lanes_follow_the_wiki_with_one_side_given() {
    let two_way = |extra: &[(&str, &str)]| {
        let mut t = vec![("highway", "primary"), ("maxspeed", "50")];
        t.extend_from_slice(extra);
        let (f, b) = car(&t);
        (f.unwrap().0, b.unwrap().0)
    };
    // Bug fixed: total 4, forward 1 on a two-way street gives backward 4 − 1 = 3 (not 4 / 2).
    assert_eq!(two_way(&[("lanes", "4"), ("lanes:forward", "1")]), (1, 3));
    assert_eq!(two_way(&[("lanes", "4"), ("lanes:backward", "1")]), (3, 1));
    // A centre lane belongs to neither side: 5 − 2 − 1 = 2.
    assert_eq!(
        two_way(&[("lanes", "5"), ("lanes:forward", "2"), ("lanes:both_ways", "1")]),
        (2, 2)
    );
    assert_eq!(two_way(&[("lanes:forward", "3"), ("lanes:backward", "2")]), (3, 2));
    // The total alone is split: 4 gives 2 and 2; 3 gives 1 and 1; 1 gives 1 and 1.
    assert_eq!(two_way(&[("lanes", "4")]), (2, 2));
    assert_eq!(two_way(&[("lanes", "3")]), (1, 1));
    assert_eq!(two_way(&[("lanes", "1")]), (1, 1));
    // No lane tag: the class's default (primary, 2 per direction).
    assert_eq!(two_way(&[]), (2, 2));
    // One way: the total is the one direction's.
    let (f, b) = car(&[("highway", "primary"), ("oneway", "yes"), ("lanes", "3")]);
    assert_eq!((f.unwrap().0, b), (3, None));
}

#[test]
fn maxspeed_by_direction_is_read_first() {
    // Bug fixed: maxspeed:forward=30 and maxspeed:backward=70 give 30 and 70 km/h.
    let t = [("highway", "secondary"), ("maxspeed:forward", "30"), ("maxspeed:backward", "70")];
    let (f, b) = car(&t);
    assert_eq!((f.unwrap().1, b.unwrap().1), (30.0, 70.0));
    let (f, b) = car(&[("highway", "secondary"), ("maxspeed", "50"), ("maxspeed:backward", "70")]);
    assert_eq!((f.unwrap().1, b.unwrap().1), (50.0, 70.0));
    // 30 mph is 30 × 1.609 344 = 48.280 32 km/h.
    let (f, _) = car(&[("highway", "secondary"), ("maxspeed", "30 mph")]);
    assert!(close(f.unwrap().1, 48.280_32, 1e-9), "{f:?}");
    assert!(close(30.0 * KM_PER_MILE, 48.280_32, 1e-12));
    // No maxspeed: the class's default (secondary, 50 km/h).
    assert_eq!(car(&[("highway", "secondary")]).0.unwrap().1, 50.0);
}

#[test]
fn a_tag_the_reader_cannot_read_is_an_error_naming_the_way() {
    // Bug fixed: nothing is silently replaced by a default.
    let refused = |extra: (&str, &str)| {
        let err = read(&street(2, &[("highway", "residential"), extra])).unwrap_err();
        match &err {
            OsmError::Tag { way: 100, key, value, .. } => {
                assert_eq!((key.as_str(), value.as_str()), extra);
            }
            other => panic!("{extra:?}: {other:?}"),
        }
        assert!(err.to_string().contains("way 100"), "{err}");
    };
    for value in ["none", "walk", "NL:urban", "50;30", "fast", "0", "-20", "50 km/h"] {
        refused(("maxspeed", value));
    }
    refused(("maxspeed:forward", "signals"));
    refused(("lanes", "two"));
    refused(("lanes", "2;3"));
    refused(("lanes:forward", "1.5"));
    refused(("oneway", "true"));
    refused(("bicycle", "maybe"));
    refused(("access", "residents"));
    let lanes = read(&street(2, &[("highway", "primary"), ("lanes", "2"), ("lanes:forward", "2")]));
    assert!(matches!(lanes, Err(OsmError::Lanes { way: 100, .. })), "{lanes:?}");
    let sum =
        [("highway", "primary"), ("lanes", "4"), ("lanes:forward", "2"), ("lanes:backward", "1")];
    assert!(matches!(read(&street(2, &sum)), Err(OsmError::Lanes { way: 100, .. })));
    // A way without cars does not read the car tags.
    assert!(read(&street(2, &[("highway", "footway"), ("maxspeed", "walk")])).is_ok());
}

#[test]
fn the_class_table_reads_as_its_readme_states() {
    // The parameter osm_class_defaults (PARAMETERS.md): the README's table, (highway, km/h,
    // lanes per direction) for every car class.
    let table = [
        ("motorway", 110.0, 2),
        ("motorway_link", 60.0, 1),
        ("trunk", 90.0, 2),
        ("trunk_link", 50.0, 1),
        ("primary", 60.0, 2),
        ("primary_link", 40.0, 1),
        ("secondary", 50.0, 1),
        ("secondary_link", 40.0, 1),
        ("tertiary", 50.0, 1),
        ("tertiary_link", 30.0, 1),
        ("unclassified", 40.0, 1),
        ("road", 40.0, 1),
        ("residential", 30.0, 1),
        ("living_street", 20.0, 1),
        ("service", 20.0, 1),
    ];
    for (highway, km_h, lanes) in table {
        assert_eq!(class_of(highway).and_then(|c| c.car), Some((km_h, lanes)), "{highway}");
        let tags = [("highway", highway), ("oneway", "no")];
        let (f, b) = car(&tags);
        assert_eq!((f, b), (Some((lanes, km_h)), Some((lanes, km_h))), "{highway}");
    }
    let car_classes = CLASSES.iter().filter(|c| c.car.is_some()).flat_map(|c| c.highway).count();
    assert_eq!(car_classes, table.len(), "every car class is in the table");
    for highway in ["construction", "proposed", "platform", "bus_guideway", "raceway"] {
        assert!(class_of(highway).is_none(), "{highway} is not a street");
    }
}

#[test]
fn what_every_rule_means_for_a_bike() {
    // `f`/`b`: whether a bike may ride along / against the way's node order.
    let bike = |tags: &[(&str, &str)]| {
        let (f, b) = modes(tags);
        (f.contains('b'), b.contains('b'))
    };
    // The class: streets and trunk roads yes (wiki), motorways no, cycleways and paths yes,
    // footways, pedestrian streets and steps no (a bike pushed is a walker), tracks yes.
    assert_eq!(bike(&[("highway", "residential")]), (true, true));
    assert_eq!(bike(&[("highway", "trunk")]), (true, true));
    assert_eq!(bike(&[("highway", "motorway")]), (false, false));
    assert_eq!(bike(&[("highway", "cycleway")]), (true, true));
    assert_eq!(bike(&[("highway", "path")]), (true, true));
    assert_eq!(bike(&[("highway", "track")]), (true, true));
    for class in ["footway", "pedestrian", "steps", "bridleway", "busway"] {
        assert_eq!(bike(&[("highway", class)]), (false, false), "{class}");
    }
    // The keys: bicycle, then vehicle, then access.
    let r = ("highway", "residential");
    assert_eq!(bike(&[r, ("bicycle", "no")]), (false, false));
    assert_eq!(bike(&[r, ("bicycle", "use_sidepath")]), (false, false));
    assert_eq!(bike(&[r, ("bicycle", "dismount")]), (false, false));
    assert_eq!(bike(&[r, ("vehicle", "no")]), (false, false));
    assert_eq!(bike(&[r, ("access", "private")]), (false, false));
    assert_eq!(bike(&[r, ("access", "no"), ("bicycle", "designated")]), (true, true));
    assert_eq!(bike(&[r, ("motor_vehicle", "no")]), (true, true), "a car key binds no bike");
    assert_eq!(bike(&[("highway", "footway"), ("bicycle", "yes")]), (true, true));
    assert_eq!(bike(&[("highway", "trunk"), ("motorroad", "yes")]), (false, false));
    assert_eq!(bike(&[("highway", "service"), ("service", "driveway")]), (true, true));
    // The direction: oneway binds a bike unless oneway:bicycle says otherwise.
    assert_eq!(bike(&[r, ("oneway", "yes")]), (true, false));
    assert_eq!(bike(&[r, ("oneway", "yes"), ("oneway:bicycle", "no")]), (true, true));
    assert_eq!(bike(&[r, ("oneway:bicycle", "yes")]), (true, false));
    assert_eq!(bike(&[r, ("junction", "roundabout")]), (true, false));
    // The car's lanes and speed are no bike's: a bike rides at bike_speed on every link.
    let (net, _) = read(&street(2, &[r, ("maxspeed", "30")])).unwrap();
    let f = direction(&net, 100, "f").unwrap();
    let bike_s = net.free_flow_time(f.links[0], openmobisim_types::mode::Mode::Bike).0;
    assert!(close(bike_s, f.length_m / (15.0 / 3.6), 1e-9), "{bike_s} s at 15 km/h");
}
