"""The run configuration file, ``run.toml``: every parameter of a run in one file.

``config_write_defaults(path)`` writes the complete file at the shipped defaults (``openmobisim
config --defaults`` prints it): every parameter on its own line, its comment giving the default,
the meaning and the unit. Change what you want; a line left out keeps its default, so a file
holding a few lines is valid. ``config_read(path)`` reads a file back as a ``Config``;
``Scenario.run(config="run.toml")`` and ``openmobisim run --config run.toml`` run one. Every run
writes the complete file it ran with, ``run.toml``, beside its ``manifest.json``
(``Run.config()`` reads it back), so a run is repeated from its folder. Nothing is read unless a
file is given.

The tables, in the order a run is read: ``[inputs]`` (paths, relative to the file), ``[run]``,
``[equilibration]``, ``[choice]``, ``[modes]``, ``[prices]``, ``[services.<name>]`` (a service's
``kind``, its tables as paths, its options), ``[service_assumptions]``, ``[policy]``, ``[network]``
(applied when the network is read), ``[loading]``, ``[routes]``, ``[route_update]``, ``[transit]``
and ``[parking]``. A method's name and its options share a table: ``[equilibration]`` holds
``method = "msa"`` and the method's options. The defaults are those of ``parameters()``, read from
the core's defaults table, so the file and the code cannot disagree.
"""

from __future__ import annotations

import functools
import json
import math
import numbers
import re
import tomllib
from collections.abc import Mapping
from dataclasses import dataclass, field
from pathlib import Path
from typing import TYPE_CHECKING, Any

from openmobisim import _core

if TYPE_CHECKING:
    from openmobisim.run import Run
    from openmobisim.scenario import Scenario

__all__ = ["Config", "config_read", "config_write_defaults"]

#: The file's tables, in the order it lists them.
TABLES = (
    "inputs",
    "run",
    "equilibration",
    "choice",
    "modes",
    "prices",
    "services",
    "service_assumptions",
    "policy",
    "network",
    "loading",
    "routes",
    "route_update",
    "transit",
    "parking",
)

#: The registry's groups (``parameters()``), by the file's table that holds them: a group, or a
#: method's group by its prefix (``equilibration.msa`` is ``equilibration``'s). The one mapping.
GROUP_TABLES = {
    "network": "network",
    "loading": "loading",
    "modes": "modes",
    "transit": "transit",
    "parking": "parking",
    "prices": "prices",
    "service_prices": "prices",
    "equilibration": "equilibration",
    "choice_model": "choice",
    "route_method": "routes",
    "bike_route": "routes",
    "route_update": "route_update",
    "service": "services",
    "service_assumptions": "service_assumptions",
    "policy": "policy",
}
#: The registry's ``scenario`` rows, each its own argument, by the table that holds it.
_SCENARIO_TABLES = {
    "flow_level": "run",
    "flow_step_s": "run",
    "window": "run",
    "default_weight": "run",
    "master_seed": "run",
    "hub_join_m": "run",
    "choice_detour_limit": "choice",
    "choice_max_routes": "choice",
    "max_cost_vectors": "routes",
}
#: Registry groups that are not a run's (a fleet day's and its environment's): in no table.
NOT_IN_A_RUN = ("fleet_environment", "fleet")

#: A method table's method: its key, the argument it sets, the method whose options a file lists
#: when it names none (the one the shipped defaults run), the options' argument and the rule.
_METHODS = {
    "equilibration": (
        "method",
        "equilibration",
        "msa",
        "equilibration_options",
        'by the flow level: "msa", or "frank_wolfe" at flow_level 1',
    ),
    "choice": (
        "model",
        "choice_model",
        "logit",
        "choice_options",
        'by the modes: "logit" with one mode, "nested_logit" with several',
    ),
    "routes": (
        "method",
        "route_method",
        "shortest",
        "route_options",
        'by the run: "shortest" if it loads more than once, "link_penalty" if once',
    ),
    "route_update": (
        "method",
        "route_update",
        "least_cost",
        "route_update_options",
        'by the run: "least_cost" if it loads more than once, "none" if once',
    ),
}
#: The options' argument of each table without a method, and the bike routes' (``bike_`` keys).
_OPTIONS = {
    "network": "network_options",
    "loading": "loading_options",
    "modes": "mode_options",
    "transit": "transit_options",
    "parking": "parking_options",
    "prices": "price_options",
    "service_assumptions": "service_assumptions",
    "policy": "policy_options",
}
_BIKE = "bike_"
#: The arguments of ``Scenario.run`` a file holds (the others are ``Scenario.from_parts``').
RUN_ARGUMENTS = (
    "hub_join_m",
    "outputs",
    "threads",
    "design_id",
    "replication",
    "service_assumptions",
    "policy",
    "policy_options",
)
#: A service's kinds: the tables it is given (paths) and the options' registry group.
_SERVICE_TABLES = {"bike_docked": ("docks",), "taxi": ("vehicles", "chargers")}

#: The parameters that are not numbers of the registry, by table: key, default (``None``: no
#: value, the line written as a comment), unit, meaning and, for a default that is a rule, the
#: rule. The registry's rows are added to them.
_EXTRA: dict[str, list[tuple[str, Any, str, str, str]]] = {
    "inputs": [
        (
            "network",
            None,
            "path",
            "a case folder, a GMNS folder, an OpenStreetMap .osm.pbf or a cached network",
            "",
        ),
        ("demand", None, "path", "the trips, a trips.csv or trips.parquet", ""),
        ("persons", None, "path", "the travellers, a persons.parquet", ""),
        ("classes", None, "path", "the traveller classes, a classes.csv", ""),
        ("transit", None, "path", "the timetable, a GTFS feed (.zip or folder)", ""),
        (
            "transit_date",
            None,
            "date",
            "the GTFS service day, YYYY-MM-DD",
            "the feed's busiest weekday",
        ),
        (
            "parkings",
            None,
            "path",
            "the parkings: a table (.csv), an OpenStreetMap .osm.pbf or a cached file",
            "",
        ),
        ("zones", None, "path", "the zones' points, a zones.csv (read at flow_level 1)", ""),
    ],
    "run": [
        (
            "link_bin_s",
            None,
            "s",
            "per-link results in time bins of this length",
            "none: not recorded",
        ),
        (
            "outputs",
            "all",
            "",
            '"all" files, or "minimal": kpis.csv, manifest.json and run.toml',
            "",
        ),
        (
            "threads",
            None,
            "",
            "the threads of the run's parallel work (results do not depend on it)",
            "none: every logical core",
        ),
        ("design_id", "default", "", "the design this run belongs to, a label of its results", ""),
        ("replication", 0, "", "which replication of its design this run is, a label", ""),
    ],
    "choice": [],
    "modes": [
        (
            "modes",
            None,
            "",
            "the modes a trip without a stated mode chooses among: car, bike, "
            "walk, transit, car_transit, bike_transit, bike_share, bike_share_transit, taxi",
            "none: such trips drive (the classes' modes, if a class table names them)",
        ),
    ],
    "routes": [
        (
            "bike_cost",
            "utility",
            "",
            'what bike routes are searched by: "utility", "dedicated" or "time"',
            "",
        ),
        (
            "bike_route_choice",
            "shortest",
            "",
            'a bike trip\'s routes: "shortest", one per pair, '
            'or "doubly_stochastic", a set it chooses from (the bike_ options below)',
            "",
        ),
    ],
    "policy": [
        (
            "name",
            None,
            "",
            "an operator steering the services in the run's service day: noop, "
            "reset, threshold, charge_rule or a registered one",
            "none: no operator",
        ),
    ],
}
#: Integer-valued arguments (a whole number in the file).
_INTEGERS = {
    "flow_level",
    "flow_step_s",
    "master_seed",
    "default_weight",
    "choice_max_routes",
    "max_cost_vectors",
    "link_bin_s",
    "threads",
    "replication",
}
_TEXTS = {"outputs", "design_id", "bike_cost", "bike_route_choice", "transit_date"}

