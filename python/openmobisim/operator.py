"""Operators: policies that steer shared-mobility services during a fleet day.

A **policy** is called every ``step_s`` seconds of the day (the first at the window's start)
with a read-only **view** of the services, and returns **decisions**, which the fleet engine
checks, refuses with a reason when they cannot be carried out (the day goes on), or carries
out. Every decision is recorded (``FleetDay.decisions()``).

A policy is one of:

- a **built-in** one, run in Rust: ``policy_noop``, ``policy_reset``, ``policy_threshold``,
  ``policy_charge_rule``, or their names ``"noop"``, ``"reset"``, ``"threshold"``,
  ``"charge_rule"``;
- **any Python object** with ``step(view)`` returning a list of decisions; optionally
  ``step_s`` (900 by default), ``describe()`` (its name and parameters: a dict ``{"name":
  ..., "parameters": {...}}`` or a name; ``"python:<class name>"`` without it) and
  ``day_start(day)`` (called once when the day begins, with ``{"window", "seed",
  "services"}``);
- a **name registered** with ``policy_register``.

**The view** (``view.t``, ``view.step``, ``view.rng_seed``, ``view.window``,
``view.services``) holds per service name a dict of arrays, one value per dock or vehicle:
a docked service's ``dock_id``, ``node`` (its road node id), ``zone``, ``bikes``, ``slots``,
``incoming`` (bikes in vans on their way to the dock), ``lon``, ``lat``, and since the last
step ``pickups``, ``returns``, ``failed``, ``redirected_in``, ``redirected_out`` (people), and
``moving`` (bikes in vans now) and ``riding``; a taxi fleet's ``vehicle_id``, ``state`` (names:
``idle``, ``to_pickup``, ``occupied``, ``to_charger``, ``queued_at_charger``, ``charging``,
``relocating``), ``node`` (road node ids: where it is, or where it is driving to), ``until_s``
(the second it gets there when driving, or ends its charge when charging; ``nan`` when idle or
queued for a plug), ``zone``, ``soc`` (charge as a share of the battery),
``nearest_charger`` (by free-flow time, within the fleet's ``charger_reach_s``, or ``None``),
the waiting requests (``waiting_id``, ``waiting_node``, ``waiting_zone``, ``waiting_since_s``)
and the chargers (``charger_id``, ``charger_node``, ``charger_plugs``, ``charger_in_use``,
``charger_queued``).
The arrays are copies, and the view refuses to be read after its step: a policy keeps copies
of what it needs. For its own random draws, a policy uses
``numpy.random.default_rng(view.rng_seed)``: the same day and seed then give the same day.

**Zones** (``view.zones``): a square grid of ``grid_m`` metres (1 000: an assumption, not
calibrated) over the road network's nodes, from the south-west corner of their extent; a
zone is a cell holding at least one road node (``id`` from 0, by row south to north then column
west to east, so the same network always gives the same zones; ``lon`` and ``lat`` of its
centre). Each service's ``zone`` gives the zone of each dock or vehicle (-1: none).

**Queries**, computed in Rust when asked (sources in parallel, the same answer at any number of
threads) and cached within the step; like the rest of the view, they refuse after the step:

- ``view.travel_times(service, from_nodes, to_nodes, max_s=inf)``: seconds from each road node
  of ``from_nodes`` to each of ``to_nodes``, leaving at the step's second on the day's road
  times (they change by the time of day; free flow without ``road_times``), a numpy matrix
  ``[len(from_nodes), len(to_nodes)]``, ``inf`` where no road gets there within ``max_s``
  seconds (nor within 4 hours). ``service`` (a name, or ``None``) changes nothing: every
  service's vehicles (a docked service's vans, a taxi fleet's cars) drive the roads.
- ``view.expected_legs(service, bin_s=900)``: the people expected to ask for ``service`` per
  time bin of ``bin_s`` seconds from the window's start, a numpy matrix ``[rows, bins]``: per
  dock for a docked service (pickups at the dock each leg planned; rows as ``dock_id``), per
  zone for a taxi fleet (requests by their origin's zone; rows as ``view.zones["id"]``). They are
  counted from the day's legs before the variation a seed draws in a ``fleet_environment``
  (``legs_keep``, ``departure_jitter_s``): the expectation over the days the seeds draw, not
  one day's own variation. **In a day without variation (``fleet_day``, or a
  ``fleet_environment`` with ``legs_keep=1`` and no jitter) these are the day's own legs:
  exactly what the day will ask for, so a policy reading them knows the day's future.**
- ``view.nearest(service, kind, node, k)``: the ``k`` docks (``kind="docks"``, by their road
  nodes) or vehicles (``kind="vehicles"``, by the node each is at or driving to) of ``service``
  nearest road node ``node`` by free-flow time: a list of ``(id, seconds)``, nearest first,
  ties by their order in the view; those no road reaches are left out.

Road nodes are named by their ids, as the view gives them (``node``, ``waiting_node``).

**Decisions** name services, docks, vehicles, chargers and road nodes by their ids: the
dataclasses below, or tuples ``("move", service, from_dock, to_dock, n)``, ``("target",
service, dock, level)``, ``("relocate", service, vehicle, node)``, ``("charge", service,
vehicle, charger, to_soc)``. An id that does not exist stops the day with an error; a decision
that cannot be carried out is refused with its reason. A policy's exception stops the day and
is raised by ``fleet_day``.
"""

