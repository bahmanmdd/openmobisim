"""Demand before a run: reading it from files, turning an OD matrix into trips, sampling it.

**Files** (S210, the standard demand files): ``zones.csv`` (``zone_id`` and a point,
``x_coord``/``y_coord`` or ``lon``/``lat``, WGS84), ``od.csv`` (``origin_zone``,
``destination_zone``, ``trips``; optionally ``start_s``, ``end_s``, ``user_class``, ``mode``) and
``trips.csv`` (the columns of ``trips.parquet``). ``demand_read_zones``, ``demand_read_od`` and
``demand_read_trips`` read them; ``demand_from_od`` turns an OD matrix into the trips a
``Scenario`` runs.

**Traveller classes** (S231): a class says which modes its travellers may use, what they own
and its own coefficients in the choice model (``beta_*``, its mode constants among them).
``demand_read_classes`` reads a class table (``classes.csv``); ``demand_assign_classes`` gives
every traveller a class, drawn by the classes' shares; ``Scenario(classes=...)`` runs them.

``demand_sample`` is the traveller-weight dial (S209): simulate one traveller for every
``weight`` people, the rest of the table unchanged. Fewer travellers cost proportionally
less to choose for, route and load, at the price of sampling noise, which replications
(different ``seed`` values) measure.
"""

from __future__ import annotations

import csv
import hashlib
import math
from collections import defaultdict
from collections.abc import Iterable, Mapping
from pathlib import Path
from typing import Any

import numpy as np

__all__ = [
    "demand_assign_classes",
    "demand_from_od",
    "demand_read_classes",
    "demand_read_od",
    "demand_read_trips",
    "demand_read_zones",
    "demand_sample",
    "demand_write_trips",
]

# Columns of a trips row (``Scenario.from_parts``' demand schema).
_TRAVELLER, _CLASS, _WEIGHT, _MODE = 0, 7, 8, 9


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

#: What a class may say besides its ``beta_*`` coefficients (S231).
CLASS_KEYS = ("share", "modes", "owns_car", "owns_bike", "has_transit_pass")
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
    # A class as ``demand_read_classes`` returns it: its coefficients under ``betas``.
    if isinstance(given.get("betas"), Mapping):
        given = {**{k: v for k, v in given.items() if k != "betas"}, **given["betas"]}
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
    # Unsaid ownership follows the modes: a class that may drive owns a car (S231).
    implied = {
        "owns_car": bool(modes) and any(m in ("car", "car_transit") for m in modes),
        "owns_bike": bool(modes) and any(m in ("bike", "bike_transit") for m in modes),
        "has_transit_pass": bool(modes) and any("transit" in m for m in modes),
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
    return entry


def _class_table(classes: Mapping[str, object]) -> dict[str, dict[str, Any]]:
    """Every class in the one shape a run reads, in the order given."""
    if not isinstance(classes, Mapping):
        raise ValueError(f"classes must be a dict of classes by name, got {type(classes).__name__}")
    return {str(name): _class_entry(str(name), spec) for name, spec in classes.items()}


def demand_read_classes(path: str | Path) -> dict[str, dict[str, Any]]:
    """Read a class table (``classes.csv``): one row per traveller class (S231).

    Columns: ``class`` (its name, as the trips' ``user_class``); optionally ``share`` (the share
    of the people in the class, for ``demand_assign_classes``), ``modes`` (the modes its
    travellers may use, separated by ``;``: ``car;bike;walk``; empty: every mode the run
    offers), ``owns_car``, ``owns_bike`` and ``has_transit_pass`` (``1`` or ``0``; empty: as the
    modes imply, a car for a class that may drive or park and ride, a bike for one that may
    cycle, a pass for one that may take transit) and any ``beta_*`` coefficient of the choice
    model (``beta_mode_bike``, ``beta_time_min`` …; empty: the model's own). Every value is
    what the user gives, not a calibration.

    Returns:
        ``{class: {"share", "modes", "owns_car", "owns_bike", "has_transit_pass", "betas"}}`` in
        the file's order, the shape ``Scenario(classes=...)`` and ``demand_assign_classes`` take.

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
    """Give every traveller a class, drawn by the classes' shares (S231).

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
) -> list[tuple]:
    """Turn an OD matrix into the trips a ``Scenario`` runs (S97's deterministic integerisation).

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
       random direction (uniform over the disc), so a zone's trips do not all start at one node.

    The draws come from one generator seeded by ``seed``, consumed in the sorted order of the
    cells, so the same inputs and seed give the same trips on any machine.

    Args:
        od: ``od.csv``'s path, or its rows (``demand_read_od``'s shape).
        zones: ``zones.csv``'s path, or ``{zone_id: (lon, lat)}``.
        start_s: The departure window's start for rows without one, seconds after the day's start.
        end_s: Its end.
        profile: ``"uniform"`` or ``"am_peak"``.
        spread_m: How far from its zone's point a trip may start or end, in metres.
        user_class: The class of rows without one.
        seed: Which draws.

    Returns:
        Trips rows (``traveller_id, trip_seq, origin_lon, origin_lat, destination_lon,
        destination_lat, departure_time_s, user_class, weight, mode``), weight ``None``.

    Raises:
        ValueError: For an unknown profile, a window that is empty, a zone the matrix names but
            ``zones`` does not have, or a negative spread.
    """
    if profile not in _PROFILES:
        raise ValueError(f"profile must be one of {_PROFILES}, got {profile!r}")
    if spread_m < 0:
        raise ValueError(f"spread_m must not be negative, got {spread_m}")
    rows = demand_read_od(od) if isinstance(od, (str, Path)) else list(od)
    points = demand_read_zones(zones) if isinstance(zones, (str, Path)) else dict(zones)
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
            for at in (o, d):
                r = spread_m * np.sqrt(rng.random(n))
                angle = 2.0 * np.pi * rng.random(n)
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
