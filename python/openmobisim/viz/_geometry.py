"""Array geometry for figures: offsets, ribbons, chevrons, curves.

Everything here works on flat arrays over the *whole* network — one array of
points, one array of per-link offsets — with no Python loop per link, so the
cost of a figure grows with the number of segments, not with Python's speed.
Coordinates are metres in a local flat projection unless a function says
otherwise.
"""

from __future__ import annotations

import numpy as np

__all__ = [
    "bezier_curves",
    "chevron_triangles",
    "link_of_vertex",
    "offset_points",
    "polylines",
    "project_lonlat",
    "right_normals",
]

#: Metres per degree of latitude; longitude scales with the cosine of latitude.
_M_PER_DEG_LAT = 110_574.0
_M_PER_DEG_LON_EQ = 111_320.0


def project_lonlat(lonlat: np.ndarray, lon0: float, lat0: float) -> np.ndarray:
    """Project ``(n, 2)`` WGS84 ``(lon, lat)`` to local metres around ``(lon0, lat0)``."""
    x = (lonlat[:, 0] - lon0) * np.cos(np.radians(lat0)) * _M_PER_DEG_LON_EQ
    y = (lonlat[:, 1] - lat0) * _M_PER_DEG_LAT
    return np.column_stack([x, y])


def link_of_vertex(offsets: np.ndarray) -> np.ndarray:
    """For each vertex of a link-polyline array, the index of the link it belongs to."""
    return np.repeat(np.arange(len(offsets) - 1), np.diff(offsets).astype(np.int64))


def right_normals(points: np.ndarray, owner: np.ndarray) -> np.ndarray:
    """The unit normal to the **right** of the direction of travel at every vertex.

    A vertex between two segments of the same link takes the normalised sum of
    both segments' normals; a link's end vertices take their one segment's.
    Segments that join two different links are ignored.
    """
    d = points[1:] - points[:-1]
    length = np.hypot(d[:, 0], d[:, 1])
    ok = (owner[1:] == owner[:-1]) & (length > 1e-9)
    tangent = np.zeros_like(d)
    tangent[ok] = d[ok] / length[ok, None]
    segment_normal = np.column_stack([tangent[:, 1], -tangent[:, 0]])
    normal = np.zeros_like(points)
    normal[:-1] += segment_normal
    normal[1:] += segment_normal
    norm = np.hypot(normal[:, 0], normal[:, 1])
    norm[norm < 1e-9] = 1.0
    return normal / norm[:, None]


def offset_points(points: np.ndarray, owner: np.ndarray, distance: np.ndarray) -> np.ndarray:
    """Move every vertex to the right of travel by its link's ``distance`` (metres).

    Two directions of one street each move to their *own* right, so they end
    up on opposite sides, ``distance_a + distance_b`` apart, and never overlap.
    """
    return points + right_normals(points, owner) * distance[owner, None]


def polylines(points: np.ndarray, offsets: np.ndarray, links: np.ndarray) -> list[np.ndarray]:
    """The polylines of `links`, in that order, as views into `points`.

    Whole polylines rather than separate segments: a plotting library pays per
    path, so this keeps the path count at the number of links, not vertices.
    """
    start, stop = offsets[links].astype(np.int64), offsets[links + 1].astype(np.int64)
    return [points[a:b] for a, b in zip(start.tolist(), stop.tolist(), strict=True)]


def chevron_triangles(
    points: np.ndarray,
    offsets: np.ndarray,
    links: np.ndarray,
    size: np.ndarray,
) -> np.ndarray:
    """One small direction triangle at the middle of each of `links`.

    ``size`` is each triangle's half-length in metres. Returns ``(k, 3, 2)``,
    pointing along the link's direction of travel. Links with fewer than two
    points are skipped, so ``k`` may be less than ``len(links)``.
    """
    start = offsets[links].astype(np.int64)
    count = np.diff(offsets)[links].astype(np.int64)
    ok = count >= 2
    start, count, size = start[ok], count[ok], size[ok]
    mid = start + (count - 1) // 2
    direction = points[mid + 1] - points[mid]
    length = np.hypot(direction[:, 0], direction[:, 1])
    length[length < 1e-9] = 1.0
    t = direction / length[:, None]
    n = np.column_stack([-t[:, 1], t[:, 0]])
    centre = (points[mid] + points[mid + 1]) / 2
    s = size[:, None]
    return np.stack(
        [centre + t * s, centre - t * s * 0.6 + n * s * 0.75, centre - t * s * 0.6 - n * s * 0.75],
        axis=1,
    )


def bezier_curves(
    a: np.ndarray, b: np.ndarray, bend: float = 0.18, samples: int = 17
) -> np.ndarray:
    """Gentle quadratic curves from each ``a`` to each ``b``, bending to the **right**.

    A pair ``a -> b`` and its reverse ``b -> a`` therefore curve to opposite
    sides, so the two directions of an origin-destination pair are both
    visible. ``a`` and ``b`` are ``(n, 2)``; the result is ``(n, samples, 2)``.
    """
    d = b - a
    control = (a + b) / 2 + np.column_stack([d[:, 1], -d[:, 0]]) * bend
    t = np.linspace(0.0, 1.0, samples)[None, :, None]
    a_, c_, b_ = a[:, None, :], control[:, None, :], b[:, None, :]
    return (1 - t) ** 2 * a_ + 2 * (1 - t) * t * c_ + t**2 * b_
