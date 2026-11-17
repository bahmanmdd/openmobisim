//! Reading a written map back: its two script blocks found in the file, the table of contents
//! parsed as JSON, the block decoded from base64 and inflated, each array read by its type. The
//! reader knows the layout of `README.md` and nothing of the encoder.

#![allow(dead_code, reason = "each test binary uses a different part of this module")]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "offsets and counts of the page's arrays, whole numbers far below 2^32; bytes of base64"
)]

use std::collections::BTreeMap;
use std::io::Read as _;

use flate2::read::ZlibDecoder;

/// A JSON value.
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Number(f64),
    Text(String),
    List(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Json {
    /// A member of an object.
    pub fn get(&self, key: &str) -> &Json {
        match self {
            Json::Object(members) => {
                &members.iter().find(|(k, _)| k == key).unwrap_or_else(|| panic!("no {key}")).1
            }
            _ => panic!("not an object"),
        }
    }

    pub fn number(&self) -> f64 {
        match self {
            Json::Number(x) => *x,
            other => panic!("not a number: {other:?}"),
        }
    }

    pub fn text(&self) -> &str {
        match self {
            Json::Text(s) => s,
            other => panic!("not a string: {other:?}"),
        }
    }

    pub fn list(&self) -> &[Json] {
        match self {
            Json::List(items) => items,
            other => panic!("not a list: {other:?}"),
        }
    }
}

/// A JSON parser for the page's table of contents: objects, lists, strings with escapes,
/// numbers, `true`, `false`, `null`.
struct Parser<'a> {
    s: &'a [u8],
    at: usize,
}

impl Parser<'_> {
    fn space(&mut self) {
        while self.at < self.s.len() && self.s[self.at].is_ascii_whitespace() {
            self.at += 1;
        }
    }

    fn eat(&mut self, c: u8) {
        self.space();
        assert_eq!(self.s[self.at], c, "at byte {}", self.at);
        self.at += 1;
    }

    fn value(&mut self) -> Json {
        self.space();
        match self.s[self.at] {
            b'{' => {
                self.at += 1;
                let mut members = Vec::new();
                self.space();
                while self.s[self.at] != b'}' {
                    let Json::Text(key) = self.value() else { panic!("a key") };
                    self.eat(b':');
                    members.push((key, self.value()));
                    self.space();
                    if self.s[self.at] == b',' {
                        self.at += 1;
                        self.space();
                    }
                }
                self.at += 1;
                Json::Object(members)
            }
            b'[' => {
                self.at += 1;
                let mut items = Vec::new();
                self.space();
                while self.s[self.at] != b']' {
                    items.push(self.value());
                    self.space();
                    if self.s[self.at] == b',' {
                        self.at += 1;
                        self.space();
                    }
                }
                self.at += 1;
                Json::List(items)
            }
            b'"' => Json::Text(self.text()),
            b't' => self.word("true", Json::Bool(true)),
            b'f' => self.word("false", Json::Bool(false)),
            b'n' => self.word("null", Json::Null),
            _ => {
                let start = self.at;
                while self.at < self.s.len() && b"+-.eE0123456789".contains(&self.s[self.at]) {
                    self.at += 1;
                }
                let text = std::str::from_utf8(&self.s[start..self.at]).expect("ASCII");
                Json::Number(text.parse().unwrap_or_else(|_| panic!("a number: {text}")))
            }
        }
    }

    fn word(&mut self, w: &str, v: Json) -> Json {
        assert!(self.s[self.at..].starts_with(w.as_bytes()));
        self.at += w.len();
        v
    }

    fn text(&mut self) -> String {
        self.at += 1;
        let mut out = Vec::new();
        while self.s[self.at] != b'"' {
            if self.s[self.at] == b'\\' {
                let e = self.s[self.at + 1];
                self.at += 2;
                match e {
                    b'u' => {
                        let hex = std::str::from_utf8(&self.s[self.at..self.at + 4]).expect("hex");
                        let c = char::from_u32(u32::from_str_radix(hex, 16).expect("hex"))
                            .expect("a character");
                        out.extend(c.to_string().bytes());
                        self.at += 4;
                    }
                    b'n' => out.push(b'\n'),
                    b't' => out.push(b'\t'),
                    b'r' => out.push(b'\r'),
                    other => out.push(other),
                }
            } else {
                out.push(self.s[self.at]);
                self.at += 1;
            }
        }
        self.at += 1;
        String::from_utf8(out).expect("UTF-8")
    }
}

