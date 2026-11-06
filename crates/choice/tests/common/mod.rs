//! Fixtures shared by the choice tests: one-class coefficients, travellers, rows, sampled
//! counts and Pearson's chi-square statistic.

#![allow(dead_code, reason = "each test binary uses a different part of this module")]

use openmobisim_choice::cost::{Coefficients, Row, Traveller};
use openmobisim_choice::model::Choice;
use openmobisim_types::ids::{EntityId, TripId, UserClassId};
use openmobisim_types::mode::Mode;

/// The 0.999 quantile of the chi-square distribution with 5 degrees of freedom (20.515, from
/// any table of the distribution): a correct sampler exceeds it once in a thousand seeds.
pub const CHI_SQUARE_999_DF5: f64 = 20.52;

/// The 0.999 quantile of the chi-square distribution with 2 degrees of freedom, `−2 ln 0.001`.
pub const CHI_SQUARE_999_DF2: f64 = 13.82;

/// Coefficients over `columns` for a run of one user class (class 0) with `betas`.
#[must_use]
pub fn one_class(columns: &[&str], betas: &[f64]) -> Coefficients {
    Coefficients::new(columns, 1, &[(UserClassId::new(0), betas.to_vec())]).expect("valid")
}

/// Trip `trip` of class `class` at iteration `iteration`.
#[must_use]
pub fn traveller(trip: u32, class: u32, iteration: u32) -> Traveller {
    Traveller { trip: TripId::new(trip), class: UserClassId::new(class), iteration }
}

/// One row per attribute vector, of `mode`, row `j` in nest `nests[j]`.
#[must_use]
pub fn rows<'a>(attributes: &'a [Vec<f64>], mode: Mode, nests: &[u32]) -> Vec<Row<'a>> {
    assert_eq!(attributes.len(), nests.len());
    attributes.iter().zip(nests).map(|(a, &nest)| Row { attributes: a, mode, nest }).collect()
}

/// How often each row is chosen by trips `0..draws` of class 0 at `iteration`, the trips
/// spread over four threads (each choice depends on its trip alone, so the counts do not
/// depend on the spread).
#[must_use]
pub fn counts(
    model: &(dyn Choice + Sync),
    rows: &[Row<'_>],
    draws: u32,
    iteration: u32,
) -> Vec<u64> {
    const THREADS: u32 = 4;
    let count = |part: u32| {
        let mut counts = vec![0; rows.len()];
        for trip in (part..draws).step_by(THREADS as usize) {
            counts[model.choose(&traveller(trip, 0, iteration), rows).expect("a choice")] += 1;
        }
        counts
    };
    std::thread::scope(|s| {
        let parts: Vec<_> = (0..THREADS).map(|part| s.spawn(move || count(part))).collect();
        let mut total = vec![0; rows.len()];
        for part in parts {
            for (t, c) in total.iter_mut().zip(part.join().expect("a counting thread")) {
                *t += c;
            }
        }
        total
    })
}

/// Pearson's statistic `Σ (observed − expected)² / expected` of `counts` against the
/// probabilities `p`.
#[must_use]
pub fn chi_square(counts: &[u64], p: &[f64]) -> f64 {
    #[allow(clippy::cast_precision_loss, reason = "counts far below 2^53")]
    let n = counts.iter().sum::<u64>() as f64;
    #[allow(clippy::cast_precision_loss, reason = "counts far below 2^53")]
    let term = |(&c, &p): (&u64, &f64)| (c as f64 - n * p).powi(2) / (n * p);
    counts.iter().zip(p).map(term).sum()
}

/// The largest absolute difference between two vectors of the same length.
#[must_use]
pub fn largest_gap(a: &[f64], b: &[f64]) -> f64 {
    assert_eq!(a.len(), b.len());
    a.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0.0, f64::max)
}
