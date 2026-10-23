"""Demand before a run: reading it from files, turning an OD matrix into trips, sampling it.

**Files** (the standard demand files): ``zones.csv`` (``zone_id`` and a point,
``x_coord``/``y_coord`` or ``lon``/``lat``, WGS84), ``od.csv`` (``origin_zone``,
``destination_zone``, ``trips``; optionally ``start_s``, ``end_s``, ``user_class``, ``mode``) and
``trips.csv`` (the columns of ``trips.parquet``). ``demand_read_zones``, ``demand_read_od`` and
``demand_read_trips`` read them; ``demand_from_od`` turns an OD matrix into the trips a
``Scenario`` runs.

**Traveller classes**: a class says which modes its travellers may use, what they own
and its own coefficients in the choice model (``beta_*``, its mode constants among them).
``demand_read_classes`` reads a class table (``classes.csv``); ``demand_assign_classes`` gives
every traveller a class, drawn by the classes' shares; ``Scenario(classes=...)`` runs them.

``demand_sample`` is the traveller-weight dial: simulate one traveller for every
``weight`` people, the rest of the table unchanged. Fewer travellers cost proportionally
less to choose for, route and load, at the price of sampling noise, which replications
(different ``seed`` values) measure.

**Aggregations**: ``demand_aggregate_space`` moves every trip end to its zone's point and
``demand_aggregate_time`` every departure to one instant; together they make a dynamic demand the
static one the static assignment (``flow_level=1``) loads, which applies both itself.
"""

from __future__ import annotations

import csv
import hashlib
import math
import numbers
from collections import defaultdict
from collections.abc import Iterable, Mapping
from pathlib import Path
from typing import Any, NamedTuple

import numpy as np

from openmobisim import _core, _log

__all__ = [
    "Trip",
    "demand_assign_classes",
    "demand_from_od",
    "demand_read_classes",
    "demand_read_od",
    "demand_read_trips",
    "demand_read_zone_points",
    "demand_read_zones",
    "demand_sample",
    "demand_write_trips",
]

# Columns of a trips row (``Scenario.from_parts``' demand schema).
_TRAVELLER, _CLASS, _WEIGHT, _MODE = 0, 7, 8, 9

#: The modes a trips row may state (``Scenario``'s ``MODES``).
_MODES = ("car", "bike", "walk", "transit", "car_transit", "bike_transit")


class Trip(NamedTuple):
    """One trip: a row of the demand ``Scenario.from_parts`` takes (``trips.parquet``'s columns).

    A plain tuple of the same nine or ten fields is the same row; ``Trip`` names them. Times are
    seconds after midnight, coordinates WGS 84 degrees; ``user_class`` names the traveller's class
    (one of ``classes``, or any name for the defaults). ``weight`` is how many people the row
    stands for (``None``: the scenario's ``default_weight``); ``mode`` the stated mode (``None``:
    a car trip, or with ``modes`` one that chooses).
    """

    traveller_id: str
    trip_seq: int
    origin_lon: float
    origin_lat: float
    destination_lon: float
    destination_lat: float
    departure_time_s: int
    user_class: str
    weight: int | None = None
    mode: str | None = None


def _whole(value: Any) -> int | None:
    """``value`` as an int if it is a whole number (a data frame's float column included)."""
    if isinstance(value, bool):
        return None
    if isinstance(value, numbers.Integral):
        return int(value)
    if isinstance(value, numbers.Real) and math.isfinite(value) and float(value).is_integer():
        return int(value)
    return None


def _missing(value: Any) -> bool:
    """A cell left empty: ``None`` or a data frame's NaN."""
    return value is None or (isinstance(value, float) and math.isnan(value))


def _trip_checked(i: int, row: Any) -> Trip:
    """Row ``i`` of a demand as a ``Trip``, or a ``ValueError`` naming the row and the field."""

    def bad(field: str, what: str, value: Any) -> ValueError:
        return ValueError(
            f"demand row {i} (from 0): {field} must be {what}, got {value!r}; the row: {row!r}"
        )

    if not isinstance(row, tuple | list) or len(row) not in (9, 10):
        raise ValueError(
            f"demand row {i} (from 0): a row has 9 fields (traveller_id, trip_seq, origin_lon, "
            "origin_lat, destination_lon, destination_lat, departure_time_s, user_class, weight) "
            f"or 10 (and mode); got {row!r}"
        )
    traveller, seq, olon, olat, dlon, dlat, depart, user_class, weight, *rest = row
    mode = rest[0] if rest else None
    if not isinstance(traveller, str) or not traveller:
        raise bad("traveller_id", "a non-empty string", traveller)
    seq_i = _whole(seq)
    if seq_i is None or seq_i < 0:
        raise bad("trip_seq", "a whole number from 0", seq)
    places = (
        ("origin_lon", olon, 180),
        ("origin_lat", olat, 90),
        ("destination_lon", dlon, 180),
        ("destination_lat", dlat, 90),
    )
    for field, value, limit in places:
        ok = isinstance(value, numbers.Real) and not isinstance(value, bool)
        if not ok or not math.isfinite(value) or abs(value) > limit:
            raise bad(field, f"degrees from -{limit} to {limit}", value)
    depart_i = _whole(depart)
    if depart_i is None or depart_i < 0:
        raise bad("departure_time_s", "whole seconds after midnight, from 0", depart)
    if not isinstance(user_class, str):
        raise bad("user_class", "a class name (a string)", user_class)
    if _missing(weight):
        weight = None
    else:
        weight_i = _whole(weight)
        if weight_i is None or weight_i < 1:
            raise bad("weight", "a whole number from 1, or None", weight)
        weight = weight_i
    if _missing(mode):
        mode = None
    elif mode not in _MODES:
        raise bad("mode", f"one of {_MODES} or None", mode)
    return Trip(traveller, seq_i, float(olon), float(olat), float(dlon), float(dlat), depart_i,
                user_class, weight, mode)  # fmt: skip


