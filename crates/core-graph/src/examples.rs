//! `manhattan_grid` — the shared synthetic-network fixture (S105).
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
