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

#: One clause per parameter for the file's comments, by ``table.key`` (a service's as
#: ``services.<kind>.key``), else by ``key``. ``test_config`` checks every parameter has one.
_MEANING: dict[str, str] = {
    # run
    "flow_level": "how vehicles load the network: 0 free flow, 1 static, 2 the point queue, 3 a "
    "spatial queue, 4 the full link transmission model",
    "flow_step_s": "the loading's time step at flow levels 2 to 4",
    "window": "the simulation window: its start and end as clock times",
    "default_weight": "how many people one simulated traveller stands for, where a trip gives none",
    "master_seed": "the seed of every random draw",
    "hub_join_m": "how far a parking may be from a stop to join its hub in the outputs (not in "
    "the fingerprint)",
    # choice
    "choice_detour_limit": "a route is offered only if at most this share slower than the best",
    "choice_max_routes": "the most routes a traveller is offered (0: every route within the limit)",
    "model": "how each trip picks an alternative: logit, nested_logit, deterministic or a "
    "registered model",
    "beta_time_min": "weighs the expected time, door to door",
    "beta_wait_min": "weighs minutes waiting at stops, on top of the time",
    "beta_walk_min": "weighs minutes walking, on top of the time",
    "beta_cost_eur": "weighs what an alternative costs",
    "beta_transfers": "weighs each change of vehicle",
    "beta_ln_path_size": "weighs the logarithm of the path-size factor: routes sharing links "
    "split their share",
    "beta_bike_mixed_km": "weighs each km of a bike leg in mixed traffic, on top of its time",
    "beta_length_km": "weighs a route's length",
    "beta_detour": "weighs an alternative's time over the best alternative's, minus one",
    "beta_overlap": "weighs a route's largest share of cost shared with an earlier route",
    "beta_n_links": "weighs a route's number of links",
    "beta_car_min": "weighs minutes driving",
    "beta_bike_min": "weighs minutes riding a bike",
    "beta_ride_min": "weighs minutes on board transit vehicles",
    "beta_parking_min": "weighs minutes parking or fetching a car or bike",
    "beta_bike_separated_km": "weighs a bike leg's km on separate tracks",
    "beta_bike_lane_km": "weighs a bike leg's km on painted lanes or cycle streets",
    "beta_walk_access_min": "weighs minutes walking to the first stop",
    "beta_walk_egress_min": "weighs minutes walking from the last stop",
    "beta_walk_transfer_min": "weighs minutes walking between stops in a change",
    "beta_wait_first_min": "weighs minutes waiting at the first stop",
    "beta_wait_transfer_min": "weighs minutes waiting at changes",
    "beta_ride_rail_min": "weighs minutes on board rail services",
    "beta_ride_metro_min": "weighs minutes on board metro services",
    "beta_ride_tram_min": "weighs minutes on board trams",
    "beta_ride_bus_min": "weighs minutes on board buses",
    "beta_ride_ferry_min": "weighs minutes on board ferries",
    "beta_ride_other_min": "weighs minutes on board other transit services",
    "beta_cost_running_eur": "weighs a car or bike leg's running cost",
    "beta_cost_toll_eur": "weighs a car leg's tolls",
    "beta_cost_parking_eur": "weighs the parking fee",
    "beta_cost_fare_eur": "weighs a transit journey's fare",
    "beta_trip_departure_h": "weighs the departure hour (alone it changes no probability)",
    "mu": "the nested logit's nest scale, above 0 and at most 1 (nested_logit only)",
    # modes
    "modes.walk_max_s": "the longest walk offered to a trip choosing its mode",
    "bike_max_s": "the longest bike ride offered to a trip choosing its mode",
    "fallback_car": "1: a trip with no alternative at all travels by car (experimental)",
    "car_access_s": "from the door to the car, setting off",
    "car_egress_s": "from the parked car to the door",
    "bike_access_s": "from the door to the bike, unlocked",
    "bike_egress_s": "from the parked bike to the door",
    "car_snap_motorway_m": "how close to a motorway-only node a trip's end must be to join the "
    "roads there",
    # prices
    "car_eur_km": "what driving costs per km (fuel)",
    "bike_eur_km": "what riding one's own bike costs per km",
    "fare_base_eur": "a transit journey's fare, once per journey",
    "fare_km_eur": "a transit fare per km ridden, as the crow flies",
    "fare_transfer_eur": "a transit fare per change of vehicle",
    "parking_car_eur": "a car park's fee per stay, where its table gives none",
    "parking_bike_eur": "a bike parking's fee per stay, where its table gives none",
    "bike_share_eur_per_ride": "a shared bike's fare per ride",
    "bike_share_eur_per_min": "a shared bike's fare per minute ridden",
    "taxi_eur_per_ride": "a taxi's fare per ride",
    "taxi_eur_per_km": "a taxi's fare per km",
    # services
    "kind": '"taxi", an electric taxi fleet, or "bike_docked", docked bike sharing',
    "docks": "the docks: dock_id, lon, lat, capacity (name, hub_id, bikes optional)",
    "vehicles": "the vehicles: vehicle_id, lon, lat (soc optional)",
    "chargers": "the chargers: charger_id, lon, lat, plugs (kw, hub_id optional); none: they "
    "never charge",
    "services.bike_docked.walk_max_s": "the longest walk to a dock, or from one to the destination",
    "services.bike_docked.candidates": "the most dock pairs a trip chooses among",
    "services.bike_docked.snap_m": "the farthest a dock may be from its layers' nearest node",
    "redirect_max": "the most other docks a traveller tries when theirs is empty or full",
    "redirect_max_m": "how far from the planned dock a traveller looks for another",
    "pickup_s": "the time to take a bike from a dock",
    "return_s": "the time to dock a bike",
    "ride_factor": "a shared bike's ride time over the bike layer's (below 1: e-bikes)",
    "start_share": "the share of a dock's slots holding a bike when the day starts, where its "
    "table gives no count",
    "fail_s": "what finding no bike near a dock costs a pickup, assumed",
    "services.bike_docked.bin_s": "the time bins dock counts and costs are kept in",
    "handling_s": "an operator's move's time per bike, on top of the drive",
    "van_bikes": "the most bikes an operator's van carries in one move",
    "battery_kwh": "a vehicle's battery",
    "kwh_per_km": "a vehicle's energy use per km driven",
    "soc_start": "a vehicle's charge when the day starts, where its table gives none",
    "reserve_soc": "the charge a vehicle keeps after a trip: a request that would end below it "
    "is not sent",
    "services.taxi.charge_below": "a vehicle dropping a passenger below this charge drives to "
    "charge",
    "services.taxi.charge_to": "the charge a vehicle charges to",
    "charger_kw": "a charger's power per plug, where its table gives none",
    "request_cancel_s": "a request no taxi can pick up within it is cancelled",
    "boarding_s": "the time to get in at the pickup",
    "dispatch_delay_s": "from the match to the taxi setting off",
    "services.taxi.snap_m": "the farthest a vehicle or charger may be from the roads",
    "charger_reach_s": "the farthest, by free-flow time, a vehicle is sent to charge",
    # service assumptions
    "assumed_wait_s": "the waiting time travellers assume before a taxi picks them up",
    "assumed_no_taxi_share": "the chance of getting no taxi travellers assume",
    "assumed_extra_walk_at_pickup_s": "the extra walk a shared-bike user assumes if the pickup "
    "dock is empty",
    "assumed_extra_walk_at_return_s": "the extra walk a shared-bike user assumes if the return "
    "dock is full",
    # policy
    "name": "an operator steering the services in the run's service day",
    "step_s": "the time between the policy's steps",
    "from_s": "the second of the day the reset starts acting at",
    "low": "below this share of its slots, a dock is topped up",
    "high": "above this share of its slots, a dock gives bikes away",
    "policy.charge_below": "below this charge, an idle vehicle is sent to charge",
    "policy.charge_to": "the charge it charges to",
    # network
    "bike_dedicated_km_h": "a bike's speed on a lane, cycle street or track",
    "bike_lane_cost_factor": 'with bike_cost "dedicated", multiplies a painted lane\'s time in '
    "the search",
    "bike_mixed_cost_factor": 'with bike_cost "dedicated", multiplies a mixed-traffic link\'s '
    "time in the search",
    "bike_mixed_km_h": "a bike's speed in mixed traffic, stops included",
    "capacity_factor": "multiplies every road link's capacity",
    "control_delay_factor": "multiplies the control delay at signals",
    "ferry_km_h": "a ferry's speed where its way gives no duration",
    "ferry_wait_s": "the expected wait for a ferry",
    "free_flow_speed_factor": "multiplies every road link's free-flow speed",
    "green_fraction_factor": "multiplies signal_green_fraction",
    "jam_density_factor": "multiplies every road link's jam density",
    "signal_cycle_s": "every signal's cycle length",
    "signal_degree_of_saturation": "the degree of saturation the signals' control delay is "
    "computed at",
    "signal_green_fraction": "the effective green time as a share of the cycle",
    "signal_merge_m": "how far along its street a signal belongs to a junction",
    "walk_km_h": "the walking speed",
    # loading
    "priority": "1: priority at unsignalised merges by road class",
    "reroute": "1: en-route rerouting of vehicles blocked at a link's downstream end",
    "reroute_after_s": "how long a blocked vehicle waits before it re-routes",
    "reroute_max": "the most times one trip re-routes",
    "reroute_min_gain": "how much faster, as a share, the new route must be",
    "pocket_length_m": "turn pockets per lane on approaches of two lanes or more",
    "node_inflow": "1: at the point queue, entering the next link waits for its inflow headway",
    "origin_inflow": "1: a departure enters its first link no faster than its capacity",
    "reroute_tree_s": "how long one rerouting search tree serves the next vehicles there",
    "period_s": "flow level 1: the period the demand's hourly flows are of (0: from the "
    "departures)",
    "bpr_alpha": "flow level 1: the volume-delay function's alpha",
    "bpr_beta": "flow level 1: the volume-delay function's beta",
    # transit
    "board_slack_s": "how long before a vehicle leaves a passenger must be at the stop",
    "max_rides": "the most vehicles one journey takes",
    "access_walk_max_s": "the longest walk to or from a stop",
    "journey_max_s": "the longest journey with transit offered, door to door",
    "transfer_walk_max_s": "the longest walk between two stops in a change",
    "stop_walk_snap_m": "the farthest a stop may be from the walk layer",
    "stop_transfer_s": "the time a change at a stop takes between its access points",
    "buses_on_roads": "1: buses ride the roads among the cars; 0: they keep to the timetable",
    "bus_dwell_s": "how long a bus stays at each stop",
    "bus_pcu": "passenger car units a bus counts for",
    "bus_plausibility_ratio": "a bus pattern slower than its schedule by more runs by the schedule",
    "bus_stop_snap_m": "the farthest a bus stop may be from the roads for its buses to ride them",
    # parking
    "parking.walk_max_s": "the longest walk from a parking to a stop",
    "reach_car_s": "the longest drive to a park-and-ride car park",
    "reach_bike_s": "the longest ride to a bike parking for bike-and-ride",
    "parking.candidates": "how many parkings a park-and-ride or bike-and-ride trip weighs",
    "rank_speed_km_h": "the speed candidate parkings are ranked by, straight line to the "
    "destination",
    "slope_car_s": "the time a car park full for a whole bin adds to parking",
    "slope_bike_s": "the time a bike parking full for a whole bin adds to parking",
    "parking.snap_m": "the farthest a parking may be from its layers' nearest node",
    "parking.bin_s": "the time bins parking availability is measured in",
    "pr_min_km": "the shortest trip, as the crow flies, offered park-and-ride and bike-and-ride",
    "reach_car_step_s": "for computation: park-and-ride drives shared within steps of this length",
    # equilibration
    "method": "the method",
    "iterations": "the most loadings",
    "gap_tolerance": "stop when the deterministic part of the duality gap is below it",
    "cost_bin_s": "the time bins link times are recorded in for the next choice",
    "gap_every": "1: the full gap after every loading; 0: after the last",
    "gap_sample": "how many trips are tested against the whole network for the network gap",
    "increments": "the groups the free-flow loading is built up in",
    "itinerary_gap_sample": "how many itinerary trips are tested for the itinerary gap",
    "route_growth_tolerance": "stopping needs the route update to grow at most this share of pairs",
    "warmup": "how many first loadings use the point queue",
    "eta_fell": "how much the step's divisor grows after a loading whose disequilibrium fell",
    "eta_rose": "how much the step's divisor grows after a loading whose disequilibrium rose",
    # routes and the route update
    "max_cost_vectors": "the most coefficient vectors route searches are made by",
    "max_attempts": "the most searches for one origin-destination pair",
    "max_detour": "no route kept costs more than this multiple of the pair's best",
    "max_overlap": "a new route may share at most this share of its cost with each kept",
    "max_paths": "the most routes kept for one pair",
    "penalty": "the factor a found route's links' cost is multiplied by before the next search",
    "draws": "how many random cost draws per pair",
    "seed": "which draws are made (independent of master_seed)",
    "sigma": "how far a link's cost may rise in a draw",
    "tau": "the spread of the preference draws",
    "twin_tolerance": "how much more than the least cost a tied route may cost, as a share",
    "biased": "1: a draw raises a link's cost by its propensity to congest",
    "max_routes": "the most routes a pair's set may hold",
    "searches": "least-cost searches per pair per iteration",
    "slack": "a pair is not searched while its best route is within this share of free flow",
    "tree_step_s": "departures rounded to this step share one search tree per origin",
}
#: The road classes whose rows nothing reads (their links are on the bike or walk layer).
_UNUSED_CLASSES = ("cycleway", "ferry", "footway", "pedestrian")
_CLASS_PARAMETERS = {
    "free_flow_km_h": "free-flow speed",
    "jam_density_veh_km_lane": "jam density per lane",
    "lanes": "lanes per direction",
    "saturation_flow_veh_h_lane": "capacity per lane",
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


def _meaning(table: str, key: str, kind: str | None = None) -> str:
    if table == "network" and "." in key:
        road, what = key.split(".", 1)
        if road in _UNUSED_CLASSES:
            return f"not read: {road} links are on the bike or walk layer"
        if road.startswith("street_"):
            return f"the capacity per lane at a signal in the {road[7:]} km/h speed band"
        article = "an" if road[0] in "aeiou" else "a"
        what = _CLASS_PARAMETERS.get(what, what)
        return f"the {what} of {article} {road} road where the data gives none"
    if (
        table == "routes"
        and key.startswith(_BIKE)
        and key not in ("bike_cost", "bike_route_choice")
    ):
        key = key[len(_BIKE) :]
    if key.startswith("beta_mode_"):
        return f"the constant of the {key[len('beta_mode_') :]} mode"
    for k in (f"services.{kind}.{key}" if kind else "", f"{table}.{key}", key):
        if k and k in _MEANING:
            return _MEANING[k]
    return ""


#: Registry rows whose default is a rule of the run, not their listed number: written as a
#: comment, passed only when stated.
_RULES = {
    ("loading", "reroute"): "on at flow_level 3 and 4, off at 2",
    ("scenario", "window"): "3 h from the earliest departure, rounded down to the quarter hour",
}
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
    "choice": _MEANING["model"],
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
    for group, name, value, unit, note in zip(
        rows["group"], rows["name"], rows["value"], rows["unit"], rows["note"], strict=True
    ):
        prefix, _, method = group.partition(".")
        if prefix in NOT_IN_A_RUN:
            continue
        note = "" if note == "0 unless set" or name == "hub_join_m" else note
        rule = _RULES.get((group, name), note.removeprefix("None: ") if math.isnan(value) else "")
        default: Any = None if rule else value
        if rule:
            note = ""
        if prefix == "scenario":
            table = _SCENARIO_TABLES[name]
            if name in _INTEGERS:
                default = int(value)
            line = _Line(table, name, name, default, unit, _meaning(table, name), rule, note=note)
        elif prefix == "service":
            line = _Line("services", name, "options", default, unit,
                         _meaning("services", name, method), rule, kind=method)  # fmt: skip
        elif prefix == "policy":
            line = _Line("policy", name, "policy_options", default, unit,
                         _meaning("policy", name), rule, method=method)  # fmt: skip
        elif prefix == "bike_route":
            key = _BIKE + name
            line = _Line("routes", key, "bike_route_options", default, unit,
                         _meaning("routes", key), rule)  # fmt: skip
        else:
            table = GROUP_TABLES[prefix]
            argument = _METHODS[table][3] if table in _METHODS else _OPTIONS[table]
            line = _Line(table, name, argument, default, unit, _meaning(table, name), rule,
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
    if isinstance(v, int):
        return str(v)
    if isinstance(v, float):
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
            for name in ("reset", "threshold", "charge_rule"):
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
            out.append(_entry("kind", kind, _meaning("services", "kind")))
            for t in _SERVICE_TABLES[kind]:
                if service.get(t) is not None:
                    out.append(_entry(t, service[t], _meaning("services", t)))
                else:
                    about = f"a path; {_meaning('services', t)} (this run's was in memory)"
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
_NOT_EXPRESSIBLE = ("link_values", "person_values", "trip_values", "disruptions")
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
        entry.update((services_given or {}).get(service.name, {}))
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
