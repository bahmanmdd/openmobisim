//! The projection, checked three ways.
//!
//! There is no PROJ here to compare against, so accuracy is established from
//! properties rather than from a reference implementation: an exact identity on
//! the central meridian, a round trip to sub-millimetre, and agreement with an
//! independent distance formula. Between them they would catch a wrong
//! coefficient, a transposed axis, a radian/degree slip or a hemisphere error —
//! which is the full list of ways this code can plausibly be wrong.

// Exact comparison is used only where the value is exact by construction.
#![allow(clippy::float_cmp, reason = "the central-meridian identity is exact")]

use openmobisim_core_graph::geometry::{
    Hemisphere, LonLat, Projected, Projection, ProjectionError, UTM_FALSE_EASTING,
    UTM_FALSE_NORTHING_SOUTH, UTM_K0, UtmZone, haversine_metres, scale_factor,
};
use proptest::prelude::*;

/// WGS84 semi-major axis and first eccentricity squared.
const A: f64 = 6_378_137.0;
const E2: f64 = 6.694_379_990_141_32e-3;

/// The exact ellipsoidal length of a short **north-south** baseline, in metres.
///
/// The meridian arc is `M(φ)·Δφ` with `M` the meridional radius of curvature.
/// Taking `M` at the midpoint makes this correct to well under a millimetre
/// over the 0.02° baselines used here — which is what allows the tolerances
/// below to be tight enough to mean something.
///
/// The spherical `haversine_metres` in the library is **not** accurate enough
/// for this job: at the equator a mean-radius sphere overstates a north-south
/// distance by 0.56 %, because the meridional radius of curvature there is
/// 6 335 km, not 6 371 km. That is the whole reason this helper exists.
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
    assert_eq!(UtmZone::new(31, Hemisphere::North).central_meridian_degrees(), 3.0);
    assert_eq!(UtmZone::new(33, Hemisphere::North).central_meridian_degrees(), 15.0);
    assert_eq!(UtmZone::new(1, Hemisphere::North).central_meridian_degrees(), -177.0);
    assert_eq!(UtmZone::new(60, Hemisphere::North).central_meridian_degrees(), 177.0);
}

#[test]
fn hemisphere_and_epsg_are_derived_correctly() {
    assert_eq!(zone_for(13.4, 52.5).hemisphere(), Hemisphere::North);
    assert_eq!(zone_for(-46.6, -23.5).hemisphere(), Hemisphere::South); // São Paulo
    assert_eq!(zone_for(13.4, 52.5).epsg(), 32_633);
    assert_eq!(zone_for(-46.6, -23.5).epsg(), 32_723);
}