def _trips_checked(demand: Any) -> list[Trip]:
    """A demand as checked ``Trip`` rows: rows (tuples, ``Trip``), columns or a data frame.

    The columns are those of ``trips.parquet``: the eight of ``Trip`` up to ``user_class``,
    with ``weight`` and ``mode`` optional. A data frame (pandas or polars) is read by its columns.
    """
    if hasattr(demand, "columns") and hasattr(demand, "to_dict"):  # a pandas or polars frame
        try:
            demand = demand.to_dict("list")  # pandas
        except (TypeError, ValueError):
            demand = demand.to_dict(as_series=False)  # polars
    if isinstance(demand, Mapping):
        names = list(demand)
        unknown = [n for n in names if n not in Trip._fields]
        missing = [n for n in Trip._fields[:8] if n not in demand]
        if unknown or missing:
            raise ValueError(
                f"a demand given as columns has the columns of trips.parquet {list(Trip._fields)} "
                f"(weight and mode optional); unknown {unknown}, missing {missing}"
            )
        lengths = {len(demand[n]) for n in names}
        if len(lengths) > 1:
            raise ValueError(f"a demand's columns must have one length, got {sorted(lengths)}")
        count = lengths.pop() if lengths else 0
        columns = [list(demand[n]) if n in demand else [None] * count for n in Trip._fields]
        demand = list(zip(*columns, strict=True))
    rows = [_trip_checked(i, row) for i, row in enumerate(demand)]
    _trips_keyed_once(rows)
    return rows


def _trips_keyed_once(rows: list[Trip]) -> None:
    """Refuse a second row with a ``(traveller_id, trip_seq)`` already given (X-68).

    A trip is keyed by the pair: a repeated key would replace or drop a trip, and every total of
    the run would be of a smaller demand than the one given, with nothing to say so.
    """
    first: dict[tuple[str, int], int] = {}
    repeats = 0
    named: tuple[int, int, str, int] | None = None
    for i, row in enumerate(rows):
        key = (row.traveller_id, row.trip_seq)
        earlier = first.setdefault(key, i)
        if earlier != i:
            repeats += 1
            if named is None:
                named = (i, earlier, row.traveller_id, row.trip_seq)
    if named is not None:
        i, earlier, traveller, seq = named
        raise ValueError(
            f"demand row {i} (from 0) repeats row {earlier}'s (traveller_id, trip_seq) = "
            f"({traveller!r}, {seq}); {repeats} row(s) repeat a key already given. A traveller's "
            "trips take distinct trip_seq values (0, 1, 2, … in the order of their day), and "
            "another traveller takes another traveller_id."
        )


def _zone_points(zones: Mapping[str, tuple[float, float]] | str | Path) -> list[tuple]:
    """Zones as ``{zone_id: (lon, lat)}`` or a ``zones.csv`` path, as their points in order."""
    if isinstance(zones, str | Path):
        zones = demand_read_zones(zones)
    if not isinstance(zones, Mapping):
        raise ValueError(
            "zones must be {zone_id: (lon, lat)} (Case.zones(), demand_read_zones) or a zones.csv "
            f"path, got {type(zones).__name__}"
        )
    points = []
    for zone, point in zones.items():
        lon, lat = point
        if not (math.isfinite(lon) and math.isfinite(lat) and abs(lon) <= 180 and abs(lat) <= 90):
            raise ValueError(f"zone {zone!r}: a point is (lon, lat) in degrees, got {point!r}")
        points.append((float(lon), float(lat)))
    return points


