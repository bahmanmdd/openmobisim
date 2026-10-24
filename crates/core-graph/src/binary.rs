//! The binary form of a built network: what a network cache file holds.
//!
//! Importing a city from OpenStreetMap takes seconds; the network it builds is a few flat
//! arrays. A [`Writer`] puts those arrays into bytes and a [`Reader`] takes them back **bit for
//! bit**: every number is stored as its little-endian bit pattern, nothing is rounded or derived
//! again, so the network read back equals the one written, field for field
//! ([`RoadNetwork::write_binary`](crate::RoadNetwork::write_binary); a
//! [`LinkGeometry`](crate::link_geometry::LinkGeometry) and a
//! [`StaticNetwork`](crate::layers::StaticNetwork) the same way), and a run on it has the same
//! fingerprint and the same results as a run on the import.
//!
//! # A cache, not an exchange format
//!
//! A file is valid only for the build that wrote it. Its [`Header`] names the format version
//! ([`FORMAT_VERSION`]), the code version, the defaults table's version
//! ([`DEFAULTS_VERSION`](crate::DEFAULTS_VERSION)), the crate version and the build, and
//! [`Header::differs_from`] says which of them differs from another header: a reader refuses a
//! file of another build rather than guess what it would have built. The formats to keep a
//! network in are its sources (an extract, a GMNS folder, a link table).
//!
//! # Layout
//!
//! The magic bytes `OMSNET\r\n`, the header, the body (each type's fields in a fixed order; a
//! sequence is its length, a `u64`, then its elements; a string is its byte length then its
//! UTF-8 bytes), and a checksum of everything before it (FNV-1a over 8-byte words). On reading,
//! the checksum is checked first, then every length against the count it must equal and every
//! index against the range it points into, so a damaged or foreign file is refused with a
//! [`FormatError`], never read into a network that fails later.
//!
//! **Adding a field** to a type stored here breaks the build until its binary form writes and
//! reads it (each one destructures its type without `..`), and the change bumps
//! [`FORMAT_VERSION`].
//!
//! **Cost:** one pass over the arrays each way, and one over the bytes for the checksum. The
//! external ids are read back through [`ExternalIdTableBuilder`], one string each.
//!
//! [`ExternalIdTableBuilder`]: openmobisim_core_types::ids::ExternalIdTableBuilder

use core::fmt;

use openmobisim_core_types::ids::{ExternalIdTable, ExternalIdTableBuilder};

/// The layout's version: bumped whenever a stored type gains, loses or reorders a field, or the
/// encoding of one changes. A file of another version is refused.
pub const FORMAT_VERSION: u32 = 1;

/// The first eight bytes of every file: a name, then a carriage return and a line feed, which a
/// transfer that rewrites line endings would damage visibly.
pub const MAGIC: [u8; 8] = *b"OMSNET\r\n";

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// The checksum a file ends with: FNV-1a over little-endian 8-byte words, then the bytes left
/// over, then the length. Each step is a bijection of the running value, so a change confined to
/// one word always changes it; it detects a damaged or truncated file and is no defence against a
/// deliberate change.
#[must_use]
pub fn checksum(bytes: &[u8]) -> u64 {
    let mut h = FNV_OFFSET;
    let mut words = bytes.chunks_exact(8);
    for word in &mut words {
        let mut w = [0u8; 8];
        w.copy_from_slice(word);
        h = (h ^ u64::from_le_bytes(w)).wrapping_mul(FNV_PRIME);
    }
    for &b in words.remainder() {
        h = (h ^ u64::from(b)).wrapping_mul(FNV_PRIME);
    }
    (h ^ bytes.len() as u64).wrapping_mul(FNV_PRIME)
}

/// Why bytes could not be read as a network.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FormatError {
    /// The bytes do not start with [`MAGIC`]: not a network file.
    NotANetworkFile,
    /// The checksum does not match: the file was damaged or cut short.
    Damaged,
    /// The checksum matches but the content is not what this version reads: a length that is
    /// not its count, an index out of range, an unknown code. Names what.
    Invalid(String),
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FormatError::NotANetworkFile => f.write_str("not a network file"),
            FormatError::Damaged => f.write_str("the file is damaged or incomplete"),
            FormatError::Invalid(what) => write!(f, "the file's content is not valid: {what}"),
        }
    }
}

impl core::error::Error for FormatError {}

/// A shorthand for a reader's results.
pub type Result<T> = core::result::Result<T, FormatError>;