from __future__ import annotations

import math
from collections.abc import Callable, Mapping
from dataclasses import dataclass, field
from typing import Any

__all__ = [
    "Charge",
    "Move",
    "PolicyBuiltin",
    "Relocate",
    "Target",
    "policy_charge_rule",
    "policy_noop",
    "policy_register",
    "policy_reset",
    "policy_threshold",
]


@dataclass(frozen=True)
class Move:
    """Move up to ``n`` bikes of a docked service from one dock to another by van.

    As many leave as the first dock holds, up to ``n`` and the service's ``van_bikes``; the van
    drives the roads' time between the docks plus ``handling_s`` a bike.
    """

    service: str
    from_dock: str
    to_dock: str
    n: int


@dataclass(frozen=True)
class Target:
    """The bikes a dock should hold: the engine's solver moves bikes to reach it.

    At the step's end, every dock given a target and holding more gives to those holding fewer,
    the dock furthest below its target first, from the nearest docks as the crow flies, at most
    the service's ``van_bikes`` a move. A dock without a target neither gives nor takes.
    """

    service: str
    dock: str
    level: int


@dataclass(frozen=True)
class Relocate:
    """Send an idle taxi, empty, to a road node, where it waits for a request."""

    service: str
    vehicle: str
    node: str


@dataclass(frozen=True)
class Charge:
    """Send an idle taxi to a charger, to charge to ``to_soc`` of its battery."""

    service: str
    vehicle: str
    charger: str
    to_soc: float


@dataclass(frozen=True)
class PolicyBuiltin:
    """A built-in policy (run in Rust): its name and parameters (``policy_threshold`` etc.)."""

    name: str
    parameters: Mapping[str, float] = field(default_factory=dict)


def policy_noop(*, step_s: float = 900.0) -> PolicyBuiltin:
    """The policy that decides nothing: the day without an operator, with steps.

    Args:
        step_s: The seconds between its steps.
    """
    return PolicyBuiltin("noop", {"step_s": float(step_s)})


def policy_reset(from_s: float = 0.0, *, step_s: float = 900.0) -> PolicyBuiltin:
    """Every dock back toward its count at the start of the day, at each step from ``from_s``.

    At each step at or after ``from_s``, every dock whose bikes differ from its start-of-day
    count is given that count as its target (``Target``); the engine's solver moves bikes from
    the docks above their count to those below it, nearest first. Bikes still in a van at the
    next step are not counted at their destination, so a dock may be served twice when vans
    take longer than ``step_s``.

    Args:
        from_s: The second of the day (from midnight) it starts acting at; 0: from the start.
        step_s: The seconds between its steps.
    """
    return PolicyBuiltin("reset", {"step_s": float(step_s), "from_s": float(from_s)})


def policy_threshold(
    low: float = 0.2, high: float = 0.8, *, step_s: float = 900.0
) -> PolicyBuiltin:
    """Docks running empty topped up from docks running full, every step.

    A dock holding fewer bikes than ``low`` of its slots is given the target ``slots × (low +
    high) / 2`` rounded down; one holding more than ``high`` of its slots, that middle rounded
    up; the engine's solver moves bikes from the second kind to the first, nearest first. The
    defaults are assumptions, not calibrated.

    Args:
        low: Below this share of its slots, a dock is topped up.
        high: Above this share, a dock gives bikes away (``0 <= low < high <= 1``, checked by
            ``fleet_day``).
        step_s: The seconds between its steps.
    """
    return PolicyBuiltin(
        "threshold", {"low": float(low), "high": float(high), "step_s": float(step_s)}
    )


