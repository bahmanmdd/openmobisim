//! A fingerprint: the 64-bit FNV-1a hash (Fowler, Noll and Vo) of what a network, a timetable or
//! a demand was built from, so that a run's record can say which inputs it ran on.
//!
//! FNV-1a starts from the offset basis `0xcbf29ce484222325` and, for each byte, takes the
//! exclusive or of the state and the byte, then multiplies by the prime `0x100000001b3` modulo
//! 2⁶⁴. A value is fed as bytes, one way per type: an integer little-endian, a float by its
//! bits (`to_bits`, little-endian, so `0.0` and `-0.0` differ and every `NaN` of the same bits is
//! the same), a flag as one byte (0 or 1), a set of modes as one byte (bit `k` for the mode of
//! discriminant `k`), a string as its length in bytes (`u64`) then its UTF-8 bytes, so that two
//! strings side by side cannot be read as another pair. Not a cryptographic hash: two inputs
//! may share a fingerprint by chance (about one pair in 2⁶⁴), never by the order of their rows.
//!
//! For bikes: the bike is bit 1 of a mode set's byte, so a link opened to or closed to bikes,
//! or a trip offered the bike or not, feeds another byte and gives another fingerprint.

use crate::mode::{Mode, ModeSet};

/// The FNV-1a state as bytes are fed.
#[derive(Clone, Copy, Debug)]
pub struct Fingerprint {
    state: u64,
}

impl Default for Fingerprint {
    fn default() -> Self {
        Self::new()
    }
}

impl Fingerprint {
    /// The 64-bit offset basis.
    pub const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    /// The 64-bit prime, 2⁴⁰ + 2⁸ + 0xb3.
    pub const PRIME: u64 = 0x0000_0100_0000_01b3;

    /// The hash of nothing: the offset basis.
    #[must_use]
    pub const fn new() -> Self {
        Self { state: Self::OFFSET_BASIS }
    }

    /// Feed raw bytes, nothing before them.
    pub fn bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.state = (self.state ^ u64::from(b)).wrapping_mul(Self::PRIME);
        }
    }

    /// Feed one byte.
    pub fn u8(&mut self, value: u8) {
        self.bytes(&[value]);
    }

    /// Feed a flag as one byte, 1 for true.
    pub fn flag(&mut self, value: bool) {
        self.u8(u8::from(value));
    }

    /// Feed a `u32`, little-endian.
    pub fn u32(&mut self, value: u32) {
        self.bytes(&value.to_le_bytes());
    }

    /// Feed a `u64`, little-endian.
    pub fn u64(&mut self, value: u64) {
        self.bytes(&value.to_le_bytes());
    }

    /// Feed a float by its bits, little-endian.
    pub fn f64(&mut self, value: f64) {
        self.u64(value.to_bits());
    }

    /// Feed a string: its length in bytes as a `u64`, then its UTF-8 bytes.
    pub fn str(&mut self, value: &str) {
        self.u64(value.len() as u64);
        self.bytes(value.as_bytes());
    }

    /// Feed a set of modes as one byte: bit `k` set when the mode of discriminant `k` is in it.
    pub fn modes(&mut self, set: ModeSet) {
        let bits =
            Mode::ALL.iter().filter(|&&m| set.contains(m)).fold(0, |b, m| b | 1 << m.as_u8());
        self.u8(bits);
    }

    /// The hash of everything fed so far.
    #[must_use]
    pub const fn finish(&self) -> u64 {
        self.state
    }
}
