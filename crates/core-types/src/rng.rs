//! Counter-based random draws, addressed by entity identity (Foundations §4).
//!
//! # Why not a sequential RNG
//!
//! A sequential generator ties the value of a draw to *how many draws came
//! before it*, which ties it to thread scheduling, to how work was chunked and
//! to whether an unrelated feature was switched on. That makes reproducibility
//! fragile and common random numbers across designs impossible.
//!
//! Here, **a draw is a pure function of its key**. There is no shared mutable
//! state, no global generator and no thread-local. Two runs that ask for the
//! draw belonging to traveller 412 002, trip 3, iteration 7, alternative 19
//! get the same bits, whatever order they asked in and however many threads
//! were running.
//!
//! # The address space
//!
//! A draw is located by four things:
//!
//! | Level | Carried by | Purpose |
//! |---|---|---|
//! | `master_seed`, `design_id`, `replication` | [`RngKey`] | The run |
//! | [`Stream`] | the stream table below | Keeps unrelated draws independent |
//! | [`DrawAddress`] | entity identity | Which draw within the stream |
//! | word index | [`DrawBlock`] | Which value at that address |
//!
//! The first two are folded into a 32-byte ChaCha8 key once, when
//! [`StreamRng`] is built. The third addresses a 64-byte ChaCha8 block
//! directly — 64 bits of ChaCha stream and 64 bits of block counter, which is
//! why [`DrawAddress`] can hold up to four `u32` identity components without
//! hashing and therefore without collisions.
//!
//! # Common random numbers
//!
//! Comparing two designs under different random draws measures the difference
//! between the draws as much as the difference between the designs. So by
//! default **`design_id` does not enter the key at all**: hold `master_seed`
//! and `replication` fixed, vary the design, and every draw whose identity is
//! unchanged is unchanged. Two designs see the same travellers making the same
//! tie-breaks, and the difference between their KPIs is the design.
//!
//! Keying choice draws on *alternative identity* rather than on position in a
//! list is what makes this hold even when a design adds or removes an
//! alternative: the surviving alternatives keep their draws instead of all
//! shifting along by one.
//!
//! A user who wants independent randomness per design — to estimate the
//! variance of the design comparison itself, say — sets
//! [`RngKey::independent_per_design`], and *then* the design id is folded in.
//! It is an explicit request, never the default.
//!
//! # Cost
//!
//! One [`DrawBlock`] is one ChaCha8 block: sixteen `u32` words for the price
//! of one keystream generation. **Pull every value you need for one entity
//! from one block.** Constructing a block per value would multiply the cost by
//! sixteen, and the choice model is the hottest consumer in the simulator.

use core::fmt;

use rand_chacha::ChaCha8Rng;
use rand_chacha::rand_core::{Rng, SeedableRng};

/// Version of the seed-derivation scheme.
///
/// Changing how [`RngKey`] and [`Stream`] are folded into the ChaCha key
/// changes **every draw in every scenario**. Bump this when that happens, so
/// that a stored manifest says which scheme produced it; it is written to
/// `manifest.json` beside the master seed.
pub const RNG_SCHEME_VERSION: u16 = 1;

/// Domain-separation tag mixed into every key. Fourteen bytes, by arithmetic:
/// 8 (seed) + 2 (stream) + 4 (design) + 4 (replication) + 14 = 32.
///
/// Fixed-width and load-bearing: this exact byte sequence is folded into
/// every draw's ChaCha8 key (see [`RNG_SCHEME_VERSION`]). It is an internal
/// domain separator, not project branding, so the public rename left it as
/// `mobisim`-prefixed rather than widening the key layout.
const DOMAIN_TAG: &[u8; 14] = b"mobisim-rng-v1";

