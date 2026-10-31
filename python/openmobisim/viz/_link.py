"""``map_link``: one ribbon per direction of every link, coloured by delay.

A two-way street is two ribbons side by side, each on its own driver's side
(right-hand traffic), so the two directions never overlap and both read.
On the road, width is the road's capacity per direction (on the bike and walk layers,
volume), colour is the delay over free flow (a link's mean crossing time over its
free-flow time, less one); free flow recedes into the canvas and congestion glows.
"""

from __future__ import annotations

from typing import Any

import numpy as np

from openmobisim import __version__
from openmobisim.scenario import LEVEL_NAMES
from openmobisim.viz import _geometry as geo
from openmobisim.viz._canvas import Canvas, road_ratio
from openmobisim.viz._figure import (
    SIZE_CM,
    Page,
    draw_furniture,
    figure_for_pyplot,
    load_matplotlib,
    nice_number,
    ramp_rgb,
    rgb,
    save_figure,
)
from openmobisim.viz._provenance import network_source, run_identity
from openmobisim.viz._style import Theme, get_theme

__all__ = ["link_table", "map_link"]

_COLOUR_WORDS = {
    "delay": "delay over free flow",
    "volume": "volume",
    "volume_capacity": "volume over capacity",
}
#: Ribbon width in points at zero and at maximum volume, before the view scale.
_WIDTH_MIN_PT, _WIDTH_SPAN_PT = 0.5, 5.2
#: No ribbon is thinner than this many points (on a full-screen page), whatever the
#: view scale: a link with any traffic must stay visible.
_WIDTH_FLOOR_PT = 0.9
#: The casing's line ends (``Canvas.casing``).
_CASING_CAP = "butt"
#: The delay colours' bands, as the upper edges of the delay over free flow (a link's mean
#: crossing time over its free-flow time, less one): 0-15 %, 15-50 %, 50-100 % and 100 % or more
#: (twice the free-flow time), the darkest red. The two upper edges are the Highway Capacity
#: Manual's urban-street level-of-service thresholds on travel speed, two thirds (B/C) and one
#: half (C/D) of the base free-flow speed, a time 1.5 and 2 times free flow (TRB, *HCM*, 7th ed.,
#: urban street facilities, as tabulated in the Texas DOT's Traffic Safety Program manual, table
#: 9-12: https://www.txdot.gov/manuals/des/tsp/chapter-9-segment-analysis--freeways--multi-lane-h/9-3-hcm-based--mesoscopic--analysis/9-3-4-measures-of-effectiveness--moes-/9-3-4-4-arterials--or-urban-streets-.html);
#: the lowest edge, 15 %, about its level A (above 85 % of the free-flow speed in *HCM 2010*).
#: Free flow counts a signal's mean wait: only the waiting beyond it is delay.
DELAY_BANDS = (0.15, 0.50, 1.00)
#: Each band's label: its range, nothing else.
DELAY_BAND_LABELS = ("0–15%", "15–50%", "50–100%", "100%+")
#: The bands of volume over capacity, as their upper edges, coloured as the delay's four: under
#: 0.6 (level of service A), 0.6-0.8 (B-C), 0.8-1.0 (D-E) and over capacity (F), from the
#: planning-level table of levels of service by volume over capacity (A under 0.60, B 0.61-0.70,
#: C 0.71-0.80, D 0.81-0.90, E 0.91-1.00, F over 1.0; Kittitas MPO, 2010 Metropolitan
#: Transportation Plan, table 3.4, p. 3-36:
#: https://www.kmpo.net/wp-content/uploads/2019/02/KMPO_Section_3_2010_Final_pgs35-88.pdf), whose
#: F, over 1.0, is the Highway Capacity Manual's.
VC_BANDS = (0.6, 0.8, 1.0)
#: Each band's label: its range, nothing else.
VC_BAND_LABELS = ("0–0.6", "0.6–0.8", "0.8–1.0", "1.0+")


def delay_band(delay: np.ndarray) -> np.ndarray:
    """Each delay's band, 0 (free) to 3 (the worst): :data:`DELAY_BANDS`'s edges."""
    return np.searchsorted(np.asarray(DELAY_BANDS), np.asarray(delay), side="right")


def vc_band(ratio: np.ndarray) -> np.ndarray:
    """Each volume over capacity's band, 0 (under 0.6) to 3 (over capacity): :data:`VC_BANDS`."""
    return np.searchsorted(np.asarray(VC_BANDS), np.asarray(ratio), side="right")


