//! Shared types: identifiers, the clock in seconds, metric units, the modes of travel,
//! fixed-order reductions and counter-based random streams. Every other crate depends on this one; it depends on
//! `rand_chacha` and `rayon` only.
//!
//! | Module | What it holds |
//! |---|---|
//! | [`ids`] | dense `u32` identifiers, one kind per entity, with a null sentinel; the table from external ids |
//! | [`mode`] | the four modes of travel (car, bike, walk, transit), peers that nothing ranks; sets of them |
//! | [`time`] | the `u32` clock in seconds, half-open time steps over a run's window, the total order of events |
//! | [`units`] | metres, metres per second, seconds, vehicles per second and cost, as zero-cost newtypes |
//! | [`reduce`] | sums and reductions in a fixed order, identical at any number of threads |
//! | [`rng`] | ChaCha8 draws addressed by seed, stream and entity identity |
//! | [`fingerprint`] | the 64-bit FNV-1a hash of what a network, a timetable or a demand was built from |
//! | [`repairs`] | what a reader set right instead of refusing: per kind its count and first ids |
//!
//! Units are metric throughout; vehicles per hour and kilometres per hour are converted once, as
//! a value is read. Bikes share every type with cars and walkers: there is no passenger-car
//! unit, and a bike's speed is a free-flow speed.
//!
//! Determinism: the same seed and the same binary on the same platform give bit-identical
//! results, at any number of threads. The Gumbel draws' logarithm is `libm`'s (pure Rust), not
//! the platform's maths library, whose last bits differ from one platform to another. Across
//! platforms, as measured: Delft's default run (test 10) gives byte-identical trips on macOS
//! (Apple silicon) and on Linux (x86_64); Windows is not measured.

// Identifiers are `u32` widened to `usize`; a narrower pointer would make that lossy.
const _: () = assert!(usize::BITS >= 32, "openmobisim needs a pointer width of at least 32 bits");

pub mod fingerprint;
pub mod ids;
pub mod mode;
pub mod reduce;
pub mod repairs;
pub mod rng;
pub mod time;
pub mod units;
