//! Foundational types for openmobisim.
//!
//! This crate is the one every other crate depends on, and it depends on
//! almost nothing itself (Foundations §10). It holds the decisions that are
//! expensive or impossible to change once code exists:
//!
//! | Module | Foundations § | What it fixes |
//! |---|---|---|
//! | [`ids`] | §1 | Dense `u32` identity spaces, one per entity kind, with a null sentinel |
//! | [`time`] | §2 | The `u32` seconds clock, the loading step grid, the event-queue total order |
//! | [`units`] | §9 | SI, per-second internal units as zero-cost newtypes |
//! | [`reduce`] | §3 | Fixed-order parallel reductions — the determinism guarantee |
//! | [`rng`] | §4 | Counter-based ChaCha8 streams addressed by entity identity |
//! | [`diagnostics`] | §6 | Aggregated run diagnostics that cost nothing until something is recorded |
//! | [`registry`] | §7 | Plugin names resolved once, at build time, never in a loop |
//!
//! # The three rules a reader should take away
//!
//! 1. **Never `par_iter().sum()`.** Use [`reduce`]. Rayon's reduction order is
//!    not deterministic, and the resulting breakage is invisible: it silently
//!    destroys common random numbers and makes the convergence report
//!    unreadable. See [`reduce`] for the wrapper and `CONTRIBUTING.md` for why.
//! 2. **Never `Option<SomeId>` in a stored array.** Every id type has a
//!    [`NULL`](ids::EntityId::NULL) sentinel; `Option<u32>` is eight bytes
//!    where the sentinel is four, which doubles every optional index array.
//! 3. **Never convert units inside a loop.** Capacities arrive as veh/h and
//!    are converted exactly once, at the scenario build boundary, by the
//!    constructors in [`units`].
//!
//! # Determinism, precisely
//!
//! Given the same seed, the same binary and the same platform, two runs
//! produce bit-identical output. This is *not* a cross-platform guarantee:
//! platform `libm` `exp`/`ln` differ in their last bits, and the exact logit
//! and the Gumbel draws use them.

#![cfg_attr(docsrs, feature(doc_cfg))]

// openmobisim indexes arrays with `u32` ids widened to `usize`. A pointer width
// below 32 bits would make `id.index()` lossy, so refuse to compile there.
const _: () = assert!(
    usize::BITS >= 32,
    "openmobisim requires a target with a 32-bit or wider pointer width"
);

pub mod diagnostics;
pub mod ids;
pub mod reduce;
pub mod registry;
pub mod rng;
pub mod time;
pub mod units;

pub use diagnostics::{Category, DiagCode, DiagRow, Diagnostics, ElementRef, Severity};
pub use ids::{
    AccessPointId, EntityId, EntityKind, ExternalIdTable, ExternalIdTableBuilder, HubId, LayerId,
    LinkId, MAX_ID, NULL_ID, NodeId, NullableId, PathId, ResourceId, TransitRunId, TravellerId,
    TripId, TurnId, UserClassId, VehicleId, ZoneId,
};
pub use reduce::{
    REDUCTION_CHUNK, fixed_order_chunk_reduce, fixed_order_map, fixed_order_map_sum,
    fixed_order_sum,
};
pub use registry::{Registry, RegistryError};
pub use rng::{DrawAddress, DrawBlock, RNG_SCHEME_VERSION, RngKey, Stream, StreamRng};
pub use time::{EventKey, FastDivU32, Second, StepGrid, StepIndex};
pub use units::{Cost, Density, Duration, Flow, Metres, Pcu, Speed};

/// Layout and build-semantics version for every cached artifact.
///
/// Part of the cache fingerprint (Foundations §8):
///
/// ```text
/// fingerprint = BLAKE3(input_bytes_hash, build_params, CODE_VERSION, defaults_version)
/// ```
///
/// **Bump this whenever the on-disk layout or the build semantics of any
/// cached artifact change.** Artifacts are never partially valid: a mismatch
/// means rebuild, never repair. Internal layouts are free to change behind a
/// bump — the public semver contracts are the plugin API and the scenario
/// schema, not the artifact format.
pub const CODE_VERSION: u32 = 1;

/// The crate version, as reported in `manifest.json`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
