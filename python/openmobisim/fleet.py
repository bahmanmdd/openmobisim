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

**An operator** (``policy=``, see ``openmobisim.operator``) may steer the services during the
day: move bikes between docks, set docks' target levels, relocate or charge taxis. Its
decisions, accepted or refused with the reason, are in ``FleetDay.decisions()``.
"""

from __future__ import annotations

import time
from collections.abc import Iterable, Mapping, Sequence
from dataclasses import dataclass, field
from typing import Any

from openmobisim import _core
from openmobisim.operator import _policy_for_binding
from openmobisim.parking import _csv_rows, _number, _text
from openmobisim.table import Columns

__all__ = [
    "FleetDay",
    "Service",
    "REWARD_WEIGHTS",
    "FleetEnvironment",
    "fleet_day",
    "fleet_environment",
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
            ``handling_s``, ``van_bikes``, ``candidates``, ``snap_m``); a name not among them is
            refused with the list.
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
    #: Every decision the policy took, in order (``decisions()``).
    decision_rows: Columns
    #: The policy's name and parameters (``{"name", "parameters"}``), or ``None`` without one.
    policy: dict[str, Any] | None

    def trip_results(self) -> Columns:
        """What became of each trip given, in the order given.

        Columns: ``trip_id, service, status, requested_s, picked_s, arrived_s, dock_from,
        dock_to, extra_walk_s, redirections, overflow`` (statuses: ``served``, ``failed``,
        ``truncated``, ``not_requested``, ``not_planned``).
        """
        return self.trips

    def decisions(self) -> Columns:
        """Every decision the policy took, in the order taken, accepted or refused.

        Columns: ``step`` (the policy step's index, 0 at the window's start), ``t_s`` (its
        second), ``service``, ``kind`` (``move``, ``target``, ``relocate``, ``charge``), the
        parameters by id (``from_dock``, ``to_dock``, ``n``, ``dock``, ``level``, ``vehicle``,
        ``node``, ``charger``, ``to_soc``; ``None`` or NaN where a kind has none),
        ``accepted``, ``reason`` (why it was refused, ``None`` if accepted), ``executed_s`` (the
        second it started), ``finished_s`` (the van's or vehicle's arrival, the charge's end;
        NaN if refused or not finished by the window's end), ``moved`` (a move's bikes in the
        van), ``metres`` (the drive's), ``from_target`` (a move the solver made for the
        step's targets).
        """
        return self.decision_rows

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


def _inputs(
    services: Sequence[Service],
    trips: str | Iterable[Mapping[str, Any]],
    window: tuple[object, object],
    road_times: RoadTimes | None,
) -> tuple[list[tuple], list[tuple], tuple | None, tuple[float, float]]:
    """A fleet day's inputs as the binding takes them, checked."""
    names = [s.name for s in services]
    if len(set(names)) != len(names):
        raise ValueError(f"service names repeat: {names}")
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
    return parts, rows, roads, (start, end)


#: The standard reward's weights (``fleet_environment``): per failed leg, per minute waited, per
#: extra minute walked, per van kilometre. Uncalibrated; named so a study sets its own.
REWARD_WEIGHTS: dict[str, float] = {"w_fail": 1.0, "w_wait": 0.1, "w_walk": 0.1, "w_km": 0.1}


def fleet_environment(
    network: Any,
    services: Sequence[Service],
    trips: str | Iterable[Mapping[str, Any]],
    *,
    window: tuple[object, object] = ("07:30", "09:30"),
    road_times: RoadTimes | None = None,
    step_s: float = 900.0,
    reward_weights: Mapping[str, float] | None = None,
    legs_keep: float = 1.0,
    departure_jitter_s: float = 0.0,
) -> Any:
    """A fleet day to step through: ``reset(seed)``, then ``step(decisions)`` until the day ends.

    The day is ``fleet_day``'s (the same services, trips, window and road times; a frozen day:
    the travellers' choices and the traffic are held fixed), with no policy of its own: the
    caller decides at each step. ``reset(seed)`` returns ``(view, info)`` at the window's start;
    ``step(decisions)`` applies the decisions (as a policy returns them: ``operator.Move``,
    ``Target``, ``Relocate``, ``Charge``), runs ``step_s`` seconds and returns ``(view, reward,
    terminated, truncated, info)``; at the window's end ``info["outcome"]`` holds the day's tables.
    ``fork()`` copies the day at its clock, to simulate ahead without touching it. A view cannot
    be read after the next step.

    The reward of a step is minus ``w_fail`` × the people of the legs that failed, ``w_wait`` ×
    the minutes they waited, ``w_walk`` × the extra minutes they walked, and ``w_km`` × the van
    kilometres of the moves carried out, each counted once, when its leg ends (``REWARD_WEIGHTS``,
    uncalibrated).

    Args:
        network: The road network.
        services: The services (``service_bike_docked``, ``service_taxi``).
        trips: The trips, as for ``fleet_day``.
        window: The day's start and end.
        road_times: An earlier run's road times, or ``None``: free flow.
        step_s: The seconds between steps.
        reward_weights: Weights by name in place of ``REWARD_WEIGHTS``'; a name not among them is
            refused with the list.
        legs_keep: The share of the trips' legs each day keeps, drawn by the day's seed (1: all).
        departure_jitter_s: How far each kept leg's request moves, drawn by the day's seed, in
            seconds either way (0: not at all). With these two, each seed of ``reset`` is another
            day; both thin and shift the same trips, they do not draw a new demand.

    Raises:
        ValueError: As ``fleet_day``.
    """
    weights = dict(REWARD_WEIGHTS)
    for k, v in (reward_weights or {}).items():
        if k not in weights:
            raise ValueError(f"unknown reward weight {k!r}; the weights are {sorted(weights)}")
        weights[k] = float(v)
    parts, rows, roads, span = _inputs(services, trips, window, road_times)
    return FleetEnvironment(
        _core._fleet_environment(
            network,
            parts,
            rows,
            roads,
            span,
            float(step_s),
            (weights["w_fail"], weights["w_wait"], weights["w_walk"], weights["w_km"]),
            float(legs_keep),
            float(departure_jitter_s),
        )
    )


class FleetEnvironment:
    """A fleet day to step through (``fleet_environment``)."""

    def __init__(self, inner: Any) -> None:
        """Wrap the engine's environment (made by ``fleet_environment``)."""
        self._inner = inner

    def reset(self, seed: int = 0) -> tuple[Any, dict[str, Any]]:
        """Start the day with ``seed``: ``(view, info)`` at the window's start."""
        return self._inner.reset(int(seed))

    def step(self, decisions: Sequence[Any] | None = None) -> tuple[Any, float, bool, bool, dict]:
        """Apply ``decisions`` at the clock and run one step.

        Returns ``(view, reward, terminated, truncated, info)``.
        """
        from openmobisim.operator import _decision

        return self._inner.step([_decision(d) for d in decisions or []])

    def fork(self) -> FleetEnvironment:
        """A copy of the day at its clock, to simulate ahead without touching this one."""
        return FleetEnvironment(self._inner.fork())

    @property
    def t(self) -> float:
        """The clock, in seconds from midnight."""
        return self._inner.t

    def plan(self) -> dict[str, int]:
        """The trips given, the legs planned and those left out, by reason."""
        return dict(self._inner.plan())


def fleet_day(
    network: Any,
    services: Sequence[Service],
    trips: str | Iterable[Mapping[str, Any]],
    *,
    window: tuple[object, object] = ("07:30", "09:30"),
    road_times: RoadTimes | None = None,
    seed: int = 0,
    policy: Any = None,
    policy_options: Mapping[str, float] | None = None,
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
        policy: An operator steering the services (``openmobisim.operator``): a policy object
            (``step(view)``), a built-in one (``policy_threshold(...)``), a name (built-in or
            registered with ``policy_register``), or ``None``: no operator.
        policy_options: A named policy's options (``{"low": 0.1}`` for ``"threshold"``).

    Raises:
        ValueError: For a service, trip, option or policy that is not valid, road times of
            another network, or a decision naming an id that does not exist.
        Exception: A Python policy's own exception, which stops the day.
    """
    started = time.perf_counter()
    parts, rows, roads, (start, end) = _inputs(services, trips, window, road_times)
    chosen = _policy_for_binding(policy, policy_options)
    out = _core._fleet_day(network, parts, rows, roads, (start, end), int(seed), chosen)
    trips_out = Columns(out["trips"])
    return FleetDay(
        fidelity="frozen_day",
        services=[s.name for s in services],
        trips=trips_out,
        dock_bins={name: Columns(cols) for name, cols in out["dock_bins"].items()},
        plan=dict(out["plan"]),
        counts=dict(out["counts"]),
        seconds={**out["seconds"], "total_s": time.perf_counter() - started},
        decision_rows=Columns(out["decisions"]),
        policy=out["policy"],
    )