def policy_charge_rule(
    charge_below: float = 0.2, charge_to: float = 0.8, *, step_s: float = 900.0
) -> PolicyBuiltin:
    """Idle taxis low on charge sent to charge, every step.

    An idle vehicle charged below ``charge_below`` of its battery is sent to the charger
    nearest it by free-flow time (within the fleet's ``charger_reach_s``), to charge to
    ``charge_to``. It acts besides the fleet's own charging rule (its options of the same
    names, applied when a vehicle drops a passenger). The defaults are assumptions, not
    calibrated.

    Args:
        charge_below: Below this share of its battery, an idle vehicle is sent to charge.
        charge_to: The share it charges to (``0 <= charge_below < charge_to <= 1``, checked
            by ``fleet_day``).
        step_s: The seconds between its steps.
    """
    return PolicyBuiltin(
        "charge_rule",
        {
            "charge_below": float(charge_below),
            "charge_to": float(charge_to),
            "step_s": float(step_s),
        },
    )


_BUILTINS = ("noop", "reset", "threshold", "charge_rule")

#: The policies registered by name: name -> factory(options) -> policy.
_REGISTERED: dict[str, Callable[[dict[str, float]], Any]] = {}


def policy_register(name: str, factory: Callable[[dict[str, float]], Any]) -> None:
    """Make a policy usable by name, like the built-in ones.

    After ``policy_register("my_rule", MyRule)``, ``fleet_day(..., policy="my_rule",
    policy_options={...})`` builds the policy as ``factory(policy_options)`` (a dict, empty
    when none are given). A name is registered once per session: registering it again
    replaces the factory.

    Args:
        name: Letters, digits and underscores, not a built-in policy's name.
        factory: Called with the options dict; returns a policy (an object with
            ``step(view)``, or a built-in one).

    Raises:
        ValueError: If the name is a built-in policy's or not a plain identifier, or the
            factory is not callable.
    """
    if not name.isidentifier():
        raise ValueError(f"a policy's name is letters, digits and underscores, got {name!r}")
    if name in _BUILTINS:
        raise ValueError(f"{name!r} is a built-in policy; register yours under another name")
    if not callable(factory):
        raise ValueError(f"the factory must be callable (options -> policy), got {factory!r}")
    _REGISTERED[name] = factory


def _decision(item: Any) -> tuple:
    """A decision as the binding reads it: a tuple naming things by id."""
    if isinstance(item, Move):
        return ("move", item.service, item.from_dock, item.to_dock, int(item.n))
    if isinstance(item, Target):
        return ("target", item.service, item.dock, int(item.level))
    if isinstance(item, Relocate):
        return ("relocate", item.service, item.vehicle, item.node)
    if isinstance(item, Charge):
        return ("charge", item.service, item.vehicle, item.charger, float(item.to_soc))
    if isinstance(item, tuple):
        return item
    raise ValueError(f"a decision is a Move, Target, Relocate or Charge, or a tuple, not {item!r}")


class _Adapter:
    """The user's policy as the binding calls it: decisions turned into tuples."""

    def __init__(self, policy: Any) -> None:
        self.policy = policy

    def step(self, view: Any) -> list[tuple]:
        out = self.policy.step(view)
        return [] if out is None else [_decision(d) for d in out]

    def day_start(self, day: dict[str, Any]) -> None:
        start = getattr(self.policy, "day_start", None)
        if callable(start):
            start(day)


def _description(policy: Any) -> tuple[str, dict[str, float]]:
    describe = getattr(policy, "describe", None)
    if not callable(describe):
        return f"python:{type(policy).__name__}", {}
    d = describe()
    if isinstance(d, str):
        return d, {}
    if isinstance(d, Mapping) and "name" in d:
        return str(d["name"]), {str(k): float(v) for k, v in dict(d.get("parameters", {})).items()}
    raise ValueError(
        f"describe() returns a name or {{'name': ..., 'parameters': {{...}}}}, not {d!r}"
    )


def _policy_for_binding(
    policy: Any, options: Mapping[str, float] | None
) -> tuple[str, Any, str, dict[str, float], float] | None:
    """The policy as the binding takes it: ``(kind, object, name, parameters, step_s)``."""
    if policy is None:
        if options:
            raise ValueError("policy_options given without a policy")
        return None
    if isinstance(policy, str):
        if policy in _BUILTINS:
            policy = PolicyBuiltin(policy, dict(options or {}))
        elif policy in _REGISTERED:
            policy = _REGISTERED[policy](dict(options or {}))
        else:
            raise ValueError(
                f"unknown policy {policy!r}; the policies are {[*_BUILTINS, *_REGISTERED]}"
            )
    elif options:
        raise ValueError("policy_options are for a policy given by name")
    if isinstance(policy, PolicyBuiltin):
        params = {str(k): float(v) for k, v in policy.parameters.items()}
        return ("builtin", None, policy.name, params, math.nan)
    if not callable(getattr(policy, "step", None)):
        raise ValueError(f"a policy has a step(view) method; {policy!r} has none")
    step_s = float(getattr(policy, "step_s", 900.0))
    name, params = _description(policy)
    return ("python", _Adapter(policy), name, params, step_s)
