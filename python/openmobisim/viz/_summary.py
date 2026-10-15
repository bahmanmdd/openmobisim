# ruff: noqa: E501
"""A run's summary as one self-contained HTML page (S227, roadmap I-ad; redesigned S281).

Headline numbers, the travel statistics (the modes, when people left, the roads' traffic over the
day and its delay, transit and parking, bikes and walking), the flow map, the convergence, the
computation time in three parts (reading, running, writing; the stage-by-stage breakdown on its
own page, ``timings.html``) and the settings, in the figures' own colours (paper by day, night in
dark mode, following the reader's system). Metric units throughout. Every number on it is also in
the run's files.
"""

from __future__ import annotations

import base64
import datetime as dt
import html
import io
import math
from pathlib import Path
from typing import Any

import numpy as np

from openmobisim import __version__
from openmobisim._paths import show_path
from openmobisim.viz._figure import CREATOR
from openmobisim.viz._link import DELAY_BAND_LABELS, _clock, delay_band, link_table, map_link
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
    "none": "no alternative",
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
        f'<div class="tile"><div class="tl">{label}</div><div class="tv"><span class="nw">{value}'
        f'</span> <span class="tu">{unit}</span></div><div class="tn">{note}</div></div>'
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
    fb = getattr(run, "fallback_car", None) or {"traveller_id": [], "trip_seq": []}
    fell_back = {(w, int(q)) for w, q in zip(fb["traveller_id"], fb["trip_seq"], strict=True)}
    if not any(modes["mode_choice"]) and not fell_back:
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
        fell = (w, int(q)) in fell_back
        if not (choosing or fell):
            continue
        cls = class_of.get((w, int(q)), "")
        people[cls] = people.get(cls, 0.0) + float(weight)
        if mode is None or fell:
            stuck[cls] = stuck.get(cls, 0.0) + float(weight)
    total, none = sum(people.values()), sum(stuck.values())
    walk, bike = manifest.get("walk_max_s"), manifest.get("bike_max_s")

    def minutes(seconds: object) -> str:
        # A limit in seconds, or "inf" (every walk or ride offered), in minutes.
        value = float(seconds)  # type: ignore[arg-type]
        return "any length" if math.isinf(value) else f"{_fmt(value / 60, 0)} min"

    limits = (
        f" Walks and bike rides are offered up to {minutes(walk)} and {minutes(bike)}"
        " (<code>walk_max_s</code>, <code>bike_max_s</code>); transit within its access walk and"
        " up to two hours door to door (<code>transit_options</code>' <code>access_walk_max_s</code>,"
        " <code>journey_max_s</code>)."
        if walk is not None and bike is not None
        else ""
    )
    own = manifest.get("class_limits") or {}
    if own:
        names = {"walk_max_s": "walk", "bike_max_s": "ride", "access_walk_max_s": "walk to a stop"}
        limits += (
            " Classes with limits of their own: "
            + "; ".join(
                f"{html.escape(c)} ("
                + ", ".join(f"{names.get(k, k)} {minutes(v)}" for k, v in lim.items())
                + ")"
                for c, lim in own.items()
            )
            + "."
        )
    # With the fallback (S271, W-1) they travel by car; without it they are not simulated.
    fate = (
        "travelled by car instead (a lift or a taxi for a traveller without one), counted under "
        "the car (<code>Run.fallback_car</code> lists them)"
        if fell_back
        else "counted, not simulated"
    )
    text = (
        f'<p class="note">Trips with no alternative: <b>{_fmt(none, 0)}</b> '
        f"({_fmt(100 * none / max(total, 1), 2)}% of the trips choosing their mode or left "
        f"without their stated one): none of the modes they may use is within reach; they "
        f"{fate}.{limits}</p>"
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


#: The three parts of the computation time (S281, the user: "a general breakdown of major
#: computation time components is enough for the summary (reading, run, writing results)").
WRITING = ("results_assembled", "results_written")


def _stacked(
    bins: list[str], series: list[tuple[str, str, str, list[float]]], label: str, unit: str
) -> str:
    """Stacked bars over time, one colour per series (``(name, day, night, values)``)."""
    w, h, pl, pr, pt, pb = 520, 190, 46, 14, 12, 40
    n = len(bins)
    if n == 0 or not series:
        return ""
    totals = [sum(v[i] for _, _, _, v in series) for i in range(n)]
    hi = max(totals) * 1.1 or 1.0
    slot = (w - pl - pr) / n
    bw = max(slot * 0.78, 1.0)
    rects = []
    for i in range(n):
        y = pt + (h - pt - pb)
        for name, day, night, values in series:
            v = values[i]
            if v <= 0:
                continue
            hh = v / hi * (h - pt - pb)
            y -= hh
            rects.append(
                f'<rect x="{pl + i * slot + (slot - bw) / 2:.1f}" y="{y:.1f}" width="{bw:.1f}" '
                f'height="{hh:.1f}" style="--c:{day};--cn:{night}" class="seg"><title>{bins[i]} '
                f"{name}: {_fmt(v, 0)} {unit}</title></rect>"
            )
    grid = "".join(
        f'<line x1="{pl}" x2="{w - pr}" y1="{pt + f * (h - pt - pb):.1f}" '
        f'y2="{pt + f * (h - pt - pb):.1f}" class="grid"/>'
        f'<text x="{pl - 6}" y="{pt + f * (h - pt - pb) + 4:.1f}" class="ax" '
        f'text-anchor="end">{_fmt(hi * (1 - f), 0)}</text>'
        for f in (0, 0.5, 1)
    )
    every = max(1, n // 8)
    ticks = "".join(
        f'<text x="{pl + i * slot + slot / 2:.1f}" y="{h - 22}" class="ax" '
        f'text-anchor="middle">{bins[i]}</text>'
        for i in range(0, n, every)
    )
    return (
        f'<svg viewBox="0 0 {w} {h}" class="chart" role="img" aria-label="{label}">{grid}'
        f'{"".join(rects)}{ticks}<text x="{(pl + w - pr) / 2}" y="{h - 3}" class="ax" '
        f'text-anchor="middle">{label}</text></svg>'
    )


def _series(xs: list[str], values: list[float], label: str, unit: str) -> str:
    """One series over clock times, as inline SVG with a tooltip per point."""
    w, h, pl, pr, pt, pb = 520, 190, 46, 14, 12, 40
    if len(values) < 2:
        return ""
    hi = max(values) * 1.15 or 1.0
    px = [pl + i * (w - pl - pr) / (len(values) - 1) for i in range(len(values))]
    py = [pt + (1 - v / hi) * (h - pt - pb) for v in values]
    pts = " ".join(f"{x:.1f},{y:.1f}" for x, y in zip(px, py, strict=True))
    grid = "".join(
        f'<line x1="{pl}" x2="{w - pr}" y1="{pt + f * (h - pt - pb):.1f}" '
        f'y2="{pt + f * (h - pt - pb):.1f}" class="grid"/>'
        f'<text x="{pl - 6}" y="{pt + f * (h - pt - pb) + 4:.1f}" class="ax" '
        f'text-anchor="end">{_fmt(hi * (1 - f), 0)}</text>'
        for f in (0, 0.5, 1)
    )
    every = max(1, len(xs) // 8)
    ticks = "".join(
        f'<text x="{px[i]:.1f}" y="{h - 22}" class="ax" text-anchor="middle">{xs[i]}</text>'
        for i in range(0, len(xs), every)
    )
    dots = "".join(
        f'<circle cx="{x:.1f}" cy="{y:.1f}" r="3.5" class="dot"><title>{xs[i]}: '
        f"{_fmt(values[i], 0)} {unit}</title></circle>"
        for i, (x, y) in enumerate(zip(px, py, strict=True))
    )
    return (
        f'<svg viewBox="0 0 {w} {h}" class="chart" role="img" aria-label="{label}">{grid}'
        f'<polyline points="{pts}" class="ln"/>{dots}{ticks}'
        f'<text x="{(pl + w - pr) / 2}" y="{h - 3}" class="ax" text-anchor="middle">{label}</text></svg>'
    )


def _band_colours(theme: Any) -> list[str]:
    """One colour per delay band, read off the theme's ramp as the map's legend does."""
    from openmobisim.viz._figure import ramp_rgb

    rgbs = ramp_rgb(theme.ramp_delay, np.linspace(0.0, 1.0, len(DELAY_BAND_LABELS)))
    return ["#" + "".join(f"{round(float(v) * 255):02x}" for v in c) for c in rgbs]


def _roads(run: Any, paper: Any, night: Any) -> tuple[str, list[str]]:
    """The roads' traffic over the day and its delay, from the per-link results.

    Returns the section's HTML and two tile lines (PCU-km, mean speed); both empty without
    per-link results.
    """
    link_bins = run.link_bins()
    network = run.network
    if link_bins is None or network is None:
        return "", []
    step = link_bins.bin_seconds
    b = link_bins.bins().astype(np.int64)
    if not len(b):
        return "", []
    n_links = network.link_count
    table = link_table(link_bins, n_links, network.link_free_flow_s(), first=None, stop=None)
    first, stop = int(b.min()), int(b.max()) + 1
    volume, mean_s, delay = table["volume"], np.nan_to_num(table["mean_s"]), table["delay"]
    length_km = network.link_length_m() / 1000.0
    hours_shown = (stop - first) * step / 3600.0
    pcu_km = volume * length_km * hours_shown  # over the bins that saw traffic
    pcu_h = volume * mean_s / 3600.0 * hours_shown
    total_km, total_h = float(pcu_km.sum()), float(pcu_h.sum())
    total_delay = float((pcu_h * delay).sum())
    speed = total_km / total_h if total_h > 0 else 0.0
    bands = delay_band(delay)
    by_band = [float(pcu_km[bands == i].sum()) for i in range(len(DELAY_BAND_LABELS))]
    day, dark = _band_colours(paper), _band_colours(night)
    stack = "".join(
        f'<div class="seg" style="flex:{v / max(total_km, 1):.4f};--c:{day[i]};--cn:{dark[i]}" '
        f'title="{DELAY_BAND_LABELS[i]} delay: {_fmt(v, 0)} PCU-km"></div>'
        for i, v in enumerate(by_band)
    )
    band_rows = [
        [
            f'<span class="sw" style="--c:{day[i]};--cn:{dark[i]}"></span>{DELAY_BAND_LABELS[i]}',
            _fmt(v, 0),
            _fmt(100 * v / max(total_km, 1)),
        ]
        for i, v in enumerate(by_band)
    ]
    # Traffic over the day: PCU on the roads per bin (the PCU-seconds in the bin over its length).
    per_bin_s = np.bincount(b, weights=link_bins.pcu_seconds(), minlength=stop)[first:stop]
    clock = [_clock(i * step) for i in range(first, stop)]
    on_road = [float(x) for x in per_bin_s / step]
    ids = network.link_ids()
    busiest = [int(i) for i in np.argsort(-volume)[:5] if volume[i] > 0]
    busy_rows = [
        [
            html.escape(str(ids[i])),
            _fmt(length_km[i] * 1000, 0),
            _fmt(volume[i], 0),
            _fmt(100 * delay[i]),
        ]
        for i in busiest
    ]
    section = (
        '<div class="grid2"><div>'
        + f'<div class="stack">{stack}</div>'
        + _table(["delay band", "PCU-km", "share (%)"], band_rows)
        + f'<p class="note">Over {_clock(first * step)}–{_clock(stop * step)}, the bins that '
        f"saw traffic: <b>{_fmt(total_km, 0)}</b> PCU-km in <b>{_fmt(total_h, 0)}</b> PCU-hours, "
        f"a mean speed of <b>{_fmt(speed)}</b> km/h; "
        f"<b>{_fmt(100 * total_delay / max(total_h, 1e-9))}</b> % of the time on the roads is "
        f"delay beyond free flow (a signal's mean wait counts as free flow).</p>"
        + "</div><div>"
        + _series(clock, on_road, "vehicles on the roads (PCU), by bin", "PCU")
        + "</div></div>"
        + "<h3>Busiest links, by volume per direction</h3>"
        + _table(["link", "length (m)", "volume (PCU/h)", "delay (%)"], busy_rows)
    )
    return section, [_fmt(total_km, 0), f"{_fmt(speed)} km/h mean speed"]


def _departures(run: Any, paper: Any, night: Any) -> str:
    """When people left, by mode, in quarter hours (stacked), from ``trip_modes()``."""
    tm = run.trip_modes()
    if len(tm["departure_s"]) == 0:
        return ""
    dep = np.asarray(tm["departure_s"], dtype=np.int64) // 900
    weight = np.asarray(tm["weight"], dtype=np.float64)
    modes = np.asarray([m if m is not None else "none" for m in tm["mode"]])
    lo, hi = int(dep.min()), int(dep.max()) + 1
    bins = [_clock(i * 900) for i in range(lo, hi)]
    series = []
    for i, m in enumerate((*MODES, "none")):
        mask = modes == m
        if not mask.any():
            continue
        counts = np.bincount(dep[mask] - lo, weights=weight[mask], minlength=hi - lo)
        c_day, c_night = (
            (paper.modes[i], night.modes[i]) if m != "none" else (paper.base, night.base)
        )
        series.append((MODE_NAMES[m], c_day, c_night, [float(x) for x in counts]))
    return _stacked(bins, series, "departures by quarter hour (people)", "people")


def _services(run: Any) -> str:
    """Transit, parking, bikes and walking in numbers; empty without any of them."""
    rows: list[list[str]] = []
    ts = run.transit_summary
    if ts:
        rows.append(
            [
                "boardings",
                _fmt(ts.get("boardings", 0.0), 0),
                "people boarding a vehicle, summed over the day's calls",
            ]
        )
        choices = run.itinerary_choices()
        if choices and len(choices["rides"]):
            rides = np.asarray(choices["rides"], dtype=np.float64)
            modes = np.asarray([m or "" for m in choices["mode"]])
            riding = rides[(rides > 0) & np.isin(modes, ["transit", "car_transit", "bike_transit"])]
            if len(riding):
                rows.append(
                    [
                        "rides per transit trip",
                        _fmt(float(riding.mean()), 2),
                        f"{_fmt(100 * float((riding > 1).mean()))} % of the trips change "
                        "vehicle at least once",
                    ]
                )
        if ts.get("bus_runs_on_roads"):
            rows.append(
                [
                    "bus runs on the roads",
                    _fmt(ts["bus_runs_on_roads"], 0),
                    f"{_fmt(ts.get('bus_runs_arrived', 0), 0)} reached their last stop in the "
                    "window; the other runs went by the schedule",
                ]
            )
            rows.append(
                [
                    "mean bus delay at the last stop",
                    f"{ts.get('bus_delay_mean_s', 0.0) / 60:+.1f} min",
                    "realised minus scheduled; negative is early",
                ]
            )
    ps = run.parking_summary
    if ps:
        for vehicle in ("car", "bike"):
            arrivals = ps.get(f"{vehicle}_arrivals", 0.0)
            if arrivals:
                full = ps.get(f"{vehicle}_overflow_arrivals", 0.0)
                rows.append(
                    [
                        f"{vehicle}s parked",
                        _fmt(arrivals, 0),
                        f"{_fmt(100 * full / max(arrivals, 1))} % found their parking full; "
                        f"{_fmt(ps.get(f'{vehicle}_left_at_end', 0.0), 0)} still parked at the end",
                    ]
                )
    for layer, word in (("bike", "cycled"), ("walk", "walked")):
        lb = run.link_bins(layer)
        if lb is None or run.network is None:
            continue
        net = run.network.layer(layer)
        link = lb.links().astype(np.int64)
        per_link = np.bincount(link, weights=lb.pcu(), minlength=net.link_count)
        km = float((per_link * net.link_length_m() / 1000.0).sum())
        hours = float(lb.pcu_seconds().sum()) / 3600.0
        if km > 0:
            rows.append(
                [
                    f"kilometres {word}",
                    _fmt(km, 0),
                    f"traveller-km on the {layer} layer; {_fmt(km / max(hours, 1e-9))} km/h "
                    "on the move",
                ]
            )
    if not rows:
        return ""
    return _table(["", "value", ""], rows, numeric_from=1).replace(
        '<td class="n">', '<td class="n v2">'
    )


def _computation(run: Any) -> tuple[str, str]:
    """The three-part time for the summary, and the stage-by-stage page's body (S281)."""
    t = run.timings()
    agg: dict[str, float] = {}
    per: dict[int, dict[str, float]] = {}
    for name, iteration, seconds in zip(t["stage"], t["iteration"], t["seconds"], strict=True):
        if name == "total":
            continue
        agg[name] = agg.get(name, 0.0) + seconds
        if iteration is not None:
            cell = per.setdefault(int(iteration), {})
            cell[name] = cell.get(name, 0.0) + seconds
    total = float(t["seconds"][-1])  # the run total, without the network read
    network_read = agg.get("network_read", 0.0)
    reading = agg.get("demand_read", 0.0)
    writing = sum(agg.get(k, 0.0) for k in WRITING)
    running = max(total - reading - writing, 0.0)
    parts = [
        ("reading the demand", reading),
        ("running", running),
        ("writing the results", writing),
    ]
    line = (
        _bars(parts, "s")
        + f'<p class="note">Run total <b>{_fmt(total)}</b> s on this machine'
        + (f", after <b>{_fmt(network_read)}</b> s reading the network" if network_read else "")
        + '. Stage by stage: <a href="timings.html">timings.html</a> beside this page, and '
        "<code>timings.csv</code>.</p>"
    )
    stages = _bars(
        [
            (STAGES.get(k, k), v)
            for k, v in sorted(agg.items(), key=lambda kv: -kv[1])
            if v >= 0.005
        ],
        "s",
    )
    kinds = (
        "partial_loading",
        "loading",
        "route_update",
        "route_choice",
        "itinerary_choice",
        "network_gap",
    )
    columns = [c for c in kinds if any(c in row for row in per.values())]
    rows = [[str(i), *(_fmt(per[i].get(c, 0.0), 2) for c in columns)] for i in sorted(per)]
    page = "<section><h2>Every stage</h2>" + stages + "</section>"
    if rows:
        page += (
            "<section><h2>By iteration (seconds)</h2>"
            + _table(["iteration", *(STAGES.get(c, c) for c in columns)], rows)
            + "</section>"
        )
    return line, page


def summary(
    run: Any,
    path: str | Path | None = None,
    *,
    title: str | None = None,
    note: str = "",
    quiet: bool = False,
) -> Path:
    """Write a run's summary as one self-contained HTML page, and return its path.

    The headline numbers (trips, completed, mean trip, total travel time, the roads' PCU-km and
    mean speed, run time), the travel statistics (the modes' shares, completion and mean trips;
    when people left, by mode; the roads' traffic over the day, its delay by band and the
    busiest links; transit, parking, bikes and walking in numbers), the flow map (if the run
    recorded per-link results, ``link_bins_s``), the convergence by iteration with its verdict,
    the computation time in three parts (reading, running, writing; stage by stage on
    ``timings.html`` beside it, S281) and the settings with the fingerprint. Light and dark
    follow the reader's system. Needs matplotlib for the map.

    Args:
        run: A ``Run``.
        path: Where to write it; ``summary.html`` in the run's output folder by default.
        title: The page's title; ``Run <run_id>`` by default.
        note: A line under the title, for example what the scenario is.
        quiet: Do not say where the page was written (by default a link to it is printed, or
            shown in a notebook).

    Returns:
        The path written, a ``Path``.
    """
    paper, night = THEMES["paper"], THEMES["night"]
    c = run.completion
    conv = run.convergence()
    manifest = run.manifest()
    by_mode = run.completion_by_mode
    # People, not simulated trips (S232): a traveller stands for its weight in people. Every
    # person completed, is still under way at the end, or was never simulated (no alternative,
    # no path, no vehicle): the three add up to all (S269).
    total = sum(r["people"] for r in by_mode.values())
    done = sum(r["completed_people"] for r in by_mode.values())
    under_way = sum(r.get("truncated_people", 0.0) for r in by_mode.values())
    not_run = sum(r.get("not_run_people", 0.0) for r in by_mode.values())
    mean_trip_min = run.mean_travel_time_s / 60 if done > 0 else 0.0

    # The simulation window (S271), and what departed outside it and was left out.
    window = getattr(run, "window", None)
    span = ""
    if window is not None:
        span = f"in {_clock(window['start_s'])}–{_clock(window['end_s'])}"
        out = window["before_people"] + window["after_people"]
        if out:
            span += f" · <b>{_fmt(out, 0)} more left out</b> (departing outside it)"

    roads_html, road_tiles = _roads(run, paper, night)

    # Tiles: the headline numbers.
    tiles = [
        _tile("trips", _fmt(total, 0), "", span),
        _tile("completed", _fmt(100 * done / max(total, 1), 2), "%", f"{_fmt(done, 0)} trips"),
        _tile(
            "not completed",
            _fmt(100 * (under_way + not_run) / max(total, 1), 2),
            "%",
            f"{_fmt(100 * under_way / max(total, 1), 2)} % still under way at the end · "
            f"{_fmt(100 * not_run / max(total, 1), 2)} % never simulated (no alternative)",
        ),
        _tile("mean trip", _fmt(mean_trip_min), "min", "of the trips that finished"),
        _tile(
            "total travel time",
            _fmt(run.total_travel_time_s / 3600, 0),
            "h",
            "person-hours, weighted",
        ),
    ]
    if road_tiles:
        tiles.append(_tile("on the roads", road_tiles[0], "PCU\u2011km", road_tiles[1]))  # no break
    tiles.append(
        _tile("run time", _fmt(run.timings()["seconds"][-1]), "s", "wall clock, this machine")
    )

    # Modes: shares, completion, mean trip and the time spent.
    mode_rows, share_cells = [], []
    for i, m in enumerate((*MODES, "none")):
        if m not in by_mode:
            continue
        r = by_mode[m]
        share = 100 * r["people"] / max(total, 1)
        mean_m = r["total_travel_time_s"] / 60 / max(r["completed_people"], 1)
        # The trips with no alternative in the theme's quiet base colour (S269).
        c_day, c_night = (
            (paper.modes[i], night.modes[i]) if m != "none" else (paper.base, night.base)
        )
        mode_rows.append(
            [
                f'<span class="sw" style="--c:{c_day};--cn:{c_night}"></span>{MODE_NAMES[m]}',
                _fmt(r["people"], 0),
                _fmt(share),
                _fmt(100 * r["completed_people"] / max(r["people"], 1)),
                _fmt(mean_m) if r["completed_people"] > 0 else "—",
                _fmt(r["total_travel_time_s"] / 3600, 0),
            ]
        )
        share_cells.append(
            f'<div class="seg" style="flex:{share:.3f};--c:{c_day};'
            f'--cn:{c_night}" title="{MODE_NAMES[m]}: {share:.1f}%"></div>'
        )
    modes_html = f'<div class="stack">{"".join(share_cells)}</div>' + _table(
        ["mode", "trips", "share (%)", "completed (%)", "mean trip (min)", "travel time (h)"],
        mode_rows,
    )
    modes_html += _no_alternative(run, manifest)
    departures_html = _departures(run, paper, night)
    services_html = _services(run)

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
        '<div class="grid2"><div>'
        + _line(measure, "disequilibrium by iteration", "")
        + "</div><div>"
        + _table(head, conv_rows)
        + "</div></div>"
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

    # Computation time: three parts here, every stage on its own page (S281).
    time_html, timings_body = _computation(run)

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
    css = _css(paper, night)
    version = html.escape(manifest.get("openmobisim_version", manifest.get("code_version", "")))
    meta = (
        f'<div class="meta">run {html.escape(run.run_id)} · fingerprint {run.fingerprint} · '
        f"seed {run.master_seed} · openmobisim {version} · {when}</div>"
    )
    generator = CREATOR.format(version=__version__)
    sections = [
        f'<div class="grid2"><section><h2>Modes</h2>{modes_html}</section>'
        f"<section><h2>When people left</h2>{departures_html}</section></div>",
        f"<section><h2>On the roads</h2>{roads_html}</section>" if roads_html else "",
        f"<section><h2>Transit, parking, bikes and walking</h2>{services_html}</section>"
        if services_html
        else "",
        f"<section><h2>Flows</h2>{figs}</section>" if figs else "",
        f"<section><h2>Convergence</h2>{conv_html}</section>",  # the chart beside its table
        f'<div class="grid2"><section><h2>Computation time</h2>{time_html}</section>'
        f"<section><h2>Settings</h2>{settings_html}</section></div>",
    ]
    page = f"""<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<meta name="generator" content="{generator}"><title>{html.escape(title)}</title>
<style>{css}</style></head><body><main>
<h1>{html.escape(title)}</h1><p class="sub">{html.escape(note)}</p>
{meta}
<div class="tiles">{"".join(tiles)}</div>
{"".join(s for s in sections if s)}
<footer>Every number here is also in the run's files (kpis.parquet, link_bins.parquet, timings.csv, manifest.json).</footer>
</main></body></html>"""
    out = Path(run.timings_path).parent / "summary.html" if path is None else Path(path)
    out.write_text(page, encoding="utf-8")
    timings_page = f"""<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<meta name="generator" content="{generator}"><title>{html.escape(title)}: timings</title>
<style>{css}</style></head><body><main>
<h1>{html.escape(title)}: where the time went</h1><p class="sub">Wall-clock seconds by stage, this machine.</p>
{meta}
{timings_body}
<footer>The same numbers are in timings.csv beside this page; the summary is <a href="summary.html">summary.html</a>.</footer>
</main></body></html>"""
    (out.parent / "timings.html").write_text(timings_page, encoding="utf-8")
    if not quiet:
        show_path("Summary page", out)
    return out


def _css(paper: Any, night: Any) -> str:
    """The page's style: cards on the theme's surface, light and dark."""
    return f"""
:root {{ --surface:{paper.surface}; --ink:{paper.ink}; --ink2:{paper.ink2}; --muted:{paper.muted};
  --base:{paper.base}; --card:#ffffff; --accent:{ION[500]}; }}
* {{ box-sizing: border-box; }}
body {{ margin:0; background:var(--surface); color:var(--ink); font:14px/1.45 Inter, "Helvetica Neue",
  "Segoe UI", sans-serif; }}
main {{ max-width:1180px; margin:0 auto; padding:24px 16px 40px; }}
h1 {{ font-size:26px; margin:0; }} .sub {{ color:var(--ink2); margin:4px 0 0; }}
.meta {{ color:var(--muted); font:12px/1.4 "JetBrains Mono", Menlo, monospace; margin-top:6px; }}
.tiles {{ display:grid; grid-template-columns:repeat(auto-fit,minmax(150px,1fr)); gap:10px; margin:20px 0; }}
.tile {{ background:var(--card); border:1px solid var(--base); border-radius:10px; padding:12px 14px; }}
.tl {{ color:var(--muted); font-size:11px; text-transform:uppercase; letter-spacing:.08em; }}
.tv {{ font-size:24px; font-weight:700; margin-top:2px; }} .nw {{ white-space:nowrap; }} .tu {{ font-size:13px; font-weight:500;
  color:var(--ink2); white-space:nowrap; }} .tn {{ color:var(--muted); font-size:12px; }}
.grid2 {{ display:grid; grid-template-columns:1fr 1fr; gap:16px; }}
@media (max-width:860px) {{ .grid2 {{ grid-template-columns:1fr; }} }}
section {{ background:var(--card); border:1px solid var(--base); border-radius:12px; padding:14px 16px;
  margin-top:16px; }}
section .grid2 {{ gap:20px; }}
h2 {{ font-size:12px; text-transform:uppercase; letter-spacing:.09em; color:var(--muted); margin:0 0 10px; }}
h3 {{ font-size:12px; text-transform:uppercase; letter-spacing:.06em; color:var(--muted); margin:14px 0 6px; font-weight:600; }}
table {{ width:100%; border-collapse:collapse; font-size:13px; }}
th {{ text-align:left; color:var(--muted); font-weight:600; border-bottom:1px solid var(--base); padding:5px 6px; }}
td {{ border-bottom:1px solid color-mix(in srgb, var(--base) 50%, transparent); padding:5px 6px; }}
.n {{ text-align:right; font-variant-numeric:tabular-nums; }} td.v2 {{ white-space:nowrap; }}
.sw {{ display:inline-block; width:10px; height:10px; border-radius:2px; background:var(--c);
  margin-right:7px; vertical-align:-1px; }}
.stack {{ display:flex; gap:2px; height:14px; margin:4px 0 12px; }}
.seg {{ background:var(--c); border-radius:3px; min-width:2px; }} svg .seg {{ fill:var(--c); }}
.chart {{ width:100%; height:auto; }} .grid {{ stroke:var(--base); stroke-width:1; }}
.ax {{ fill:var(--muted); font-size:11px; }} .ln {{ fill:none; stroke:var(--accent); stroke-width:2; }}
.dot {{ fill:var(--accent); stroke:var(--card); stroke-width:2; }}
.bars {{ display:grid; gap:5px; }} .bar {{ display:grid; grid-template-columns:200px 1fr 64px; gap:8px;
  align-items:center; font-size:13px; }} .bl {{ color:var(--ink2); }}
.bt {{ background:color-mix(in srgb, var(--base) 35%, transparent); border-radius:4px; height:12px; }}
.bf {{ background:var(--accent); height:12px; border-radius:4px; }}
.bv {{ text-align:right; font-variant-numeric:tabular-nums; }}
.note {{ color:var(--muted); font-size:12px; margin:8px 0 0; }} a {{ color:var(--accent); }}
img {{ width:100%; border-radius:8px; }} img.dark {{ display:none; }}
footer {{ color:var(--muted); font:12px "JetBrains Mono", Menlo, monospace; margin-top:22px; }}
.grid2 > section {{ min-width:0; }} td.v {{ word-break:break-all; font:12px "JetBrains Mono", Menlo, monospace; }}
@media (prefers-color-scheme: dark) {{ :root {{ --surface:{night.surface}; --ink:{night.ink};
  --ink2:{night.ink2}; --muted:{night.muted}; --base:{night.base}; --card:#101b2d; --accent:{ION[400]}; }}
  .sw, .seg {{ background: var(--cn) !important; }} svg .seg {{ fill: var(--cn) !important; }}
  img.day {{ display:none; }} img.dark {{ display:block; }} }}
"""
