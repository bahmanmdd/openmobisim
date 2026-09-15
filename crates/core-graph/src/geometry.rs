//! Coordinates: WGS84 in, projected metres out — and nothing else (S81,
//! Foundations §9).
//!
//! # Why this file exists at all
//!
//! Supporting arbitrary coordinate reference systems means, in practice,
//! depending on PROJ — whose mature Rust route is bindings to a C library.
//! That single dependency would cost the strongest form of the install
//! promise: `pip install`, no compiler, no system libraries, on all three
//! operating systems. So the core supports **one** projection family, and
//! implements it from scratch in about two hundred lines of arithmetic with no
//! dependencies at all.
//!
//! Users with a national grid — Lambert-93, Dutch RD, British National Grid —
//! reproject before import. OSM and GTFS are WGS84 already, so this never bites
//! the zero-data path.
//!
//! # The method
//!
//! Transverse Mercator by the **Krüger series in the third flattening `n`**,
//! carried to fourth order. Accurate to well under a millimetre anywhere inside
//! a UTM zone, which is four orders of magnitude finer than anything a mobility
//! model can distinguish. The alternative, Snyder's series in `e²`, is about as
//! long and less accurate; there is no reason to prefer it.
//!
//! Accuracy is asserted three ways in `tests/geometry.rs`: an exact identity on
//! the central meridian, a round-trip to sub-millimetre over a property-test
//! sweep of the whole zone, and a distance check against the Vincenty geodesic.
//!
//! # Zone selection is ours, not the UTM standard's
//!
//! [`UtmZone::for_centroid`] applies the plain formula and **deliberately
//! ignores the Norway and Svalbard exceptions** that the official UTM grid
//! carries. Those exceptions exist so that grid squares line up with the
//! MGRS system; we are choosing a projection to minimise distortion across one
//! study area, and honouring them would move the central meridian *away* from
//! the study area it is meant to sit under. The choice is recorded rather than
//! silent, because it is the kind of thing that looks like a bug to a
//! geodesist.

use core::fmt;

/// WGS84 semi-major axis, in metres.
pub const WGS84_A: f64 = 6_378_137.0;
/// WGS84 inverse flattening.
pub const WGS84_INV_F: f64 = 298.257_223_563;
/// The UTM scale factor on the central meridian.
pub const UTM_K0: f64 = 0.999_6;
/// The UTM false easting, in metres.
pub const UTM_FALSE_EASTING: f64 = 500_000.0;
/// The UTM false northing applied in the southern hemisphere, in metres.
pub const UTM_FALSE_NORTHING_SOUTH: f64 = 10_000_000.0;

/// A point in WGS84 degrees, as it arrives from OSM or GTFS.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct LonLat {
    /// Longitude in degrees, positive east.
    pub lon: f64,
    /// Latitude in degrees, positive north.
    pub lat: f64,
}

impl LonLat {
    /// A point.
    #[inline]
    #[must_use]
    pub const fn new(lon: f64, lat: f64) -> Self {
        Self { lon, lat }
    }

    /// Whether the coordinates are finite and within the valid ranges.
    ///
    /// Latitudes beyond ±84° are outside the UTM system; the transverse
    /// Mercator series degrades badly at the poles, and no mobility study
    /// happens there.
    #[must_use]
    pub fn is_projectable(self) -> bool {
        self.lon.is_finite()
            && self.lat.is_finite()
            && (-180.0..=180.0).contains(&self.lon)
            && (-84.0..=84.0).contains(&self.lat)
    }
}

/// A point in projected metres — the only coordinates the core works in.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Projected {
    /// Easting, in metres.
    pub x: f64,
    /// Northing, in metres.
    pub y: f64,
}

impl Projected {
    /// A point.
    #[inline]
    #[must_use]
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    /// Straight-line distance to another projected point, in metres.
    ///
    /// Exact in the projected plane; the projection's own distortion is
    /// `1/k0 - 1` ≈ 0.04 % on the central meridian, rising toward the zone
    /// edges. That is far inside the error of any network geometry.
    #[inline]
    #[must_use]
    pub fn distance_to(self, other: Projected) -> f64 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        dx.hypot(dy)
    }
}