def delay_time_share(delay: np.ndarray) -> np.ndarray:
    """The share of the crossing time that is delay, from the delay over free flow ``d``.

    ``d / (1 + d)``: a link crossed in twice its free-flow time spends half of it delayed.
    """
    d = np.asarray(delay, dtype=float)
    return d / (1.0 + d)


def free_flow_moving_s(network: Any) -> np.ndarray:
    """Each link's free-flow time without a signal's wait: its length at its free-flow speed.

    The figures' delay is measured against it (like a navigation map's, a link's
    whole crossing time, its signal's wait included, over its free flow), so a queue at a signal
    and the signal's own wait both show; ``Network.link_free_flow_s`` has the wait included.
    """
    length = np.asarray(network.link_length_m(), dtype=float)
    speed = np.asarray(network.link_speed_km_h(), dtype=float)
    return np.where(speed > 0, length / np.maximum(speed, 1e-9) * 3.6, 0.0)


def capacity_passing(network: Any) -> tuple[np.ndarray, np.ndarray]:
    """Each road link's capacity as it passes traffic, in PCU/h, and whether it ends at a signal.

    A link that ends at a signal discharges at its capacity times the green fraction (what the
    loading does); volume over capacity is judged against that, not against the link's
    nominal capacity (a two-lane approach of 3 800 PCU/h passes 1 824 at its signal).
    """
    from openmobisim.network import network_options

    capacity = np.asarray(network.link_capacity_pcu_h(), dtype=float)
    signal = np.asarray(network.link_signalised())  # per approach: a signal may stop one direction
    options = network_options(network)
    green = min(
        1.0, float(options["signal_green_fraction"]) * float(options["green_fraction_factor"])
    )
    return np.where(signal, capacity * green, capacity), signal


