//! The projection against the ellipsoid and the UTM grid's definition.
//!
//! The expected values come from outside the code under test: the six-degree zone grid of the
//! UTM standard, the exact identities on the central meridian (easting 500 000 m, northing 0 at
//! the equator, 10 000 000 m in the south), and the WGS84 ellipsoid's meridian and parallel arcs,
//! against which a projected distance on the central meridian is exactly k0 = 0.9996 times the
//! arc. Between them they catch a wrong coefficient, a transposed axis, a radian/degree slip or a
//! hemisphere error. The way back, metres to degrees, is held to the property that it undoes the
//! projection, and to the grid's origin.

// Exact comparison is used only where the value is exact by construction.
#![allow(clippy::float_cmp, reason = "the central-meridian identity is exact")]

use openmobisim_graph::geometry::{
    Hemisphere, LonLat, Projection, ProjectionError, UTM_FALSE_EASTING, UTM_FALSE_NORTHING_SOUTH,
    UTM_K0, UtmZone, Xy,
};

/// WGS84 semi-major axis and first eccentricity squared.
const A: f64 = 6_378_137.0;
const E2: f64 = 6.694_379_990_141_32e-3;

/// The exact ellipsoidal length of a short **north-south** baseline, in metres.
///
/// The meridian arc is `M(φ)·Δφ` with `M` the meridional radius of curvature.
/// Taking `M` at the midpoint makes this correct to well under a millimetre
/// over the 0.1° baselines used here, which is what allows the tolerances
/// below to be tight enough to mean something. (A mean-radius sphere would not
/// do: at the equator it overstates a north-south distance by 0.56 %, because
/// the meridional radius of curvature there is 6 335 km, not 6 371 km.)
fn meridian_arc(lat_a: f64, lat_b: f64) -> f64 {
    let mid = ((lat_a + lat_b) / 2.0).to_radians();
    let w = 1.0 - E2 * mid.sin().powi(2);
    let m = A * (1.0 - E2) / (w * w.sqrt());
    m * (lat_b - lat_a).to_radians().abs()
}

/// The exact ellipsoidal length of a short **east-west** baseline, in metres.
///
/// A parallel of latitude is a circle of radius `N(φ)·cos φ`, so this is exact
/// for an arc along it.
fn parallel_arc(lat: f64, lon_a: f64, lon_b: f64) -> f64 {
    let phi = lat.to_radians();
    let n = A / (1.0 - E2 * phi.sin().powi(2)).sqrt();
    n * phi.cos() * (lon_b - lon_a).to_radians().abs()
}

fn zone_for(lon: f64, lat: f64) -> UtmZone {
    UtmZone::for_centroid(LonLat::new(lon, lat)).expect("projectable")
}

#[test]
fn zone_numbering_follows_the_six_degree_grid() {
    assert_eq!(zone_for(-180.0, 0.0).number(), 1);
    assert_eq!(zone_for(-177.0, 0.0).number(), 1);
    assert_eq!(zone_for(-174.0, 0.0).number(), 2);
    assert_eq!(zone_for(0.0, 0.0).number(), 31);
    assert_eq!(zone_for(4.8, 45.8).number(), 31); // Lyon
    assert_eq!(zone_for(13.4, 52.5).number(), 33); // Berlin
    assert_eq!(zone_for(180.0, 0.0).number(), 60);
}

#[test]
fn central_meridians_sit_in_the_middle_of_their_zones() {
    assert_eq!(zone_for(4.8, 45.8).central_meridian_degrees(), 3.0);
    assert_eq!(zone_for(13.4, 52.5).central_meridian_degrees(), 15.0);
    assert_eq!(zone_for(-179.0, 0.0).central_meridian_degrees(), -177.0);
    assert_eq!(zone_for(179.0, 0.0).central_meridian_degrees(), 177.0);
}

