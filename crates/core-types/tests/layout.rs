//! Memory layout is a design commitment, not an implementation detail.
//!
//! Every type below sits in an array with one entry per link, per traveller or
//! per step. A type that silently grows from four bytes to eight doubles a
//! structure that a Berlin-scale run holds tens of millions of, and nothing
//! else in the test suite would notice. These assertions are deliberately
//! brittle: if one fails, the question is whether the growth was intended, not
//! how to make the test pass.

use std::mem::{align_of, size_of};

use openmobisim_core_types::diagnostics::{Category, DiagKey, DiagRow, ElementRef, Severity};
use openmobisim_core_types::ids::{EntityKind, LinkId, NodeId, TravellerId, TripId};
use openmobisim_core_types::rng::{DrawAddress, DrawBlock, RngKey};
use openmobisim_core_types::time::{EventKey, FastDivU32, Second, StepGrid, StepIndex};
use openmobisim_core_types::units::{Cost, Density, Duration, Flow, Metres, Pcu, Speed};

#[test]
fn ids_are_four_bytes() {
    assert_eq!(size_of::<NodeId>(), 4, "an id must be exactly a u32");
    assert_eq!(size_of::<LinkId>(), 4);
    assert_eq!(size_of::<TravellerId>(), 4);
    assert_eq!(size_of::<TripId>(), 4);
    assert_eq!(align_of::<LinkId>(), 4);
}

#[test]
fn optional_ids_would_cost_double() {
    // This is the assertion behind "never `Option<Id>` in a stored array".
    // If this ever stops being true, the sentinel has lost its reason to exist.
    assert_eq!(size_of::<Option<LinkId>>(), 8);
    assert_eq!(size_of::<LinkId>(), 4);
}

#[test]
fn entity_kind_is_one_byte() {
    assert_eq!(size_of::<EntityKind>(), 1);
}

#[test]
fn clock_types_are_compact() {
    assert_eq!(size_of::<Second>(), 4, "the clock is u32 seconds");
    assert_eq!(size_of::<StepIndex>(), 4);
    // (second, kind, entity) with u32 alignment.
    assert_eq!(size_of::<EventKey>(), 12);
    assert_eq!(size_of::<FastDivU32>(), 16);
    // The grid is shared per run, so its size matters far less; assert it
    // stays small enough to pass by value.
    assert!(size_of::<StepGrid>() <= 32, "StepGrid grew to {}", size_of::<StepGrid>());
}

#[test]
fn units_are_transparent_over_f64() {
    for (name, size) in [
        ("Metres", size_of::<Metres>()),
        ("Speed", size_of::<Speed>()),
        ("Duration", size_of::<Duration>()),
        ("Flow", size_of::<Flow>()),
        ("Density", size_of::<Density>()),
        ("Pcu", size_of::<Pcu>()),
        ("Cost", size_of::<Cost>()),
    ] {
        assert_eq!(size, 8, "{name} must be exactly an f64");
    }
    // The claim that `Vec<Metres>` has the layout of `Vec<f64>`.
    assert_eq!(size_of::<[Metres; 16]>(), size_of::<[f64; 16]>());
}

#[test]
fn rng_types_are_the_expected_size() {
    assert_eq!(size_of::<DrawBlock>(), 64, "one ChaCha block is 64 bytes");
    assert_eq!(size_of::<DrawAddress>(), 16, "64 bits of stream, 64 of block counter");
    assert_eq!(size_of::<RngKey>(), 24, "seed, design, replication and the CRN flag");
}

#[test]
fn diagnostic_rows_stay_small() {
    assert_eq!(size_of::<Category>(), 1);
    assert_eq!(size_of::<Severity>(), 1);
    assert_eq!(size_of::<ElementRef>(), 8, "one byte of kind, four of id, padded");
    // A map entry per reported element; keep an eye on this one.
    assert!(size_of::<DiagKey>() <= 32, "DiagKey grew to {}", size_of::<DiagKey>());
    assert!(size_of::<DiagRow>() <= 40, "DiagRow grew to {}", size_of::<DiagRow>());
}
