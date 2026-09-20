"""The openmobisim look, in one place (S162).

Every colour, font and mark that makes a figure recognisably openmobisim is
defined here, so changing the look is a change to this file and nothing else.

The two brand ramps were built in OKLCH and checked with a lightness-monotone,
adjacent-step, single-hue and contrast validator: *ion* is a cool azure for
volume, *ember* a warm coral-crimson for delay. Usable ranges: on the paper
theme steps 400-700 (more = darker); on the night theme steps 100-600
(more = lighter, so that congestion glows). The pair is also the diverging
scale for differences between scenarios.
"""

from __future__ import annotations

from dataclasses import dataclass
from functools import cache

__all__ = ["EMBER", "ION", "THEMES", "Theme", "font_mono", "font_sans", "get_theme"]

ION = {
    100: "#e0f0f8",
    200: "#ace2ff",
    300: "#67cdfe",
    400: "#0eb5f1",
    500: "#099acf",
    600: "#057da9",
    700: "#055a7a",
}
EMBER = {
    100: "#fae8e7",
    200: "#ffcac7",
    300: "#ffa39f",
    400: "#ff7475",
    500: "#ef4b53",
    600: "#ce2739",
    700: "#9d0221",
}
#: The middle lamp of the signal glyph.
AMBER = "#fab219"

_SANS = ("Inter", "Helvetica Neue", "Segoe UI", "DejaVu Sans")
_MONO = ("JetBrains Mono", "Menlo", "Consolas", "DejaVu Sans Mono")


@dataclass(frozen=True)
class Theme:
    """One theme's colours. All are ``#rrggbb`` strings."""

    name: str
    surface: str
    ink: str
    ink2: str
    muted: str
    #: The network's own geometry, drawn quietly under the data.
    base: str
    #: A ribbon with no delay: it recedes.
    free: str
    #: Whether flows get a soft glow (night) or a hairline halo (paper).
    glow: bool
    #: Delay, low to high, as the colour stops of a continuous ramp.
    ramp_delay: tuple[str, ...]
    #: Volume, low to high.
    ramp_volume: tuple[str, ...]


THEMES: dict[str, Theme] = {
    "paper": Theme(
        name="paper",
        surface="#fbfcfd",
        ink="#0a1626",
        ink2="#4b5b6e",
        muted="#8593a6",
        base="#dce3ea",
        free="#8fa1b5",
        glow=False,
        ramp_delay=(EMBER[300], EMBER[400], EMBER[500], EMBER[600], EMBER[700]),
        ramp_volume=(ION[300], ION[400], ION[500], ION[600], ION[700]),
    ),
    "night": Theme(
        name="night",
        surface="#0a1220",
        ink="#f2f6fa",
        ink2="#b4c0cf",
        muted="#7b8aa0",
        base="#1f2c40",
        free="#4f6684",
        glow=True,
        ramp_delay=(EMBER[600], EMBER[500], EMBER[400], EMBER[300]),
        ramp_volume=(ION[600], ION[500], ION[400], ION[300], ION[200]),
    ),
}


def get_theme(theme: str | Theme) -> Theme:
    """Look a theme up by name (``"paper"`` or ``"night"``), or pass one through."""
    if isinstance(theme, Theme):
        return theme
    try:
        return THEMES[theme]
    except KeyError:
        raise ValueError(f"unknown theme {theme!r}; choose from {sorted(THEMES)}") from None


@cache
def _first_available(candidates: tuple[str, ...]) -> str:
    """The first installed font of `candidates`, or the last one (matplotlib's own)."""
    from matplotlib import font_manager

    for name in candidates:
        try:
            font_manager.findfont(name, fallback_to_default=False)
        except ValueError:
            continue
        return name
    return candidates[-1]


def font_sans() -> str:
    """The label font: Inter if installed, else a system sans, else DejaVu Sans."""
    return _first_available(_SANS)


def font_mono() -> str:
    """The provenance and number font: JetBrains Mono if installed, else a system mono."""
    return _first_available(_MONO)
