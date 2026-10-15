"""What every openmobisim figure shares (S162).

The page, the signal glyph, the legend chips and the provenance strip.

A figure is a page with a map area in the middle. Everything around the map
is furniture and lives here, so a map of links, a map of demand and (later) a
map of transit look like one family. Text and marks are placed in figure
fractions, and the map's metres-per-point scale is fixed up front, so a
ribbon's width in points and its offset in metres always agree.
"""

from __future__ import annotations

import io
from dataclasses import dataclass
from typing import Any

import numpy as np

from openmobisim.viz._logo import logo_patches
from openmobisim.viz._style import Theme, font_mono, font_sans

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


#: Who made a file the package saves, in its metadata: the software and its creator (T-2), never on
#: the picture itself.
CREATOR = "openmobisim {version}, by Bahman Madadi"


def save_figure(fig: Any, path: str, surface: str) -> None:
    """Save ``fig`` at ``path`` (PNG, SVG or PDF by extension), the software in its metadata."""
    from openmobisim import __version__

    meta = {"Creator": CREATOR.format(version=__version__)}
    kind = str(path).rsplit(".", 1)[-1].lower()
    fig.savefig(path, facecolor=surface, metadata=meta if kind in ("png", "svg", "pdf") else None)


def load_matplotlib() -> Any:
    """Import matplotlib or explain how to get it."""
    try:
        import matplotlib
    except ImportError as e:
        raise ImportError("figures need matplotlib: pip install 'openmobisim[viz]'") from e
    return matplotlib


_FIGURE_CLASS: Any = None


def figure_class() -> Any:
    """The figure type the package draws on: a matplotlib ``Figure`` that a notebook can show.

    The figures are made without pyplot (no global state), so IPython has no rule to display
    them. This subclass carries its own ``_repr_png_`` (S232): a PNG of the page as designed, not
    cropped, shown when a figure ends a cell. Nothing is registered with IPython, so a user's own
    figures display as they always did (P-43). Built once, when matplotlib is first needed.
    """
    global _FIGURE_CLASS
    if _FIGURE_CLASS is None:
        from matplotlib.figure import Figure

        class OpenmobisimFigure(Figure):
            """A matplotlib figure that shows itself as a PNG in a notebook."""

            def _repr_png_(self) -> bytes:
                buffer = io.BytesIO()
                self.savefig(buffer, format="png", facecolor=self.get_facecolor())
                return buffer.getvalue()

        _FIGURE_CLASS = OpenmobisimFigure
    return _FIGURE_CLASS


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


#: Centimetres per inch: matplotlib sizes figures in inches.
CM_PER_INCH = 2.54

#: A full-screen page's width (16:9, the figures' default ``size_cm``: 40.64 x 22.86 cm).
FULL_WIDTH_CM = 40.64

#: The default figure size, width and height in centimetres: a full-screen 16:9 frame.
SIZE_CM = (40.64, 22.86)


@dataclass
class Page:
    """A figure being drawn: the figure, its map axes and the map's scale."""

    fig: Any
    ax: Any
    theme: Theme
    #: The figure's size as matplotlib holds it (it works in inches, 2.54 cm each); users give
    #: centimetres (``size_cm``), converted once in :meth:`new`.
    size: tuple[float, float]
    #: Metres on the map per typographic point on the page.
    metres_per_point: float
    #: Furniture and ribbons scale with the page: 1 at a full-screen page (40.64 cm wide), less
    #: below, so a journal-column figure keeps the same proportions.
    k: float = 1.0

    @classmethod
    def new(
        cls,
        theme: Theme,
        size_cm: tuple[float, float],
        dpi: int,
        lo: np.ndarray,
        hi: np.ndarray,
    ) -> Page:
        """A blank page whose map area shows the rectangle ``lo``..``hi`` (metres).

        ``size_cm`` is the page's width and height in centimetres.
        """
        load_matplotlib()
        size = (size_cm[0] / CM_PER_INCH, size_cm[1] / CM_PER_INCH)
        fig = figure_class()(figsize=size, dpi=dpi, facecolor=theme.surface)
        left, bottom, width, height = MAP_RECT
        ax = fig.add_axes((left, bottom, width, height), facecolor=theme.surface)
        ax.set_axis_off()
        box_pt = np.array([width * size[0], height * size[1]]) * 72.0
        centre = (lo + hi) / 2
        span = np.maximum((hi - lo) * 1.04, 1.0)
        mpp = float(max(span[0] / box_pt[0], span[1] / box_pt[1]))
        ax.set_xlim(centre[0] - mpp * box_pt[0] / 2, centre[0] + mpp * box_pt[0] / 2)
        ax.set_ylim(centre[1] - mpp * box_pt[1] / 2, centre[1] + mpp * box_pt[1] / 2)
        k = float(np.clip(size_cm[0] / FULL_WIDTH_CM, 0.4, 1.5))
        return cls(fig=fig, ax=ax, theme=theme, size=size, metres_per_point=mpp, k=k)


