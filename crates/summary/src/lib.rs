//! The summary of a finished run: one self-contained HTML page of its numbers, and nothing else.
//!
//! [`write`] takes the run as [`Summarised`] (what the map keeps of it, the network and the
//! timetable it ran on, its iterations table, whether the stopping rule ended it, its record
//! `run.toml`) and writes the page: headline tiles, the modes, when people left, the roads,
//! transit, bikes and walking, the convergence, the computation time with the network's size, the
//! record whole, and a map per mode. Its colours are the map's (`openmobisim_map::theme`), light
//! and dark following the reader's system, with the map's Paper / Night button. It names no
//! address and loads nothing; a second write of the same run gives the same bytes.

use core::fmt;
use std::path::Path;

use openmobisim_graph::network::Network;
use openmobisim_io_gtfs::Lines;
use openmobisim_loop::assess::Iteration;
use openmobisim_map::theme::{NIGHT, PAPER, Theme};
use openmobisim_map::{Kept, LOGO, MapError, html_text as esc};
use openmobisim_transit::timetable::Timetable;

mod html;
mod maps;
mod sections;

use html::count;

/// The run a summary is written of.
#[derive(Clone, Copy, Debug)]
pub struct Summarised<'a> {
    /// What the pages keep of the run.
    pub kept: &'a Kept,
    /// The network it ran on.
    pub network: &'a Network,
    /// The timetable it ran on, if any.
    pub timetable: Option<&'a Timetable>,
    /// The stops' names and the runs' lines and kinds (the GTFS reader's side table), if known.
    pub lines: Option<&'a Lines>,
    /// Its iterations table.
    pub iterations: &'a [Iteration],
    /// Whether the stopping rule ended it (else the iteration cap); `None` when its record
    /// does not say.
    pub stopped_by_rule: Option<bool>,
    /// Its record, `run.toml`, as written.
    pub record: &'a str,
}

/// Why a summary was not written.
#[derive(Clone, Debug, PartialEq)]
pub enum SummaryError {
    /// The run's journeys are not of this timetable (the map's refusal).
    Map(MapError),
    /// The file could not be written: its path and the system's message.
    Io(String),
}

impl fmt::Display for SummaryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Map(e) => write!(f, "{e}"),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

impl core::error::Error for SummaryError {}

/// The page's style and its one script, the theme button.
const CSS: &str = include_str!("../page/summary.css");
const SCRIPT: &str = include_str!("../page/theme.js");

/// Write the summary of `run`, titled `title`, as one HTML file at `path` (replaced if there),
/// through a partial sibling renamed, as the map is (`openmobisim_map::save`).
///
/// # Errors
///
/// [`SummaryError`]: journeys not of the timetable; a file that cannot be written.
pub fn write(run: &Summarised<'_>, title: &str, path: &Path) -> Result<(), SummaryError> {
    let page = page(run, title)?;
    openmobisim_map::save(path, &page)
        .map_err(|e| SummaryError::Io(format!("{}: {e}", path.display())))
}

/// A theme's colours as CSS custom properties.
fn properties(t: &Theme) -> String {
    let bands = t.bands();
    format!(
        "--surface:{};--ink:{};--ink2:{};--muted:{};--base:{};--mark:{};--car:{};--bike:{};\
         --walk:{};--transit:{};--b0:{};--b1:{};--b2:{};--b3:{};--k0:{};--k1:{};--k2:{};--k3:{};\
         --k4:{};--k5:{};",
        t.surface,
        t.ink,
        t.ink2,
        t.muted,
        t.base,
        t.mark,
        t.modes[0],
        t.modes[1],
        t.modes[2],
        t.modes[3],
        bands[0],
        bands[1],
        bands[2],
        bands[3],
        t.kinds[0],
        t.kinds[1],
        t.kinds[2],
        t.kinds[3],
        t.kinds[4],
        t.kinds[5]
    )
}

/// The themes: paper by default, night when the reader's system is dark (unless paper is chosen),
/// or when night is chosen with the button.
fn themes() -> String {
    format!(
        ":root{{{p}}}@media (prefers-color-scheme: dark){{:root:not([data-theme=\"paper\"]){{{n}}}}}\
         :root[data-theme=\"night\"]{{{n}}}",
        p = properties(&PAPER),
        n = properties(&NIGHT)
    )
}

/// A section: its heading and its body; nothing for an empty body.
fn section(title: &str, body: &str) -> String {
    if body.is_empty() {
        String::new()
    } else {
        format!("<section><h2>{title}</h2>{body}</section>")
    }
}

/// The page's text.
fn page(run: &Summarised<'_>, title: &str) -> Result<String, SummaryError> {
    let ridden =
        openmobisim_map::ridden(run.kept, run.timetable, run.lines).map_err(SummaryError::Map)?;
    let roads = sections::roads(run);
    let version = env!("CARGO_PKG_VERSION");
    let meta = format!(
        "openmobisim {version} · window {} · {} trips",
        sections::window(run),
        count(run.kept.trip_count())
    );
    let roads_html =
        if roads.vkm > 0.0 { sections::on_the_roads(run, &roads) } else { String::new() };
    let body = [
        format!(
            "<div class=\"grid2\">{}{}</div>",
            section("Modes", &sections::modes(run)),
            section("When people left", &sections::departures(run))
        ),
        section("On the roads", &roads_html),
        section("Transit, bikes and walking", &sections::services(run, &ridden)),
        section("Convergence", &sections::convergence(run)),
        format!(
            "<div class=\"grid2\">{}{}</div>",
            section("Computation time", &sections::computation(run)),
            section("Settings", &format!("<pre class=\"record\">{}</pre>", esc(run.record)))
        ),
        maps::maps(run, &roads, &ridden.hops),
    ]
    .concat();
    Ok(format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" \
         content=\"width=device-width,initial-scale=1\"><meta name=\"generator\" content=\"openmobisim \
         {version}\"><title>{t}</title><style>{themes}{CSS}</style></head><body data-state=\"loading\">\
         <main><header><div><svg class=\"mark\" viewBox=\"0 0 64 64\" aria-hidden=\"true\"><path \
         d=\"{LOGO}\"/></svg><h1>{t}</h1></div><button id=\"theme\" type=\"button\">Night</button>\
         </header><div class=\"meta\">{meta}</div>{tiles}{body}<footer>Every number here comes from \
         the run's files (trips.parquet, links.parquet, iterations.parquet, routes.parquet and run.toml) with the \
         network's links, except transit's boardings and rides, which come from the journeys taken \
         (as on the map).{left_out}{source}</footer></main><script>{SCRIPT}</script></body></html>",
        t = esc(title),
        themes = themes(),
        tiles = sections::tiles(run, &roads),
        left_out = match run.kept.left_out() {
            0 => String::new(),
            n => format!(
                "<br><span data-k=\"left-out\">{}</span> {} left out of the journeys: their \
                 routes do not agree with the trips table.",
                count(n),
                if n == 1 { "trip" } else { "trips" }
            ),
        },
        source = match openmobisim_map::record::source(run.record) {
            "" => String::new(),
            s => format!("<br>{s}"),
        },
    ))
}