def demand_aggregate_space(
    trips: Any, zones: Mapping[str, tuple[float, float]] | str | Path, quiet: bool = False
) -> list[Trip]:
    """Every trip end moved to its zone's point: the demand aggregated over space.

    A trips table carries no zone, so a trip end's zone is **the zone whose point is nearest
    it** (by the ground distance at the city's scale; of equally near ones, the first listed); a
    gateway, a zone of one point, keeps its trips there. The trips of one zone pair then share
    their two points, and so their routes. Nothing else about a trip changes. On a demand made
    with one point per zone (``Case.demand(one_point_per_zone=True)``, ``demand_from_od``'s
    ``points_per_zone=1``) nothing moves; a trip end drawn near the edge of its zone (a
    ``spread_m``, a ``zone_points`` table) may be nearer a neighbour's point and go there.
    The static assignment (``flow_level=1``) applies it before it routes, with the scenario's
    ``zones``. **Cost:** one comparison per trip end and zone.

    Args:
        trips: In any form ``Scenario.from_parts`` takes (rows, columns, a data frame); checked.
        zones: ``{zone_id: (lon, lat)}`` (``Case.zones()``, ``demand_read_zones``) or a path to a
            ``zones.csv``.
        quiet: If ``False`` (the default), log one line: how many trip ends moved and how many
            were at their zone's point already.

    Returns:
        The trips as ``Trip`` rows, in the order given.

    Raises:
        ValueError: If a row is refused (as ``Scenario.from_parts`` refuses it) or a zone's point
            is not (lon, lat) in degrees.
    """
    rows = _trips_checked(trips)
    out, moved, kept = _core.demand_aggregate_space([tuple(r) for r in rows], _zone_points(zones))
    if not quiet:
        _log.logger.info(
            "demand_aggregate_space: %d of %d trip ends moved to their zone's point "
            "(%d there already)",
            moved,
            2 * len(rows),
            kept,
        )
    return [Trip(*r) for r in out]


def demand_aggregate_time(trips: Any, start: str | float | None = None) -> list[Trip]:
    """Every departure set to one instant, ``start``: the demand aggregated in time to one bin.

    The static assignment (``flow_level=1``) applies it before it routes, with the window's start;
    the hour of flow it loads is read from the departures before they move (``loading_options``
    ``period_s``). Nothing else about a trip changes.

    Args:
        trips: In any form ``Scenario.from_parts`` takes; checked.
        start: The instant, a clock time (``"07:30"``) or seconds after midnight; ``None`` (the
            default), the earliest departure rounded down to the quarter hour, the window's
            start by default.

    Returns:
        The trips as ``Trip`` rows, in the order given.

    Raises:
        ValueError: If a row is refused, or ``start`` is not a time of day.
    """
    rows = _trips_checked(trips)
    if start is not None:
        from openmobisim.scenario import _clock_s

        try:
            start = int(_clock_s(start, "start"))
        except ValueError:
            raise ValueError(
                f"start {start!r}: give a clock time such as '07:30' or seconds after midnight"
            ) from None
    out = _core.demand_aggregate_time([tuple(r) for r in rows], start)
    return [Trip(*r) for r in out]


def _draw(seed: int, traveller: object) -> bytes:
    """A traveller's place in the sample's random order: a hash of the seed and its id."""
    return hashlib.blake2b(f"{seed}\x1f{traveller}".encode(), digest_size=8).digest()


def demand_sample(trips: list[tuple], weight: int, seed: int = 0) -> list[tuple]:
    """Keep one traveller in every ``weight``, each standing for ``weight`` times as many people.

    A traveller is kept or left out with all of their trips, so a day's chain of trips stays
    whole. The travellers are put in a random order fixed by ``seed`` and their id, and every
    ``weight``-th is kept, **within each group of the same user class and the same stated mode
    of their first trip**: each group keeps its share of the demand to within one traveller,
    and a mode or class that is rare is not lost or doubled by chance. Every kept trip's
    weight (column 9, ``None`` meaning the scenario's ``default_weight``, taken as 1 here) is
    multiplied by ``weight``, so totals in the results (travel time, volumes, mode shares by
    weight) estimate those of the whole table.

    **A small aggregation, not a rescaled network**: a kept traveller is a group of
    ``weight`` people who travel and choose together; on the roads it is a platoon of ``weight``
    cars, each counted at its full size on every link it uses, and the network keeps its real
    capacities, storage and signal timings. (Sample-based agent simulations such as MATSim often
    scale the network's capacities to the sample instead, a well-established approach; here
    nothing about the network is scaled.) What a group gives up is individuality: one choice for
    all its members, one platoon in the queues; under congestion its queues form and clear in
    blocks, so delays read a little higher than with one traveller per person.

    The same table, ``weight`` and ``seed`` always keep the same travellers, on any machine;
    another ``seed`` is another sample, for replications.

    Args:
        trips: Rows in the schema of ``Scenario.from_parts``' ``demand``.
        weight: People per kept traveller, a whole number of at least 1 (1 keeps everyone).
        seed: Which sample.

    Returns:
        The kept rows, in their original order, with their weights multiplied.

    Raises:
        ValueError: If ``weight`` is not a whole number of at least 1.
    """
    if isinstance(weight, bool) or int(weight) != weight or weight < 1:
        raise ValueError(f"weight must be a whole number of at least 1, got {weight!r}")
    weight = int(weight)
    if weight == 1:
        return list(trips)
    first: dict[object, tuple] = {}
    for row in trips:
        first.setdefault(row[_TRAVELLER], row)
    groups: dict[tuple, list[object]] = defaultdict(list)
    for traveller, row in first.items():
        mode = row[_MODE] if len(row) > _MODE else None
        groups[(row[_CLASS], mode)].append(traveller)
    kept: set[object] = set()
    for members in groups.values():
        members.sort(key=lambda t: (_draw(seed, t), str(t)))
        kept.update(members[::weight])
    out = []
    for row in trips:
        if row[_TRAVELLER] in kept:
            row = list(row)
            row[_WEIGHT] = (row[_WEIGHT] or 1) * weight
            out.append(tuple(row))
    return out


