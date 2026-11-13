//! The fingerprint's hash against the published FNV-1a 64-bit test vectors (Fowler, Noll and
//! Vo; the reference implementation's test suite): raw bytes, no length before them.
//!
//! | Input | FNV-1a 64 |
//! |---|---|
//! | `""` | `cbf29ce484222325` (the offset basis) |
//! | `"a"` | `af63dc4c8601ec8c` |
//! | `"foobar"` | `85944171f73967e8` |
//!
//! For bikes: a set of modes is one byte, the bike its bit 1; {car, walk} is 0b101, and adding
//! the bike would make it 0b111.

use openmobisim_types::fingerprint::Fingerprint;
use openmobisim_types::mode::{Mode, ModeSet};

fn fnv1a(bytes: &[u8]) -> String {
    let mut h = Fingerprint::new();
    h.bytes(bytes);
    format!("{:016x}", h.finish())
}

#[test]
fn the_published_vectors_empty_a_and_foobar() {
    assert_eq!(fnv1a(b""), "cbf29ce484222325");
    assert_eq!(fnv1a(b"a"), "af63dc4c8601ec8c");
    assert_eq!(fnv1a(b"foobar"), "85944171f73967e8");
}

#[test]
fn a_value_is_its_little_endian_bytes_and_a_string_its_length_then_its_bytes() {
    // By the definitions in the module documentation: each typed feed equals the raw bytes it
    // names, so the hash of the typed feed is the hash of those bytes.
    let typed = |feed: &dyn Fn(&mut Fingerprint)| {
        let mut h = Fingerprint::new();
        feed(&mut h);
        h.finish()
    };
    let raw = |bytes: &[u8]| {
        let mut h = Fingerprint::new();
        h.bytes(bytes);
        h.finish()
    };
    assert_eq!(typed(&|h| h.u32(0x0403_0201)), raw(&[1, 2, 3, 4]));
    assert_eq!(typed(&|h| h.u64(1)), raw(&[1, 0, 0, 0, 0, 0, 0, 0]));
    // 1.0 is 0x3ff0000000000000 in IEEE 754 binary64.
    assert_eq!(typed(&|h| h.f64(1.0)), raw(&[0, 0, 0, 0, 0, 0, 0xf0, 0x3f]));
    assert_eq!(typed(&|h| h.str("ab")), raw(&[2, 0, 0, 0, 0, 0, 0, 0, b'a', b'b']));
    assert_eq!(typed(&|h| h.flag(true)), raw(&[1]));
    // {car, walk}: bits 0 and 2, the byte 0b101.
    assert_eq!(typed(&|h| h.modes(ModeSet::of(&[Mode::Walk, Mode::Car]))), raw(&[0b101]));
    // "ab" then "c" is not "a" then "bc": the lengths tell them apart.
    assert_ne!(
        typed(&|h| {
            h.str("ab");
            h.str("c");
        }),
        typed(&|h| {
            h.str("a");
            h.str("bc");
        })
    );
}
