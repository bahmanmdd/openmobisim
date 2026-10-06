# ruff: noqa: E501
"""A run's summary as one self-contained HTML page (S227, roadmap I-ad).

Headline numbers, the flow map, the modes, the convergence, where the time went and the settings,
in the figures' own colours (paper by day, night in dark mode, following the reader's system).
Metric units throughout. Every number on it is also in the run's files.
"""

from __future__ import annotations

import base64
import datetime as dt
import html
import io
from pathlib import Path
from typing import Any

from openmobisim.viz._link import map_link
from openmobisim.viz._style import ION, THEMES

__all__ = ["summary"]

MODES = ("car", "bike", "walk", "transit", "car_transit", "bike_transit")
MODE_NAMES = {
    "car": "car",
    "bike": "bike",
    "walk": "walk",
    "transit": "transit",
    "car_transit": "park-and-ride",
    "bike_transit": "bike-and-ride",
}
STAGES = {
    "network_read": "network read (before the run)",
    "demand_read": "demand read",
    "setup": "setup",
    "static_routes": "bike and walk routes",
    "itineraries_setup": "itineraries prepared",
    "route_sets": "route sets",
    "partial_loading": "free-flow loading's groups",
    "loading": "loadings",
    "route_update": "route updates",
    "route_choice": "route choices",
    "itinerary_choice": "itinerary choices",
    "network_gap": "network gap",
    "results_assembled": "results assembled",
    "results_written": "results written",
}


#: The flow map's size on the page, and its resolution: 1 920 x 1 080 pixels (S227: sharper than
#: the prototype's).
MAP_SIZE_CM = (30.48, 17.145)
MAP_DPI = 160


def _png(fig: Any) -> str:
    buf = io.BytesIO()
    fig.savefig(buf, format="png", dpi=MAP_DPI, facecolor=fig.get_facecolor())
    return base64.b64encode(buf.getvalue()).decode()


def _fmt(x: float, digits: int = 1) -> str:
    return f"{x:,.{digits}f}".replace(",", "\u00a0")


def _tile(label: str, value: str, unit: str = "", note: str = "") -> str:
    return (
        f'<div class="tile"><div class="tl">{label}</div><div class="tv">{value}'
        f'<span class="tu">{unit}</span></div><div class="tn">{note}</div></div>'
    )


def _table(head: list[str], rows: list[list[str]], numeric_from: int = 1) -> str:
    th = "".join(
        f'<th class="{"n" if i >= numeric_from else ""}">{h}</th>' for i, h in enumerate(head)
    )
    body = "".join(
        "<tr>"
        + "".join(
            f'<td class="{"n" if i >= numeric_from else ""}">{c}</td>' for i, c in enumerate(r)
        )
        + "</tr>"
        for r in rows
    )
    return f"<table><thead><tr>{th}</tr></thead><tbody>{body}</tbody></table>"


def _no_alternative(run: Any, manifest: dict[str, Any]) -> str:
    """The trips choosing their mode that had none to choose (S233), overall and by class.

    Counted in people; the walk and bike limits mode choice applies are stated with them.
    """
    modes = run.trip_modes()
    if not any(modes["mode_choice"]):
        return ""
    choices = run.itinerary_choices() or {"traveller_id": [], "trip_seq": [], "user_class": []}
    class_of = {
        (w, int(q)): c
        for w, q, c in zip(
            choices["traveller_id"], choices["trip_seq"], choices["user_class"], strict=True
        )
    }
    people: dict[str, float] = {}
    stuck: dict[str, float] = {}
    for w, q, mode, choosing, weight in zip(
        modes["traveller_id"], modes["trip_seq"], modes["mode"], modes["mode_choice"],
        modes["weight"], strict=True,
    ):  # fmt: skip
        if not choosing:
            continue
        cls = class_of.get((w, int(q)), "")
        people[cls] = people.get(cls, 0.0) + float(weight)
        if mode is None:
            stuck[cls] = stuck.get(cls, 0.0) + float(weight)
    total, none = sum(people.values()), sum(stuck.values())
    walk, bike = manifest.get("walk_max_s"), manifest.get("bike_max_s")
    limits = (
        f" Walks and bike rides are offered up to {_fmt(walk / 60, 0)} and {_fmt(bike / 60, 0)} min"
        " (<code>walk_max_s</code>, <code>bike_max_s</code>); transit within its access walk."
        if walk is not None and bike is not None
        else ""
    )
    text = (
        f'<p class="note">Trips with no alternative: <b>{_fmt(none, 0)}</b> '
        f"({_fmt(100 * none / max(total, 1), 2)}% of the trips choosing their mode), counted, "
        f"not simulated: none of the modes their class may use is within reach.{limits}</p>"
    )
    if len(people) > 1:
        rows = [
            [html.escape(c), _fmt(people[c], 0), _fmt(100 * stuck.get(c, 0.0) / people[c], 2)]
            for c in sorted(people)
        ]
        text += _table(["class", "trips choosing", "no alternative (%)"], rows)
    return text


