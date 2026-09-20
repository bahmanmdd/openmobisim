//! `manhattan_grid` and `toy_network` — the shared synthetic-network fixtures
//! (S105, S161).
//!
//! A grid is the right shape for a fixture that has to double as both a
//! correctness check and a demo: every count — nodes, links, turns — is
//! known in closed form, so a test can assert what the answer *should* be
//! rather than only what the code currently produces, and a reader can
//! eyeball a small one on a map. This is the same builder the notebook
//! ladder's `ms.examples.manhattan_grid(n, block_metres, signals)` wraps
//! (`06_INTERFACE_V0.md` §3, N3) and that `core-graph`'s own test suite
//! already used privately before this module existed to promote it into
//! (roadmap Phase 1 step 8).
//!
//! Coordinates use a flat local approximation (metres per degree at a fixed
//! reference latitude), not the ellipsoidal projection the rest of this
//! crate is built on (`geometry::Projection`) — deliberately: a synthetic
//! grid represents nowhere in particular, so a uniform block size in metres
//! matters more here than geodesic precision, which this fixture was never
//! claiming.

use openmobisim_core_types::diagnostics::Diagnostics;

use crate::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
use crate::geometry::LonLat;
use crate::network::{LinkSpec, RoadNetwork, RoadNetworkBuilder};

/// The latitude the grid is built at. Arbitrary but fixed, so that two
/// grids built with the same parameters are identical, and mid-latitude so
/// the longitude/latitude metre-per-degree ratio is representative of
/// nowhere in particular.
const REFERENCE_LATITUDE_DEG: f64 = 45.7;

/// Metres per degree of latitude — the same constant everywhere on the
/// ellipsoid to the precision a synthetic grid needs.
const METRES_PER_DEGREE_LAT: f64 = 110_574.0;

/// Metres per degree of longitude at [`REFERENCE_LATITUDE_DEG`] — shrinks
/// toward the poles; fixed here because the grid is built at one latitude.
fn metres_per_degree_lon() -> f64 {
    111_320.0 * REFERENCE_LATITUDE_DEG.to_radians().cos()
}

/// The node external id at grid position `(row, column)` — exposed so a
/// caller can build demand between named positions without duplicating the
/// naming convention.
#[must_use]
pub fn node_name(row: u32, col: u32) -> String {
    format!("n_{row:03}_{col:03}")
}

/// How many directed links an `n × n` grid has: two directions on each of
/// the `n(n − 1)` horizontal and `n(n − 1)` vertical edges.
#[must_use]
pub fn link_count(n: u32) -> u32 {
    4 * n * (n - 1)
}

/// An `n × n` grid of nodes, `block_metres` apart, with bidirectional links
/// along every edge.
///
/// `signals` marks every interior node (not on the grid's outer ring) as
/// signal-controlled, which is how the control-delay and turn-capacity
/// paths get exercised — matching `06_INTERFACE_V0.md`'s
/// `ms.examples.manhattan_grid(n, block_metres, signals)`.
///
/// # Panics
///
/// Panics if `n < 2` — a grid needs at least one edge to be a network at
/// all, and this is a bug in the caller, not a data condition.
#[must_use]
pub fn manhattan_grid(n: u32, block_metres: f64, signals: bool) -> (RoadNetwork, Diagnostics) {
    assert!(n >= 2, "a manhattan_grid needs at least 2×2 nodes to have an edge");

    let d_lat = block_metres / METRES_PER_DEGREE_LAT;
    let d_lon = block_metres / metres_per_degree_lon();

    let mut builder = RoadNetworkBuilder::new();
    for r in 0..n {
        for c in 0..n {
            builder.add_node(
                node_name(r, c),
                LonLat::new(
                    4.8 + f64::from(c) * d_lon,
                    REFERENCE_LATITUDE_DEG + f64::from(r) * d_lat,
                ),
            );
            let interior = r > 0 && r + 1 < n && c > 0 && c + 1 < n;
            if signals && interior {
                builder.mark_signalised(node_name(r, c));
            }
        }
    }

    let spec = LinkSpec::new(RoadClass::Secondary);
    let add_pair = |b: &mut RoadNetworkBuilder, a: String, z: String| {
        b.add_link(format!("l_{a}__{z}"), a.clone(), z.clone(), spec);
        b.add_link(format!("l_{z}__{a}"), z, a, spec);
    };
    for r in 0..n {
        for c in 0..n {
            if c + 1 < n {
                add_pair(&mut builder, node_name(r, c), node_name(r, c + 1));
            }
            if r + 1 < n {
                add_pair(&mut builder, node_name(r, c), node_name(r + 1, c));
            }
        }
    }

    let mut diagnostics = Diagnostics::new();
    let network = builder
        .build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut diagnostics)
        .expect("a manhattan_grid is always projectable: coordinates are built, not read");
    (network, diagnostics)
}

