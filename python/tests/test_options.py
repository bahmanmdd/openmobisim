"""The typed option classes (D-5 a), generated from the core's parameter table, and in step.

``openmobisim/options.py`` is written by this file (``python python/tests/test_options.py
--write``) from ``_core.parameter_rows()``: one class per group, a field per parameter at its
shipped default. The test regenerates it in memory and compares, so a parameter added, removed or
re-defaulted in the core fails here until the file is rewritten.
"""

from __future__ import annotations

import sys
import textwrap
from pathlib import Path

import openmobisim as ms
import pytest
from openmobisim import _core
from openmobisim.parameters import _COUNTS, _FLAGS, _NOTES, _unit

TARGET = Path(ms.__file__).parent / "options.py"

#: Each group's class: (group, class name, the argument it is given as, what it is).
CLASSES = [
    ("network", "NetworkOptions", "network_options", "the network readers' parameters"),
    ("loading", "LoadingOptions", "loading_options", "the loading's parameters"),
    ("modes", "ModeOptions", "mode_options", "the modes' parameters"),
    ("transit", "TransitOptions", "transit_options", "public transport's parameters"),
    ("parking", "ParkingOptions", "parking_options", "the parkings' parameters"),
    ("prices", "PriceOptions", "price_options", "the prices"),
    ("equilibration.msa", "MsaOptions", "equilibration_options", '``equilibration="msa"``'),
    ("equilibration.sra", "SraOptions", "equilibration_options", '``equilibration="sra"``'),
    (
        "equilibration.free_flow",
        "FreeFlowOptions",
        "equilibration_options",
        '``equilibration="free_flow"``',
    ),
    (
        "equilibration.frank_wolfe",
        "FrankWolfeOptions",
        "equilibration_options",
        '``equilibration="frank_wolfe"`` (``flow_level=1`` only)',
    ),
    (
        "route_method.link_penalty",
        "LinkPenaltyOptions",
        "route_options",
        '``route_method="link_penalty"``',
    ),
    (
        "route_method.shortest",
        "ShortestOptions",
        "route_options",
        '``route_method="shortest"``',
    ),
    (
        "route_method.simulation",
        "SimulationOptions",
        "route_options",
        '``route_method="simulation"``',
    ),
    (
        "route_update.least_cost",
        "LeastCostOptions",
        "route_update_options",
        '``route_update="least_cost"``',
    ),
    ("choice_model.logit", "LogitOptions", "choice_options", '``choice_model="logit"``'),
    (
        "choice_model.nested_logit",
        "NestedLogitOptions",
        "choice_options",
        '``choice_model="nested_logit"``',
    ),
]

#: One line on what a class's method is, under its first line (S299: the terms of G's wording).
WHAT = {
    "ShortestOptions": "The least-cost route, and its equal-cost twin where it has one: a route "
    "that leaves it once and joins it again and costs at most ``twin_tolerance`` more.",
    "LinkPenaltyOptions": "The link penalty method (de la Barra, Pérez & Añez 1993): after each "
    "least-cost route, the costs of its links are multiplied by ``penalty`` and the search is "
    "repeated.",
    "SimulationOptions": "Stochastic simulation of link costs (Ramming 2002): the least-cost route "
    "under link costs drawn at random, ``draws`` times, the draws biased towards the links likely "
    "to congest.",
    "LeastCostOptions": "Column generation: after each loading, each pair's least-cost route at "
    "that loading's link times is added to its set when the set lacks it and it costs no more "
    "than the set's best.",
}

#: The old names of renamed classes (S299), kept as deprecated subclasses: old, new.
RENAMED = [
    ("PenaltyOptions", "LinkPenaltyOptions"),
    ("MonteCarloOptions", "SimulationOptions"),
    ("BestResponseOptions", "LeastCostOptions"),
]

