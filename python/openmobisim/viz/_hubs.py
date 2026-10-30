"""Shared bikes in a run: how full the hubs' docks were, and the service over the day.

:func:`map_hubs` draws each hub's docks at one time of day: a disc sized by the docks' slots,
coloured by the share of the slots holding a bike, from empty to full, with a ring where the
docks held no bike, or had no free slot, for more than half of that time bin. :func:`chart_service`
draws a docked bike service's day: the bikes docked and ridden, the docks without a bike, and the
travellers who found a dock empty, bin by bin.

Both read the run's last loading (``Run.bike_share_docks()``, ``Run.bike_share_dock_places()``,
``Run.bike_share_trips()``), the day the travellers met after the run settled.
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

#: A hub whose docks held no bike (or no free slot) for more than this share of the bin gets a
#: ring.
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
    """The run's dock bins and docks, and which bin rows are of ``service`` (all: ``None``)."""
    bins, places = run.bike_share_docks(), run.bike_share_dock_places()
    if bins is None or places is None:
        raise ValueError(f"{what} needs a run of a scenario with a service (services=[...])")
    names = sorted(set(bins["service"]))
    if service is not None and service not in names:
        raise ValueError(f"the run has no service {service!r}; its services: {names}")
    keep = np.array([service is None or s == service for s in bins["service"]], dtype=bool)
    return bins, places, keep


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
    of the slots holding a bike over the time bin that holds ``at`` (the bin's time-weighted
    mean, ``bikes_mean`` of ``Run.bike_share_docks()``), from empty to full. A ring marks a hub
    whose docks held no bike for more than half of the bin (no bike to take: in the delay
    spectrum's red), or had no free slot (no room to return one: in its gold). The run's
    network is drawn quietly underneath.

    Args:
        run: A ``Run`` of a scenario with a docked bike service.
        at: The time: ``"HH:MM"`` or seconds from midnight, inside the run's window.
        service: The service to draw, by name; every service of the run by default.
        theme: ``"paper"`` or ``"night"``.
        title: Figure title.
        subtitle: A line under the title; defaults to the time bin and what size, colour and
            rings mean.
        note: A third, italic line; defaults to the bin's bikes taken and returned and the
            travellers who found a dock empty.
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
    start = np.asarray(bins["bin_start_s"], dtype=float)
    end = np.asarray(bins["bin_end_s"], dtype=float)
    t = _seconds(at)
    inside = keep & (start <= t) & (t < end)
    if not inside.any():
        span = f"{_clock(start[keep].min())}–{_clock(end[keep].max())}"
        raise ValueError(f"at {_clock(t)} is outside the run's bins, {span}")
    t0, t1 = float(start[inside][0]), float(end[inside][0])
    length = max(t1 - t0, 1.0)

    rows = np.flatnonzero(inside)
    hub_of_row = [bins["hub_id"][i] for i in rows]
    hubs = sorted(set(hub_of_row))
    index = {h: i for i, h in enumerate(hubs)}
    at_hub = np.array([index[h] for h in hub_of_row])
    n = len(hubs)

    def per_hub(column: str, weight: np.ndarray | None = None) -> np.ndarray:
        v = np.asarray(bins[column], dtype=float)[rows]
        return np.bincount(at_hub, weights=v if weight is None else v * weight, minlength=n)

    slots = per_hub("slots")
    fill = np.clip(per_hub("bikes_mean") / np.maximum(slots, 1.0), 0.0, 1.0)
    # A hub's share of the bin empty (or full): its docks' shares, weighted by their slots.
    dock_slots = np.asarray(bins["slots"], dtype=float)[rows]
    empty = per_hub("seconds_empty", dock_slots) / length / np.maximum(slots, 1.0)
    full = per_hub("seconds_full", dock_slots) / length / np.maximum(slots, 1.0)

    # Where each hub is: its docks' mean position.
    where: dict[str, list[tuple[float, float]]] = {h: [] for h in hubs}
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
    top = float(slots.max()) if n else 1.0
    radius = 1.5 + 4.5 * np.sqrt(slots / max(top, 1.0))
    ramp = _fill_ramp(th)
    colours = ramp_rgb(ramp, fill)
    ring_empty, ring_full = th.ramp_delay[2], th.ramp_delay[1]
    order = np.argsort(-slots, kind="stable")  # the small hubs on top
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

    def total(column: str) -> float:
        return float(np.asarray(bins[column], dtype=float)[rows].sum())

    found_empty = total("people_failed_attempt_no_bike") + total("people_gave_up_no_bike")
    span = f"{_clock(t0)}–{_clock(t1)}"
    draw_furniture(
        page,
        title=title or f"Shared-bike hubs at {_clock(t)}",
        subtitle=subtitle
        or (
            f"{n:,} hubs, {int(slots.sum()):,} slots · bin {span} · size: slots · "
            "colour: share of the slots holding a bike (bin mean)"
        ),
        note=note
        or (
            f"in the bin: {total('bikes_taken'):,.0f} bikes taken, "
            f"{total('bikes_returned'):,.0f} returned · {found_empty:,.0f} travellers found a "
            f"dock empty, {total('people_gave_up_no_bike'):,.0f} of them gave up and walked"
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
            (ring_empty, f"ring: no bike over half the bin ({n_empty:,})"),
            (ring_full, f"ring: no free slot over half the bin ({n_full:,})"),
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
    """Draw a docked bike service's day, or a taxi fleet's (``service`` naming one: see below).

    Three panels over the run's window. **Bikes**: one stacked bar per bin, the bikes docked
    (the docks' time-weighted means, summed) and the bikes ridden (the fleet less those docked;
    bikes in an operator's van would count here, and a run has no operator yet). **Docks
    without a bike**: the mean number of docks empty over the bin (their seconds empty over the
    bin's length, summed), beside the docks: a fleet with many bikes docked can still have its
    busy docks empty. **Travellers who found a dock empty** (people): those who went on to
    another dock, and those who gave up and walked to their destination.

    **A taxi fleet** (``service="taxi"``, its name), bin by bin of ``Run.taxi_vehicles()`` and
    ``Run.taxi_trips()``: **vehicles** by state (the mean over the bin: carrying a passenger,
    driving empty to a pickup, idle, at or driving to a charger); **travellers waiting** for a
    taxi (the mean over the bin, from each request until its vehicle reached the pickup, it
    failed, or the window ended); **travellers who met trouble**, by the bin they asked in: no
    taxi within the longest wait (they walked), and carried after a wait more than a minute
    longer than expected.

    Args:
        run: A ``Run`` of a scenario with a docked bike service or a taxi fleet.
        service: The service, by name.
        theme: ``"paper"`` or ``"night"``.
        title: Figure title.
        subtitle: A line under the title; defaults to the fleet, the docks and the bins.
        note: A third, italic line; defaults to the trips and how they went.
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
    taxis = run.taxi_vehicles() if hasattr(run, "taxi_vehicles") else None
    if taxis is not None and service in set(taxis["service"]):
        return _chart_taxi(
            run,
            service,
            theme=theme,
            title=title,
            subtitle=subtitle,
            note=note,
            size_cm=size_cm,
            dpi=dpi,
            credit=credit,
            logo=logo,
            path=path,
        )
    load_matplotlib()
    from matplotlib.patches import Rectangle

    th = get_theme(theme)
    bins, places, keep = _service_rows(run, service, "chart_service")
    start = np.asarray(bins["bin_start_s"], dtype=float)[keep]
    end = np.asarray(bins["bin_end_s"], dtype=float)[keep]
    times, at = np.unique(start, return_inverse=True)
    ends = np.zeros(len(times))
    np.maximum.at(ends, at, end)
    length = np.maximum(ends - times, 1.0)

    def per_bin(column: str) -> np.ndarray:
        return np.bincount(at, weights=np.asarray(bins[column], dtype=float)[keep])

    docked = per_bin("bikes_mean")
    docks_empty = per_bin("seconds_empty") / length
    went_on = per_bin("people_failed_attempt_no_bike")
    gave_up = per_bin("people_gave_up_no_bike")
    mine = np.array([s == service for s in places["service"]], dtype=bool)
    fleet = float(np.asarray(places["bikes_start"], dtype=float)[mine].sum())
    riding = np.maximum(fleet - docked, 0.0)
    n_docks = int(mine.sum())

    x, width = times / 3600.0, length / 3600.0
    bin_s = float(np.max(ends - times))
    page = Page.new(th, size_cm, dpi, np.array([0.0, 0.0]), np.array([1.0, 1.0]))
    page.fig.delaxes(page.ax)
    k = page.k
    mono = font_mono_name()
    top = page.fig.add_axes((0.07, 0.53, 0.74, 0.30), facecolor=th.surface)
    middle = page.fig.add_axes((0.07, 0.33, 0.74, 0.14), facecolor=th.surface, sharex=top)
    low = page.fig.add_axes((0.07, 0.12, 0.74, 0.15), facecolor=th.surface, sharex=top)
    c_docked, c_riding = th.ramp_volume[1], th.modes[1]
    c_empty, c_went_on, c_gave_up = th.ink2, th.ramp_delay[1], th.ramp_delay[2]
    bar = {"align": "edge", "linewidth": 0}
    top.bar(x, docked, width=width * 0.92, color=c_docked, **bar)
    top.bar(x, riding, width=width * 0.92, bottom=docked, color=c_riding, **bar)
    middle.bar(x, docks_empty, width=width * 0.92, color=c_empty, **bar)
    low.bar(x, went_on, width=width * 0.92, color=c_went_on, **bar)
    low.bar(x, gave_up, width=width * 0.92, bottom=went_on, color=c_gave_up, **bar)
    first, last = float(x.min()), float((ends.max()) / 3600.0)
    top.set_xlim(first, last)
    top.set_ylim(0, max(fleet, 1.0) * 1.05)
    middle.set_ylim(0, max(float(docks_empty.max()), 1.0) * 1.15)
    low.set_ylim(0, max(float((went_on + gave_up).max()), 1.0) * 1.15)
    for ax in (top, middle, low):
        for side in ("top", "right"):
            ax.spines[side].set_visible(False)
        for side in ("left", "bottom"):
            ax.spines[side].set_color(th.base)
        ax.tick_params(colors=th.muted, labelsize=9 * k)
        for label in ax.get_yticklabels():
            label.set_fontfamily(mono)
    top.tick_params(labelbottom=False)
    middle.tick_params(labelbottom=False)
    hours = np.arange(np.ceil(first), np.floor(last) + 1)
    hours = hours[:: max(int(np.ceil(len(hours) / 25)), 1)]
    low.set_xticks(hours, [f"{int(h):02d}:00" for h in hours], fontfamily=mono)
    minutes = f"{bin_s / 60:g}-minute bins"
    low.set_xlabel(f"time of day ({minutes}, the run's window)", color=th.ink2, fontsize=10 * k)
    top.set_ylabel("bikes", color=th.ink2, fontsize=10 * k)
    middle.set_ylabel(f"docks empty\n(of {n_docks})", color=th.ink2, fontsize=10 * k)
    low.set_ylabel("travellers", color=th.ink2, fontsize=10 * k)

    page.fig.text(0.835, 0.83, "per bin", color=th.ink2, fontsize=10 * k)
    y = 0.80
    for colour, text in (
        (c_riding, "bikes ridden (mean)"),
        (c_docked, "bikes docked (mean)"),
        (c_empty, "docks with no bike (mean)"),
        (c_went_on, "found a dock empty, went on"),
        (c_gave_up, "found a dock empty, gave up"),
    ):
        page.fig.add_artist(
            Rectangle(
                (0.835, y - 0.004), 0.012, 0.018, transform=page.fig.transFigure, color=colour
            )
        )
        page.fig.text(0.852, y, text, color=th.ink2, fontsize=9.5 * k, fontfamily=mono)
        y -= 0.034

    trips = run.bike_share_trips()
    if note is None and trips is not None:
        mine_trips = np.array([s == service for s in trips["service"]], dtype=bool)
        w = np.asarray(trips["travellers"], dtype=float)[mine_trips]
        result = np.array(trips["result"], dtype=object)[mine_trips]
        total = w.sum()

        def share(r: str) -> str:
            v = float(w[result == r].sum())
            return f"{v:,.0f} ({100 * v / total:.0f}%)" if total > 0 else "0"

        note = (
            f"{total:,.0f} travellers chose a shared bike · {share('bike_at_another_dock')} took "
            f"one at another dock · {share('no_bike_walked_to_destination')} found none and "
            "walked"
        )
    draw_furniture(
        page,
        title=title or f"Shared bikes over the day: {service}",
        subtitle=subtitle or f"{fleet:,.0f} bikes at {n_docks:,} docks · {minutes}",
        note=note,
        provenance=f"{__version__} · {run_identity(run)}",
        credit=credit,
        logo=logo,
        compass=False,
    )
    if path is not None:
        save_figure(page.fig, path, th.surface)
    return figure_for_pyplot(page.fig)


def _chart_taxi(
    run: Any,
    service: str,
    *,
    theme: str | Theme,
    title: str | None,
    subtitle: str | None,
    note: str | None,
    size_cm: tuple[float, float],
    dpi: int,
    credit: str | None,
    logo: bool,
    path: str | None,
) -> Any:
    """``chart_service`` for a taxi fleet (see there)."""
    load_matplotlib()
    from matplotlib.patches import Rectangle

    th = get_theme(theme)
    vb = run.taxi_vehicles()
    keep = np.array([s == service for s in vb["service"]], dtype=bool)
    start = np.asarray(vb["bin_start_s"], dtype=float)[keep]
    end = np.asarray(vb["bin_end_s"], dtype=float)[keep]
    times, at = np.unique(start, return_inverse=True)
    ends = np.zeros(len(times))
    np.maximum.at(ends, at, end)
    length = np.maximum(ends - times, 1.0)

    def per_bin(column: str) -> np.ndarray:
        return np.bincount(at, weights=np.asarray(vb[column], dtype=float)[keep]) / length

    occupied = per_bin("seconds_occupied")
    to_pickup = per_bin("seconds_to_pickup") + per_bin("seconds_relocating")
    idle = per_bin("seconds_idle")
    charging = (
        per_bin("seconds_to_charger")
        + per_bin("seconds_charging")
        + per_bin("seconds_queued_at_charger")
    )
    fleet = len(set(np.asarray(vb["vehicle"], dtype=object)[keep]))
    # Travellers waiting (time-weighted over each bin) and in trouble (by the bin they asked).
    waiting = np.zeros(len(times))
    no_taxi = np.zeros(len(times))
    longer = np.zeros(len(times))
    trips = run.taxi_trips()
    total = 0.0
    if trips is not None:
        mine = np.array([s == service for s in trips["service"]], dtype=bool)
        w = np.asarray(trips["travellers"], dtype=float)[mine]
        asked = np.asarray(trips["requested_s"], dtype=float)[mine]
        waited = np.asarray(trips["waited_s"], dtype=float)[mine]
        result = np.array(trips["result"], dtype=object)[mine]
        total = float(w.sum())
        for b, (lo, hi) in enumerate(zip(times, ends, strict=True)):
            overlap = np.clip(np.minimum(asked + waited, hi) - np.maximum(asked, lo), 0.0, None)
            waiting[b] = float((w * overlap).sum()) / max(hi - lo, 1.0)
            inside = (asked >= lo) & (asked < hi)
            walked = (result == "no_taxi_walked_to_destination") | (
                result == "no_taxi_still_walking_at_window_end"
            )
            no_taxi[b] = float(w[inside & walked].sum())
            longer[b] = float(w[inside & (result == "waited_longer_than_expected")].sum())

    x, width = times / 3600.0, length / 3600.0
    bin_s = float(np.max(ends - times))
    page = Page.new(th, size_cm, dpi, np.array([0.0, 0.0]), np.array([1.0, 1.0]))
    page.fig.delaxes(page.ax)
    k = page.k
    mono = font_mono_name()
    top = page.fig.add_axes((0.07, 0.53, 0.74, 0.30), facecolor=th.surface)
    middle = page.fig.add_axes((0.07, 0.33, 0.74, 0.14), facecolor=th.surface, sharex=top)
    low = page.fig.add_axes((0.07, 0.12, 0.74, 0.15), facecolor=th.surface, sharex=top)
    c_occupied, c_pickup = th.modes[0], th.ramp_delay[1]
    c_idle, c_charging = th.ramp_volume[1], th.ink2
    c_waiting, c_no_taxi, c_longer = th.ink2, th.ramp_delay[2], th.ramp_delay[1]
    bar = {"align": "edge", "linewidth": 0}
    bottom = np.zeros(len(times))
    for values, colour in (
        (occupied, c_occupied),
        (to_pickup, c_pickup),
        (charging, c_charging),
        (idle, c_idle),
    ):
        top.bar(x, values, width=width * 0.92, bottom=bottom, color=colour, **bar)
        bottom = bottom + values
    middle.bar(x, waiting, width=width * 0.92, color=c_waiting, **bar)
    low.bar(x, no_taxi, width=width * 0.92, color=c_no_taxi, **bar)
    low.bar(x, longer, width=width * 0.92, bottom=no_taxi, color=c_longer, **bar)
    first, last = float(x.min()), float((ends.max()) / 3600.0)
    top.set_xlim(first, last)
    top.set_ylim(0, max(fleet, 1) * 1.05)
    middle.set_ylim(0, max(float(waiting.max()), 1.0) * 1.15)
    low.set_ylim(0, max(float((no_taxi + longer).max()), 1.0) * 1.15)
    for ax in (top, middle, low):
        for side in ("top", "right"):
            ax.spines[side].set_visible(False)
        for side in ("left", "bottom"):
            ax.spines[side].set_color(th.base)
        ax.tick_params(colors=th.muted, labelsize=9 * k)
        for label in ax.get_yticklabels():
            label.set_fontfamily(mono)
    top.tick_params(labelbottom=False)
    middle.tick_params(labelbottom=False)
    hours = np.arange(np.ceil(first), np.floor(last) + 1)
    hours = hours[:: max(int(np.ceil(len(hours) / 25)), 1)]
    low.set_xticks(hours, [f"{int(h):02d}:00" for h in hours], fontfamily=mono)
    minutes = f"{bin_s / 60:g}-minute bins"
    low.set_xlabel(f"time of day ({minutes}, the run's window)", color=th.ink2, fontsize=10 * k)
    top.set_ylabel("vehicles", color=th.ink2, fontsize=10 * k)
    middle.set_ylabel("travellers\nwaiting", color=th.ink2, fontsize=10 * k)
    low.set_ylabel("travellers", color=th.ink2, fontsize=10 * k)

    page.fig.text(0.835, 0.83, "per bin", color=th.ink2, fontsize=10 * k)
    y = 0.80
    for colour, text in (
        (c_idle, "idle (mean)"),
        (c_charging, "charging or to it"),
        (c_pickup, "driving to a pickup"),
        (c_occupied, "with a passenger"),
        (c_waiting, "travellers waiting"),
        (c_longer, "waited > expected + 1 min"),
        (c_no_taxi, "no taxi: walked"),
    ):
        page.fig.add_artist(
            Rectangle(
                (0.835, y - 0.004), 0.012, 0.018, transform=page.fig.transFigure, color=colour
            )
        )
        page.fig.text(0.852, y, text, color=th.ink2, fontsize=9.5 * k, fontfamily=mono)
        y -= 0.034

    if note is None and total > 0:
        note = (
            f"{total:,.0f} travellers chose a taxi · {no_taxi.sum():,.0f} got none and walked · "
            f"{longer.sum():,.0f} waited more than a minute longer than expected"
        )
    draw_furniture(
        page,
        title=title or f"Taxis over the day: {service}",
        subtitle=subtitle or f"{fleet:,} vehicles · {minutes}",
        note=note,
        provenance=f"{__version__} · {run_identity(run)}",
        credit=credit,
        logo=logo,
        compass=False,
    )
    if path is not None:
        save_figure(page.fig, path, th.surface)
    return figure_for_pyplot(page.fig)
