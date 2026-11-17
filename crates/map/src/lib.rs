//! The interactive map of a finished run: one self-contained HTML file, and nothing else.
//!
//! [`Kept::of`] takes what the map needs from a run's tables, read back from its files
//! (`openmobisim_io_parquet::read`), and the `Demand` it ran on;
//! [`write`] writes the page from it, the `Network` and the `Timetable` the run ran on. The page
//! holds its data (the arrays in `README.md`, compressed and in base64), its style and its script:
//! it names no address and needs no server, network or library. A second write of the same run
//! gives the same bytes. The display rules are in [`rules`].

use core::fmt;
use std::path::Path;

use openmobisim_graph::geometry::Xy;
use openmobisim_graph::network::Network;
use openmobisim_io_gtfs::{Kind, Lines};
use openmobisim_transit::timetable::Timetable;
use openmobisim_types::ids::{EntityId, StopId};

mod blob;
mod kept;
mod network;
mod pairs;
mod places;
pub mod record;
pub mod rules;
pub mod theme;
mod way;

pub use kept::{Kept, Trip};

use blob::{Blob, number, numbers, string, strings};
use way::Geometry;

/// Why a map was not written. Each names what it refuses.
#[derive(Clone, Debug, PartialEq)]
pub enum MapError {
    /// The run names a link the network does not have: not the run's network.
    NotTheRunsNetwork {
        /// The link, by internal id.
        link: u32,
        /// The network's links.
        links: u32,
    },
    /// A trip took a journey on a timetable and none was given.
    NoTimetable,
    /// A journey rides a run, or calls, the timetable does not have: not the run's timetable.
    NotTheRunsTransit,
    /// A position more than 2³¹ − 1 decimetres (214 748 km) from the network's least x or y.
    TooFar {
        /// How far, metres.
        metres: f64,
    },
    /// The file could not be written: its path and the system's message.
    Io(String),
}

impl fmt::Display for MapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotTheRunsNetwork { link, links } => {
                write!(f, "the run names link {link} and the network has {links} links")
            }
            Self::NoTimetable => write!(f, "a trip took transit and no timetable was given"),
            Self::NotTheRunsTransit => {
                write!(f, "a journey rides a run the timetable does not have")
            }
            Self::TooFar { metres } => {
                write!(f, "a position {metres} m from the network's corner: too far to store")
            }
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

impl core::error::Error for MapError {}

/// The page: its markup, style and script, assembled at build time.
const HTML: &str = include_str!("../page/page.html");
const CSS: &str = include_str!("../page/page.css");
const JS: &str = concat!(
    include_str!("../page/data.js"),
    include_str!("../page/draw.js"),
    include_str!("../page/panels.js"),
    include_str!("../page/start.js"),
);
/// The openmobisim mark: three shapes on a 64-unit square, as one SVG path (every page's).
pub const LOGO: &str = "M32 6a5 5 0 0 1 5 5v42a5 5 0 0 1-10 0V11a5 5 0 0 1 5-5Z\
                    M24 44H21Q13 44 13 36V27a4 4 0 0 1 8 0V36H24Z\
                    M40 36H43Q51 36 51 28V18a4 4 0 0 0-8 0V28H40Z";

/// What every journey the run took rode on its timetable, counted once for every page: the
/// hops (two consecutive stops of a pattern, merged across patterns of the same kind of service),
/// each with its stops, its kind and the travellers who rode it, in kind then stop id order; and
/// the boardings, every ride boarded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ridden {
    /// The hops: from stop, to stop, kind (rail 0 to other 5), travellers.
    pub hops: Vec<(StopId, StopId, u8, u32)>,
    /// The rides boarded.
    pub boardings: u32,
}

/// What the journeys the run `kept` holds took on `timetable` rode ([`Ridden`]). The map's
/// transit layer draws and counts the same.
///
/// # Errors
///
/// [`MapError::NoTimetable`] for a journey without a timetable; [`MapError::NotTheRunsTransit`]
/// for a ride the timetable does not have.
pub fn ridden(
    kept: &Kept,
    timetable: Option<&Timetable>,
    lines: Option<&Lines>,
) -> Result<Ridden, MapError> {
    let ways = pairs::ways(kept, timetable)?;
    let (hops, boarded) = pairs::ridden(&ways, &pairs::Lining::new(timetable, lines));
    Ok(Ridden {
        hops: hops.into_iter().map(|((k, a, b), n)| (a, b, k, n)).collect(),
        boardings: boarded.values().sum(),
    })
}

/// A time of day, `HH:MM`, from seconds of the clock, rounded to the minute (every page's).
#[must_use]
pub fn clock(seconds: u32) -> String {
    let m = (seconds + 30) / 60;
    format!("{:02}:{:02}", m / 60, m % 60)
}