#: The file's own keys that are not parameters of the registry: a service's ``kind`` and tables, and
#: the choice table's method. Every parameter's meaning is the registry's (``parameters()``'s
#: ``meaning``, one copy in the package's ``data/parameters.csv``).
_FILE_KEYS = {
    "kind": '"taxi", an electric taxi fleet, or "bike_docked", docked bike sharing',
    "docks": "the docks: dock_id, lon, lat, capacity (name, hub_id, bikes optional)",
    "vehicles": "the vehicles: vehicle_id, lon, lat (soc optional)",
    "chargers": "the chargers: charger_id, lon, lat, plugs (kw, hub_id optional); none: they "
    "never charge",
    "model": "how each trip picks an alternative: logit, nested_logit, deterministic or a "
    "registered model",
}
#: Unit words in a comment: none for the units a number carries in its name only.
_UNIT_WORDS = {
    **dict.fromkeys(("—", "count", "level", "share", "0 or 1", "path", "date"), ""),
    **dict.fromkeys(("travellers", "routes", "vectors", "per unit of the attribute"), ""),
}


@dataclass(frozen=True)
class _Line:
    """One parameter of the file: where it is, what it sets, its default and its words."""

    table: str
    key: str
    argument: str
    default: Any
    unit: str
    meaning: str
    rule: str = ""
    method: str | None = None
    kind: str | None = None  # a service's kind, for its options
    note: str = ""  # a default's exception, for its comment


def _clause(meaning: str) -> str:
    """A registry meaning as a comment's clause: its first sentence (the registry holds it all)."""
    first = re.split(r"(?<=\.)\s+(?=[A-Z0-9`(])", meaning, maxsplit=1)[0].rstrip(".")
    if first[:1].isupper() and first[1:2] in (" ", *"abcdefghijklmnopqrstuvwxyz"):
        first = first[0].lower() + first[1:]
    return first


#: Registry rows whose default is a rule of the run, not their listed number: written as a
#: comment, passed only when stated.
_RULES = {
    ("loading", "reroute"): "on at flow_level 3 and 4, off at 2",
    ("scenario", "window"): "3 h from the earliest departure, rounded down to the quarter hour",
}
#: The road classes' own volume-delay parameters of ``[loading]`` (``motorway.bpr_alpha``).
_PER_CLASS = (".bpr_alpha", ".bpr_beta")
#: The order of a table's first lines (the rest follow in the registry's order).
_FIRST = {
    "run": ("flow_level", "flow_step_s", "window", "master_seed", "default_weight", "link_bin_s",
            "hub_join_m", "outputs", "threads", "design_id", "replication"),
    "routes": ("method", "max_cost_vectors", "bike_cost", "bike_route_choice"),
}  # fmt: skip
#: What a method table's method chooses.
_METHOD_MEANING = {
    "equilibration": "how choice and loading are repeated: msa, sra, frank_wolfe (flow_level 1) "
    "or free_flow",
    "choice": _FILE_KEYS["model"],
    "routes": "how route sets are generated: shortest, link_penalty, simulation or "
    "doubly_stochastic",
    "route_update": "how route sets grow between iterations: least_cost, doubly_stochastic or none",
}


@functools.cache
def _lines() -> dict[str, list[_Line]]:
    """Every parameter the file holds, by table, in the order it lists them."""
    from openmobisim.parameters import parameters

    out: dict[str, list[_Line]] = {t: [] for t in TABLES}
    for t, (key, argument, _, _, rule) in _METHODS.items():
        out[t].append(_Line(t, key, argument, None, "", _METHOD_MEANING[t], rule))
    for table, extra in _EXTRA.items():
        for key, default, unit, meaning, rule in extra:
            argument = "inputs" if table == "inputs" else {"name": "policy"}.get(key, key)
            out[table].append(_Line(table, key, argument, default, unit, meaning, rule))
    rows = parameters()
    for group, name, value, unit, note, meaning in zip(
        rows["group"], rows["name"], rows["value"], rows["unit"], rows["note"], rows["meaning"],
        strict=True,
    ):  # fmt: skip
        meaning = _clause(meaning)
        prefix, _, method = group.partition(".")
        if prefix in NOT_IN_A_RUN:
            continue
        note = "" if note == "0 unless set" or name == "hub_join_m" else note
        rule = _RULES.get((group, name), note.removeprefix("None: ") if math.isnan(value) else "")
        if group == "loading" and name.endswith(_PER_CLASS):
            # A road class's own pair: given, it wins over the pair for every class, so it is
            # written as a comment and passed only when stated.
            rule = f"{name.split('.')[1]} above, every class's"
        default: Any = None if rule else value
        if rule:
            note = ""
        if prefix == "scenario":
            table = _SCENARIO_TABLES[name]
            if name in _INTEGERS:
                default = int(value)
            line = _Line(table, name, name, default, unit, meaning, rule, note=note)
        elif prefix == "service":
            line = _Line("services", name, "options", default, unit, meaning, rule, kind=method)
        elif prefix == "policy":
            line = _Line("policy", name, "policy_options", default, unit, meaning, rule,
                         method=method)  # fmt: skip
        elif prefix == "bike_route":
            key = _BIKE + name
            line = _Line("routes", key, "bike_route_options", default, unit, meaning, rule)
        else:
            table = GROUP_TABLES[prefix]
            argument = _METHODS[table][3] if table in _METHODS else _OPTIONS[table]
            line = _Line(table, name, argument, default, unit, meaning, rule,
                         method=method or None, note=note)  # fmt: skip
        out[line.table].append(line)
    for table, first in _FIRST.items():
        rank = {k: i for i, k in enumerate(first)}
        out[table].sort(key=lambda x: (rank.get(x.key, len(rank)), x.method is None))
    return out