def draw_furniture(
    page: Page,
    *,
    title: str,
    subtitle: str,
    note: str | None,
    provenance: str,
    width_legend: tuple[str, list[tuple[float, float]], str] | None = None,
    colour_legend: tuple[str, tuple[str, ...], str | tuple[str, ...], str] | None = None,
    credit: str | None = None,
    logo: bool = True,
    compass: bool = True,
    route_legend: list[tuple[np.ndarray, str]] | None = None,
    marker_legend: list[tuple[str, bool, str]] | None = None,
) -> None:
    """Title with the signal glyph, legends, the provenance strip and the logo.

    `width_legend` is ``(label, [(value, width_pt), ...], unit)``, the unit shown once, after
    the label (empty: none); `colour_legend`
    is ``(label, ramp_stops, low_label, high_label)``, or, for bands (S268), ``(label, colours,
    labels, "")`` with one label per colour, each under its swatch; its label left out when it is
    empty or is the width
    legend's title (both show one quantity: the title once); `route_legend` is a list of
    ``(rgb, text)`` rows, one per route, in place of both; `marker_legend` is a list
    of ``(marker, filled, text)`` rows, one per kind of symbol on the map, in the place
    of the width legend. `credit` is the user's own
    line (a name, an institution, their copyright) placed before the logo; the
    logo is a signature, not a claim over the figure or its data.
    """
    from matplotlib.lines import Line2D
    from matplotlib.patches import Rectangle

    fig, t, k = page.fig, page.theme, page.k
    w, h = page.size
    sans, mono = font_sans(), font_mono()

    # The mark, the only decoration (T-2: the cactus, in the identity's teal).
    for patch in logo_patches(0.0215, 0.927, 0.042 * k, w / h, t.mark, fig.transFigure):
        fig.add_artist(patch)
    fig.text(
        0.0235 + 0.042 * k * h / w + 0.008,
        0.9345,
        title,
        color=t.ink,
        fontsize=21 * k,
        fontweight="bold",
        fontfamily=sans,
    )
    fig.text(0.0235, 0.9075, subtitle, color=t.ink2, fontsize=11.5 * k, fontfamily=sans)
    if note:
        fig.text(
            0.0235, 0.885, note, color=t.muted, fontsize=10 * k, fontfamily=sans, style="italic"
        )

    # Legends, top right.
    lx, ly = 0.585, 0.905
    title_text = None
    if width_legend is not None:
        label, samples, unit = width_legend
        # The unit once, in the title (S232): the chips stay short at any page width.
        title_text = f"{label} ({unit})" if unit else label
        fig.text(lx, ly + 0.030, title_text, color=t.ink2, fontsize=9.5 * k, fontfamily=sans)
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
                f"{value:g}",
                color=t.muted,
                fontsize=8.5 * k,
                fontfamily=mono,
            )
    if marker_legend:
        for i, (marker, filled, text) in enumerate(marker_legend[:4]):
            y0 = 0.950 - i * 0.021
            fig.add_artist(
                Line2D(
                    [lx + 0.008],
                    [y0 + 0.004],
                    transform=fig.transFigure,
                    linestyle="none",
                    marker=marker,
                    markersize=(7.0 if filled else 4.0) * k,
                    markerfacecolor=t.ink2 if filled else t.surface,
                    markeredgecolor=t.ink2 if filled else t.muted,
                    markeredgewidth=0.6 * k,
                )
            )
            fig.text(lx + 0.022, y0, text, color=t.ink2, fontsize=9 * k, fontfamily=font_mono())
    if route_legend:
        for i, (colour, text) in enumerate(route_legend[:8]):
            col, row = divmod(i, 4)
            x0 = 0.585 + col * 0.20
            y0 = 0.950 - row * 0.021
            fig.add_artist(
                Line2D(
                    [x0, x0 + 0.018],
                    [y0 + 0.004] * 2,
                    transform=fig.transFigure,
                    color=colour,
                    linewidth=3.0 * k,
                    solid_capstyle="round",
                )
            )
            fig.text(x0 + 0.024, y0, text, color=t.ink2, fontsize=9 * k, fontfamily=mono)
    if colour_legend is not None:
        label, stops, low, high = colour_legend
        cx = 0.815
        if label and label != title_text:
            # Width and colour showing the same quantity share the width legend's title (S243).
            fig.text(cx, ly + 0.030, label, color=t.ink2, fontsize=9.5 * k, fontfamily=sans)
    if colour_legend is not None and not isinstance(low, str):
        # Bands (S268): a swatch per colour, its range under it.
        step = 0.16 / len(stops)
        for i, (colour, text) in enumerate(zip(stops, low, strict=True)):
            fig.add_artist(
                Rectangle(
                    (cx + i * step, ly + 0.008),
                    step * 0.92,
                    0.012,
                    transform=fig.transFigure,
                    facecolor=colour,
                    edgecolor="none",
                )
            )
            fig.text(
                cx + i * step + step * 0.46,
                ly - 0.006,
                text,
                color=t.ink2,
                fontsize=8.5 * k,
                fontfamily=mono,
                ha="center",
            )
    elif colour_legend is not None:
        bar = fig.add_axes((cx, ly + 0.008, 0.16, 0.012), facecolor=t.surface)
        bar.set_axis_off()
        bar.imshow(ramp_rgb(stops, np.linspace(0, 1, 256))[None, :, :], aspect="auto")
        fig.text(cx, ly - 0.006, low, color=t.ink2, fontsize=8.5 * k, fontfamily=mono)
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
    line = fig.text(0.0235, 0.0105, provenance, color=t.muted, fontsize=8.6 * k, fontfamily=mono)
    if compass:
        _draw_compass(page)
    _fit_before(line, _draw_logo(page, credit, logo) - 0.008 * k)