/// The run a map is drawn of: what the pages keep of it, the network and the timetable it ran on,
/// the names beside the timetable (the GTFS reader's [`Lines`]), and its record, `run.toml`.
#[derive(Clone, Copy, Debug)]
pub struct Mapped<'a> {
    /// What the pages keep of the run.
    pub kept: &'a Kept,
    /// The network it ran on.
    pub network: &'a Network,
    /// The timetable it ran on, if any.
    pub timetable: Option<&'a Timetable>,
    /// The stops' names and the runs' lines and kinds, if known.
    pub lines: Option<&'a Lines>,
    /// Its record, as written.
    pub record: &'a str,
}

/// Write the map of `run`, titled `title`, as one HTML file at `path` (replaced if there). The page is written to a sibling
/// file, `.<name>.partial`, then renamed to `path`: a write that fails leaves no part of a page at
/// `path` (and removes the sibling).
///
/// # Errors
///
/// [`MapError`]: a run not of this network or timetable, a position too far to store, a path
/// without a file name, a file that cannot be written or renamed.
pub fn write(run: &Mapped<'_>, title: &str, path: &Path) -> Result<(), MapError> {
    let page = page(run, title)?;
    save(path, &page).map_err(|e| MapError::Io(format!("{}: {e}", path.display())))
}

