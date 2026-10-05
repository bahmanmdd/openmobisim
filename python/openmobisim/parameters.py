"""Every parameter in one table (S230, roadmap I-ae): the place to start a calibration.

Every number openmobisim assumes has a name, an uncalibrated default and an argument that
overrides it by name (S202). ``parameters()`` lists them all, at the defaults a ``Scenario`` uses.
"""

from __future__ import annotations

from typing import Any

from openmobisim import _core
from openmobisim.choice import ROUTE_ATTRIBUTES
from openmobisim.scenario import (
    FREE_FLOW_INCREMENTS,
    GAP_TOLERANCE,
    ROUTE_GROWTH_TOLERANCE,
)

__all__ = ["parameters"]

#: Where a ``Scenario`` departs from the core's default, and why (S223, S229).
_SCENARIO_DEFAULTS: dict[tuple[str, str], tuple[float, str]] = {
    ("equilibration.msa", "gap_tolerance"): (
        GAP_TOLERANCE,
        "Scenario's default: stop when converged",
    ),
    ("equilibration.msa", "route_growth_tolerance"): (
        ROUTE_GROWTH_TOLERANCE,
        "Scenario's default: the gap alone decides",
    ),
    ("equilibration.msa", "increments"): (
        FREE_FLOW_INCREMENTS,
        "Scenario's default at flow_level 2-4 (1 at 0)",
    ),
    ("equilibration.msa", "warmup"): (1, "Scenario's default at flow_level 2-4"),
    ("equilibration.free_flow", "increments"): (
        FREE_FLOW_INCREMENTS,
        "Scenario's default at flow_level 2-4 (1 at 0)",
    ),
    ("equilibration.free_flow", "warmup"): (1, "Scenario's default at flow_level 2-4"),
    ("loading", "reroute"): (0, "Scenario's default at flow_level 2 (1 at 3 and 4)"),
}

#: Scenario arguments that are numbers, with their defaults.
_SCENARIO = [
    ("flow_level", 2.0, "level"),
    ("flow_step_s", 300.0, "s"),
    ("window_hours", 24.0, "h"),
    ("default_weight", 1.0, "travellers"),
    ("master_seed", 0.0, "—"),
    ("choice_detour_limit", 0.5, "share"),
]

_COUNTS = ("iterations", "max_paths", "max_attempts", "max_routes", "searches", "candidates",
           "lanes", "increments", "max_rides", "reroute_max", "draws", "gap_sample",
           "itinerary_gap_sample", "warmup", "seed")  # fmt: skip
_FLAGS = ("priority", "reroute", "biased")


def _unit(name: str) -> str:
    """The unit, read from the name's suffix (the names carry their units, S202)."""
    leaf = name.split(".")[-1]
    suffixes = [
        ("_veh_h_lane", "veh/h per lane"),
        ("_veh_km_lane", "veh/km per lane"),
        ("_km_h", "km/h"),
        ("_min", "min"),
        ("_km", "km"),
        ("_pcu", "PCU"),
        ("_s", "s"),
        ("_m", "m"),
    ]
    if leaf.startswith("beta_"):
        attribute = leaf[len("beta_") :]
        for suffix, unit in suffixes:
            if attribute.endswith(suffix):
                return f"per {unit}"
        return "per unit of the attribute"
    if leaf in _FLAGS:
        return "0 or 1"
    if leaf in _COUNTS:
        return "count"
    for suffix, unit in suffixes:
        if leaf.endswith(suffix):
            return unit
    return "—"


def parameters() -> dict[str, list[Any]]:
    """Every parameter openmobisim assumes, at the defaults a ``Scenario`` uses.

    Columns by name, one row per parameter: ``group`` (``network``, ``loading``, ``modes``,
    ``transit``, ``parking``, ``scenario``, ``equilibration.<strategy>``,
    ``route_method.<method>``, ``route_update.<update>``, ``choice_model.<model>``), ``argument``
    (what overrides it by name: ``network_options`` of a network reader, ``loading_options``,
    ``equilibration_options`` … of ``Scenario.from_parts``, or the argument itself for the
    ``scenario`` group), ``name``, ``value``, ``unit`` (read from the name, which carries it) and
    ``note`` (where a ``Scenario`` departs from the core's default). Every value is an
    uncalibrated default; the documentation of each argument says where it comes from.

    Returns:
        The table, as lists by column.
    """
    rows = []
    for name, value, unit in _SCENARIO:
        rows.append(("scenario", name, name, value, unit, ""))
    listed: dict[str, set[str]] = {}
    for group, argument, name, value in _core.parameter_rows():
        value, note = _SCENARIO_DEFAULTS.get((group, name), (value, ""))
        rows.append((group, argument, name, float(value), _unit(name), note))
        listed.setdefault(group, set()).add(name)
    # A random-utility model weighs every attribute an alternative carries; those it does not
    # name weigh 0 (the mode constants among them: what a calibration usually sets first).
    for group in ("choice_model.logit", "choice_model.nested_logit"):
        for attribute in ROUTE_ATTRIBUTES:
            name = f"beta_{attribute}"
            if attribute != "nest" and name not in listed.get(group, set()):
                rows.append((group, "choice_options", name, 0.0, _unit(name), "0 unless set"))
    columns = ("group", "argument", "name", "value", "unit", "note")
    return {c: [r[i] for r in rows] for i, c in enumerate(columns)}