HEADER = '''"""Typed options (C2, D-5 a): every parameter of a group as a keyword with its default.

Generated from the core's parameter table by ``python/tests/test_options.py --write``; do not edit
by hand. Each class is a read-only ``dict`` of the options given, so it goes wherever the options
dict goes: ``Scenario.from_parts(..., loading_options=LoadingOptions(reroute=1))`` is
``loading_options={{"reroute": 1}}``. An editor completes the names and shows the defaults, and a
misspelt name fails where it is written. A value equal to its default is left out (it changes
nothing); a field whose default is a rule of the run's level is ``None`` until set.
``parameters()`` lists every one with its unit, and the manual explains each.
"""

from __future__ import annotations

import warnings
from typing import Any, ClassVar

__all__ = [
{names}]


class _Options(dict):
    """A read-only dict of options, built by a class's keywords."""

    _argument: ClassVar[str] = ""
    _defaults: ClassVar[dict[str, Any]] = {{}}

    def _keep(self, given: dict[str, Any]) -> None:
        for name, value in given.items():
            if value is not None and value != self._defaults[name]:
                dict.__setitem__(self, name, value)

    def __getattr__(self, name: str) -> Any:
        if name in self._defaults:
            return self.get(name, self._defaults[name])
        raise AttributeError(f"{{type(self).__name__}} has no option {{name!r}}")

    def _read_only(self, *args: Any, **kwargs: Any) -> None:
        raise TypeError(f"{{type(self).__name__}} is read-only: make a new one")

    __setitem__ = __delitem__ = _read_only  # type: ignore[assignment]
    clear = pop = popitem = setdefault = update = _read_only  # type: ignore[assignment]

    def __repr__(self) -> str:
        given = ", ".join(f"{{k}}={{v!r}}" for k, v in self.items())
        return f"{{type(self).__name__}}({{given}})"


class LinkClassOptions(_Options):
    """One road class's parameters, for ``NetworkOptions`` (``motorway=LinkClassOptions(lanes=3)``).

    Each is ``None`` by default: the class's own shipped value, which ``parameters()`` lists (as
    ``motorway.lanes`` …). A class without one of them refuses it when the network is read.

    Attributes:
        free_flow_km_h: km/h.
        jam_density_veh_km_lane: veh/km per lane.
        lanes: count.
        saturation_flow_veh_h_lane: veh/h per lane.
    """

    _defaults: ClassVar[dict[str, Any]] = {{
        "free_flow_km_h": None,
        "jam_density_veh_km_lane": None,
        "lanes": None,
        "saturation_flow_veh_h_lane": None,
    }}

    def __init__(
        self,
        *,
        free_flow_km_h: float | None = None,
        jam_density_veh_km_lane: float | None = None,
        lanes: int | None = None,
        saturation_flow_veh_h_lane: float | None = None,
    ) -> None:
        """Keep the options given that differ from their defaults."""
        given = dict(locals())
        del given["self"]
        self._keep(given)
'''


def _number(value: float, whole: bool) -> str:
    return str(int(value)) if whole else repr(float(value))


def _default(group: str, name: str, value: float) -> str:
    """A field's default as written: ``None`` where it is a rule of the run's level."""
    return (
        "None" if _NOTES.get((group, name)) else _number(value, name in _COUNTS or name in _FLAGS)
    )


def generate() -> str:
    rows: dict[str, list[tuple[str, float]]] = {}
    for group, _argument, name, value in _core.parameter_rows():
        rows.setdefault(group, []).append((name, float(value)))
    for group in ("choice_model.logit", "choice_model.nested_logit"):  # the attributes' 0s
        listed = {n for n, _ in rows[group]}
        table = ms.parameters()
        for g, n, v in zip(table["group"], table["name"], table["value"], strict=True):
            if g == group and n not in listed:
                rows[group].append((n, float(v)))
    classes = sorted(
        [c for _, c, _, _ in CLASSES] + ["LinkClassOptions"] + [old for old, _ in RENAMED]
    )
    names = "".join(f'    "{cls}",\n' for cls in classes)
    out = [HEADER.format(names=names)]
    for group, cls, argument, what in CLASSES:
        fields = sorted(rows[group])
        link_classes = sorted({n.split(".")[0] for n, _ in fields if "." in n})
        fields = [(n, v) for n, v in fields if "." not in n]
        lines = [f"\n\nclass {cls}(_Options):"]
        lines.append(f'    """{what[0].upper() + what[1:]}: ``{argument}``.')
        lines.append("")
        if cls in WHAT:
            lines.extend(
                textwrap.wrap(
                    WHAT[cls], width=96, initial_indent=" " * 4, subsequent_indent=" " * 4
                )
            )
            lines.append("")
        lines.append("    Attributes:")
        for name, value in fields:
            whole = name in _COUNTS or name in _FLAGS
            note = _NOTES.get((group, name))
            shown = f"by default ``None``: {note}" if note else f"``{_number(value, whole)}``"
            lines.extend(
                textwrap.wrap(
                    f"{name}: {_unit(name)}; {shown}.",
                    width=100,
                    initial_indent=" " * 8,
                    subsequent_indent=" " * 12,
                )
            )
        for name in link_classes:
            lines.append(
                f"        {name}: the class's ``LinkClassOptions``; its own values by default."
            )
        lines.append('    """')
        lines.append("")
        lines.append(f'    _argument: ClassVar[str] = "{argument}"')
        if link_classes:
            lines.append(f"    _link_classes: ClassVar[tuple[str, ...]] = {tuple(link_classes)!r}")

        defaults = ", ".join(f'"{n}": {_default(group, n, v)}' for n, v in fields)
        defaults += "".join(f', "{n}": None' for n in link_classes)
        lines.append(f"    _defaults: ClassVar[dict[str, Any]] = {{{defaults}}}")
        lines.append("")
        lines.append("    def __init__(")
        lines.append("        self,")
        lines.append("        *,")
        for name, value in fields:
            whole = name in _COUNTS or name in _FLAGS
            kind = "int" if whole else "float"
            if _NOTES.get((group, name)):
                lines.append(f"        {name}: {kind} | None = None,")
            else:
                lines.append(f"        {name}: {kind} = {_number(value, whole)},")
        for name in link_classes:
            lines.append(f"        {name}: LinkClassOptions | None = None,")
        lines.append("    ) -> None:")
        lines.append('        """Keep the options given that differ from their defaults."""')
        lines.append("        given = dict(locals())")
        lines.append('        del given["self"]')
        if link_classes:
            lines.append("        for link_class in self._link_classes:")
            lines.append("            for name, value in (given.pop(link_class) or {}).items():")
            lines.append('                dict.__setitem__(self, f"{link_class}.{name}", value)')
        lines.append("        self._keep(given)")
        out.append("\n".join(lines))
    for old, new in RENAMED:
        out.append(
            f"""


class {old}({new}):
    \"\"\"The old name of ``{new}``, still accepted with a ``DeprecationWarning``.\"\"\"

    def __init__(self, **options: Any) -> None:
        \"\"\"``{new}(**options)``, warning that the name has changed.\"\"\"
        warnings.warn(
            "{old} is now {new}; the old name is still accepted",
            DeprecationWarning,
            stacklevel=2,
        )
        super().__init__(**options)"""
        )
    return "".join(out) + "\n"


