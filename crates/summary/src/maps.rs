//! The maps at the end of the page, static inline SVG drawn here: one per mode that has travellers
//! (car traffic by delay band with widths by capacity, cyclists, walkers, transit hops by riders
//! in their kind of service's colour, as on the map),
//! on the base network defined once and reused. Links follow their shapes (straight between their
//! nodes without one), each direction on the right of its travel, mitred at a bend as on the map;
//! coordinates are whole units of a frame 2 000 wide; one path per colour and width.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use openmobisim_graph::network::Network;
use openmobisim_io_gtfs::Kind;
use openmobisim_map::rules::{DELAY_BAND_LABELS, DELAY_BANDS, FLOW_FULL, delay, flow_share};
use openmobisim_types::ids::{EntityId, LinkId, NodeId, StopId};
use openmobisim_types::mode::Mode;

use crate::Summarised;
use crate::html::swatch;
use crate::sections::Roads;

/// The frame's width and margin, units.
const WIDTH: f64 = 2_000.0;
const MARGIN: f64 = 20.0;

/// From metres in the network's plane to whole units of the frame, y down.
struct Frame {
    x0: f64,
    y1: f64,
    scale: f64,
    height: f64,
}

impl Frame {
    fn of(network: &Network) -> Self {
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        let nodes = (0..network.node_count()).map(|i| network.node_position(NodeId::new(i)));
        let shapes = (0..network.link_count()).flat_map(|l| network.shape(LinkId::new(l)));
        for p in nodes.chain(shapes.copied()) {
            (x0, y0, x1, y1) = (x0.min(p.x), y0.min(p.y), x1.max(p.x), y1.max(p.y));
        }
        // The width fills the frame; a network much taller than wide is drawn at half its height.
        let scale = (WIDTH - 2.0 * MARGIN) / (x1 - x0).max((y1 - y0) / 2.0).max(1.0);
        let height = ((y1 - y0) * scale + 2.0 * MARGIN).max(2.0 * MARGIN + 40.0).ceil();
        Self { x0, y1, scale, height }
    }

    /// A point in the frame, unrounded.
    fn at(&self, (x, y): (f64, f64)) -> (f64, f64) {
        (MARGIN + (x - self.x0) * self.scale, MARGIN + (self.y1 - y) * self.scale)
    }

    /// The line through `points` (metres) moved `off` units to the right of its travel, as path
    /// data in whole units: each point moved along its two segments' normals averaged, as far out
    /// at a bend as keeps both segments `off` from their line (at most twice `off`); a U-turn
    /// keeps the last segment's normal (the map's `offsetLine`).
    #[allow(clippy::cast_possible_truncation, reason = "whole units of a frame 2 000 wide")]
    fn line(&self, points: &[(f64, f64)], off: f64, out: &mut String) {
        let p: Vec<(f64, f64)> = points.iter().map(|&q| self.at(q)).collect();
        let n = p.len();
        let (mut normal, mut last) = (vec![(0.0, 0.0); n], vec![(0.0, 0.0); n]);
        for i in 0..n.saturating_sub(1) {
            let (dx, dy) = (p[i + 1].0 - p[i].0, p[i + 1].1 - p[i].1);
            let m = dx.hypot(dy);
            if m < 1e-6 {
                continue;
            }
            let u = (-dy / m, dx / m);
            normal[i] = (normal[i].0 + u.0, normal[i].1 + u.1);
            normal[i + 1] = (normal[i + 1].0 + u.0, normal[i + 1].1 + u.1);
            if last[i] == (0.0, 0.0) {
                last[i] = u;
            }
            last[i + 1] = u;
        }
        let r = |v: f64| v.round() as i64;
        for i in 0..n {
            let (mut a, mut m, mut k) = (normal[i], normal[i].0.hypot(normal[i].1), 1.0);
            if m < 0.35 {
                a = last[i];
                m = a.0.hypot(a.1).max(f64::MIN_POSITIVE);
            } else if i > 0 && i < n - 1 {
                k = (2.0 / m).min(2.0);
            }
            let q = (p[i].0 + a.0 / m * off * k, p[i].1 + a.1 / m * off * k);
            let _ = write!(out, "{}{} {}", if i == 0 { 'M' } else { 'L' }, r(q.0), r(q.1));
        }
    }
}

/// A link's points in metres, in order of travel: its upstream end, its shape, its downstream end.
fn points(network: &Network, link: LinkId) -> Vec<(f64, f64)> {
    let at = |p: openmobisim_graph::geometry::Xy| (p.x, p.y);
    let mut out = vec![at(network.node_position(network.upstream(link)))];
    out.extend(network.shape(link).iter().map(|&p| at(p)));
    out.push(at(network.node_position(network.downstream(link))));
    out
}

/// A width, in half units.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a few units")]
fn half_units(w: f64) -> u32 {
    (w * 2.0).round() as u32
}

/// Paths grouped by (colour class, width in half units, dashes), drawn thinnest first.
type Groups = BTreeMap<(u32, String, String), String>;

