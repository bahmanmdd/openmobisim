"""Transit on the map: lines, stops as hubs, passengers on board (V2).

Every run's hop from one call to the next is a straight segment from stop to
stop (a feed's shapes are not read yet), coloured by the kind of service or, asked
for, by its line (lines sharing a stretch drawn side by side), as wide as the
passengers the day's runs carried over it; stops are hubs, sized by their
boardings, and named on a small network. The hops are ribbons in the
link map's own idiom: the same width rule, the two directions side by side on the
driver's side, a halo, the legend block and the three-line footer.
"""

from __future__ import annotations

from typing import Any

import numpy as np

from openmobisim import __version__
from openmobisim.viz import _geometry as geo
from openmobisim.viz._canvas import Canvas
from openmobisim.viz._figure import (
    SIZE_CM,
    Page,
    draw_furniture,
    load_matplotlib,
    nice_number,
    rgb,
    save_figure,
)
from openmobisim.viz._link import _WIDTH_MIN_PT, _WIDTH_SPAN_PT
from openmobisim.viz._provenance import network_source, run_identity
from openmobisim.viz._route import _ROUTE_NIGHT, _ROUTE_PAPER
from openmobisim.viz._style import KINDS, Theme, get_theme

__all__ = ["map_transit"]


def map_transit(
    run: Any,
    *,
    theme: str | Theme = "paper",
    colour: str = "kind",
    stop_labels: bool | None = None,
    title: str | None = None,
    subtitle: str | None = None,
    note: str | None = None,
    size_cm: tuple[float, float] = SIZE_CM,
    dpi: int = 120,
    credit: str | None = None,
    logo: bool = True,
    path: str | None = None,
) -> Any:
    """Draw a run's transit: lines by kind, width = passengers on board, stops as hubs.

    Each hop of every run, from one stop to the next, is a straight ribbon
    coloured by its kind of service (rail, metro, tram, bus, ferry, other) and as
    wide as the passengers on board over the whole day, summed over the runs that
    make it; the two directions side by side, each on the driver's side, as on the
    link map; the busiest on top. Stops are drawn as hubs, sized by boardings. The
    run's network is drawn quietly underneath.

    Args:
        run: A ``Run`` of a scenario with ``transit``.
        theme: ``"paper"`` or ``"night"``.
        colour: ``"kind"``, each kind of service in its colour, or ``"line"``, each line
            in its own (eight colours, repeating past eight lines); lines that share a
            stretch are then drawn side by side, each right of its direction of travel.
        stop_labels: Write each stop's name beside it. ``None`` (the default) names them
            on a network of at most 40 stops, where the names fit.
        title: Figure title.
        subtitle: A line under the title; defaults to the timetable's counts.
        note: A third, italic line; defaults to the boardings.
        size_cm: Figure size, width and height in centimetres.
        dpi: Resolution in dots per inch (25.4 mm), the image-file convention.
        credit: A line of your own before the logo (a name, an institution).
        logo: Draw the openmobisim logo at the bottom right.
        path: If given, also save the figure there.

    Returns:
        A ``matplotlib.figure.Figure``.

    Raises:
        ValueError: If the run had no timetable, or `colour` is unknown.
        ImportError: If matplotlib is not installed (``openmobisim[viz]``).
    """
    load_matplotlib()
    from matplotlib.collections import LineCollection

    calls = run.transit_calls()
    transit = run.transit
    if calls is None or transit is None:
        raise ValueError("map_transit needs a run of a scenario with transit")
    if colour not in ("kind", "line"):
        raise ValueError(f'colour must be "kind" or "line", got {colour!r}')
    by_line = colour == "line"
    th = get_theme(theme)
    stops = transit.stops()
    lonlat = np.column_stack([stops["lon"], stops["lat"]])
    lon0, lat0 = float(lonlat[:, 0].mean()), float(lonlat[:, 1].mean())
    xy_stop = geo.project_lonlat(lonlat, lon0, lat0)
    # A station's platforms and a street's two bus stops are one place on the map (S282, the
    # user: the two directions of a rail hop made a V): stops with the same name within 300 m
    # are drawn as one node, so a hop's two directions share their ends and lie side by side.
    node_of, xy, names = _merge_stops(stops, xy_stop)
    index = {s: node_of[i] for i, s in enumerate(stops["stop_id"])}

    # Passengers on board between consecutive calls, summed per (kind or line, from, to).
    runs, kinds, stop_ids = calls["run"], calls["kind"], calls["stop_id"]
    group = calls["line"] if by_line else kinds
    on_board = np.asarray(calls["on_board"])
    loads: dict[tuple[str, int, int], float] = {}
    line_runs: dict[str, set[str]] = {}
    for i in range(len(runs) - 1):
        a, b = index[stop_ids[i]], index[stop_ids[i + 1]]
        if runs[i] == runs[i + 1] and a != b:
            key = (group[i], a, b)
            loads[key] = loads.get(key, 0.0) + float(on_board[i])
            line_runs.setdefault(group[i], set()).add(runs[i])
    boardings = np.zeros(len(xy))
    np.add.at(boardings, [index[s] for s in stop_ids], np.asarray(calls["boardings"]))

    everything = xy
    canvas = None
    network = run.network
    if network is not None:
        canvas = Canvas.from_network(network, lon0, lat0)
        everything = canvas.points[:: max(len(canvas.points) // 20000, 1)]
    lo, hi = np.percentile(everything, 0.5, axis=0), np.percentile(everything, 99.5, axis=0)
    # Every stop inside the frame: a line may run beyond the roads (a tram to a stop off them).
    lo, hi = np.minimum(lo, xy.min(axis=0)), np.maximum(hi, xy.max(axis=0))
    pad = (hi - lo) * 0.04
    page = Page.new(th, size_cm, dpi, lo - pad, hi + pad)
    k = page.k
    if canvas is not None:
        page.ax.add_collection(canvas.base(th.base, 0.3 * k))
    keys = sorted(loads, key=lambda key: (loads[key], key))  # the busiest on top
    top = 1.0
    width_of = None
    if keys:
        values = np.array([loads[key] for key in keys])
        # The link map's width rule (S281): the square root of the load up to the busiest hop.
        top = float(values.max()) or 1.0

        def width_of(v: Any) -> Any:
            return (_WIDTH_MIN_PT + _WIDTH_SPAN_PT * np.sqrt(np.minimum(v, top) / top)) * k

        width = width_of(values)
        segments = np.array([[xy[key[1]], xy[key[2]]] for key in keys])
        mpp = page.metres_per_point
        if by_line:
            palette = _ROUTE_NIGHT if th.glow else _ROUTE_PAPER
            lines = sorted(line_runs)
            colours = np.array([rgb(palette[lines.index(key[0]) % len(palette)]) for key in keys])
            segments = _side_by_side(keys, segments, 7.5 * k * mpp)
        else:
            colours = np.array([rgb(th.kinds[KINDS.index(key[0])]) for key in keys])
            # Each direction on the driver's side, by its own width, as the link map does.
            segments = _offset_right(segments, (width / 2 + 0.25) * mpp)
        surface = rgb(th.surface)
        if th.glow:
            page.ax.add_collection(
                LineCollection(
                    segments,
                    colors=surface + (colours - surface) * 0.14,
                    linewidths=width * 2.8 + 1.2,
                    capstyle="round",
                    zorder=2,
                )
            )
        else:
            page.ax.add_collection(
                LineCollection(
                    segments,
                    colors=[surface],
                    linewidths=width + 1.4,
                    capstyle="round",
                    zorder=2,
                )
            )
        page.ax.add_collection(
            LineCollection(segments, colors=colours, linewidths=width, capstyle="round", zorder=3)
        )
    most = float(boardings.max()) or 1.0
    radius = (1.0 + 4.0 * np.sqrt(boardings / most)) * k
    page.ax.scatter(
        xy[:, 0],
        xy[:, 1],
        s=(2 * radius) ** 2,
        c=[th.surface],
        edgecolors=[th.ink2],
        linewidths=0.6 * k,
        zorder=4,
    )

    if stop_labels is None:
        stop_labels = len(xy) <= 40
    if stop_labels:
        from matplotlib import patheffects

        halo = [patheffects.withStroke(linewidth=2.5 * k, foreground=th.surface)]
        for i, name in enumerate(names):
            page.ax.annotate(
                name,
                xy[i],
                xytext=(6 * k, 5 * k),
                textcoords="offset points",
                fontsize=9 * k,
                color=th.ink2,
                path_effects=halo,
                zorder=5,
            )
    if by_line:
        palette = _ROUTE_NIGHT if th.glow else _ROUTE_PAPER
        lines = sorted(line_runs)
        legend = [
            (
                rgb(palette[j % len(palette)]),
                f"{line:<6} {len(line_runs[line]):>6,} runs".replace(",", "\u202f"),
            )
            for j, line in enumerate(lines)
        ]
        if len(lines) > 8:
            legend = legend[:7] + [(rgb(th.muted), f"+ {len(lines) - 7} more lines")]
    else:
        count = transit.runs_by_kind()
        legend = [
            (rgb(th.kinds[i]), f"{kind:<6} {count[kind]:>6,} runs".replace(",", "\u202f"))
            for i, kind in enumerate(KINDS)
            if kind in count
        ]
    total = float(np.asarray(calls["boardings"]).sum())
    samples = []
    if width_of is not None:
        for v in (nice_number(top / 10), nice_number(top / 3), nice_number(top)):
            samples.append((v, float(width_of(np.array(v)))))
    # The footer's first line: the day's numbers (S281).
    busiest = max(loads.values()) if loads else 0.0
    summary = run.transit_summary or {}
    numbers = [
        f"{total:,.0f} boardings".replace(",", "\u202f"),
        f"{transit.run_count:,} runs on {transit.route_count:,} lines".replace(",", "\u202f"),
        f"busiest hop {busiest:,.0f} passengers".replace(",", "\u202f"),
    ]
    if summary.get("bus_runs_on_roads"):
        delay = summary.get("bus_delay_mean_s", 0.0)
        on_roads = f"{summary['bus_runs_on_roads']:,.0f}".replace(",", "\u202f")
        numbers.append(f"{on_roads} bus runs on the roads, mean delay {delay / 60:+.1f} min")
    draw_furniture(
        page,
        title=title or "Transit: lines and stops",
        subtitle=subtitle
        or (
            f"{transit.date} · straight ribbons from stop to stop, one per direction of travel · "
            "stops sized by boardings"
        ),
        note=note,
        provenance=[
            " · ".join(numbers),
            f"run {run.run_id} · {run_identity(run)}",
            f"openmobisim {__version__} · GTFS · {network_source(network)}",
        ],
        width_legend=("Width: passengers on board over the day", samples, ""),
        route_legend=legend,
        credit=credit,
        logo=logo,
        compass=True,
    )
    if path is not None:
        save_figure(page.fig, path, th.surface)
    return page.fig


def _merge_stops(
    stops: dict[str, Any], xy: np.ndarray, within_m: float = 300.0
) -> tuple[np.ndarray, np.ndarray, list[str]]:
    """Stops with the same name within ``within_m`` of each other as one node.

    Returns each stop's node, the nodes' coordinates (the mean of their stops') and their
    names.
    """
    names = stops.get("name") or stops["stop_id"]
    key = [str(n or stops["stop_id"][i]).strip().casefold() for i, n in enumerate(names)]
    node_of = np.full(len(key), -1, dtype=np.int64)
    members: list[list[int]] = []
    by_name: dict[str, list[int]] = {}
    for i in np.argsort(key, kind="stable"):
        i = int(i)
        for n in by_name.get(key[i], ()):
            if float(np.hypot(*(xy[members[n]].mean(axis=0) - xy[i]))) <= within_m:
                node_of[i] = n
                members[n].append(i)
                break
        else:
            node_of[i] = len(members)
            by_name.setdefault(key[i], []).append(len(members))
            members.append([i])
    nodes = np.array([xy[m].mean(axis=0) for m in members]) if members else xy[:0]
    labels = [str(names[m[0]] or stops["stop_id"][m[0]]) for m in members]
    return node_of, nodes, labels


def _offset_right(segments: np.ndarray, distance: np.ndarray) -> np.ndarray:
    """Shift each straight segment right of its direction of travel by ``distance`` metres."""
    d = segments[:, 1] - segments[:, 0]
    length = np.hypot(d[:, 0], d[:, 1])
    length[length == 0] = 1.0
    right = np.column_stack([d[:, 1], -d[:, 0]]) / length[:, None]
    return segments + (right * distance[:, None])[:, None, :]


def _side_by_side(
    keys: list[tuple[str, int, int]], segments: np.ndarray, step: float
) -> np.ndarray:
    """Shift each line's segment right of its direction of travel, ``step`` metres a lane.

    Lines that share a stretch (between the same two stops, either way) go one beside the other.
    """
    shared: dict[tuple[int, int], list[str]] = {}
    for line, a, b in keys:
        stretch = shared.setdefault((min(a, b), max(a, b)), [])
        if line not in stretch:
            stretch.append(line)
    out = segments.copy()
    for n, (line, a, b) in enumerate(keys):
        d = segments[n, 1] - segments[n, 0]
        length = float(np.hypot(*d)) or 1.0
        right = np.array([d[1], -d[0]]) / length
        lane = sorted(shared[(min(a, b), max(a, b))]).index(line)
        out[n] += right * step * (lane + 0.5)
    return out