#[test]
fn the_hemisphere_follows_the_sign_of_the_latitude() {
    assert_eq!(zone_for(13.4, 52.5).hemisphere(), Hemisphere::North);
    assert_eq!(zone_for(-46.6, -23.5).hemisphere(), Hemisphere::South); // São Paulo
    assert_eq!(zone_for(-46.6, -23.5).number(), 23);
    assert_eq!(zone_for(-46.6, -23.5).to_string(), "UTM zone 23S");
}

#[test]
fn the_central_meridian_identity_is_exact() {
    // On the central meridian the easting is exactly the false easting, and at
    // the equator the northing is exactly zero. Any coefficient error shows up
    // here immediately.
    let proj = Projection::wgs84(zone_for(3.0, 0.0));
    let origin = proj.project(LonLat::new(3.0, 0.0));
    assert!((origin.x - UTM_FALSE_EASTING).abs() < 1e-6, "easting was {}", origin.x);
    assert!(origin.y.abs() < 1e-6, "northing was {}", origin.y);

    // And away from the equator the easting is still exactly the false easting.
    for lat in [-60.0, -30.0, 10.0, 45.0, 70.0] {
        let p = proj.project(LonLat::new(3.0, lat));
        assert!((p.x - UTM_FALSE_EASTING).abs() < 1e-6, "at {lat}°, easting was {}", p.x);
    }
}

#[test]
fn the_southern_hemisphere_gets_its_false_northing() {
    let proj = Projection::wgs84(zone_for(-45.0, -1.0));
    let at_equator = proj.project(LonLat::new(-45.0, 0.0));
    assert!(
        (at_equator.y - UTM_FALSE_NORTHING_SOUTH).abs() < 1e-6,
        "northing was {}",
        at_equator.y
    );
    // Southern latitudes are below the false northing, and stay positive.
    let sao_paulo = proj.project(LonLat::new(-46.6, -23.5));
    assert!(sao_paulo.y > 0.0 && sao_paulo.y < UTM_FALSE_NORTHING_SOUTH);
}

#[test]
fn northings_and_eastings_increase_in_the_right_directions() {
    let proj = Projection::wgs84(zone_for(3.0, 45.0));
    let south = proj.project(LonLat::new(3.0, 45.0));
    let north = proj.project(LonLat::new(3.0, 46.0));
    assert!(north.y > south.y, "northing must increase northward");

    let west = proj.project(LonLat::new(2.0, 45.0));
    let east = proj.project(LonLat::new(4.0, 45.0));
    assert!(east.x > west.x, "easting must increase eastward");
    // Symmetric about the central meridian.
    assert!(
        ((UTM_FALSE_EASTING - west.x) - (east.x - UTM_FALSE_EASTING)).abs() < 1e-6,
        "the projection must be symmetric about the central meridian"
    );
}

#[test]
fn distances_agree_with_the_ellipsoid_to_a_few_parts_per_million() {
    // On the central meridian the projection shrinks distances by exactly k0.
    // Against a true meridian arc the agreement should be at the part-per-
    // million level, so this catches a wrong rectifying radius or a
    // degree/radian slip that a looser check would wave through.
    let proj = Projection::wgs84(zone_for(3.0, 1.0));
    for lat in [0.0, 15.0, 30.0, 45.0, 60.0, 75.0] {
        let a = LonLat::new(3.0, lat);
        let b = LonLat::new(3.0, lat + 0.1); // ~11 km due north
        let projected = proj.project(a).distance_to(proj.project(b));
        let exact = meridian_arc(lat, lat + 0.1);
        let ratio = projected / exact;
        assert!(
            (ratio - UTM_K0).abs() < 5e-6,
            "at {lat}°, projected/exact was {ratio}, expected {UTM_K0}"
        );
    }
}

