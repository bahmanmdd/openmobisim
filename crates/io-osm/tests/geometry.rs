//! A link's length against pyproj (`Geod(ellps="WGS84")`, Karney's geodesic algorithm,
//! pyproj 3.8.0): the sum of the WGS84 geodesics between the nodes of its way, on each layer.

mod common;

use common::{close, direction, read};
use openmobisim_io_osm::geodesic::geodesic_m;
use openmobisim_io_osm::{Osm, OsmNode, OsmWay};
use openmobisim_types::mode::Mode;

#[test]
fn a_five_node_way_is_as_long_as_its_geodesics() {
    // A zigzag through (4.800, 45.700), (4.801, 45.701), (4.802, 45.700), (4.803, 45.701),
    // (4.804, 45.700). pyproj: Geod(ellps="WGS84").line_length(lons, lats) = 542.861119846913 m.
    let points =
        [(4.800, 45.700), (4.801, 45.701), (4.802, 45.700), (4.803, 45.701), (4.804, 45.700)];
    let mut nodes: Vec<OsmNode> =
        (1..=5).zip(points).map(|(id, (lon, lat))| OsmNode::new(id, lon, lat)).collect();
    nodes.push(OsmNode::new(9, 4.802, 45.702));
    let osm = Osm {
        nodes,
        ways: vec![
            OsmWay::new(100, &[1, 2, 3, 4, 5], &[("highway", "residential")]),
            OsmWay::new(200, &[5, 9, 1], &[("highway", "residential")]),
        ],
    };
    let (net, _) = read(&osm).unwrap();
    for mode in [Mode::Car, Mode::Bike, Mode::Walk] {
        let f = direction(&net, mode, 100, "f").unwrap();
        assert_eq!(f.links.len(), 1, "the inner nodes are geometry, not junctions");
        assert!(close(f.length_m, 542.861_119_846_913, 1e-6), "{mode}: {} m", f.length_m);
    }
    let f = direction(&net, Mode::Car, 100, "f").unwrap();
    let straight = geodesic_m((4.800, 45.700), (4.804, 45.700)).unwrap();
    assert!(f.length_m > 1.7 * straight, "the zigzag, not the straight line of {straight} m");
}

#[test]
fn vincenty_agrees_with_pyproj_from_a_street_to_an_ocean() {
    // (from, to, pyproj's Geod.inv distance in metres)
    let cases = [
        ((4.36, 52.01), (4.37, 52.0), 1_307.519_174_026_217),
        ((0.0, 0.0), (1.0, 0.0), 111_319.490_793_273_57),
        ((0.0, 0.0), (0.0, 1.0), 110_574.388_557_798_78),
        ((4.0, 52.0), (5.0, 53.0), 130_359.285_520_442_3),
        ((-74.0, 40.7), (2.35, 48.85), 5_853_490.432_575_24),
        ((4.3601, 52.0101), (4.3601, 52.0101), 0.0),
    ];
    // Within 1 µm up to 131 km. Over 5 853 km, Vincenty's series (truncated after u⁸) is
    // within the 0.5 mm Vincenty (1975) states; measured: 0.018 mm.
    for (a, b, expected) in cases {
        let d = geodesic_m(a, b).unwrap();
        let tol = if expected > 1e6 { 5e-4 } else { 1e-6 };
        assert!(close(d, expected, tol), "{a:?} to {b:?}: {d} m, pyproj {expected} m");
    }
    assert_eq!(geodesic_m((f64::NAN, 0.0), (0.0, 0.0)), None);
}