def _methods(table: str) -> list[str]:
    """The methods whose options a table lists (the registry's)."""
    seen: list[str] = []
    for line in _lines()[table]:
        if line.method is not None and line.method not in seen:
            seen.append(line.method)
    return seen


def _listed(table: str, method: str | None) -> list[_Line]:
    """The lines a table writes: its own, and the options of ``method`` (or the default's)."""
    lines = _lines()[table]
    if table == "policy":
        return [x for x in lines if x.method is None or x.method == method]
    if table not in _METHODS:
        return [x for x in lines if x.kind is None]
    chosen = method if method is not None else _METHODS[table][2]
    keep = {chosen}
    if table == "choice" and method is None:
        keep.add("nested_logit")  # its mu: the default model with several modes
    head = [x for x in lines if x.key == _METHODS[table][0]]
    options, seen = [], set()
    for x in lines:
        if x.method in keep and x.key not in seen:
            options.append(x)
            seen.add(x.key)
    rest = [x for x in lines if x.method is None and x not in head]
    if table == "routes":  # its own lines after the method's options, the bike routes' last
        return head + options + rest
    return head + rest + options


def _option_defaults(table: str, method: str | None) -> dict[str, Any]:
    """The numbers the options a table lists fall back to (``None``: the default is a rule)."""
    return {x.key: None if x.rule else x.default for x in _listed(table, method)}


# --- the TOML text ---------------------------------------------------------------------------

_BARE = re.compile(r"^[A-Za-z0-9_-]+$")


def _key(key: str) -> str:
    """A key as TOML writes it: bare, dotted (a road class's), or quoted."""
    if all(_BARE.match(part) for part in key.split(".")):
        return key
    return json.dumps(key, ensure_ascii=False)


def _value(v: Any) -> str:
    """A value as TOML writes it: a string, a number, a boolean, or a list of those."""
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, str | Path):
        return json.dumps(str(v), ensure_ascii=False)
    if isinstance(v, numbers.Integral):
        return str(int(v))
    if isinstance(v, numbers.Real):
        v = float(v)
        if math.isnan(v):
            return "nan"
        if math.isinf(v):
            return "inf" if v > 0 else "-inf"
        if v == int(v) and abs(v) < 2**53:
            return str(int(v))
        return repr(v)
    if isinstance(v, list | tuple):
        return "[" + ", ".join(_value(x) for x in v) + "]"
    raise TypeError(f"run.toml holds strings, numbers, booleans and lists of them, not {v!r}")


def _comment(line: _Line, resolved: Any = None) -> str:
    """A line's comment: its default, its unit and its meaning."""
    if line.table == "inputs":
        return line.meaning + (f"; default: {line.rule}" if line.rule else "")
    unit = _UNIT_WORDS.get(line.unit, line.unit)
    if line.rule:
        text = f"default {line.rule}"
    else:
        text = f"default {_value(line.default)}" + (f" {unit}" if unit else "")
        if line.note:
            text += f" ({line.note})"
    if resolved is not None:
        text += f"; this run: {_value(resolved)}"
    if not line.meaning:
        return text
    return (
        f"{text}; {line.meaning}"
        if line.rule or resolved is not None
        else f"{text}: {line.meaning}"
    )


def _entry(key: str, value: Any, comment: str, commented: bool = False) -> str:
    """One line: ``key = value`` and its comment, aligned."""
    return _aligned(f"{'# ' if commented else ''}{_key(key)} = {_value(value)}", comment)


def _aligned(text: str, comment: str) -> str:
    return f"{text:<35} # {comment}" if len(text) < 35 else f"{text}  # {comment}"


def _example(line: _Line) -> Any:
    """The value a commented line shows when its default is no value."""
    examples = {
        "network": "network/",
        "demand": "demand/trips.csv",
        "persons": "demand/persons.parquet",
        "classes": "classes.csv",
        "transit": "gtfs.zip",
        "transit_date": "2026-03-10",
        "parkings": "parkings.csv",
        "zones": "zones.csv",
        "link_bin_s": 900,
        "threads": 4,
        "window": ["07:30", "09:30"],
        "modes": ["car", "bike", "walk", "transit"],
        "name": "threshold",
        "method": _METHODS.get(line.table, ("", "", ""))[2],
        "model": "logit",
        "reroute": 1,
        "increments": 5,
        "assumed_extra_walk_at_pickup_s": 300,
        "assumed_extra_walk_at_return_s": 300,
    }
    if line.key.endswith(_PER_CLASS):
        return 0.15 if line.key.endswith("alpha") else 4
    return examples.get(line.key, 0)


# --- the configuration ---------------------------------------------------------------------


