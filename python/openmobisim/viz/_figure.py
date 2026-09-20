"""What every openmobisim figure shares (S162).

The page, the signal glyph, the legend chips and the provenance strip.

A figure is a page with a map area in the middle. Everything around the map
is furniture and lives here, so a map of links, a map of demand and (later) a
map of transit look like one family. Text and marks are placed in figure
fractions, and the map's metres-per-point scale is fixed up front, so a
ribbon's width in points and its offset in metres always agree.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

import numpy as np

from openmobisim.viz._style import AMBER, EMBER, ION, Theme, font_mono, font_sans

__all__ = [
    "Page",
    "draw_furniture",
    "font_mono_name",
    "load_matplotlib",
    "nice_number",
    "ramp_rgb",
    "rgb",
]

#: The map area, as figure fractions ``(left, bottom, width, height)``.
MAP_RECT = (0.02, 0.085, 0.96, 0.78)


def load_matplotlib() -> Any:
    """Import matplotlib or explain how to get it."""
    try:
        import matplotlib
    except ImportError as e:
        raise ImportError("figures need matplotlib: pip install 'openmobisim[viz]'") from e
    return matplotlib


def font_mono_name() -> str:
    """The name of the mono font, for text drawn outside the furniture."""
    return font_mono()


def rgb(hex_colour: str) -> np.ndarray:
    """``"#rrggbb"`` as a float RGB triple in ``[0, 1]``."""
    h = hex_colour.lstrip("#")
    return np.array([int(h[i : i + 2], 16) for i in (0, 2, 4)], dtype=np.float64) / 255.0


def ramp_rgb(stops: tuple[str, ...], t: np.ndarray) -> np.ndarray:
    """Colours along a ramp of hex `stops` at positions `t` in ``[0, 1]``: ``(n, 3)``."""
    colours = np.stack([rgb(s) for s in stops])
    positions = np.linspace(0.0, 1.0, len(stops))
    t = np.clip(t, 0.0, 1.0)
    return np.column_stack([np.interp(t, positions, colours[:, c]) for c in range(3)])


def nice_number(x: float) -> float:
    """The nearest of 1, 2, 5 times a power of ten at or below `x` (for legend values)."""
    if x <= 0:
        return 1.0
    power = 10.0 ** np.floor(np.log10(x))
    for m in (5.0, 2.0, 1.0):
        if m * power <= x:
            return float(m * power)
    return float(power)


@dataclass
class Page:
    """A figure being drawn: the figure, its map axes and the map's scale."""

    fig: Any
    ax: Any
    theme: Theme
    size: tuple[float, float]
    #: Metres on the map per typographic point on the page.
    metres_per_point: float
    #: Furniture and ribbons scale with the page: 1 at 16 inches wide, less
    #: below, so a journal-column figure keeps the same proportions.
    k: float = 1.0

    @classmethod
    def new(
        cls,
        theme: Theme,
        size: tuple[float, float],
        dpi: int,
        lo: np.ndarray,
        hi: np.ndarray,
    ) -> Page:
        """A blank page whose map area shows the rectangle ``lo``..``hi`` (metres)."""
        load_matplotlib()
        from matplotlib.figure import Figure

        fig = Figure(figsize=size, dpi=dpi, facecolor=theme.surface)
        left, bottom, width, height = MAP_RECT
        ax = fig.add_axes((left, bottom, width, height), facecolor=theme.surface)
        ax.set_axis_off()
        box_pt = np.array([width * size[0], height * size[1]]) * 72.0
        centre = (lo + hi) / 2
        span = np.maximum((hi - lo) * 1.04, 1.0)
        mpp = float(max(span[0] / box_pt[0], span[1] / box_pt[1]))
        ax.set_xlim(centre[0] - mpp * box_pt[0] / 2, centre[0] + mpp * box_pt[0] / 2)
        ax.set_ylim(centre[1] - mpp * box_pt[1] / 2, centre[1] + mpp * box_pt[1] / 2)
        k = float(np.clip(size[0] / 16.0, 0.4, 1.5))
        return cls(fig=fig, ax=ax, theme=theme, size=size, metres_per_point=mpp, k=k)


