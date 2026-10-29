"""The canvas every map draws on: a network's links projected around one origin.

One place for what each map repeated: the links' vertices in metres around an origin, which link
each vertex belongs to, the links as polylines, and the thin network drawn under everything else,
with the road hierarchy shown in its widths (``road_tier``).
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

import numpy as np

from openmobisim.viz import _geometry as geo

#: The road tiers' bounds on ``road_ratio`` (a link's capacity over the network's large roads'):
#: tier 1 below 0.25, tier 2 from 0.25, tier 3 from 0.45, tier 4 from 0.75. Tier 0: no car may use
#: the link.
ROAD_TIER_BOUNDS = (0.25, 0.45, 0.75)
#: The network drawn under every map, by tier, as a multiple of the map's base width.
ROAD_TIER_WIDTHS = (0.5, 0.8, 1.2, 2.0, 3.0)
#: The grey margin a traffic map draws around each ribbon, by tier: how much wider than the ribbon
#: its casing is, in points at the default size (the road's capacity; the ribbon's width is still
#: its volume). Tier 0 has none.
ROAD_CASING_PT = (0.0, 2.0, 3.6, 6.0, 9.0)
#: The capacity of the network's large roads: this percentile of the drivable links' capacities,
#: so one exceptional link does not set the scale.
ROAD_RATIO_PERCENTILE = 99.0


def road_ratio(network: Any) -> np.ndarray:
    """Each link's capacity over the network's large roads' (``c / c_ref``, not capped).

    ``c`` is the link's capacity (PCU/h, all lanes), ``c_ref`` the 99th percentile of the drivable
    links' capacities: a road's place is relative to the network it is in, so a 2 000 PCU/h road
    is 1 where nothing is larger and about 0.5 beside a 4 000 PCU/h road. A link no car may use
    (a footway, a cycleway, a busway; every link of a bike or walk layer) is NaN.
    """
    capacity = np.asarray(network.link_capacity_pcu_h(), dtype=float)
    drivable = np.asarray(network.link_drivable(), dtype=bool)
    road = drivable & np.isfinite(capacity) & (capacity > 0)
    ratio = np.full(len(capacity), np.nan)
    if road.any():
        reference = float(np.percentile(capacity[road], ROAD_RATIO_PERCENTILE))
        ratio[road] = capacity[road] / reference
    return ratio


def road_tier(network: Any) -> np.ndarray | None:
    """Each link's road tier (uint8, 0 to 4; ``ROAD_TIER_BOUNDS``), or ``None`` without roads.

    Tier 0 is a link no car may use; tiers 1 to 4 the roads, minor to major. A network with no
    drivable link (a bike or walk layer) has no hierarchy: ``None``.
    """
    ratio = road_ratio(network)
    road = np.isfinite(ratio)
    if not road.any():
        return None
    tier = np.zeros(len(ratio), dtype=np.uint8)
    tier[road] = 1 + np.digitize(ratio[road], ROAD_TIER_BOUNDS)
    return tier


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
    #: Each link's road tier (``road_tier``); ``None``: no hierarchy shown.
    tier: np.ndarray | None = None

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
            tier=road_tier(network),
        )

    def lines(self, links: Any | None = None) -> list[np.ndarray]:
        """The links (every one by default) as polylines, for a ``LineCollection``."""
        chosen = np.arange(self.link_count) if links is None else links
        return geo.polylines(self.points, self.offsets, chosen)

    def base(self, colour: Any, width: float) -> Any:
        """Every link, thin, under everything else: a map's network, its road hierarchy shown.

        Each link is ``width`` times its tier's ``ROAD_TIER_WIDTHS`` wide, the minor links drawn
        first so the major roads sit on top. A network with no drivable link (a bike or walk
        layer) is drawn at ``width`` throughout.
        """
        from matplotlib.collections import LineCollection

        if self.tier is None:
            return LineCollection(
                self.lines(), colors=colour, linewidths=width, capstyle="round", zorder=1
            )
        order = np.argsort(self.tier, kind="stable")
        widths = width * np.asarray(ROAD_TIER_WIDTHS)[self.tier[order]]
        return LineCollection(
            self.lines(order), colors=colour, linewidths=widths, capstyle="round", zorder=1
        )

    def casing(
        self,
        colour: Any,
        links: np.ndarray,
        lines: list[np.ndarray],
        widths: np.ndarray,
        scale: float,
        capstyle: str = "butt",
    ) -> Any:
        """A traffic map's casing: each ribbon widened by its road's tier, in ``colour``.

        ``lines`` and ``widths`` (points) are the ribbons of ``links``, in that order; each casing
        is ``widths + ROAD_CASING_PT[tier] * scale`` wide, the major roads on top, so the margin
        around a ribbon is its road's capacity and the ribbon's own width still its volume.
        ``None`` without roads.
        """
        from matplotlib.collections import LineCollection

        if self.tier is None:
            return None
        links = np.asarray(links)
        at = np.nonzero(self.tier[links] > 0)[0]
        at = at[np.argsort(self.tier[links[at]], kind="stable")]
        margin = scale * np.asarray(ROAD_CASING_PT)[self.tier[links[at]]]
        return LineCollection(
            [lines[i] for i in at],
            colors=colour,
            linewidths=np.asarray(widths)[at] + margin,
            capstyle=capstyle,
            joinstyle="round",
            zorder=2.5,
        )