@dataclass
class Config:
    """A run's parameters, as a ``run.toml`` states them (``config_read``) or a run was given.

    ``values`` holds what is stated, by table (``values["choice"]["beta_time_min"]``; a service's
    under ``values["services"][name]``); every parameter not stated keeps its default.
    ``write(path)`` writes the complete file, ``text()`` returns it, ``differences()`` lists what
    differs from the defaults, ``scenario()`` reads its ``[inputs]`` into a ``Scenario`` and
    ``run()`` runs it.
    """

    #: The run's name (``run_id``, and its output folder's default name); ``None``: not stated.
    name: str | None = None
    #: What the file states, by table.
    values: dict[str, Any] = field(default_factory=dict)
    #: The folder relative paths are resolved against: the file's.
    folder: Path | None = None
    #: What a run had that a file cannot hold (written in the header): set by a run's copy.
    absent: list[str] = field(default_factory=list)
    #: What a rule chose, by ``table.key``, written in the comments of a run's copy.
    resolved: dict[str, Any] = field(default_factory=dict)

    @classmethod
    def from_arguments(cls, run_id: str | None = None, **arguments: Any) -> Config:
        """The configuration of a run with these arguments, to write as a file.

        The keyword arguments of ``Scenario.from_parts`` and ``Scenario.run`` that are
        parameters (numbers, names, lists of modes, options dicts, services, a policy by name or a
        built-in one), and ``network_options``; ``run_id`` is the file's ``name``. What a file
        cannot hold (link values, disruptions, a Python choice model or policy) is listed in
        ``absent`` and the file's header.

        Raises:
            TypeError: For an argument that is not one of these, with the list of them.
        """
        known = set(_ARGUMENTS) | set(RUN_ARGUMENTS)
        unknown = sorted(set(arguments) - known)
        if unknown:
            raise TypeError(
                f"Config.from_arguments: no argument {unknown}; they are {sorted(known)}"
            )
        parts = {k: v for k, v in arguments.items() if k not in RUN_ARGUMENTS}
        run = {k: v for k, v in arguments.items() if k in RUN_ARGUMENTS}
        values, absent = _stated(parts, run)
        return cls(name=run_id, values=values, absent=absent)

    # --- writing ---

    def differences(self) -> list[str]:
        """The stated parameters that differ from their defaults, as ``table.key``.

        A service or a policy is listed as a whole (``services.taxi``, ``policy``); the inputs
        are not parameters.
        """
        out: list[str] = []
        for table in TABLES:
            stated = self.values.get(table)
            if not stated or table == "inputs":
                continue
            if table == "services":
                out += [f"services.{name}" for name in stated]
                continue
            if table == "policy":
                if stated.get("name") is not None:
                    out.append("policy")
                continue
            method = _method_of(table, stated)
            defaults = {x.key: x for x in _lines()[table] if x.method in (None, method)}
            if table in _METHODS and method is None:
                defaults.update({x.key: x for x in _listed(table, None)})
            for key, value in stated.items():
                line = defaults.get(key)
                if line is None or line.rule or line.default is None or value != line.default:
                    out.append(f"{table}.{key}")
        return out

    def text(self) -> str:
        """The complete file: every parameter, stated or at its default, with its comment."""
        info = _core.build_info()
        differs = self.differences()
        out = [
            f"# openmobisim run configuration, written by openmobisim {info['version']} "
            f"(defaults version {info['defaults_version']}).",
            "# Differs from the defaults: " + (", ".join(differs) if differs else "nothing") + ".",
            "# Every line not given in a file you write keeps its default; this file lists them "
            "all.",
        ]
        if self.absent:
            out.append(
                "# Not in this file (given in Python; a run from this file alone lacks them): "
                + ", ".join(self.absent)
                + "."
            )
        about = "the run's id; also its output folder's name"
        if self.name is None:
            out += ["", _entry("name", "run", f'default "run": {about}', commented=True)]
        else:
            out += ["", _entry("name", self.name, about)]
        for table in TABLES:
            out.append("")
            out += self._table_text(table)
        return "\n".join(out).rstrip() + "\n"

    def _table_text(self, table: str) -> list[str]:
        stated = self.values.get(table) or {}
        if table == "services":
            return self._services_text(stated)
        heads = {
            "inputs": "paths relative to this file",
            "service_assumptions": "what travellers assume when they choose, fixed for the run",
            "policy": "an operator steering the services; none by default",
            "network": "applied when the network is read",
        }
        out = [_aligned(f"[{table}]", heads[table]) if table in heads else f"[{table}]"]
        method = stated.get("name") if table == "policy" else _method_of(table, stated)
        lines = _listed(table, method)
        known = {x.key for x in lines}
        for line in lines:
            resolved = self.resolved.get(f"{table}.{line.key}")
            if line.key in stated:
                out.append(_entry(line.key, stated[line.key], _comment(line)))
            elif line.default is None:
                out.append(_entry(line.key, resolved if resolved is not None else _example(line),
                                  _comment(line, resolved), commented=True))  # fmt: skip
            else:
                out.append(_entry(line.key, line.default, _comment(line)))
        for key, value in stated.items():  # a registered method's own options
            if key not in known:
                out.append(_entry(key, value, "an option of the method named above"))
        if table == "policy" and method is None:
            for name in ("noop", "reset", "threshold", "charge_rule"):
                options = ", ".join(
                    f"{x.key} {_value(x.default)}" for x in _lines()["policy"] if x.method == name
                )
                out.append(f'#   name = "{name}" takes {options}')
        return out

    def _services_text(self, stated: Mapping[str, Mapping[str, Any]]) -> list[str]:
        out: list[str] = []
        for name, service in stated.items():
            kind = service.get("kind")
            if kind not in _SERVICE_TABLES:
                raise ValueError(f"[services.{name}]: kind is one of {sorted(_SERVICE_TABLES)}")
            what = "a taxi fleet" if kind == "taxi" else "a docked bike service"
            out.append(
                _aligned(f"[services.{_key(name)}]", f"{what}; the table's name is its name")
            )
            out.append(_entry("kind", kind, _FILE_KEYS["kind"]))
            for t in _SERVICE_TABLES[kind]:
                if service.get(t) is not None:
                    out.append(_entry(t, service[t], _FILE_KEYS[t]))
                else:
                    about = f"a path; {_FILE_KEYS[t]} (this run's was in memory)"
                    out.append(_entry(t, f"{t}.csv", about, commented=True))
            for line in _lines()["services"]:
                if line.kind == kind:
                    value = service.get(line.key, line.default)
                    out.append(_entry(line.key, value, _comment(line)))
            out.append("")
        if out:
            out.pop()
        if not stated:
            out.append("# [services.<name>]: a shared-mobility service, none by default. Each:")
            for kind, tables in _SERVICE_TABLES.items():
                options = ", ".join(
                    f"{x.key} {_value(x.default)}" for x in _lines()["services"] if x.kind == kind
                )
                out.append(f'#   kind = "{kind}", {" and ".join(tables)} (paths); its options '
                           f"(default): {options}")  # fmt: skip
        return out

    def write(self, path: str | Path) -> Path:
        """Write the complete file to ``path`` and return the path."""
        p = Path(path)
        p.write_text(self.text(), encoding="utf-8")
        return p

    # --- reading into arguments ---

    def _path(self, value: Any, where: str) -> str:
        if not isinstance(value, str) or not value:
            raise ValueError(f"run.toml {where}: a path, got {value!r}")
        p = Path(value).expanduser()
        if not p.is_absolute() and self.folder is not None:
            p = self.folder / p
        return str(p.resolve())

    def inputs(self) -> dict[str, Any]:
        """The ``[inputs]`` table's paths, resolved against the file's folder (dates as given)."""
        out: dict[str, Any] = {}
        for key, value in (self.values.get("inputs") or {}).items():
            out[key] = value if key == "transit_date" else self._path(value, f"[inputs] {key}")
        return out

    def _service_inputs(self) -> dict[str, dict[str, str]]:
        out: dict[str, dict[str, str]] = {}
        for name, service in (self.values.get("services") or {}).items():
            for t in _SERVICE_TABLES.get(service.get("kind"), ()):
                if service.get(t) is not None:
                    where = f"[services.{name}] {t}"
                    out.setdefault(name, {})[t] = self._path(service[t], where)
        return out

    def scenario(self) -> Scenario:
        """A ``Scenario`` of the file's ``[inputs]`` (read by the package's readers) and values.

        ``network`` is read by its kind: a starter case's folder (its ``case.json``), a GMNS folder,
        an OpenStreetMap ``.osm.pbf`` (both with the file's ``[network]``) or a file
        ``network_write_cache`` wrote; ``demand`` (required), ``persons`` and ``zones`` are
        given to ``Scenario.from_parts`` as paths; ``classes`` by ``demand_read_classes``;
        ``transit`` by ``transit_read_gtfs`` (on ``transit_date``); ``parkings`` by
        ``parking_read_table`` (a ``.csv``), ``parking_read_osm`` (a ``.pbf``) or
        ``parking_read_cache``; a service's tables by ``dock_read_table``,
        ``vehicle_read_table`` and ``charger_read_table``.

        Raises:
            ValueError: If ``network`` or ``demand`` is missing, or a value is not valid.
        """
        from openmobisim.scenario import Scenario

        inputs = self.inputs()
        for required in ("network", "demand"):
            if required not in inputs:
                raise ValueError(f"run.toml: [inputs] needs {required} to build a scenario")
        stated, _ = self._arguments()
        network = _network_read(inputs["network"], stated.pop("network_options", None) or {})
        parts: dict[str, Any] = {}
        if "persons" in inputs:
            parts["persons"] = inputs["persons"]
        if "zones" in inputs:
            parts["zones"] = inputs["zones"]
        if "classes" in inputs:
            from openmobisim.demand import demand_read_classes

            parts["classes"] = demand_read_classes(inputs["classes"])
        if "transit" in inputs:
            parts["transit"] = _core.transit_read_gtfs(
                inputs["transit"], network=network, date=inputs.get("transit_date")
            )
        if "parkings" in inputs:
            parts["parkings"] = _parkings_read(inputs["parkings"])
        services = self._services_built()
        if services:
            parts["services"] = services
        arguments = _scenario_arguments(stated, {})
        scenario = Scenario.from_parts(network, inputs["demand"], **parts, **arguments)
        scenario._inputs = {**inputs, "services": self._service_inputs()}
        return scenario

    def run(
        self, output_dir: str | None = None, *, overwrite: bool = False, quiet: bool = False
    ) -> Run:
        """Run the file: ``scenario()``, then ``Scenario.run(config=self)``."""
        return self.scenario().run(
            config=self, output_dir=output_dir, overwrite=overwrite, quiet=quiet
        )

    def _services_built(self) -> list[Any]:
        from openmobisim.fleet import service_bike_docked, service_taxi
        from openmobisim.services import charger_read_table, dock_read_table, vehicle_read_table

        tables = self._service_inputs()
        out = []
        for name, service in (self.values.get("services") or {}).items():
            kind = service.get("kind")
            options = _service_options(name, service)
            given = tables.get(name, {})
            if kind == "bike_docked":
                if "docks" not in given:
                    raise ValueError(
                        f"run.toml [services.{name}]: a docked bike service needs docks"
                    )
                docks = dock_read_table(given["docks"])
                out.append(service_bike_docked(docks, name=name, options=options))
            else:
                if "vehicles" not in given:
                    raise ValueError(f"run.toml [services.{name}]: a taxi fleet needs vehicles")
                chargers = given.get("chargers")
                out.append(
                    service_taxi(
                        vehicle_read_table(given["vehicles"]),
                        charger_read_table(chargers) if chargers else None,
                        name=name,
                        options=options,
                    )
                )
        return out

    def _arguments(self) -> tuple[dict[str, Any], dict[str, Any]]:
        """The stated values as arguments: ``(scenario's, run's)``, options as stated."""
        scenario: dict[str, Any] = {}
        run: dict[str, Any] = {}
        for table in TABLES:
            stated = self.values.get(table)
            if not stated or table in ("inputs", "services"):
                continue
            method = _method_of(table, stated)
            for key, value in stated.items():
                line = _find(table, key, method, stated)
                target = run if line.argument in RUN_ARGUMENTS else scenario
                if line.argument.endswith("_options") or line.argument in ("service_assumptions",):
                    name = key[len(_BIKE) :] if line.argument == "bike_route_options" else key
                    target.setdefault(line.argument, {})[name] = value
                else:
                    target[line.argument] = value
        return scenario, run