/// Which hemisphere a zone is used in. Decides the false northing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hemisphere {
    /// Northern: false northing zero.
    North,
    /// Southern: false northing 10 000 000 m, so northings stay positive.
    South,
}

/// A UTM zone: the projection the whole scenario is built in.
///
/// Chosen once, at build time, from the study area's bounding box, and then
/// carried by the network fingerprint — two builds of the same input must
/// project identically or every cached artifact is incomparable.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct UtmZone {
    number: u8,
    hemisphere: Hemisphere,
}

/// Why a study area could not be projected.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ProjectionError {
    /// A coordinate was not finite, or was outside the projectable range.
    NotProjectable {
        /// The offending point.
        point: LonLat,
    },
    /// The bounding box was empty — no points were offered.
    EmptyExtent,
}

impl fmt::Display for ProjectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProjectionError::NotProjectable { point } => write!(
                f,
                "coordinate ({}, {}) is not projectable: longitude must be within ±180°, \
                 latitude within ±84°, and both finite",
                point.lon, point.lat
            ),
            ProjectionError::EmptyExtent => {
                f.write_str("cannot choose a projection for an empty study area")
            }
        }
    }
}

impl core::error::Error for ProjectionError {}

impl UtmZone {
    /// The zone whose central meridian best suits a study area centred on
    /// `centroid`.
    ///
    /// # Errors
    ///
    /// [`ProjectionError::NotProjectable`] if the centroid is out of range.
    ///
    /// # Note
    ///
    /// The Norway and Svalbard exceptions of the official UTM grid are
    /// deliberately not applied; see the [module docs](self).
    pub fn for_centroid(centroid: LonLat) -> Result<Self, ProjectionError> {
        if !centroid.is_projectable() {
            return Err(ProjectionError::NotProjectable { point: centroid });
        }
        // Zones are six degrees wide, numbered 1..=60 from 180°W.
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "the quotient is in 0..=60 because longitude is within ±180°"
        )]
        let number = (((centroid.lon + 180.0) / 6.0).floor() as u8).min(59) + 1;
        Ok(Self {
            number,
            hemisphere: if centroid.lat >= 0.0 { Hemisphere::North } else { Hemisphere::South },
        })
    }

    /// Build a zone explicitly, for reading back a stored artifact.
    ///
    /// # Panics
    ///
    /// Panics if `number` is outside `1..=60`.
    #[must_use]
    pub const fn new(number: u8, hemisphere: Hemisphere) -> Self {
        assert!(number >= 1 && number <= 60, "UTM zone number must be in 1..=60");
        Self { number, hemisphere }
    }

    /// The zone number, `1..=60`.
    #[inline]
    #[must_use]
    pub const fn number(self) -> u8 {
        self.number
    }

    /// Which hemisphere the false northing is set for.
    #[inline]
    #[must_use]
    pub const fn hemisphere(self) -> Hemisphere {
        self.hemisphere
    }

    /// The zone's central meridian, in degrees.
    #[inline]
    #[must_use]
    pub fn central_meridian_degrees(self) -> f64 {
        f64::from(self.number) * 6.0 - 183.0
    }

    /// The EPSG code for this zone, for labelling exported files.
    ///
    /// 326xx in the north, 327xx in the south.
    #[inline]
    #[must_use]
    pub const fn epsg(self) -> u16 {
        let base: u16 = match self.hemisphere {
            Hemisphere::North => 32_600,
            Hemisphere::South => 32_700,
        };
        base + self.number as u16
    }

    /// How many degrees of longitude `lon` sits from the central meridian.
    ///
    /// Distortion grows with this; the builder warns past about 4.5°, which is
    /// where the scale error exceeds roughly 0.1 %.
    #[must_use]
    pub fn degrees_from_central_meridian(self, lon: f64) -> f64 {
        (lon - self.central_meridian_degrees()).abs()
    }
}

impl fmt::Display for UtmZone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let h = match self.hemisphere {
            Hemisphere::North => 'N',
            Hemisphere::South => 'S',
        };
        write!(f, "UTM {}{h} (EPSG:{})", self.number, self.epsg())
    }
}

