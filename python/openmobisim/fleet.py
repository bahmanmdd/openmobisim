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

import json
import math
import time
from collections.abc import Iterable, Mapping, Sequence
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

import numpy as np

from openmobisim import _core
from openmobisim.operator import _policy_for_binding
from openmobisim.parking import _csv_rows, _number, _text
from openmobisim.table import Columns

__all__ = [
    "FleetDay",
    "Service",
    "REWARD_WEIGHTS",
    "ZONE_GRID_M",
    "FleetEnvironment",
    "FleetVectorEnvironment",
    "TAXI_STATES",
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
            refused with the list. A fleet day does not read ``candidates`` or ``fail_s``.
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
    #: A day replayed from a run (``frozen=``): the frozen day's ``setup.json`` (the run's,
    #: network's and services' fingerprints, the window, the seed, ...); ``None`` for a day of
    #: trips.
    frozen: dict[str, Any] | None = None

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


def _frozen_read(
    folder: str | Path,
    services: Sequence[Service],
    trips: object,
    road_times: RoadTimes | None,
    window: tuple[object, object] | None,
) -> tuple[dict[str, Any], tuple[str, str, str], tuple[float, float]]:
    """A run's frozen day (``Run.freeze``): its ``setup.json``, the binding's argument, its window.

    Refuses trips or road times given beside it, a window other than the day's, and services
    other than the run's, naming what differs where ``setup.json`` shows it (the binding then
    checks the fingerprints).
    """
    from openmobisim.run import _FROZEN_DAY_FORMAT, _FROZEN_DAY_VERSION

    if trips is not None or road_times is not None:
        raise ValueError(
            "a frozen day brings its own legs and road times: give frozen= without trips or "
            "road_times"
        )
    folder = Path(folder)
    path = folder / "setup.json"
    if not path.is_file():
        raise FileNotFoundError(f"{folder} holds no setup.json: not a frozen day (Run.freeze)")
    setup = json.loads(path.read_text(encoding="utf-8"))
    if setup.get("format") != _FROZEN_DAY_FORMAT or setup.get("version") != _FROZEN_DAY_VERSION:
        raise ValueError(
            f"{path}: not a frozen day of this version (format {setup.get('format')!r}, version "
            f"{setup.get('version')!r}; this package reads version {_FROZEN_DAY_VERSION})"
        )
    theirs = setup["services"]
    names = [s.name for s in services]
    if names != [s["name"] for s in theirs]:
        raise ValueError(
            f"the frozen day was made with the services {[s['name'] for s in theirs]}, not {names}"
        )
    for given, made in zip(services, theirs, strict=True):
        if given.kind != made["kind"]:
            raise ValueError(f"{given.name}: a {made['kind']} service in the run, not {given.kind}")
        if given.docks is not None and "docks" in made and given.docks.count != made["docks"]:
            raise ValueError(
                f"{given.name}: {made['docks']} docks in the run, {given.docks.count} here"
            )
        options = made.get("options", {})
        differ = {
            k: (options[k], float(v))
            for k, v in given.options.items()
            if k in options and float(v) != options[k]
        }
        if differ:
            shown = ", ".join(f"{k} {a} in the run, {b} here" for k, (a, b) in differ.items())
            raise ValueError(f"{given.name}: options differ ({shown})")
    start, end = (float(x) for x in setup["window"])
    if window is not None:
        given = tuple(_seconds(w, "window") for w in window)
        if given != (start, end):
            raise ValueError(
                f"the frozen day's window is ({start}, {end}) s, not {given}: leave window out"
            )
    arg = (str(folder), str(setup["services_fingerprint"]), str(setup["network_fingerprint"]))
    return setup, arg, (start, end)


#: A fleet day's window when none is given: 07:30 to 09:30.
_DEFAULT_WINDOW: tuple[object, object] = ("07:30", "09:30")

#: The side of the square zones a policy's view groups places by, in metres: an assumption,
#: uncalibrated (``zone_grid_m`` of ``fleet_day`` and ``fleet_environment``).
ZONE_GRID_M: float = 1000.0

#: The standard reward's weights (``fleet_environment``): per failed leg, per minute waited, per
#: extra minute walked, per van kilometre. Uncalibrated; named so a study sets its own.
REWARD_WEIGHTS: dict[str, float] = {"w_fail": 1.0, "w_wait": 0.1, "w_walk": 0.1, "w_km": 0.1}


def fleet_environment(
    network: Any,
    services: Sequence[Service],
    trips: str | Iterable[Mapping[str, Any]] | None = None,
    *,
    frozen: str | Path | None = None,
    window: tuple[object, object] | None = None,
    road_times: RoadTimes | None = None,
    step_s: float = 900.0,
    reward_weights: Mapping[str, float] | None = None,
    legs_keep: float = 1.0,
    departure_jitter_s: float = 0.0,
    n: int = 1,
    zone_grid_m: float = ZONE_GRID_M,
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
    the minutes they waited, ``w_walk`` × the extra minutes they walked, and ``w_km`` × the
    kilometres the policy's decisions drove (vans moving bikes, taxis relocating or driving to
    charge), each counted once, when its leg ends (``REWARD_WEIGHTS``,
    uncalibrated). ``info["refused"]`` holds, per decision given at the step, why the engine
    refused it (``None``: accepted).

    With ``n`` above 1 the result is a ``FleetVectorEnvironment``: ``n`` days stepped together,
    in parallel, ``reset(seeds)`` taking one seed per day and ``step`` one list of decisions per
    day. Seed ``k`` gives the same day there as here.

    ``observation(view, spec)`` and ``action(vector, spec)`` turn views and decisions into arrays
    of fixed shapes, for learning methods (``spec`` is described with ``observation``).

    With ``frozen`` (a folder ``Run.freeze`` wrote), the day is a run's last loading: its legs as
    the run played them and its road times, in place of trips; ``frozen`` on the environment holds
    its ``setup.json``. ``legs_keep`` and ``departure_jitter_s`` vary it by seed as they vary a
    day of trips; with neither, every seed replays the run's day.

    Args:
        network: The road network.
        services: The services (``service_bike_docked``, ``service_taxi``).
        trips: The trips, as for ``fleet_day``; ``None`` with ``frozen``.
        frozen: A run's frozen day, as for ``fleet_day``.
        window: The day's start and end; ``None``: 07:30 to 09:30, or a frozen day's own.
        road_times: An earlier run's road times, or ``None``: free flow (a frozen day's own
            with ``frozen``).
        step_s: The seconds between steps.
        reward_weights: Weights by name in place of ``REWARD_WEIGHTS``'; a name not among them is
            refused with the list.
        legs_keep: The share of the trips' legs each day keeps, drawn by the day's seed (1: all).
        zone_grid_m: The side of the square zones a view groups places by, in metres
            (``ZONE_GRID_M``, 1 000: an assumption, uncalibrated).
        departure_jitter_s: How far each kept leg's request moves, drawn by the day's seed, in
            seconds either way (0: not at all). With these two, each seed of ``reset`` is another
            day; both thin and shift the same trips, they do not draw a new demand.
        n: How many days to step together (1: a ``FleetEnvironment``).

    Raises:
        ValueError: As ``fleet_day``.
    """
    weights = dict(REWARD_WEIGHTS)
    for k, v in (reward_weights or {}).items():
        if k not in weights:
            raise ValueError(f"unknown reward weight {k!r}; the weights are {sorted(weights)}")
        weights[k] = float(v)
    if int(n) < 1:
        raise ValueError(f"n must be 1 or more, got {n}")
    setup, from_run, frozen_span = None, None, None
    if frozen is not None:
        setup, from_run, frozen_span = _frozen_read(frozen, services, trips, road_times, window)
        trips = []
    elif trips is None:
        raise ValueError("give the trips, or a run's frozen day (frozen=)")
    parts, rows, roads, span = _inputs(services, trips, window or _DEFAULT_WINDOW, road_times)
    inner = _core._fleet_environment(
        network,
        parts,
        rows,
        roads,
        frozen_span or span,
        float(step_s),
        (weights["w_fail"], weights["w_wait"], weights["w_walk"], weights["w_km"]),
        float(legs_keep),
        float(departure_jitter_s),
        float(zone_grid_m),
        from_run,
    )
    if int(n) == 1:
        return FleetEnvironment(inner, network, frozen=setup)
    return FleetVectorEnvironment(inner.vector(int(n)), network, frozen=setup)


#: A taxi's state codes in ``observation``: its state's index here.
TAXI_STATES: tuple[str, ...] = (
    "idle",
    "to_pickup",
    "occupied",
    "to_charger",
    "queued_at_charger",
    "charging",
    "relocating",
)

_SPEC_KEYS = {"service", "flat", "reach_m"}


class _Encoding:
    """The fixed-shape observations and actions of a fleet environment (both kinds share them)."""

    _inner: Any
    _network: Any

    def _spec(self, spec: Mapping[str, Any]) -> tuple[str, dict[str, Any]]:
        unknown = set(spec) - _SPEC_KEYS
        if unknown:
            raise ValueError(
                f"unknown spec keys {sorted(unknown)}; the keys are {sorted(_SPEC_KEYS)}"
            )
        layout = self.layout()
        name = spec.get("service")
        if name not in layout:
            raise ValueError(f"spec['service'] must be one of {list(layout)}, got {name!r}")
        return name, layout[name]

    def layout(self) -> dict[str, dict[str, Any]]:
        """Per service: its ``kind``, and its docks' or vehicles' ids, in the views' order.

        A docked service has ``dock_id`` and ``slots``; a taxi fleet ``vehicle_id``.
        """
        if not hasattr(self, "_layout"):
            self._layout = {k: dict(v) for k, v in self._inner.layout().items()}
        return self._layout

    @property
    def window(self) -> tuple[float, float]:
        """The window's start and end, in seconds from midnight."""
        return tuple(self._inner.window)

    def observation(self, view: Any, spec: Mapping[str, Any]) -> Any:
        """A view as arrays of a fixed shape, for one service.

        A docked service gives ``{"docks": float32 [docks, 5], "time": float32 [3]}``: per dock
        (in ``layout()``'s order) ``bikes, slots, pickups, returns, failed``, the last three
        counted since the previous step (people); the time is ``t_frac`` (the share of the
        window gone), and the sine and cosine of the hour of the day (2π · t / 86 400 s). A taxi
        fleet gives ``{"vehicles": float32 [vehicles, 3], "time": ...}``: per vehicle its state's
        code (its index in ``TAXI_STATES``), its charge (share of the battery) and the requests
        waiting within ``reach_m`` in a straight line of its ``node`` in the view (where it is,
        or where it is heading).

        Args:
            view: A view from ``reset`` or ``step``.
            spec: A dict: ``service`` (the service's name; required), ``flat`` (``True``: one 1-D
                array, the rows then the time; ``False`` by default), ``reach_m`` (taxis: the
                metres within which a waiting request counts; 1000 by default). Any other key is
                refused with the list.
        """
        name, layout = self._spec(spec)
        v = view[name]
        start, end = self.window
        t = float(view.t)
        angle = 2.0 * math.pi * t / 86_400.0
        frac = (t - start) / (end - start)
        clock = np.array([frac, math.sin(angle), math.cos(angle)], dtype=np.float32)
        if layout["kind"] == "bike_docked":
            key = "docks"
            rows = np.stack(
                [np.asarray(v[c], dtype=np.float32) for c in _DOCK_COLUMNS], axis=1
            ).reshape(len(layout["dock_id"]), len(_DOCK_COLUMNS))
        else:
            key = "vehicles"
            rows = self._taxi_rows(v, float(spec.get("reach_m", 1000.0)))
        if spec.get("flat", False):
            return np.concatenate([rows.ravel(), clock])
        return {key: rows, "time": clock}

    def _taxi_rows(self, v: Mapping[str, Any], reach_m: float) -> np.ndarray:
        code = np.array([TAXI_STATES.index(s) for s in v["state"]], dtype=np.float32)
        soc = np.asarray(v["soc"], dtype=np.float32)
        here = self._lonlat(v["node"])
        waiting = self._lonlat(v["waiting_node"])
        near = np.zeros(len(code), dtype=np.float32)
        if len(waiting) and len(here):
            lat0 = np.radians(here[:, 1])[:, None]
            dx = np.radians(waiting[None, :, 0] - here[:, None, 0]) * np.cos(lat0)
            dy = np.radians(waiting[None, :, 1] - here[:, None, 1])
            metres = 6_371_000.0 * np.hypot(dx, dy)
            near = (metres <= reach_m).sum(axis=1).astype(np.float32)
        return np.stack([code, soc, near], axis=1).reshape(len(code), 3)

    def _lonlat(self, nodes: Sequence[str]) -> np.ndarray:
        if not hasattr(self, "_nodes"):
            self._nodes: dict[str, tuple[float, float]] = {}
        for n in nodes:
            if n not in self._nodes:
                self._nodes[n] = tuple(self._network.node_lonlat(n))
        return np.array([self._nodes[n] for n in nodes], dtype=np.float64).reshape(len(nodes), 2)

    def action(self, vector: Any, spec: Mapping[str, Any]) -> list[Any]:
        """A vector of target levels, one per dock of a docked service, as ``Target`` decisions.

        Entry ``i`` is the bikes dock ``i`` (in ``layout()``'s order) should hold, or -1: no
        target. Levels are not checked here: the engine refuses one above the dock's slots, with
        its reason (``info["refused"]``). A taxi fleet is steered with decisions
        (``Relocate``, ``Charge``), not by this function.

        Args:
            vector: The levels, integers, one per dock.
            spec: As for ``observation``.
        """
        from openmobisim.operator import Target

        name, layout = self._spec(spec)
        if layout["kind"] != "bike_docked":
            raise ValueError(
                f"{name!r} is a {layout['kind']} service: action() encodes docked services only; "
                "steer it with decisions (Relocate, Charge)"
            )
        levels = np.asarray(vector).ravel()
        docks = layout["dock_id"]
        if len(levels) != len(docks):
            raise ValueError(f"the vector has {len(levels)} levels, the service {len(docks)} docks")
        if not np.all(np.equal(np.mod(levels, 1), 0)):
            raise ValueError("the levels must be integers")
        return [Target(name, d, int(x)) for d, x in zip(docks, levels, strict=True) if x >= 0]

    def plan(self) -> dict[str, int]:
        """The trips given, the legs planned and those left out, by reason."""
        return dict(self._inner.plan())