def _method_of(table: str, stated: Mapping[str, Any]) -> str | None:
    if table not in _METHODS:
        return None
    m = stated.get(_METHODS[table][0])
    return m if isinstance(m, str) else None


def _find(table: str, key: str, method: str | None, stated: Mapping[str, Any]) -> _Line:
    """The line of ``key`` in ``table``; a name not there is refused with the table's names."""
    lines = _lines()[table]
    if table == "policy":
        if key == "name":
            return lines[0]
        name = stated.get("name")
        known = [x for x in lines if x.method == name]
        if not known and name is not None:  # a registered policy: options of its own
            return _Line("policy", key, "policy_options", None, "", "")
        for x in known:
            if x.key == key:
                return x
        names = ["name", *(x.key for x in known)]
        raise ValueError(f"run.toml [policy]: no {key!r} for policy {name!r}; its names: {names}")
    if table in _METHODS and method is not None and method not in _methods(table):
        # A method without options of the registry (a registered choice model, "none" ...).
        own = [x for x in lines if x.method is None]
        for x in own:
            if x.key == key:
                return x
        return _Line(table, key, _METHODS[table][3], None, "", "")
    candidates = [x for x in lines if x.key == key and x.method in (None, method)]
    if method is None:
        candidates = [x for x in lines if x.key == key]
    if candidates:
        return candidates[0]
    names = sorted({x.key for x in lines if x.method in (None, method) or method is None})
    if table in _METHODS and method is not None:
        raise ValueError(
            f"run.toml [{table}]: no {key!r} for {_METHODS[table][0]} = {method!r}; its names: "
            f"{names} (the other options listed may be another method's: remove them)"
        )
    raise ValueError(f"run.toml [{table}]: no parameter {key!r}; its names: {names}")


def _service_options(name: str, service: Mapping[str, Any]) -> dict[str, float]:
    kind = service.get("kind")
    defaults = {x.key: x.default for x in _lines()["services"] if x.kind == kind}
    out = {}
    for key, value in service.items():
        if key == "kind" or key in _SERVICE_TABLES.get(kind, ()):
            continue
        if key not in defaults:
            raise ValueError(
                f"run.toml [services.{name}]: no {key!r} for kind {kind!r}; its names: "
                f"{['kind', *_SERVICE_TABLES.get(kind, ()), *defaults]}"
            )
        if value != defaults[key]:
            out[key] = float(value)
    return out


def _options(
    table: str, method: str | None, base: Mapping[str, Any] | None, stated: Mapping[str, Any]
) -> dict[str, Any] | None:
    """``base`` with the stated options: a value at its default removed (it changes nothing)."""
    defaults = _option_defaults(table, method)
    out = dict(base or {})
    for key, value in stated.items():
        if key in defaults and defaults[key] is not None and value == defaults[key]:
            out.pop(key, None)
        else:
            out[key] = value
    return out or None