/// The independent random streams (Foundations §4).
///
/// Separate streams mean that switching a feature on cannot perturb the draws
/// of an unrelated one. The discriminants are **stable**: they are folded into
/// the key, so renumbering changes every draw.
///
/// Which streams were live during a run is recorded in `manifest.json`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
#[repr(u16)]
pub enum Stream {
    /// Synthetic population, trips and locations. Keyed on (person, trip, draw).
    DemandSynthesis = 0,
    /// Tie-breaks and locations when integerising OD matrices. Keyed on (cell, interval, draw).
    DemandIntegerisation = 1,
    /// Departure times drawn from a profile. Keyed on (traveller, trip).
    DepartureTime = 2,
    /// Discrete choice. Live by default. Keyed on (traveller, trip, iteration, alternative).
    Choice = 3,
    /// Which travellers re-choose at iteration *n*. Keyed on (traveller, iteration).
    MsaReselection = 4,
    /// Handed to tier-2 fleet policies. Keyed on (policy tick, draw).
    FleetPolicy = 5,
    /// Optional stochastic disruption timing. Keyed on (event, draw).
    DisruptionJitter = 6,
}

impl Stream {
    /// Every stream, in discriminant order.
    pub const ALL: [Stream; 7] = [
        Stream::DemandSynthesis,
        Stream::DemandIntegerisation,
        Stream::DepartureTime,
        Stream::Choice,
        Stream::MsaReselection,
        Stream::FleetPolicy,
        Stream::DisruptionJitter,
    ];

    /// The stable snake_case name written to `manifest.json`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Stream::DemandSynthesis => "demand_synthesis",
            Stream::DemandIntegerisation => "demand_integerisation",
            Stream::DepartureTime => "departure_time",
            Stream::Choice => "choice",
            Stream::MsaReselection => "msa_reselection",
            Stream::FleetPolicy => "fleet_policy",
            Stream::DisruptionJitter => "disruption_jitter",
        }
    }
}

impl fmt::Display for Stream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What identifies a run's randomness.
///
/// Reproducibility is a guarantee about this struct: the same key, the same
/// binary and the same platform give bit-identical draws.
///
/// # Examples
///
/// ```
/// use openmobisim_core_types::rng::{DrawAddress, RngKey, Stream, StreamRng};
///
/// let addr = DrawAddress::from_quad(1, 0, 0, 5);
///
/// // Two designs, same seed and replication: common random numbers.
/// let a = StreamRng::new(RngKey::new(42, 0, 0), Stream::Choice);
/// let b = StreamRng::new(RngKey::new(42, 7, 0), Stream::Choice);
/// assert_eq!(a.unit(addr), b.unit(addr));
///
/// // Asked for explicitly, the design id separates the streams.
/// let c = StreamRng::new(RngKey::new(42, 7, 0).independent_per_design(), Stream::Choice);
/// assert_ne!(a.unit(addr), c.unit(addr));
///
/// // A different replication is always a different draw.
/// let d = StreamRng::new(RngKey::new(42, 0, 1), Stream::Choice);
/// assert_ne!(a.unit(addr), d.unit(addr));
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct RngKey {
    /// The scenario's master seed. The one number a user sets.
    pub master_seed: u64,
    /// Which design in a batch.
    ///
    /// **Ignored unless [`independent_per_design`](Self::independent_per_design)
    /// is set** — that is what gives common random numbers across designs.
    pub design_id: u32,
    /// Which replication of that design. Always part of the key.
    pub replication: u32,
    /// Whether to give each design its own randomness.
    ///
    /// `false` (the default) means designs share draws, so a KPI difference
    /// between two designs is the design and not the noise. Set it only when
    /// the independence is what you are after.
    pub independent_per_design: bool,
}

impl RngKey {
    /// A key for one replication of one design, with common random numbers.
    #[must_use]
    pub const fn new(master_seed: u64, design_id: u32, replication: u32) -> Self {
        Self { master_seed, design_id, replication, independent_per_design: false }
    }

    /// A key for replication zero of design zero — the common case for one run.
    #[must_use]
    pub const fn from_seed(master_seed: u64) -> Self {
        Self::new(master_seed, 0, 0)
    }

    /// The same key, with the design id folded in.
    ///
    /// Gives each design its own randomness, at the cost of the common random
    /// numbers that make design comparison cheap.
    #[must_use]
    pub const fn independent_per_design(self) -> Self {
        Self { independent_per_design: true, ..self }
    }

