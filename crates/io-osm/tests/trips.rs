//! The trips drawn from a network's streets (experimental), by hand and by property.
//!
//! The extract: way 100, a two-way residential street through nodes 1, 2, 3 (4.361°E to
//! 4.363°E on 52.0°N), and way 101, a two-way tertiary road on through nodes 3, 4, 5 (to
//! 4.365°E). By hand: homes are counted on the residential street alone and activities on the
//! tertiary road alone, so every trip starts on the first (longitude 4.361 to 4.363) and ends
//! on the second (4.363 to 4.365), on the parallel. No region cuts a motorway, so no trip
//! drives through. Properties: the ids count up; a departure lies in the hour after the start;
//! the classes come by their shares (within four standard deviations of a binomial draw);
//! without a timetable no trip may ride transit; the same seed gives the same trips, another
//! seed others. Bikes: the classes `car_and_bike` and `bike_only`, 60 % of the travellers, may
//! ride; the others may not.

mod common;

use common::{at, read};
use openmobisim_graph::geometry::Position;
use openmobisim_io_osm::trips::{CLASSES, DEPARTURES_S, draw};
use openmobisim_io_osm::{Osm, OsmNode, OsmWay};
use openmobisim_types::mode::Mode;
use openmobisim_types::time::Second;

fn extract() -> Osm {
    let nodes: Vec<OsmNode> = (1..=5).map(|i| at(i, i)).collect();
    let ways = vec![
        OsmWay::new(100, &[1, 2, 3], &[("highway", "residential")]),
        OsmWay::new(101, &[3, 4, 5], &[("highway", "tertiary")]),
    ];
    Osm { nodes, ways }
}

fn lon_lat(p: Position) -> (f64, f64) {
    match p {
        Position::LonLat(at) => (at.lon, at.lat),
        Position::Metres(_) => panic!("a network read from an extract places trips in degrees"),
    }
}

#[test]
fn trips_start_on_home_streets_and_end_on_activity_streets() {
    let (net, _) = read(&extract()).unwrap();
    let trips = draw(&net, 2_000, Second(27_000), false, 7);
    assert_eq!(trips.len(), 2_000);
    for (k, t) in trips.iter().enumerate() {
        assert_eq!(t.id, format!("t{k:07}"));
        let ((ox, oy), (dx, dy)) = (lon_lat(t.origin), lon_lat(t.destination));
        assert!((4.361 - 1e-7..=4.363 + 1e-7).contains(&ox), "{} starts at {ox}", t.id);
        assert!((4.363 - 1e-7..=4.365 + 1e-7).contains(&dx), "{} ends at {dx}", t.id);
        assert!((oy - 52.0).abs() < 1e-6 && (dy - 52.0).abs() < 1e-6);
        assert!((27_000..27_000 + DEPARTURES_S).contains(&t.departure.get()));
        let (_, _, open) = CLASSES.iter().find(|c| c.0 == t.class).expect("one of the classes");
        assert!(!t.modes.contains(Mode::Transit), "no timetable, no transit");
        for m in [Mode::Car, Mode::Bike, Mode::Walk] {
            assert_eq!(t.modes.contains(m), open.contains(&m), "{} of class {}", t.id, t.class);
        }
    }
    for (class, share, _) in CLASSES {
        #[allow(clippy::cast_precision_loss, reason = "small test counts")]
        let found = trips.iter().filter(|t| t.class == class).count() as f64 / 2_000.0;
        let deviation = (share * (1.0 - share) / 2_000.0).sqrt();
        assert!((found - share).abs() < 4.0 * deviation, "{class}: {found} for {share}");
    }
}

#[test]
fn the_same_seed_gives_the_same_trips_and_transit_is_open_with_a_timetable() {
    let (net, _) = read(&extract()).unwrap();
    let first = draw(&net, 300, Second(0), true, 1);
    assert_eq!(first, draw(&net, 300, Second(0), true, 1));
    assert_ne!(first, draw(&net, 300, Second(0), true, 2));
    assert!(first.iter().all(|t| t.modes.contains(Mode::Transit)));
    // A longer draw starts with the shorter one: a trip's draws are its own.
    assert_eq!(first[..], draw(&net, 400, Second(0), true, 1)[..300]);
}

#[test]
fn a_network_without_a_street_of_the_classes_gives_no_trips() {
    // A cycleway alone: neither a home street nor an activity street.
    let nodes: Vec<OsmNode> = (1..=3).map(|i| at(i, i)).collect();
    let ways = vec![OsmWay::new(100, &[1, 2, 3], &[("highway", "cycleway")])];
    let (net, _) = read(&Osm { nodes, ways }).unwrap();
    assert!(draw(&net, 10, Second(0), false, 0).is_empty());
}