def _scenario_arguments(stated: Mapping[str, Any], parts: Mapping[str, Any]) -> dict[str, Any]:
    """The arguments of ``from_parts`` the file states, over ``parts`` (a scenario's)."""
    out: dict[str, Any] = {}
    option_tables = {v[3]: t for t, v in _METHODS.items()}
    option_tables.update({v: t for t, v in _OPTIONS.items()})
    for argument, value in stated.items():
        if argument in option_tables or argument == "bike_route_options":
            continue
        out[argument] = value
    for argument, value in stated.items():
        if argument == "bike_route_options":
            given = parts.get("bike_route_options")
            out[argument] = _bike_options(given, value)
            continue
        if argument not in option_tables:
            continue
        table = option_tables[argument]
        method = None
        base = parts.get(argument)
        if table in _METHODS:
            margument = _METHODS[table][1]
            method = out.get(margument, parts.get(margument))
            if margument in out and out[margument] != parts.get(margument):
                base = None  # the scenario's options were another method's
            if not isinstance(method, str):
                method = None
        if table == "routes" and method is None and _options(table, None, None, value):
            raise ValueError(
                'run.toml [routes]: options without a method; name it (method = "shortest" ...):'
                " unsaid, the method depends on the run, and options alone mean link_penalty"
            )
        out[argument] = _options(table, method, base, value)
    return out


def _bike_options(base: Mapping[str, Any] | None, stated: Mapping[str, Any]) -> Any:
    defaults = {x.key[len(_BIKE) :]: x.default for x in _lines()["routes"]
                if x.argument == "bike_route_options"}  # fmt: skip
    out = dict(base or {})
    for key, value in stated.items():
        if value == defaults.get(key):
            out.pop(key, None)
        else:
            out[key] = value
    return out or None


def _run_arguments(stated: Mapping[str, Any]) -> dict[str, Any]:
    """The arguments of ``Scenario.run`` the file states, options at their defaults left out."""
    out = dict(stated)
    if "service_assumptions" in out:
        out["service_assumptions"] = _options("service_assumptions", None, None,
                                              out["service_assumptions"])  # fmt: skip
    if "policy_options" in out:
        name = out.get("policy")
        defaults = {x.key: x.default for x in _lines()["policy"] if x.method == name}
        given = {k: v for k, v in out["policy_options"].items() if defaults.get(k) != v}
        out["policy_options"] = given or None
    return out


# --- values: checked and converted ---------------------------------------------------------


def _flat(table: str, given: Mapping[str, Any], prefix: str = "") -> dict[str, Any]:
    """A table's values, a dotted key's sub-tables flattened (``motorway.lanes``)."""
    out: dict[str, Any] = {}
    for key, value in given.items():
        name = f"{prefix}{key}"
        if isinstance(value, Mapping):
            out.update(_flat(table, value, f"{name}."))
        else:
            out[name] = value
    return out


def _checked(table: str, key: str, value: Any, line: _Line) -> Any:
    """A value of the file, converted to what its argument takes, or refused naming it."""
    where = f"run.toml [{table}] {key}"
    if table == "inputs":
        if not isinstance(value, str):
            raise ValueError(f"{where}: a path (a string), got {value!r}")
        return value
    argument = line.argument
    if argument in _METHODS.get(table, ("", "x"))[1:2] or key in _TEXTS or argument == "policy":
        if not isinstance(value, str):
            raise ValueError(f"{where}: a name (a string), got {value!r}")
        return value
    if key == "modes":
        if not (isinstance(value, list) and all(isinstance(m, str) for m in value)):
            raise ValueError(f'{where}: a list of modes, such as ["car", "bike"], got {value!r}')
        return value
    if key == "window":
        if not (isinstance(value, list) and len(value) == 2):
            raise ValueError(f'{where}: [start, end], such as ["07:30", "09:30"], got {value!r}')
        for v in value:
            if isinstance(v, bool) or not isinstance(v, str | int | float):
                raise ValueError(f"{where}: clock times or seconds, got {value!r}")
        return tuple(value)
    if isinstance(value, bool):
        value = int(value)
    if not isinstance(value, int | float):
        raise ValueError(f"{where}: a number, got {value!r}")
    if key in _INTEGERS and line.method is None and argument == key:
        if value != int(value):
            raise ValueError(f"{where}: a whole number, got {value!r}")
        return int(value)
    if argument == key and key in ("choice_detour_limit", "hub_join_m"):
        return float(value)
    return float(value)


def _values(data: Mapping[str, Any]) -> tuple[str | None, dict[str, Any]]:
    """The file's name and values, every table and name checked."""
    name = data.get("name")
    if name is not None and not isinstance(name, str):
        raise ValueError(f"run.toml name: a string, got {name!r}")
    unknown = sorted(set(data) - {"name", *TABLES})
    if unknown:
        raise ValueError(
            f"run.toml: no table or name {unknown}; its tables: {list(TABLES)} (and name)"
        )
    values: dict[str, Any] = {}
    for table in TABLES:
        given = data.get(table)
        if given is None:
            continue
        if not isinstance(given, Mapping):
            raise ValueError(f"run.toml: {table} is a table ([{table}]), got {given!r}")
        if table == "services":
            values["services"] = {n: _service_values(n, s) for n, s in given.items()}
            continue
        flat = _flat(table, given)
        method = _method_of(table, flat)
        checked = {}
        for key, value in flat.items():
            line = _find(table, key, method, flat)
            checked[key] = _checked(table, key, value, line)
        values[table] = checked
    return name, values


def _service_values(name: str, given: Any) -> dict[str, Any]:
    if not isinstance(given, Mapping):
        raise ValueError(f"run.toml: [services.{name}] is a table, got {given!r}")
    kind = given.get("kind")
    if kind not in _SERVICE_TABLES:
        raise ValueError(
            f"run.toml [services.{name}]: kind is one of {sorted(_SERVICE_TABLES)}, got {kind!r}"
        )
    out: dict[str, Any] = {"kind": kind}
    lines = {x.key: x for x in _lines()["services"] if x.kind == kind}
    for key, value in given.items():
        if key == "kind":
            continue
        if key in _SERVICE_TABLES[kind]:
            if not isinstance(value, str):
                raise ValueError(f"run.toml [services.{name}] {key}: a path, got {value!r}")
            out[key] = value
        elif key in lines:
            out[key] = _checked("services", key, value, lines[key])
        else:
            names = ["kind", *_SERVICE_TABLES[kind], *lines]
            raise ValueError(
                f"run.toml [services.{name}]: no {key!r} for kind {kind!r}; its names: {names}"
            )
    return out