_DOCK_COLUMNS = ("bikes", "slots", "pickups", "returns", "failed")


class FleetEnvironment(_Encoding):
    """A fleet day to step through (``fleet_environment``)."""

    def __init__(
        self, inner: Any, network: Any = None, *, frozen: dict[str, Any] | None = None
    ) -> None:
        """Wrap the engine's environment (made by ``fleet_environment``)."""
        self._inner = inner
        self._network = network
        #: A day replayed from a run (``frozen=``): the frozen day's ``setup.json``; ``None``
        #: for a day of trips.
        self.frozen = frozen

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
        return FleetEnvironment(self._inner.fork(), self._network, frozen=self.frozen)

    @property
    def t(self) -> float:
        """The clock, in seconds from midnight."""
        return self._inner.t


class FleetVectorEnvironment(_Encoding):
    """``n`` fleet days stepped together, in parallel (``fleet_environment(..., n=...)``).

    One call steps every day; the engines run in parallel on the machine's cores. Seed ``k``
    gives the same day here as in a single environment. ``observation`` and ``action`` encode
    one day's view or decisions, as for ``FleetEnvironment``.
    """

    def __init__(
        self, inner: Any, network: Any = None, *, frozen: dict[str, Any] | None = None
    ) -> None:
        """Wrap the engine's environments (made by ``fleet_environment``)."""
        self._inner = inner
        self._network = network
        #: As ``FleetEnvironment.frozen``.
        self.frozen = frozen

    def __len__(self) -> int:
        """How many days."""
        return len(self._inner)

    def reset(self, seeds: Sequence[int]) -> tuple[list[Any], list[dict[str, Any]]]:
        """Start each day with its seed (one per day): ``(views, infos)``."""
        return self._inner.reset([int(s) for s in seeds])

    def step(
        self, decisions: Sequence[Sequence[Any] | None]
    ) -> tuple[list[Any], np.ndarray, list[bool], list[bool], list[dict[str, Any]]]:
        """Apply one list of decisions per day and run each one step.

        Returns ``(views, rewards, terminated, truncated, infos)``: lists of one per day,
        ``rewards`` an array.

        Raises:
            ValueError: Unless there is one list per day.
        """
        from openmobisim.operator import _decision

        return self._inner.step([[_decision(d) for d in ds or []] for ds in decisions])

    @property
    def t(self) -> list[float]:
        """Each day's clock, in seconds from midnight."""
        return list(self._inner.t)