# --- files -------------------------------------------------------------------------------------

#: The columns of a trips table, in the order of a row.
TRIP_COLUMNS = (
    "traveller_id", "trip_seq", "origin_lon", "origin_lat", "destination_lon", "destination_lat",
    "departure_time_s", "user_class", "weight", "mode",
)  # fmt: skip


def _rows(path: str | Path) -> list[dict[str, str]]:
    with Path(path).open(newline="", encoding="utf-8-sig") as f:
        return [
            {k.strip().lower(): (v or "").strip() for k, v in row.items() if k}
            for row in csv.DictReader(f)
        ]


def _first(row: Mapping[str, str], *names: str) -> str:
    for name in names:
        if row.get(name, "") != "":
            return row[name]
    return ""


def demand_read_zones(path: str | Path) -> dict[str, tuple[float, float]]:
    """Read ``zones.csv``: each zone's point, ``{zone_id: (lon, lat)}``.

    Columns: ``zone_id`` and ``x_coord``/``y_coord`` (GMNS's names) or ``lon``/``lat``, in
    WGS84 degrees. Other columns (a ``name``, a ``boundary``) are not read.

    Raises:
        ValueError: If a row has no id or no point, or an id repeats.
    """
    zones: dict[str, tuple[float, float]] = {}
    for i, row in enumerate(_rows(path)):
        zone = _first(row, "zone_id", "zone", "id")
        x, y = _first(row, "x_coord", "lon", "longitude"), _first(row, "y_coord", "lat", "latitude")
        if not zone or not x or not y:
            raise ValueError(f"{path}: row {i + 1} needs zone_id and x_coord/y_coord (or lon/lat)")
        if zone in zones:
            raise ValueError(f"{path}: zone {zone!r} is listed twice")
        zones[zone] = (float(x), float(y))
    return zones


def demand_read_zone_points(path: str | Path) -> dict[str, list[tuple[float, float, float, float]]]:
    """Read ``zone_points.csv``: where within each zone its trips may start and end, with weights.

    Columns: ``zone_id``, ``x_coord``/``y_coord`` (or ``lon``/``lat``, WGS84 degrees) and the
    weights ``weight_origin`` and ``weight_destination`` (either may be replaced by ``weight``,
    which stands for both; a missing weight is 1). A point with a residents' weight as origin and
    a workplaces' weight as destination places a zone's trips where its buildings are, instead of
    around its centre. Returns ``{zone_id: [(lon, lat, weight_origin, weight_destination), …]}``
    in the file's order; see ``demand_from_od``'s ``zone_points``.

    Raises:
        ValueError: If a row has no id or no point, or a weight is negative or not a number.
    """
    points: dict[str, list[tuple[float, float, float, float]]] = defaultdict(list)
    for i, row in enumerate(_rows(path)):
        zone = _first(row, "zone_id", "zone", "id")
        x, y = _first(row, "x_coord", "lon", "longitude"), _first(row, "y_coord", "lat", "latitude")
        if not zone or not x or not y:
            raise ValueError(f"{path}: row {i + 1} needs zone_id and x_coord/y_coord (or lon/lat)")
        weights = []
        for name in ("weight_origin", "weight_destination"):
            text = _first(row, name, "weight")
            try:
                w = float(text) if text else 1.0
            except ValueError:
                raise ValueError(
                    f"{path}: row {i + 1}: {name} must be a number, got {text!r}"
                ) from None
            if not w >= 0.0:
                raise ValueError(f"{path}: row {i + 1}: {name} must not be negative, got {w}")
            weights.append(w)
        points[zone].append((float(x), float(y), weights[0], weights[1]))
    return dict(points)


def demand_read_od(path: str | Path) -> list[dict[str, Any]]:
    """Read ``od.csv``: one row per (origin zone, destination zone[, window, class, mode]).

    Columns: ``origin_zone``, ``destination_zone``, ``trips`` (people, may be fractional);
    optional ``start_s`` and ``end_s`` (the departure window, seconds after the day's start),
    ``user_class`` and ``mode`` (``car``, ``bike``, …; empty: chosen, or car). Returns the rows
    as dicts with those keys, missing ones ``None``.

    Raises:
        ValueError: If a row lacks a zone or a number of trips, or the number is negative.
    """
    out = []
    for i, row in enumerate(_rows(path)):
        o, d = (
            _first(row, "origin_zone", "origin", "o"),
            _first(row, "destination_zone", "destination", "d"),
        )
        n = _first(row, "trips", "demand", "volume")
        if not o or not d or not n:
            raise ValueError(f"{path}: row {i + 1} needs origin_zone, destination_zone and trips")
        if float(n) < 0:
            raise ValueError(f"{path}: row {i + 1} has a negative number of trips")
        start, end = _first(row, "start_s"), _first(row, "end_s")
        out.append({
            "origin_zone": o, "destination_zone": d, "trips": float(n),
            "start_s": int(float(start)) if start else None,
            "end_s": int(float(end)) if end else None,
            "user_class": row.get("user_class") or None, "mode": row.get("mode") or None,
        })  # fmt: skip
    return out