def draw_furniture(
    page: Page,
    *,
    title: str,
    subtitle: str,
    note: str | None,
    provenance: str,
    width_legend: tuple[str, list[tuple[float, float]], str] | None = None,
    colour_legend: tuple[str, tuple[str, ...], str, str] | None = None,
    credit: str | None = None,
    logo: bool = True,
) -> None:
    """Title with the signal glyph, legends, the provenance strip and the logo.

    `width_legend` is ``(label, [(value, width_pt), ...], unit)``; `colour_legend`
    is ``(label, ramp_stops, low_label, high_label)``. `credit` is the user's own
    line (a name, an institution, their copyright) placed before the logo; the
    logo is a signature, not a claim over the figure or its data.
    """
    from matplotlib.lines import Line2D
    from matplotlib.patches import Ellipse

    fig, t, k = page.fig, page.theme, page.k
    w, h = page.size
    sans, mono = font_sans(), font_mono()

    # The signal: three lamps, the only decoration.
    for i, colour in enumerate((EMBER[500], AMBER, ION[400])):
        fig.add_artist(
            Ellipse(
                (0.0235 + i * 0.0105, 0.943),
                0.0084 * k,
                0.0084 * k * w / h,
                transform=fig.transFigure,
                facecolor=colour,
                edgecolor="none",
            )
        )
    fig.text(
        0.0755, 0.9345, title, color=t.ink, fontsize=21 * k, fontweight="bold", fontfamily=sans
    )
    fig.text(0.0235, 0.9075, subtitle, color=t.ink2, fontsize=11.5 * k, fontfamily=sans)
    if note:
        fig.text(
            0.0235, 0.885, note, color=t.muted, fontsize=10 * k, fontfamily=sans, style="italic"
        )

    # Legends, top right.
    lx, ly = 0.585, 0.905
    if width_legend is not None:
        label, samples, unit = width_legend
        fig.text(lx, ly + 0.030, label, color=t.ink2, fontsize=9.5 * k, fontfamily=sans)
        for i, (value, width_pt) in enumerate(samples):
            x0 = lx + i * 0.075
            fig.add_artist(
                Line2D(
                    [x0, x0 + 0.05],
                    [ly + 0.014] * 2,
                    transform=fig.transFigure,
                    color=t.free,
                    linewidth=width_pt,
                    solid_capstyle="round",
                )
            )
            fig.text(
                x0,
                ly - 0.006,
                f"{value:g} {unit}",
                color=t.muted,
                fontsize=8.5 * k,
                fontfamily=mono,
            )
    if colour_legend is not None:
        label, stops, low, high = colour_legend
        cx = 0.815
        fig.text(cx, ly + 0.030, label, color=t.ink2, fontsize=9.5 * k, fontfamily=sans)
        bar = fig.add_axes((cx, ly + 0.008, 0.16, 0.012), facecolor=t.surface)
        bar.set_axis_off()
        bar.imshow(ramp_rgb(stops, np.linspace(0, 1, 256))[None, :, :], aspect="auto")
        fig.text(cx, ly - 0.006, low, color=t.muted, fontsize=8.5 * k, fontfamily=mono)
        fig.text(
            cx + 0.16,
            ly - 0.006,
            high,
            color=t.muted,
            fontsize=8.5 * k,
            fontfamily=mono,
            ha="right",
        )

    # The provenance strip: what is needed to regenerate the figure.
    fig.add_artist(
        Line2D(
            [0.0235, 0.9765],
            [0.030, 0.030],
            transform=fig.transFigure,
            color=t.base,
            linewidth=0.8,
        )
    )
    fig.text(0.0235, 0.0105, provenance, color=t.muted, fontsize=8.6 * k, fontfamily=mono)
    _draw_logo(page, credit, logo)


def _draw_logo(page: Page, credit: str | None, logo: bool) -> None:
    """The signature at the bottom right: an optional credit, then the mark.

    The mark is the three-lamp signal and the wordmark, a lockup like a
    publisher's imprint. It is a logo, not a copyright notice: a figure and its
    data belong to whoever made the run, so the credit line is theirs to fill.
    """
    from matplotlib.backends.backend_agg import FigureCanvasAgg
    from matplotlib.lines import Line2D
    from matplotlib.patches import Ellipse

    fig, t, k = page.fig, page.theme, page.k
    w, h = page.size
    sans = font_sans()
    canvas = FigureCanvasAgg(fig)
    renderer = canvas.get_renderer()
    inverse = fig.transFigure.inverted()
    right, y = 0.9765, 0.0105
    if logo:
        word = fig.text(
            right,
            y,
            "openmobisim",
            color=t.ink,
            fontsize=11.5 * k,
            fontweight="bold",
            ha="right",
            fontfamily=sans,
        )
        box = word.get_window_extent(renderer).transformed(inverse)
        radius = 0.0034 * k
        gap = 0.0075 * k
        lamp_y = y + (box.y1 - box.y0) * 0.36
        x = box.x0 - gap
        for colour in (ION[400], AMBER, EMBER[500]):  # read left to right: ember, amber, ion
            x -= radius
            fig.add_artist(
                Ellipse(
                    (x, lamp_y),
                    2 * radius,
                    2 * radius * w / h,
                    transform=fig.transFigure,
                    facecolor=colour,
                    edgecolor="none",
                )
            )
            x -= radius + 0.0022 * k
        right = x - 0.0035 * k
    if credit:
        if logo:
            fig.add_artist(
                Line2D(
                    [right - 0.006 * k] * 2,
                    [y - 0.002, y + 0.018],
                    transform=fig.transFigure,
                    color=t.base,
                    linewidth=1.0,
                )
            )
            right -= 0.012 * k
        fig.text(right, y, credit, color=t.ink2, fontsize=9.5 * k, ha="right", fontfamily=sans)