/// The Krüger series coefficients for one ellipsoid, precomputed.
///
/// Built once per scenario and shared. Holding the coefficients rather than
/// recomputing them keeps [`Projection::project`] to arithmetic only, which
/// matters because it runs once per OSM node — millions of times on a city
/// import.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Projection {
    zone: UtmZone,
    /// Rectifying radius times the scale factor.
    k0_a: f64,
    /// Central meridian, in radians.
    lambda0: f64,
    /// False northing for the hemisphere.
    false_northing: f64,
    /// Forward series coefficients α₁..α₄.
    alpha: [f64; 4],
    /// Inverse series coefficients β₁..β₄.
    beta: [f64; 4],
    /// Inverse latitude series coefficients δ₁..δ₄.
    delta: [f64; 4],
    /// `2√n / (1 + n)`, the conformal-latitude constant.
    two_sqrt_n_over_1pn: f64,
}

impl Projection {
    /// Build the projection for a zone on the WGS84 ellipsoid.
    #[must_use]
    pub fn wgs84(zone: UtmZone) -> Self {
        let f = 1.0 / WGS84_INV_F;
        let n = f / (2.0 - f);
        let (n2, n3, n4) = (n * n, n * n * n, n * n * n * n);

        // Rectifying radius A = a/(1+n) · (1 + n²/4 + n⁴/64 + …)
        let a_rect = WGS84_A / (1.0 + n) * (1.0 + n2 / 4.0 + n4 / 64.0);

        let alpha = [
            n / 2.0 - 2.0 * n2 / 3.0 + 5.0 * n3 / 16.0 + 41.0 * n4 / 180.0,
            13.0 * n2 / 48.0 - 3.0 * n3 / 5.0 + 557.0 * n4 / 1440.0,
            61.0 * n3 / 240.0 - 103.0 * n4 / 140.0,
            49561.0 * n4 / 161_280.0,
        ];
        let beta = [
            n / 2.0 - 2.0 * n2 / 3.0 + 37.0 * n3 / 96.0 - n4 / 360.0,
            n2 / 48.0 + n3 / 15.0 - 437.0 * n4 / 1440.0,
            17.0 * n3 / 480.0 - 37.0 * n4 / 840.0,
            4397.0 * n4 / 161_280.0,
        ];
        let delta = [
            2.0 * n - 2.0 * n2 / 3.0 - 2.0 * n3 + 116.0 * n4 / 45.0,
            7.0 * n2 / 3.0 - 8.0 * n3 / 5.0 - 227.0 * n4 / 45.0,
            56.0 * n3 / 15.0 - 136.0 * n4 / 35.0,
            4279.0 * n4 / 630.0,
        ];

        Self {
            zone,
            k0_a: UTM_K0 * a_rect,
            lambda0: zone.central_meridian_degrees().to_radians(),
            false_northing: match zone.hemisphere() {
                Hemisphere::North => 0.0,
                Hemisphere::South => UTM_FALSE_NORTHING_SOUTH,
            },
            alpha,
            beta,
            delta,
            two_sqrt_n_over_1pn: 2.0 * n.sqrt() / (1.0 + n),
        }
    }

    /// Choose a zone from a study area's bounding box and build its projection.
    ///
    /// The zone is taken from the box's **centroid**, so the central meridian
    /// sits under the middle of the study area.
    ///
    /// # Errors
    ///
    /// [`ProjectionError::EmptyExtent`] if `points` is empty, or
    /// [`ProjectionError::NotProjectable`] for the first unusable coordinate.
    pub fn for_extent(points: impl IntoIterator<Item = LonLat>) -> Result<Self, ProjectionError> {
        let mut min = LonLat::new(f64::INFINITY, f64::INFINITY);
        let mut max = LonLat::new(f64::NEG_INFINITY, f64::NEG_INFINITY);
        let mut seen = false;

        for p in points {
            if !p.is_projectable() {
                return Err(ProjectionError::NotProjectable { point: p });
            }
            seen = true;
            min.lon = min.lon.min(p.lon);
            min.lat = min.lat.min(p.lat);
            max.lon = max.lon.max(p.lon);
            max.lat = max.lat.max(p.lat);
        }
        if !seen {
            return Err(ProjectionError::EmptyExtent);
        }

        let centroid = LonLat::new((min.lon + max.lon) / 2.0, (min.lat + max.lat) / 2.0);
        Ok(Self::wgs84(UtmZone::for_centroid(centroid)?))
    }

