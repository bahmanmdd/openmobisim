//! A small, dependency-free hash for fingerprints (S168).
//!
//! FNV-1a, 64 bits, written by hand: not cryptographic and not meant as one.
//! A fingerprint here only has to tell two inputs apart when they differ by
//! accident (another network, another seed, an edited demand row), not resist
//! someone who builds a collision on purpose. Ten lines of arithmetic do that
//! without adding to the dependency graph the wheel-building promise has to
//! keep pure Rust (Foundations §3). The wider, cache-keyed fingerprint of
//! Foundations §8 (BLAKE3) arrives with the artifact cache and can replace this
//! behind the same call sites.
//!
//! Every write is little-endian and fixed-width, so a fingerprint is the same
//! on every platform; a string is written with its length first, so `("ab",
//! "c")` and `("a", "bc")` differ.

/// A running FNV-1a 64-bit hash.
#[derive(Clone, Copy, Debug)]
pub struct Fnv1a(u64);

impl Fnv1a {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    /// A new hash, before anything is written.
    #[must_use]
    pub const fn new() -> Self {
        Self(Self::OFFSET_BASIS)
    }

    /// Write raw bytes, as they are.
    pub fn write_bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(Self::PRIME);
        }
    }

    /// Write one byte.
    pub fn write_u8(&mut self, v: u8) {
        self.write_bytes(&[v]);
    }

    /// Write a `u32`, little-endian.
    pub fn write_u32(&mut self, v: u32) {
        self.write_bytes(&v.to_le_bytes());
    }

    /// Write a `u64`, little-endian.
    pub fn write_u64(&mut self, v: u64) {
        self.write_bytes(&v.to_le_bytes());
    }

    /// Write an `f64` by its bit pattern, so `0.0` and `-0.0` differ and a
    /// value is never rounded on the way in.
    pub fn write_f64(&mut self, v: f64) {
        self.write_u64(v.to_bits());
    }

    /// Write a flag as one byte.
    pub fn write_bool(&mut self, v: bool) {
        self.write_u8(u8::from(v));
    }

    /// Write a string, its byte length first.
    ///
    /// # Panics
    ///
    /// Panics if the string is longer than `u32::MAX` bytes.
    pub fn write_str(&mut self, s: &str) {
        let len = u32::try_from(s.len()).expect("a hashed string is shorter than 4 GiB");
        self.write_u32(len);
        self.write_bytes(s.as_bytes());
    }

    /// The hash of everything written so far.
    #[must_use]
    pub const fn finish(self) -> u64 {
        self.0
    }
}

impl Default for Fnv1a {
    fn default() -> Self {
        Self::new()
    }
}

/// A 64-bit fingerprint as 16 lower-case hex digits: the form written to a
/// manifest and a results file.
#[must_use]
pub fn fingerprint_hex(value: u64) -> String {
    format!("{value:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_published_fnv1a_test_vectors() {
        // From the FNV reference: the empty string is the offset basis, and
        // "a" and "foobar" have these 64-bit FNV-1a values.
        assert_eq!(Fnv1a::new().finish(), 0xcbf2_9ce4_8422_2325);
        let mut a = Fnv1a::new();
        a.write_bytes(b"a");
        assert_eq!(a.finish(), 0xaf63_dc4c_8601_ec8c);
        let mut foobar = Fnv1a::new();
        foobar.write_bytes(b"foobar");
        assert_eq!(foobar.finish(), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn strings_are_delimited_and_floats_are_exact() {
        let hash = |parts: &[&str]| {
            let mut h = Fnv1a::new();
            for p in parts {
                h.write_str(p);
            }
            h.finish()
        };
        assert_ne!(hash(&["ab", "c"]), hash(&["a", "bc"]));
        let mut zero = Fnv1a::new();
        zero.write_f64(0.0);
        let mut negative_zero = Fnv1a::new();
        negative_zero.write_f64(-0.0);
        assert_ne!(zero.finish(), negative_zero.finish());
    }

    #[test]
    fn hex_is_sixteen_digits() {
        assert_eq!(fingerprint_hex(0xabc), "0000000000000abc");
        assert_eq!(fingerprint_hex(u64::MAX).len(), 16);
    }
}
