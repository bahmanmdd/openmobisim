//! Seeded test inputs: SplitMix64 (Steele, Lea and Flood, 2014), written out here so that the
//! inputs do not come from the crate under test.

/// A SplitMix64 sequence from `seed`.
pub struct SplitMix64(pub u64);

impl SplitMix64 {
    /// The next 64-bit value.
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// The next value in `0..n`, by the remainder (its bias, below n / 2⁶⁴, is irrelevant here).
    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }

    /// The next index in `0..n`.
    pub fn index(&mut self, n: usize) -> usize {
        usize::try_from(self.below(n as u64)).expect("below n, which is a usize")
    }
}
