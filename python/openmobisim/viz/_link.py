"""``map_link``: one ribbon per direction of every link, coloured by delay (S162, S163).

A two-way street is two ribbons side by side, each on its own driver's side
(right-hand traffic), so the two directions never overlap and both read.
Width is volume, colour is delay over free flow; free flow recedes into the
canvas and congestion glows.
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
    rgb,
)
from openmobisim.viz._provenance import run_identity
from openmobisim.viz._style import Theme, get_theme

__all__ = ["link_table", "map_link"]

_COLOUR_WORDS = {
    "delay": "delay over free flow",
    "volume": "volume",
    "volume_capacity": "volume over capacity",
}
_LEVEL_NAMES = {0: "free flow", 2: "point queue", 3: "spatial queue", 4: "full"}
#: Ribbon width in points at zero and at maximum volume, before the view scale.
_WIDTH_MIN_PT, _WIDTH_SPAN_PT = 0.5, 5.2
#: No ribbon is thinner than this many points (at 16 inches wide), whatever the
#: view scale: a link with any traffic must stay visible.
_WIDTH_FLOOR_PT = 0.9
#: A delay of this share of the free-flow time is the top of the colour ramp.
_DELAY_TOP = 0.70
#: Below this delay a ribbon counts as free flow.
_DELAY_FREE = 0.04


def link_table(
    bins: Any, n_links: int, free_flow_s: np.ndarray, *, first: int | None, stop: int | None
) -> dict[str, np.ndarray]:
    """Per-link volume and delay over the bins ``first <= bin < stop``.

    Returns arrays of length `n_links`: ``volume`` in PCU/h averaged over the
    selected bins, ``mean_s`` the PCU-weighted mean traversal time (NaN where
    nothing crossed), and ``delay`` in ``[0, 1)`` as the share of the mean time
    that is not free flow. ``first``/``stop`` of ``None`` select every bin from
    the first to the last that saw traffic.
    """
    b, link = bins.bins().astype(np.int64), bins.links().astype(np.int64)
    pcu, pcu_s = bins.pcu(), bins.pcu_seconds()
    if first is None:
        first = int(b.min()) if len(b) else 0
    if stop is None:
        stop = int(b.max()) + 1 if len(b) else first + 1
    keep = (b >= first) & (b < stop)
    total = np.bincount(link[keep], weights=pcu[keep], minlength=n_links)
    seconds = np.bincount(link[keep], weights=pcu_s[keep], minlength=n_links)
    span_s = max(stop - first, 1) * bins.bin_seconds
    volume = total / span_s * 3600.0
    with np.errstate(invalid="ignore", divide="ignore"):
        mean_s = np.where(total > 0, seconds / total, np.nan)
        delay = np.where(total > 0, 1.0 - np.minimum(free_flow_s / mean_s, 1.0), 0.0)
    return {"volume": volume, "mean_s": mean_s, "delay": delay}


def _bin_range(bins_arg: int | tuple[int, int] | None) -> tuple[int | None, int | None]:
    if bins_arg is None:
        return None, None
    if isinstance(bins_arg, int):
        return bins_arg, bins_arg + 1
    first, stop = bins_arg
    return int(first), int(stop)


def _minutes(seconds: float) -> str:
    return f"{seconds / 60:g}"


_TITLES = {
    "road": "Flow and delay by direction",
    "bike": "Bike flow by direction",
    "walk": "Walking flow by direction",
}


def map_link(
    run: Any,
    *,
    bins: int | tuple[int, int] | None = None,
    layer: str = "road",
    colour: str | None = None,
    ramp: str = "spectrum",
    theme: str | Theme = "paper",
    title: str | None = None,
    subtitle: str | None = None,
    note: str | None = None,
    view: tuple[tuple[float, float], tuple[float, float]] | None = None,
    size: tuple[float, float] = (16.0, 9.0),
    dpi: int = 120,
    scale: float | None = None,
    chevrons: bool | None = None,
    min_volume: float = 0.0,
    credit: str | None = None,
    logo: bool = True,
    path: str | None = None,
) -> Any:
    """Draw a run's traffic on its network: one ribbon per direction of every link.

    Width is volume; colour is delay over free flow (or volume, see `colour`).
    The two directions of a two-way street are drawn side by side, each on its
    own driver's side, so they never overlap. Free-flowing links recede and
    congestion glows.

    Args:
        run: A ``Run`` from ``Scenario.run()`` on a scenario built with
            ``link_bin_s``, at ``flow_level`` 2-4 for congestion to show.
        bins: Which time bins to show: ``None`` for every bin from the first to
            the last that saw traffic, an ``int`` for one bin, or
            ``(first, stop)`` for a range (``stop`` excluded).
        layer: Which layer's traffic to draw: ``"road"`` (default: cars, on the
            road network), ``"bike"`` or ``"walk"`` (each on its own layer's links,
            ``network.layer(...)``; volume counts travellers, not PCU).
        colour: What the colour shows: ``"delay"`` (delay over free flow; the
            default on the road), ``"volume"`` (the default on the bike and walk
            layers, where nothing is delayed), or ``"volume_capacity"`` (volume
            over the link's capacity, so 1 is a link at capacity and beyond it is
            over; the road only).
        ramp: For delay and volume-over-capacity, ``"spectrum"`` (default:
            blue-green for little, gold in the middle, deeper red for more) or
            ``"ember"`` (the single-hue ramp of the first version).
        theme: ``"paper"`` (print, slides) or ``"night"`` (screens, video).
        title: Figure title; defaults to a description of what is drawn.
        subtitle: A line under the title; defaults to the time window and the encodings.
        note: A third, italic line, for example to say the demand is a fixture.
        view: ``((lon_min, lat_min), (lon_max, lat_max))`` to zoom to; the whole
            network by default.
        size: Figure size in inches. 16 x 9 is a full-screen frame. The text is
            laid out for figures **8 inches wide or more**; narrower, the footer
            runs into the logo and the legends crowd (a layout for a journal
            column is planned).
        dpi: Dots per inch.
        scale: Ribbon width multiplier; by default it follows the view, thin
            for a whole region and bolder when zoomed to a town.
        chevrons: Draw a small direction triangle on each ribbon; by default
            only when the view is a few kilometres across.
        min_volume: Links carrying no more than this (PCU/h) are drawn only as
            canvas; the default 0 draws every link that saw any traffic, at least
            a fine line wide.
        credit: A line of your own before the logo (a name, an institution, your
            copyright). The logo is a signature, not a claim on your figure.
        logo: Draw the openmobisim logo at the bottom right (``False`` for none).
        path: If given, also save the figure there (PNG, SVG or PDF by extension).

    Returns:
        A ``matplotlib.figure.Figure``.

    Raises:
        ValueError: If the run has no per-link results, or `colour` is unknown.
        ImportError: If matplotlib is not installed (``openmobisim[viz]``).
    """
    load_matplotlib()
    from matplotlib.collections import LineCollection, PolyCollection

    if layer not in ("road", "bike", "walk"):
        raise ValueError(f"layer must be 'road', 'bike' or 'walk', got {layer!r}")
    road = layer == "road"
    if colour is None:
        colour = "delay" if road else "volume"
    if colour not in ("delay", "volume", "volume_capacity"):
        raise ValueError(f"colour must be 'delay', 'volume' or 'volume_capacity', got {colour!r}")
    if colour == "volume_capacity" and not road:
        raise ValueError(
            "colour='volume_capacity' is for the road: bike and walk links have no capacity"
        )
    if ramp not in ("spectrum", "ember"):
        raise ValueError(f"ramp must be 'spectrum' or 'ember', got {ramp!r}")
    link_bins = run.link_bins(layer)
    network = run.network
    if link_bins is None or network is None:
        raise ValueError(
            f"this run has no per-link results on the {layer} layer: build the scenario with "
            "link_bin_s=..., and use flow_level 2, 3 or 4 for congestion to show"
            + (
                ""
                if road
                else f"; the {layer} layer has results only if some trip's mode is {layer}"
            )
        )
    if not road:
        network = network.layer(layer)
    unit = "PCU/h" if road else "travellers/h"
    th = get_theme(theme)
    n_links = network.link_count

    coords, offsets = network.link_geometry()
    owner = geo.link_of_vertex(offsets)
    lon0, lat0 = float(coords[:, 0].mean()), float(coords[:, 1].mean())
    points = geo.project_lonlat(coords, lon0, lat0)

    first, stop = _bin_range(bins)
    table = link_table(link_bins, n_links, network.link_free_flow_s(), first=first, stop=stop)
    volume, delay = table["volume"], table["delay"]
    active = (volume > 0) & (volume >= min_volume)

    # The view, in metres.
    if view is None:
        if len(points) > 2000:
            lo, hi = np.percentile(points, 0.5, axis=0), np.percentile(points, 99.5, axis=0)
        else:
            lo, hi = points.min(axis=0), points.max(axis=0)
    else:
        lo = geo.project_lonlat(np.array([view[0]]), lon0, lat0)[0]
        hi = geo.project_lonlat(np.array([view[1]]), lon0, lat0)[0]
    width_m = float(max(hi[0] - lo[0], 1.0))
    zs = scale if scale is not None else float(np.clip(1.15 * np.sqrt(2600.0 / width_m), 0.2, 2.2))
    if chevrons is None:
        chevrons = width_m < 6000.0
    page = Page.new(th, size, dpi, lo, hi)
    mpp = page.metres_per_point

    # Width: square root of volume up to the 99th percentile of active links.
    vmax = float(np.percentile(volume[active], 99)) if active.any() else 1.0
    vmax = max(vmax, min_volume)
    frac = np.sqrt(np.minimum(volume, vmax) / vmax)
    floor = _WIDTH_FLOOR_PT * page.k
    width_pt = np.where(
        active, np.maximum((_WIDTH_MIN_PT + _WIDTH_SPAN_PT * frac) * zs * page.k, floor), 0.0
    )

    # 1. The canvas: every link, thin, in the network's own geometry.
    everything = np.arange(n_links)
    page.ax.add_collection(
        LineCollection(
            geo.polylines(points, offsets, everything),
            colors=th.base,
            linewidths=0.35 if width_m > 6000.0 else 0.6,
            capstyle="round",
            zorder=1,
        )
    )

    # 2. One ribbon per directed link, offset to the driver's side.
    offset_m = (width_pt / 2 + 0.25) * mpp
    ribbon_points = geo.offset_points(points, owner, offset_m)
    stops = th.ramp_delay if ramp == "spectrum" else th.ramp_delay_ember
    if colour == "delay":
        t = np.clip((delay - _DELAY_FREE) / _DELAY_TOP, 0.0, 1.0)
        link_rgb = ramp_rgb(stops, t)
        if ramp == "ember":  # the first version: free flow recedes as a quiet slate
            link_rgb = np.where((delay < _DELAY_FREE)[:, None], rgb(th.free)[None, :], link_rgb)
        severity = delay
        legend_stops, low, high = stops, "0%", f"{_DELAY_TOP:.0%}+"
        legend_label = "Delay over free flow"
    elif colour == "volume_capacity":
        capacity = network.link_capacity_pcu_h()
        ratio = np.where(capacity > 0, volume / np.maximum(capacity, 1e-9), 0.0)
        link_rgb = ramp_rgb(stops, np.clip(ratio, 0.0, 1.0))
        severity = ratio
        legend_stops, low, high = stops, "0", "1.0+"
        legend_label = "Volume over capacity"
    else:
        t = np.sqrt(np.minimum(volume, vmax) / vmax)
        link_rgb = ramp_rgb(th.ramp_volume, t)
        severity = volume
        legend_stops, low, high = th.ramp_volume, "0", f"{vmax:,.0f}+"
        legend_label = f"Volume per direction ({unit})"
    drawn = np.nonzero(active)[0]
    drawn = drawn[np.argsort(severity[drawn], kind="stable")]  # the worst on top
    lines = geo.polylines(ribbon_points, offsets, drawn)
    line_width, line_rgb = width_pt[drawn], link_rgb[drawn]
    surface = rgb(th.surface)
    if th.glow:
        # A soft under-stroke: the ribbon's colour blended most of the way into the surface.
        page.ax.add_collection(
            LineCollection(
                lines,
                colors=surface + (line_rgb - surface) * 0.14,
                linewidths=line_width * 2.8 + 1.2,
                capstyle="round",
                zorder=2,
            )
        )
    else:
        # A hairline halo in the surface colour separates overlapping ribbons.
        page.ax.add_collection(
            LineCollection(
                lines,
                colors=[surface],
                linewidths=line_width + 1.4,
                capstyle="round",
                zorder=2,
            )
        )
    page.ax.add_collection(
        LineCollection(lines, colors=line_rgb, linewidths=line_width, capstyle="round", zorder=3)
    )

    # 3. Direction chevrons, one per ribbon.
    if chevrons:
        long_enough = active & (network.link_length_m() > 40.0)
        links = np.nonzero(long_enough)[0]
        triangles = geo.chevron_triangles(
            ribbon_points, offsets, links, (width_pt[links] * 0.42 + 0.9) * mpp
        )
        page.ax.add_collection(
            PolyCollection(triangles, facecolors=[surface], edgecolors="none", alpha=0.9, zorder=4)
        )

    # Furniture.
    step = run.link_bins().bin_seconds
    window = (first if first is not None else int(link_bins.bins().min())) * step
    end = (stop if stop is not None else int(link_bins.bins().max()) + 1) * step
    default_subtitle = (
        f"{_minutes(window)}–{_minutes(end)} min · one ribbon per direction · "
        f"width = volume, colour = {_COLOUR_WORDS[colour]}"
    )
    level = _LEVEL_NAMES.get(run.flow_level, str(run.flow_level))
    stepping = f" · step {run.flow_step_s} s" if run.flow_level else ""
    source = (
        "© OpenStreetMap contributors (ODbL)"
        if network.source == "osm"
        else f"{network.source} network"
    )
    provenance = (
        f"{__version__} · run {run.run_id} · level {run.flow_level} ({level})"
        f"{stepping} · bin {step} s · {run_identity(run)} · {source}"
    )
    sample_values = [nice_number(vmax / 10), nice_number(vmax / 3), nice_number(vmax)]
    samples = [
        (
            v,
            max(
                (_WIDTH_MIN_PT + _WIDTH_SPAN_PT * float(np.sqrt(min(v, vmax) / vmax)))
                * zs
                * page.k,
                floor,
            ),
        )
        for v in sample_values
    ]
    draw_furniture(
        page,
        title=title or _TITLES[layer],
        subtitle=subtitle or default_subtitle,
        note=note,
        provenance=provenance,
        width_legend=("Volume per direction", samples, unit),
        colour_legend=(legend_label, legend_stops, low, high),
        credit=credit,
        logo=logo,
        compass=True,
    )
    if path is not None:
        page.fig.savefig(path, facecolor=th.surface)
    return page.fig
