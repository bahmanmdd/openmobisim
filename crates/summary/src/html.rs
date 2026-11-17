//! The page's pieces as HTML text: numbers in one format, tiles, tables, share bars, horizontal
//! bars, and the inline SVG charts (one series over iterations or clock times, stacked bars).
//! Every number a test reads carries a `data-k` name.

use openmobisim_map::html_text;

/// A number with `digits` decimals and commas between thousands, the map's format (`65,959`).
pub(crate) fn num(x: f64, digits: usize) -> String {
    if !x.is_finite() {
        return "—".to_owned();
    }
    let text = format!("{:.*}", digits, x.abs());
    let (whole, fraction) =
        text.split_once('.').map_or((text.as_str(), None), |(w, f)| (w, Some(f)));
    let mut out = String::new();
    for (i, c) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if let Some(f) = fraction {
        out.push('.');
        out.push_str(f);
    }
    let zero = out.chars().all(|c| matches!(c, '0' | '.' | ','));
    if x < 0.0 && !zero { format!("−{out}") } else { out }
}

/// A count as a number.
#[allow(clippy::cast_precision_loss, reason = "counts of trips and links, far below 2^53")]
pub(crate) fn count(n: usize) -> String {
    num(n as f64, 0)
}

/// `part` of `whole` in percent, with `digits` decimals; `—` of nothing.
#[allow(clippy::cast_precision_loss, reason = "counts of trips, far below 2^53")]
pub(crate) fn percent(part: usize, whole: usize, digits: usize) -> String {
    if whole == 0 { "—".to_owned() } else { num(100.0 * part as f64 / whole as f64, digits) }
}

/// A value a test reads: `text` in a span named `key`.
pub(crate) fn named(key: &str, text: &str) -> String {
    format!("<span data-k=\"{}\">{text}</span>", html_text(key))
}

/// A headline tile: its label, its value (named `key`) and unit, and a note under it.
pub(crate) fn tile(label: &str, key: &str, value: &str, unit: &str, note: &str) -> String {
    format!(
        "<div class=\"tile\"><div class=\"tl\">{label}</div><div class=\"tv\"><span class=\"nw\" \
         data-k=\"{key}\">{value}</span> <span class=\"tu\">{unit}</span></div>\
         <div class=\"tn\">{note}</div></div>"
    )
}

/// A cell: its text, and the name a test reads it by (none: not read).
pub(crate) struct Cell(pub String, pub Option<String>);

impl Cell {
    /// A cell of plain text.
    pub fn text(s: impl Into<String>) -> Self {
        Self(s.into(), None)
    }

    /// A cell a test reads by `key`.
    pub fn named(key: impl Into<String>, s: impl Into<String>) -> Self {
        Self(s.into(), Some(key.into()))
    }
}

/// A table: its head, and its rows of cells; the columns from `numeric_from` on right-aligned.
pub(crate) fn table(head: &[&str], rows: Vec<Vec<Cell>>, numeric_from: usize) -> String {
    let class = |i: usize| if i >= numeric_from { " class=\"n\"" } else { "" };
    let th: String =
        head.iter().enumerate().map(|(i, h)| format!("<th{}>{h}</th>", class(i))).collect();
    let body: String = rows
        .into_iter()
        .map(|row| {
            let cells: String = row
                .into_iter()
                .enumerate()
                .map(|(i, Cell(text, key))| match key {
                    Some(k) => format!("<td{} data-k=\"{}\">{text}</td>", class(i), html_text(&k)),
                    None => format!("<td{}>{text}</td>", class(i)),
                })
                .collect();
            format!("<tr>{cells}</tr>")
        })
        .collect();
    format!("<table><thead><tr>{th}</tr></thead><tbody>{body}</tbody></table>")
}

/// A swatch of a colour class (`k-car`, `k-b0`, ...) before a name.
pub(crate) fn swatch(colour: &str, name: &str) -> String {
    format!("<span class=\"sw {colour}\"></span>{name}")
}

