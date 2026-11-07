//! The length of a segment: the geodesic on the WGS84 ellipsoid, by Vincenty's inverse formula.
//!
//! T. Vincenty, "Direct and inverse solutions of geodesics on the ellipsoid with application of
//! nested equations", *Survey Review* 23 (176), 1975, pp. 88–93, equations as numbered there.
//! The iteration on the auxiliary longitude λ stops when λ changes by less than 1e-14 rad (the
//! usual 1e-12 rad leaves about 6 µm per segment on the ground, measured on Delft's ways
//! against pyproj); it converges for any two points that are not nearly antipodal, and a street
//! segment never is. A segment that does not
//! converge in 200 iterations, or a position that is not finite, has no length (`None`), and
//! the reader names its way.

/// WGS84 semi-major axis, metres.
pub const WGS84_A: f64 = 6_378_137.0;
/// WGS84 flattening.
pub const WGS84_F: f64 = 1.0 / 298.257_223_563;

/// The geodesic distance in metres between two points given as (longitude, latitude) in WGS84
/// degrees; `None` if it does not converge or a coordinate is not finite.
#[must_use]
pub fn geodesic_m(a: (f64, f64), b: (f64, f64)) -> Option<f64> {
    if ![a.0, a.1, b.0, b.1].iter().all(|v| v.is_finite()) {
        return None;
    }
    let f = WGS84_F;
    let semi_minor = WGS84_A * (1.0 - f);
    let l = (b.0 - a.0).to_radians();
    let (sin_u1, cos_u1) = ((1.0 - f) * a.1.to_radians().tan()).atan().sin_cos();
    let (sin_u2, cos_u2) = ((1.0 - f) * b.1.to_radians().tan()).atan().sin_cos();
    let mut lambda = l;
    for _ in 0..200 {
        let (sin_l, cos_l) = lambda.sin_cos();
        let sin_sigma = (cos_u2 * sin_l).hypot(cos_u1 * sin_u2 - sin_u1 * cos_u2 * cos_l);
        if sin_sigma == 0.0 {
            return Some(0.0); // the same point
        }
        let cos_sigma = sin_u1 * sin_u2 + cos_u1 * cos_u2 * cos_l;
        let sigma = sin_sigma.atan2(cos_sigma);
        let sin_alpha = cos_u1 * cos_u2 * sin_l / sin_sigma;
        let cos2_alpha = 1.0 - sin_alpha * sin_alpha;
        // On the equator cos²α = 0 and the term vanishes (Vincenty's remark after (17)).
        let cos_2sm =
            if cos2_alpha == 0.0 { 0.0 } else { cos_sigma - 2.0 * sin_u1 * sin_u2 / cos2_alpha };
        let c = f / 16.0 * cos2_alpha * (4.0 + f * (4.0 - 3.0 * cos2_alpha)); // (10)
        let previous = lambda;
        lambda = l
            + (1.0 - c)
                * f
                * sin_alpha
                * (sigma
                    + c * sin_sigma * (cos_2sm + c * cos_sigma * (-1.0 + 2.0 * cos_2sm * cos_2sm))); // (11)
        if (lambda - previous).abs() < 1e-14 {
            let u2 = cos2_alpha * (WGS84_A * WGS84_A - semi_minor * semi_minor)
                / (semi_minor * semi_minor);
            let big_a = 1.0 + u2 / 16_384.0 * (4_096.0 + u2 * (-768.0 + u2 * (320.0 - 175.0 * u2))); // (3)
            let big_b = u2 / 1_024.0 * (256.0 + u2 * (-128.0 + u2 * (74.0 - 47.0 * u2))); // (4)
            let delta_sigma = big_b
                * sin_sigma
                * (cos_2sm
                    + big_b / 4.0
                        * (cos_sigma * (-1.0 + 2.0 * cos_2sm * cos_2sm)
                            - big_b / 6.0
                                * cos_2sm
                                * (-3.0 + 4.0 * sin_sigma * sin_sigma)
                                * (-3.0 + 4.0 * cos_2sm * cos_2sm))); // (6)
            return Some(semi_minor * big_a * (sigma - delta_sigma)); // (19)
        }
    }
    None
}