def _line(values: list[float], label: str, unit: str) -> str:
    """One series over the iterations, as inline SVG with a tooltip per point."""
    w, h, pl, pr, pt, pb = 520, 180, 46, 14, 12, 40
    vals = [v for v in values if v == v]
    if len(vals) < 2:
        return ""
    lo, hi = 0.0, max(vals) * 1.15 or 1.0
    xs = [pl + i * (w - pl - pr) / (len(values) - 1) for i in range(len(values))]
    ys = [pt + (1 - (v - lo) / (hi - lo)) * (h - pt - pb) if v == v else None for v in values]
    pts = " ".join(f"{x:.1f},{y:.1f}" for x, y in zip(xs, ys, strict=True) if y is not None)
    grid = "".join(
        f'<line x1="{pl}" x2="{w - pr}" y1="{pt + f * (h - pt - pb):.1f}" '
        f'y2="{pt + f * (h - pt - pb):.1f}" class="grid"/>'
        f'<text x="{pl - 6}" y="{pt + f * (h - pt - pb) + 4:.1f}" class="ax" '
        f'text-anchor="end">{hi * (1 - f):.2f}</text>'
        for f in (0, 0.5, 1)
    )
    ticks = "".join(
        f'<text x="{x:.1f}" y="{h - 22}" class="ax" text-anchor="middle">{i}</text>'
        for i, x in enumerate(xs)
    )
    dots = "".join(
        f'<circle cx="{x:.1f}" cy="{y:.1f}" r="4.5" class="dot"><title>iteration {i}: '
        f"{values[i]:.3f} {unit}</title></circle>"
        for i, (x, y) in enumerate(zip(xs, ys, strict=True))
        if y is not None
    )
    return (
        f'<svg viewBox="0 0 {w} {h}" class="chart" role="img" aria-label="{label}">{grid}'
        f'<polyline points="{pts}" class="ln"/>{dots}{ticks}'
        f'<text x="{(pl + w - pr) / 2}" y="{h - 3}" class="ax" text-anchor="middle">iteration</text></svg>'
    )


def _bars(items: list[tuple[str, float]], unit: str) -> str:
    """Horizontal bars, one hue, labelled; a tooltip per bar."""
    if not items:
        return ""
    top = max(v for _, v in items) or 1.0
    rows = []
    for name, v in items:
        pct = 100 * v / top
        rows.append(
            f'<div class="bar"><div class="bl">{name}</div><div class="bt">'
            f'<div class="bf" style="width:{pct:.1f}%" title="{name}: {v:.1f} {unit}"></div></div>'
            f'<div class="bv">{_fmt(v)}</div></div>'
        )
    return '<div class="bars">' + "".join(rows) + "</div>"


