//! Counter-based draws: purity, separation, and common random numbers.

// Exact float comparison is the assertion here, not an oversight: bit-identical
// results are precisely the property under test.
#![allow(clippy::float_cmp, reason = "bit-exact results are the property under test")]

use std::collections::HashSet;

use openmobisim_core_types::rng::{DrawAddress, DrawBlock, RngKey, Stream, StreamRng};
use proptest::prelude::*;

fn choice_rng(seed: u64) -> StreamRng {
    StreamRng::new(RngKey::from_seed(seed), Stream::Choice)
}

#[test]
fn a_draw_is_a_pure_function_of_its_key() {
    let rng = choice_rng(1234);
    let addr = DrawAddress::from_quad(412_002, 3, 7, 19);
    let first = rng.block(addr);
    for _ in 0..50 {
        assert_eq!(rng.block(addr), first);
    }
    // And a freshly built generator agrees with the old one.
    assert_eq!(choice_rng(1234).block(addr), first);
}

#[test]
fn every_identity_component_separates_draws() {
    let rng = choice_rng(7);
    let base = DrawAddress::from_quad(10, 20, 30, 40);
    let variants = [
        DrawAddress::from_quad(11, 20, 30, 40),
        DrawAddress::from_quad(10, 21, 30, 40),
        DrawAddress::from_quad(10, 20, 31, 40),
        DrawAddress::from_quad(10, 20, 30, 41),
    ];
    let b = rng.block(base);
    for v in variants {
        assert_ne!(rng.block(v), b, "changing one component left the draw unchanged");
    }
}

#[test]
fn address_packing_is_injective_within_one_arity() {
    // No hashing anywhere in the address, so distinct identities must give
    // distinct addresses — not merely usually-distinct ones.
    let mut seen = HashSet::new();
    for a in 0u32..12 {
        for b in 0u32..12 {
            for c in 0u32..6 {
                for d in 0u32..6 {
                    assert!(
                        seen.insert(DrawAddress::from_quad(a, b, c, d)),
                        "address collision at ({a}, {b}, {c}, {d})"
                    );
                }
            }
        }
    }

    let mut pairs = HashSet::new();
    for a in 0u32..40 {
        for b in 0u32..40 {
            assert!(pairs.insert(DrawAddress::from_pair(a, b)), "pair collision at ({a}, {b})");
        }
    }
}

#[test]
fn arities_are_prefixes_of_each_other_by_design() {
    // 128 bits hold exactly four u32s, with no room for an arity tag. This
    // test pins that consequence so it is a documented property rather than a
    // surprise: a stream must use one arity throughout, and `DrawAddress`
    // says which arity each stream uses.
    assert_eq!(DrawAddress::from_pair(1, 2), DrawAddress::from_triple(1, 2, 0));
    assert_eq!(DrawAddress::from_triple(1, 2, 3), DrawAddress::from_quad(1, 2, 0, 3));
    // Different streams never collide even at the same address, because the
    // stream is part of the key rather than of the address.
    let key = RngKey::from_seed(1);
    let addr = DrawAddress::from_pair(1, 2);
    assert_ne!(
        StreamRng::new(key, Stream::DepartureTime).block(addr),
        StreamRng::new(key, Stream::MsaReselection).block(addr)
    );
}

#[test]
fn streams_are_independent() {
    let key = RngKey::from_seed(99);
    let addr = DrawAddress::from_pair(5, 5);
    let mut seen = HashSet::new();
    for stream in Stream::ALL {
        let block = StreamRng::new(key, stream).block(addr);
        assert!(seen.insert(block.words().to_vec()), "{stream} collided with another stream");
    }
}

#[test]
fn designs_share_random_numbers_by_default() {
    // The property that makes design comparison cheap: two designs see the
    // same draws, so a KPI difference is the design, not the noise.
    let addr = DrawAddress::from_quad(1, 0, 0, 5);
    let a = StreamRng::new(RngKey::new(42, 0, 0), Stream::Choice);
    let b = StreamRng::new(RngKey::new(42, 9_999, 0), Stream::Choice);
    assert_eq!(a.block(addr), b.block(addr));
}

#[test]
fn designs_can_be_given_independent_randomness_on_request() {
    let addr = DrawAddress::from_quad(1, 0, 0, 5);
    let shared = StreamRng::new(RngKey::new(42, 9, 0), Stream::Choice);
    let independent =
        StreamRng::new(RngKey::new(42, 9, 0).independent_per_design(), Stream::Choice);
    assert_ne!(shared.block(addr), independent.block(addr));
}