def demand_read_trips(path: str | Path) -> list[tuple]:
    """Read ``trips.csv``: the columns of ``trips.parquet``, as rows ``Scenario`` takes.

    Columns: ``traveller_id``, ``trip_seq``, ``origin_lon``, ``origin_lat``, ``destination_lon``,
    ``destination_lat``, ``departure_time_s``, ``user_class``, and optionally ``weight`` and
    ``mode`` (empty: the default weight; no stated mode).

    Raises:
        ValueError: If a required column is missing in a row.
    """
    out = []
    for i, row in enumerate(_rows(path)):
        try:
            weight = row.get("weight", "")
            out.append((
                row["traveller_id"], int(float(row["trip_seq"])),
                float(row["origin_lon"]), float(row["origin_lat"]),
                float(row["destination_lon"]), float(row["destination_lat"]),
                int(float(row["departure_time_s"])), row["user_class"],
                int(float(weight)) if weight else None, row.get("mode") or None,
            ))  # fmt: skip
        except (KeyError, ValueError) as e:
            raise ValueError(f"{path}: row {i + 1}: {e}") from None
    return out


def demand_write_trips(trips: Iterable[tuple], path: str | Path) -> int:
    """Write trips rows as ``trips.csv`` (the columns of ``trips.parquet``); returns the count."""
    n = 0
    with Path(path).open("w", newline="", encoding="utf-8") as f:
        w = csv.writer(f, lineterminator="\n")
        w.writerow(TRIP_COLUMNS)
        for row in trips:
            row = tuple(row) + (None,) * (len(TRIP_COLUMNS) - len(row))
            w.writerow(["" if v is None else v for v in row])
            n += 1
    return n


# --- traveller classes ------------------------------------------------------------------------

#: A class's own choice-set limits, in seconds: the longest walk and bike ride offered to
#: its trips choosing their mode, and the longest walk to or from a stop.
CLASS_LIMITS = ("walk_max_s", "bike_max_s", "access_walk_max_s")
#: What a class may say besides its ``beta_*`` coefficients (its limits).
CLASS_KEYS = ("share", "modes", "owns_car", "owns_bike", "has_transit_pass", *CLASS_LIMITS)
_OWNERSHIP = ("owns_car", "owns_bike", "has_transit_pass")
_TRUE, _FALSE = ("1", "true", "yes", "y", "t"), ("0", "false", "no", "n", "f")


def _flag(value: object, what: str) -> bool:
    if isinstance(value, bool):
        return value
    if isinstance(value, (int, float)) and value in (0, 1):
        return bool(value)
    text = str(value).strip().lower()
    if text in _TRUE or text in _FALSE:
        return text in _TRUE
    raise ValueError(f"{what} must be true or false (1 or 0), got {value!r}")


def _class_entry(name: str, spec: object) -> dict[str, Any]:
    """One class, in the one shape a run reads (see ``Scenario.from_parts``' ``classes``)."""
    from openmobisim.scenario import MODES

    if isinstance(spec, (tuple, list)):
        # The ownership tuple of the old class_defaults (S127): what the class owns, every mode.
        if len(spec) != 3:
            raise ValueError(
                f"class {name!r}: a tuple is (owns_car, owns_bike, has_transit_pass), got {spec!r}"
            )
        spec = dict(zip(_OWNERSHIP, spec, strict=True))
    if not isinstance(spec, Mapping):
        raise ValueError(f"class {name!r}: give a dict of its columns, got {spec!r}")
    given = {k: v for k, v in spec.items() if v is not None and v != ""}
    # A class as ``demand_read_classes`` returns it: its coefficients under ``betas``, its
    # limits under ``limits``.
    for nested in ("betas", "limits"):
        if isinstance(given.get(nested), Mapping):
            given = {**{k: v for k, v in given.items() if k != nested}, **given[nested]}
    unknown = [k for k in given if k not in CLASS_KEYS and not str(k).startswith("beta_")]
    if unknown:
        raise ValueError(
            f"class {name!r}: no such column {unknown}; the columns are "
            f"{', '.join(CLASS_KEYS)} and the choice model's beta_* coefficients"
        )
    modes = given.get("modes")
    if modes is not None:
        modes = [m.strip() for m in modes.split(";")] if isinstance(modes, str) else list(modes)
        modes = [m for m in modes if m]
        bad = [m for m in modes if m not in MODES]
        if bad:
            raise ValueError(f"class {name!r}: modes must be among {MODES}, got {bad}")
        modes = [m for m in MODES if m in modes]
    share = given.get("share")
    if share is not None:
        share = float(share)
        if not (math.isfinite(share) and share >= 0):
            raise ValueError(f"class {name!r}: share must be a number of at least 0, got {share}")
    # Unsaid vehicles follow the modes: a class that may drive owns a car (S231); one whose
    # modes are unsaid may use every mode, so it owns a car and a bike (S243). A pass is never
    # implied (U-42): it only waives the fare, and transit needs none, so a class holds one
    # only when it says so.
    implied = {
        "owns_car": modes is None or any(m in ("car", "car_transit") for m in modes),
        "owns_bike": modes is None or any(m in ("bike", "bike_transit") for m in modes),
        "has_transit_pass": False,
    }
    entry: dict[str, Any] = {"share": share, "modes": modes}
    for key in _OWNERSHIP:
        entry[key] = _flag(given[key], f"class {name!r}: {key}") if key in given else implied[key]
    betas = {}
    for key, value in given.items():
        if str(key).startswith("beta_"):
            v = float(value)
            if not math.isfinite(v):
                raise ValueError(f"class {name!r}: {key} must be a finite number, got {value!r}")
            betas[str(key)] = v
    entry["betas"] = betas
    limits = {}
    for key in CLASS_LIMITS:
        if key in given:
            v = float(given[key])
            finite = key == "access_walk_max_s"
            if not (v > 0 and (math.isfinite(v) or not finite)):
                raise ValueError(
                    f"class {name!r}: {key} must be a {'finite ' if finite else ''}number of "
                    f"seconds above 0, got {given[key]!r}"
                )
            limits[key] = v
    entry["limits"] = limits
    return entry