    /// The 32-byte ChaCha key for one stream of this run.
    ///
    /// Layout — stable, and versioned by [`RNG_SCHEME_VERSION`]:
    ///
    /// | Bytes | Contents |
    /// |---|---|
    /// | 0..8 | `master_seed`, little-endian |
    /// | 8..10 | stream discriminant |
    /// | 10..14 | `design_id`, or zero when designs share randomness |
    /// | 14..18 | `replication` |
    /// | 18..32 | the domain-separation tag |
    fn chacha_key(self, stream: Stream) -> [u8; 32] {
        let design = if self.independent_per_design { self.design_id } else { 0 };
        let mut key = [0u8; 32];
        key[0..8].copy_from_slice(&self.master_seed.to_le_bytes());
        key[8..10].copy_from_slice(&(stream as u16).to_le_bytes());
        key[10..14].copy_from_slice(&design.to_le_bytes());
        key[14..18].copy_from_slice(&self.replication.to_le_bytes());
        key[18..32].copy_from_slice(DOMAIN_TAG);
        key
    }
}

/// Where a draw lives inside a stream.
///
/// Built from the identity of whatever the draw belongs to, never from a
/// counter that increments. The constructors pack up to four `u32` components
/// into the 128 bits of address ChaCha8 offers — 64 bits of stream position
/// and 64 of block counter — with no hashing, and therefore no birthday
/// collisions.
///
/// # One stream, one arity
///
/// The 128 bits are **exactly** four `u32`s, with nothing left over for an
/// arity tag. So `from_pair(a, b)` and `from_triple(a, b, 0)` are the same
/// address, as are `from_triple(a, b, c)` and `from_quad(a, b, 0, c)`.
///
/// That is harmless, and it is harmless for a stated reason: **every stream
/// addresses its draws with one arity throughout**, fixed by what the stream
/// is keyed on. Mixing arities inside one stream is the bug this note exists
/// to prevent; two streams using different arities cannot collide, because
/// their keys differ.
///
/// | Stream | Arity | Components |
/// |---|---|---|
/// | [`Stream::DemandSynthesis`] | triple | person, trip, draw |
/// | [`Stream::DemandIntegerisation`] | triple | cell, interval, draw |
/// | [`Stream::DepartureTime`] | pair | traveller, trip |
/// | [`Stream::Choice`] | quad | traveller, trip, iteration, alternative |
/// | [`Stream::MsaReselection`] | pair | traveller, iteration |
/// | [`Stream::FleetPolicy`] | pair | policy tick, draw |
/// | [`Stream::DisruptionJitter`] | pair | event, draw |
///
/// Where one identity needs more than sixteen words, use
/// [`advance`](Self::advance) rather than reaching for a wider constructor.
///
/// # Examples
///
/// ```
/// use openmobisim_core_types::rng::DrawAddress;
///
/// // A choice draw: (traveller, trip, iteration, alternative).
/// let a = DrawAddress::from_quad(412_002, 3, 7, 19);
/// let b = DrawAddress::from_quad(412_002, 3, 7, 20);
/// assert_ne!(a, b);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct DrawAddress {
    stream_pos: u64,
    block_pos: u64,
}

impl DrawAddress {
    /// Address a draw by one identity component.
    #[inline]
    #[must_use]
    pub const fn from_one(a: u32) -> Self {
        Self { stream_pos: a as u64, block_pos: 0 }
    }

    /// Address a draw by two identity components — for example (traveller, trip).
    #[inline]
    #[must_use]
    pub const fn from_pair(a: u32, b: u32) -> Self {
        Self { stream_pos: ((a as u64) << 32) | b as u64, block_pos: 0 }
    }

    /// Address a draw by three identity components — for example
    /// (traveller, trip, iteration).
    #[inline]
    #[must_use]
    pub const fn from_triple(a: u32, b: u32, c: u32) -> Self {
        Self { stream_pos: ((a as u64) << 32) | b as u64, block_pos: c as u64 }
    }

    /// Address a draw by four identity components — the choice stream's
    /// (traveller, trip, iteration, alternative).
    #[inline]
    #[must_use]
    pub const fn from_quad(a: u32, b: u32, c: u32, d: u32) -> Self {
        Self { stream_pos: ((a as u64) << 32) | b as u64, block_pos: ((c as u64) << 32) | d as u64 }
    }