#[test]
fn replications_are_always_independent() {
    let addr = DrawAddress::from_quad(1, 0, 0, 5);
    let r0 = StreamRng::new(RngKey::new(42, 0, 0), Stream::Choice);
    let r1 = StreamRng::new(RngKey::new(42, 0, 1), Stream::Choice);
    assert_ne!(r0.block(addr), r1.block(addr));
}

#[test]
fn adding_an_alternative_leaves_the_others_alone() {
    // Keying on alternative *identity* rather than position is what buys this.
    // Design A offers alternatives {2, 5, 9}; design B adds 7. The draws for
    // 2, 5 and 9 must not move.
    let rng = choice_rng(555);
    let draw = |alt: u32| rng.block(DrawAddress::from_quad(100, 0, 3, alt)).unit(0);

    let before: Vec<f64> = [2u32, 5, 9].iter().map(|&a| draw(a)).collect();
    let _new_alternative = draw(7);
    let after: Vec<f64> = [2u32, 5, 9].iter().map(|&a| draw(a)).collect();

    assert_eq!(before, after);
}

#[test]
fn the_buffered_and_word_by_word_paths_agree() {
    // `block` takes the keystream in one `fill_bytes` because it is three
    // times faster. This asserts the optimisation did not change a single bit
    // of any draw in any scenario.
    let rng = choice_rng(4242);
    for i in 0..500u32 {
        let addr = DrawAddress::from_quad(i, i / 3, i / 7, i % 11);
        assert_eq!(rng.block(addr), rng.block_word_by_word(addr), "at {addr:?}");
    }
}

#[test]
fn derived_distributions_are_finite_and_in_range() {
    let rng = choice_rng(3);
    for i in 0..2_000u32 {
        let block = rng.block(DrawAddress::from_pair(i, 0));
        for w in 0..DrawBlock::LEN {
            let u = block.unit(w);
            assert!((0.0..1.0).contains(&u), "unit out of range: {u}");
            let o = block.open_unit(w);
            assert!(o > 0.0 && o < 1.0, "open_unit out of range: {o}");
            assert!(block.gumbel(w).is_finite(), "gumbel must never be infinite");
            assert!(block.exponential(w) >= 0.0);
        }
        for w in 0..DrawBlock::LEN / 2 {
            let u = block.unit_hq(w);
            assert!((0.0..1.0).contains(&u), "unit_hq out of range: {u}");
        }
    }
}

#[test]
fn uniform_draws_look_uniform() {
    // Not a statistical test — a smoke test that the wiring is right. A broken
    // address or a constant block would fail this by a mile.
    let rng = choice_rng(2024);
    let n = 20_000;
    let mut total = 0.0;
    let mut buckets = [0u32; 10];
    for i in 0..n {
        let u = rng.block(DrawAddress::from_pair(i, 0)).unit(0);
        total += u;
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "u is asserted to lie in [0, 1), so u * 10 is in [0, 10)"
        )]
        let bucket = (u * 10.0) as usize;
        buckets[bucket] += 1;
    }
    let mean = total / f64::from(n);
    assert!((mean - 0.5).abs() < 0.02, "mean {mean} is not near 0.5");
    let expected = f64::from(n) / 10.0;
    for (i, &count) in buckets.iter().enumerate() {
        let deviation = (f64::from(count) - expected).abs() / expected;
        assert!(deviation < 0.15, "bucket {i} holds {count}, expected about {expected}");
    }
}

#[test]
fn index_below_stays_in_range() {
    let rng = choice_rng(11);
    for n in [1u32, 2, 3, 7, 64, 1_000] {
        for i in 0..500 {
            let idx = rng.block(DrawAddress::from_pair(i, n)).index_below(0, n);
            assert!(idx < n, "index_below({n}) returned {idx}");
        }
    }
}

proptest! {
    /// Distinct addresses give distinct blocks, over a wide random sample.
    #[test]
    fn distinct_addresses_give_distinct_blocks(
        a in any::<u32>(), b in any::<u32>(), c in any::<u32>(), d in any::<u32>(),
    ) {
        let rng = choice_rng(8);
        let base = DrawAddress::from_quad(a, b, c, d);
        let shifted = DrawAddress::from_quad(a, b, c, d.wrapping_add(1));
        prop_assert_ne!(rng.block(base), rng.block(shifted));
    }

    /// `advance` walks to a genuinely different block and stays reproducible.
    #[test]
    fn advance_is_reproducible(a in any::<u32>(), n in 1u64..1_000) {
        let rng = choice_rng(17);
        let base = DrawAddress::from_one(a);
        prop_assert_ne!(rng.block(base), rng.block(base.advance(n)));
        prop_assert_eq!(rng.block(base.advance(n)), rng.block(base.advance(n)));
    }
}