def summary(run: Any, path: str | None = None, *, title: str | None = None, note: str = "") -> str:
    """Write a run's summary as one self-contained HTML page, and return its path.

    Six headline numbers (trips, completed, never finished, mean trip, total travel time, run
    time), the flow map (if the run recorded per-link results, ``link_bins_s``), the modes (each
    one's share, completion and mean trip), the convergence by iteration with its verdict (for a
    run where trips choose their mode, the share whose mode changed), where the time went
    (``Run.timings()``), and the settings with the fingerprint. Light and dark follow the
    reader's system. Needs matplotlib for the map.

    Args:
        run: A ``Run``.
        path: Where to write it; ``summary.html`` in the run's output folder by default.
        title: The page's title; ``Run <run_id>`` by default.
        note: A line under the title, for example what the scenario is.

    Returns:
        The path written.
    """
    paper, night = THEMES["paper"], THEMES["night"]
    c = run.completion
    conv = run.convergence()
    manifest = run.manifest()
    by_mode = run.completion_by_mode
    # People, not simulated trips (S232): a traveller stands for its weight in people.
    total = sum(r["people"] for r in by_mode.values())
    done = sum(r["completed_people"] for r in by_mode.values())
    mean_trip_min = run.mean_travel_time_s / 60 if done > 0 else 0.0
    per_person = total / max(c["total_trips"], 1)

    # Tiles: the headline numbers.
    tiles = "".join(
        [
            _tile("trips", _fmt(total, 0)),
            _tile("completed", _fmt(100 * done / max(total, 1)), "%", f"{_fmt(done, 0)} trips"),
            _tile(
                "never finished",
                _fmt(100 * c["truncated"] / max(c["total_trips"], 1), 2),
                "%",
                f"{_fmt(c['truncated'] * per_person, 0)} still under way at the end",
            ),
            _tile("mean trip", _fmt(mean_trip_min), "min", "of the trips that finished"),
            _tile(
                "total travel time",
                _fmt(run.total_travel_time_s / 3600, 0),
                "h",
                "person-hours, weighted",
            ),
            _tile("run time", _fmt(run.timings()["seconds"][-1]), "s", "wall clock, this machine"),
        ]
    )

    # Modes: shares and completion.
    mode_rows, share_cells = [], []
    for i, m in enumerate(MODES):
        if m not in by_mode:
            continue
        r = by_mode[m]
        share = 100 * r["people"] / max(total, 1)
        mean_m = r["total_travel_time_s"] / 60 / max(r["completed_people"], 1)
        mode_rows.append(
            [
                f'<span class="sw" style="--c:{paper.modes[i]};--cn:{night.modes[i]}"></span>'
                f"{MODE_NAMES[m]}",
                _fmt(r["people"], 0),
                _fmt(share),
                _fmt(100 * r["completed_people"] / max(r["people"], 1)),
                _fmt(mean_m),
            ]
        )
        share_cells.append(
            f'<div class="seg" style="flex:{share:.3f};--c:{paper.modes[i]};'
            f'--cn:{night.modes[i]}" title="{MODE_NAMES[m]}: {share:.1f}%"></div>'
        )
    modes_html = f'<div class="stack">{"".join(share_cells)}</div>' + _table(
        ["mode", "trips", "share (%)", "completed (%)", "mean trip (min)"], mode_rows
    )
    modes_html += _no_alternative(run, manifest)

    # Convergence.
    it = list(conv["iteration"])
    conv_rows = []
    # The disequilibrium: the routes', else the itineraries' (I-al, S227); a run where trips
    # choose their mode also shows how its mode split settled against what chance alone gives.
    choosing = any(x == x for x in conv["mode_changed_share"])
    measure = [
        float(r) if r == r else float(i)
        for r, i in zip(conv["gap_excess"], conv["itinerary_gap_excess"], strict=True)
    ]
    for k in range(len(it)):
        mt = conv["total_travel_time_s"][k] / 60 / max(conv["completed_people"][k], 1)
        gx = measure[k]
        row = [str(it[k]), _fmt(mt, 2), "—" if gx != gx else f"{gx:.3f}"]
        if choosing:
            for x in (conv["mode_changed_share"][k], conv["mode_changed_floor"][k]):
                row.append("—" if x != x else f"{100 * x:.2f}")
        row += [
            _fmt(100 * conv["truncated"][k] / max(c["total_trips"], 1), 2),
            _fmt(conv["reroute_searches"][k], 0),
        ]
        conv_rows.append(row)
    verdict = run.convergence_verdict
    head = ["iteration", "mean trip (min)", "disequilibrium"]
    if choosing:
        head += ["mode changed (%)", "by chance (%)"]
    head += ["never finished (%)", "reroute searches"]
    conv_html = (
        _line(measure, "disequilibrium by iteration", "")
        + _table(head, conv_rows)
        + (
            f'<p class="note">Verdict on the last iteration: <b>{verdict}</b> (below 0.05 good, '
            f"below 0.15 acceptable). Iteration 0 is the free-flow loading.</p>"
            if verdict
            else ""
        )
        + (
            '<p class="note">Trips choose their mode: the disequilibrium is the itineraries\' '
            "(each trip against the best of the mode it kept); the mode split has settled when the "
            "share that changed mode is near what chance alone changes.</p>"
            if choosing
            else ""
        )
    )

    # Time by stage.
    t = run.timings()
    agg: dict[str, float] = {}
    for name, _, s in zip(t["stage"], t["iteration"], t["seconds"], strict=True):
        if name != "total":
            agg[name] = agg.get(name, 0.0) + s
    time_html = (
        _bars(
            [
                (STAGES.get(k, k), v)
                for k, v in sorted(agg.items(), key=lambda kv: -kv[1])
                if v >= 0.05
            ],
            "s",
        )
        + f'<p class="note">Run total {_fmt(t["seconds"][-1])} s, '
        "without the network read.</p>"
    )

    # Settings.
    settings = [
        ["equilibration", html.escape(manifest.get("equilibration_descriptor", ""))],
        ["route method", html.escape(str(manifest.get("route_descriptor", "")))],
        [
            "route update",
            html.escape(str(manifest.get("route_update_descriptor", run.route_update))),
        ],
        ["choice model", html.escape(run.choice_model)],
        ["flow level", str(manifest.get("flow_level", ""))],
        ["master seed", str(run.master_seed)],
        ["fingerprint", run.fingerprint],
    ]
    settings_html = _table(["setting", "value"], settings, numeric_from=9).replace(
        '</td><td class="">', '</td><td class="v">'
    )

    # Figures: the flow map in both themes, swapped by the colour scheme.
    figs = ""
    if run.link_bins() is not None:
        day = _png(map_link(run, theme="paper", size_cm=MAP_SIZE_CM))
        dark = _png(map_link(run, theme="night", size_cm=MAP_SIZE_CM))
        figs = (
            f'<img class="day" src="data:image/png;base64,{day}" alt="flow map">'
            f'<img class="dark" src="data:image/png;base64,{dark}" alt="flow map">'
        )

    when = dt.datetime.now().strftime("%Y-%m-%d %H:%M")
    title = title or f"Run {run.run_id}"
    css = f"""
:root {{ --surface:{paper.surface}; --ink:{paper.ink}; --ink2:{paper.ink2}; --muted:{paper.muted};
  --base:{paper.base}; --card:#ffffff; --accent:{ION[500]}; }}
* {{ box-sizing: border-box; }}
body {{ margin:0; background:var(--surface); color:var(--ink); font:14px/1.45 Inter, "Helvetica Neue",
  "Segoe UI", sans-serif; }}
main {{ max-width:1180px; margin:0 auto; padding:24px 16px 40px; }}
h1 {{ font-size:26px; margin:0; }} .sub {{ color:var(--ink2); margin:4px 0 0; }}
.meta {{ color:var(--muted); font:12px/1.4 "JetBrains Mono", Menlo, monospace; margin-top:6px; }}
.tiles {{ display:grid; grid-template-columns:repeat(auto-fit,minmax(160px,1fr)); gap:10px; margin:20px 0; }}
.tile {{ background:var(--card); border:1px solid var(--base); border-radius:10px; padding:12px 14px; }}
.tl {{ color:var(--muted); font-size:11px; text-transform:uppercase; letter-spacing:.08em; }}
.tv {{ font-size:28px; font-weight:700; margin-top:2px; }} .tu {{ font-size:14px; font-weight:500;
  color:var(--ink2); margin-left:3px; }} .tn {{ color:var(--muted); font-size:12px; }}
.grid2 {{ display:grid; grid-template-columns:1fr 1fr; gap:16px; }}
@media (max-width:860px) {{ .grid2 {{ grid-template-columns:1fr; }} }}
section {{ background:var(--card); border:1px solid var(--base); border-radius:12px; padding:14px 16px;
  margin-top:16px; }}
h2 {{ font-size:12px; text-transform:uppercase; letter-spacing:.09em; color:var(--muted); margin:0 0 10px; }}
table {{ width:100%; border-collapse:collapse; font-size:13px; }}
th {{ text-align:left; color:var(--muted); font-weight:600; border-bottom:1px solid var(--base); padding:5px 6px; }}
td {{ border-bottom:1px solid color-mix(in srgb, var(--base) 50%, transparent); padding:5px 6px; }}
.n {{ text-align:right; font-variant-numeric:tabular-nums; }}
.sw {{ display:inline-block; width:10px; height:10px; border-radius:2px; background:var(--c);
  margin-right:7px; vertical-align:-1px; }}
.stack {{ display:flex; gap:2px; height:14px; margin:4px 0 12px; }}
.seg {{ background:var(--c); border-radius:3px; min-width:2px; }}
.chart {{ width:100%; height:auto; }} .grid {{ stroke:var(--base); stroke-width:1; }}
.ax {{ fill:var(--muted); font-size:11px; }} .ln {{ fill:none; stroke:var(--accent); stroke-width:2; }}
.dot {{ fill:var(--accent); stroke:var(--card); stroke-width:2; }}
.bars {{ display:grid; gap:5px; }} .bar {{ display:grid; grid-template-columns:200px 1fr 64px; gap:8px;
  align-items:center; font-size:13px; }} .bl {{ color:var(--ink2); }}
.bt {{ background:color-mix(in srgb, var(--base) 35%, transparent); border-radius:4px; height:12px; }}
.bf {{ background:var(--accent); height:12px; border-radius:4px; }}
.bv {{ text-align:right; font-variant-numeric:tabular-nums; }}
.note {{ color:var(--muted); font-size:12px; margin:8px 0 0; }}
img {{ width:100%; border-radius:8px; }} img.dark {{ display:none; }}
footer {{ color:var(--muted); font:12px "JetBrains Mono", Menlo, monospace; margin-top:22px; }}
.grid2 > section {{ min-width:0; }} td.v {{ word-break:break-all; font:12px "JetBrains Mono", Menlo, monospace; }}
@media (prefers-color-scheme: dark) {{ :root {{ --surface:{night.surface}; --ink:{night.ink};
  --ink2:{night.ink2}; --muted:{night.muted}; --base:{night.base}; --card:#101b2d; --accent:{ION[400]}; }}
  .sw, .seg {{ background: var(--cn) !important; }} img.day {{ display:none; }} img.dark {{ display:block; }} }}
"""
    page = f"""<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1"><title>{html.escape(title)}</title>
<style>{css}</style></head><body><main>
<h1>{html.escape(title)}</h1><p class="sub">{html.escape(note)}</p>
<div class="meta">run {html.escape(run.run_id)} · fingerprint {run.fingerprint} · seed {run.master_seed} ·
openmobisim {html.escape(manifest.get("openmobisim_version", manifest.get("code_version", "")))} · {when}</div>
<div class="tiles">{tiles}</div>
{f"<section><h2>Flows</h2>{figs}</section>" if figs else ""}
<div class="grid2"><section><h2>Modes</h2>{modes_html}</section>
<section><h2>Convergence</h2>{conv_html}</section></div>
<div class="grid2"><section><h2>Where the time went</h2>{time_html}</section>
<section><h2>Settings</h2>{settings_html}</section></div>
<footer>Every number here is also in the run's files (kpis.parquet, timings.csv, manifest.json).</footer>
</main></body></html>"""
    if path is None:
        path = str(Path(run.timings_path).parent / "summary.html")
    Path(path).write_text(page, encoding="utf-8")
    return path