/// Parse JSON text.
pub fn json(text: &str) -> Json {
    let mut p = Parser { s: text.as_bytes(), at: 0 };
    let v = p.value();
    p.space();
    assert_eq!(p.at, text.len(), "trailing text");
    v
}

/// Bytes from base64 (RFC 4648, standard alphabet, `=` padding).
pub fn base64(text: &str) -> Vec<u8> {
    let value = |c: u8| -> u32 {
        match c {
            b'A'..=b'Z' => u32::from(c - b'A'),
            b'a'..=b'z' => u32::from(c - b'a') + 26,
            b'0'..=b'9' => u32::from(c - b'0') + 52,
            b'+' => 62,
            b'/' => 63,
            _ => panic!("not base64: {c}"),
        }
    };
    let bytes = text.trim().as_bytes();
    assert_eq!(bytes.len() % 4, 0, "base64 comes in fours");
    let mut out = Vec::new();
    for quad in bytes.chunks(4) {
        let pad = quad.iter().filter(|&&c| c == b'=').count();
        let n = quad.iter().take(4 - pad).fold(0, |n, &c| n << 6 | value(c)) << (6 * pad);
        let three = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        out.extend_from_slice(&three[..3 - pad]);
    }
    out
}

/// An array of the page's data, by its type.
#[derive(Clone, Debug, PartialEq)]
pub enum Column {
    U8(Vec<u8>),
    U32(Vec<u32>),
    I32(Vec<i32>),
    F32(Vec<f32>),
}

/// A written map read back: its table of contents and its arrays by name.
pub struct Page {
    pub html: String,
    pub meta: Json,
    pub arrays: BTreeMap<String, Column>,
}

impl Page {
    pub fn u8(&self, name: &str) -> Vec<u8> {
        match &self.arrays[name] {
            Column::U8(v) => v.clone(),
            other => panic!("{name} is {other:?}"),
        }
    }

    pub fn u32(&self, name: &str) -> Vec<u32> {
        match &self.arrays[name] {
            Column::U32(v) => v.clone(),
            other => panic!("{name} is {other:?}"),
        }
    }

    pub fn i32(&self, name: &str) -> Vec<i32> {
        match &self.arrays[name] {
            Column::I32(v) => v.clone(),
            other => panic!("{name} is {other:?}"),
        }
    }

    pub fn f32(&self, name: &str) -> Vec<f32> {
        match &self.arrays[name] {
            Column::F32(v) => v.clone(),
            other => panic!("{name} is {other:?}"),
        }
    }

    /// Each held pair's two places' names: the distinct names (`place_names`) by the pair's
    /// indices (`qpo`, `qpd`).
    pub fn places(&self) -> Vec<[String; 2]> {
        let names: Vec<String> =
            self.meta.get("place_names").list().iter().map(|j| j.text().to_owned()).collect();
        let (o, d) = (self.u32("qpo"), self.u32("qpd"));
        o.iter()
            .zip(&d)
            .map(|(&a, &b)| [names[a as usize].clone(), names[b as usize].clone()])
            .collect()
    }

    /// The texts of `name`, a block of UTF-8 texts each followed by a 0 byte.
    pub fn texts(&self, name: &str) -> Vec<String> {
        let bytes = self.u8(name);
        let text = String::from_utf8(bytes).expect("UTF-8");
        let mut out: Vec<String> = text.split('\0').map(str::to_owned).collect();
        assert_eq!(out.pop().as_deref(), Some(""), "each text ends with a 0 byte");
        out
    }