/// A [`FormatError::Invalid`] naming `what`: for a reader of a type stored here.
///
/// # Errors
///
/// Always.
pub fn invalid<T>(what: impl Into<String>) -> Result<T> {
    Err(FormatError::Invalid(what.into()))
}

/// Writes values as little-endian bytes. See the [module docs](self).
#[derive(Debug)]
pub struct Writer {
    buf: Vec<u8>,
}

impl Default for Writer {
    fn default() -> Self {
        Self::new()
    }
}

impl Writer {
    /// A writer holding the magic bytes.
    #[must_use]
    pub fn new() -> Self {
        let mut buf = Vec::with_capacity(1 << 20);
        buf.extend_from_slice(&MAGIC);
        Self { buf }
    }

    /// One byte.
    pub fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }

    /// A flag, as one byte (0 or 1).
    pub fn bool(&mut self, v: bool) {
        self.buf.push(u8::from(v));
    }

    /// A `u32`.
    pub fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    /// A `u64`.
    pub fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    /// An `i64`.
    pub fn i64(&mut self, v: i64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    /// An `f64`, by its bit pattern: `-0.0`, infinities and every NaN come back as written.
    pub fn f64(&mut self, v: f64) {
        self.u64(v.to_bits());
    }

    /// A sequence's length, as a `u64`.
    pub fn len(&mut self, n: usize) {
        self.u64(n as u64);
    }

    /// A string: its byte length, then its bytes.
    pub fn str(&mut self, s: &str) {
        self.len(s.len());
        self.buf.extend_from_slice(s.as_bytes());
    }

    /// An optional `f64`: a flag, then the value if there is one.
    pub fn opt_f64(&mut self, v: Option<f64>) {
        self.bool(v.is_some());
        if let Some(v) = v {
            self.f64(v);
        }
    }

    /// A sequence of `f64`: its length, then each value.
    pub fn f64s(&mut self, values: impl ExactSizeIterator<Item = f64>) {
        self.len(values.len());
        self.buf.reserve(values.len() * 8);
        for v in values {
            self.f64(v);
        }
    }

    /// A sequence of `u32`: its length, then each value.
    pub fn u32s(&mut self, values: impl ExactSizeIterator<Item = u32>) {
        self.len(values.len());
        self.buf.reserve(values.len() * 4);
        for v in values {
            self.u32(v);
        }
    }

    /// A sequence of bytes: its length, then each byte.
    pub fn u8s(&mut self, values: impl ExactSizeIterator<Item = u8>) {
        self.len(values.len());
        self.buf.extend(values);
    }

    /// A sequence of flags, one byte each.
    pub fn bools(&mut self, values: &[bool]) {
        self.u8s(values.iter().map(|&b| u8::from(b)));
    }

    /// An external id table: its length, then each id in internal-id order (sorted, as the
    /// table holds them).
    pub fn ids(&mut self, table: &ExternalIdTable) {
        self.len(table.len());
        for id in table.iter() {
            self.str(id);
        }
    }

    /// The bytes, with the checksum appended.
    #[must_use]
    pub fn finish(mut self) -> Vec<u8> {
        let sum = checksum(&self.buf);
        self.buf.extend_from_slice(&sum.to_le_bytes());
        self.buf
    }
}

