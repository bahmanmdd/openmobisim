//! The link-geometry artifact: the shape of each street, kept **outside**
//! [`RoadNetwork`] (S125).
//!
//! # Why an artifact and not a field
//!
//! Routing and loading read a link's length, never its shape — nothing here
//! is on their hot path. The stop snapper, the map matcher, the SUMO exporter
//! and anything that plots the network need the shape; every one of them runs
//! outside the iteration loop. A [`LinkGeometry`] is built once, held
//! alongside a [`RoadNetwork`] rather than inside it, and a run that never
//! asks for it never pays for it (brief §3e) — this is what keeps the road
//! network's structure-of-arrays layout from doubling in size for the sake of
//! a feature most runs do not touch.
//!
//! # What is stored
//!
//! - **Raw OSM points, not a simplified line.** Simplification belongs at
//!   *export*, where the tolerance can be chosen for the target; applied at
//!   storage time it is irreversible, and the map matcher is the thing it
//!   would break.
//! - **WGS84 lon/lat, `f64`.** Projected metres are valid only for the UTM
//!   zone the study area's extent selected (Foundations §9); storing them
//!   would bake a build parameter into stored data.
//! - **One point run per undirected street.** A two-way street's two
//!   directed links share one run; the direction that does not own it reads
//!   the same points back to front — see [`LinkGeometry::points`].
//! - **Length is never re-derived from this.** [`RoadNetwork::link_length`]
//!   stays the measured fact of S119; `LinkGeometry` is for placement and
//!   display only. [`LinkGeometry::build`] checks the two agree to a
//!   micrometre in every debug build, which is how that separation stays
//!   honest rather than assumed.

use std::collections::HashMap;

use openmobisim_core_types::ids::{EntityId, LinkId};

use crate::geometry::{LonLat, polyline_length_metres};
use crate::network::RoadNetwork;

/// The sentinel meaning "this link has no stored geometry".
const NO_STREET: u32 = u32::MAX;

/// Two links agreeing to within this many metres are treated as sharing one
/// physical street when pairing geometry (S125's "micrometre" tolerance).
///
/// A true reverse pair's lengths are the same length measured twice from the
/// same underlying points, so they agree far inside this; two distinct
/// one-way streets that happen to share both endpoints — a short one-way
/// loop, a divided carriageway — essentially never coincide to a micrometre,
/// so this is what keeps them from being mistaken for one street's two
/// directions.
const LENGTH_AGREEMENT_TOLERANCE_M: f64 = 1e-6;

/// A lightweight identity for the exact [`RoadNetwork`] a [`LinkGeometry`]
/// was built from.
///
/// **Not** the full cache fingerprint Foundations §8 describes —
/// `BLAKE3(input_bytes_hash, build_params, CODE_VERSION, defaults_version)`
/// — which belongs to the artifact cache (S65, not yet built: roadmap Phase 2
/// item 1) and needs the scenario builder's input bytes and build parameters,
/// neither of which this module has. This is a content hash of the network's
/// own identity — every node and link external id, in the deterministic
/// order [`Foundations §1`](crate) assigns them — good enough to catch a
/// geometry artifact paired with a network it was not built from. When the
/// artifact cache lands, this can feed into that wider formula rather than
/// being replaced by it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct NetworkFingerprint(u64);

impl NetworkFingerprint {
    /// The fingerprint as a number, for storing in an artifact's header.
    #[must_use]
    pub fn value(self) -> u64 {
        self.0
    }

    /// Fingerprint a built network's identity.
    #[must_use]
    pub fn of(network: &RoadNetwork) -> Self {
        let mut hash = Fnv1a::new();
        hash.write_u32(network.node_count());
        for external in network.node_external_ids().iter() {
            hash.write_bytes(external.as_bytes());
        }
        hash.write_u32(network.link_count());
        for external in network.link_external_ids().iter() {
            hash.write_bytes(external.as_bytes());
        }
        Self(hash.finish())
    }
}

/// A hand-written FNV-1a 64-bit hash.
///
/// Not a cryptographic hash and not meant as one — [`NetworkFingerprint`]
/// only has to catch an accidental mismatch, not resist a deliberate one. A
/// dependency-free ten-line function does that without adding to the
/// dependency graph the wheel-building promise has to keep pure Rust (§3).
struct Fnv1a(u64);

impl Fnv1a {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    fn new() -> Self {
        Self(Self::OFFSET_BASIS)
    }

    fn write_bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(Self::PRIME);
        }
    }

    fn write_u32(&mut self, v: u32) {
        self.write_bytes(&v.to_le_bytes());
    }

    fn finish(self) -> u64 {
        self.0
    }
}

/// The shape of every street in a network, kept outside the network itself.
///
/// See the [module docs](self) for what is stored and why. Built by
/// [`LinkGeometry::build`] from the same per-link points an importer already
/// has at hand; read back per directed link with [`LinkGeometry::points`].
#[derive(Clone, Debug)]
pub struct LinkGeometry {
    fingerprint: NetworkFingerprint,
    /// `street_count() + 1` entries; street `s`'s points are
    /// `points[offsets[s]..offsets[s + 1]]`.
    offsets: Vec<u32>,
    points: Vec<LonLat>,
    /// Per directed link: which street it reads from, or [`NO_STREET`].
    street_of: Vec<u32>,
    /// Per directed link: whether its own direction runs back to front
    /// through the stored points.
    reversed: Vec<bool>,
}

