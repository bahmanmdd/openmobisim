//! Counter-based draws: the documented ChaCha8 keystream, purity, separation, and the
//! distributions of the draws at sampling error.

use std::collections::HashSet;

use openmobisim_types::rng::{DrawAddress, DrawBlock, RngKey, Stream, StreamRng};
use rand_chacha::ChaCha8Rng;
use rand_chacha::rand_core::{Rng, SeedableRng};
use rayon::prelude::*;

/// Upper 0.1 % points of the chi-square distribution (scipy.stats.chi2.ppf(0.999, df)).
const CHI2_999_DF19: f64 = 43.820;
const CHI2_999_DF24: f64 = 51.179;

fn choice(seed: u64) -> StreamRng {
    StreamRng::new(RngKey::new(seed), Stream::Choice)
}

#[test]
fn a_block_is_the_chacha8_keystream_of_the_documented_key() {
    // The external answer: the ChaCha8 generator of the rand_chacha library, keyed by hand with
    // the layout in the module documentation and read one word at a time.
    for (seed, stream, (a, b, c)) in [
        (0u64, Stream::Choice, (0u32, 0u32, 0u32)),
        (2_024, Stream::Choice, (412_002, 7, 19)),
        (u64::MAX, Stream::MsaReselection, (u32::MAX, 1, 0)),
    ] {
        let mut key = [0u8; 32];
        key[0..8].copy_from_slice(&seed.to_le_bytes());
        key[8..10].copy_from_slice(&(stream as u16).to_le_bytes());
        key[10..32].copy_from_slice(b"openmobisim-stream-rng");
        let mut reference = ChaCha8Rng::from_seed(key);
        reference.set_stream((u64::from(a) << 32) | u64::from(b));
        reference.set_word_pos(u128::from(c) * 16);
        let expected: Vec<u32> = (0..DrawBlock::LEN).map(|_| reference.next_u32()).collect();
        let block =
            StreamRng::new(RngKey::new(seed), stream).block(DrawAddress::from_triple(a, b, c));
        assert_eq!(block.words().to_vec(), expected, "seed {seed}, address ({a}, {b}, {c})");
    }
}

#[test]
fn a_draw_is_a_pure_function_of_its_key() {
    let addr = DrawAddress::from_triple(412_002, 7, 19);
    let first = choice(1_234).block(addr);
    assert_eq!(choice(1_234).block(addr), first, "a freshly built generator agrees");
    // Asked in parallel, in any order, at any thread count: the same bits.
    let rng = choice(1_234);
    let sequential: Vec<DrawBlock> =
        (0..2_000u32).map(|i| rng.block(DrawAddress::from_triple(i, 7, i % 5))).collect();
    for threads in [1, 8] {
        let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
        let parallel: Vec<DrawBlock> = pool.install(|| {
            (0..2_000u32)
                .into_par_iter()
                .rev()
                .map(|i| rng.block(DrawAddress::from_triple(i, 7, i % 5)))
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect()
        });
        assert_eq!(parallel, sequential, "at {threads} threads");
    }
}

#[test]
fn every_identity_component_and_the_seed_separate_draws() {
    let base = choice(7).block(DrawAddress::from_triple(10, 20, 30));
    for other in [
        choice(7).block(DrawAddress::from_triple(11, 20, 30)),
        choice(7).block(DrawAddress::from_triple(10, 21, 30)),
        choice(7).block(DrawAddress::from_triple(10, 20, 31)),
        choice(8).block(DrawAddress::from_triple(10, 20, 30)),
    ] {
        assert_ne!(other, base);
    }
}

#[test]
fn address_packing_is_injective_within_one_arity() {
    // No hashing in the address, so distinct identities give distinct addresses, always.
    let mut triples = HashSet::new();
    for a in 0u32..20 {
        for b in 0u32..20 {
            for c in 0u32..20 {
                assert!(triples.insert(DrawAddress::from_triple(a, b, c)), "({a}, {b}, {c})");
            }
        }
    }
    // The extremes of each component do not run into each other.
    assert_ne!(DrawAddress::from_pair(0, u32::MAX), DrawAddress::from_pair(1, 0));
    assert_ne!(DrawAddress::from_triple(0, 0, u32::MAX), DrawAddress::from_triple(0, 1, 0));
}

#[test]
fn a_pair_is_the_triple_with_a_zero_third_component() {
    // By the documented packing; each stream therefore keeps to one arity.
    assert_eq!(DrawAddress::from_pair(1, 2), DrawAddress::from_triple(1, 2, 0));
}

#[test]
fn adding_an_alternative_leaves_the_draws_of_the_others_unchanged() {
    // Keyed on the alternative's identity, not its position in a list.
    let rng = choice(555);
    let draw = |alt: u32| rng.block(DrawAddress::from_triple(100, 3, alt)).unit(0).to_bits();
    let before: Vec<u64> = [2u32, 5, 9].map(draw).to_vec();
    let _new = draw(7);
    assert_eq!([2u32, 5, 9].map(draw).to_vec(), before);
}

fn chi_square(counts: &[u32], expected: f64) -> f64 {
    counts.iter().map(|&c| (f64::from(c) - expected).powi(2) / expected).sum()
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "u is in [0, 1)")]
fn bin(u: f64, bins: u32) -> usize {
    (u * f64::from(bins)) as usize
}