/// Reads what a [`Writer`] wrote, checking as it goes. See the [module docs](self).
#[derive(Debug)]
pub struct Reader<'a> {
    /// The bytes between the magic and the checksum.
    body: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    /// A reader of `bytes`, once the magic and the checksum are checked.
    ///
    /// # Errors
    ///
    /// [`FormatError::NotANetworkFile`] without the magic bytes, [`FormatError::Damaged`] if
    /// the checksum does not match.
    pub fn new(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < MAGIC.len() || bytes[..MAGIC.len()] != MAGIC {
            return Err(FormatError::NotANetworkFile);
        }
        if bytes.len() < MAGIC.len() + 8 {
            return Err(FormatError::Damaged);
        }
        let (content, sum) = bytes.split_at(bytes.len() - 8);
        let mut stored = [0u8; 8];
        stored.copy_from_slice(sum);
        if checksum(content) != u64::from_le_bytes(stored) {
            return Err(FormatError::Damaged);
        }
        Ok(Self { body: &content[MAGIC.len()..], at: 0 })
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.at.checked_add(n).filter(|&e| e <= self.body.len());
        let Some(end) = end else { return invalid("the file ends before its content does") };
        let out = &self.body[self.at..end];
        self.at = end;
        Ok(out)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut out = [0u8; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    /// One byte.
    ///
    /// # Errors
    ///
    /// [`FormatError::Invalid`] past the end, as every read here.
    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    /// A flag.
    ///
    /// # Errors
    ///
    /// Past the end, or a byte other than 0 or 1.
    pub fn bool(&mut self) -> Result<bool> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            other => invalid(format!("a flag of {other}")),
        }
    }

    /// A `u32`.
    ///
    /// # Errors
    ///
    /// Past the end.
    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    /// A `u64`.
    ///
    /// # Errors
    ///
    /// Past the end.
    pub fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    /// An `i64`.
    ///
    /// # Errors
    ///
    /// Past the end.
    pub fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_le_bytes(self.array()?))
    }

    /// An `f64`, by its bit pattern.
    ///
    /// # Errors
    ///
    /// Past the end.
    pub fn f64(&mut self) -> Result<f64> {
        Ok(f64::from_bits(self.u64()?))
    }

    /// A sequence's length, checked against the bytes left for elements of `element_bytes`
    /// each, so that a wrong length is refused before anything is allocated for it.
    ///
    /// # Errors
    ///
    /// Past the end, or a length the rest of the file cannot hold.
    pub fn len(&mut self, element_bytes: usize) -> Result<usize> {
        let n = self.u64()?;
        let left = (self.body.len() - self.at) as u64;
        if n.checked_mul(element_bytes.max(1) as u64).is_none_or(|bytes| bytes > left) {
            return invalid(format!("a sequence of {n} elements in {left} bytes"));
        }
        usize::try_from(n).or_else(|_| invalid("a sequence too long for this machine"))
    }

    /// A string.
    ///
    /// # Errors
    ///
    /// Past the end, or bytes that are not UTF-8.
    pub fn str(&mut self) -> Result<&'a str> {
        let n = self.len(1)?;
        core::str::from_utf8(self.take(n)?).or_else(|_| invalid("a string that is not UTF-8"))
    }

    /// An optional `f64`.
    ///
    /// # Errors
    ///
    /// Past the end, or a bad flag.
    pub fn opt_f64(&mut self) -> Result<Option<f64>> {
        Ok(if self.bool()? { Some(self.f64()?) } else { None })
    }

    /// A sequence of `f64`.
    ///
    /// # Errors
    ///
    /// Past the end.
    pub fn f64s(&mut self) -> Result<Vec<f64>> {
        let n = self.len(8)?;
        let bytes = self.take(n * 8)?;
        Ok(bytes
            .chunks_exact(8)
            .map(|c| {
                let mut w = [0u8; 8];
                w.copy_from_slice(c);
                f64::from_bits(u64::from_le_bytes(w))
            })
            .collect())
    }

    /// A sequence of `u32`.
    ///
    /// # Errors
    ///
    /// Past the end.
    pub fn u32s(&mut self) -> Result<Vec<u32>> {
        let n = self.len(4)?;
        let bytes = self.take(n * 4)?;
        Ok(bytes
            .chunks_exact(4)
            .map(|c| {
                let mut w = [0u8; 4];
                w.copy_from_slice(c);
                u32::from_le_bytes(w)
            })
            .collect())
    }

    /// A sequence of bytes.
    ///
    /// # Errors
    ///
    /// Past the end.
    pub fn u8s(&mut self) -> Result<&'a [u8]> {
        let n = self.len(1)?;
        self.take(n)
    }

    /// A sequence of flags.
    ///
    /// # Errors
    ///
    /// Past the end, or a byte other than 0 or 1.
    pub fn bools(&mut self) -> Result<Vec<bool>> {
        self.u8s()?
            .iter()
            .map(|&b| match b {
                0 => Ok(false),
                1 => Ok(true),
                other => invalid(format!("a flag of {other}")),
            })
            .collect()
    }

    /// An external id table, its ids checked to be strictly increasing (the order the table
    /// keeps, so the internal ids are those written).
    ///
    /// # Errors
    ///
    /// Past the end, or ids out of order or repeated.
    pub fn ids(&mut self) -> Result<ExternalIdTable> {
        let n = self.len(8)?;
        let mut builder = ExternalIdTableBuilder::with_capacity(n);
        let mut previous: Option<&str> = None;
        for _ in 0..n {
            let id = self.str()?;
            if previous.is_some_and(|p| p >= id) {
                return invalid(format!("external id {id:?} out of order"));
            }
            builder.insert(id);
            previous = Some(id);
        }
        Ok(builder.build())
    }

    /// Whether every byte has been read.
    ///
    /// # Errors
    ///
    /// [`FormatError::Invalid`] if bytes are left: the file holds more than this version
    /// reads.
    pub fn finish(self) -> Result<()> {
        if self.at == self.body.len() {
            Ok(())
        } else {
            invalid(format!("{} bytes left after the content", self.body.len() - self.at))
        }
    }
}

