"""A fleet day without a run: shared-mobility services, a table of trips, one simulated day.

``service_bike_docked`` and ``service_taxi`` describe a service: its docks, or its vehicles and
chargers, its options and its name. ``fleet_day`` runs every service's day together on the
fleet engine, from a table of trips that use them and, optionally, the road times of an
earlier run (``road_times_read``), and returns what became of each trip and of each dock.

**A frozen day, an approximation:** the travellers' choices and the traffic are held fixed.
Each trip's service is given; a shared-bike trip takes the dock nearest its origin on foot and
returns at the dock nearest its destination on foot, whatever the docks hold; a taxi drives on
the given road times and adds no traffic. What a full run adds — travellers choosing among
modes and docks by what they expect, the services' vehicles in the traffic — is not here. The
result says so (``FleetDay.fidelity``).
"""

from __future__ import annotations

import time
from collections.abc import Iterable, Mapping, Sequence
from dataclasses import dataclass, field
from typing import Any

from openmobisim import _core
from openmobisim.parking import _csv_rows, _number, _text
from openmobisim.table import Columns

__all__ = [
    "FleetDay",
    "Service",
    "fleet_day",
    "road_times_read",
    "service_bike_docked",
    "service_taxi",
]


@dataclass(frozen=True)
class Service:
    """One shared-mobility service: its kind, name, parts and options (see the makers)."""

    kind: str
    name: str
    docks: Any = None
    vehicles: Any = None
    chargers: Any = None
    options: Mapping[str, float] = field(default_factory=dict)


def service_bike_docked(
    docks: Any, *, name: str = "bike_share", options: Mapping[str, float] | None = None
) -> Service:
    """A docked bike-sharing service: bikes counted per dock.

    Args:
        docks: Its docks (``dock_read_table`` or ``dock_read_gbfs``).
        name: Its name, unique among a day's services.
        options: Its options by name (``walk_max_s``, ``redirect_max``, ``redirect_max_m``,
            ``pickup_s``, ``return_s``, ``ride_factor``, ``start_share``, ``fail_s``, ``bin_s``,
            ``handling_s``, ``candidates``, ``snap_m``); a name not among them is refused with
            the list.
    """
    return Service("bike_docked", name, docks=docks, options=dict(options or {}))


def service_taxi(
    vehicles: Any,
    chargers: Any = None,
    *,
    name: str = "taxi",
    options: Mapping[str, float] | None = None,
) -> Service:
    """An electric taxi fleet: vehicles tracked one by one, with a battery.

    Args:
        vehicles: Its vehicles (``vehicle_read_table``).
        chargers: Its chargers (``charger_read_table``), or ``None``: its vehicles never charge.
        name: Its name, unique among a day's services.
        options: Its options by name (``battery_kwh``, ``kwh_per_km``, ``soc_start``,
            ``reserve_soc``, ``charge_below``, ``charge_to``, ``charger_kw``, ``max_wait_s``,
            ``board_s``, ``snap_m``, ``charger_reach_s``); a name not among them is refused with
            the list.
    """
    return Service("taxi", name, vehicles=vehicles, chargers=chargers, options=dict(options or {}))


def _seconds(value: object, what: str) -> float:
    """Seconds from midnight from a number or ``"HH:MM"`` / ``"HH:MM:SS"``."""
    if isinstance(value, str) and ":" in value:
        parts = [float(p) for p in value.split(":")]
        if len(parts) not in (2, 3):
            raise ValueError(f"{what}: {value!r} is not HH:MM or HH:MM:SS")
        return parts[0] * 3600 + parts[1] * 60 + (parts[2] if len(parts) == 3 else 0.0)
    try:
        return float(value)  # type: ignore[arg-type]
    except (TypeError, ValueError):
        raise ValueError(f"{what}: {value!r} is not a time") from None


@dataclass(frozen=True)
class RoadTimes:
    """Road times for a fleet day: per link index and time bin, the mean seconds to cross it."""

    bin_s: float
    rows: list[tuple[int, int, float]]
    network_fingerprint: str | None = None


def road_times_read(link_bins: str | Any) -> RoadTimes:
    """The road times of an earlier run, from its ``link_bins.parquet``.

    Each link's mean time in each bin is ``pcu_seconds / pcu``; a link or bin without a row is
    taken at free flow. The run's network must be the network the fleet day is given (its
    fingerprint is checked). These times are by the bin a vehicle *left* the link, which the
    fleet day reads as the bin it enters: an approximation.

    Args:
        link_bins: The path of a run's ``link_bins.parquet``.

    Raises:
        ImportError: Without pyarrow — install ``openmobisim[all]``.
    """
    import pyarrow.parquet as pq

    table = pq.read_table(str(link_bins), columns=["bin", "link_index", "pcu", "pcu_seconds"])
    meta = table.schema.metadata or {}
    bin_s = float(meta.get(b"openmobisim.bin_seconds", b"900"))
    fingerprint = meta.get(b"openmobisim.network_fingerprint")
    rows = [
        (int(link), int(b), float(s) / float(p))
        for b, link, p, s in zip(
            table["bin"].to_pylist(),
            table["link_index"].to_pylist(),
            table["pcu"].to_pylist(),
            table["pcu_seconds"].to_pylist(),
            strict=True,
        )
        if p and p > 0 and s and s > 0
    ]
    return RoadTimes(bin_s, rows, fingerprint.decode() if fingerprint else None)


