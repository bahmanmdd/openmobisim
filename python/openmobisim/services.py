"""Shared-mobility services' tables: bike-sharing docks, a fleet's vehicles, chargers.

``dock_read_table`` and ``dock_read_gbfs`` read a bike-sharing system's docks: from a table (a
CSV/TSV file, its text, or rows as dicts), or from the system's GBFS feed (the open standard
bike-sharing systems publish: ``station_information.json`` and, for the bikes docked,
``station_status.json``). ``vehicle_read_table`` and ``charger_read_table`` read an electric
fleet's vehicles and chargers from tables.

A dock or charger may name the hub it belongs to (``hub_id``); without one it joins the hub of
the nearest transit stop within 150 m, or is a hub of its own.
"""

from __future__ import annotations

import json
from collections.abc import Iterable, Mapping
from pathlib import Path
from typing import Any

from openmobisim import _core
from openmobisim.parking import _count, _csv_rows, _number, _text

__all__ = ["charger_read_table", "dock_read_gbfs", "dock_read_table", "vehicle_read_table"]


def _rows(source: str | Iterable[Mapping[str, Any]]) -> list[dict[str, Any]]:
    """The rows of a table's path or text, or of rows given, with their column names lowered."""
    rows = _csv_rows(source) if isinstance(source, str) else list(source)
    return [{str(k).strip().lower(): v for k, v in row.items()} for row in rows]


def _place(row: Mapping[str, Any], what: str, ident: str) -> tuple[float, float]:
    lon, lat = _number(row, "lon", ident), _number(row, "lat", ident)
    if lon is None or lat is None:
        raise ValueError(f"{what} {ident!r}: lon and lat are required")
    return lon, lat


def dock_read_table(source: str | Iterable[Mapping[str, Any]]) -> _core.Docks:
    """Docks from a table: one row per dock.

    Columns (names in any case):

    * ``dock_id`` — unique in the table;
    * ``lon``, ``lat`` — where it is (WGS84 degrees);
    * ``capacity`` — its slots: how many bikes it holds;
    * optional: ``name``; ``hub_id`` (the hub it belongs to); ``bikes`` (bikes docked when
      the day starts; empty: the service's ``start_share`` of the slots, half by default).

    Each dock is snapped to the walk and bike layers; one too far from either (the service's
    ``snap_m``) or without slots is left out.

    Args:
        source: A CSV/TSV file's path or text, or rows as dicts.

    Raises:
        ValueError: If a required column is missing, a value is not valid, a dock holds more
            bikes than slots, or a ``dock_id`` repeats.
    """
    parsed = []
    for row in _rows(source):
        dock = _text(row, "dock_id")
        if dock is None:
            raise ValueError(f"a dock row has no dock_id: {row!r}")
        lon, lat = _place(row, "dock", dock)
        bikes = _count(row, "bikes", dock, 0) if _number(row, "bikes", dock) is not None else None
        parsed.append(
            (
                dock,
                lon,
                lat,
                _count(row, "capacity", dock, None),
                _text(row, "name"),
                _text(row, "hub_id"),
                bikes,
            )
        )
    return _core._docks_from_rows(parsed)


def _feed(source: str | Path | Mapping[str, Any], what: str) -> list[Mapping[str, Any]]:
    """The ``data.stations`` list of a GBFS file, from its path, its text or its parsed JSON."""
    if isinstance(source, Mapping):
        doc = source
    else:
        text = str(source)
        is_path = not text.lstrip().startswith("{")
        doc = json.loads(Path(text).read_text(encoding="utf-8") if is_path else text)
    stations = doc.get("data", {}).get("stations") if isinstance(doc, Mapping) else None
    if not isinstance(stations, list):
        raise ValueError(f"{what}: not a GBFS file (no data.stations list)")
    return stations


def _name(value: object) -> str | None:
    """A GBFS name: a string (version 2), or a list of ``{"text", "language"}`` (version 3)."""
    if isinstance(value, list):
        value = next((v.get("text") for v in value if isinstance(v, Mapping)), None)
    return None if value is None else str(value).strip() or None


