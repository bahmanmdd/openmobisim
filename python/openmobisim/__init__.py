"""openmobisim — an agent-based multimodal mobility simulator.

The public Python surface. Most of it is a thin re-export of the compiled
core (``openmobisim._core``), which is private: its shape is free to change behind
this module, whereas this module is a semver contract. ``Scenario``, ``Run``
and ``Table`` (``openmobisim.scenario``) and the fixture helpers in
``openmobisim.examples`` are where the core becomes ergonomic: behaviour in
Python, physics in Rust.

Figures live in ``openmobisim.viz`` (``pip install 'openmobisim[viz]'``); it is
imported on demand, so ``import openmobisim`` never needs matplotlib.

The scenario surface is intentionally narrow: a network, a trips table, one
run, its output artifacts, route choice among each trip's alternatives
(``openmobisim.choice`` says how to add your own choice model) and, by default
since the car-ready checkpoint (2026-09-23), iteration towards an equilibrium
(``equilibration="msa"``), during which the route sets may grow
(``route_update="best_response"``). Trips may also cycle, walk, take scheduled
public transport (``transit_read_gtfs``, ``Scenario(transit=...)``), whose buses
ride the roads among the cars, or drive or cycle to a parking and go on by
transit (``parking_read_osm``, ``parking_read_table``, ``Scenario(parkings=...)``),
the station chosen by the choice model. ``demand_sample`` simulates one traveller for
every few people, for faster screening runs. No disruptions yet.

Networks and demand may come from files in standard formats: GMNS networks
(``network_read_gmns``, ``network_write_gmns``), checked before a run by ``network_check``;
zones, OD matrices and trips as CSV (``demand_read_zones``, ``demand_read_od``,
``demand_read_trips``, ``demand_write_trips``), an OD matrix made trips by ``demand_from_od``;
traveller classes, each with its own modes and coefficients, from a class table
(``demand_read_classes``), drawn for each traveller by ``demand_assign_classes``.
"""

from openmobisim import _core, choice, examples
from openmobisim._core import (
    Parkings,
    Transit,
    build_info,
    choice_draw,
    choice_models,
    equilibration_strategies,
    fixed_order_sum_f64,
    network_read_osm,
    parking_read_osm,
    route_cache_clear,
    route_cache_info,
    route_methods,
    route_sets_build,
    route_update_methods,
    step_of,
    transit_read_gtfs,
)
from openmobisim.audit import network_check
from openmobisim.demand import (
    demand_assign_classes,
    demand_from_od,
    demand_read_classes,
    demand_read_od,
    demand_read_trips,
    demand_read_zones,
    demand_sample,
    demand_write_trips,
)
from openmobisim.gmns import network_read_gmns, network_write_gmns
from openmobisim.network import network_edit, network_options, network_read_table
from openmobisim.parameters import parameters
from openmobisim.parking import parking_read_table
from openmobisim.scenario import MODES, Run, Scenario, Table

__version__: str = _core.__version__

__all__ = [
    "MODES",
    "Parkings",
    "Run",
    "Scenario",
    "Table",
    "Transit",
    "__version__",
    "build_info",
    "choice",
    "choice_draw",
    "choice_models",
    "demand_assign_classes",
    "demand_from_od",
    "demand_read_classes",
    "demand_read_od",
    "demand_read_trips",
    "demand_read_zones",
    "demand_sample",
    "demand_write_trips",
    "equilibration_strategies",
    "examples",
    "fixed_order_sum_f64",
    "network_check",
    "network_read_gmns",
    "network_edit",
    "network_options",
    "parameters",
    "network_read_osm",
    "network_read_table",
    "network_write_gmns",
    "parking_read_osm",
    "parking_read_table",
    "route_methods",
    "route_cache_clear",
    "route_cache_info",
    "route_sets_build",
    "route_update_methods",
    "step_of",
    "transit_read_gtfs",
]