fn paths(groups: &Groups) -> String {
    groups
        .iter()
        .map(|((w, class, dash), d)| {
            let dash = if dash.is_empty() {
                String::new()
            } else {
                format!(" stroke-dasharray=\"{dash}\"")
            };
            format!(
                "<path class=\"{class}\" stroke-width=\"{}\"{dash} d=\"{d}\"/>",
                f64::from(*w) / 2.0
            )
        })
        .collect()
}

/// One map: the base network, then `groups`, and a note under it.
fn map(frame: &Frame, title: &str, key: &str, groups: &Groups, note: &str) -> String {
    format!(
        "<section><h2>{title}</h2><svg viewBox=\"0 0 {WIDTH} {}\" class=\"map\" data-k=\"map.{key}\" \
         role=\"img\" aria-label=\"{title}\"><use href=\"#net\"/>{}</svg><p class=\"note\">{note}</p>\
         </section>",
        frame.height,
        paths(groups)
    )
}

/// The base network, every line once whatever its direction and modes (two links between the same
/// nodes along the same points are one line): defined once, reused by every map.
fn base(frame: &Frame, network: &Network) -> String {
    let lines: BTreeSet<((u32, u32), String)> = (0..network.link_count())
        .map(LinkId::new)
        .map(|l| {
            let (u, d) = (network.upstream(l).raw(), network.downstream(l).raw());
            let mut p = points(network, l);
            if u > d {
                p.reverse();
            }
            let mut path = String::new();
            frame.line(&p, 0.0, &mut path);
            ((u.min(d), u.max(d)), path)
        })
        .collect();
    let d: String = lines.into_iter().map(|(_, path)| path).collect();
    format!(
        "<svg class=\"defs\" width=\"0\" height=\"0\" aria-hidden=\"true\"><defs><path id=\"net\" \
         d=\"{d}\"/></defs></svg>"
    )
}

/// Car traffic over the window: each link with cars in its delay band (its cars' mean crossing
/// time over the free-flow time, less one), as wide as its capacity.
fn cars(frame: &Frame, s: &Summarised<'_>, roads: &Roads) -> Groups {
    let net = s.network;
    let mut caps: Vec<f64> = (0..net.link_count())
        .map(LinkId::new)
        .filter(|&l| net.modes(l).contains(Mode::Car))
        .map(|l| net.capacity(l).as_veh_per_hour())
        .filter(|c| c.is_finite() && *c > 0.0)
        .collect();
    caps.sort_by(f64::total_cmp);
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "an index into the capacities"
    )]
    let reference =
        caps.get(((caps.len() as f64) * 0.99) as usize).or(caps.last()).copied().unwrap_or(1.0);
    let mut groups = Groups::new();
    for (&l, &(n, time)) in &roads.links {
        let free = net.free_flow_time(l, Mode::Car).get();
        let band = DELAY_BANDS.iter().filter(|&&e| delay(time / f64::from(n), free) >= e).count();
        let c = net.capacity(l).as_veh_per_hour();
        let w = 2.0 + 6.0 * (if c.is_finite() { c / reference } else { 1.0 }).min(1.0).sqrt();
        let w = half_units(w);
        let d = groups.entry((w, format!("k-b{band}"), String::new())).or_default();
        frame.line(&points(net, l), f64::from(w) / 4.0 + 1.0, d);
    }
    groups
}

/// The width of a cyclists' or walkers' line of `n` travellers, units: from 1.5, the full 9.5 at
/// the map's `FLOW_FULL` and more.
fn flow_width(n: u32) -> f64 {
    1.5 + 8.0 * flow_share(n)
}

/// Bikes or walkers over the window: each link with any, as wide as their count (from 1.5 units, the
/// full 9.5 at the map's `FLOW_FULL` and more, the same for both), dashed (bikes) or dotted
/// (walkers); a link whose other direction allows the mode on its own side.
fn active(frame: &Frame, s: &Summarised<'_>, mode: Mode) -> Groups {
    let (net, r) = (s.network, s.kept.rows());
    let mut per: BTreeMap<LinkId, u32> = BTreeMap::new();
    for i in 0..r.link.len() {
        let n = if mode == Mode::Bike { r.bikes[i] } else { r.walkers[i] };
        if n > 0 {
            *per.entry(r.link[i]).or_default() += n;
        }
    }
    let ways: BTreeSet<(u32, u32)> = (0..net.link_count())
        .map(LinkId::new)
        .filter(|&l| net.modes(l).contains(mode))
        .map(|l| (net.upstream(l).raw(), net.downstream(l).raw()))
        .collect();
    let (class, bike) = if mode == Mode::Bike { ("k-bike", true) } else { ("k-walk", false) };
    let mut groups = Groups::new();
    for (&l, &n) in &per {
        let w = half_units(flow_width(n));
        let half = f64::from(w) / 2.0;
        // Dashes to the hundredth of a unit (2.4 × 1.5 is 3.5999… in floating point).
        let hundredths = |x: f64| (x * 100.0).round() / 100.0;
        let dash = if bike {
            format!("{} {}", hundredths(2.4 * half), hundredths(1.5 * half))
        } else {
            format!("0.1 {}", hundredths(2.0 * half))
        };
        let (u, d) = (net.upstream(l), net.downstream(l));
        let off = if ways.contains(&(d.raw(), u.raw())) { half / 2.0 + 1.0 } else { 0.0 };
        let path = groups.entry((w, class.to_owned(), dash)).or_default();
        frame.line(&points(net, l), off, path);
    }
    groups
}