def dock_read_gbfs(
    station_information: str | Path | Mapping[str, Any],
    station_status: str | Path | Mapping[str, Any] | None = None,
) -> _core.Docks:
    """Docks from a bike-sharing system's GBFS feed (General Bikeshare Feed Specification).

    Every system publishes its stations in ``station_information.json`` (id, name, place,
    ``capacity``) and their state in ``station_status.json`` (bikes available now). Give the two
    files here, as paths, their text or their parsed JSON; versions 2 and 3 are read. A station
    without a ``capacity`` is left out.

    Args:
        station_information: ``station_information.json``.
        station_status: ``station_status.json``, for the bikes docked when the day starts: each
            station's ``num_bikes_available`` (version 2) or ``num_vehicles_available`` (version
            3), at most its capacity. Without it, a dock starts at the service's
            ``start_share`` of its slots. A snapshot of one moment: take one from early morning
            for a day's start.

    Raises:
        ValueError: If a file is not a GBFS file or a station is not valid.
    """
    bikes: dict[str, int] = {}
    if station_status is not None:
        for s in _feed(station_status, "station_status"):
            n = s.get("num_bikes_available", s.get("num_vehicles_available"))
            if s.get("station_id") is not None and n is not None:
                bikes[str(s["station_id"])] = int(n)
    rows = []
    for s in _feed(station_information, "station_information"):
        capacity = s.get("capacity")
        if s.get("station_id") is None or capacity is None:
            continue
        dock = str(s["station_id"])
        start = bikes.get(dock)
        rows.append(
            {
                "dock_id": dock,
                "lon": s.get("lon"),
                "lat": s.get("lat"),
                "capacity": int(capacity),
                "name": _name(s.get("name")),
                "bikes": None if start is None else min(start, int(capacity)),
            }
        )
    return dock_read_table(rows)


def vehicle_read_table(source: str | Iterable[Mapping[str, Any]]) -> _core.Vehicles:
    """An electric fleet's vehicles from a table: one row per vehicle.

    Columns (names in any case): ``vehicle_id`` (unique), ``lon``, ``lat`` (where it is when the
    day starts, WGS84 degrees) and, optionally, ``soc`` (its charge then, a share from 0 to 1;
    empty: the service's ``soc_start``, 0.8 by default). Each is snapped to the road network;
    one farther than the service's ``snap_m`` is left out.

    Args:
        source: A CSV/TSV file's path or text, or rows as dicts.

    Raises:
        ValueError: If a required column is missing, a value is not valid, or an id repeats.
    """
    parsed = []
    for row in _rows(source):
        vehicle = _text(row, "vehicle_id")
        if vehicle is None:
            raise ValueError(f"a vehicle row has no vehicle_id: {row!r}")
        lon, lat = _place(row, "vehicle", vehicle)
        parsed.append((vehicle, lon, lat, _number(row, "soc", vehicle)))
    return _core._vehicles_from_rows(parsed)


def charger_read_table(source: str | Iterable[Mapping[str, Any]]) -> _core.Chargers:
    """Chargers from a table: one row per charger.

    Columns (names in any case): ``charger_id`` (unique), ``lon``, ``lat``, ``plugs`` (how many
    vehicles it charges at once) and, optionally, ``kw`` (its power per plug; empty: the
    service's ``charger_kw``, 50 by default) and ``hub_id`` (the hub it belongs to). Each is
    snapped to the road network; one farther than the service's ``snap_m``, or without plugs,
    is left out.

    Args:
        source: A CSV/TSV file's path or text, or rows as dicts.

    Raises:
        ValueError: If a required column is missing, a value is not valid, or an id repeats.
    """
    parsed = []
    for row in _rows(source):
        charger = _text(row, "charger_id")
        if charger is None:
            raise ValueError(f"a charger row has no charger_id: {row!r}")
        lon, lat = _place(row, "charger", charger)
        parsed.append(
            (
                charger,
                lon,
                lat,
                _count(row, "plugs", charger, None),
                _text(row, "hub_id"),
                _number(row, "kw", charger),
            )
        )
    return _core._chargers_from_rows(parsed)
