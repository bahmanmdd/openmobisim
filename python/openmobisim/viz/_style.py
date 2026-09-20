"""The openmobisim look, in one place (S162).

Every colour, font and mark that makes a figure recognisably openmobisim is
defined here, so changing the look is a change to this file and nothing else.

The two brand ramps were built in OKLCH and checked with a lightness-monotone,
adjacent-step, single-hue and contrast validator: *ion* is a cool azure for
volume, *ember* a warm coral-crimson for delay and congestion. The pair is also
the diverging scale for differences between scenarios.

**Delay is a spectrum** (S164): a blue-green for little delay, gold in the
middle, and ember for a lot, so more delay is always a deeper red on both
themes. Three hues and no more, to stay clean. The single-hue ember ramp it
replaced (S162) is kept as ``ramp_delay_ember`` and selectable with
``map_link(..., ramp="ember")``; its night version ran the other way (lighter =
more) so that congestion glowed, which read as backwards and was dropped.

The paper theme's greys were darkened in S164 for contrast; the S162 values are
noted beside each.
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

#: The low and middle stops of the delay spectrum, per theme (S164). Built with
#: the same OKLCH generator as the brand ramps; the high end is ember.
TEAL_PAPER, GOLD_PAPER = "#04938e", "#ce9c0f"  # 3.7:1 and 2.4:1 on the paper surface
TEAL_NIGHT, GOLD_NIGHT = "#40c8c1", "#f7cd3a"  # 9.2:1 and 12.3:1 on the night surface

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
    #: Delay, low to high: the spectrum (blue-green, gold, ember). More delay is
    #: always a deeper red.
    ramp_delay: tuple[str, ...]
    #: The single-hue delay ramp of S162, kept for reversal.
    ramp_delay_ember: tuple[str, ...]
    #: Volume, low to high.
    ramp_volume: tuple[str, ...]


THEMES: dict[str, Theme] = {
    "paper": Theme(
        name="paper",
        surface="#fbfcfd",
        ink="#0a1626",
        ink2="#3d4c5f",  # S162: #4b5b6e
        muted="#55657a",  # S162: #8593a6; S164: #66768a
        base="#c5cfda",  # S162: #dce3ea
        free="#5d7189",  # S162: #8fa1b5
        glow=False,
        ramp_delay=(TEAL_PAPER, GOLD_PAPER, EMBER[500], EMBER[700]),
        ramp_delay_ember=(EMBER[300], EMBER[400], EMBER[500], EMBER[600], EMBER[700]),
        ramp_volume=(ION[400], ION[500], ION[600], ION[700]),  # S162: from ION[300]
    ),
    "night": Theme(
        name="night",
        surface="#0a1220",
        ink="#f2f6fa",
        ink2="#cdd6e1",  # S164: #b4c0cf
        muted="#93a2b6",  # S164: #7b8aa0
        base="#364966",  # S164: #1f2c40 - small streets were invisible
        free="#5f7897",  # S164: #4f6684
        glow=True,
        ramp_delay=(TEAL_NIGHT, GOLD_NIGHT, EMBER[500], EMBER[600]),
        ramp_delay_ember=(EMBER[600], EMBER[500], EMBER[400], EMBER[300]),
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
