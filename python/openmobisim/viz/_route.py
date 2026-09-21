"""``map_route``: the alternative routes of one origin-destination pair (S165).

Each route is drawn as a line in its own colour, and each is shifted a little to
its right of the last, so where routes **coincide** they run side by side
instead of hiding each other. That is the whole point of looking at a route set:
which alternatives exist, where they part and where they share a street.
"""

from __future__ import annotations

from typing import Any

import numpy as np

from openmobisim import __version__
from openmobisim.viz import _geometry as geo
from openmobisim.viz._figure import Page, draw_furniture, load_matplotlib, rgb
from openmobisim.viz._style import Theme, get_theme

__all__ = ["map_route"]

#: Route colours by rank (the documented categorical order, first five), light
#: and dark surface. Routes need identity, not magnitude, so they are the one
#: place a map uses more than the brand pair.
_ROUTE_PAPER = (
    "#2a78d6",
    "#eb6834",
    "#1baf7a",
    "#eda100",
    "#e87ba4",
    "#008300",
    "#4a3aa7",
    "#e34948",
)
_ROUTE_NIGHT = (
    "#3987e5",
    "#d95926",
    "#199e70",
    "#c98500",
    "#d55181",
    "#008300",
    "#9085e9",
    "#e66767",
)
_WIDTH_PT = 3.0


