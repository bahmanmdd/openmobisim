"""``transit_edit``: a timetable with lines cancelled or run at another headway (S238).

A scenario edit for frequency setting and disruption studies, like ``network_edit`` for the
roads: the original timetable is left as it was, and stops and lines keep their ids.
"""

from __future__ import annotations

from collections.abc import Mapping, Sequence

from openmobisim import _core

__all__ = ["transit_edit"]


def transit_edit(
    transit: _core.Transit,
    *,
    cancel: Sequence[str] | None = None,
    headway: Mapping[str, float | tuple[float, float, float]] | None = None,
) -> _core.Transit:
    """The same service day with a scenario's changes made (S238).

    Args:
        transit: The timetable (``transit_read_gtfs``).
        cancel: Lines that do not run that day (a whole line out of service). Everyone knows
            in advance: passengers plan without them.
        headway: ``{line: seconds}`` to run a line every so many seconds through its day (from
            its first departure to its last, ``lines()``' ``first_s`` and ``last_s``), or
            ``{line: (seconds, from_s, to_s)}`` within a window of seconds after midnight
            (``(300, 7 * 3600, 9 * 3600)``: every 5 minutes from 07:00 to 09:00). Within the
            window, each of the line's stop patterns (its runs calling at the same stops, each
            direction one) runs every ``seconds`` from its first departure there, at the times
            of its middle run there; a pattern with no run in the window gets none, so a line
            is made more or less frequent, not new. A new run is named
            ``<template run>@<seconds after midnight>``.

    A line is named by its GTFS ``route_id`` or by its name (``route_short_name``) where that
    is unique; ``transit.lines()`` lists both. Without crowding or vehicle capacities (not yet
    modelled), a headway changes the waits and transfers passengers plan on, and how many buses
    ride the roads.

    Returns:
        The changed timetable.

    Raises:
        ValueError: For an unknown or ambiguous line, a headway not above 0, or a window that
            ends before it starts.
    """
    lines = transit.lines()

    def index(line: str) -> int:
        return _line_index(lines, line)

    changes = []
    for line, value in (headway or {}).items():
        if isinstance(value, tuple):
            seconds, start, end = value
        else:
            # Its own day: from its first departure to its last.
            i = index(line)
            first, last = lines["first_s"][i], lines["last_s"][i]
            seconds, start, end = value, first or 0, (last + 1) if last is not None else 1
        if not seconds > 0:
            raise ValueError(f"line {line!r}: a headway is a number of seconds above 0")
        changes.append((index(line), round(seconds), round(start), round(end)))
    edited, _ = _core.transit_edit(
        transit, cancelled=[index(line) for line in cancel or []], headways=changes
    )
    return edited


def _line_index(lines: dict, line: str) -> int:
    """A line's index in ``transit.lines()``, by ``route_id`` or by a name only it has."""
    ids, names = lines["route_id"], lines["name"]
    if line in ids:
        return ids.index(line)
    matches = [i for i, n in enumerate(names) if n == line]
    if len(matches) == 1:
        return matches[0]
    if matches:
        raise ValueError(
            f"line {line!r} names {len(matches)} lines; give one of their route_ids: "
            f"{[ids[i] for i in matches]}"
        )
    raise ValueError(f"no line {line!r}; transit.lines() lists the route_ids and names")