def test_the_options_classes_are_the_core_s_parameter_table() -> None:
    from openmobisim import options

    table = ms.parameters()
    for group, cls, argument, _ in CLASSES:
        listed = {n for g, n in zip(table["group"], table["name"], strict=True) if g == group}
        klass = getattr(options, cls)
        link_classes = getattr(klass, "_link_classes", ())
        fields = {n for n in klass._defaults if n not in link_classes}
        assert {n for n in listed if "." not in n} == fields and {
            n.split(".")[0] for n in listed if "." in n
        } == set(link_classes), (
            f"{cls} differs from the core's {group}: rewrite options.py with "
            "python python/tests/test_options.py --write"
        )
        assert klass._argument == argument


def test_an_options_object_is_the_dict_it_stands_for() -> None:
    from openmobisim.options import LoadingOptions, MsaOptions

    assert LoadingOptions(reroute=1) == {"reroute": 1}
    assert LoadingOptions(reroute_after_s=300.0) == {}, "a default changes nothing"
    assert LoadingOptions(reroute=1).reroute_after_s == 300.0 and LoadingOptions().reroute is None
    assert MsaOptions(gap_tolerance=0.04) == {"gap_tolerance": 0.04}
    with pytest.raises(TypeError):
        LoadingOptions(reroute_afer_s=60)  # type: ignore[call-arg]
    with pytest.raises(TypeError, match="read-only"):
        LoadingOptions()["reroute"] = 1


def test_options_objects_and_dicts_make_the_same_run() -> None:
    from openmobisim.options import BestResponseOptions, NestedLogitOptions, ParkingOptions

    net = ms.examples.toy_network()
    rows = ms.examples.trips_random(net, 40, seed=3, min_m=100.0, max_m=2000.0, spread_s=1800)
    classes = ms.examples.traveller_classes()
    rows = ms.demand_assign_classes(rows, classes, seed=1)
    common = dict(
        classes=classes,
        transit=ms.examples.toy_network_transit(),
        parkings=ms.examples.toy_network_parkings(),
    )
    typed = ms.Scenario.from_parts(
        net,
        rows,
        route_update_options=BestResponseOptions(tree_step_s=1800),
        parking_options=ParkingOptions(reach_car_step_s=1800),
        choice_options=NestedLogitOptions(mu=0.7),
        **common,
    )
    plain = ms.Scenario.from_parts(
        net,
        rows,
        route_update_options={"tree_step_s": 1800},
        parking_options={"reach_car_step_s": 1800},
        choice_options={"mu": 0.7},
        **common,
    )
    assert typed._fingerprint() == plain._fingerprint()


def test_every_parameter_has_its_field_at_its_default() -> None:
    from openmobisim import options

    table = ms.parameters()
    by_group = {group: getattr(options, cls) for group, cls, _, _ in CLASSES}
    for group, name, value in zip(table["group"], table["name"], table["value"], strict=True):
        if group in by_group and "." not in name:
            default = by_group[group]._defaults[name]
            assert default is None or default == value, (group, name)


if __name__ == "__main__" and sys.argv[1:] == ["--write"]:
    import subprocess

    TARGET.write_text(generate())
    subprocess.run([sys.executable, "-m", "ruff", "format", str(TARGET)], check=False)
    print(f"written {TARGET}")