/// What a file says about itself, before its content: the versions it was written by, the
/// build, and what the network was made from.
#[derive(Clone, Debug, PartialEq)]
pub struct Header {
    /// [`FORMAT_VERSION`] of the writer.
    pub format_version: u32,
    /// The code version of the writer (`openmobisim_core_types::CODE_VERSION`).
    pub code_version: u32,
    /// The defaults table's version of the writer ([`crate::DEFAULTS_VERSION`]).
    pub defaults_version: u32,
    /// The crate version of the writer.
    pub crate_version: String,
    /// The build that wrote it, as the writer names it (its commit, and whatever else tells
    /// one build from another): an import by another build may differ, so it is compared.
    pub build: String,
    /// What the network was made from, as the writer describes it (a checksum of its source
    /// files and reading options): compared by whoever knows what it should be; empty if not
    /// said.
    pub source: String,
    /// How long making the network took when it was made (the import), in seconds; NaN if not
    /// known. Informative only.
    pub made_seconds: f64,
}

impl Header {
    /// This build's header, for a network made from `source` in `made_seconds`.
    #[must_use]
    pub fn current(build: &str, source: &str, made_seconds: f64) -> Self {
        Self {
            format_version: FORMAT_VERSION,
            code_version: openmobisim_core_types::CODE_VERSION,
            defaults_version: crate::DEFAULTS_VERSION,
            crate_version: openmobisim_core_types::VERSION.to_string(),
            build: build.to_string(),
            source: source.to_string(),
            made_seconds,
        }
    }

    /// Write the header.
    pub fn write(&self, w: &mut Writer) {
        let Header {
            format_version,
            code_version,
            defaults_version,
            crate_version,
            build,
            source,
            made_seconds,
        } = self;
        w.u32(*format_version);
        w.u32(*code_version);
        w.u32(*defaults_version);
        w.str(crate_version);
        w.str(build);
        w.str(source);
        w.f64(*made_seconds);
    }

    /// Read a header. The format version comes first, and a file of another one is refused
    /// before anything else of it is read.
    ///
    /// # Errors
    ///
    /// [`FormatError::Invalid`] for another format version, or past the end.
    pub fn read(r: &mut Reader<'_>) -> Result<Self> {
        let format_version = r.u32()?;
        if format_version != FORMAT_VERSION {
            return invalid(format!(
                "format version {format_version}; this version reads {FORMAT_VERSION}"
            ));
        }
        Ok(Self {
            format_version,
            code_version: r.u32()?,
            defaults_version: r.u32()?,
            crate_version: r.str()?.to_string(),
            build: r.str()?.to_string(),
            source: r.str()?.to_string(),
            made_seconds: r.f64()?,
        })
    }

    /// The header of a file from its first bytes alone (`bytes` may be the whole file or only
    /// its beginning), **without checking the file**: to decide whether to read it at all.
    /// Reading the file ([`Reader::new`], then [`Self::read`]) checks it.
    ///
    /// # Errors
    ///
    /// [`FormatError::NotANetworkFile`] without the magic bytes; [`FormatError::Invalid`] for
    /// another format version, or bytes that end inside the header.
    pub fn peek(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < MAGIC.len() || bytes[..MAGIC.len()] != MAGIC {
            return Err(FormatError::NotANetworkFile);
        }
        Self::read(&mut Reader { body: &bytes[MAGIC.len()..], at: 0 })
    }