impl LinkGeometry {
    /// Build the artifact for `network`, taking each directed link's own
    /// polyline — from [`RoadNetwork::link_from`] to
    /// [`RoadNetwork::link_to`] — from `points_of`.
    ///
    /// `points_of` is keyed by the same external link id `network` was built
    /// from; a link absent from it, or one `network` dropped for its own
    /// reasons (Foundations §1's "an internal id is meaningful only with its
    /// fingerprint" cuts both ways), simply has no stored geometry —
    /// [`LinkGeometry::points`] reports `None` for it, which is a normal
    /// condition here, not a bug.
    ///
    /// A two-way street's opposite link is detected via
    /// [`RoadNetwork::is_reverse_of`] and made to share the one point run,
    /// **provided the two links' measured lengths agree to a micrometre** —
    /// see the module docs for why that guard exists. Every debug build also
    /// checks that a link's own
    /// points re-measure, via [`polyline_length_metres`], to its own stored
    /// [`RoadNetwork::link_length`] within the same tolerance: this is what
    /// keeps "length is never re-derived from geometry" honest rather than
    /// merely stated.
    #[must_use]
    pub fn build(network: &RoadNetwork, points_of: &HashMap<String, Vec<LonLat>>) -> Self {
        let link_count = network.link_count();
        let mut street_of = vec![NO_STREET; link_count as usize];
        let mut reversed = vec![false; link_count as usize];
        let mut offsets = vec![0u32];
        let mut points: Vec<LonLat> = Vec::new();

        for raw in 0..link_count {
            let link = LinkId::new(raw);
            if street_of[link.index()] != NO_STREET {
                continue;
            }
            let Some(external) = network.link_external_ids().external_of(link) else { continue };
            let Some(own_points) = points_of.get(external) else { continue };

            debug_assert!(
                (polyline_length_metres(own_points) - network.link_length(link).get()).abs()
                    < LENGTH_AGREEMENT_TOLERANCE_M,
                "stored geometry for {link:?} does not remeasure to its stored length"
            );

            #[allow(
                clippy::cast_possible_truncation,
                reason = "street count is bounded by link_count, which is u32"
            )]
            let street = (offsets.len() - 1) as u32;
            points.extend_from_slice(own_points);
            #[allow(
                clippy::cast_possible_truncation,
                reason = "total points are bounded by the source extract, far under u32::MAX"
            )]
            offsets.push(points.len() as u32);
            street_of[link.index()] = street;
            reversed[link.index()] = false;

            let to = network.link_to(link);
            let partner = network
                .out_links(to)
                .iter()
                .copied()
                .find(|&cand| network.is_reverse_of(link, cand));
            if let Some(partner) = partner {
                let agrees = (network.link_length(partner).get() - network.link_length(link).get())
                    .abs()
                    < LENGTH_AGREEMENT_TOLERANCE_M;
                if agrees && street_of[partner.index()] == NO_STREET {
                    street_of[partner.index()] = street;
                    reversed[partner.index()] = true;
                }
            }
        }

        Self { fingerprint: NetworkFingerprint::of(network), offsets, points, street_of, reversed }
    }

    /// Whether this artifact was built from `network`.
    ///
    /// A geometry file must never be read against a network it was not built
    /// with (S125); this is the check that catches it.
    #[must_use]
    pub fn matches(&self, network: &RoadNetwork) -> bool {
        self.fingerprint == NetworkFingerprint::of(network)
    }

    /// How many distinct streets are stored — undirected, so a two-way
    /// street with geometry counts once.
    #[must_use]
    pub fn street_count(&self) -> u32 {
        #[allow(
            clippy::cast_possible_truncation,
            reason = "offsets.len() is street_count + 1, both far under u32::MAX"
        )]
        let n = (self.offsets.len() - 1) as u32;
        n
    }

    /// Whether `link` has stored geometry.
    #[must_use]
    pub fn has_geometry(&self, link: LinkId) -> bool {
        self.street_of.get(link.index()).is_some_and(|&s| s != NO_STREET)
    }

    /// `link`'s polyline, from [`RoadNetwork::link_from`] to
    /// [`RoadNetwork::link_to`], or `None` if this link has no stored
    /// geometry.
    ///
    /// Reads back-to-front rather than allocating when `link` is the
    /// direction that does not own the stored run — that sharing is the
    /// point of storing one run per undirected street rather than one per
    /// directed link.
    pub fn points(&self, link: LinkId) -> Option<impl Iterator<Item = LonLat> + '_> {
        let &street = self.street_of.get(link.index())?;
        if street == NO_STREET {
            return None;
        }
        let start = self.offsets[street as usize] as usize;
        let end = self.offsets[street as usize + 1] as usize;
        let slice = &self.points[start..end];
        Some(if self.reversed[link.index()] {
            PointRun::Reversed(slice.iter().rev())
        } else {
            PointRun::Forward(slice.iter())
        })
    }

    /// Approximate bytes held, for the manifest's memory figures.
    #[must_use]
    pub fn bytes(&self) -> usize {
        self.offsets.len() * size_of::<u32>()
            + self.points.len() * size_of::<LonLat>()
            + self.street_of.len() * size_of::<u32>()
            + self.reversed.len()
    }
}

/// One link's points, read in its own direction.
///
/// The two variants exist because a forward and a reverse iterator over a
/// slice are different concrete types; this is the zero-allocation
/// alternative to collecting one of them into a `Vec` just to unify the type.
enum PointRun<'a> {
    Forward(std::slice::Iter<'a, LonLat>),
    Reversed(std::iter::Rev<std::slice::Iter<'a, LonLat>>),
}

impl Iterator for PointRun<'_> {
    type Item = LonLat;

    fn next(&mut self) -> Option<LonLat> {
        match self {
            PointRun::Forward(it) => it.next().copied(),
            PointRun::Reversed(it) => it.next().copied(),
        }
    }
}