def config_read(path: str | Path) -> Config:
    """Read a ``run.toml``: what it states, every name and value checked.

    Relative paths (``[inputs]``, a service's tables) are resolved against the file's folder
    when used. A table or a name not in the file's layout is refused with the list of its table's
    names, a value of the wrong kind with what it should be.

    Raises:
        FileNotFoundError: If there is no such file.
        ValueError: If it is not TOML, or a name or value is not valid.
    """
    p = Path(path)
    if p.is_dir():
        p = p / "run.toml"
    try:
        data = tomllib.loads(p.read_text(encoding="utf-8"))
    except tomllib.TOMLDecodeError as e:
        raise ValueError(f"{str(p)!r} is not a valid TOML file: {e}") from None
    name, values = _values(data)
    return Config(name=name, values=values, folder=p.resolve().parent)


def config_write_defaults(path: str | Path) -> Path:
    """Write the complete ``run.toml`` at the shipped defaults to ``path``; return the path.

    Every parameter on its line, its comment giving its default, its meaning and its unit; the
    inputs, a service and a policy (none by default) as comments to fill in.
    """
    return Config().write(path)


# --- a run's copy, and a run from a file ---------------------------------------------------

#: The arguments of ``from_parts`` that are inputs, by the ``[inputs]`` key naming them.
_INPUT_PARTS = {
    "network": "network",
    "demand": "demand",
    "persons": "persons",
    "classes": "classes",
    "transit": "transit",
    "transit_date": "transit",
    "parkings": "parkings",
    "zones": "zones",
}
#: Arguments a file cannot hold, by what the header calls them, when set.
_NOT_EXPRESSIBLE = (
    "link_values",
    "person_values",
    "trip_values",
    "disruptions",
    "disruptions_known",
)
#: The arguments of ``from_parts`` that ``Config.from_arguments`` takes.
_ARGUMENTS = (
    "default_weight", "window", "flow_level", "flow_step_s", "link_bin_s", "route_method",
    "route_options", "master_seed", "choice_model", "choice_options", "equilibration",
    "equilibration_options", "route_update", "route_update_options", "choice_detour_limit",
    "choice_max_routes", "max_cost_vectors", "bike_cost", "bike_route_choice",
    "bike_route_options", "transit_options", "parking_options", "services", "modes",
    "mode_options", "loading_options", "price_options", "network_options", "preset",
    *_NOT_EXPRESSIBLE,
)  # fmt: skip


def _config_of(scenario: Scenario, run: Mapping[str, Any], manifest: Mapping[str, Any]) -> Config:
    """The complete, effective configuration of a run of ``scenario`` with ``run``'s arguments."""
    parts = scenario._parts
    recorded = dict(getattr(scenario, "_inputs", {}))
    services_given = recorded.pop("services", {})
    inputs = {k: v for k, v in recorded.items() if v is not None}
    for key in ("demand", "persons", "zones"):
        if key not in inputs and isinstance(parts.get(key), str | Path):
            inputs[key] = str(Path(parts[key]).resolve())
    absent = [
        f"{key} (in memory)"
        for key in ("network", "demand", "persons", "classes", "transit", "parkings", "zones")
        if key not in inputs and parts.get(key) is not None
    ]
    shipped = {x.key: x.default for x in _lines()["network"]}
    network = {
        k: v
        for k, v in _core.network_options(scenario._network)
        if k in shipped and v != shipped[k]
    }
    values, more = _stated({**parts, "network_options": network}, run, services_given)
    if inputs:
        values = {"inputs": inputs, **values}
    resolved = {
        "equilibration.method": scenario._equilibration,
        "routes.method": manifest.get("route_method"),
        "route_update.method": _route_update_name(manifest.get("route_update")),
    }
    if isinstance(scenario._choice_model, str):
        resolved["choice.model"] = scenario._choice_model
    return Config(
        name=run.get("run_id"),
        values=values,
        absent=absent + more,
        resolved={k: v for k, v in resolved.items() if v is not None},
    )


def _stated(
    parts: Mapping[str, Any],
    run: Mapping[str, Any],
    services_given: Mapping[str, Mapping[str, str]] | None = None,
) -> tuple[dict[str, Any], list[str]]:
    """The file's values of ``from_parts``' and ``Scenario.run``'s arguments, and what it lacks."""
    from openmobisim.operator import PolicyBuiltin
    from openmobisim.scenario import PRESETS

    values: dict[str, dict[str, Any]] = {}
    absent: list[str] = []

    def put(table: str, key: str, value: Any) -> None:
        values.setdefault(table, {})[key] = value

    for argument in ("flow_level", "flow_step_s", "master_seed", "default_weight", "link_bin_s"):
        if parts.get(argument) is not None:
            put("run", argument, parts[argument])
    window = parts.get("window")
    if window is not None:
        if any(w is None for w in window):
            absent.append("window (one end given)")
        else:
            put("run", "window", list(window))
    for argument in ("hub_join_m", "outputs", "threads", "design_id", "replication"):
        if run.get(argument) is not None:
            put("run", argument, run[argument])
    for argument, table, key in (
        ("choice_detour_limit", "choice", "choice_detour_limit"),
        ("choice_max_routes", "choice", "choice_max_routes"),
        ("max_cost_vectors", "routes", "max_cost_vectors"),
        ("bike_cost", "routes", "bike_cost"),
        ("bike_route_choice", "routes", "bike_route_choice"),
        ("modes", "modes", "modes"),
    ):
        if parts.get(argument) is not None:
            put(table, key, list(parts[argument]) if key == "modes" else parts[argument])
    preset = PRESETS.get(parts.get("preset") or "", {})
    for table, (key, margument, _, oargument, _) in _METHODS.items():
        method = parts.get(margument)
        if isinstance(method, str):
            put(table, key, method)
        elif method is not None:
            absent.append(f"{margument} (a Python object)")
        for k, v in {**preset.get(oargument, {}), **(parts.get(oargument) or {})}.items():
            put(table, k, v)
    for k, v in (parts.get("bike_route_options") or {}).items():
        put("routes", _BIKE + k, v)
    for table, oargument in _OPTIONS.items():
        if table in ("network", "service_assumptions", "policy"):
            continue
        for k, v in {**preset.get(oargument, {}), **(parts.get(oargument) or {})}.items():
            put(table, k, v)
    for k, v in (parts.get("network_options") or {}).items():
        put("network", k, v)
    for service in parts.get("services") or ():
        entry: dict[str, Any] = {"kind": service.kind}
        tables = (services_given or {}).get(service.name, {})
        entry.update(tables)
        for t in _SERVICE_TABLES[service.kind]:
            if t not in tables and getattr(service, t) is not None:
                absent.append(f"services.{service.name}.{t} (in memory)")
        known = {x.key for x in _lines()["services"] if x.kind == service.kind}
        for k, v in service.options.items():
            if k in known:
                entry[k] = v
            else:
                absent.append(f"services.{service.name}.{k}")
        values.setdefault("services", {})[service.name] = entry
    for k, v in (run.get("service_assumptions") or {}).items():
        put("service_assumptions", k, v)
    policy, options = run.get("policy"), run.get("policy_options")
    if isinstance(policy, PolicyBuiltin):
        put("policy", "name", policy.name)
        for k, v in policy.parameters.items():
            put("policy", k, v)
    elif isinstance(policy, str):
        put("policy", "name", policy)
        for k, v in (options or {}).items():
            put("policy", k, v)
    elif policy is not None:
        absent.append("policy (a Python object)")
    for argument in _NOT_EXPRESSIBLE:
        if parts.get(argument):
            absent.append(argument)
    return values, absent