/// A share bar: one segment per `(colour class, share, title)`.
pub(crate) fn stack(parts: &[(String, f64, String)]) -> String {
    let segments: String = parts
        .iter()
        .filter(|p| p.1 > 0.0)
        .map(|(c, share, title)| {
            format!(
                "<div class=\"seg {c}\" style=\"flex:{share:.4}\" title=\"{}\"></div>",
                html_text(title)
            )
        })
        .collect();
    format!("<div class=\"stack\">{segments}</div>")
}

/// Horizontal bars, one hue, each labelled with its value (`unit`), a tooltip per bar.
pub(crate) fn bars(items: &[(&str, f64)], unit: &str) -> String {
    let top = items.iter().map(|i| i.1).fold(0.0, f64::max);
    let top = if top > 0.0 { top } else { 1.0 };
    let rows: String = items
        .iter()
        .map(|(name, v)| {
            format!(
                "<div class=\"bar\"><div class=\"bl\">{name}</div><div class=\"bt\"><div \
                 class=\"bf\" style=\"width:{:.1}%\" title=\"{name}: {} {unit}\"></div></div>\
                 <div class=\"bv\" data-k=\"stage.{name}\">{}</div></div>",
                100.0 * v / top,
                num(*v, 2),
                num(*v, 2)
            )
        })
        .collect();
    format!("<div class=\"bars\">{rows}</div>")
}

/// A chart's frame: width, height, and the plot's left, right, top and bottom margins.
const W: f64 = 520.0;
const PL: f64 = 46.0;
const PR: f64 = 14.0;
const PT: f64 = 12.0;
const PB: f64 = 40.0;

/// The top of a chart's value axis: the least of 1, 2 or 5 times a power of ten at or above
/// `max` (1 for nothing above 0), so the lines at 0, half and the top fall on round values.
pub(crate) fn top(max: f64) -> f64 {
    if max.is_nan() || max <= 0.0 {
        return 1.0;
    }
    let mut p = 10_f64.powf(max.log10().floor());
    loop {
        for m in [1.0, 2.0, 5.0] {
            if m * p >= max * (1.0 - 1e-12) {
                return m * p;
            }
        }
        p *= 10.0;
    }
}

/// A value as written: with as few decimals (up to six) as write it exactly.
fn exact(v: f64) -> String {
    let d = (0..6).find(|&d| {
        let f = 10_f64.powi(d);
        ((v * f).round() / f - v).abs() <= 1e-9 * v.abs().max(1.0)
    });
    num(v, usize::try_from(d.unwrap_or(6)).unwrap_or(6))
}

/// The value axis from 0 to `hi` (a [`top`]): three grid lines, at 0, half and `hi`, each
/// labelled with its value exactly.
fn grid(hi: f64, h: f64) -> String {
    [0.0, 0.5, 1.0]
        .iter()
        .map(|f| {
            let y = PT + f * (h - PT - PB);
            format!(
                "<line x1=\"{PL}\" x2=\"{}\" y1=\"{y:.1}\" y2=\"{y:.1}\" class=\"grid\"/>\
                 <text x=\"{}\" y=\"{:.1}\" class=\"ax\" text-anchor=\"end\">{}</text>",
                W - PR,
                PL - 6.0,
                y + 4.0,
                exact(hi * (1.0 - f))
            )
        })
        .collect()
}

