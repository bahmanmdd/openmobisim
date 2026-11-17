//! The page's data as text: typed arrays laid end to end in one block, compressed with zlib and
//! written in base64; and the few JSON values the page's table of contents needs, written by
//! hand.

use std::io::Write as _;

use flate2::Compression;
use flate2::write::ZlibEncoder;

/// Typed arrays laid end to end, each little-endian and starting on a multiple of 8 bytes, with
/// their table of contents: name, type (`u8`, `u32`, `i32`, `f32`), byte offset, element count.
#[derive(Debug, Default)]
pub(crate) struct Blob {
    bytes: Vec<u8>,
    table: Vec<(&'static str, &'static str, usize, usize)>,
}

impl Blob {
    fn add(&mut self, name: &'static str, kind: &'static str, count: usize, data: &[u8]) {
        debug_assert!(self.table.iter().all(|e| e.0 != name), "{name} added twice");
        self.table.push((name, kind, self.bytes.len(), count));
        self.bytes.extend_from_slice(data);
        let pad = (8 - self.bytes.len() % 8) % 8;
        self.bytes.extend(std::iter::repeat_n(0, pad));
    }

    /// Add an array of bytes.
    pub fn u8(&mut self, name: &'static str, values: &[u8]) {
        self.add(name, "u8", values.len(), values);
    }

    /// Add an array of unsigned 32-bit integers.
    pub fn u32(&mut self, name: &'static str, values: &[u32]) {
        let data: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        self.add(name, "u32", values.len(), &data);
    }

    /// Add an array of signed 32-bit integers.
    pub fn i32(&mut self, name: &'static str, values: &[i32]) {
        let data: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        self.add(name, "i32", values.len(), &data);
    }

    /// Add an array of 32-bit floats.
    pub fn f32(&mut self, name: &'static str, values: &[f32]) {
        let data: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        self.add(name, "f32", values.len(), &data);
    }

    /// The table of contents as a JSON array of `{"n": name, "t": type, "o": offset, "c": count}`.
    pub fn table(&self) -> String {
        let entries: Vec<String> = self
            .table
            .iter()
            .map(|(n, t, o, c)| format!("{{\"n\":\"{n}\",\"t\":\"{t}\",\"o\":{o},\"c\":{c}}}"))
            .collect();
        format!("[{}]", entries.join(","))
    }

    /// The block compressed with zlib (level 6) and written in base64: the same bytes for the
    /// same arrays.
    pub fn packed(&self) -> String {
        let mut z = ZlibEncoder::new(Vec::new(), Compression::new(6));
        z.write_all(&self.bytes).expect("writing to memory");
        base64(&z.finish().expect("writing to memory"))
    }
}

/// Bytes in base64, the standard alphabet with `=` padding (RFC 4648), as the browser's `atob`
/// reads it.
pub(crate) fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], chunk.get(1).copied().unwrap_or(0), chunk.get(2).copied().unwrap_or(0)];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        for (i, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            let c = if i <= chunk.len() {
                char::from(ALPHABET[(n >> shift & 63) as usize])
            } else {
                '='
            };
            out.push(c);
        }
    }
    out
}

/// A JSON string: quoted, with the quote, the backslash and every control character escaped,
/// and `<` written `<`, so no text can end the script element that holds it.
pub(crate) fn string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '<' => out.push_str("\\u003c"),
            c if u32::from(c) < 0x20 => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A finite number as JSON (Rust's shortest form that reads back to the same value).
pub(crate) fn number(x: f64) -> String {
    debug_assert!(x.is_finite(), "JSON has no {x}");
    format!("{x:?}")
}

/// A JSON array of strings.
pub(crate) fn strings<'a>(items: impl IntoIterator<Item = &'a str>) -> String {
    let items: Vec<String> = items.into_iter().map(string).collect();
    format!("[{}]", items.join(","))
}

/// A JSON array of numbers.
pub(crate) fn numbers(items: &[f64]) -> String {
    let items: Vec<String> = items.iter().map(|&x| number(x)).collect();
    format!("[{}]", items.join(","))
}