    /// Address the `n`-th block after this one.
    ///
    /// For the rare consumer that needs more than sixteen words at one
    /// identity — a synthetic-population draw, say. Prefer widening the
    /// identity tuple where one exists.
    #[inline]
    #[must_use]
    pub const fn advance(self, n: u64) -> Self {
        Self { stream_pos: self.stream_pos, block_pos: self.block_pos.wrapping_add(n) }
    }
}

/// One stream of one run: a ChaCha8 key, ready to be addressed.
///
/// Cheap to clone, `Send + Sync`, and safe to share across threads — it holds
/// no mutable state, which is the point.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct StreamRng {
    key: [u8; 32],
    stream: Stream,
}

impl StreamRng {
    /// Build the generator for one stream of one run.
    #[must_use]
    pub fn new(key: RngKey, stream: Stream) -> Self {
        Self { key: key.chacha_key(stream), stream }
    }

    /// Which stream this is.
    #[inline]
    #[must_use]
    pub const fn stream(&self) -> Stream {
        self.stream
    }

    /// The sixteen words at `address`.
    ///
    /// One ChaCha8 block, costing roughly 35 ns. Pull every value you need for
    /// one entity from the block you already have rather than making another.
    ///
    /// The keystream is taken with a single `fill_bytes` into a 64-byte buffer
    /// rather than sixteen `next_u32` calls. The two produce **identical**
    /// words — `tests/rng.rs` asserts it — but the buffered path costs about a
    /// third as much, because each `next_u32` carries its own bounds check and
    /// buffer bookkeeping. At the volumes the choice model draws at, that
    /// difference is minutes.
    #[must_use]
    pub fn block(&self, address: DrawAddress) -> DrawBlock {
        let mut rng = ChaCha8Rng::from_seed(self.key);
        rng.set_stream(address.stream_pos);
        // The word position is the block counter times sixteen words.
        rng.set_word_pos(u128::from(address.block_pos) << 4);

        let mut bytes = [0u8; DrawBlock::LEN * 4];
        rng.fill_bytes(&mut bytes);

        let mut words = [0u32; DrawBlock::LEN];
        for (word, chunk) in words.iter_mut().zip(bytes.chunks_exact(4)) {
            *word = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        }
        DrawBlock { words }
    }

    /// The sixteen words at `address`, taken one at a time.
    ///
    /// Kept only so that the test suite can assert it agrees with
    /// [`block`](Self::block). Never use it in anger: it costs about three
    /// times as much for the same bits.
    #[doc(hidden)]
    #[must_use]
    pub fn block_word_by_word(&self, address: DrawAddress) -> DrawBlock {
        let mut rng = ChaCha8Rng::from_seed(self.key);
        rng.set_stream(address.stream_pos);
        rng.set_word_pos(u128::from(address.block_pos) << 4);

        let mut words = [0u32; DrawBlock::LEN];
        for w in &mut words {
            *w = rng.next_u32();
        }
        DrawBlock { words }
    }

    /// A single uniform draw in `[0, 1)` at `address`, word zero.
    ///
    /// Convenience for the one-value case. If you need two values at the same
    /// address, call [`block`](Self::block) once instead — this costs a full
    /// block either way.
    #[must_use]
    pub fn unit(&self, address: DrawAddress) -> f64 {
        self.block(address).unit(0)
    }
}

/// Sixteen random words drawn at one address.
///
/// The words are independent; which one you use for which quantity is part of
/// your consumer's own stable convention. Write that convention down, because
/// changing it changes results.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DrawBlock {
    words: [u32; Self::LEN],
}

impl DrawBlock {
    /// Words per block. One ChaCha block is 64 bytes.
    pub const LEN: usize = 16;

    /// `2^-32`, the scale that turns a `u32` into a unit interval value.
    const SCALE_32: f64 = 1.0 / 4_294_967_296.0;

    /// The raw word at `i`.
    ///
    /// # Panics
    ///
    /// Panics if `i >= 16`.
    #[inline]
    #[must_use]
    pub const fn word(&self, i: usize) -> u32 {
        self.words[i]
    }

    /// All sixteen words.
    #[inline]
    #[must_use]
    pub const fn words(&self) -> &[u32; Self::LEN] {
        &self.words
    }