def _draw_compass(page: Page) -> None:
    """A north arrow and a distance scale, inside the map at its lower left.

    Maps here are always north-up, so the arrow is fixed; the scale's length is
    the round number (1, 2 or 5 times a power of ten) nearest a tenth of the
    map's width, converted through the map's own metres-per-point.
    """
    from matplotlib.lines import Line2D
    from matplotlib.patches import Polygon

    fig, t, k = page.fig, page.theme, page.k
    w, h = page.size
    left, bottom, width, height = MAP_RECT
    map_width_m = page.metres_per_point * width * w * 72.0
    metres = nice_number(map_width_m * 0.11)
    bar = metres / page.metres_per_point / (72.0 * w)  # in figure widths
    x0, y0 = left + 0.014, bottom + 0.030
    tick = 0.006 * k
    # The bar: a line with an end tick at each end.
    for xs, ys in (
        ([x0, x0 + bar], [y0, y0]),
        ([x0, x0], [y0 - tick, y0 + tick]),
        ([x0 + bar, x0 + bar], [y0 - tick, y0 + tick]),
    ):
        fig.add_artist(
            Line2D(
                xs,
                ys,
                transform=fig.transFigure,
                color=t.ink,
                linewidth=1.6 * k,
                solid_capstyle="butt",
            )
        )
    label = f"{metres / 1000:g} km" if metres >= 1000 else f"{metres:g} m"
    fig.text(x0, y0 + 2.2 * tick, label, color=t.ink, fontsize=9.5 * k, fontfamily=font_mono())
    # North: a small arrow above the bar's start, with its letter.
    ax_x, ax_y = x0 + 0.004, y0 + 0.075
    arrow = 0.022
    aw = 0.0045 * k
    fig.add_artist(
        Line2D(
            [ax_x, ax_x],
            [ax_y, ax_y + arrow * 0.7],
            transform=fig.transFigure,
            color=t.ink,
            linewidth=1.6 * k,
        )
    )
    fig.add_artist(
        Polygon(
            [
                (ax_x, ax_y + arrow),
                (ax_x - aw, ax_y + arrow * 0.55),
                (ax_x + aw, ax_y + arrow * 0.55),
            ],
            closed=True,
            transform=fig.transFigure,
            facecolor=t.ink,
            edgecolor="none",
        )
    )
    fig.text(
        ax_x,
        ax_y - 0.004,
        "N",
        color=t.ink,
        fontsize=9.5 * k,
        fontweight="bold",
        ha="center",
        va="top",
        fontfamily=font_sans(),
    )