def fleet_day(
    network: Any,
    services: Sequence[Service],
    trips: str | Iterable[Mapping[str, Any]] | None = None,
    *,
    frozen: str | Path | None = None,
    window: tuple[object, object] | None = None,
    road_times: RoadTimes | None = None,
    seed: int | None = None,
    policy: Any = None,
    policy_options: Mapping[str, float] | None = None,
    zone_grid_m: float = ZONE_GRID_M,
) -> FleetDay:
    """Run every service's day together on the fleet engine, from a table of trips.

    **Or from a run's frozen day** (``frozen``, a folder ``Run.freeze`` wrote): the legs of the
    run's last loading as the run played them (the docks the travellers chose, the second each
    asks for a bike, its ride and walk), on that loading's road times, over its window. With no
    policy, the day replays the run's: every leg's status, seconds and docks, and the docks' bins,
    are the run's own. The services must be the run's (the same names, docks, bikes and options)
    and the network too: either differing is refused, saying what differs. Each trip of the
    result is a leg: its ``trip_id`` the trip's index in the run (``Run.trip_results()``'s
    ``trip``); ``FleetDay.frozen`` holds the day's ``setup.json``.

    Args:
        network: The road network (its walk and bike layers are made from it).
        services: The services (``service_bike_docked``, ``service_taxi``), names unique.
        trips: One row per trip: ``origin_lon, origin_lat, destination_lon, destination_lat``,
            ``departure`` (seconds from midnight, or ``"HH:MM"``), and optionally ``trip_id``,
            ``service`` (a service's name; required with more than one service) and
            ``weight`` (the people it stands for, 1 by default). A CSV/TSV path or text, or rows
            as dicts. ``None`` with ``frozen``.
        frozen: A folder ``Run.freeze`` wrote, in place of ``trips`` and ``road_times``.
        window: The day's start and end (seconds or ``"HH:MM"``); a trip asking before it opens
            asks at its start, one after it ends is never asked. ``None``: 07:30 to 09:30, or a
            frozen day's own (another window is refused with ``frozen``).
        road_times: An earlier run's road times (``road_times_read``); ``None``: free flow.
        seed: The day's seed (a policy's draws); ``None``: 0, or a frozen day's (the run's master
            seed).
        policy: An operator steering the services (``openmobisim.operator``): a policy object
            (``step(view)``), a built-in one (``policy_threshold(...)``), a name (built-in or
            registered with ``policy_register``), or ``None``: no operator.
        policy_options: A named policy's options (``{"low": 0.1}`` for ``"threshold"``).
        zone_grid_m: The side of the square zones a view groups places by, in metres
            (``ZONE_GRID_M``, 1 000: an assumption, uncalibrated).

    Raises:
        ValueError: For a service, trip, option or policy that is not valid, road times of
            another network, or a decision naming an id that does not exist; with ``frozen``,
            for another network, other services, another window, or trips or road times given
            too.
        FileNotFoundError: For a ``frozen`` folder without ``setup.json``.
        Exception: A Python policy's own exception, which stops the day.
    """
    started = time.perf_counter()
    setup, from_run = None, None
    if frozen is not None:
        setup, from_run, span = _frozen_read(frozen, services, trips, road_times, window)
        trips, window = [], span
        seed = int(setup["seed"]) if seed is None else seed
    elif trips is None:
        raise ValueError("give the trips, or a run's frozen day (frozen=)")
    parts, rows, roads, (start, end) = _inputs(
        services, trips, window or _DEFAULT_WINDOW, road_times
    )
    chosen = _policy_for_binding(policy, policy_options)
    out = _core._fleet_day(
        network,
        parts,
        rows,
        roads,
        (start, end),
        int(seed or 0),
        chosen,
        float(zone_grid_m),
        from_run,
    )
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
        frozen=setup,
    )