/// Transit: each hop ridden, straight between its stops, as wide as its travellers.
fn transit(frame: &Frame, s: &Summarised<'_>, hops: &[(StopId, StopId, u8, u32)]) -> Groups {
    let mut groups = Groups::new();
    let Some(tt) = s.timetable else { return groups };
    let at = |stop: StopId| {
        let (x, y) = tt.stop_position(stop);
        (x.get(), y.get())
    };
    let most = f64::from(hops.iter().map(|h| h.3).max().unwrap_or(1));
    for &(a, b, kind, n) in hops {
        let w = half_units(2.0 + 8.0 * (f64::from(n) / most).sqrt());
        let path = groups.entry((w, format!("k-k{kind}"), String::new())).or_default();
        frame.line(&[at(a), at(b)], f64::from(w) / 4.0 + 1.0, path);
    }
    groups
}

/// The maps, one per mode with travellers, after the base network's definition.
pub(crate) fn maps(
    s: &Summarised<'_>,
    roads: &Roads,
    hops: &[(StopId, StopId, u8, u32)],
) -> String {
    let frame = Frame::of(s.network);
    let mut out = vec![base(&frame, s.network)];
    if !roads.links.is_empty() {
        let bands: String = (0..4)
            .map(|i| {
                format!(
                    "<span class=\"lg\">{}</span> ",
                    swatch(&format!("k-b{i}"), DELAY_BAND_LABELS[i])
                )
            })
            .collect();
        let note = format!(
            "{bands}· delay over free flow over the window; width = road capacity, one ribbon per direction on the right of its travel"
        );
        out.push(map(&frame, "Car traffic", "car", &cars(&frame, s, roads), &note));
    }
    for (mode, title, key, note) in [
        (
            Mode::Bike,
            "Cyclists",
            "bike",
            "dashed; width = cyclists over the window, full at FULL and more (as walkers), each direction on the right of its travel",
        ),
        (
            Mode::Walk,
            "Walkers",
            "walk",
            "dotted; width = walkers on links over the window, full at FULL and more (as cyclists), each direction on the right of its travel",
        ),
    ] {
        let groups = active(&frame, s, mode);
        if !groups.is_empty() {
            let note = note.replace("FULL", &crate::html::num(f64::from(FLOW_FULL), 0));
            out.push(map(&frame, title, key, &groups, &note));
        }
    }
    let groups = transit(&frame, s, hops);
    if !groups.is_empty() {
        let present: BTreeSet<u8> = hops.iter().map(|h| h.2).collect();
        let kinds: String = present
            .iter()
            .map(|&k| {
                format!(
                    "<span class=\"lg\">{}</span> ",
                    swatch(&format!("k-k{k}"), Kind::ALL[usize::from(k)].as_str())
                )
            })
            .collect();
        let note = format!(
            "{kinds}· width = travellers on board over the window, every journey taken; straight between stops"
        );
        let note = note.as_str();
        out.push(map(&frame, "Transit", "transit", &groups, note));
    }
    out.concat()
}

#[cfg(test)]
mod tests {
    use super::{Frame, flow_width};

    /// A frame of one unit a metre from (0, 0): a point (x, y) m at (20 + x, 20 − y).
    const FRAME: Frame = Frame { x0: 0.0, y1: 0.0, scale: 1.0, height: 100.0 };

    #[test]
    fn a_line_follows_its_points() {
        let mut d = String::new();
        FRAME.line(&[(0.0, 0.0), (10.0, 0.0), (10.0, -10.0)], 0.0, &mut d);
        assert_eq!(d, "M20 20L30 20L30 30");
    }

    /// East 10, then south 10 on the frame (y down), moved 2 units to the right of travel: south of
    /// the first piece, west of the second, by hand (20, 22), (28, 22), (28, 30). At the bend the
    /// two normals, (0, 1) and (−1, 0), sum to (−1, 1), of length √2: the point moves by
    /// (−1, 1) / √2 × 2 × min(2, 2 / √2) = (−2, 2), both pieces 2 from their line.
    #[test]
    fn a_line_moved_to_the_right_is_mitred_at_a_bend() {
        let mut d = String::new();
        FRAME.line(&[(0.0, 0.0), (10.0, 0.0), (10.0, -10.0)], 2.0, &mut d);
        assert_eq!(d, "M20 22L28 22L28 30");
    }

    /// Above the floor of 1.5 units by 8 √(n / 3 000): 0 travellers 1.5, 750 (a quarter of the
    /// full count) half the way, 1.5 + 8 × ½ = 5.5, 3 000 and 6 000 the full 9.5.
    #[test]
    fn a_line_of_750_travellers_is_half_way_above_the_floor() {
        let widths: Vec<f64> = [0, 750, 3_000, 6_000].into_iter().map(flow_width).collect();
        assert_eq!(widths, [1.5, 5.5, 9.5, 9.5]);
    }
}
