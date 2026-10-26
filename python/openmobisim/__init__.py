"""openmobisim — an agent-based multimodal mobility simulator.

The public Python surface. Most of it is a thin re-export of the compiled
core (``openmobisim._core``), which is private: its shape is free to change behind
this module, whereas this module is a semver contract. ``Scenario``, ``Run``
and ``Table`` (``openmobisim.scenario``) and the fixture helpers in
``openmobisim.examples`` are where the core becomes ergonomic: behaviour in
Python, physics in Rust.

Figures live in ``openmobisim.viz`` (``pip install 'openmobisim[viz]'``); it is
imported on demand, so ``import openmobisim`` never needs matplotlib.

New here? ``openmobisim.notebooks()`` copies the tutorial notebooks into a folder and links the
first, ``00_getting_started.ipynb``, which lists the others (``openmobisim notebooks`` in a
terminal also opens it in JupyterLab).
Documentation: https://openmobisim.github.io

The scenario surface is intentionally narrow: a network, a trips table, one
run, its output artifacts, route choice among each trip's alternatives
(``openmobisim.choice`` says how to add your own choice model) and, by default,
iteration towards an equilibrium
(``equilibration="msa"``), during which the route sets may grow
(``route_update="least_cost"``). Trips may also cycle, walk, take scheduled
public transport (``transit_read_gtfs``, ``Scenario(transit=...)``), whose buses
ride the roads among the cars, or drive or cycle to a parking and go on by
transit (``parking_read_osm``, ``parking_read_table``, ``Scenario(parkings=...)``),
the station chosen by the choice model. ``demand_sample`` simulates one traveller for
every few people, for faster screening runs. Roads and lines can be disrupted at a time of day
(``Scenario.from_parts(..., disruptions=...)``).

Networks and demand may come from files in standard formats: GMNS networks
(``network_read_gmns``, ``network_write_gmns``), checked before a run by ``network_check``;
zones, OD matrices and trips as CSV (``demand_read_zones``, ``demand_read_zone_points``,
``demand_read_od``, ``demand_read_trips``, ``demand_write_trips``), an OD matrix made trips by
``demand_from_od``;
traveller classes, each with its own modes and coefficients, from a class table
(``demand_read_classes``), drawn for each traveller by ``demand_assign_classes``.
"""

from openmobisim import _core, choice, examples, options
from openmobisim._core import (
    Parkings,
    Transit,
    build_info,
    choice_draw,
    equilibration_strategies,
    fixed_order_sum_f64,
    network_read_cache,
    network_read_osm,
    network_write_cache,
    parking_read_cache,
    parking_read_osm,
    parking_write_cache,
    route_cache_clear,
    route_cache_info,
    route_methods,
    route_sets_build,
    route_update_methods,
    step_of,
    transit_read_gtfs,
)
from openmobisim._start import notebooks, pointer_on_import
from openmobisim.audit import network_check
from openmobisim.choice import choice_model_register, choice_models
from openmobisim.demand import (
    Trip,
    demand_aggregate_space,
    demand_aggregate_time,
    demand_assign_classes,
    demand_from_od,
    demand_read_classes,
    demand_read_od,
    demand_read_trips,
    demand_read_zone_points,
    demand_read_zones,
    demand_sample,
    demand_write_trips,
)
from openmobisim.gmns import network_read_gmns, network_write_gmns
from openmobisim.network import LINK_CLASSES, network_edit, network_options, network_read_table
from openmobisim.options import (
    BestResponseOptions,
    FrankWolfeOptions,
    FreeFlowOptions,
    LeastCostOptions,
    LinkClassOptions,
    LinkPenaltyOptions,
    LoadingOptions,
    LogitOptions,
    ModeOptions,
    MonteCarloOptions,
    MsaOptions,
    NestedLogitOptions,
    NetworkOptions,
    ParkingOptions,
    PenaltyOptions,
    PriceOptions,
    SimulationOptions,
    SraOptions,
    TransitOptions,
)
from openmobisim.parameters import parameters
from openmobisim.parking import parking_read_table
from openmobisim.scenario import (
    MODES,
    PRESETS,
    FallbackWarning,
    NoAlternativeWarning,
    Run,
    Scenario,
    Table,
    WindowWarning,
)
from openmobisim.table import Columns
from openmobisim.transit import transit_edit

__version__: str = _core.__version__

__all__ = [
    "BestResponseOptions",
    "LeastCostOptions",
    "LinkPenaltyOptions",
    "Columns",
    "FallbackWarning",
    "FrankWolfeOptions",
    "FreeFlowOptions",
    "LINK_CLASSES",
    "LinkClassOptions",
    "LoadingOptions",
    "LogitOptions",
    "MODES",
    "ModeOptions",
    "MonteCarloOptions",
    "SimulationOptions",
    "MsaOptions",
    "NestedLogitOptions",
    "NetworkOptions",
    "NoAlternativeWarning",
    "PRESETS",
    "ParkingOptions",
    "Parkings",
    "PenaltyOptions",
    "PriceOptions",
    "Run",
    "Scenario",
    "SraOptions",
    "Table",
    "Transit",
    "TransitOptions",
    "Trip",
    "WindowWarning",
    "build_info",
    "choice",
    "choice_draw",
    "choice_model_register",
    "choice_models",
    "demand_aggregate_space",
    "demand_aggregate_time",
    "demand_assign_classes",
    "demand_from_od",
    "demand_read_classes",
    "demand_read_od",
    "demand_read_trips",
    "demand_read_zone_points",
    "demand_read_zones",
    "demand_sample",
    "demand_write_trips",
    "equilibration_strategies",
    "examples",
    "fixed_order_sum_f64",
    "network_check",
    "network_edit",
    "network_options",
    "network_read_cache",
    "network_read_gmns",
    "network_read_osm",
    "network_read_table",
    "network_write_cache",
    "network_write_gmns",
    "notebooks",
    "options",
    "parameters",
    "parking_read_cache",
    "parking_read_osm",
    "parking_read_table",
    "parking_write_cache",
    "route_cache_clear",
    "route_cache_info",
    "route_methods",
    "route_sets_build",
    "route_update_methods",
    "step_of",
    "transit_edit",
    "transit_read_gtfs",
    "__version__",
]

pointer_on_import()
