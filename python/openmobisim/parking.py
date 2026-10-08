"""Parkings from a table: park-and-ride car parks and bike parkings.

``parking_read_table`` reads a parking table — a CSV/TSV file, its text, or
in-memory rows (dicts) — into the ``Parkings`` a ``Scenario`` takes
(``parkings=``). ``openmobisim.parking_read_osm`` reads them from an
OpenStreetMap extract instead.
"""

from __future__ import annotations

import csv
import io
import math
from collections.abc import Iterable, Mapping
from pathlib import Path
from typing import Any

from openmobisim import _core

__all__ = ["parking_read_table"]

#: The columns a parking table has; the first five are required.
PARKING_COLUMNS: tuple[str, ...] = (
    "parking_id",
    "lon",
    "lat",
    "vehicle",
    "capacity",
    "name",
    "hub_id",
    "initial_occupancy",
    "fee_eur",
)


def _text(row: Mapping[str, Any], key: str) -> str | None:
    value = row.get(key)
    if value is None:
        return None
    text = str(value).strip()
    return text or None


def _number(row: Mapping[str, Any], key: str, parking: str) -> float | None:
    value = row.get(key)
    if value is None or (isinstance(value, str) and not value.strip()):
        return None
    try:
        number = float(value)
    except (TypeError, ValueError) as e:
        raise ValueError(f"parking {parking!r}: {key} is not a number: {value!r}") from e
    if not math.isfinite(number):
        raise ValueError(f"parking {parking!r}: {key} is not a finite number: {value!r}")
    return number


def _count(row: Mapping[str, Any], key: str, parking: str, default: int | None) -> int:
    number = _number(row, key, parking)
    if number is None:
        if default is None:
            raise ValueError(f"parking {parking!r}: {key} is missing")
        return default
    if number < 0 or number != int(number):
        raise ValueError(f"parking {parking!r}: {key} must be a whole number of at least 0")
    return int(number)


def _fee(row: Mapping[str, Any], parking: str) -> float | None:
    fee = _number(row, "fee_eur", parking)
    if fee is not None and fee < 0:
        raise ValueError(f"parking {parking!r}: fee_eur must be 0 or more, got {fee}")
    return fee


def _csv_rows(source: str) -> list[dict[str, str]]:
    """A CSV or TSV table's rows, from its path or its text: quoted fields and blank cells kept."""
    # The string is the table's text, not a path, if it has a line break or there is no such file.
    is_path = "\n" not in source and Path(source).exists()
    text = Path(source).read_text(encoding="utf-8-sig") if is_path else source
    header = next((line for line in text.splitlines() if line.strip()), "")
    delimiter = "\t" if "\t" in header else ","
    reader = csv.DictReader(io.StringIO(text.strip("\n")), delimiter=delimiter)
    return [{k.strip(): (v or "").strip() for k, v in row.items() if k} for row in reader]


def parking_read_table(source: str | Iterable[Mapping[str, Any]]) -> _core.Parkings:
    """Parkings from a table: one row per parking.

    Columns (names in any case):

    * ``parking_id`` — unique in the table;
    * ``lon``, ``lat`` — where it is (WGS84 degrees);
    * ``vehicle`` — ``"car"`` or ``"bike"``;
    * ``capacity`` — how many vehicles it holds;
    * optional: ``name``; ``hub_id`` (rows sharing one join one hub, such as a
      station's car park and its bike parking; else each parking is its own hub);
      ``initial_occupancy`` (vehicles parked when the day starts; 0); ``fee_eur`` (what a
      stay costs, in euros, seen by choice models as ``cost_parking_eur``; empty: the
      scenario's ``price_options`` ``parking_car_eur`` or ``parking_bike_eur``, S248).

    A run snaps each parking to its vehicle's layer and to the walk layer, and
    keeps it only if a stop is within walking distance (see ``Scenario``'s
    ``parking_options``).

    Args:
        source: A CSV/TSV file's path or text, or rows as dicts.

    Raises:
        ValueError: If a required column is missing, a value is not valid, or a
            ``parking_id`` repeats.
    """
    rows = _csv_rows(source) if isinstance(source, str) else list(source)
    parsed = []
    for row in rows:
        lowered = {str(k).strip().lower(): v for k, v in row.items()}
        parking = _text(lowered, "parking_id")
        if parking is None:
            raise ValueError(f"a parking row has no parking_id: {row!r}")
        lon = _number(lowered, "lon", parking)
        lat = _number(lowered, "lat", parking)
        if lon is None or lat is None:
            raise ValueError(f"parking {parking!r}: lon and lat are required")
        vehicle = _text(lowered, "vehicle")
        if vehicle is None:
            raise ValueError(f"parking {parking!r}: vehicle is missing")
        parsed.append(
            (
                parking,
                lon,
                lat,
                vehicle.lower(),
                _count(lowered, "capacity", parking, None),
                _text(lowered, "name"),
                _text(lowered, "hub_id"),
                _count(lowered, "initial_occupancy", parking, 0),
                _fee(lowered, parking),
            )
        )
    return _core._parking_from_rows(parsed)