    /// The zone this projects into.
    #[inline]
    #[must_use]
    pub const fn zone(self) -> UtmZone {
        self.zone
    }

    /// Project WGS84 degrees to metres.
    ///
    /// Out-of-range input is not rejected here — this runs once per OSM node,
    /// and a branch per node to re-check what the builder already validated is
    /// exactly the kind of cost §3e forbids. Validate the extent once, with
    /// [`LonLat::is_projectable`], then project freely.
    #[must_use]
    pub fn project(&self, p: LonLat) -> Projected {
        let phi = p.lat.to_radians();
        let lambda = p.lon.to_radians() - self.lambda0;

        // Gauss–Schreiber intermediate:
        //     t = sinh( atanh(sin φ) − c · atanh(c · sin φ) ),   c = 2√n/(1+n)
        let sin_phi = phi.sin();
        let c = self.two_sqrt_n_over_1pn;
        let t = (sin_phi.atanh() - c * (c * sin_phi).atanh()).sinh();

        let (sin_lambda, cos_lambda) = lambda.sin_cos();
        let xi_p = t.atan2(cos_lambda);
        let eta_p = (sin_lambda / (1.0 + t * t).sqrt()).atanh();

        let mut xi = xi_p;
        let mut eta = eta_p;
        for (j, &a) in self.alpha.iter().enumerate() {
            #[allow(clippy::cast_precision_loss, reason = "j is 0..4")]
            let m = 2.0 * (j + 1) as f64;
            xi += a * (m * xi_p).sin() * (m * eta_p).cosh();
            eta += a * (m * xi_p).cos() * (m * eta_p).sinh();
        }

        Projected {
            x: UTM_FALSE_EASTING + self.k0_a * eta,
            y: self.false_northing + self.k0_a * xi,
        }
    }

    /// Unproject metres back to WGS84 degrees.
    ///
    /// Used for outputs and for round-trip verification, never on a hot path.
    #[must_use]
    pub fn unproject(&self, p: Projected) -> LonLat {
        let xi = (p.y - self.false_northing) / self.k0_a;
        let eta = (p.x - UTM_FALSE_EASTING) / self.k0_a;

        let mut xi_p = xi;
        let mut eta_p = eta;
        for (j, &b) in self.beta.iter().enumerate() {
            #[allow(clippy::cast_precision_loss, reason = "j is 0..4")]
            let m = 2.0 * (j + 1) as f64;
            xi_p -= b * (m * xi).sin() * (m * eta).cosh();
            eta_p -= b * (m * xi).cos() * (m * eta).sinh();
        }

        let chi = (xi_p.sin() / eta_p.cosh()).asin();
        let mut phi = chi;
        for (j, &d) in self.delta.iter().enumerate() {
            #[allow(clippy::cast_precision_loss, reason = "j is 0..4")]
            let m = 2.0 * (j + 1) as f64;
            phi += d * (m * chi).sin();
        }

        let lambda = self.lambda0 + (eta_p.sinh()).atan2(xi_p.cos());
        LonLat { lon: normalise_longitude(lambda.to_degrees()), lat: phi.to_degrees() }
    }
}

/// Wrap a longitude into `[-180, 180]`.
fn normalise_longitude(deg: f64) -> f64 {
    let mut d = deg;
    while d > 180.0 {
        d -= 360.0;
    }
    while d < -180.0 {
        d += 360.0;
    }
    d
}