#[test]
fn the_central_meridian_identity_is_exact() {
    // On the central meridian the easting is exactly the false easting, and at
    // the equator the northing is exactly zero. Any coefficient error shows up
    // here immediately.
    let zone = UtmZone::new(31, Hemisphere::North);
    let proj = Projection::wgs84(zone);
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
    let proj = Projection::wgs84(UtmZone::new(23, Hemisphere::South));
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
    let proj = Projection::wgs84(UtmZone::new(31, Hemisphere::North));
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
    let proj = Projection::wgs84(UtmZone::new(31, Hemisphere::North));
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
    let zone = UtmZone::new(31, Hemisphere::North);
    let proj = Projection::wgs84(zone);
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
fn the_library_haversine_is_only_as_good_as_it_claims() {
    // Documented as a sanity-check tool, not a geodesy tool. This pins the
    // claim so nobody later mistakes it for something to project with: at the
    // equator it overstates a north-south distance by about half a percent.
    let a = LonLat::new(3.0, 0.0);
    let b = LonLat::new(3.0, 0.1);
    let error = haversine_metres(a, b) / meridian_arc(0.0, 0.1) - 1.0;
    assert!(error > 0.004 && error < 0.007, "haversine error at the equator was {error}");
}

#[test]
fn scale_factor_is_k0_on_the_central_meridian_and_grows_outward() {
    let zone = UtmZone::new(31, Hemisphere::North);
    assert!((scale_factor(zone, LonLat::new(3.0, 45.0)) - UTM_K0).abs() < 1e-12);
    let edge = scale_factor(zone, LonLat::new(6.0, 45.0));
    assert!(edge > UTM_K0, "scale must grow away from the central meridian");
    assert!(edge < 1.001, "scale at the zone edge should stay under 0.1 %, was {edge}");
}

#[test]
fn a_zone_is_chosen_from_the_centre_of_the_study_area() {
    // Lyon: the box straddles nothing awkward, and the zone must be 31.
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

proptest! {
    /// Project and unproject, anywhere inside a zone, and come back to where
    /// you started — to well under a millimetre.
    ///
    /// This is the strongest statement available without a reference
    /// implementation: the forward and inverse series are independent
    /// expansions, so agreeing to 1e-9 degrees means both are right, not that
    /// they share a mistake.
    #[test]
    fn projection_round_trips(
        zone_number in 1u8..=60,
        south in any::<bool>(),
        dlon in -3.0f64..3.0,
        lat in -80.0f64..80.0,
    ) {
        let hemisphere = if south { Hemisphere::South } else { Hemisphere::North };
        let zone = UtmZone::new(zone_number, hemisphere);
        let proj = Projection::wgs84(zone);

        let lon = zone.central_meridian_degrees() + dlon;
        prop_assume!((-180.0..=180.0).contains(&lon));

        let original = LonLat::new(lon, lat);
        let back = proj.unproject(proj.project(original));

        // 1e-9 degrees is about 0.1 mm of latitude.
        prop_assert!(
            (back.lat - original.lat).abs() < 1e-9,
            "latitude {} came back as {}", original.lat, back.lat
        );
        prop_assert!(
            (back.lon - original.lon).abs() < 1e-9,
            "longitude {} came back as {}", original.lon, back.lon
        );
    }

    /// Projected distance tracks spherical distance over short baselines,
    /// everywhere in a zone, within the scale distortion the projection admits.
    #[test]
    fn short_distances_are_preserved_within_the_scale_distortion(
        dlon in -2.5f64..2.5,
        lat in -70.0f64..70.0,
        bearing_east in any::<bool>(),
    ) {
        let zone = UtmZone::new(31, Hemisphere::North);
        let proj = Projection::wgs84(zone);
        let a = LonLat::new(zone.central_meridian_degrees() + dlon, lat);
        let b = if bearing_east {
            LonLat::new(a.lon + 0.02, a.lat)
        } else {
            LonLat::new(a.lon, a.lat + 0.02)
        };

        let projected = proj.project(a).distance_to(proj.project(b));
        let exact = if bearing_east {
            parallel_arc(a.lat, a.lon, b.lon)
        } else {
            meridian_arc(a.lat, b.lat)
        };
        prop_assume!(exact > 100.0);

        let ratio = projected / exact;
        let expected = scale_factor(zone, a);
        // `scale_factor` is itself a second-order approximation, so the
        // tolerance is set by that rather than by the projection.
        prop_assert!(
            (ratio - expected).abs() < 1e-4,
            "ratio {} vs expected scale {} at ({}, {})", ratio, expected, a.lon, a.lat
        );
    }

    /// Unprojecting an arbitrary plausible coordinate and projecting it back is
    /// also a round trip — the inverse must be a true inverse, not merely a
    /// left inverse.
    #[test]
    fn unprojection_round_trips(
        x in 200_000.0f64..800_000.0,
        y in 1_000_000.0f64..8_000_000.0,
    ) {
        let proj = Projection::wgs84(UtmZone::new(31, Hemisphere::North));
        let original = Projected::new(x, y);
        let back = proj.project(proj.unproject(original));
        prop_assert!((back.x - original.x).abs() < 1e-4, "easting {} came back as {}", x, back.x);
        prop_assert!((back.y - original.y).abs() < 1e-4, "northing {} came back as {}", y, back.y);
    }
}
