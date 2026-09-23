"""openmobisim — an agent-based multimodal mobility simulator.

The public Python surface. Most of it is a thin re-export of the compiled
core (``openmobisim._core``), which is private: its shape is free to change behind
this module, whereas this module is a semver contract. ``Scenario``, ``Run``
and ``Table`` (``openmobisim.scenario``) and the fixture helpers in
``openmobisim.examples`` are where the core becomes ergonomic — Foundations §10's
"behaviour in Python, physics in Rust".

Figures live in ``openmobisim.viz`` (``pip install 'openmobisim[viz]'``); it is
imported on demand, so ``import openmobisim`` never needs matplotlib.

The scenario surface is intentionally narrow: a network, a trips table, one
run, its output artifacts, route choice among each trip's alternatives
(``openmobisim.choice`` says how to add your own choice model) and, by default
since the car-ready checkpoint (2026-09-23), iteration towards an equilibrium
(``equilibration="msa"``), during which the route sets may grow
(``route_update="best_response"``). No hubs, no
disruptions — those arrive with the mechanisms that give them meaning.
"""

from openmobisim import _core, choice, examples
from openmobisim._core import (
    build_info,
    choice_draw,
    choice_models,
    equilibration_strategies,
    fixed_order_sum_f64,
    network_read_osm,
    route_cache_clear,
    route_cache_info,
    route_methods,
    route_sets_build,
    route_update_methods,
    step_of,
)
from openmobisim.network import network_read_table
from openmobisim.scenario import Run, Scenario, Table

__version__: str = _core.__version__

__all__ = [
    "Run",
    "Scenario",
    "Table",
    "__version__",
    "build_info",
    "choice",
    "choice_draw",
    "choice_models",
    "equilibration_strategies",
    "examples",
    "fixed_order_sum_f64",
    "network_read_osm",
    "network_read_table",
    "route_methods",
    "route_cache_clear",
    "route_cache_info",
    "route_sets_build",
    "route_update_methods",
    "step_of",
]