/// The toy network's road part (I-m, S161): a small, one-way network on which
/// every number the loading produces can be checked by hand.
///
/// Sixteen nodes and sixteen links, each a mechanism in a few lines. Streets
/// are `residential` unless stated, `length_m` is set exactly (so hand
/// derivations are exact), and node positions agree with those lengths.
///
/// ```text
///  N1 ─s1 300─┐                                         ┌─ a4 300 ─▶ D1
///             ▼                                         │
///  W ─a1 300─▶ S* ─a2 200─▶ M ─a3 400─▶ X0 ▷c1▷c2▷c3▷ X3 ┤   c1–c3: 4 m each
///                           ▲                           └─ a5 60 (service) ─▶ R1
///  N2 ═m1 300, 2 lanes══════┘                                                  │
///                                                                      ring r1–r4
///  N3 ─n3 100─▶ R4 (top right); the ring runs R1→R2→R3→R4→R1               (4 × 30 m,
///  counter-clockwise; `e2` (300 m) leaves it at R2 towards D2.        junction=roundabout)
/// ```
///
/// - `S` is signalised: `a1` and `s1` are its approaches, both leading to
///   `a2`.
/// - `M` merges `a2` with the two-lane `m1` into `a3`.
/// - `c1`–`c3` are shorter than a vehicle (4 m against 7.14 m of jam spacing):
///   the sub-vehicle-length case.
/// - `X3` diverges into `a4` and the slower service road `a5`, which enters
///   the roundabout ring at `R1`; `n3` enters it at `R4`. The ring's four
///   links carry the `junction=roundabout` flag.
/// - `D2` is reserved as the site of a hub for the multimodal part.
///
/// Link and node external ids are the names above (`"a1"`, `"S"`, …).
///
/// # Panics
///
/// Never in practice: coordinates are built, not read.
#[must_use]
pub fn toy_network() -> (RoadNetwork, Diagnostics) {
    // (name, east metres, north metres)
    const NODES: [(&str, f64, f64); 16] = [
        ("W", 0.0, 0.0),
        ("N1", 300.0, 300.0),
        ("S", 300.0, 0.0),
        ("N2", 500.0, 300.0),
        ("M", 500.0, 0.0),
        ("X0", 900.0, 0.0),
        ("X1", 904.0, 0.0),
        ("X2", 908.0, 0.0),
        ("X3", 912.0, 0.0),
        ("D1", 1212.0, 0.0),
        ("R1", 912.0, -60.0),
        ("R2", 912.0, -90.0),
        ("R3", 942.0, -90.0),
        ("R4", 942.0, -60.0),
        ("N3", 1042.0, -60.0),
        ("D2", 912.0, -390.0),
    ];
    // (name, from, to, length in metres, class, lanes, roundabout)
    type LinkRow = (&'static str, &'static str, &'static str, f64, RoadClass, Option<u8>, bool);
    const LINKS: [LinkRow; 16] = [
        ("a1", "W", "S", 300.0, RoadClass::Residential, None, false),
        ("s1", "N1", "S", 300.0, RoadClass::Residential, None, false),
        ("a2", "S", "M", 200.0, RoadClass::Residential, None, false),
        ("m1", "N2", "M", 300.0, RoadClass::Residential, Some(2), false),
        ("a3", "M", "X0", 400.0, RoadClass::Residential, None, false),
        ("c1", "X0", "X1", 4.0, RoadClass::Residential, None, false),
        ("c2", "X1", "X2", 4.0, RoadClass::Residential, None, false),
        ("c3", "X2", "X3", 4.0, RoadClass::Residential, None, false),
        ("a4", "X3", "D1", 300.0, RoadClass::Residential, None, false),
        ("a5", "X3", "R1", 60.0, RoadClass::Service, None, false),
        ("r1", "R1", "R2", 30.0, RoadClass::Residential, None, true),
        ("r2", "R2", "R3", 30.0, RoadClass::Residential, None, true),
        ("r3", "R3", "R4", 30.0, RoadClass::Residential, None, true),
        ("r4", "R4", "R1", 30.0, RoadClass::Residential, None, true),
        ("n3", "N3", "R4", 100.0, RoadClass::Residential, None, false),
        ("e2", "R2", "D2", 300.0, RoadClass::Residential, None, false),
    ];

    let mut builder = RoadNetworkBuilder::new();
    for (name, east, north) in NODES {
        builder.add_node(
            name,
            LonLat::new(
                4.8 + east / metres_per_degree_lon(),
                REFERENCE_LATITUDE_DEG + north / METRES_PER_DEGREE_LAT,
            ),
        );
    }
    builder.mark_signalised("S");
    for (name, from, to, length_m, class, lanes, roundabout) in LINKS {
        let mut spec = LinkSpec::new(class);
        spec.length_m = Some(length_m);
        spec.lanes = lanes;
        spec.roundabout = roundabout;
        builder.add_link(name, from, to, spec);
    }

    let mut diagnostics = Diagnostics::new();
    let network = builder
        .build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut diagnostics)
        .expect("the toy network is always projectable: coordinates are built, not read");
    (network, diagnostics)
}