def _route_update_name(name: Any) -> Any:
    from openmobisim.scenario import RENAMED_ROUTE_UPDATES

    return RENAMED_ROUTE_UPDATES.get(name, name) if isinstance(name, str) else name


def _network_read(path: str, options: Mapping[str, float]) -> _core.Network:
    """The network at ``path``, by its kind, with ``options`` where its reader takes them."""
    p = Path(path)
    if p.is_dir() and (p / "case.json").exists():
        from openmobisim._cases import Case

        network = Case(p.name, p).network()
    elif p.is_dir():
        from openmobisim.gmns import network_read_gmns

        return network_read_gmns(str(p), network_options=dict(options) or None)
    elif p.name.endswith(".pbf"):
        return _core.network_read_osm(str(p), network_options=dict(options) or None)
    else:
        network = _core.network_read_cache(str(p))
    _check_network(network, options, f"the network {str(p)!r}, read as it was built")
    return network


def _check_network(network: _core.Network, options: Mapping[str, float], what: str) -> None:
    """Refuse ``[network]`` values a network already read does not have."""
    own = dict(_core.network_options(network))
    differ = sorted(k for k, v in options.items() if k in own and own[k] != v)
    if differ:
        raise ValueError(
            f"run.toml [network] is applied when the network is read; {what} has other values "
            f"of {differ}: read it again with them, or leave them out of the file"
        )


def _parkings_read(path: str) -> _core.Parkings:
    if path.endswith((".csv", ".tsv")):
        from openmobisim.parking import parking_read_table

        return parking_read_table(path)
    if path.endswith(".pbf"):
        return _core.parking_read_osm(path)
    return _core.parking_read_cache(path)


def _same_inputs(scenario: Scenario, config: Config) -> None:
    """Refuse a file whose ``[inputs]`` are not the scenario's own."""
    given = config.inputs()
    if not given:
        return
    own = dict(getattr(scenario, "_inputs", {}))
    own.pop("services", None)
    parts = scenario._parts
    for key in ("demand", "persons", "zones"):
        if key not in own and isinstance(parts.get(key), str | Path):
            own[key] = str(Path(parts[key]).resolve())
    differ = [k for k, v in given.items() if own.get(k) != v]
    if differ:
        shown = {k: own.get(k, "given in memory") for k in differ}
        raise ValueError(
            f"run.toml [inputs] {differ} are not this scenario's ({shown}): a scenario runs on its "
            "own inputs. Remove them from the file, or run the file by itself "
            "(openmobisim run --config, or config_read(path).run())"
        )


def _with_services(scenario: Scenario, config: Config) -> list[Any] | None:
    """The scenario's services with the file's options, or ``None`` if the file states none."""
    from dataclasses import replace

    stated = config.values.get("services")
    if not stated:
        return None
    own = {s.name: s for s in scenario._parts.get("services") or ()}
    tables = config._service_inputs()
    recorded = getattr(scenario, "_inputs", {}).get("services", {})
    out = dict(own)
    for name, service in stated.items():
        if name not in own:
            raise ValueError(
                f"run.toml [services.{name}]: this scenario has no service {name!r} (its: "
                f"{sorted(own)}); a service is an input: build the scenario with it"
            )
        if own[name].kind != service["kind"]:
            raise ValueError(
                f"run.toml [services.{name}]: kind {service['kind']!r}, the scenario's is "
                f"{own[name].kind!r}"
            )
        for t, path in tables.get(name, {}).items():
            if recorded.get(name, {}).get(t) != path:
                raise ValueError(
                    f"run.toml [services.{name}] {t} is not this scenario's: a scenario runs on "
                    "its own inputs; remove it from the file"
                )
        options = dict(own[name].options)
        stated_options = {k: v for k, v in service.items()
                          if k != "kind" and k not in _SERVICE_TABLES[service["kind"]]}  # fmt: skip
        defaults = {x.key: x.default for x in _lines()["services"] if x.kind == service["kind"]}
        for k, v in stated_options.items():
            if v == defaults.get(k):
                options.pop(k, None)
            else:
                options[k] = float(v)
        out[name] = replace(own[name], options=options)
    return [out[s.name] for s in scenario._parts.get("services") or ()]


def _configured(scenario: Scenario, config: Config) -> Scenario:
    """``scenario`` with the file's values in place of its own (itself if none differs)."""
    _same_inputs(scenario, config)
    stated, _ = config._arguments()
    network_options = stated.pop("network_options", None)
    if network_options:
        _check_network(scenario._network, network_options, "this scenario's network")
    parts = scenario._parts
    changes = _scenario_arguments(stated, parts)
    services = _with_services(scenario, config)
    if services is not None:
        changes["services"] = services
    changes = {k: v for k, v in changes.items() if parts.get(k) != v}
    if not changes:
        return scenario
    new = scenario.replace(**changes)
    new._inputs = dict(getattr(scenario, "_inputs", {}))  # the file changes no input
    return new


def _given(call: Mapping[str, Any], defaults: Mapping[str, Any]) -> list[str]:
    """The arguments of a call given other than at their defaults."""
    return [
        k for k, v in call.items() if k in defaults and v is not defaults[k] and v != defaults[k]
    ]


def _run_with(scenario: Scenario, config: str | Path | Config, call: dict[str, Any]) -> Run:
    """``scenario.run(**call, config=config)``: the file's values, the call's other arguments."""
    import inspect

    from openmobisim.scenario import Scenario as _Scenario

    cfg = config if isinstance(config, Config) else config_read(config)
    signature = inspect.signature(_Scenario.run).parameters
    defaults = {k: p.default for k, p in signature.items() if k not in ("self", "config")}
    given = set(_given(call, defaults))
    _, run = cfg._arguments()
    run = _run_arguments(run)
    both = sorted(given & set(run))
    if cfg.name is not None and "run_id" in given:
        both.insert(0, "run_id (the file's name)")
    if both:
        raise ValueError(
            f"{both} given both in the call and in the file: give each in one place (the file's "
            "values are the run's arguments)"
        )
    target = _configured(scenario, cfg)
    arguments = {**call, **run}
    if cfg.name is not None:
        arguments["run_id"] = cfg.name
    return target.run(**arguments)