def _class_table(classes: Mapping[str, object]) -> dict[str, dict[str, Any]]:
    """Every class in the one shape a run reads, in the order given."""
    if not isinstance(classes, Mapping):
        raise ValueError(f"classes must be a dict of classes by name, got {type(classes).__name__}")
    return {str(name): _class_entry(str(name), spec) for name, spec in classes.items()}


def demand_read_classes(path: str | Path) -> dict[str, dict[str, Any]]:
    """Read a class table (``classes.csv``): one row per traveller class.

    Columns: ``class`` (its name, as the trips' ``user_class``); optionally ``share`` (the share of
    the people in the class, for ``demand_assign_classes``), ``modes`` (the modes its travellers may
    use, separated by ``;``: ``car;bike;walk``; empty: every mode the run offers), ``owns_car``,
    ``owns_bike`` and ``has_transit_pass`` (``1`` or ``0``; empty: a car for a class that may drive
    or park and ride, a bike for one that may cycle, both when the modes are empty, and no pass —
    a pass only waives the fare, never gates transit), any ``beta_*`` coefficient of the choice
    model (``beta_mode_bike``, ``beta_time_min`` …; empty: the model's own) and the class's own
    choice-set limits in seconds, ``walk_max_s``, ``bike_max_s`` and ``access_walk_max_s`` (empty:
    the run's ``mode_options`` and ``transit_options``). Every value is what the user gives, not a
    calibration.

    Returns:
        The classes in the file's order, a dict by class name of dicts with the keys ``share``,
        ``modes``, ``owns_car``, ``owns_bike``, ``has_transit_pass``, ``betas`` and ``limits``:
        the shape ``Scenario(classes=...)`` and ``demand_assign_classes`` take.

    Raises:
        ValueError: If a row has no class, a class repeats, or a column or value is not one of
            those above.
    """
    out: dict[str, Any] = {}
    for i, row in enumerate(_rows(path)):
        name = _first(row, "class", "user_class", "name")
        if not name:
            raise ValueError(f"{path}: row {i + 1} has no class")
        if name in out:
            raise ValueError(f"{path}: class {name!r} is listed twice")
        spec = {k: v for k, v in row.items() if k not in ("class", "user_class", "name")}
        try:
            out[name] = _class_entry(name, spec)
        except ValueError as e:
            raise ValueError(f"{path}: row {i + 1}: {e}") from None
    return out


def demand_assign_classes(
    trips: list[tuple], classes: Mapping[str, object], seed: int = 0
) -> list[tuple]:
    """Give every traveller a class, drawn by the classes' shares.

    The shares are of people (a trip's weight, ``None`` taken as 1), scaled to sum to one, and
    are met to within one traveller: the travellers are put in a random order fixed by
    ``seed`` and their id, and the classes take consecutive runs of that order, each as many
    people as its share. Who falls in which class is random; how many is not. A traveller's
    trips all get the same class, which replaces the ``user_class`` they had.

    The same table, classes and ``seed`` always give the same classes, on any machine; another
    ``seed`` is another draw, for replications.

    Args:
        trips: Rows in the schema of ``Scenario.from_parts``' ``demand``.
        classes: The classes, as ``demand_read_classes`` returns them (or a dict of the same
            columns), each with a ``share``.
        seed: Which draw.

    Returns:
        The rows, in their original order, with their ``user_class`` replaced.

    Raises:
        ValueError: If a class has no share, or the shares sum to 0.
    """
    table = _class_table(classes)
    missing = [name for name, c in table.items() if c["share"] is None]
    if missing:
        raise ValueError(f"every class needs a share to be drawn by; {missing} have none")
    total_share = sum(c["share"] for c in table.values())
    if total_share <= 0:
        raise ValueError("the classes' shares sum to 0")
    weight: dict[object, float] = {}
    for row in trips:
        weight.setdefault(row[_TRAVELLER], float(row[_WEIGHT] or 1))
    order = sorted(weight, key=lambda t: (_draw(seed, t), str(t)))
    people = sum(weight.values())
    bounds, cumulative = [], 0.0
    for name, c in table.items():
        cumulative += c["share"] / total_share
        bounds.append((cumulative, name))
    names = [name for _, name in bounds]
    assigned: dict[object, str] = {}
    seen, k = 0.0, 0
    for traveller in order:
        middle = (seen + weight[traveller] / 2) / people
        while k < len(bounds) - 1 and middle >= bounds[k][0]:
            k += 1
        assigned[traveller] = names[k]
        seen += weight[traveller]
    out = []
    for row in trips:
        row = list(row)
        row[_CLASS] = assigned[row[_TRAVELLER]]
        out.append(tuple(row))
    return out


