"""The canvas every map draws on: a network's links projected around one origin.

One place for what each map repeated: the links' vertices in metres around an origin, which link
each vertex belongs to, the links as polylines, and the thin network drawn under everything else,
with the road hierarchy shown in its widths (``road_weight``).
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

import numpy as np

from openmobisim.viz import _geometry as geo

#: The base network's width, as a multiple of a map's base width, is
#: ``ROAD_WIDTH_MIN + ROAD_WIDTH_SPAN * weight``: a link no car may use 0.6 times it, the
#: network's largest roads twice it.
ROAD_WIDTH_MIN = 0.6
ROAD_WIDTH_SPAN = 1.4
#: The capacity a road needs to weigh 1: this percentile of the drivable links' capacities, so one
#: exceptional link does not set the scale.
ROAD_WEIGHT_PERCENTILE = 99.0


def road_weight(network: Any) -> np.ndarray:
    """Each link's place in the road hierarchy, in ``[0, 1]``, from its capacity.

    ``sqrt(min(c / c_ref, 1))``, with ``c`` the link's capacity (PCU/h, all lanes) and ``c_ref``
    the 99th percentile of the drivable links' capacities: a road's weight is relative to the
    network it is in, so a 2 000 PCU/h road weighs 1 where nothing is larger and about 0.71
    beside a 4 000 PCU/h road. A link no car may use (a footway, a cycleway, a busway; every link
    of a bike or walk layer) weighs 0.
    """
    capacity = np.asarray(network.link_capacity_pcu_h(), dtype=float)
    drivable = np.asarray(network.link_drivable(), dtype=bool)
    road = drivable & np.isfinite(capacity) & (capacity > 0)
    weight = np.zeros(len(capacity))
    if road.any():
        reference = float(np.percentile(capacity[road], ROAD_WEIGHT_PERCENTILE))
        weight[road] = np.sqrt(np.minimum(capacity[road] / reference, 1.0))
    return weight


#: The interactive page's four widths of the base network, as multiples of its base width.
ROAD_TIER_WIDTHS = (0.6, 1.0, 1.5, 2.0)


def road_tier(network: Any) -> np.ndarray:
    """Each link's ``road_weight`` as one of four tiers (uint8, 0 to 3), for the interactive page.

    The tier is the one of ``ROAD_TIER_WIDTHS`` nearest the link's static-map width multiple
    (``ROAD_WIDTH_MIN + ROAD_WIDTH_SPAN * weight``). A network with no drivable link is all tier
    1, today's width.
    """
    weight = road_weight(network)
    if not weight.any():
        return np.ones(len(weight), dtype=np.uint8)
    factor = ROAD_WIDTH_MIN + ROAD_WIDTH_SPAN * weight
    tiers = np.asarray(ROAD_TIER_WIDTHS)
    return np.digitize(factor, (tiers[:-1] + tiers[1:]) / 2).astype(np.uint8)


@dataclass(frozen=True)
class Canvas:
    """A network's links, projected for a map."""

    #: The projection's origin, in degrees.
    lon0: float
    lat0: float
    #: Every vertex of every link, in metres east and north of the origin.
    points: np.ndarray
    #: Link ``i``'s vertices are ``points[offsets[i]:offsets[i + 1]]``.
    offsets: np.ndarray
    #: The link each vertex belongs to.
    owner: np.ndarray
    #: How many links.
    link_count: int
    #: Each link's place in the road hierarchy (``road_weight``); ``None``: no hierarchy shown.
    weight: np.ndarray | None = None

    @classmethod
    def from_network(
        cls, network: Any, lon0: float | None = None, lat0: float | None = None
    ) -> Canvas:
        """The network's canvas, around ``(lon0, lat0)`` or, by default, its vertices' mean."""
        coords, offsets = network.link_geometry()
        if lon0 is None or lat0 is None:
            lon0, lat0 = float(coords[:, 0].mean()), float(coords[:, 1].mean())
        return cls(
            lon0=lon0,
            lat0=lat0,
            points=geo.project_lonlat(coords, lon0, lat0),
            offsets=offsets,
            owner=geo.link_of_vertex(offsets),
            link_count=network.link_count,
            weight=road_weight(network),
        )

    def lines(self, links: Any | None = None) -> list[np.ndarray]:
        """The links (every one by default) as polylines, for a ``LineCollection``."""
        chosen = np.arange(self.link_count) if links is None else links
        return geo.polylines(self.points, self.offsets, chosen)

    def base(self, colour: Any, width: float) -> Any:
        """Every link, thin, under everything else: a map's network, its road hierarchy shown.

        Each link is ``width`` times ``ROAD_WIDTH_MIN + ROAD_WIDTH_SPAN * weight`` wide, the minor
        links drawn first so the major roads sit on top. A network with no drivable link (a bike
        or walk layer) is drawn at ``width`` throughout.
        """
        from matplotlib.collections import LineCollection

        if self.weight is None or not self.weight.any():
            return LineCollection(
                self.lines(), colors=colour, linewidths=width, capstyle="round", zorder=1
            )
        order = np.argsort(self.weight, kind="stable")
        widths = width * (ROAD_WIDTH_MIN + ROAD_WIDTH_SPAN * self.weight[order])
        return LineCollection(
            self.lines(order), colors=colour, linewidths=widths, capstyle="round", zorder=1
        )