/// Write `text` to `path` whole or not at all: to a sibling, `.<name>.partial`, then renamed to
/// `path`; a write that fails removes the sibling and leaves `path` as it was. Every page of a
/// run is written so.
///
/// # Errors
///
/// A path without a file name (`InvalidInput`), a sibling that cannot be written, a rename that
/// fails (onto a folder, say).
pub fn save(path: &Path, text: &str) -> std::io::Result<()> {
    let Some(name) = path.file_name() else {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "not a file name"));
    };
    let mut partial = std::ffi::OsString::from(".");
    partial.push(name);
    partial.push(".partial");
    let partial = path.with_file_name(partial);
    let written = std::fs::write(&partial, text).and_then(|()| std::fs::rename(&partial, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    written
}

/// The page's text.
fn page(run: &Mapped<'_>, title: &str) -> Result<String, MapError> {
    let (kept, network, timetable) = (run.kept, run.network, run.timetable);
    let ways = pairs::ways(kept, timetable)?;
    let links = network.link_count();
    for leg in ways.iter().flatten().flat_map(|w| &w.legs) {
        if let Geometry::Links(route) = leg.geometry
            && let Some(l) = route.iter().find(|l| l.raw() >= links)
        {
            return Err(MapError::NotTheRunsNetwork { link: l.raw(), links });
        }
    }
    let origin = network::origin(network);
    let mut blob = Blob::default();
    let road_classes = network::add_network(&mut blob, network, origin)?;
    let rows = network::add_traffic(&mut blob, kept, network)?;
    let mut lining = pairs::Lining::new(timetable, run.lines);
    let stops = pairs::add_transit(&mut blob, &ways, timetable, &lining, origin)?;
    let (doors, all) = pairs::add_pairs(&mut blob, kept, &ways, &stops, &mut lining, origin)?;
    let (stop_names, place_names) = names(run, &stops, &doors, &mut blob);
    let (line_names, line_kinds) = lining.held();
    let kinds: Vec<&str> = Kind::ALL.iter().map(|k| k.as_str()).collect();
    let w = kept.window;
    let meta = vec![
        ("version", string(env!("CARGO_PKG_VERSION"))),
        ("title", string(title)),
        ("n_nodes", network.node_count().to_string()),
        ("n_links", links.to_string()),
        ("n_rows", rows.to_string()),
        ("n_bins", w.n_steps().to_string()),
        ("bin_seconds", w.step_seconds().to_string()),
        ("window_start", w.origin().get().to_string()),
        ("trips", kept.trip_count().to_string()),
        ("left_out", kept.left_out().to_string()),
        ("origin", numbers(&[origin.0, origin.1])),
        ("n_pairs", doors.len().to_string()),
        ("pairs_total", all.to_string()),
        ("stops", stop_names),
        ("kinds", strings(kinds.iter().copied())),
        ("lines", strings(line_names.iter().map(String::as_str))),
        (
            "line_kinds",
            format!("[{}]", line_kinds.iter().map(u8::to_string).collect::<Vec<_>>().join(",")),
        ),
        ("place_names", place_names),
        ("provenance", string(&record::provenance(run.record))),
        ("source", string(record::source(run.record))),
        ("delay_bands", numbers(&rules::DELAY_BANDS)),
        ("delay_band_labels", strings(rules::DELAY_BAND_LABELS)),
        ("vc_bands", numbers(&rules::VC_BANDS)),
        ("vc_band_labels", strings(rules::VC_BAND_LABELS)),
        ("peak_seconds", rules::PEAK_SECONDS.to_string()),
        (
            "road_classes",
            road_classes.map_or("null".into(), |n| strings(n.iter().map(String::as_str))),
        ),
        ("road_tiers", numbers(&rules::ROAD_TIER_WIDTHS)),
        ("road_tier_bounds", numbers(&rules::ROAD_TIER_BOUNDS)),
        ("flow_full", rules::FLOW_FULL.to_string()),
        ("tokens", theme::tokens()),
        ("arrays", blob.table()),
    ];
    let packed = blob.packed();
    // The file's size in MB, as the footer shows it: measured on the page itself, written once with
    // 0 in its place, then with the size found (the same length while under 10 MB).
    let render = |megabytes: f64| {
        let mut meta = meta.clone();
        meta.push(("megabytes", number(megabytes)));
        assemble(title, &meta, &packed)
    };
    #[allow(clippy::cast_precision_loss, reason = "a page's length, far below 2^53")]
    let megabytes = (render(0.0).len() as f64 / 1e5).round() / 10.0;
    Ok(render(megabytes))
}

/// The page: the markup with its title, style, mark, data (`meta`, as keys and JSON values, and
/// the block) and script.
fn assemble(title: &str, meta: &[(&str, String)], packed: &str) -> String {
    let meta: Vec<String> = meta.iter().map(|(k, v)| format!("{}:{v}", string(k))).collect();
    let meta = format!("{{{}}}", meta.join(","));
    let generator = format!("openmobisim {}", env!("CARGO_PKG_VERSION"));
    fill(
        HTML,
        &[
            ("GENERATOR", &generator),
            ("TITLE", &html_text(title)),
            ("CSS", CSS),
            ("LOGO", LOGO),
            ("META", &meta),
            ("BLOB", packed),
            ("JS", &fill(JS, &[("LOGO", LOGO)])),
        ],
    )
}

/// The names the page shows: the held stops' (their GTFS names, else their ids), as JSON; and the
/// held pairs' places ([`places::names`]) as the distinct names, as JSON, in the order first met,
/// and each pair's origin's and destination's place among them (`qpo`, `qpd`, in the block).
fn names(
    run: &Mapped<'_>,
    stops: &pairs::Stops,
    doors: &[(Xy, Xy)],
    blob: &mut Blob,
) -> (String, String) {
    let stop_names: Vec<&str> = run.timetable.map_or(Vec::new(), |tt| {
        stops
            .kept
            .iter()
            .map(|&s| {
                let id = tt.stop_ids().external(s.raw());
                run.lines.and_then(|l| l.stop_name(id)).unwrap_or(id)
            })
            .collect()
    });
    let points: Vec<Xy> = doors.iter().flat_map(|&(o, d)| [o, d]).collect();
    let mut distinct: Vec<String> = Vec::new();
    let mut at: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    let index: Vec<u32> = places::names(run.network, run.timetable, run.lines, &points)
        .into_iter()
        .map(|name| {
            *at.entry(name.clone()).or_insert_with(|| {
                distinct.push(name);
                u32::try_from(distinct.len() - 1).expect("fewer than 2^32 places")
            })
        })
        .collect();
    let (origins, destinations): (Vec<u32>, Vec<u32>) =
        index.chunks(2).map(|p| (p[0], p[1])).unzip();
    blob.u32("qpo", &origins);
    blob.u32("qpd", &destinations);
    (strings(stop_names), strings(distinct.iter().map(String::as_str)))
}

/// `template` with each `__NAME__` of `values` replaced by its value, in one pass over the
/// template alone (a value is never searched again).
fn fill(template: &str, values: &[(&str, &str)]) -> String {
    let mut out =
        String::with_capacity(template.len() + values.iter().map(|v| v.1.len()).sum::<usize>());
    let mut rest = template;
    while let Some(at) = rest.find("__") {
        let found = values.iter().find(|(name, _)| {
            rest[at + 2..].starts_with(name) && rest[at + 2 + name.len()..].starts_with("__")
        });
        match found {
            Some((name, value)) => {
                out.push_str(&rest[..at]);
                out.push_str(value);
                rest = &rest[at + 4 + name.len()..];
            }
            None => {
                out.push_str(&rest[..at + 2]);
                rest = &rest[at + 2..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Text for HTML: `&`, `<`, `>` and `"` escaped (every page's).
#[must_use]
pub fn html_text(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}
