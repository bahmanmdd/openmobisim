//! Prints a fingerprint of everything in `core-types` that must be
//! reproducible.
//!
//! CI runs this twice and compares the two outputs byte for byte. That is the
//! run-twice bit-identity gate in the form it can take before there is a
//! `Run` to execute: it exercises the reductions, the counter-based draws, the
//! step grid and the diagnostics ordering, which between them cover every
//! mechanism in this crate whose output is allowed to depend on the seed and
//! on nothing else.
//!
//! When `core-sim` lands, the gate becomes a comparison of two `kpis.parquet`
//! files and this probe stays as the cheap, fast version of it.
//!
//! Run it yourself with:
//!
//! ```text
//! cargo run -p openmobisim-core-types --release --example determinism_probe
//! ```
//!
//! Floats are printed as raw bits, because the property under test is
//! bit-identity and a decimal rendering would hide exactly the difference the
//! gate exists to catch.

use openmobisim_core_types::diagnostics::{
    Category, DiagKey, Diagnostics, ElementRef, Severity, codes,
};
use openmobisim_core_types::ids::{EntityId, ExternalIdTableBuilder, LinkId};
use openmobisim_core_types::reduce::{fixed_order_map_sum, fixed_order_sum, is_parallel_build};
use openmobisim_core_types::rng::{DrawAddress, RngKey, Stream, StreamRng};
use openmobisim_core_types::time::{Second, StepGrid};
use openmobisim_core_types::{CODE_VERSION, RNG_SCHEME_VERSION, VERSION};

/// Magnitudes several orders apart, so that a change in association order
/// shows up in the low bits rather than hiding below the rounding.
fn adversarial_values(n: u32) -> Vec<f64> {
    (0..n)
        .map(|i| {
            let x = f64::from(i);
            match i % 4 {
                0 => 1e12 + x,
                1 => 1e-12 * (x + 1.0),
                2 => -1e12 + x,
                _ => 1.0 / (x + 1.0),
            }
        })
        .collect()
}

fn main() {
    println!("version               {VERSION}");
    println!("code_version          {CODE_VERSION}");
    println!("rng_scheme_version    {RNG_SCHEME_VERSION}");
    // Printed for the reader, and deliberately *not* expected to change any
    // line below it: the parallel and sequential builds must agree.
    println!("parallel_build        {}", is_parallel_build());

    // --- Reductions --------------------------------------------------------
    let xs = adversarial_values(1_000_000);
    println!("sum                   {:016x}", fixed_order_sum(&xs).to_bits());
    println!(
        "map_sum               {:016x}",
        fixed_order_map_sum(&xs, |&x| x * 1.000_001).to_bits()
    );

    // --- The step grid -----------------------------------------------------
    let grid = StepGrid::new(Second::from_hours(7), 300, 4 * 3600).unwrap();
    let mut step_digest = 0u64;
    for t in (0..4 * 3600).step_by(97) {
        let second = Second(grid.origin().get() + t);
        step_digest = step_digest
            .wrapping_mul(0x0100_0000_01b3)
            .wrapping_add(u64::from(grid.step_of(second).get()));
        step_digest = step_digest
            .wrapping_mul(0x0100_0000_01b3)
            .wrapping_add(grid.fraction_into_step(second).to_bits());
    }
    println!("step_grid_digest      {step_digest:016x}");

    // --- Random draws ------------------------------------------------------
    for stream in Stream::ALL {
        let rng = StreamRng::new(RngKey::new(20_260_912, 3, 1), stream);
        let mut digest = 0u64;
        for i in 0..5_000u32 {
            let block = rng.block(DrawAddress::from_quad(i, i / 3, 2, i % 7));
            digest = digest.wrapping_mul(0x0100_0000_01b3).wrapping_add(u64::from(block.word(0)));
            digest = digest.wrapping_mul(0x0100_0000_01b3).wrapping_add(block.gumbel(1).to_bits());
        }
        println!("draws[{:<21}] {digest:016x}", stream.as_str());
    }

    // --- Identity assignment ----------------------------------------------
    let mut builder = ExternalIdTableBuilder::new();
    for i in (0..20_000u32).rev() {
        builder.insert(format!("way/{:08}", i * 7 % 20_000));
    }
    let table = builder.build();
    let mut id_digest = 0u64;
    for (i, external) in table.iter().enumerate() {
        id_digest =
            id_digest.wrapping_mul(0x0100_0000_01b3).wrapping_add(external.len() as u64 ^ i as u64);
    }
    println!("external_ids          {} entries, digest {id_digest:016x}", table.len());

    // --- Diagnostics ordering ---------------------------------------------
    let mut diag = Diagnostics::new();
    for i in 0..50_000u32 {
        diag.record(DiagKey::new(
            if i % 3 == 0 { Category::Modelling } else { Category::DataQuality },
            if i % 5 == 0 { codes::NO_FEASIBLE_PATH } else { codes::TRIP_TRUNCATED },
            if i % 7 == 0 { Severity::Warning } else { Severity::Info },
            ElementRef::of(LinkId::new(i % 9_000)),
        ));
    }
    let mut diag_digest = 0u64;
    for row in diag.rows() {
        diag_digest = diag_digest.wrapping_mul(0x0100_0000_01b3).wrapping_add(row.count);
        diag_digest = diag_digest
            .wrapping_mul(0x0100_0000_01b3)
            .wrapping_add(u64::from(row.key.element.id().unwrap_or(u32::MAX)));
    }
    println!("diagnostics           {} rows, digest {diag_digest:016x}", diag.rows().len());
    println!("diagnostics_total     {}", diag.total());
}