#[test]
fn uniform_draws_pass_a_chi_square_test_in_every_stream() {
    // 10 000 addresses × 16 words = 160 000 draws in 20 equal bins: 19 degrees of freedom.
    for stream in Stream::ALL {
        let rng = StreamRng::new(RngKey::new(2_024), stream);
        let mut counts = [0u32; 20];
        for i in 0..10_000u32 {
            let block = rng.block(DrawAddress::from_pair(i, 0));
            for w in 0..DrawBlock::LEN {
                let u = block.unit(w);
                assert!((0.0..1.0).contains(&u));
                counts[bin(u, 20)] += 1;
            }
        }
        let x2 = chi_square(&counts, 160_000.0 / 20.0);
        assert!(x2 < CHI2_999_DF19, "{stream:?}: chi-square {x2} at 19 degrees of freedom");
    }
}

#[test]
fn neighbouring_draws_and_the_two_streams_are_independent() {
    // Pairs in a 5 × 5 grid of equal cells: 24 degrees of freedom under independence. Pairs
    // are (word 0, word 1) of one block, (word 0 at trip i, word 0 at trip i + 1), and (the
    // choice stream, the reselection stream) at the same address.
    let key = RngKey::new(99);
    let n = 50_000u32;
    let blocks = |stream| {
        let rng = StreamRng::new(key, stream);
        (0..=n).map(|i| rng.block(DrawAddress::from_pair(i, 0))).collect::<Vec<_>>()
    };
    let (c, r) = (blocks(Stream::Choice), blocks(Stream::MsaReselection));
    let at = |i: u32| i as usize;
    let pairs: [&dyn Fn(u32) -> (f64, f64); 3] = [
        &|i| (c[at(i)].unit(0), c[at(i)].unit(1)),
        &|i| (c[at(i)].unit(0), c[at(i + 1)].unit(0)),
        &|i| (c[at(i)].unit(0), r[at(i)].unit(0)),
    ];
    for (k, pair) in pairs.iter().enumerate() {
        let mut counts = [0u32; 25];
        for i in 0..n {
            let (u, v) = pair(i);
            counts[5 * bin(u, 5) + bin(v, 5)] += 1;
        }
        let x2 = chi_square(&counts, f64::from(n) / 25.0);
        assert!(x2 < CHI2_999_DF24, "pair {k}: chi-square {x2} at 24 degrees of freedom");
    }
}

#[test]
fn gumbel_draws_have_mean_euler_gamma_and_variance_pi_squared_over_six() {
    // Theory: mean γ = 0.577 215 66…, variance π²/6, excess kurtosis 12/5. With n = 160 000,
    // the standard error of the mean is √(π²/6 / n) = 0.0032 and that of the variance is
    // √((12/5 + 2) (π²/6)² / n) = 0.0086; the tolerance is four standard errors.
    let rng = choice(31);
    let xs: Vec<f64> = (0..10_000u32)
        .flat_map(|i| {
            let b = rng.block(DrawAddress::from_pair(i, 0));
            (0..DrawBlock::LEN).map(move |w| b.gumbel(w))
        })
        .collect();
    assert!(xs.iter().all(|x| x.is_finite()));
    let n = 160_000.0;
    assert_eq!(xs.len(), 160_000);
    let mean = xs.iter().sum::<f64>() / n;
    let var = xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0);
    let (gamma, pi2_6) = (0.577_215_664_901_532_9, std::f64::consts::PI.powi(2) / 6.0);
    assert!((mean - gamma).abs() < 4.0 * (pi2_6 / n).sqrt(), "mean {mean}");
    assert!((var - pi2_6).abs() < 4.0 * (4.4 * pi2_6 * pi2_6 / n).sqrt(), "variance {var}");
}

#[test]
fn bernoulli_draws_are_true_with_their_probability() {
    // Share of 160 000 draws at p = 0.3: standard error √(0.3 × 0.7 / n) = 0.0011.
    let rng = StreamRng::new(RngKey::new(5), Stream::MsaReselection);
    let mut hits = 0u32;
    for i in 0..10_000u32 {
        let b = rng.block(DrawAddress::from_pair(i, 1));
        hits += (0..DrawBlock::LEN).map(|w| u32::from(b.bernoulli(w, 0.3))).sum::<u32>();
    }
    let n = 160_000.0;
    let share = f64::from(hits) / n;
    assert!((share - 0.3).abs() < 4.0 * (0.21f64 / n).sqrt(), "share {share}");
}

#[test]
fn open_unit_draws_lie_strictly_inside_the_unit_interval() {
    let rng = choice(3);
    for i in 0..5_000u32 {
        let b = rng.block(DrawAddress::from_pair(i, 0));
        for w in 0..DrawBlock::LEN {
            let o = b.open_unit(w);
            assert!(o > 0.0 && o < 1.0, "{o}");
            // By hand: (word + ½) · 2⁻³² sits half a step above word · 2⁻³².
            assert_eq!((o - b.unit(w)).to_bits(), (0.5f64 / 4_294_967_296.0).to_bits());
            assert_eq!(b.word(w), b.words()[w]);
        }
    }
}