@dataclass(frozen=True)
class FleetDay:
    """What one fleet day did (``fleet_day``)."""

    #: ``"frozen_day"``: the travellers' choices and the traffic were held fixed.
    fidelity: str
    #: The services' names, in the order given.
    services: list[str]
    #: Per trip given: what became of it (``trip_results()``).
    trips: Columns
    #: Per docked service, its docks bin by bin.
    dock_bins: dict[str, Columns]
    #: The trips planned and left out, by reason.
    plan: dict[str, int]
    #: The day's counts: events, conservation failures (always 0 unless there is a bug).
    counts: dict[str, float]
    #: The seconds the day took to set up and to simulate.
    seconds: dict[str, float]

    def trip_results(self) -> Columns:
        """What became of each trip given, in the order given.

        Columns: ``trip_id, service, status, requested_s, picked_s, arrived_s, dock_from,
        dock_to, extra_walk_s, redirections, overflow`` (statuses: ``served``, ``failed``,
        ``truncated``, ``not_requested``, ``not_planned``).
        """
        return self.trips

    def __repr__(self) -> str:
        """The trips, how many were served, and the day's seconds."""
        served = sum(1 for s in self.trips.get("status", []) if s == "served")
        return (
            f"FleetDay({self.fidelity}: {self.trips.row_count} trips, {served} served; "
            f"{self.seconds.get('day_s', float('nan')):.2f} s)"
        )


def _trip_rows(trips: str | Iterable[Mapping[str, Any]], names: Sequence[str]) -> list[tuple]:
    rows = _csv_rows(trips) if isinstance(trips, str) else list(trips)
    out = []
    for k, raw in enumerate(rows):
        row = {str(key).strip().lower(): v for key, v in raw.items()}
        trip = _text(row, "trip_id") or str(k)
        service = _text(row, "service")
        if service is None and len(names) == 1:
            service = names[0]
        if service not in names:
            raise ValueError(f"trip {trip!r}: service {service!r} is not one of {list(names)}")
        coords = [
            _number(row, c, trip)
            for c in ("origin_lon", "origin_lat", "destination_lon", "destination_lat")
        ]
        if any(c is None for c in coords):
            raise ValueError(
                f"trip {trip!r}: origin_lon, origin_lat, destination_lon and destination_lat "
                "are required"
            )
        if row.get("departure") is None:
            raise ValueError(f"trip {trip!r}: departure is required (seconds or HH:MM)")
        weight = _number(row, "weight", trip)
        out.append(
            (
                trip,
                names.index(service),
                1 if weight is None else int(weight),
                _seconds(row["departure"], f"trip {trip!r}"),
                *coords,
            )
        )
    return out


def fleet_day(
    network: Any,
    services: Sequence[Service],
    trips: str | Iterable[Mapping[str, Any]],
    *,
    window: tuple[object, object] = ("07:30", "09:30"),
    road_times: RoadTimes | None = None,
    seed: int = 0,
) -> FleetDay:
    """Run every service's day together on the fleet engine, from a table of trips.

    Args:
        network: The road network (its walk and bike layers are made from it).
        services: The services (``service_bike_docked``, ``service_taxi``), names unique.
        trips: One row per trip: ``origin_lon, origin_lat, destination_lon, destination_lat``,
            ``departure`` (seconds from midnight, or ``"HH:MM"``), and optionally ``trip_id``,
            ``service`` (a service's name; required with more than one service) and
            ``weight`` (the people it stands for, 1 by default). A CSV/TSV path or text, or rows
            as dicts.
        window: The day's start and end (seconds or ``"HH:MM"``); a trip asking before it opens
            asks at its start, one after it ends is never asked.
        road_times: An earlier run's road times (``road_times_read``); ``None``: free flow.
        seed: The day's seed.

    Raises:
        ValueError: For a service, trip or option that is not valid, or road times of another
            network.
    """
    names = [s.name for s in services]
    if len(set(names)) != len(names):
        raise ValueError(f"service names repeat: {names}")
    started = time.perf_counter()
    rows = _trip_rows(trips, names)
    start, end = (_seconds(w, "window") for w in window)
    if not start < end:
        raise ValueError(f"the window must end after it starts, got {window}")
    roads = None
    if road_times is not None:
        # The binding checks the fingerprint against the network's.
        roads = (road_times.bin_s, road_times.rows, road_times.network_fingerprint)
    parts = [
        (
            s.kind,
            s.name,
            s.docks,
            s.vehicles,
            s.chargers,
            {k: float(v) for k, v in s.options.items()},
        )
        for s in services
    ]
    out = _core._fleet_day(network, parts, rows, roads, (start, end), int(seed))
    trips_out = Columns(out["trips"])
    return FleetDay(
        fidelity="frozen_day",
        services=names,
        trips=trips_out,
        dock_bins={name: Columns(cols) for name, cols in out["dock_bins"].items()},
        plan=dict(out["plan"]),
        counts=dict(out["counts"]),
        seconds={**out["seconds"], "total_s": time.perf_counter() - started},
    )
