"""Mode share by time of day: which modes the trips took, bin by departure bin.

One stacked bar per departure bin with trips, the modes in families from the
light ones at the bottom to the car at the top; each mode's share of the day at
the right. The look was put to the user with the first figure (Amsterdam) and
accepted as drafted (D17).
"""

from __future__ import annotations

from typing import Any

import numpy as np

from openmobisim import MODES, __version__
from openmobisim.viz._figure import SIZE_CM, Page, draw_furniture, font_mono_name, load_matplotlib
from openmobisim.viz._provenance import run_identity
from openmobisim.viz._style import Theme, get_theme

__all__ = ["chart_mode_share"]

#: Bottom to top: the active modes, then transit and its combinations, the car last.
_STACK = ("walk", "bike", "bike_transit", "transit", "car_transit", "car")
_LABEL = {
    "walk": "walk",
    "bike": "bike",
    "bike_transit": "bike-and-ride",
    "transit": "transit",
    "car_transit": "park-and-ride",
    "car": "car",
}
#: A bin with fewer trips than this share of the busiest bin's is left out: a handful of
#: trips would draw a full bar of one mode.
_BIN_FLOOR = 0.01


def chart_mode_share(
    run: Any,
    *,
    bin_s: int = 900,
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
    """Draw the share of the trips each mode took, by departure time.

    One stacked bar per departure bin: the share of the trips departing then that took
    each mode, walk at the bottom and the car at the top; each mode's share of the day
    at the right. Trips count as many times as the people their traveller stands for.
    A trip counts under the mode it took (``Run.trip_modes``): its stated mode, or with
    mode choice (``modes``) the one it chose. Bins with fewer than 1% of the busiest
    bin's trips are left out, and so are trips that had no mode to choose.

    Args:
        run: A ``Run``.
        bin_s: The departure bins' length, in seconds.
        theme: ``"paper"`` or ``"night"``.
        title: Figure title.
        subtitle: A line under the title; defaults to the trips and how many chose
            their mode.
        note: A third, italic line.
        size_cm: Figure size, width and height in centimetres.
        dpi: Resolution in dots per inch (25.4 mm), the image-file convention.
        credit: A line of your own before the logo (a name, an institution).
        logo: Draw the openmobisim logo at the bottom right.
        path: If given, also save the figure there.

    Returns:
        A ``matplotlib.figure.Figure``.

    Raises:
        ValueError: If ``bin_s`` is not positive, or no trip had a mode.
        ImportError: If matplotlib is not installed (``openmobisim[viz]``).
    """
    if bin_s <= 0:
        raise ValueError(f"bin_s must be positive, got {bin_s}")
    load_matplotlib()
    from matplotlib.patches import Rectangle

    th = get_theme(theme)
    colours = dict(zip(MODES, th.modes, strict=True))
    trips = run.trip_modes()
    has_mode = np.array([m is not None for m in trips["mode"]])
    if not has_mode.any():
        raise ValueError("chart_mode_share needs a run with at least one trip that had a mode")
    mode = np.array([m or "" for m in trips["mode"]])[has_mode]
    weight = np.asarray(trips["weight"], dtype=float)[has_mode]
    departure_bin = np.asarray(trips["departure_s"], dtype=np.int64)[has_mode] // bin_s
    chose = int(np.count_nonzero(np.asarray(trips["mode_choice"], dtype=bool)[has_mode]))
    total = float(weight.sum())

    bins, at = np.unique(departure_bin, return_inverse=True)
    per_bin = np.bincount(at, weights=weight)
    shown = per_bin >= _BIN_FLOOR * per_bin.max()
    x = bins[shown] * bin_s / 3600.0
    width = bin_s / 3600.0

    page = Page.new(th, size_cm, dpi, np.array([0.0, 0.0]), np.array([1.0, 1.0]))
    page.fig.delaxes(page.ax)
    ax = page.fig.add_axes((0.07, 0.12, 0.74, 0.70), facecolor=th.surface)
    bottom = np.zeros(len(x))
    for m in _STACK:
        share = np.bincount(at, weights=weight * (mode == m), minlength=len(bins))[shown]
        share = share / per_bin[shown]
        ax.bar(
            x,
            share,
            width=width * 0.92,
            bottom=bottom,
            align="edge",
            color=colours[m],
            linewidth=0,
        )
        bottom = bottom + share
    first, last = float(np.floor(x.min())), float(np.ceil(x.max() + width))
    ax.set_xlim(first, last)
    ax.set_ylim(0, 1)
    mono = font_mono_name()
    k = page.k
    for side in ("top", "right"):
        ax.spines[side].set_visible(False)
    for side in ("left", "bottom"):
        ax.spines[side].set_color(th.base)
    ax.tick_params(colors=th.muted, labelsize=9 * k)
    ax.set_yticks([0, 0.25, 0.5, 0.75, 1.0], ["0", "25%", "50%", "75%", "100%"], fontfamily=mono)
    hours = np.arange(first, last + 1)
    step = max(int(np.ceil(len(hours) / 25)), 1)
    hours = hours[::step]
    ax.set_xticks(hours, [f"{int(h):02d}:00" for h in hours], fontfamily=mono)
    minutes = f"{bin_s / 60:g}-minute bins"
    ax.set_xlabel(f"departure time ({minutes})", color=th.ink2, fontsize=10 * k)
    ax.set_ylabel("share of the trips departing", color=th.ink2, fontsize=10 * k)

    # Each mode's share of the day, at the right, in the stack's order (the top first).
    page.fig.text(0.835, 0.83, "share of the day", color=th.ink2, fontsize=10 * k)
    y = 0.80
    for m in reversed(_STACK):
        day = float(weight[mode == m].sum()) / total
        page.fig.add_artist(
            Rectangle(
                (0.835, y - 0.004),
                0.012,
                0.018,
                transform=page.fig.transFigure,
                color=colours[m],
            )
        )
        page.fig.text(
            0.852,
            y,
            f"{_LABEL[m]:<14}{100 * day:5.1f}%",
            color=th.ink2,
            fontsize=9.5 * k,
            fontfamily=mono,
        )
        y -= 0.034

    n = len(mode)
    if subtitle is None:
        subtitle = f"{n:,} trips"
        if chose == n:
            subtitle += ", each choosing its mode with its route"
        elif chose:
            subtitle += f", {chose:,} of them choosing their mode with its route"
        subtitle += f" · {minutes}"
    draw_furniture(
        page,
        title=title or "Mode share by departure time",
        subtitle=subtitle,
        note=note,
        provenance=f"{__version__} · {run_identity(run)}",
        credit=credit,
        logo=logo,
        compass=False,
    )
    if path is not None:
        page.fig.savefig(path, facecolor=th.surface)
    return page.fig
