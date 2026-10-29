"""Shared bikes in a run: how full the hubs' docks were, and the fleet over the day.

:func:`map_hubs` draws each hub's docks at one time of day: a disc sized by the docks' slots,
coloured by the share of the slots holding a bike, from empty to full, with a ring where the
docks were empty, or full, for more than half of that time bin. :func:`chart_service` draws a
service's bikes over the day, docked and ridden, and the pickups that failed.

Both read the run's last loading (``Run.hub_bins()``, ``Run.hub_places()``,
``Run.service_legs()``), the day the travellers met after the run settled.
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
    figure_for_pyplot,
    font_mono_name,
    load_matplotlib,
    ramp_rgb,
    save_figure,
)
from openmobisim.viz._provenance import network_source, run_identity
from openmobisim.viz._style import ION, Theme, get_theme

__all__ = ["chart_service", "map_hubs"]

#: A hub whose docks were empty (or full) for more than this share of the bin gets a ring.
_RING_SHARE = 0.5


def _fill_ramp(th: Theme) -> tuple[str, ...]:
    """Empty to full, in the volume hue (ion).

    On paper from its palest step, so an empty hub reads as nearly blank; on night the theme's
    volume ramp, which brightens with more.
    """
    if th.name == "paper":
        return (ION[100], ION[300], ION[500], ION[700])
    return th.ramp_volume


def _seconds(at: str | float) -> float:
    """``"08:30"`` (or ``"08:30:15"``) or seconds from midnight, as seconds from midnight."""
    if isinstance(at, str):
        parts = [float(p) for p in at.split(":")]
        if not 2 <= len(parts) <= 3:
            raise ValueError(f"at must be 'HH:MM' or seconds from midnight, got {at!r}")
        return parts[0] * 3600.0 + parts[1] * 60.0 + (parts[2] if len(parts) == 3 else 0.0)
    return float(at)


def _clock(s: float) -> str:
    return f"{int(s // 3600):02d}:{int(s % 3600 // 60):02d}"


def _service_rows(run: Any, service: str | None, what: str) -> tuple[Any, Any, np.ndarray]:
    """The run's hub bins and docks, and which bin rows are of ``service`` (all: ``None``)."""
    bins, places = run.hub_bins(), run.hub_places()
    if bins is None or places is None:
        raise ValueError(f"{what} needs a run of a scenario with a service (services=[...])")
    names = sorted(set(bins["service"]))
    if service is not None and service not in names:
        raise ValueError(f"the run has no service {service!r}; its services: {names}")
    keep = np.array([service is None or s == service for s in bins["service"]], dtype=bool)
    return bins, places, keep


def _bin_lengths(run: Any, start: np.ndarray, bin_s: float) -> np.ndarray:
    """Each bin's length inside the window, in seconds (the last may be shorter)."""
    try:
        end = float(run.window["end_s"])
    except (KeyError, RuntimeError, TypeError):
        return np.full(len(start), bin_s)
    return np.clip(end - start, 1.0, bin_s)