/// The great-circle distance between two WGS84 points, in metres.
///
/// Spherical, on the mean Earth radius. **Accurate to about half a percent** —
/// the meridional radius of curvature at the equator is 6 335 km, not the
/// 6 371 km mean, so a north-south baseline there comes out 0.56 % long.
///
/// That is good enough to sanity-check an OSM extract's extent, to sort
/// candidates before an exact test, and to put a number in a log message. It is
/// **not** good enough to build network geometry from, and it is not good
/// enough to verify the projection against — `tests/geometry.rs` uses the
/// ellipsoidal meridian and parallel arcs for that. The core never uses this
/// for anything a result depends on.
#[must_use]
pub fn haversine_metres(a: LonLat, b: LonLat) -> f64 {
    /// Mean Earth radius, in metres (IUGG).
    const R: f64 = 6_371_008.8;
    let (phi1, phi2) = (a.lat.to_radians(), b.lat.to_radians());
    let dphi = phi2 - phi1;
    let dlambda = (b.lon - a.lon).to_radians();
    let h = (dphi / 2.0).sin().powi(2) + phi1.cos() * phi2.cos() * (dlambda / 2.0).sin().powi(2);
    2.0 * R * h.sqrt().asin()
}

/// WGS84 first eccentricity squared, `e² = f(2 − f)`.
pub const WGS84_E2: f64 = 6.694_379_990_141_32e-3;

/// The true ground distance between two nearby WGS84 points, in metres.
///
/// This is what a link's **length** is measured with. It is deliberately not
/// the distance between the two projected points: a link's length is a fact
/// about the street, and it should not change because a different UTM zone was
/// chosen for the study area.
///
/// The method is the local-radii approximation — the meridian arc from the
/// meridional radius of curvature `M(φ)`, the parallel arc from `N(φ)·cos φ`,
/// both evaluated at the midpoint, combined by Pythagoras:
///
/// ```text
/// dy = M(φm) · Δφ        dx = N(φm) · cos(φm) · Δλ        d = √(dx² + dy²)
/// ```
///
/// **Accurate to well under a millimetre for segments up to a few kilometres**,
/// which is every OSM way segment there is. It degrades over hundreds of
/// kilometres, where a proper geodesic is needed — and where nothing in a
/// mobility network lives.
///
/// Prefer this over [`haversine_metres`] for anything a result depends on;
/// that one is half a percent out and exists only for sanity checks.
///
/// # Examples
///
/// ```
/// use openmobisim_core_graph::geometry::{ground_distance_metres, LonLat};
///
/// // A tenth of a degree of latitude at the equator: about 11.06 km, not the
/// // 11.12 km a mean-radius sphere would claim.
/// let d = ground_distance_metres(LonLat::new(0.0, 0.0), LonLat::new(0.0, 0.1));
/// assert!((d - 11_055.0).abs() < 5.0, "got {d}");
/// ```
#[must_use]
pub fn ground_distance_metres(a: LonLat, b: LonLat) -> f64 {
    let mid = ((a.lat + b.lat) / 2.0).to_radians();
    let sin_mid = mid.sin();
    let w = 1.0 - WGS84_E2 * sin_mid * sin_mid;
    let sqrt_w = w.sqrt();

    let meridional = WGS84_A * (1.0 - WGS84_E2) / (w * sqrt_w);
    let transverse = WGS84_A / sqrt_w;

    let dy = meridional * (b.lat - a.lat).to_radians();
    let dx = transverse * mid.cos() * (b.lon - a.lon).to_radians();
    dx.hypot(dy)
}

/// The total ground length of a polyline, in metres.
///
/// The sum of [`ground_distance_metres`] over consecutive pairs. This is how an
/// imported way's length is computed: the geometry between two junctions is
/// what makes the link longer than the straight line between its ends, and
/// throwing it away would understate travel time on every curved street.
#[must_use]
pub fn polyline_length_metres(points: &[LonLat]) -> f64 {
    points.windows(2).map(|w| ground_distance_metres(w[0], w[1])).sum()
}

/// The scale factor of the projection at a point.
///
/// `k0` on the central meridian, growing toward the zone edges. Exposed so the
/// builder can report the worst distortion over the study area rather than
/// leaving a user to wonder.
#[must_use]
pub fn scale_factor(zone: UtmZone, p: LonLat) -> f64 {
    // k ≈ k0 · (1 + (Δλ·cos φ)²/2) to second order — enough to report with.
    let dlambda = (p.lon - zone.central_meridian_degrees()).to_radians();
    let c = dlambda * p.lat.to_radians().cos();
    UTM_K0 * (1.0 + c * c / 2.0)
}
