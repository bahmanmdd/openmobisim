//! Counter-based random draws, addressed by seed, stream and entity identity.
//!
//! A sequential generator ties the value of a draw to how many draws came before it, and so to
//! thread scheduling and to how work was split. Here a draw is a pure function of its key: no
//! shared state, no global generator, no thread-local. The draw belonging to trip 412 002 at
//! iteration 7 for alternative 19 has the same bits whatever order it is asked for in and
//! however many threads run.
//!
//! # The address of a draw
//!
//! | Level | Carried by | Purpose |
//! |---|---|---|
//! | the run's seed | [`RngKey`] | the run |
//! | the stream | [`Stream`] | keeps unrelated draws independent |
//! | the entity identity | [`DrawAddress`] | which draw within the stream |
//! | the word | [`DrawBlock`] | which of the sixteen values at that address |
//!
//! The seed and the stream make the 32-byte ChaCha8 key: bytes 0 to 7 the seed (little-endian),
//! bytes 8 and 9 the stream's discriminant (little-endian), bytes 10 to 31 the ASCII tag
//! `openmobisim-stream-rng`. The address selects one 64-byte ChaCha8 block directly: 64 bits of
//! ChaCha stream and 64 bits of block counter, so up to four `u32` identity components fit
//! without hashing and therefore without collisions.
//!
//! One [`DrawBlock`] is sixteen `u32` words for the price of one keystream block: take every
//! value one entity needs from one block.

use rand_chacha::ChaCha8Rng;
use rand_chacha::rand_core::{Rng, SeedableRng};

/// The tag in bytes 10 to 31 of every key: 8 (seed) + 2 (stream) + 22 = 32.
const DOMAIN_TAG: &[u8; 22] = b"openmobisim-stream-rng";

/// The independent random streams.
///
/// Separate streams mean the draws of one never move the draws of the other. The discriminant
/// enters the key, so renumbering changes every draw.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
#[repr(u16)]
pub enum Stream {
    /// The random part of choice. Addressed by (trip, block), each block holding the draws of
    /// eight alternatives in its even words and eight nests in its odd words; the draw is the
    /// traveller's fixed taste for the run.
    Choice = 0,
    /// Who is offered a new choice at an iteration (the method of successive averages,
    /// adjusted per traveller). Addressed by
    /// (trip, iteration).
    MsaReselection = 1,
}

impl Stream {
    /// Every stream, in discriminant order.
    pub const ALL: [Stream; 2] = [Stream::Choice, Stream::MsaReselection];
}

/// What identifies a run's randomness: its seed.
///
/// The same key and the same binary on the same platform give bit-identical draws.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct RngKey {
    /// The run's seed.
    pub seed: u64,
}

impl RngKey {
    /// The key of a run with seed `seed`.
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self { seed }
    }

    /// The 32-byte ChaCha8 key of one stream (layout in the module documentation).
    fn chacha_key(self, stream: Stream) -> [u8; 32] {
        let mut key = [0u8; 32];
        key[0..8].copy_from_slice(&self.seed.to_le_bytes());
        key[8..10].copy_from_slice(&(stream as u16).to_le_bytes());
        key[10..32].copy_from_slice(DOMAIN_TAG);
        key
    }
}

/// Where a draw lives inside a stream: built from the identity of what the draw belongs to,
/// never from an incrementing counter.
///
/// `from_pair(a, b)` sets the ChaCha stream to `a · 2³² + b` and the block counter to zero;
/// `from_triple(a, b, c)` sets the same stream and the block counter to `c`. So
/// `from_pair(a, b)` and `from_triple(a, b, 0)` are the same address: each stream uses one arity
/// throughout ([`Stream::Choice`] and [`Stream::MsaReselection`] the pair), and two streams
/// cannot collide because their keys differ.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct DrawAddress {
    stream_pos: u64,
    block_pos: u64,
}

impl DrawAddress {
    /// The address of the draw for two identity components, for example (trip, iteration).
    #[inline]
    #[must_use]
    pub const fn from_pair(a: u32, b: u32) -> Self {
        Self { stream_pos: ((a as u64) << 32) | b as u64, block_pos: 0 }
    }

    /// The address of the draw for three identity components, for example
    /// (trip, iteration, alternative).
    #[inline]
    #[must_use]
    pub const fn from_triple(a: u32, b: u32, c: u32) -> Self {
        Self { stream_pos: ((a as u64) << 32) | b as u64, block_pos: c as u64 }
    }
}

/// One stream of one run: a ChaCha8 key, ready to be addressed.
///
/// Holds no mutable state, so it is shared across threads as it is.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct StreamRng {
    key: [u8; 32],
    stream: Stream,
}

impl StreamRng {
    /// The generator of stream `stream` of the run keyed `key`.
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

    /// The sixteen words at `address`: one ChaCha8 block, its 64 bytes read as little-endian
    /// `u32` words.
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
}

/// Sixteen random words drawn at one address.
///
/// Which word a consumer uses for which quantity is its own stable convention, written down
/// where it is used: changing it changes results.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DrawBlock {
    words: [u32; Self::LEN],
}

impl DrawBlock {
    /// Words per block: one ChaCha block is 64 bytes.
    pub const LEN: usize = 16;

    /// `2⁻³²`, the scale that maps a `u32` into the unit interval.
    const SCALE_32: f64 = 1.0 / 4_294_967_296.0;

    /// The raw word `i`.
    ///
    /// # Panics
    ///
    /// If `i >= 16`.
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

    /// A uniform draw in `[0, 1)` from word `i`, with 32 bits of resolution: `word · 2⁻³²`.
    ///
    /// # Panics
    ///
    /// If `i >= 16`.
    #[inline]
    #[must_use]
    pub fn unit(&self, i: usize) -> f64 {
        f64::from(self.words[i]) * Self::SCALE_32
    }

    /// A uniform draw in the open interval `(0, 1)` from word `i`: `(word + ½) · 2⁻³²`.
    ///
    /// Never exactly zero or one, so its logarithm is always finite.
    ///
    /// # Panics
    ///
    /// If `i >= 16`.
    #[inline]
    #[must_use]
    pub fn open_unit(&self, i: usize) -> f64 {
        (f64::from(self.words[i]) + 0.5) * Self::SCALE_32
    }

    /// A standard Gumbel draw from word `i`: `−ln(−ln u)`, `u` from [`open_unit`](Self::open_unit).
    ///
    /// Its mean is Euler's constant (0.5772…) and its variance π²/6. Uses the platform's `ln`, so
    /// the bits are reproducible on one platform, not across platforms.
    ///
    /// # Panics
    ///
    /// If `i >= 16`.
    #[inline]
    #[must_use]
    pub fn gumbel(&self, i: usize) -> f64 {
        -(-self.open_unit(i).ln()).ln()
    }

    /// A Bernoulli draw from word `i`: true with probability `p`.
    ///
    /// # Panics
    ///
    /// If `i >= 16`.
    #[inline]
    #[must_use]
    pub fn bernoulli(&self, i: usize, p: f64) -> bool {
        self.unit(i) < p
    }
}