def map_hubs(
    run: Any,
    at: str | float = "08:30",
    *,
    service: str | None = None,
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
    """Draw how full each hub's shared-bike docks were at one time of the day.

    A hub is a place where travellers change vehicle (a stop, or a dock standing alone); its
    docks are added up. Each hub is a disc sized by its docks' slots and coloured by the share
    of the slots holding a bike over the time bin that holds ``at`` (the bins' mean, from
    ``Run.hub_bins()``), from empty to full. A ring marks a hub whose docks were empty for more
    than half of the bin (no bike to take: in the delay spectrum's red), or full (no slot to
    return one: in its gold). The run's network is drawn quietly underneath.

    Args:
        run: A ``Run`` of a scenario with a docked bike service.
        at: The time: ``"HH:MM"`` or seconds from midnight, inside the run's window.
        service: The service to draw, by name; every service of the run by default.
        theme: ``"paper"`` or ``"night"``.
        title: Figure title.
        subtitle: A line under the title; defaults to the time bin and what size, colour and
            rings mean.
        note: A third, italic line; defaults to the bin's pickups, returns and failed pickups.
        size_cm: Figure size, width and height in centimetres.
        dpi: Resolution in dots per inch (25.4 mm), the image-file convention.
        credit: A line of your own before the logo (a name, an institution).
        logo: Draw the openmobisim logo at the bottom right.
        path: If given, also save the figure there.

    Returns:
        A ``matplotlib.figure.Figure``.

    Raises:
        ValueError: If the run had no service, has no service ``service``, or ``at`` is
            outside its window.
        ImportError: If matplotlib is not installed (``openmobisim[viz]``).
    """
    load_matplotlib()
    from matplotlib.lines import Line2D

    th = get_theme(theme)
    bins, places, keep = _service_rows(run, service, "map_hubs")
    start = np.asarray(bins["start_s"], dtype=float)
    bin_s = float(start[1] - start[0]) if len(set(start)) > 1 else 900.0
    t = _seconds(at)
    inside = keep & (start <= t) & (t < start + bin_s)
    if not inside.any():
        first, last = start[keep].min(), start[keep].max() + bin_s
        span = f"{_clock(first)}–{_clock(last)}"
        raise ValueError(f"at {_clock(t)} is outside the run's bins, {span}")
    t0 = float(start[inside][0])
    length = float(_bin_lengths(run, np.array([t0]), bin_s)[0])

    rows = np.flatnonzero(inside)
    hub_of_row = [bins["hub_id"][i] for i in rows]
    hubs = sorted(set(hub_of_row))
    index = {h: i for i, h in enumerate(hubs)}
    at_hub = np.array([index[h] for h in hub_of_row])
    n = len(hubs)

    def per_hub(column: str) -> np.ndarray:
        return np.bincount(at_hub, weights=np.asarray(bins[column], dtype=float)[rows], minlength=n)

    capacity = per_hub("capacity")
    level = per_hub("level_mean")
    fill = np.clip(level / np.maximum(capacity, 1.0), 0.0, 1.0)
    # A hub's share of the bin empty (or full): its docks' shares, weighted by their slots.
    slots = np.asarray(bins["capacity"], dtype=float)[rows]
    share_empty = slots * np.asarray(bins["empty_s"])[rows] / length
    share_full = slots * np.asarray(bins["full_s"])[rows] / length
    empty = np.bincount(at_hub, weights=share_empty, minlength=n)
    full = np.bincount(at_hub, weights=share_full, minlength=n)
    empty, full = empty / np.maximum(capacity, 1.0), full / np.maximum(capacity, 1.0)

    # Where each hub is: its docks' mean position.
    where = {h: [] for h in hubs}
    for h, lon, lat, s in zip(
        places["hub_id"], places["lon"], places["lat"], places["service"], strict=True
    ):
        if h in where and (service is None or s == service):
            where[h].append((lon, lat))
    lonlat = np.array([np.mean(where[h], axis=0) for h in hubs])
    lon0, lat0 = float(lonlat[:, 0].mean()), float(lonlat[:, 1].mean())
    xy = geo.project_lonlat(lonlat, lon0, lat0)

    everything = xy
    canvas = None
    network = run.network
    if network is not None:
        canvas = Canvas.from_network(network, lon0, lat0)
        everything = canvas.points[:: max(len(canvas.points) // 20000, 1)]
    lo, hi = np.percentile(everything, 0.5, axis=0), np.percentile(everything, 99.5, axis=0)
    lo, hi = np.minimum(lo, xy.min(axis=0)), np.maximum(hi, xy.max(axis=0))
    pad = (hi - lo) * 0.04
    page = Page.new(th, size_cm, dpi, lo - pad, hi + pad)
    k = page.k
    if canvas is not None:
        page.ax.add_collection(canvas.base(th.base, 0.3 * k))
    top = float(capacity.max()) if n else 1.0
    radius = 1.5 + 4.5 * np.sqrt(capacity / max(top, 1.0))
    ramp = _fill_ramp(th)
    colours = ramp_rgb(ramp, fill)
    ring_empty, ring_full = th.ramp_delay[2], th.ramp_delay[1]
    order = np.argsort(-capacity, kind="stable")  # the small hubs on top
    page.ax.scatter(
        xy[order, 0],
        xy[order, 1],
        s=(2.0 * radius[order] * k) ** 2,
        c=colours[order],
        edgecolors=[th.ink2],
        linewidths=0.4 * k,
        zorder=3,
    )
    for mask, colour in ((empty > _RING_SHARE, ring_empty), (full > _RING_SHARE, ring_full)):
        page.ax.scatter(
            xy[mask, 0],
            xy[mask, 1],
            s=(2.0 * (radius[mask] + 1.6) * k) ** 2,
            facecolors="none",
            edgecolors=[colour],
            linewidths=1.4 * k,
            zorder=4,
        )

    n_empty, n_full = int((empty > _RING_SHARE).sum()), int((full > _RING_SHARE).sum())
    picked = float(np.asarray(bins["pickups"])[rows].sum())
    returned = float(np.asarray(bins["returns"])[rows].sum())
    failed = float(np.asarray(bins["failed"])[rows].sum())
    span = f"{_clock(t0)}–{_clock(t0 + length)}"
    draw_furniture(
        page,
        title=title or f"Shared-bike hubs at {_clock(t)}",
        subtitle=subtitle
        or (
            f"{n:,} hubs, {int(capacity.sum()):,} slots · bin {span} · size: slots · "
            "colour: share of the slots holding a bike (bin mean)"
        ),
        note=note
        or (
            f"in the bin: {picked:,.0f} bikes taken, {returned:,.0f} returned, "
            f"{failed:,.0f} pickups failed · {n_empty:,} hubs mostly empty, {n_full:,} mostly full"
        ),
        provenance=f"{__version__} · {run_identity(run)} · {network_source(network)}",
        colour_legend=("bikes docked / slots", ramp, "empty", "full"),
        credit=credit,
        logo=logo,
        compass=True,
    )
    # The rings' legend, where a marker legend goes (top left of the legend block).
    fig, lx = page.fig, 0.585
    for i, (colour, text) in enumerate(
        (
            (ring_empty, f"ring: empty over half the bin ({n_empty:,})"),
            (ring_full, f"ring: full over half the bin ({n_full:,})"),
        )
    ):
        y0 = 0.950 - i * 0.021
        fig.add_artist(
            Line2D(
                [lx + 0.008],
                [y0 + 0.004],
                transform=fig.transFigure,
                linestyle="none",
                marker="o",
                markersize=7.0 * k,
                markerfacecolor="none",
                markeredgecolor=colour,
                markeredgewidth=1.4 * k,
            )
        )
        fig.text(lx + 0.022, y0, text, color=th.ink2, fontsize=9 * k, fontfamily=font_mono_name())
    if path is not None:
        save_figure(page.fig, path, th.surface)
    return figure_for_pyplot(page.fig)


def chart_service(
    run: Any,
    service: str = "bike_share",
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
    """Draw a docked bike service's bikes over the day, and the pickups that failed.

    Above, one stacked bar per time bin of ``Run.hub_bins()``: the bikes docked (the docks'
    mean over the bin, summed) and the bikes ridden (the fleet less those docked; bikes in an
    operator's van would count here, and a run has no operator yet). Below, the pickups that
    failed in the bin (travellers who found no bike at the docks they tried and walked on).

    Args:
        run: A ``Run`` of a scenario with a docked bike service.
        service: The service, by name.
        theme: ``"paper"`` or ``"night"``.
        title: Figure title.
        subtitle: A line under the title; defaults to the fleet, the docks and the bins.
        note: A third, italic line; defaults to the legs and how many failed.
        size_cm: Figure size, width and height in centimetres.
        dpi: Resolution in dots per inch (25.4 mm), the image-file convention.
        credit: A line of your own before the logo (a name, an institution).
        logo: Draw the openmobisim logo at the bottom right.
        path: If given, also save the figure there.

    Returns:
        A ``matplotlib.figure.Figure``.

    Raises:
        ValueError: If the run had no service, or no service ``service``.
        ImportError: If matplotlib is not installed (``openmobisim[viz]``).
    """
    load_matplotlib()
    from matplotlib.patches import Rectangle

    th = get_theme(theme)
    bins, places, keep = _service_rows(run, service, "chart_service")
    start = np.asarray(bins["start_s"], dtype=float)[keep]
    times, at = np.unique(start, return_inverse=True)
    bin_s = float(times[1] - times[0]) if len(times) > 1 else 900.0
    docked = np.bincount(at, weights=np.asarray(bins["level_mean"], dtype=float)[keep])
    failed = np.bincount(at, weights=np.asarray(bins["failed"], dtype=float)[keep])
    mine = np.array([s == service for s in places["service"]], dtype=bool)
    fleet = float(np.asarray(places["bikes_start"], dtype=float)[mine].sum())
    riding = np.maximum(fleet - docked, 0.0)
    docks = int(mine.sum())

    x = times / 3600.0
    width = bin_s / 3600.0
    page = Page.new(th, size_cm, dpi, np.array([0.0, 0.0]), np.array([1.0, 1.0]))
    page.fig.delaxes(page.ax)
    k = page.k
    mono = font_mono_name()
    upper = page.fig.add_axes((0.07, 0.40, 0.74, 0.42), facecolor=th.surface)
    lower = page.fig.add_axes((0.07, 0.12, 0.74, 0.20), facecolor=th.surface, sharex=upper)
    c_docked, c_riding, c_failed = th.ramp_volume[1], th.modes[1], th.ramp_delay[2]
    upper.bar(x, docked, width=width * 0.92, align="edge", color=c_docked, linewidth=0)
    upper.bar(
        x, riding, width=width * 0.92, bottom=docked, align="edge", color=c_riding, linewidth=0
    )
    lower.bar(x, failed, width=width * 0.92, align="edge", color=c_failed, linewidth=0)
    first, last = float(np.floor(x.min())), float(np.ceil(x.max() + width))
    upper.set_xlim(first, last)
    upper.set_ylim(0, max(fleet, 1.0) * 1.05)
    lower.set_ylim(0, max(float(failed.max()), 1.0) * 1.15)
    for ax in (upper, lower):
        for side in ("top", "right"):
            ax.spines[side].set_visible(False)
        for side in ("left", "bottom"):
            ax.spines[side].set_color(th.base)
        ax.tick_params(colors=th.muted, labelsize=9 * k)
        for label in ax.get_yticklabels():
            label.set_fontfamily(mono)
    upper.tick_params(labelbottom=False)
    hours = np.arange(first, last + 1)
    hours = hours[:: max(int(np.ceil(len(hours) / 25)), 1)]
    lower.set_xticks(hours, [f"{int(h):02d}:00" for h in hours], fontfamily=mono)
    minutes = f"{bin_s / 60:g}-minute bins"
    lower.set_xlabel(f"time of day ({minutes})", color=th.ink2, fontsize=10 * k)
    upper.set_ylabel("bikes", color=th.ink2, fontsize=10 * k)
    lower.set_ylabel("failed pickups", color=th.ink2, fontsize=10 * k)

    page.fig.text(0.835, 0.83, "per bin", color=th.ink2, fontsize=10 * k)
    y = 0.80
    for colour, text in (
        (c_riding, "bikes ridden (mean)"),
        (c_docked, "bikes docked (mean)"),
        (c_failed, "pickups failed (people)"),
    ):
        page.fig.add_artist(
            Rectangle(
                (0.835, y - 0.004), 0.012, 0.018, transform=page.fig.transFigure, color=colour
            )
        )
        page.fig.text(0.852, y, text, color=th.ink2, fontsize=9.5 * k, fontfamily=mono)
        y -= 0.034

    legs = run.service_legs()
    if note is None and legs is not None:
        mine_legs = np.array([s == service for s in legs["service"]], dtype=bool)
        w = np.asarray(legs["weight"], dtype=float)[mine_legs]
        out = np.array(legs["outcome"], dtype=object)[mine_legs]
        n_failed = float(w[out == "failed"].sum())
        share = n_failed / w.sum() if w.sum() > 0 else 0.0
        note = (
            f"{w.sum():,.0f} legs ridden or tried · {n_failed:,.0f} failed ({100 * share:.0f}%), "
            "walked on from the last dock tried"
        )
    draw_furniture(
        page,
        title=title or f"Shared bikes over the day: {service}",
        subtitle=subtitle or f"{fleet:,.0f} bikes at {docks:,} docks · {minutes}",
        note=note,
        provenance=f"{__version__} · {run_identity(run)}",
        credit=credit,
        logo=logo,
        compass=False,
    )
    if path is not None:
        save_figure(page.fig, path, th.surface)
    return figure_for_pyplot(page.fig)
