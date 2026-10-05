"""Transit on the map: lines, stops as hubs, passengers on board (S199, V2).

Every run's hop from one call to the next is a straight segment from stop to
stop (a feed's shapes are not read yet), coloured by the kind of service, as
wide as the passengers the day's runs carried over it; stops are hubs, sized by
their boardings. The look was put to the user with the first figure and accepted
(S200).
"""

from __future__ import annotations

from typing import Any

import numpy as np

from openmobisim import __version__
from openmobisim.viz import _geometry as geo
from openmobisim.viz._figure import SIZE_CM, Page, draw_furniture, load_matplotlib, rgb
from openmobisim.viz._provenance import run_identity
from openmobisim.viz._style import Theme, get_theme

__all__ = ["map_transit"]

#: The kinds of service, in the order the theme's colours follow.
KINDS = ("rail", "metro", "tram", "bus", "ferry", "other")


def map_transit(
    run: Any,
    *,
    theme: str | Theme = "paper",
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

    Each hop of every run, from one stop to the next, is a straight segment
    coloured by its kind of service (rail, metro, tram, bus, ferry, other) and as
    wide as the passengers on board over the whole day, summed over the runs that
    make it; the busiest on top. Stops are drawn as hubs, sized by boardings. The
    run's network is drawn quietly underneath.

    Args:
        run: A ``Run`` of a scenario with ``transit``.
        theme: ``"paper"`` or ``"night"``.
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
        ValueError: If the run had no timetable.
        ImportError: If matplotlib is not installed (``openmobisim[viz]``).
    """
    load_matplotlib()
    from matplotlib.collections import LineCollection

    calls = run.transit_calls()
    transit = run.transit
    if calls is None or transit is None:
        raise ValueError("map_transit needs a run of a scenario with transit")
    th = get_theme(theme)
    stops = transit.stops()
    index = {s: i for i, s in enumerate(stops["stop_id"])}
    lonlat = np.column_stack([stops["lon"], stops["lat"]])
    lon0, lat0 = float(lonlat[:, 0].mean()), float(lonlat[:, 1].mean())
    xy = geo.project_lonlat(lonlat, lon0, lat0)

    # Passengers on board between consecutive calls, summed per (kind, from, to).
    runs, kinds, stop_ids = calls["run"], calls["kind"], calls["stop_id"]
    on_board = np.asarray(calls["on_board"])
    loads: dict[tuple[str, int, int], float] = {}
    for i in range(len(runs) - 1):
        a, b = index[stop_ids[i]], index[stop_ids[i + 1]]
        if runs[i] == runs[i + 1] and a != b:
            key = (kinds[i], a, b)
            loads[key] = loads.get(key, 0.0) + float(on_board[i])
    boardings = np.zeros(len(index))
    np.add.at(boardings, [index[s] for s in stop_ids], np.asarray(calls["boardings"]))

    everything = xy
    canvas = None
    network = run.network
    if network is not None:
        coords, offsets = network.link_geometry()
        pts = geo.project_lonlat(coords, lon0, lat0)
        canvas = geo.polylines(pts, offsets, np.arange(network.link_count))
        everything = pts[:: max(len(pts) // 20000, 1)]
    lo, hi = np.percentile(everything, 0.5, axis=0), np.percentile(everything, 99.5, axis=0)
    pad = (hi - lo) * 0.04
    page = Page.new(th, size_cm, dpi, lo - pad, hi + pad)
    k = page.k
    if canvas is not None:
        page.ax.add_collection(
            LineCollection(canvas, colors=th.base, linewidths=0.3 * k, capstyle="round", zorder=1)
        )
    keys = sorted(loads, key=lambda key: (loads[key], key))  # the busiest on top
    if keys:
        values = np.array([loads[key] for key in keys])
        top = float(values.max()) or 1.0
        width = (0.5 + 6.0 * np.sqrt(values / top)) * k
        segments = np.array([[xy[key[1]], xy[key[2]]] for key in keys])
        colours = np.array([rgb(th.kinds[KINDS.index(key[0])]) for key in keys])
        page.ax.add_collection(
            LineCollection(
                segments,
                colors=[rgb(th.surface)],
                linewidths=width + 1.0 * k,
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

    count = transit.runs_by_kind()
    legend = [
        (rgb(th.kinds[i]), f"{kind:<6} {count[kind]:>6,} runs")
        for i, kind in enumerate(KINDS)
        if kind in count
    ]
    total = float(np.asarray(calls["boardings"]).sum())
    draw_furniture(
        page,
        title=title or "Transit: lines and stops",
        subtitle=subtitle
        or (
            f"{transit.date} · {transit.run_count:,} runs, {transit.stop_count:,} stops · "
            "width: passengers on board"
        ),
        note=note
        or f"{total:,.0f} boardings · stops sized by boardings · straight lines stop to stop",
        provenance=f"{__version__} · {run_identity(run)} · GTFS and © OpenStreetMap contributors",
        route_legend=legend,
        credit=credit,
        logo=logo,
        compass=True,
    )
    if path is not None:
        page.fig.savefig(path, facecolor=th.surface)
    return page.fig