    /// What differs between this header and `other` among the format, code and defaults
    /// versions, the crate version and the build (not the source, which only the caller can
    /// judge), as a phrase naming the first difference; `None` if nothing does.
    #[must_use]
    pub fn differs_from(&self, other: &Header) -> Option<String> {
        let pairs = [
            ("format version", self.format_version.to_string(), other.format_version.to_string()),
            ("code version", self.code_version.to_string(), other.code_version.to_string()),
            (
                "defaults version",
                self.defaults_version.to_string(),
                other.defaults_version.to_string(),
            ),
            ("openmobisim version", self.crate_version.clone(), other.crate_version.clone()),
            ("build", self.build.clone(), other.build.clone()),
        ];
        pairs
            .into_iter()
            .find(|(_, a, b)| a != b)
            .map(|(what, a, b)| format!("{what} {a}, not {b}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_come_back_bit_for_bit() {
        let mut w = Writer::new();
        w.u8(7);
        w.bool(true);
        w.u32(u32::MAX);
        w.u64(1 << 60);
        w.i64(-5);
        for v in [-0.0, f64::INFINITY, f64::NAN, 1.0 / 3.0] {
            w.f64(v);
        }
        w.str("way/7:1");
        w.opt_f64(None);
        w.opt_f64(Some(2.5));
        w.f64s([1.5, -2.0].into_iter());
        w.u32s([3, 4, 5].into_iter());
        w.bools(&[true, false]);
        let bytes = w.finish();
        let mut r = Reader::new(&bytes).unwrap();
        assert_eq!(r.u8().unwrap(), 7);
        assert!(r.bool().unwrap());
        assert_eq!(r.u32().unwrap(), u32::MAX);
        assert_eq!(r.u64().unwrap(), 1 << 60);
        assert_eq!(r.i64().unwrap(), -5);
        for v in [-0.0f64, f64::INFINITY, f64::NAN, 1.0 / 3.0] {
            assert_eq!(r.f64().unwrap().to_bits(), v.to_bits());
        }
        assert_eq!(r.str().unwrap(), "way/7:1");
        assert_eq!(r.opt_f64().unwrap(), None);
        assert_eq!(r.opt_f64().unwrap(), Some(2.5));
        assert_eq!(r.f64s().unwrap(), vec![1.5, -2.0]);
        assert_eq!(r.u32s().unwrap(), vec![3, 4, 5]);
        assert_eq!(r.bools().unwrap(), vec![true, false]);
        r.finish().unwrap();
    }

    #[test]
    fn a_damaged_or_foreign_file_is_refused() {
        let mut w = Writer::new();
        w.f64s([1.0, 2.0, 3.0].into_iter());
        let bytes = w.finish();
        // One bit flipped anywhere: the checksum catches it.
        for i in [MAGIC.len(), bytes.len() / 2, bytes.len() - 9] {
            let mut bad = bytes.clone();
            bad[i] ^= 0x10;
            assert_eq!(Reader::new(&bad).unwrap_err(), FormatError::Damaged, "byte {i}");
        }
        // Cut short.
        assert_eq!(Reader::new(&bytes[..bytes.len() - 3]).unwrap_err(), FormatError::Damaged);
        // Not a network file at all.
        assert_eq!(Reader::new(b"PAR1....").unwrap_err(), FormatError::NotANetworkFile);
    }

    #[test]
    fn a_length_the_file_cannot_hold_is_refused_before_allocating() {
        let mut w = Writer::new();
        w.len(usize::MAX / 2);
        let bytes = w.finish();
        let mut r = Reader::new(&bytes).unwrap();
        assert!(matches!(r.f64s(), Err(FormatError::Invalid(_))));
    }

    #[test]
    fn a_header_of_another_build_or_version_says_what_differs() {
        let here = Header::current("abc", "source", 1.5);
        let mut w = Writer::new();
        here.write(&mut w);
        let bytes = w.finish();
        let mut r = Reader::new(&bytes).unwrap();
        let back = Header::read(&mut r).unwrap();
        r.finish().unwrap();
        assert_eq!(back, here);
        assert_eq!(back.differs_from(&here), None);
        let other = Header::current("def", "source", 0.0);
        assert_eq!(here.differs_from(&other).as_deref(), Some("build abc, not def"));
        let older = Header { defaults_version: 1, ..here.clone() };
        assert!(older.differs_from(&here).unwrap().starts_with("defaults version 1"));

        // Peeked from the file's first bytes alone, the checksum not read.
        assert_eq!(Header::peek(&bytes[..bytes.len() - 8]).unwrap(), here);
        assert_eq!(Header::peek(b"PAR1").unwrap_err(), FormatError::NotANetworkFile);
        assert!(matches!(Header::peek(&bytes[..12]), Err(FormatError::Invalid(_))));

        let mut w = Writer::new();
        Header { format_version: FORMAT_VERSION + 1, ..here }.write(&mut w);
        let bytes = w.finish();
        let err = Header::read(&mut Reader::new(&bytes).unwrap()).unwrap_err();
        assert!(matches!(err, FormatError::Invalid(m) if m.contains("format version")));
    }
}
