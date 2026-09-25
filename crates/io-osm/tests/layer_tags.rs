//! The bike and walk tag rules (S193, S195): one test per row of the rule
//! table, so that a changed rule is a failed test and not a quietly different
//! bike network.

use openmobisim_core_graph::defaults::RoadClass;
use openmobisim_core_graph::layers::BikeInfrastructure::{Lane, Mixed, Separated};
use openmobisim_io_osm::tags::{BikeWay, bike_way, walk_way};

fn tags(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
}

fn bike(pairs: &[(&str, &str)]) -> Option<BikeWay> {
    bike_way(&tags(pairs), 2)
}

// --- which ways are bike links -------------------------------------------------

#[test]
fn motorways_and_ways_closed_to_bikes_are_not_bike_links() {
    for pairs in [
        &[("highway", "motorway")][..],
        &[("highway", "motorway_link")],
        &[("highway", "primary"), ("motorroad", "yes")],
        &[("highway", "primary"), ("bicycle", "no")],
        &[("highway", "primary"), ("bicycle", "use_sidepath")],
        &[("highway", "service"), ("access", "private")],
        &[("highway", "pedestrian"), ("area", "yes"), ("bicycle", "yes")],
        &[("building", "yes")],
    ] {
        assert_eq!(bike(pairs), None, "{pairs:?}");
    }
    assert_eq!(bike_way(&tags(&[("highway", "residential")]), 1), None, "one node");
}

#[test]
fn every_other_car_road_is_a_mixed_bike_link_trunk_included() {
    for highway in ["trunk", "trunk_link", "primary", "secondary", "tertiary", "unclassified"]
        .into_iter()
        .chain(["residential", "living_street", "service"])
    {
        let way = bike(&[("highway", highway)]).expect(highway);
        assert_eq!((way.forward, way.backward), (Some(Mixed), Some(Mixed)), "{highway}");
        assert!(!way.dismount);
    }
}

#[test]
fn a_bike_only_road_behind_access_no_is_kept() {
    for value in ["yes", "designated", "permissive"] {
        let way = bike(&[("highway", "service"), ("access", "no"), ("bicycle", value)]);
        assert!(way.is_some(), "access=no, bicycle={value}");
    }
}

#[test]
fn a_cycleway_and_a_designated_path_are_separated() {
    let cycleway = bike(&[("highway", "cycleway")]).unwrap();
    assert_eq!((cycleway.forward, cycleway.backward), (Some(Separated), Some(Separated)));
    assert_eq!(cycleway.class, RoadClass::Cycleway);
    for highway in ["path", "footway", "bridleway"] {
        let way = bike(&[("highway", highway), ("bicycle", "designated")]).expect(highway);
        assert_eq!(way.forward, Some(Separated), "{highway}");
    }
}

#[test]
fn footways_paths_tracks_and_pedestrian_streets_need_a_bicycle_tag() {
    for highway in ["footway", "path", "track", "pedestrian", "steps", "bridleway"] {
        assert_eq!(bike(&[("highway", highway)]), None, "{highway} without a bicycle tag");
        let shared = bike(&[("highway", highway), ("bicycle", "yes")]).expect(highway);
        assert_eq!(shared.forward, Some(Mixed), "{highway}: shared with pedestrians");
    }
}

#[test]
fn dismount_and_steps_are_walked() {
    assert!(bike(&[("highway", "footway"), ("bicycle", "dismount")]).unwrap().dismount);
    assert!(bike(&[("highway", "primary"), ("bicycle", "dismount")]).unwrap().dismount);
    assert!(bike(&[("highway", "steps"), ("bicycle", "yes")]).unwrap().dismount);
    assert!(!bike(&[("highway", "path"), ("bicycle", "yes")]).unwrap().dismount);
}

#[test]
fn a_cycle_street_counts_as_a_lane() {
    for key in ["cyclestreet", "bicycle_road"] {
        let way = bike(&[("highway", "residential"), (key, "yes")]).unwrap();
        assert_eq!((way.forward, way.backward), (Some(Lane), Some(Lane)), "{key}");
    }
}

// --- infrastructure by side -----------------------------------------------------

#[test]
fn a_lane_or_track_on_both_sides_serves_both_directions() {
    for (key, value, level) in [
        ("cycleway", "lane", Lane),
        ("cycleway", "track", Separated),
        ("cycleway:both", "lane", Lane),
        ("cycleway:both", "track", Separated),
    ] {
        let way = bike(&[("highway", "secondary"), (key, value)]).unwrap();
        assert_eq!((way.forward, way.backward), (Some(level), Some(level)), "{key}={value}");
    }
}

