"""The canvas every map draws on: a network's links projected around one origin.

One place for what each map repeated: the links' vertices in metres around an origin, which link
each vertex belongs to, the links as polylines, and the thin network drawn under everything else.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

import numpy as np

from openmobisim.viz import _geometry as geo


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
        )

    def lines(self, links: Any | None = None) -> list[np.ndarray]:
        """The links (every one by default) as polylines, for a ``LineCollection``."""
        chosen = np.arange(self.link_count) if links is None else links
        return geo.polylines(self.points, self.offsets, chosen)

    def base(self, colour: Any, width: float) -> Any:
        """Every link, thin, under everything else: a map's network."""
        from matplotlib.collections import LineCollection

        return LineCollection(
            self.lines(), colors=colour, linewidths=width, capstyle="round", zorder=1
        )