/// One series over `xs` (iterations or clock times) as a line with a dot and a tooltip per point,
/// `label` under the axis; none for fewer than two finite values.
#[allow(clippy::cast_precision_loss, reason = "a few hundred points at most")]
pub(crate) fn line(
    xs: &[String],
    values: &[f64],
    label: &str,
    unit: &str,
    digits: usize,
) -> String {
    let h = 190.0;
    if values.iter().filter(|v| v.is_finite()).count() < 2 {
        return String::new();
    }
    let hi = top(values.iter().copied().filter(|v| v.is_finite()).fold(0.0, f64::max));
    let n = values.len();
    let px: Vec<f64> = (0..n).map(|i| PL + i as f64 * (W - PL - PR) / (n - 1) as f64).collect();
    let py = |v: f64| PT + (1.0 - v / hi) * (h - PT - PB);
    let points: Vec<String> = (0..n)
        .filter(|&i| values[i].is_finite())
        .map(|i| format!("{:.1},{:.1}", px[i], py(values[i])))
        .collect();
    let dots: String = (0..n)
        .filter(|&i| values[i].is_finite())
        .map(|i| {
            format!(
                "<circle cx=\"{:.1}\" cy=\"{:.1}\" r=\"3.5\" class=\"dot\"><title>{}: {} {unit}\
                 </title></circle>",
                px[i],
                py(values[i]),
                xs[i],
                num(values[i], digits)
            )
        })
        .collect();
    let every = (n / 8).max(1);
    let ticks: String = (0..n)
        .step_by(every)
        .map(|i| {
            format!(
                "<text x=\"{:.1}\" y=\"{}\" class=\"ax\" text-anchor=\"middle\">{}</text>",
                px[i],
                h - 22.0,
                xs[i]
            )
        })
        .collect();
    format!(
        "<svg viewBox=\"0 0 {W} {h}\" class=\"chart\" role=\"img\" aria-label=\"{label}\">{}\
         <polyline points=\"{}\" class=\"ln\"/>{dots}{ticks}<text x=\"{}\" y=\"{}\" class=\"ax\" \
         text-anchor=\"middle\">{label}</text></svg>",
        grid(hi, h),
        points.join(" "),
        (PL + W - PR) / 2.0,
        h - 3.0
    )
}

/// Stacked bars over `bins`, one colour class per series (`(name, colour class, values)`).
#[allow(clippy::cast_precision_loss, reason = "a few dozen bins")]
pub(crate) fn stacked(bins: &[String], series: &[(&str, &str, Vec<f64>)], label: &str) -> String {
    let (h, n) = (190.0, bins.len());
    if n == 0 || series.is_empty() {
        return String::new();
    }
    let totals: Vec<f64> = (0..n).map(|i| series.iter().map(|s| s.2[i]).sum()).collect();
    let hi = top(totals.iter().copied().fold(0.0, f64::max));
    let slot = (W - PL - PR) / n as f64;
    let bw = (slot * 0.78).max(1.0);
    let mut rects = String::new();
    for i in 0..n {
        let mut y = h - PB;
        for (name, colour, values) in series {
            if values[i] <= 0.0 {
                continue;
            }
            let hh = values[i] / hi * (h - PT - PB);
            y -= hh;
            rects.push_str(&format!(
                "<rect x=\"{:.1}\" y=\"{y:.1}\" width=\"{bw:.1}\" height=\"{hh:.1}\" class=\"seg \
                 {colour}\"><title>{} {name}: {}</title></rect>",
                PL + i as f64 * slot + (slot - bw) / 2.0,
                bins[i],
                num(values[i], 0)
            ));
        }
    }
    let ticks: String = (0..n)
        .step_by((n / 8).max(1))
        .map(|i| {
            format!(
                "<text x=\"{:.1}\" y=\"{}\" class=\"ax\" text-anchor=\"middle\">{}</text>",
                PL + i as f64 * slot + slot / 2.0,
                h - 22.0,
                bins[i]
            )
        })
        .collect();
    format!(
        "<svg viewBox=\"0 0 {W} {h}\" class=\"chart\" role=\"img\" aria-label=\"{label}\">{}{rects}\
         {ticks}<text x=\"{}\" y=\"{}\" class=\"ax\" text-anchor=\"middle\">{label}</text></svg>",
        grid(hi, h),
        (PL + W - PR) / 2.0,
        h - 3.0
    )
}