#[test]
fn east_west_distances_agree_with_the_ellipsoid_too() {
    let proj = Projection::wgs84(zone_for(3.0, 1.0));
    for lat in [0.0, 30.0, 52.0, 70.0] {
        let a = LonLat::new(2.9, lat);
        let b = LonLat::new(3.1, lat);
        let projected = proj.project(a).distance_to(proj.project(b));
        let exact = parallel_arc(lat, 2.9, 3.1);
        let ratio = projected / exact;
        assert!(
            (ratio - UTM_K0).abs() < 2e-5,
            "at {lat}°, east-west projected/exact was {ratio}, expected about {UTM_K0}"
        );
    }
}

#[test]
fn a_zone_is_chosen_from_the_centre_of_the_study_area() {
    // A box around Lyon (4.75° to 4.95° E): the zone must be 31.
    let proj = Projection::for_extent([
        LonLat::new(4.75, 45.70),
        LonLat::new(4.95, 45.83),
        LonLat::new(4.80, 45.75),
    ])
    .expect("projectable");
    assert_eq!(proj.zone().number(), 31);
    assert_eq!(proj.zone().hemisphere(), Hemisphere::North);
}

#[test]
fn an_unusable_extent_is_refused_at_build_time() {
    assert!(matches!(
        Projection::for_extent(core::iter::empty()),
        Err(ProjectionError::EmptyExtent)
    ));
    assert!(matches!(
        Projection::for_extent([LonLat::new(4.8, 89.0)]),
        Err(ProjectionError::NotProjectable { .. })
    ));
    assert!(matches!(
        Projection::for_extent([LonLat::new(f64::NAN, 45.0)]),
        Err(ProjectionError::NotProjectable { .. })
    ));
    // The message has to be usable by someone holding a shapefile.
    let err = Projection::for_extent([LonLat::new(4.8, 89.0)]).unwrap_err();
    assert!(err.to_string().contains("±84°"), "{err}");
}

#[test]
fn projectability_is_checked_where_it_is_cheap() {
    assert!(LonLat::new(4.8, 45.8).is_projectable());
    assert!(!LonLat::new(4.8, 85.0).is_projectable());
    assert!(!LonLat::new(181.0, 45.0).is_projectable());
    assert!(!LonLat::new(0.0, f64::INFINITY).is_projectable());
}

#[test]
fn a_point_projected_and_brought_back_is_the_point() {
    // A property: `unproject` undoes `project`. Over each zone's width and beyond its edge
    // (4° either side of the central meridian), from 78°S to 82°N, in three zones of both
    // hemispheres, to 1e-11 degree (a micrometre). `project` itself is held to the meridian
    // and parallel arcs above.
    for (meridian, south) in [(3.0, false), (15.0, false), (-45.0, true)] {
        let lat0 = if south { -1.0 } else { 1.0 };
        let proj = Projection::wgs84(zone_for(meridian, lat0));
        for i in -8..=8 {
            for j in -16..=16 {
                let p = LonLat::new(meridian + f64::from(i) * 0.5, f64::from(j) * 5.0 + 2.0);
                if (p.lat < 0.0) != south || p.lat > 84.0 {
                    continue;
                }
                let back = proj.unproject(proj.project(p));
                assert!(
                    (back.lon - p.lon).abs() < 1e-11 && (back.lat - p.lat).abs() < 1e-11,
                    "{p:?} came back as {back:?}"
                );
            }
        }
    }
}

#[test]
fn the_grids_origin_is_the_central_meridian_at_the_equator() {
    // By the grid's definition: easting 500 000 m is the central meridian, northing 0 the
    // equator (10 000 000 m in the south).
    let north = Projection::wgs84(zone_for(3.0, 1.0));
    let o = north.unproject(Xy { x: UTM_FALSE_EASTING, y: 0.0 });
    assert!((o.lon - 3.0).abs() < 1e-12 && o.lat.abs() < 1e-12, "{o:?}");
    let south = Projection::wgs84(zone_for(-45.0, -1.0));
    let o = south.unproject(Xy { x: UTM_FALSE_EASTING, y: UTM_FALSE_NORTHING_SOUTH });
    assert!((o.lon + 45.0).abs() < 1e-12 && o.lat.abs() < 1e-12, "{o:?}");
}
