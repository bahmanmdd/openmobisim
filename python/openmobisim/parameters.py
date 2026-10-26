"""Every parameter in one table: the place to start a calibration.

Every number openmobisim assumes has a name, an uncalibrated default and an argument that
overrides it by name. ``parameters()`` lists them all, at the defaults a ``Scenario`` uses.
"""

from __future__ import annotations

from openmobisim import _core
from openmobisim.choice import ROUTE_ATTRIBUTES
from openmobisim.table import Columns

__all__ = ["parameters"]

#: Where a default is a rule of the run's level rather than one number (the core's
#: defaults are the table; a ``Scenario`` sets none of its own).
_NOTES: dict[tuple[str, str], str] = {
    ("equilibration.msa", "increments"): "1 at flow_level 0 whatever is set",
    ("equilibration.sra", "increments"): "1 at flow_level 0 whatever is set",
    ("equilibration.free_flow", "increments"): "1 at flow_level 0 whatever is set",
    ("loading", "reroute"): "on at flow_level 3 and 4; off at 2 unless asked for (1)",
}

#: Scenario arguments that are numbers, with their defaults.
_SCENARIO = [
    ("flow_level", 2.0, "level"),
    ("flow_step_s", 300.0, "s"),
    ("window", 3.0, "h", "from the earliest departure (rounded down to the quarter hour)"),
    ("default_weight", 1.0, "travellers"),
    ("master_seed", 0.0, "—"),
    ("choice_detour_limit", 0.5, "share"),
    ("choice_max_routes", 5.0, "routes"),
    ("max_cost_vectors", 2.0, "vectors"),
    ("hub_group_m", 150.0, "m", "a label of the outputs (Scenario.run); not in the fingerprint"),
]

_COUNTS = ("iterations", "max_paths", "max_attempts", "max_routes", "searches", "candidates",
           "lanes", "increments", "max_rides", "reroute_max", "draws", "gap_sample",
           "itinerary_gap_sample", "warmup", "seed")  # fmt: skip
_FLAGS = ("priority", "reroute", "node_inflow", "origin_inflow", "biased", "fallback_car",
          "buses_on_roads")  # fmt: skip


def _unit(name: str) -> str:
    """The unit, read from the name's suffix (the names carry their units)."""
    leaf = name.split(".")[-1]
    suffixes = [
        ("_veh_h_lane", "veh/h per lane"),
        ("_veh_km_lane", "veh/km per lane"),
        ("_km_h", "km/h"),
        ("_eur_km", "EUR/km"),
        ("_km_eur", "EUR/km"),
        ("_eur", "EUR"),
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


def parameters() -> Columns:
    """Every parameter openmobisim assumes, at the defaults a ``Scenario`` uses.

    Columns by name, one row per parameter: ``group`` (``network``, ``loading``, ``modes``,
    ``transit``, ``parking``, ``prices``, ``scenario``, ``equilibration.<strategy>``,
    ``route_method.<method>``, ``route_update.<update>``, ``choice_model.<model>``), ``argument``
    (what overrides it by name: ``network_options`` of a network reader, ``loading_options``,
    ``equilibration_options`` … of ``Scenario.from_parts``, or the argument itself for the
    ``scenario`` group), ``name``, ``value``, ``unit`` (read from the name, which carries it) and
    ``note`` (where a default is a rule of the run's level). Every value is an
    uncalibrated default; the documentation of each argument says where it comes from.

    Returns:
        The table, as ``Columns`` (a dict of lists by column, with ``to_pandas()`` …).
    """
    rows = []
    for name, value, unit, *note in _SCENARIO:
        rows.append(("scenario", name, name, value, unit, note[0] if note else ""))
    listed: dict[str, set[str]] = {}
    for group, argument, name, value in _core.parameter_rows():
        note = _NOTES.get((group, name), "")
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
    return Columns({c: [r[i] for r in rows] for i, c in enumerate(columns)})