    /// The pairs that use each link by car, bike or walk, with their travellers, read from the
    /// journeys' legs as the page reads them: per link, `[(pair, travellers)]` in pair order.
    pub fn link_users(&self) -> BTreeMap<u32, Vec<(usize, u32)>> {
        let (qs, on, ol, lk) = (self.u32("qs"), self.u32("on"), self.u32("ol"), self.u8("lk"));
        let (la, lb, pl) = (self.u32("la"), self.u32("lb"), self.u32("pl"));
        let mut users: BTreeMap<u32, BTreeMap<usize, u32>> = BTreeMap::new();
        for p in 0..qs.len() - 1 {
            for o in qs[p] as usize..qs[p + 1] as usize {
                for g in ol[o] as usize..ol[o + 1] as usize {
                    if lk[g] <= 2 {
                        for &l in &pl[la[g] as usize..lb[g] as usize] {
                            *users.entry(l).or_default().entry(p).or_default() += on[o];
                        }
                    }
                }
            }
        }
        users.into_iter().map(|(l, by)| (l, by.into_iter().collect())).collect()
    }
}

/// The text between `open` and the next `</script>`.
fn block<'a>(html: &'a str, open: &str) -> &'a str {
    let start = html.find(open).unwrap_or_else(|| panic!("no {open}")) + open.len();
    let end = html[start..].find("</script>").expect("the block's end") + start;
    &html[start..end]
}

/// Read the map written at `path`.
pub fn read(path: &std::path::Path) -> Page {
    let html = std::fs::read_to_string(path).expect("the map");
    let meta = json(block(&html, r#"<script id="meta" type="application/json">"#));
    let packed = base64(block(&html, r#"<script id="blob" type="text/plain">"#));
    let mut raw = Vec::new();
    ZlibDecoder::new(packed.as_slice()).read_to_end(&mut raw).expect("zlib");
    let mut arrays = BTreeMap::new();
    for entry in meta.get("arrays").list() {
        let (name, kind) = (entry.get("n").text(), entry.get("t").text());
        let (at, n) = (entry.get("o").number() as usize, entry.get("c").number() as usize);
        assert_eq!(at % 8, 0, "{name} starts on 8 bytes");
        let four = |i: usize| -> [u8; 4] { raw[at + 4 * i..at + 4 * i + 4].try_into().expect("4") };
        let column = match kind {
            "u8" => Column::U8(raw[at..at + n].to_vec()),
            "u32" => Column::U32((0..n).map(|i| u32::from_le_bytes(four(i))).collect()),
            "i32" => Column::I32((0..n).map(|i| i32::from_le_bytes(four(i))).collect()),
            "f32" => Column::F32((0..n).map(|i| f32::from_le_bytes(four(i))).collect()),
            other => panic!("{name}: type {other}"),
        };
        arrays.insert(name.to_string(), column);
    }
    Page { html, meta, arrays }
}

/// What the pages keep of run `r` of `demand` on `network` and `timetable` with `parameters`:
/// its four tables written by `openmobisim_io_parquet::write` into the folder `name` of the
/// tests' scratch folder, then read back, as the binding reads them. The tests' networks are
/// built at 1 800 veh/h per lane, 15 km/h by bike and 4.8 km/h on foot.
///
/// # Panics
///
/// When the run cannot be written or read back.
pub fn kept(
    r: &openmobisim_loop::run::Run,
    network: &openmobisim_graph::network::Network,
    demand: &openmobisim_demand::demand::Demand,
    timetable: Option<&openmobisim_transit::timetable::Timetable>,
    parameters: &openmobisim_loop::parameters::Parameters,
    name: &str,
) -> openmobisim_map::Kept {
    use openmobisim_io_parquet::{Files, Given, Source, Transit};
    let report = openmobisim_io_gtfs::Report::default();
    let given = Given {
        source: Source::Tables,
        files: Files::default(),
        build_values: openmobisim_graph::network::BuildValues {
            capacity_per_lane: 1_800.0,
            bike_speed: 15.0,
            walk_speed: 4.8,
        },
        stop_walk_max: 1_500.0,
        transit: timetable.map(|tt| Transit {
            timetable: tt,
            report: &report,
            transfer_walk_max: 400.0,
        }),
        parameters,
        network_repairs: network.repairs(),
        trips_given: u64::from(demand.trip_count()),
        demand_repairs: demand.repairs(),
    };
    let folder = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    openmobisim_io_parquet::write(r, network, demand, &given, &folder).expect("written");
    let window = r.links.window();
    let t = openmobisim_io_parquet::read(&folder, demand, network, timetable, window, r.dropped())
        .expect("read back");
    assert!(t.warnings.is_empty(), "{:?}", t.warnings);
    openmobisim_map::Kept::of(demand, &t.trips, t.routes, t.links, window)
}
