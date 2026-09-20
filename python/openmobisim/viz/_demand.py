"""Demand on the map: origin-destination desire lines and the matrix (S162, S163).

Demand here is the same trips table a ``Scenario`` takes (rows of
``(traveller_id, trip_seq, origin_lon, origin_lat, destination_lon,
destination_lat, departure_time_s, user_class, weight)``). Until zones exist,
places are square cells of `cell_m` metres; when zones arrive they replace the
cells and everything else stays.
"""

from __future__ import annotations

from typing import Any

import numpy as np

from openmobisim import __version__
from openmobisim.viz import _geometry as geo
from openmobisim.viz._figure import (
    Page,
    draw_furniture,
    load_matplotlib,
    nice_number,
    ramp_rgb,
)
from openmobisim.viz._style import Theme, get_theme

__all__ = ["chart_demand_matrix", "map_demand", "od_cells"]


def od_cells(trips: list[tuple], cell_m: float) -> dict[str, Any]:
    """Aggregate trips into origin-destination cell pairs.

    Returns a dict with ``origin`` and ``destination`` as ``(k, 2)`` integer
    cell indices, ``trips`` the (weighted) number of trips per pair, ``lon0``
    and ``lat0`` the projection centre, and ``intra`` the weighted number of
    trips that start and end in one cell. Pairs are sorted by decreasing trips.
    """
    if not trips:
        raise ValueError("no trips to draw")
    if cell_m <= 0:
        raise ValueError(f"cell_m must be positive, got {cell_m}")
    a = np.array(
        [(r[2], r[3], r[4], r[5], 1.0 if r[8] is None else float(r[8])) for r in trips],
        dtype=np.float64,
    )
    lon0 = float((a[:, 0].mean() + a[:, 2].mean()) / 2)
    lat0 = float((a[:, 1].mean() + a[:, 3].mean()) / 2)
    o = geo.project_lonlat(a[:, 0:2], lon0, lat0)
    d = geo.project_lonlat(a[:, 2:4], lon0, lat0)
    cells = np.floor(np.column_stack([o, d]) / cell_m).astype(np.int64)
    unique, inverse = np.unique(cells, axis=0, return_inverse=True)
    weight = np.bincount(inverse.reshape(-1), weights=a[:, 4], minlength=len(unique))
    same = (unique[:, 0] == unique[:, 2]) & (unique[:, 1] == unique[:, 3])
    keep = ~same
    order = np.argsort(-weight[keep], kind="stable")
    return {
        "origin": unique[keep][order][:, 0:2],
        "destination": unique[keep][order][:, 2:4],
        "trips": weight[keep][order],
        "intra": float(weight[same].sum()),
        "lon0": lon0,
        "lat0": lat0,
        "cell_m": cell_m,
    }


def _centre(cell: np.ndarray, cell_m: float) -> np.ndarray:
    return (cell.astype(np.float64) + 0.5) * cell_m


