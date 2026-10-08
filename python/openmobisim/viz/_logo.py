"""The openmobisim mark: a cactus of three flat shapes, the arms standing apart from the trunk.

One drawing, kept here, for every place the mark appears: the figures (matplotlib patches),
the interactive page and its exports (an SVG string), and the page's tab icon. Drawn on a
64-unit square, y downwards, in the identity's teal (the first stop of the spectrum ramp):
``TEAL_PAPER`` on paper, ``TEAL_NIGHT`` on night.
"""

from __future__ import annotations

from typing import Any

from openmobisim.viz._style import TEAL_NIGHT, TEAL_PAPER

__all__ = ["LOGO_PATH", "logo_colour", "logo_patches", "logo_svg"]

#: The three shapes as one SVG path (trunk, left arm, right arm), on a 64-unit square.
LOGO_PATH = (
    "M32 6a5 5 0 0 1 5 5v42a5 5 0 0 1-10 0V11a5 5 0 0 1 5-5Z"
    "M24 44H21Q13 44 13 36V27a4 4 0 0 1 8 0V36H24Z"
    "M40 36H43Q51 36 51 28V18a4 4 0 0 0-8 0V28H40Z"
)

#: A quarter circle as a cubic Bézier: the control points' distance, as a share of the radius.
_K = 0.5523


def logo_colour(night: bool) -> str:
    """The mark's colour on the paper or the night surface."""
    return TEAL_NIGHT if night else TEAL_PAPER


def logo_svg(colour: str = TEAL_PAPER, size: int = 64) -> str:
    """The mark as a standalone SVG document, ``size`` pixels square."""
    return (
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64" '
        f'width="{size}" height="{size}">'
        f'<path d="{LOGO_PATH}" fill="{colour}"/></svg>'
    )


def _shapes() -> list[tuple[list[tuple[float, float]], list[int]]]:
    """The three shapes as (vertices, codes) in matplotlib's ``Path`` codes, y downwards."""
    move, line, curve3, curve4, close = 1, 2, 3, 4, 79
    k = _K

    def arc(
        cx: float, cy: float, r: float, quarters: list[tuple[float, float, float, float]]
    ) -> list:
        """Béziers along quarter circles, each ``(from dx, from dy, to dx, to dy)`` in radii."""
        points = []
        for fx, fy, tx, ty in quarters:
            sx, sy = cx + r * fx, cy + r * fy
            ex, ey = cx + r * tx, cy + r * ty
            # Over a quarter turn, the tangent at the start points along the end's radius, and back.
            c1 = (sx + k * r * tx, sy + k * r * ty)
            c2 = (ex + k * r * fx, ey + k * r * fy)
            points += [c1, c2, (ex, ey)]
        return points

    trunk_v = [(27.0, 11.0)]
    trunk_v += arc(32, 11, 5, [(-1, 0, 0, -1), (0, -1, 1, 0)])
    trunk_v += [(37.0, 53.0)]
    trunk_v += arc(32, 53, 5, [(1, 0, 0, 1), (0, 1, -1, 0)])
    trunk_v += [(27.0, 11.0), (27.0, 11.0)]
    trunk_c = [move] + [curve4] * 6 + [line] + [curve4] * 6 + [line, close]

    left_v = [(24.0, 44.0), (21.0, 44.0), (13.0, 44.0), (13.0, 36.0), (13.0, 27.0)]
    left_v += arc(17, 27, 4, [(-1, 0, 0, -1), (0, -1, 1, 0)])
    left_v += [(21.0, 36.0), (24.0, 36.0), (24.0, 44.0)]
    left_c = [move, line, curve3, curve3, line] + [curve4] * 6 + [line, line, close]

    right_v = [(40.0, 36.0), (43.0, 36.0), (51.0, 36.0), (51.0, 28.0), (51.0, 18.0)]
    right_v += arc(47, 18, 4, [(1, 0, 0, -1), (0, -1, -1, 0)])
    right_v += [(43.0, 28.0), (40.0, 28.0), (40.0, 36.0)]
    right_c = [move, line, curve3, curve3, line] + [curve4] * 6 + [line, line, close]
    return [(trunk_v, trunk_c), (left_v, left_c), (right_v, right_c)]


def logo_patches(
    x: float, y: float, height: float, aspect: float, colour: str, transform: Any
) -> list[Any]:
    """The mark as matplotlib patches, its bottom left at ``(x, y)`` and ``height`` tall.

    In ``transform``'s units; ``aspect`` is the height of one unit of x in units of y (a
    figure's width over its height, for figure coordinates), so the mark stays square.
    """
    from matplotlib.patches import PathPatch
    from matplotlib.path import Path

    s = height / 64.0
    patches = []
    for vertices, codes in _shapes():
        pts = [(x + vx * s / aspect, y + (64.0 - vy) * s) for vx, vy in vertices]
        patches.append(
            PathPatch(Path(pts, codes), facecolor=colour, edgecolor="none", transform=transform)
        )
    return patches
