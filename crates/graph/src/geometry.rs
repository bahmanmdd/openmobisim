//! Coordinates: longitude/latitude (WGS84) in, metres out, and nothing else.
//!
//! # One projection family
//!
//! Supporting arbitrary coordinate reference systems means, in practice,
//! depending on PROJ, whose mature Rust route is bindings to a C library.
//! That dependency would cost a plain `pip install` with no compiler and no
//! system libraries. So the core supports **one** projection family,
//! implemented from scratch in arithmetic with no dependencies at all.
//!
//! Users with a national grid (Lambert-93, Dutch RD, British National Grid)
//! reproject before import, or give positions in metres directly
//! ([`Position::Metres`]). OpenStreetMap and GTFS are WGS84 already.
//!
//! # The method
//!
//! Transverse Mercator by the **Krüger series in the third flattening `n`**,
//! carried to fourth order (Krüger, 1912, in the form of Karney, 2011). Accurate to well under a millimetre
//! anywhere inside a UTM zone. `tests/geometry.rs` checks it against the
//! ellipsoid's meridian and parallel arcs and the exact identities on the
//! central meridian.
//!
//! # Zone selection
//!
//! [`UtmZone::for_centroid`] applies the plain six-degree formula and
//! **deliberately ignores the Norway and Svalbard exceptions** of the official
//! UTM grid. Those exceptions exist so that grid squares line up with MGRS;
//! here the projection is chosen to minimise distortion across one study area,
//! and honouring them would move the central meridian *away* from it.

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

/// A point in WGS84 degrees, as it arrives from OpenStreetMap or GTFS.
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

/// A point in metres on a plane: projected, or given in metres. The only coordinates the core
/// works in.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Xy {
    /// Easting, in metres.
    pub x: f64,
    /// Northing, in metres.
    pub y: f64,
}

impl Xy {
    /// A point.
    #[inline]
    #[must_use]
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    /// Straight-line distance to another point, in metres.
    ///
    /// Exact in the plane. For projected points the projection's own scale
    /// distortion applies: a factor 0.9996 on the central meridian, rising to
    /// about 1.001 at a zone's edge, so within 0.1 % of the ground distance.
    #[inline]
    #[must_use]
    pub fn distance_to(self, other: Xy) -> f64 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        libm::hypot(dx, dy)
    }
}

/// Where a node is: longitude/latitude, projected once by the network builder, or already in
/// metres on a plane, used as it is. A network's nodes are all of one kind.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Position {
    /// WGS84 degrees.
    LonLat(LonLat),
    /// Metres on a plane (x east, y north), not projected.
    Metres(Xy),
}

/// Which hemisphere a zone is used in. Decides the false northing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hemisphere {
    /// Northern: false northing zero.
    North,
    /// Southern: false northing 10 000 000 m, so northings stay positive.
    South,
}

/// A UTM zone: the projection a whole network is built in.
///
/// Chosen once, at build time, from the study area's bounding box.
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
}

impl fmt::Display for UtmZone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let h = match self.hemisphere {
            Hemisphere::North => 'N',
            Hemisphere::South => 'S',
        };
        write!(f, "UTM zone {}{h}", self.number)
    }
}

/// The Krüger series coefficients for one zone on the WGS84 ellipsoid, precomputed.
///
/// Holding the coefficients rather than recomputing them keeps
/// [`Projection::project`] to arithmetic only: it runs once per node.
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
    /// Out-of-range input is not rejected here: this runs once per node, and the
    /// network builder has already checked every node with
    /// [`LonLat::is_projectable`].
    #[must_use]
    pub fn project(&self, p: LonLat) -> Xy {
        let phi = p.lat.to_radians();
        let lambda = p.lon.to_radians() - self.lambda0;

        // Gauss–Schreiber intermediate:
        //     t = sinh( atanh(sin φ) − c · atanh(c · sin φ) ),   c = 2√n/(1+n)
        let sin_phi = libm::sin(phi);
        let c = self.two_sqrt_n_over_1pn;
        let t = libm::sinh(libm::atanh(sin_phi) - c * libm::atanh(c * sin_phi));

        let (sin_lambda, cos_lambda) = libm::sincos(lambda);
        let xi_p = libm::atan2(t, cos_lambda);
        let eta_p = libm::atanh(sin_lambda / (1.0 + t * t).sqrt());

        let mut xi = xi_p;
        let mut eta = eta_p;
        for (j, &a) in self.alpha.iter().enumerate() {
            #[allow(clippy::cast_precision_loss, reason = "j is 0..4")]
            let m = 2.0 * (j + 1) as f64;
            xi += a * libm::sin(m * xi_p) * libm::cosh(m * eta_p);
            eta += a * libm::cos(m * xi_p) * libm::sinh(m * eta_p);
        }

        Xy { x: UTM_FALSE_EASTING + self.k0_a * eta, y: self.false_northing + self.k0_a * xi }
    }

    /// Metres of this projection back to WGS84 degrees: the inverse of [`Projection::project`]
    /// (Krüger's inverse series for the conformal latitude, then the latitude by Newton's method
    /// on the relation `project` uses, Karney 2011, to the last bit or two: a point projected
    /// and brought back is within 1e-11 degree, a micrometre).
    #[must_use]
    pub fn unproject(&self, p: Xy) -> LonLat {
        let xi = (p.y - self.false_northing) / self.k0_a;
        let eta = (p.x - UTM_FALSE_EASTING) / self.k0_a;
        let (mut xi_p, mut eta_p) = (xi, eta);
        for (j, &b) in self.beta.iter().enumerate() {
            #[allow(clippy::cast_precision_loss, reason = "j is 0..4")]
            let m = 2.0 * (j + 1) as f64;
            xi_p -= b * libm::sin(m * xi) * libm::cosh(m * eta);
            eta_p -= b * libm::cos(m * xi) * libm::sinh(m * eta);
        }
        let lambda = libm::atan2(libm::sinh(eta_p), libm::cos(xi_p));
        // t = tan of the conformal latitude; tau = tan of the latitude, with
        // t(tau) = tau √(1 + σ²) − σ √(1 + tau²), σ = sinh(e · atanh(e · tau / √(1 + tau²))).
        let t = libm::sin(xi_p) / libm::hypot(libm::sinh(eta_p), libm::cos(xi_p));
        let e = self.two_sqrt_n_over_1pn;
        let mut tau = t;
        for _ in 0..8 {
            let root = libm::hypot(1.0, tau);
            let sigma = libm::sinh(e * libm::atanh(e * tau / root));
            let t_of = tau * libm::hypot(1.0, sigma) - sigma * root;
            let step = (t - t_of) / libm::hypot(1.0, t_of) * (1.0 + (1.0 - e * e) * tau * tau)
                / ((1.0 - e * e) * root);
            tau += step;
            if step.abs() <= 1e-15 * tau.abs().max(1.0) {
                break;
            }
        }
        LonLat::new((self.lambda0 + lambda).to_degrees(), libm::atan(tau).to_degrees())
    }
}