def map_demand(
    trips: list[tuple],
    *,
    network: Any | None = None,
    cell_m: float = 500.0,
    top: int = 300,
    theme: str | Theme = "paper",
    title: str | None = None,
    subtitle: str | None = None,
    note: str | None = None,
    size: tuple[float, float] = (16.0, 9.0),
    dpi: int = 120,
    path: str | None = None,
) -> Any:
    """Draw demand as origin-destination desire lines on a map.

    Each line runs from an origin cell to a destination cell and bends to the
    **right** of its direction, so a pair and its reverse are two separate
    curves. Width and colour are the number of trips (traveller weights
    included); the largest `top` pairs are drawn. Trips that start and end in
    one cell are counted in the subtitle, not drawn.

    Args:
        trips: Rows in the scenario's trips schema.
        network: If given, its links are drawn quietly underneath as a canvas.
        cell_m: Side of the square cells that stand in for zones, in metres.
        top: How many of the largest origin-destination pairs to draw.
        theme: ``"paper"`` or ``"night"``.
        title: Figure title.
        subtitle: A line under the title; defaults to the counts.
        note: A third, italic line.
        size: Figure size in inches.
        dpi: Dots per inch.
        path: If given, also save the figure there.

    Returns:
        A ``matplotlib.figure.Figure``.

    Raises:
        ValueError: If there are no trips.
        ImportError: If matplotlib is not installed (``openmobisim[viz]``).
    """
    load_matplotlib()
    from matplotlib.collections import LineCollection

    th = get_theme(theme)
    od = od_cells(trips, cell_m)
    lon0, lat0 = od["lon0"], od["lat0"]
    total = float(od["trips"].sum() + od["intra"])
    o = _centre(od["origin"][:top], cell_m)
    d = _centre(od["destination"][:top], cell_m)
    w = od["trips"][:top]

    everything = np.vstack([o, d]) if len(o) else np.zeros((1, 2))
    canvas = None
    if network is not None:
        coords, offsets = network.link_geometry()
        pts = geo.project_lonlat(coords, lon0, lat0)
        canvas = geo.polylines(pts, offsets, np.arange(network.link_count))
        everything = np.vstack([everything, pts[:: max(len(pts) // 20000, 1)]])
    lo, hi = np.percentile(everything, 1, axis=0), np.percentile(everything, 99, axis=0)
    pad = (hi - lo) * 0.06
    page = Page.new(th, size, dpi, lo - pad, hi + pad)
    mpp = page.metres_per_point

    if canvas is not None:
        page.ax.add_collection(
            LineCollection(canvas, colors=th.base, linewidths=0.35, capstyle="round", zorder=1)
        )
    wmax = float(w.max()) if len(w) else 1.0
    frac = np.sqrt(w / wmax)
    width_pt = 0.6 + 5.4 * frac
    curves = geo.bezier_curves(o, d)
    samples = curves.shape[1]
    segments = np.stack([curves[:, :-1, :], curves[:, 1:, :]], axis=2).reshape(-1, 2, 2)
    order = np.argsort(w, kind="stable")  # the largest on top
    rank = np.empty(len(w), dtype=np.int64)
    rank[order] = np.arange(len(w))
    per_segment = np.repeat(np.arange(len(w)), samples - 1)
    seg_order = np.argsort(rank[per_segment], kind="stable")
    segments, per_segment = segments[seg_order], per_segment[seg_order]
    colours = ramp_rgb(th.ramp_volume, frac)[per_segment]
    surface = ramp_rgb((th.surface, th.surface), np.zeros(1))[0]
    if th.glow:
        page.ax.add_collection(
            LineCollection(
                segments,
                colors=surface + (colours - surface) * 0.14,
                linewidths=width_pt[per_segment] * 2.8 + 1.2,
                capstyle="round",
                zorder=2,
            )
        )
    else:
        page.ax.add_collection(
            LineCollection(
                segments,
                colors=[surface],
                linewidths=width_pt[per_segment] + 1.2,
                capstyle="round",
                zorder=2,
            )
        )
    page.ax.add_collection(
        LineCollection(
            segments, colors=colours, linewidths=width_pt[per_segment], capstyle="round", zorder=3
        )
    )
    # Cells that start or end many of the drawn trips.
    cells = np.vstack([od["origin"][:top], od["destination"][:top]])
    both = np.concatenate([w, w])
    unique, inverse = np.unique(cells, axis=0, return_inverse=True)
    mass = np.bincount(inverse.reshape(-1), weights=both, minlength=len(unique))
    radius_pt = 1.2 + 4.6 * np.sqrt(mass / mass.max())
    centres = _centre(unique, cell_m)
    page.ax.scatter(
        centres[:, 0],
        centres[:, 1],
        s=(2 * radius_pt) ** 2,
        c=[th.ink2],
        edgecolors=[th.surface],
        linewidths=0.8,
        zorder=4,
    )

    default_subtitle = (
        f"{total:,.0f} trips in {len(od['trips']):,} cell pairs ({cell_m:g} m cells) · "
        f"{od['intra']:,.0f} within one cell, not drawn · "
        "lines bend to the right of their direction"
    )
    provenance = (
        f"openmobisim {__version__} · demand · {len(w)} of {len(od['trips'])} pairs drawn · "
        f"cells {cell_m:g} m"
    )
    sample_values = [nice_number(wmax / 10), nice_number(wmax / 3), nice_number(wmax)]
    samples_pt = [(v, 0.6 + 5.4 * float(np.sqrt(min(v, wmax) / wmax))) for v in sample_values]
    draw_furniture(
        page,
        title=title or "Demand: where trips go",
        subtitle=subtitle or default_subtitle,
        note=note,
        provenance=provenance,
        width_legend=("Trips per pair", samples_pt, "trips"),
        colour_legend=("Trips per pair", th.ramp_volume, "few", f"{wmax:g}"),
    )
    del mpp
    if path is not None:
        page.fig.savefig(path, facecolor=th.surface)
    return page.fig


def chart_demand_matrix(
    trips: list[tuple],
    *,
    cell_m: float = 500.0,
    top: int = 30,
    theme: str | Theme = "paper",
    title: str | None = None,
    note: str | None = None,
    size: tuple[float, float] = (16.0, 9.0),
    dpi: int = 120,
    path: str | None = None,
) -> Any:
    """Draw the origin-destination matrix as a heat-map.

    Rows are origins and columns destinations, for the `top` cells that start
    or end the most trips, ordered by their total. Colour is the number of
    trips on the *ion* ramp. Cells stand in for zones until zones exist.

    Args:
        trips: Rows in the scenario's trips schema.
        cell_m: Side of the square cells that stand in for zones, in metres.
        top: How many of the busiest cells to show.
        theme: ``"paper"`` or ``"night"``.
        title: Figure title.
        note: A third, italic line.
        size: Figure size in inches.
        dpi: Dots per inch.
        path: If given, also save the figure there.

    Returns:
        A ``matplotlib.figure.Figure``.

    Raises:
        ValueError: If there are no trips.
        ImportError: If matplotlib is not installed (``openmobisim[viz]``).
    """
    load_matplotlib()
    from openmobisim.viz._figure import font_mono_name

    th = get_theme(theme)
    od = od_cells(trips, cell_m)
    cells = np.vstack([od["origin"], od["destination"]])
    unique, inverse = np.unique(cells, axis=0, return_inverse=True)
    inverse = inverse.reshape(-1)
    k = len(od["trips"])
    o_idx, d_idx = inverse[:k], inverse[k:]
    mass = np.bincount(inverse, weights=np.concatenate([od["trips"], od["trips"]]))
    rank_of = np.empty(len(unique), dtype=np.int64)
    rank_of[np.argsort(-mass, kind="stable")] = np.arange(len(unique))
    n = int(min(top, len(unique)))
    ro, rd = rank_of[o_idx], rank_of[d_idx]
    inside = (ro < n) & (rd < n)
    matrix = np.zeros((n, n))
    np.add.at(matrix, (ro[inside], rd[inside]), od["trips"][inside])
    shown = float(matrix.sum())
    total = float(od["trips"].sum())

    page = Page.new(th, size, dpi, np.array([0.0, 0.0]), np.array([float(n), float(n)]))
    page.ax.set_axis_off()
    fraction = np.sqrt(matrix / matrix.max()) if matrix.max() > 0 else matrix
    image = ramp_rgb(th.ramp_volume, fraction.reshape(-1)).reshape(n, n, 3)
    image[matrix == 0] = ramp_rgb((th.base, th.base), np.zeros(1))[0]
    page.ax.imshow(image, extent=(0, n, 0, n), origin="upper", interpolation="nearest", zorder=1)
    page.ax.set_xlim(*page.ax.get_xlim())
    mono = font_mono_name()
    step = max(n // 10, 1)
    for i in range(0, n, step):
        page.ax.text(
            -0.4,
            n - i - 0.5,
            f"{i + 1}",
            ha="right",
            va="center",
            fontsize=8,
            color=th.muted,
            fontfamily=mono,
        )
        page.ax.text(
            i + 0.5,
            n + 0.4,
            f"{i + 1}",
            ha="center",
            va="bottom",
            fontsize=8,
            color=th.muted,
            fontfamily=mono,
        )
    page.ax.text(
        n / 2, n + 1.8, "destination cell, busiest first", ha="center", fontsize=9.5, color=th.ink2
    )
    page.ax.text(
        -2.4,
        n / 2,
        "origin cell, busiest first",
        ha="center",
        va="center",
        rotation=90,
        fontsize=9.5,
        color=th.ink2,
    )
    wmax = float(matrix.max()) if matrix.max() > 0 else 1.0
    draw_furniture(
        page,
        title=title or "Origin–destination matrix",
        subtitle=(
            f"the {n} busiest of {len(unique)} cells ({cell_m:g} m) · "
            f"{shown:,.0f} of {total:,.0f} inter-cell trips shown"
        ),
        note=note,
        provenance=f"openmobisim {__version__} · demand · cells {cell_m:g} m",
        colour_legend=("Trips per pair", th.ramp_volume, "0", f"{wmax:g}"),
    )
    if path is not None:
        page.fig.savefig(path, facecolor=th.surface)
    return page.fig