    /// A 64-bit value from word pair `i` — `i` in `0..8`.
    ///
    /// # Panics
    ///
    /// Panics if `i >= 8`.
    #[inline]
    #[must_use]
    pub const fn word64(&self, i: usize) -> u64 {
        ((self.words[2 * i] as u64) << 32) | self.words[2 * i + 1] as u64
    }

    /// A uniform draw in `[0, 1)` from word `i`, with 32 bits of resolution.
    ///
    /// Enough for choice and for acceptance tests. Use
    /// [`unit_hq`](Self::unit_hq) where the tail matters.
    ///
    /// # Panics
    ///
    /// Panics if `i >= 16`.
    #[inline]
    #[must_use]
    pub fn unit(&self, i: usize) -> f64 {
        f64::from(self.words[i]) * Self::SCALE_32
    }

    /// A uniform draw in the **open** interval `(0, 1)` from word `i`.
    ///
    /// Never returns exactly zero or one, so `ln` of it is always finite. This
    /// is the one to use before a logarithm.
    ///
    /// # Panics
    ///
    /// Panics if `i >= 16`.
    #[inline]
    #[must_use]
    pub fn open_unit(&self, i: usize) -> f64 {
        (f64::from(self.words[i]) + 0.5) * Self::SCALE_32
    }

    /// A uniform draw in `[0, 1)` from word pair `i`, with 53 bits of
    /// resolution — `i` in `0..8`.
    ///
    /// # Panics
    ///
    /// Panics if `i >= 8`.
    #[inline]
    #[must_use]
    pub fn unit_hq(&self, i: usize) -> f64 {
        #[allow(
            clippy::cast_precision_loss,
            reason = "the value is shifted to 53 bits, which f64 represents exactly"
        )]
        let x = (self.word64(i) >> 11) as f64;
        x * (1.0 / 9_007_199_254_740_992.0) // 2^-53
    }

    /// A standard Gumbel draw from word `i`: `-ln(-ln(u))`, `u` in `(0, 1)`.
    ///
    /// The error term of a multinomial logit. Adding one of these to each
    /// alternative's systematic utility and taking the argmax reproduces the
    /// logit choice probabilities exactly — the Gumbel-max trick — without
    /// ever forming the probabilities.
    ///
    /// Uses the platform's `ln`, so results are reproducible on one platform
    /// and not across platforms. That is the documented guarantee.
    ///
    /// # Panics
    ///
    /// Panics if `i >= 16`.
    #[inline]
    #[must_use]
    pub fn gumbel(&self, i: usize) -> f64 {
        -(-self.open_unit(i).ln()).ln()
    }

    /// A standard exponential draw from word `i`: `-ln(u)`, `u` in `(0, 1)`.
    ///
    /// # Panics
    ///
    /// Panics if `i >= 16`.
    #[inline]
    #[must_use]
    pub fn exponential(&self, i: usize) -> f64 {
        -self.open_unit(i).ln()
    }

    /// A Bernoulli draw from word `i`, true with probability `p`.
    ///
    /// # Panics
    ///
    /// Panics if `i >= 16`.
    #[inline]
    #[must_use]
    pub fn bernoulli(&self, i: usize, p: f64) -> bool {
        self.unit(i) < p
    }

    /// A uniform integer in `0..n` from word `i`.
    ///
    /// Uses the multiply-shift map, which is one multiply rather than a
    /// rejection loop. It carries a bias below `2^-32 · n`, which is
    /// negligible for every `n` openmobisim draws against (route-set sizes, fleet
    /// sizes) and — more importantly here — it is *branch-free and therefore
    /// constant-time*, so it cannot make one run diverge from another.
    ///
    /// # Panics
    ///
    /// Panics if `i >= 16` or if `n` is zero.
    #[inline]
    #[must_use]
    pub fn index_below(&self, i: usize, n: u32) -> u32 {
        assert!(n > 0, "index_below needs a non-empty range");
        #[allow(
            clippy::cast_possible_truncation,
            reason = "the product is shifted right by 32, so the result is below n"
        )]
        let idx = ((u64::from(self.words[i]) * u64::from(n)) >> 32) as u32;
        idx
    }
}