# --- OD to trips -------------------------------------------------------------------------------

_PROFILES = ("uniform", "am_peak")


def _key(seed: int, *parts: object) -> int:
    text = "\x1f".join(str(p) for p in (seed, *parts))
    return int.from_bytes(hashlib.blake2b(text.encode(), digest_size=8).digest(), "big")


def _integerise(cells: list[tuple[tuple, float]], seed: int) -> list[int]:
    """Whole trips per cell, keeping the group's total (rounded).

    Each cell gets its whole part, then one more for the cells with the largest remainders,
    ties broken by a keyed draw.
    """
    whole = [math.floor(x) for _, x in cells]
    target = round(sum(x for _, x in cells))
    left = target - sum(whole)
    if left > 0:
        order = sorted(
            range(len(cells)),
            key=lambda i: (-(cells[i][1] - whole[i]), _key(seed, *cells[i][0])),
        )
        for i in order[:left]:
            whole[i] += 1
    return whole


def demand_from_od(
    od: str | Path | list[Mapping[str, Any]],
    zones: str | Path | Mapping[str, tuple[float, float]],
    *,
    start_s: int = 7 * 3600,
    end_s: int = 9 * 3600,
    profile: str = "uniform",
    spread_m: float = 300.0,
    user_class: str = "default",
    seed: int = 0,
    spread_m_by_zone: Mapping[str, float] | None = None,
    points_per_zone: int | None = None,
    zone_points: Mapping[str, list[tuple[float, float, float, float]]] | None = None,
) -> list[tuple]:
    """Turn an OD matrix into the trips a ``Scenario`` runs (the deterministic integerisation).

    1. **Whole trips:** within each group of rows with the same window, class and mode, every
       cell gets the whole part of its trips, and the cells with the largest remainders one more
       each until the group's total (rounded to a whole number) is reached; a tie between
       remainders is broken by a draw keyed on the seed and the cell, so the result never
       depends on the order of the rows.
    2. **One traveller per trip,** with one trip (a chain needs a trips table): ids ``od0000001``…
       in the order of the sorted cells.
    3. **When:** a departure in the row's window (``start_s``–``end_s``, else the arguments'),
       drawn from ``profile``: ``"uniform"``, or ``"am_peak"``, a bell over the window (a Beta(3, 3)
       distribution: two thirds of the trips in its middle 40%, against 40% for ``"uniform"``).
    4. **Where:** the zone's point, moved by a random distance of up to ``spread_m`` metres in a
       random direction (uniform over the disc), so a zone's trips do not all start at one node;
       ``spread_m_by_zone`` gives some zones their own (0 for a gateway: a road where the study
       area's edge cuts it, where trips from outside enter). With ``points_per_zone``, a
       trip end is instead one of that many fixed points around the zone's (half its spread
       away, evenly around it; 1: the zone's own point), drawn at random: trips from the same
       zones then share their routes and their route searches. With ``zone_points``
       (``demand_read_zone_points``'s shape), a zone that has points draws each trip end among
       them by weight, the origin by ``weight_origin`` and the destination by
       ``weight_destination`` (both uniform when a zone's weights are all 0), so trips start
       where people live and end where they work; a trip within such a zone ends at a point
       other than its origin's when the zone has more than one (with one point, both ends are
       that point). A zone without points keeps the rule above.

    The draws (steps 3 and 4) come from one NumPy generator seeded by ``seed``, consumed in the
    sorted order of the cells, so the same inputs and seed give the same trips on any machine for
    one NumPy version: NumPy does not promise the same stream across its releases, so another
    release may give other departures and places (the whole trips of step 1 are keyed hashes and
    do not depend on it). Keep the returned trips, not only the seed, to repeat a study.

    Args:
        od: ``od.csv``'s path, or its rows (``demand_read_od``'s shape).
        zones: ``zones.csv``'s path, or ``{zone_id: (lon, lat)}``.
        start_s: The departure window's start for rows without one, seconds after the day's start.
        end_s: Its end.
        profile: ``"uniform"`` or ``"am_peak"``.
        spread_m: How far from its zone's point a trip may start or end, in metres.
        user_class: The class of rows without one.
        seed: Which draws.
        spread_m_by_zone: ``{zone_id: metres}``, the spread of these zones instead of
            ``spread_m``. The draws are the same either way: only their distance is scaled.
        points_per_zone: Draw each trip end from this many fixed points per zone instead of
            anywhere in its disc (at least 1; ``None``: anywhere).
        zone_points: ``{zone_id: [(lon, lat, weight_origin, weight_destination), …]}``, the
            places a zone's trips start and end, drawn by weight; the other zones as above.

    Returns:
        Trips rows (``traveller_id, trip_seq, origin_lon, origin_lat, destination_lon,
        destination_lat, departure_time_s, user_class, weight, mode``), weight ``None``.

    Raises:
        ValueError: For an unknown profile, a window that is empty, a zone the matrix names but
            ``zones`` does not have, a negative spread, or a zone in ``zone_points`` that
            ``zones`` does not have or whose points have a negative weight.
    """
    if profile not in _PROFILES:
        raise ValueError(f"profile must be one of {_PROFILES}, got {profile!r}")
    spreads = dict(spread_m_by_zone or {})
    ppz = points_per_zone
    if ppz is not None and (int(ppz) != ppz or ppz < 1):
        raise ValueError(f"points_per_zone must be a whole number of at least 1, got {ppz}")
    if spread_m < 0 or any(v < 0 for v in spreads.values()):
        raise ValueError(f"spread_m must not be negative: {spread_m}, by zone {spreads}")
    rows = demand_read_od(od) if isinstance(od, (str, Path)) else list(od)
    points = demand_read_zones(zones) if isinstance(zones, (str, Path)) else dict(zones)
    # A zone with its own points: their coordinates, origin weights and destination weights.
    placed: dict[str, tuple[np.ndarray, np.ndarray, np.ndarray]] = {}
    for zone, its in (zone_points or {}).items():
        if zone not in points:
            raise ValueError(f"zone_points names zone {zone!r}, which zones does not have")
        if not its:
            continue
        xy = np.array([[p[0], p[1]] for p in its], dtype=float)
        wo = np.array([p[2] for p in its], dtype=float)
        wd = np.array([p[3] for p in its], dtype=float)
        if (wo < 0).any() or (wd < 0).any():
            raise ValueError(f"zone_points: zone {zone!r} has a negative weight")
        uniform = np.full(len(its), 1.0 / len(its))
        placed[zone] = (
            xy,
            wo / wo.sum() if wo.sum() > 0 else uniform,
            wd / wd.sum() if wd.sum() > 0 else uniform,
        )
    groups: dict[tuple, list[tuple[tuple, float]]] = defaultdict(list)
    for row in rows:
        o, d = str(row["origin_zone"]), str(row["destination_zone"])
        for z in (o, d):
            if z not in points:
                raise ValueError(f"the OD matrix names zone {z!r}, which zones does not have")
        lo = row.get("start_s") if row.get("start_s") is not None else start_s
        hi = row.get("end_s") if row.get("end_s") is not None else end_s
        if hi <= lo:
            raise ValueError(f"an empty departure window, {lo}–{hi}, for {o} → {d}")
        group = (lo, hi, row.get("user_class") or user_class, row.get("mode") or "")
        groups[group].append(((o, d, *group), float(row["trips"])))
    rng = np.random.default_rng(seed)
    trips: list[tuple] = []
    for group in sorted(groups):
        cells = sorted(groups[group], key=lambda c: c[0])
        counts = _integerise(cells, seed)
        lo, hi, cls, mode = group
        for (cell, _), n in zip(cells, counts, strict=True):
            if n == 0:
                continue
            o, d = points[cell[0]], points[cell[1]]
            u = rng.beta(3.0, 3.0, n) if profile == "am_peak" else rng.random(n)
            departures = lo + np.floor(u * (hi - lo)).astype(np.int64)
            ends = []
            chosen: list[np.ndarray] = []  # each end's point, for a trip within a zone with points
            for at, zone, which in ((o, cell[0], 1), (d, cell[1], 2)):
                if zone in placed:  # one of the zone's own points, by weight (S282)
                    xy, wo, wd = placed[zone]
                    k = rng.choice(len(xy), size=n, p=wo if which == 1 else wd)
                    if which == 2 and cell[0] == cell[1] and len(xy) > 1:
                        for i in np.flatnonzero(k == chosen[0]):  # not the trip's own origin
                            others = wd.copy()
                            others[k[i]] = 0.0
                            if others.sum() <= 0:
                                others = np.where(np.arange(len(xy)) != k[i], 1.0, 0.0)
                            k[i] = rng.choice(len(xy), p=others / others.sum())
                    chosen.append(k)
                    ends.append((xy[k, 0], xy[k, 1]))
                    continue
                if points_per_zone is None:
                    r = spreads.get(zone, spread_m) * np.sqrt(rng.random(n))
                    angle = 2.0 * np.pi * rng.random(n)
                else:  # one of the zone's fixed points (S265)
                    k = rng.integers(points_per_zone, size=n)
                    r = np.full(n, spreads.get(zone, spread_m) / 2 if points_per_zone > 1 else 0.0)
                    angle = 2.0 * np.pi * (k + 0.125) / points_per_zone
                lat_m = 110_574.0
                lon_m = 111_320.0 * math.cos(math.radians(at[1]))
                ends.append((at[0] + r * np.cos(angle) / lon_m, at[1] + r * np.sin(angle) / lat_m))
            for k in range(n):
                trips.append((
                    f"od{len(trips) + 1:07d}", 0,
                    float(ends[0][0][k]), float(ends[0][1][k]),
                    float(ends[1][0][k]), float(ends[1][1][k]),
                    int(departures[k]), cls, None, mode or None,
                ))  # fmt: skip
    return trips