def _fit_before(text: Any, limit: float) -> None:
    """Shorten a left-aligned line in its middle, with "…", so it ends before ``limit``.

    In figure coordinates. A small figure's provenance line would otherwise run into the
    signature; its start (the version, the run) and its end (the fingerprint, the network's
    source) are what is kept. Widths are the glyphs' outlines, the same at any resolution the
    figure is saved at (a screen renderer's hinting widens small text).
    """
    from matplotlib.textpath import TextPath

    fig = text.get_figure()
    start = float(text.get_position()[0])
    width_in = fig.get_figwidth()
    prop, size = text.get_fontproperties(), text.get_fontsize()

    def fits(line: str) -> bool:
        width_pt = TextPath((0, 0), line, size=size, prop=prop).get_extents().x1 if line else 0
        return start + width_pt / 72 / width_in <= limit

    def shown(n: int) -> str:
        head, tail = (n + 1) // 2, n // 2
        return full[:head].rstrip() + " … " + full[len(full) - tail :].lstrip() if n else ""

    full = text.get_text()
    if fits(full):
        return
    lo, hi = 0, len(full) - 1  # the most characters kept that fit, by bisection
    while lo < hi:
        mid = (lo + hi + 1) // 2
        if fits(shown(mid)):
            lo = mid
        else:
            hi = mid - 1
    text.set_text(shown(lo))


def _draw_logo(page: Page, credit: str | None, logo: bool) -> float:
    """The signature at the bottom right: an optional credit, then the mark.

    The mark is the cactus and the wordmark, a lockup like a
    publisher's imprint. It is a logo, not a copyright notice: a figure and its
    data belong to whoever made the run, so the credit line is theirs to fill.
    Returns the signature's left edge, in figure coordinates.
    """
    from matplotlib.backends.backend_agg import FigureCanvasAgg
    from matplotlib.lines import Line2D
    from matplotlib.textpath import TextPath

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
        # The mark is centred on the wordmark's lowercase body (the ink of an "x", measured on
        # its outline: a text's extent is the font's whole line), not on its ascenders and
        # descenders, and kept clear of the provenance rule above it.
        x_ink = TextPath((0, 0), "x", size=11.5 * k, prop=word.get_fontproperties()).get_extents()
        middle = y + (x_ink.y0 + x_ink.y1) / 2 / 72 / fig.get_figheight()
        tall = min((box.y1 - box.y0) * 1.45, 2 * (0.030 - 0.0025 - middle))
        wide = tall * h / w
        x = box.x0 - 0.004 * k - wide
        for patch in logo_patches(x, middle - tall / 2, tall, w / h, t.mark, fig.transFigure):
            fig.add_artist(patch)
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
        label = fig.text(
            right, y, credit, color=t.ink2, fontsize=9.5 * k, ha="right", fontfamily=sans
        )
        right = label.get_window_extent(renderer).transformed(inverse).x0
    return float(right)