#[test]
fn on_a_two_way_street_right_is_forward_and_left_is_backward() {
    let right = bike(&[("highway", "secondary"), ("cycleway:right", "track")]).unwrap();
    assert_eq!((right.forward, right.backward), (Some(Separated), Some(Mixed)));
    let left = bike(&[("highway", "secondary"), ("cycleway:left", "lane")]).unwrap();
    assert_eq!((left.forward, left.backward), (Some(Mixed), Some(Lane)));
}

#[test]
fn shared_lanes_and_separately_mapped_tracks_leave_the_road_mixed() {
    for value in ["shared_lane", "separate", "no"] {
        let way = bike(&[("highway", "secondary"), ("cycleway", value)]).unwrap();
        assert_eq!((way.forward, way.backward), (Some(Mixed), Some(Mixed)), "cycleway={value}");
    }
}

// --- direction --------------------------------------------------------------------

#[test]
fn bikes_follow_a_one_way_street_unless_told_otherwise() {
    let one_way = bike(&[("highway", "residential"), ("oneway", "yes")]).unwrap();
    assert_eq!((one_way.forward, one_way.backward), (Some(Mixed), None));

    let contraflow =
        bike(&[("highway", "residential"), ("oneway", "yes"), ("oneway:bicycle", "no")]).unwrap();
    assert_eq!((contraflow.forward, contraflow.backward), (Some(Mixed), Some(Mixed)));
}

#[test]
fn opposite_values_open_the_contraflow_with_their_own_infrastructure() {
    let plain = bike(&[("highway", "residential"), ("oneway", "yes"), ("cycleway", "opposite")]);
    assert_eq!(plain.map(|w| (w.forward, w.backward)), Some((Some(Mixed), Some(Mixed))));
    let lane =
        bike(&[("highway", "residential"), ("oneway", "yes"), ("cycleway", "opposite_lane")]);
    assert_eq!(lane.map(|w| (w.forward, w.backward)), Some((Some(Mixed), Some(Lane))));
    let track =
        bike(&[("highway", "residential"), ("oneway", "yes"), ("cycleway", "opposite_track")]);
    assert_eq!(track.map(|w| (w.forward, w.backward)), Some((Some(Mixed), Some(Separated))));
}

#[test]
fn a_lane_on_either_side_of_a_one_way_street_serves_its_direction() {
    for key in ["cycleway:right", "cycleway:left"] {
        let way = bike(&[("highway", "tertiary"), ("oneway", "yes"), (key, "lane")]).unwrap();
        assert_eq!((way.forward, way.backward), (Some(Lane), None), "{key}");
    }
}

#[test]
fn oneway_bicycle_overrides_in_both_senses() {
    let one_way_for_bikes = bike(&[("highway", "residential"), ("oneway:bicycle", "yes")]).unwrap();
    assert_eq!((one_way_for_bikes.forward, one_way_for_bikes.backward), (Some(Mixed), None));
    let reversed = bike(&[("highway", "residential"), ("oneway:bicycle", "-1")]).unwrap();
    assert_eq!((reversed.forward, reversed.backward), (None, Some(Mixed)));
    let one_way_cycleway = bike(&[("highway", "cycleway"), ("oneway", "yes")]).unwrap();
    assert_eq!((one_way_cycleway.forward, one_way_cycleway.backward), (Some(Separated), None));
}

// --- walking ------------------------------------------------------------------------

#[test]
fn walking_is_everywhere_but_motorways_cycleways_and_closed_ways() {
    for highway in ["primary", "trunk", "residential", "footway", "path", "steps", "pedestrian"] {
        assert!(walk_way(&tags(&[("highway", highway)]), 2).is_some(), "{highway}");
    }
    for pairs in [
        &[("highway", "motorway")][..],
        &[("highway", "motorway_link")],
        &[("highway", "cycleway")],
        &[("highway", "primary"), ("foot", "no")],
        &[("highway", "primary"), ("foot", "use_sidepath")],
        &[("highway", "trunk"), ("motorroad", "yes")],
        &[("highway", "service"), ("access", "private")],
        &[("highway", "pedestrian"), ("area", "yes")],
    ] {
        assert_eq!(walk_way(&tags(pairs), 2), None, "{pairs:?}");
    }
}

#[test]
fn a_cycleway_or_closed_way_open_to_walkers_is_walked() {
    assert_eq!(
        walk_way(&tags(&[("highway", "cycleway"), ("foot", "yes")]), 2),
        Some(RoadClass::Cycleway)
    );
    assert!(
        walk_way(&tags(&[("highway", "service"), ("access", "no"), ("foot", "designated")]), 2)
            .is_some()
    );
}