def map_route(
    routes: Any,
    network: Any,
    pair: int | tuple[tuple[float, float], tuple[float, float]] = 0,
    *,
    theme: str | Theme = "paper",
    title: str | None = None,
    subtitle: str | None = None,
    note: str | None = None,
    size: tuple[float, float] = (16.0, 9.0),
    dpi: int = 120,
    credit: str | None = None,
    logo: bool = True,
    path: str | None = None,
) -> Any:
    """Draw the alternative routes of one origin-destination pair on the network.

    Route 1 (the cheapest) is drawn first, in blue; the others follow in the
    order of their cost. Each route is shifted a little to the right of the
    one before, so where two routes use the same street they run side by side
    and the overlap is visible. The legend gives each route's free-flow time,
    its detour over the best, and the share of it that overlaps an earlier
    route.

    Args:
        routes: ``RouteSets``, from ``Run.route_sets()`` or
            ``openmobisim.route_sets_build``.
        network: The network the routes are on.
        pair: Which pair: the position among the sets' keys, or two
            ``(lon, lat)`` points, snapped to their nearest nodes.
        theme: ``"paper"`` or ``"night"``.
        title: Figure title.
        subtitle: A line under the title; defaults to the counts.
        note: A third, italic line.
        size: Figure size in inches.
        dpi: Dots per inch.
        credit: A line of your own before the logo (a name, an institution).
        logo: Draw the openmobisim logo at the bottom right.
        path: If given, also save the figure there.

    Returns:
        A ``matplotlib.figure.Figure``.

    Raises:
        ValueError: If the pair has no set, or no route.
        ImportError: If matplotlib is not installed (``openmobisim[viz]``).
    """
    load_matplotlib()
    from matplotlib.collections import LineCollection, PolyCollection
    from matplotlib.patches import Circle

    th = get_theme(theme)
    key = _key_index(routes, network, pair)
    alternatives = routes.routes(key)
    if not alternatives:
        raise ValueError(
            "this pair has no route: there is no way from its origin to its destination"
        )
    costs = routes.costs()
    offsets_r = routes.set_offsets()
    first_route = int(offsets_r[key])
    route_costs = costs[first_route : first_route + len(alternatives)]
    route_overlap = routes.overlaps()[first_route : first_route + len(alternatives)]

    coords, offsets = network.link_geometry()
    owner = geo.link_of_vertex(offsets)
    lon0, lat0 = float(coords[:, 0].mean()), float(coords[:, 1].mean())
    points = geo.project_lonlat(coords, lon0, lat0)
    normals = geo.right_normals(points, owner)

    used = np.unique(np.concatenate(alternatives))
    span_pts = np.concatenate(
        [points[int(offsets[link]) : int(offsets[link + 1])] for link in used]
    )
    lo, hi = span_pts.min(axis=0), span_pts.max(axis=0)
    pad = np.maximum((hi - lo) * 0.18, 150.0)
    page = Page.new(th, size, dpi, lo - pad, hi + pad)
    mpp = page.metres_per_point
    k = page.k

    everything = np.arange(network.link_count)
    page.ax.add_collection(
        LineCollection(
            geo.polylines(points, offsets, everything),
            colors=th.base,
            linewidths=0.6,
            capstyle="round",
            zorder=1,
        )
    )
    palette = _ROUTE_NIGHT if th.glow else _ROUTE_PAPER
    surface = rgb(th.surface)
    step = (_WIDTH_PT + 0.8) * k * mpp
    legend: list[tuple[np.ndarray, str]] = []
    for r, links in enumerate(alternatives):
        colour = rgb(palette[r % len(palette)])
        shifted = points + normals * (step * (r + 0.5))
        lines = geo.polylines(shifted, offsets, links.astype(np.int64))
        page.ax.add_collection(
            LineCollection(
                lines,
                colors=[surface],
                linewidths=(_WIDTH_PT + 1.6) * k,
                capstyle="round",
                zorder=2 + r * 0.01,
            )
        )
        page.ax.add_collection(
            LineCollection(
                lines,
                colors=[colour],
                linewidths=_WIDTH_PT * k,
                capstyle="round",
                zorder=3 + r * 0.01,
            )
        )
        tri = geo.chevron_triangles(
            shifted, offsets, links.astype(np.int64), np.full(len(links), 1.5 * k * mpp)
        )
        if len(tri):
            page.ax.add_collection(
                PolyCollection(tri, facecolors=[surface], edgecolors="none", alpha=0.9, zorder=4)
            )
        detour = route_costs[r] / route_costs[0] - 1.0
        text = (
            f"{r + 1}  {route_costs[r] / 60:5.1f} min  +{detour:4.0%}  "
            f"overlap {route_overlap[r]:.0%}"
        )
        legend.append((colour, text))

    # Origin and destination: the first link's start and the last link's end.
    first, last = alternatives[0], alternatives[0]
    a = points[int(offsets[first[0]])]
    b = points[int(offsets[last[-1] + 1]) - 1]
    for xy, label in ((a, "A"), (b, "B")):
        page.ax.add_patch(
            Circle(
                xy, 7 * k * mpp, facecolor=th.ink, edgecolor=th.surface, linewidth=1.2 * k, zorder=6
            )
        )
        page.ax.text(
            xy[0],
            xy[1],
            label,
            color=th.surface,
            fontsize=8.5 * k,
            fontweight="bold",
            ha="center",
            va="center",
            zorder=7,
        )

    n = len(alternatives)
    source = (
        "© OpenStreetMap contributors (ODbL)"
        if network.source == "osm"
        else f"{network.source} network"
    )
    draw_furniture(
        page,
        title=title or "Route alternatives",
        subtitle=subtitle
        or (
            f"{n} route{'s' if n != 1 else ''} from A to B · method {routes.method} · "
            "routes shift right of one another so shared streets show side by side"
        ),
        note=note,
        provenance=(
            f"{__version__} · route set {routes.identity[:8]} · "
            f"{routes.descriptor.replace(';', ' ')} · {source}"
        ),
        route_legend=legend,
        credit=credit,
        logo=logo,
        compass=True,
    )
    if path is not None:
        page.fig.savefig(path, facecolor=th.surface)
    return page.fig


def _key_index(routes: Any, network: Any, pair: Any) -> int:
    """The position among `routes`' keys of `pair` (an index, or two points)."""
    if isinstance(pair, (int, np.integer)):
        if not 0 <= int(pair) < routes.key_count:
            raise ValueError(f"pair {pair} is out of range: there are {routes.key_count} pairs")
        return int(pair)
    (olon, olat), (dlon, dlat) = pair
    found = routes.find(network.node_nearest(olon, olat), network.node_nearest(dlon, dlat))
    if found is None:
        raise ValueError("no route set for that pair: build one with route_sets_build")
    return int(found)