def link_table(
    bins: Any, n_links: int, free_flow_s: np.ndarray, *, first: int | None, stop: int | None
) -> dict[str, np.ndarray]:
    """Per-link volume and delay over the bins ``first <= bin < stop``.

    Returns arrays of length `n_links`: ``volume`` in PCU/h averaged over the
    selected bins, ``mean_s`` the PCU-weighted mean traversal time (NaN where
    nothing crossed), and ``delay`` the delay over free flow, ``mean_s / free_flow_s - 1``
    (0 at free flow, 1 at twice the free-flow time; never below 0; 0 where nothing crossed
    or the free-flow time is 0). ``first``/``stop`` of ``None`` select every bin from
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
        delay = np.where(
            (total > 0) & (free_flow_s > 0), np.maximum(mean_s / free_flow_s - 1.0, 0.0), 0.0
        )
    return {"volume": volume, "mean_s": mean_s, "delay": delay}


def flow_summary(network: Any, table: dict[str, np.ndarray], unit: str, active: np.ndarray) -> str:
    """The map's flow numbers in one line: the footer's first line.

    Vehicle-kilometres per hour over the bins shown (volume times length, summed over the
    drawn ribbons), the mean speed over them (kilometres over hours spent), the share of the
    time spent that is delay beyond free flow, and the busiest ribbon's volume.
    """
    length_km = network.link_length_m() / 1000.0
    volume, mean_s, delay = table["volume"], table["mean_s"], table["delay"]
    drawn = active & (volume > 0)
    if not drawn.any():
        return "no traffic on this layer"
    veh_km = float((volume[drawn] * length_km[drawn]).sum())
    hours = float((volume[drawn] * np.nan_to_num(mean_s[drawn]) / 3600.0).sum())
    lost = delay_time_share(delay[drawn])
    delay_h = float((volume[drawn] * np.nan_to_num(mean_s[drawn]) * lost / 3600.0).sum())
    what = "PCU" if unit.startswith("PCU") else "traveller"
    speed = veh_km / hours if hours > 0 else 0.0
    parts = [
        f"{veh_km:,.0f} {what}-km per hour".replace(",", "\u202f"),
        f"mean speed {speed:.0f} km/h",
    ]
    if hours > 0:
        parts.append(f"{100 * delay_h / hours:.0f} % of the time spent is delay")
    parts.append(f"busiest ribbon {volume[drawn].max():,.0f} {unit}".replace(",", "\u202f"))
    return " · ".join(parts)


#: The default window: the bins around the busiest one carrying at least this share of its traffic.
BUSY_SHARE = 0.25


def busiest_bins(link_bins: Any) -> tuple[int, int] | tuple[None, None]:
    """The busiest part of a run, as ``(first, stop)`` bins.

    The bin with the most traffic (PCU), widened bin by bin on either side while a bin carries
    at least ``BUSY_SHARE`` of it. A day's timetable runs buses from dawn to midnight; without
    this, a morning's figure averaged its peak over the whole day.
    """
    b = link_bins.bins().astype(np.int64)
    if not len(b):
        return None, None
    per_bin = np.bincount(b, weights=link_bins.pcu())
    peak = int(np.argmax(per_bin))
    busy = per_bin >= BUSY_SHARE * per_bin[peak]
    first, stop = peak, peak + 1
    while first > 0 and busy[first - 1]:
        first -= 1
    while stop < len(per_bin) and busy[stop]:
        stop += 1
    return first, stop


def _clock_seconds(value: str) -> int:
    """``"07:30"`` (or ``"07:30:15"``) as seconds after midnight."""
    try:
        parts = [int(x) for x in value.split(":")]
        if len(parts) not in (2, 3) or not 0 <= parts[1] < 60 or parts[0] < 0:
            raise ValueError
        return parts[0] * 3600 + parts[1] * 60 + (parts[2] if len(parts) == 3 else 0)
    except ValueError:
        raise ValueError(f"a clock time reads like '07:30', got {value!r}") from None


def _bin_range(bins_arg: Any, bin_s: int = 1) -> tuple[int | None, int | None]:
    """The bins asked for, as ``(first, stop)``: bin numbers counted from midnight, or clock times.

    A clock time alone is the bin it falls in; a pair of clock times, the bins that cover the
    span between them.
    """
    if bins_arg is None:
        return None, None
    if isinstance(bins_arg, str):
        b = _clock_seconds(bins_arg) // bin_s
        return b, b + 1
    if isinstance(bins_arg, int):
        return bins_arg, bins_arg + 1
    first, stop = bins_arg
    if isinstance(first, str) or isinstance(stop, str):
        start_s, end_s = _clock_seconds(str(first)), _clock_seconds(str(stop))
        if end_s <= start_s:
            raise ValueError(f"bins: the second clock time must be later, got {bins_arg!r}")
        return start_s // bin_s, -(-end_s // bin_s)
    return int(first), int(stop)


def _clock(seconds: float) -> str:
    """A second of the day as clock time, ``07:00``.

    Past midnight it reads ``24:30``, so a window that runs on past it still reads in order.
    """
    minutes = round(seconds / 60)
    return f"{minutes // 60:02d}:{minutes % 60:02d}"


_TITLES = {
    "road": "Flow and delay by direction",
    "bike": "Bike flow by direction",
    "walk": "Walking flow by direction",
}


def map_link(
    run: Any,
    *,
    bins: int | str | tuple[int, int] | tuple[str, str] | None = None,
    layer: str = "road",
    colour: str | None = None,
    ramp: str = "spectrum",
    theme: str | Theme = "paper",
    title: str | None = None,
    subtitle: str | None = None,
    note: str | None = None,
    view: tuple[tuple[float, float], tuple[float, float]] | None = None,
    size_cm: tuple[float, float] = SIZE_CM,
    dpi: int = 120,
    scale: float | None = None,
    chevrons: bool | None = None,
    min_volume: float = 0.0,
    credit: str | None = None,
    logo: bool = True,
    path: str | None = None,
) -> Any:
    """Draw a run's traffic on its network: one ribbon per direction of every link.

    Width is the road's capacity per direction, the same for every road of one capacity (on
    the bike and walk layers, which have no capacity, volume); colour is the delay over free
    flow (each link's mean crossing time over its free-flow time, less one), or volume, see
    `colour`.
    The two directions of a two-way street are drawn side by side, each on its
    own driver's side, so they never overlap. Free-flowing links recede and
    congestion glows.

    Args:
        run: A ``Run`` from ``Scenario.run()`` on a scenario built with
            ``link_bin_s``, at ``flow_level`` 2-4 for congestion to show.
        bins: Which time bins to show: ``None`` for the busiest part of the run
            (the busiest bin and the bins around it carrying at least a quarter of
            its traffic), ``"all"`` for every bin from the first to the last
            that saw traffic, a clock time for the bin it falls in (``"07:45"``), a
            pair of clock times for the bins that cover the span (``("07:30",
            "08:30")``), or bin numbers counted from midnight: an ``int`` for one
            bin, ``(first, stop)`` for a range (``stop`` excluded).
        layer: Which layer's traffic to draw: ``"road"`` (default: cars, on the
            road network), ``"bike"`` or ``"walk"`` (each on its own layer's links,
            ``network.layer(...)``; volume counts travellers, not PCU).
        colour: What the colour shows: ``"delay"`` (the delay over free flow, a
            link's mean crossing time over its free-flow time less one, in four
            bands, 0-15 %, 15-50 %, 50-100 % and 100 % or more, after the Highway
            Capacity Manual's levels of service; free flow counts a signal's mean
            wait, so only waiting beyond it is delay; the default on the road),
            ``"volume"`` (the default on the bike and walk layers, where nothing is
            delayed), or ``"volume_capacity"`` (volume over the link's capacity as it
            passes traffic — at a signal, times the green fraction — in four bands
            coloured as the delay's: under 0.6, 0.6-0.8, 0.8-1.0 and over capacity;
            the road only).
        ramp: For delay and volume over capacity, ``"spectrum"`` (default:
            blue-green for little, gold in the middle, deeper red for more) or
            ``"ember"`` (the single-hue ramp of the first version).
        theme: ``"paper"`` (print, slides) or ``"night"`` (screens, video).
        title: Figure title; defaults to a description of what is drawn.
        subtitle: A line under the title; defaults to the time window and the encodings.
        note: A third, italic line, for example to say the demand is a fixture.
        view: ``((lon_min, lat_min), (lon_max, lat_max))`` to zoom to; the whole
            network by default.
        size_cm: Figure size, width and height in centimetres. 40.64 x 22.86 (16:9) is a
            full-screen frame. The text is laid out for figures **20 cm wide or more**; narrower,
            the footer
            runs into the logo and the legends crowd (a layout for a journal
            column is planned).
        dpi: Resolution in dots per inch (25.4 mm), the image-file convention.
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

    canvas = Canvas.from_network(network)
    lon0, lat0, points, owner = canvas.lon0, canvas.lat0, canvas.points, canvas.owner
    offsets = canvas.offsets

    if bins is None:
        first, stop = busiest_bins(link_bins)
    else:
        first, stop = (None, None) if bins == "all" else _bin_range(bins, link_bins.bin_seconds)
    free = network.link_free_flow_s()  # a signal's mean wait in it (S268)
    table = link_table(link_bins, n_links, free, first=first, stop=stop)
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
    page = Page.new(th, size_cm, dpi, lo, hi)
    mpp = page.metres_per_point

    # Width (S308): on the road, its capacity over the network's large roads' (linear, the
    # largest and above the widest), so every road of one capacity is one width whatever its
    # traffic; on the bike and walk layers, the square root of volume up to the 99th percentile.
    vmax = float(np.percentile(volume[active], 99)) if active.any() else 1.0
    vmax = max(vmax, min_volume)
    ratio = road_ratio(network) if road else None
    by_capacity = ratio is not None and bool(np.isfinite(ratio).any())
    if by_capacity:
        frac = np.minimum(np.nan_to_num(ratio, nan=0.0), 1.0)
        finite = np.isfinite(ratio) & (ratio > 0)
        capacity_ref = float(
            np.median(
                np.asarray(network.link_capacity_pcu_h(), dtype=float)[finite] / ratio[finite]
            )
        )
    else:
        frac = np.sqrt(np.minimum(volume, vmax) / vmax)
    floor = _WIDTH_FLOOR_PT * page.k
    width_pt = np.where(
        active, np.maximum((_WIDTH_MIN_PT + _WIDTH_SPAN_PT * frac) * zs * page.k, floor), 0.0
    )

    # 1. The canvas: every link, thin, in the network's own geometry.
    page.ax.add_collection(canvas.base(th.base, 0.35 if width_m > 6000.0 else 0.6))

    # 2. One ribbon per directed link, offset to the driver's side.
    offset_m = (width_pt / 2 + 0.25) * mpp
    ribbon_points = geo.offset_points(points, owner, offset_m)
    stops = th.ramp_delay if ramp == "spectrum" else th.ramp_delay_ember
    if colour == "delay":
        band = delay_band(delay)
        band_rgb = ramp_rgb(stops, np.linspace(0.0, 1.0, len(DELAY_BAND_LABELS)))
        link_rgb = band_rgb[band]
        if ramp == "ember":  # the first version: free flow recedes as a quiet slate
            link_rgb = np.where((band == 0)[:, None], rgb(th.free)[None, :], link_rgb)
        severity = delay
        legend_stops = tuple(
            "#" + "".join(f"{round(float(v) * 255):02x}" for v in c) for c in band_rgb
        )
        low, high = DELAY_BAND_LABELS, ""
        legend_label = "Colour: delay over free flow"  # the bands name only their ranges (S269)
    elif colour == "volume_capacity":
        capacity, _ = capacity_passing(network)
        ratio = np.where(capacity > 0, volume / np.maximum(capacity, 1e-9), 0.0)
        # The delay's four colours in four bands, so the two read alike (S308).
        band = vc_band(ratio)
        band_rgb = ramp_rgb(stops, np.linspace(0.0, 1.0, len(VC_BAND_LABELS)))
        link_rgb = band_rgb[band]
        severity = ratio
        legend_stops = tuple(
            "#" + "".join(f"{round(float(v) * 255):02x}" for v in c) for c in band_rgb
        )
        low, high = VC_BAND_LABELS, ""
        legend_label = "Colour: volume over capacity"
    else:
        t = np.sqrt(np.minimum(volume, vmax) / vmax)
        link_rgb = ramp_rgb(th.ramp_volume, t)
        severity = volume
        legend_stops, low, high = th.ramp_volume, "0", f"{vmax:,.0f}+"
        legend_label = (
            f"Colour: volume per direction ({unit})"
            if by_capacity
            else f"Width: volume per direction ({unit})"
        )
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
    # The casing (S302): a grey margin around each ribbon, as wide as its road's capacity tier;
    # not where the width itself is the capacity (S308).
    casing = (
        None
        if by_capacity
        else canvas.casing(th.base, drawn, lines, line_width, zs * page.k, _CASING_CAP)
    )
    if casing is not None:
        page.ax.add_collection(casing)
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

    # Furniture (S281: the subtitle says when; the legends say how to read the ribbons; the
    # footer's first line gives the flow numbers, its second what the run was).
    step = link_bins.bin_seconds  # this layer's bins (S282: a run may record only the layers')
    seen = link_bins.bins()
    if len(seen) or (first is not None and stop is not None):
        window = (first if first is not None else int(seen.min())) * step
        end = (stop if stop is not None else int(seen.max()) + 1) * step
        when = "the busiest part of the run" if bins is None else "the bins shown"
        default_subtitle = (
            f"{_clock(window)}–{_clock(end)} ({when}) · one ribbon per direction of travel, "
            "on the driver's side"
        )
    else:  # no vehicle crossed a link on this layer: nothing to time
        default_subtitle = "no traffic on this layer · one ribbon per direction of travel"
    level = LEVEL_NAMES.get(run.flow_level, str(run.flow_level))
    stepping = f" · step {run.flow_step_s} s" if run.flow_level else ""
    source = network_source(network)
    provenance = [
        flow_summary(network, table, unit, active),
        f"run {run.run_id} · {run_identity(run)}",
        f"openmobisim {__version__} · level {run.flow_level} ({level}){stepping} · "
        f"bins of {step} s · {source}",
    ]
    if by_capacity:  # the legend's samples: capacities, as wide as their roads are drawn
        top = capacity_ref
        sample_values = [nice_number(top / 4), nice_number(top / 2), nice_number(top)]
        share = [min(v / top, 1.0) for v in sample_values]
        width_words, width_unit = "Width: road capacity per direction", "PCU/h"
    else:
        sample_values = [nice_number(vmax / 10), nice_number(vmax / 3), nice_number(vmax)]
        share = [float(np.sqrt(min(v, vmax) / vmax)) for v in sample_values]
        width_words, width_unit = "Width: volume per direction", unit
    samples = [
        (v, max((_WIDTH_MIN_PT + _WIDTH_SPAN_PT * f) * zs * page.k, floor))
        for v, f in zip(sample_values, share, strict=True)
    ]
    draw_furniture(
        page,
        title=title or _TITLES[layer],
        subtitle=subtitle or default_subtitle,
        note=note,
        provenance=provenance,
        width_legend=(width_words, samples, width_unit),
        colour_legend=(legend_label, legend_stops, low, high),
        band_legend=(
            "Grey margin: road capacity" if canvas.tier is not None and not by_capacity else None
        ),
        credit=credit,
        logo=logo,
        compass=True,
    )
    if path is not None:
        save_figure(page.fig, path, th.surface)
    return figure_for_pyplot(page.fig)
