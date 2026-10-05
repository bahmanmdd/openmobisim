"""Parking on the map: park-and-ride car parks and bike parkings, how full they got.

Car parks are squares and bike parkings circles, each kind sized by its capacity
on its own scale (a car park of 500 spaces is large, a bike rack of 500 is not),
coloured by the peak occupancy on the delay ramp, from empty to full; a parking
nobody used is a small hollow mark. The look was put to the user with the first
figure (Amsterdam) and accepted with smaller marks and a legend that tells car
from bike parking.
"""

from __future__ import annotations

from typing import Any

import numpy as np

from openmobisim import __version__
from openmobisim.viz import _geometry as geo
from openmobisim.viz._figure import SIZE_CM, Page, draw_furniture, load_matplotlib, ramp_rgb
from openmobisim.viz._provenance import run_identity
from openmobisim.viz._style import Theme, get_theme

__all__ = ["map_parking"]


def map_parking(
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
    """Draw a run's parkings: car parks and bike parkings, sized and coloured.

    Car parks are squares and bike parkings circles, each kind sized by its capacity
    on its own scale; the colour is the most vehicles parked at once over the day as
    a share of the capacity, from empty to full (a parking above capacity, which soft
    capacity allows, is full). Parkings nobody used are small hollow marks. The run's
    network is drawn quietly underneath.

    Args:
        run: A ``Run`` of a scenario with ``parkings``.
        theme: ``"paper"`` or ``"night"``.
        title: Figure title.
        subtitle: A line under the title; defaults to the counts and what size and
            colour mean.
        note: A third, italic line; defaults to the vehicles parked and how many
            found their parking full.
        size_cm: Figure size, width and height in centimetres.
        dpi: Resolution in dots per inch (25.4 mm), the image-file convention.
        credit: A line of your own before the logo (a name, an institution).
        logo: Draw the openmobisim logo at the bottom right.
        path: If given, also save the figure there.

    Returns:
        A ``matplotlib.figure.Figure``.

    Raises:
        ValueError: If the run had no parkings.
        ImportError: If matplotlib is not installed (``openmobisim[viz]``).
    """
    load_matplotlib()
    from matplotlib.collections import LineCollection

    places, bins, summary = run.parking_places(), run.parking_bins(), run.parking_summary
    if places is None or bins is None or summary is None:
        raise ValueError("map_parking needs a run of a scenario with parkings")
    th = get_theme(theme)
    n = len(places["parking_id"])
    peak, used = np.zeros(n), np.zeros(n)
    np.maximum.at(peak, np.asarray(bins["parking"]), np.asarray(bins["occupancy_max"]))
    np.add.at(used, np.asarray(bins["parking"]), np.asarray(bins["arrivals"]))
    cap = np.asarray(places["capacity"], dtype=float)
    share = np.clip(np.where(cap > 0, peak / np.maximum(cap, 1.0), 1.0), 0.0, 1.0)
    car = np.array([v == "car" for v in places["vehicle"]])
    lonlat = np.column_stack([places["lon"], places["lat"]])
    lon0, lat0 = float(lonlat[:, 0].mean()), float(lonlat[:, 1].mean())
    xy = geo.project_lonlat(lonlat, lon0, lat0)

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
    top_car = float(cap[car].max()) if car.any() else 1.0
    top_bike = float(cap[~car].max()) if (~car).any() else 1.0
    radius = np.where(
        car,
        2.0 + 4.0 * np.sqrt(cap / max(top_car, 1.0)),
        1.0 + 3.5 * np.sqrt(cap / max(top_bike, 1.0)),
    )
    colours = ramp_rgb(th.ramp_delay, share)
    for kind, marker, z in ((~car, "o", 3), (car, "s", 4)):
        quiet = kind & (used == 0)
        page.ax.scatter(
            xy[quiet, 0],
            xy[quiet, 1],
            s=(1.4 * k) ** 2,
            marker=marker,
            c=[th.surface],
            edgecolors=[th.muted],
            linewidths=0.3 * k,
            zorder=2,
        )
        busy = np.flatnonzero(kind & (used > 0))
        busy = busy[np.argsort(share[busy], kind="stable")]  # the fullest on top
        page.ax.scatter(
            xy[busy, 0],
            xy[busy, 1],
            s=(2.0 * radius[busy] * k) ** 2,
            marker=marker,
            c=colours[busy],
            edgecolors=[th.ink2],
            linewidths=0.4 * k,
            zorder=z,
        )

    n_car, n_bike = int(car.sum()), int((~car).sum())
    overflow = summary["car_overflow_arrivals"] + summary["bike_overflow_arrivals"]
    legend = [
        ("s", True, f"park-and-ride car park  {n_car:>5,}  up to {top_car:,.0f} spaces"),
        ("o", True, f"bike parking            {n_bike:>5,}  up to {top_bike:,.0f} spaces"),
        ("o", False, "unused"),
    ]
    draw_furniture(
        page,
        title=title or "Parking: park-and-ride and bike-and-ride",
        subtitle=subtitle
        or "size: capacity, each kind on its own scale · colour: peak occupancy over the day",
        note=note
        or (
            f"{summary['car_arrivals']:,.0f} cars and {summary['bike_arrivals']:,.0f} bikes parked"
            f" · {overflow:,.0f} found their parking full"
        ),
        provenance=f"{__version__} · {run_identity(run)} · © OpenStreetMap contributors",
        colour_legend=("peak occupancy", th.ramp_delay, "empty", "full"),
        marker_legend=legend,
        credit=credit,
        logo=logo,
        compass=True,
    )
    if path is not None:
        page.fig.savefig(path, facecolor=th.surface)
    return page.fig
