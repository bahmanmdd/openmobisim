"""``Scenario`` and what ``.run()`` returns.

A network, demand (an in-memory table or a ``trips.parquet`` path — the same
schema either way), a loading with queues at capacity (the point queue, ``flow_level=2``),
route choice among each trip's alternatives (a path-size ``"logit"``) and
iteration towards an equilibrium (``"msa"``), during which the route sets may
grow (``route_update="best_response"``) — all by default. Trips drive, cycle,
walk, take transit (``transit=``) or drive or cycle to a parking and go on by
transit (``parkings=``). Roads and lines can be disrupted at a time of day
(``disruptions=``).
"""

from __future__ import annotations

import tempfile
import warnings
from collections.abc import Mapping
from pathlib import Path
from typing import Any

from openmobisim import _core, choice
from openmobisim._log import logger
from openmobisim.demand import _class_table, _trips_checked, demand_read_trips
from openmobisim.run import MODES, Run
from openmobisim.table import Table

__all__ = [
    "PRESETS",
    "FallbackWarning",
    "NoAlternativeWarning",
    "Run",
    "Scenario",
    "Table",
    "WindowWarning",
]

#: Named settings for speed (D-5 a): ``"fast"`` is what the starter cases set: the
#: route searches shared by origin and half hour, and the park-and-ride drives likewise;
#: ``"complete"`` is the core's defaults, every search made on its own.
PRESETS: dict[str, dict[str, dict[str, float]]] = {
    "fast": {
        "route_update_options": {"tree_step_s": 1800},
        "parking_options": {"reach_car_step_s": 1800},
    },
    "complete": {},
}

#: The flow levels ``Scenario`` accepts: 0 is free flow
#: with no interaction between vehicles; 2, 3 and 4 are the link transmission
#: model as a point queue, a spatial queue and the full triangular diagram.
FLOW_LEVELS = (0, 2, 3, 4)
#: Each flow level's name, as figures and pages write it (C3, P-14: one place).
LEVEL_NAMES = {0: "free flow", 2: "point queue", 3: "spatial queue", 4: "full"}


class WindowWarning(UserWarning):
    """Trips of the demand departed outside the simulation window and were left out.

    They are not simulated and not counted in any result.
    """


class FallbackWarning(UserWarning):
    """Trips with no alternative at all travelled by car instead.

    Only with ``mode_options={"fallback_car": 1}`` (experimental, off by default).
    They are counted under the car; ``Run.fallback_car`` lists them.
    """


class NoAlternativeWarning(UserWarning):
    """Trips had no alternative at all and were not simulated.

    No mode they may use reached their destination within the limits. The warning says how
    many, of which classes, why each mode gave them none and what to change;
    ``Run.report_no_alternative()`` lists every one.
    """


def _clock_s(value: str | float, what: str) -> int:
    """Seconds after midnight from ``"07:30"``, ``"07:30:15"`` or a number of seconds."""
    if isinstance(value, str):
        parts = value.strip().split(":")
        if 2 <= len(parts) <= 3 and all(p.isdigit() for p in parts):
            h, m, s = (int(p) for p in (*parts, "0")[:3])
            if m < 60 and s < 60:
                return h * 3600 + m * 60 + s
        raise ValueError(
            f"window {what} {value!r}: give a clock time such as '07:30' or seconds after midnight"
        )
    seconds = float(value)
    if not (seconds >= 0 and seconds < float("inf")):
        raise ValueError(f"window {what} {value!r}: seconds after midnight, 0 or more")
    return round(seconds)


def _window(window: Any) -> tuple[int | None, int | None]:
    """A scenario's ``window`` as (start, end) seconds after midnight, ``None`` where not given."""
    if window is None:
        return None, None
    if isinstance(window, str) or len(window) != 2:
        raise ValueError(
            f"window {window!r}: give (start, end), for example ('07:30', '09:30') or "
            "(27000, 34200) in seconds after midnight"
        )
    start, end = (
        None if v is None else _clock_s(v, what)
        for v, what in zip(window, ("start", "end"), strict=True)
    )
    if start is not None and end is not None and end <= start:
        raise ValueError(f"window {window!r}: it ends before it starts")
    return start, end


BIKE_COSTS = ("dedicated", "time")


def _check_modes_can_run(modes: list[str], has_transit: bool, has_parkings: bool) -> None:
    """Refuse a mode asked for in ``modes=`` that the run has nothing to simulate it with.

    A mode left out silently would read as a result (its share is 0); the error names the mode and
    what it needs. Modes the classes imply are not checked: a class table may name them for
    scenarios with and without a timetable.
    """
    needs = {
        "transit": ("transit=…", has_transit),
        "car_transit": ("transit=… and parkings=…", has_transit and has_parkings),
        "bike_transit": ("transit=… and parkings=…", has_transit and has_parkings),
    }
    for mode in modes:
        what, present = needs.get(mode, ("", True))
        if not present:
            raise ValueError(f'modes: "{mode}" needs {what}; give it, or leave "{mode}" out')


def _link_values(
    network: _core.Network, given: dict[str, dict[str, Any]]
) -> dict[str, dict[str, list[float]]]:
    """The user's link values as the core takes them: per layer and name, a value per link.

    Each column is an array in the layer's link order, or a dict from link id to value (a link
    not in it taking 0). Its length, the layer and the ids are checked here; the names and the
    values by the core.
    """
    if not isinstance(given, dict):
        raise ValueError("link_values is {layer: {name: values}}")
    out: dict[str, dict[str, list[float]]] = {}
    for layer, columns in given.items():
        if layer not in ("road", "bike", "walk"):
            raise ValueError(
                f"link_values has no layer {layer!r}; the layers are road, bike and walk"
            )
        graph = network if layer == "road" else network.layer(layer)
        n = graph.link_count
        ids: dict[str, int] | None = None
        out[layer] = {}
        for name, values in columns.items():
            if isinstance(values, dict):
                if ids is None:
                    ids = {link: i for i, link in enumerate(graph.link_ids())}
                column = [0.0] * n
                for link, value in values.items():
                    if str(link) not in ids:
                        raise ValueError(
                            f"link_values {layer!r} {name!r}: the {layer} layer has no link "
                            f"{link!r} (see network.layer({layer!r}).link_ids())"
                        )
                    column[ids[str(link)]] = float(value)
            else:
                column = [float(v) for v in values]
                if len(column) != n:
                    raise ValueError(
                        f"link_values {layer!r} {name!r} has {len(column)} values; the {layer} "
                        f"layer has {n} links"
                    )
            out[layer][str(name)] = column
    return out


def _person_values(given: Mapping[str, Any]) -> dict[str, list[tuple[str, float]]]:
    """The user's traveller values as the core takes them: per name, ``(id, value)`` pairs.

    Each column maps a traveller id to a value: a dict, or anything with ``items()`` (a pandas
    Series indexed by id). A traveller not in it takes 0; an id the demand does not have is
    ignored (a sampled demand keeps a share of the people). The names and values are checked by
    the core.
    """
    if not isinstance(given, Mapping):
        raise ValueError("person_values is {name: {traveller_id: value}}")
    out: dict[str, list[tuple[str, float]]] = {}
    for name, values in given.items():
        if not hasattr(values, "items"):
            raise ValueError(
                f"person_values {name!r}: a mapping from traveller id to value (a dict or a "
                "Series indexed by id)"
            )
        out[str(name)] = [(str(k), float(v)) for k, v in values.items()]
    return out


def _trip_values(
    given: Mapping[str, Any],
) -> tuple[dict[str, list[float]], dict[str, list[tuple[str, int, float]]]]:
    """The user's trip values as the core takes them: ``(aligned, keyed)``.

    A column is either a sequence with one value per row of the demand, in its order (a list, an
    array, a Series), or a dict from ``(traveller_id, trip_seq)`` to value, a trip not in it
    taking 0 and a key the demand does not have ignored. The lengths are checked by the core,
    which has the rows.
    """
    if not isinstance(given, Mapping):
        raise ValueError("trip_values is {name: values}")
    aligned: dict[str, list[float]] = {}
    keyed: dict[str, list[tuple[str, int, float]]] = {}
    for name, values in given.items():
        if isinstance(values, dict):
            column = []
            for key, value in values.items():
                if not (isinstance(key, tuple) and len(key) == 2):
                    raise ValueError(
                        f"trip_values {name!r}: a dict's keys are (traveller_id, trip_seq), "
                        f"got {key!r}"
                    )
                column.append((str(key[0]), int(key[1]), float(value)))
            keyed[str(name)] = column
        else:
            aligned[str(name)] = [float(v) for v in values]
    return aligned, keyed


def _disruptions(
    network: _core.Network, transit: _core.Transit | None, given: list[dict[str, Any]]
) -> tuple[list[tuple], list[tuple]]:
    """The disruptions as the core takes them: links and lines by index."""
    from openmobisim.transit import _line_index

    road: list[tuple] = []
    lines_of: list[tuple] = []
    ids: dict[str, int] | None = None
    lines = transit.lines() if transit is not None else None
    for k, d in enumerate(given):
        if not isinstance(d, dict):
            raise ValueError(f"disruption {k}: a dict, as Scenario's disruptions says")
        known = {"links", "capacity_factor", "line", "delay_s", "cancel", "from_s", "to_s"}
        unknown = set(d) - known
        if unknown:
            raise ValueError(
                f"disruption {k}: no such key {sorted(unknown)}; the keys: {sorted(known)}"
            )
        if "from_s" not in d or "to_s" not in d:
            raise ValueError(f"disruption {k}: from_s and to_s (seconds after midnight) are needed")
        start, end = float(d["from_s"]), float(d["to_s"])
        if "links" in d:
            if "line" in d or "capacity_factor" not in d:
                raise ValueError(f"disruption {k}: a road one has links and a capacity_factor")
            out = []
            for link in d["links"]:
                if isinstance(link, int):
                    out.append(link)
                    continue
                if ids is None:
                    ids = {name: i for i, name in enumerate(network.link_ids())}
                if str(link) not in ids:
                    raise ValueError(f"disruption {k}: the network has no link {link!r}")
                out.append(ids[str(link)])
            road.append((out, float(d["capacity_factor"]), start, end))
        elif "line" in d:
            if lines is None:
                raise ValueError(f"disruption {k}: a line's disruption needs transit=")
            cancel = bool(d.get("cancel", False))
            if cancel == ("delay_s" in d):
                raise ValueError(f"disruption {k}: a line's is either delay_s or cancel=True")
            delay = -1 if cancel else int(round(float(d["delay_s"])))
            if not cancel and delay < 0:
                raise ValueError(f"disruption {k}: delay_s is 0 or more")
            if start < 0:
                raise ValueError(f"disruption {k}: from_s is a second of the day")
            lines_of.append((_line_index(lines, str(d["line"])), delay, round(start), round(end)))
        else:
            raise ValueError(f"disruption {k}: give links (a road) or line (transit)")
    return road, lines_of


class Scenario:
    """A network, demand, and the settings of a run.

    Build with ``from_parts`` rather than the constructor directly.
    """

    def __init__(
        self,
        network: _core.Network,
        *,
        trips: list[tuple] | None = None,
        trips_path: str | None = None,
        persons: list[tuple] | None = None,
        persons_path: str | None = None,
        classes: dict[str, Any] | None = None,
        default_weight: int = 1,
        window: tuple[str | float | None, str | float | None] | None = None,
        flow_level: int = 2,
        flow_step_s: int = 300,
        link_bin_s: int | None = None,
        route_method: str | None = None,
        route_options: dict[str, float] | None = None,
        master_seed: int = 0,
        choice_model: str | Any | None = None,
        choice_options: dict[str, float] | None = None,
        equilibration: str = "msa",
        equilibration_options: dict[str, float] | None = None,
        route_update: str | None = None,
        route_update_options: dict[str, float] | None = None,
        choice_detour_limit: float | None = None,
        choice_max_routes: int | None = None,
        route_cache: bool = False,
        bike_cost: str = "dedicated",
        transit: _core.Transit | None = None,
        transit_options: dict[str, float] | None = None,
        parkings: _core.Parkings | None = None,
        parking_options: dict[str, float] | None = None,
        modes: tuple[str, ...] | list[str] | None = None,
        mode_options: dict[str, float] | None = None,
        loading_options: dict[str, float] | None = None,
        price_options: dict[str, float] | None = None,
        link_values: dict[str, dict[str, Any]] | None = None,
        person_values: dict[str, Any] | None = None,
        trip_values: dict[str, Any] | None = None,
        disruptions: list[dict[str, Any]] | None = None,
        disruptions_known: bool = False,
        preset: str | None = None,
    ) -> None:
        """Store the parts; prefer `from_parts` to calling this directly."""
        # The arguments as given, by ``from_parts``' names (``parts``, ``replace``; C2).
        given = dict(locals())
        del given["self"]
        trips_given, persons_given = given.pop("trips"), given.pop("persons")
        trips_path_given, persons_path_given = given.pop("trips_path"), given.pop("persons_path")
        self._parts = {
            "network": given.pop("network"),
            "demand": trips_given if trips_given is not None else trips_path_given,
            "persons": persons_given if persons_given is not None else persons_path_given,
            **given,
        }
        if preset is not None:
            if preset not in PRESETS:
                raise ValueError(f"preset must be one of {sorted(PRESETS)} or None, got {preset!r}")
            # The preset's settings, under any option given by name.
            chosen = PRESETS[preset]
            route_update_options = {
                **chosen.get("route_update_options", {}),
                **(route_update_options or {}),
            } or None
            parking_options = {
                **chosen.get("parking_options", {}),
                **(parking_options or {}),
            } or None
        if bike_cost not in BIKE_COSTS:
            raise ValueError(f"bike_cost must be one of {BIKE_COSTS}, got {bike_cost!r}")
        if trips is not None:
            trips = _trips_checked(trips)
        if flow_level not in FLOW_LEVELS:
            raise ValueError(f"flow_level must be one of {FLOW_LEVELS}, got {flow_level}")
        if flow_step_s <= 0:
            raise ValueError(f"flow_step_s must be positive, got {flow_step_s}")
        if link_bin_s is not None and link_bin_s <= 0:
            raise ValueError(f"link_bin_s must be positive, got {link_bin_s}")
        if not 0 <= master_seed < 2**64:
            raise ValueError(
                f"master_seed must be an integer from 0 to 2**64 - 1, got {master_seed}"
            )
        self._network = network
        self._trips = trips
        self._trips_path = trips_path
        self._persons = persons
        self._persons_path = persons_path
        # Traveller classes (S231): what each owns, the modes it may use, its coefficients.
        table = _class_table(classes) if classes is not None else {}
        self._class_defaults = {
            name: (c["owns_car"], c["owns_bike"], c["has_transit_pass"])
            for name, c in table.items()
        } or None
        self._class_modes = {
            name: c["modes"] for name, c in table.items() if c["modes"] is not None
        } or None
        self._class_options = {name: c["betas"] for name, c in table.items() if c["betas"]} or None
        self._class_limits = {name: c["limits"] for name, c in table.items() if c["limits"]} or None
        modes_asked = modes is not None
        if modes is None and self._class_modes:
            # The classes say which modes their travellers may use: the run offers them all.
            modes = [m for m in MODES if any(m in ms for ms in self._class_modes.values())]
        if isinstance(choice_model, str) and choice_model in choice._REGISTERED:
            # A Python model registered by name (C5): built from the options, which it reads
            # (they reach the fingerprint through its descriptor, not the core's options).
            choice_model = choice._choice_model_built(choice_model, choice_options)
            choice_options = None
        if choice_model is None:
            # Mode choice by the nested logit, one nest per mode, so a mode's share does not
            # grow with how many routes or journeys its nest holds; a run offering one mode
            # (or none) chooses routes by the path-size logit (S279, CM-2).
            choice_model = "nested_logit" if modes and len(modes) > 1 else "logit"
        self._default_weight = default_weight
        self._window = _window(window)
        self._flow_level = flow_level
        self._flow_step_s = flow_step_s
        self._link_bin_s = link_bin_s
        # One default table (S280, C1): every default below is the core's, listed by
        # ``parameters()``; the strategy's options, the route method and the route update are
        # passed as given (``None``: the core's rule — a run that loads more than once starts
        # from one route per pair and lets ``best_response`` grow the sets, S179, S223).
        equilibration_options = dict(equilibration_options or {}) or None
        self._route_method = route_method
        self._route_options = route_options
        self._master_seed = master_seed
        self._choice_model = choice_model
        self._choice_options = choice_options
        if choice_options and self._class_options:
            # A class's coefficients replace the run's by name: say so, or a study varying a
            # run-wide one that every class sets would see no effect and not know why.
            for name in sorted(choice_options):
                classes = sorted(c for c, betas in self._class_options.items() if name in betas)
                if classes:
                    whom = "every class" if len(classes) == len(table) else ", ".join(classes)
                    warnings.warn(
                        f"choice_options' {name} is replaced by the class's own for {whom}: a "
                        "class's coefficients win by name, so the run-wide value applies to the "
                        "other classes only.",
                        stacklevel=2,
                    )
        self._equilibration = equilibration
        self._equilibration_options = equilibration_options
        self._route_update = route_update
        self._route_update_options = route_update_options
        self._choice_detour_limit = choice_detour_limit
        self._choice_max_routes = choice_max_routes
        self._route_cache = route_cache
        self._bike_cost = bike_cost
        self._transit = transit
        self._transit_options = transit_options
        self._parkings = parkings
        self._parking_options = parking_options
        if modes is not None:
            modes = list(modes)
            unknown = [m for m in modes if m not in MODES]
            if unknown:
                raise ValueError(f"modes must be among {MODES}, got {unknown}")
            if modes_asked:
                _check_modes_can_run(modes, transit is not None, parkings is not None)
        self._modes = modes
        self._mode_options = mode_options
        # ``reroute`` unsaid is the core's rule: on at levels 3 and 4, off at the point queue,
        # where nothing waits for room (S229, S280).
        self._loading_options = dict(loading_options or {}) or None
        self._price_options = price_options
        self._link_values = _link_values(network, link_values) if link_values else None
        self._person_values = _person_values(person_values) if person_values else None
        self._trip_values, self._trip_values_keyed = (
            _trip_values(trip_values) if trip_values else (None, None)
        )
        self._road_disruptions, self._transit_disruptions = _disruptions(
            network, transit, disruptions or []
        )
        self._disruptions_known = bool(disruptions_known)

    @classmethod
    def from_parts(
        cls,
        network: _core.Network,
        demand: Any,
        persons: list[tuple] | str | None = None,
        classes: dict[str, Any] | None = None,
        default_weight: int = 1,
        window: tuple[str | float | None, str | float | None] | None = None,
        flow_level: int = 2,
        flow_step_s: int = 300,
        link_bin_s: int | None = None,
        route_method: str | None = None,
        route_options: dict[str, float] | None = None,
        master_seed: int = 0,
        choice_model: str | Any | None = None,
        choice_options: dict[str, float] | None = None,
        equilibration: str = "msa",
        equilibration_options: dict[str, float] | None = None,
        route_update: str | None = None,
        route_update_options: dict[str, float] | None = None,
        choice_detour_limit: float | None = None,
        choice_max_routes: int | None = None,
        route_cache: bool = False,
        bike_cost: str = "dedicated",
        transit: _core.Transit | None = None,
        transit_options: dict[str, float] | None = None,
        parkings: _core.Parkings | None = None,
        parking_options: dict[str, float] | None = None,
        modes: tuple[str, ...] | list[str] | None = None,
        mode_options: dict[str, float] | None = None,
        loading_options: dict[str, float] | None = None,
        price_options: dict[str, float] | None = None,
        link_values: dict[str, dict[str, Any]] | None = None,
        person_values: dict[str, Any] | None = None,
        trip_values: dict[str, Any] | None = None,
        disruptions: list[dict[str, Any]] | None = None,
        disruptions_known: bool = False,
        preset: str | None = None,
    ) -> Scenario:
        """Build a scenario from a network and demand.

        Args:
            network: Built by, for example, ``examples.manhattan_grid``.
            demand: The trips, in the schema of ``trips.parquet`` whatever the form: rows
                (``Trip`` named tuples, or plain tuples of the same fields, as
                ``examples.fixed_car_trips`` returns), a dict of columns or a pandas or polars
                data frame with those columns (``weight`` and ``mode`` optional), or a path to a
                ``trips.parquet`` file or a ``trips.csv`` (read by ``demand_read_trips``). Every
                row in memory is checked first, and a bad one is refused with its number and the
                field (a latitude outside ±90, a departure that is not whole seconds …). A row
                may end with
                a **mode** (the ``mode`` column of ``trips.parquet``): ``"car"``,
                ``"bike"``, ``"walk"``, ``"transit"``, ``"car_transit"`` (park-and-ride)
                or ``"bike_transit"`` (bike-and-ride; see ``parkings``). A trip without
                one is a car trip, or, with ``modes``, chooses one. Bike and walk trips
                travel on the network's bike and walk layers (``network.layer("bike")``),
                each by its shortest route
                there; a bike trip needs the traveller's bike at its origin, as a
                car trip needs their car. A transit trip needs ``transit``: it
                walks to a stop, rides and walks on (see ``transit``).
            persons: The ``persons.parquet`` equivalent — an in-memory table,
                a file path, or ``None`` if every traveller takes their
                class's default ownership.
            classes: The traveller classes, ``{class_name: columns}``, a trip's class being
                its ``user_class``: ``demand_read_classes`` reads them from a CSV file, or give a
                dict of the same columns. A class may say ``modes`` (the modes its travellers may
                use: ``["car", "transit", "car_transit"]``; unsaid: every mode the run offers),
                ``owns_car``, ``owns_bike``, ``has_transit_pass`` (what each of them owns, unless a
                ``persons`` row says otherwise; unsaid: a car for a class that may drive or park
                and ride, a bike for one that may cycle, both for one whose modes are unsaid, and
                no pass; **a traveller with a transit pass pays no fare**, whatever the
                ``price_options`` — the pass is money, never access: who may take transit is said
                by the modes, so give ``has_transit_pass`` as 1 for a class that holds one; its
                own price is not counted) and any
                ``beta_*`` coefficient of the choice model (its own mode
                constants, ``beta_mode_bike`` …, or its own value of time, ``beta_time_min``;
                unsaid: ``choice_options``'), for ``"logit"`` and ``"nested_logit"``, and its own
                choice-set limits in seconds: ``walk_max_s`` and ``bike_max_s``, the longest
                walk and ride offered to its trips choosing their mode, and ``access_walk_max_s``,
                the longest walk to or from a stop (unsaid: ``mode_options``' and
                ``transit_options``'; for example a class of keen cyclists riding up to 90 minutes,
                ``{"bike_max_s": 5400}``); its ``share`` is read only by ``demand_assign_classes``.
                The tuple ``(owns_car, owns_bike, has_transit_pass)`` is a class that says only what
                it owns. **When the classes say their modes and ``modes`` is not given, the run
                offers every mode a class names**, and a trip without a stated mode chooses among
                those its class may use. A class not listed (all of them, without ``classes``) may
                use every mode, with the model's coefficients, and owns a car, a bike and a transit
                pass unless a ``persons`` row says otherwise. A class with none of the run's
                modes leaves its choosing trips without an itinerary. Every value is the user's, not
                a calibration.
            default_weight: How many people a simulated traveller stands for,
                for a trip whose row gives no weight. 1 simulates everyone; a
                larger number is faster and coarser.
            window: The simulation window, ``(start, end)`` as clock times
                (``("07:30", "09:30")``) or seconds after midnight. The run starts at the start
                and stops at the end: a trip still under way then is unfinished, counted and
                reported; a timetable keeps only each run's calls inside the window. A trip
                departing outside it is **left out** — not simulated, not counted in any result
                — with a ``WindowWarning`` that says how many, when and how to widen the window.
                Not given (or ``None`` for either end): from the earliest departure, rounded down
                to the quarter hour, for three hours. Leave each trip time to finish: a demand
                whose departures end well before the window does (an hour, say) finishes inside it.
            flow_level: How vehicles load the network. ``2`` (the default) is the
                **point queue**: the link transmission model with unlimited storage. A queue
                forms at the end of a link, behind its capacity or that of the link ahead (a
                bottleneck in Vickrey's sense), and delays everyone behind it on that link,
                but takes no road space, so it never blocks the links upstream and nothing
                gridlocks. Equilibrium iterations converge steadily with it, and it is the
                cheapest loading with congestion: the right model for planning studies.
                ``Run.report_spillback()`` lists the links whose queue would not have fitted
                on the road. ``4`` is the full triangular fundamental diagram:
                queues take road space and **spill back** to the links upstream, with turn
                pockets and en-route rerouting against gridlock — for studies where spillback
                itself matters (traffic control, incidents, evacuation); under heavy congestion
                its iterations can drift. ``3`` is a spatial queue. ``0`` is free flow:
                no vehicle affects another, so there is no congestion, and a run that iterates
                has nothing to settle — for debugging, or as a lower bound. Under
                ``equilibration="free_flow"`` (the free-flow loading alone, no iterations) every
                level from 2 loads as the point queue (``equilibration_options`` ``warmup``, 1 by
                default), so levels 2, 3 and 4 differ only in whether stuck vehicles may re-route
                (``loading_options`` ``reroute``: off at 2, on at 3 and 4); the level sets how
                ``"msa"``'s iterations after it load.
            flow_step_s: The loading step in seconds, for levels 2-4. It is a
                bookkeeping boundary: results do not depend on it.
            link_bin_s: If given, also record per-link results in time bins of
                this many seconds, read with ``Run.link_bins()`` and drawn
                with ``openmobisim.viz.map_link``. It changes no result: the link times
                travellers choose on are binned at the equilibration's ``cost_bin_s``.
            route_method: How route sets are generated, by name (see
                ``openmobisim.route_methods()``). **The default depends on the run:** a run
                that loads once uses ``"penalty"``; a run that **loads more than once** (an
                ``equilibration`` with more than one iteration, or a free-flow loading built up
                in groups, the default at ``flow_level`` 2 to 4) starts from ``"shortest"``, one
                route per pair, and lets the route update find the rest (measured: as good
                as the alternatives of the other methods at about half the time). Naming a
                method, or giving ``route_options``, overrides this (options alone mean
                ``"penalty"``). ``"penalty"`` finds
                distinct alternatives by penalising the links of the routes found.
                ``"shortest"`` makes one route per pair. ``"montecarlo"`` finds routes
                by searching under random link costs **biased towards the links the
                demand is likely to congest** (the busiest half-hour's load on shortest
                routes over each link's capacity, from this scenario's trips and their
                weights): routes that go round what the demand will jam. It has been
                measured on few networks: better alternatives than ``"penalty"`` on one
                city's heavy load, worse on another's. Which route a trip takes among its
                alternatives is ``choice_model``'s business.
            route_options: The method's options, numbers by name; unknown names
                and out-of-range values are refused. For ``"penalty"``: ``max_paths``
                (5), ``max_detour`` (1.3), ``max_overlap`` (0.75), ``penalty`` (1.5),
                ``max_attempts`` (15). For ``"montecarlo"``: ``draws`` (16),
                ``sigma`` (4: how far a link's cost may rise, times its bias),
                ``max_detour`` (2), ``max_overlap`` (0.9), ``max_paths`` (10), ``seed``
                (0: another seed is another set of draws) and ``biased`` (1; 0 for no
                bias, which measured worse on the one heavy load it was tried on).
            master_seed: The one number that starts every random stream, so a
                run is reproduced by giving it the same one. Under a sampled
                choice model (the defaults, ``"nested_logit"`` and ``"logit"``) it decides who
                takes which route; under ``"deterministic"`` nothing draws from it
                and it changes only the run's fingerprint. A non-negative
                integer below 2**64.
            choice_model: How each trip picks a route from its pair's set: a
                name from ``openmobisim.choice_models()`` or an object with a
                ``choose(batch)`` method (see ``openmobisim.choice`` for how to
                write one). Unsaid: ``"nested_logit"`` when the run offers more than one mode
                (``modes``, or the classes name them), ``"logit"`` otherwise. ``"logit"`` is the
                standard path-size logit, sampled per traveller; ``"nested_logit"`` is the logit
                with the alternatives of each mode in a nest of their own, for mode choice: a
                mode's many routes do not outweigh another mode's one (under a flat logit the
                car's share grows with how many of its routes are offered), and within a nest
                the choice is sharper by ``1/mu`` — a stated car trip's route choice too, in
                a run that offers modes; ``"deterministic"`` sends
                everyone down the best route, with probability 1 — useful for debugging or
                an upper bound, not for a result to report.
            choice_options: A built-in model's options, numbers by name: for
                ``"logit"`` and ``"nested_logit"`` a coefficient per attribute,
                ``beta_time_min`` (default -0.2), ``beta_ln_path_size`` (1),
                ``beta_walk_min`` (-0.13) and ``beta_wait_min`` (-0.09, on top of the
                time), ``beta_transfers`` (-1), ``beta_bike_mixed_km`` (-0.16, a bike leg's
                kilometre in mixed traffic, on top of its time), ``beta_cost_eur`` (-1.2 per
                euro, 10 €/h: money, see ``price_options``), and 0 for the rest, such as
                ``beta_length_km``, ``beta_parking_min`` or the mode constants
                ``beta_mode_bike`` … (``openmobisim.choice.ROUTE_ATTRIBUTES`` lists them), and
                products of two attributes, ``beta_<a>*<b>`` (see ``trip_values``);
                for ``"nested_logit"`` also ``mu`` (0.5), the nests' scale, from above 0
                to 1 (1 is the logit). Unknown names and non-numbers are refused. The
                defaults are an assumption, not a calibration.
            choice_detour_limit: A **time-dependent choice set**: a traveller is offered only the
                routes of the pair's set whose expected time, at the times of the last loading
                (free flow for the first choice), is within this share of the best route's:
                ``0.5`` offers a route only if it is at most 50% slower than the best right now.
                A route far slower than the best is no realistic alternative, and in a logit it
                only takes probability that belongs to routes that compete (the gap grows with the
                size of the set). ``0`` offers every route of the set; ``None`` (the default) is the
                library's default, **0.5**. With mode choice it applies within each mode: a walk
                slower than a drive is still offered. The best route is always offered; a route
                left out is still in the set and returns when times change. The gaps are still
                measured against the whole set. The ``ln_path_size`` attribute is computed over
                the routes offered, so overlap with a route nobody is offered is no overlap.
            choice_max_routes: The most routes a traveller is offered: the fastest at their
                departure, of those within ``choice_detour_limit`` (people consider a
                handful of routes, Prato 2009, https://doi.org/10.1016/S1755-5345(13)70005-8;
                uncalibrated). ``None`` (the default) is the library's default, **5**; ``0``
                offers every route within the limit. A pair's set may hold more (the route
                update's ``max_routes``, 10), so a route not offered now can be later.
            bike_cost: How bike trips choose their route on the bike layer: ``"dedicated"``
                (the default) is the fastest route with every minute in mixed traffic
                counting a little more (1.2 times), so cyclists go a little out of their way
                for a cycle track or lane; ``"time"`` is the fastest route. Bikes ride at 15
                km/h in mixed traffic and 18 km/h on dedicated infrastructure; these values
                and the 1.2 are defaults, not a calibration.
            route_cache: Keep the generated route sets in memory for the next run **of this
                process** that asks for the same: the same network, the same origin-destination
                pairs, the same route method and options and, for a method that reads the demand
                (``"montecarlo"``), the same trips. Generating them is the largest single cost of
                a run that iterates, so a study that runs one scenario many times, changing only
                the equilibration, the choice model, the flow level or a seed, pays it once. Results
                do not depend on whether the cache was used. Holds at most four sets (about 17 MB at
                the scale of a country's network each); ``openmobisim.route_cache_clear()`` forgets
                them, ``openmobisim.route_cache_info()`` says ``(hits, misses, held)``.
            equilibration: How choice and loading are repeated (see
                ``openmobisim.equilibration_strategies()``). ``"msa"`` (the
                default) is the method of successive
                averages in traveller form: load the network, read the link
                times it produced, let a share ``1/(i + 1)`` of travellers
                choose again on those times at iteration ``i``, load again.
                ``"free_flow"`` makes only the free-flow loading: a quick
                estimate of the day without iterating.
                **The free-flow loading** is how every run starts (iteration 0).
                The travellers are split into groups of about equal size
                (``increments``, 5 by default); the first group chooses on
                free-flow costs, then the network is loaded with the groups so
                far and the next group chooses on the times that produced (its
                pairs searched for routes on those times first), until everyone
                has chosen and the whole demand is loaded. Each group sees the
                congestion of those before it, so routes spread before the
                first full loading (incremental assignment). It uses the
                point-queue model (``warmup``): queues delay traffic but take
                no space, so nothing spills back and nothing locks; the
                iterations that follow use ``flow_level``. At ``flow_level=0``
                there are no groups, since nothing congests.
                Either way, equilibration only changes anything when vehicles
                interact (``flow_level`` 2 to 4, the default 2) and a choice
                model gives travellers something to choose between
                (``"logit"``): at ``flow_level=0`` (free flow, no interaction),
                ``"msa"`` still runs but has nothing to disagree with itself
                about, so it settles immediately at the choice model's own
                floor. ``Run.convergence()`` says how
                the iterations went, and ``Run.convergence_verdict`` judges
                the disequilibrium the last iteration ended at.
            equilibration_options: The strategy's options, numbers by name: for
                ``"msa"``, ``iterations`` (10; the most loadings), ``gap_tolerance``
                (0.05: stop once the disequilibrium, averaged over the last three
                iterations, is below this, the verdict's "good"; 0.15 is acceptable; 0 never
                stops early), ``route_growth_tolerance`` (1: and only while the last route update
                added routes for at most this share of the pairs it searched; 1 lets the gap
                alone decide, since it is measured against the sets as grown),
                ``gap_sample`` (300; how many trips are tested against the whole
                network at the last iteration, 0 for none), ``itinerary_gap_sample``
                (100 000 000, that is every trip: how many transit, park-and-ride and
                bike-and-ride trips, besides those choosing again, plan their whole choice set
                after each loading to measure their gap; with fewer, the others that keep an
                itinerary out through a parking are re-costed at that parking, which is faster
                but makes the gap before the last iteration read low; the last iteration always
                plans every trip) and ``cost_bin_s`` (300: the
                length of the time bins the link times are read in). For both ``"msa"`` and
                ``"free_flow"``: ``increments`` (5 at ``flow_level`` 2 to 4, else 1: in how many
                groups the free-flow loading is built up; 1 lets everyone choose at once on
                free-flow costs) and ``warmup`` (1 at ``flow_level`` 2 to 4 or for a run that
                iterates, else 0: how many of the first loadings, the free-flow loading's groups
                included, use the point-queue model, which cannot gridlock; the last loading of a
                run that iterates, its result, always uses ``flow_level``, while a run of one
                loading is its free-flow loading. ``"free_flow"`` with ``increments`` 1 and
                ``warmup`` 0 is a single loading at ``flow_level`` on free-flow choices).
            route_update: How the route sets grow between iterations (see
                ``openmobisim.route_update_methods()``). **The default depends on the run:**
                ``"best_response"`` when the run loads more than once (it iterates, or builds
                its free-flow loading in groups; or ``route_update_options`` is given),
                ``"none"`` when it loads once. ``"none"`` leaves
                the sets as the route method made them, at free-flow costs. ``"best_response"``
                is for a run that loads more than once: after each loading it searches, for every
                origin-destination pair, the fastest route at the congested times the
                loading produced, and adds it to the pair's set if it is new and at least as
                fast as the best route already there, before travellers choose again. It
                closes the gap that ``Run.convergence()["gap_network"]`` shows when traffic
                has made a route worthwhile that no set held. Where nothing queues it adds
                little or nothing. It is for congestion that is heavy but not gridlock: near
                gridlock it can make a run worse. **The equilibrium is then over the sets the
                route method and the update produce**, which the run's fingerprint and
                manifest say.
            route_update_options: The update's options, numbers by name: for
                ``"best_response"``, ``searches`` (1: how many searches per pair per
                iteration, at spread quantiles of the pair's departures; more finds routes
                that pay only at some hours and makes bigger sets) and ``max_routes`` (10:
                the most routes a pair's set may hold; a full set is still searched, and a
                route at least a second faster than its best replaces a route nobody chose,
                never its free-flow shortest; if all are in use it waits) and
                ``slack`` (0.02: a pair is not searched while its set's best route is within this
                share of free flow, since nothing is faster than free flow and so at most that
                much is left to gain).
                Unknown names and out-of-range values are refused.
            transit: The timetable of a service day (``openmobisim.transit_read_gtfs``),
                for the ``"transit"`` trips. A transit trip walks (4.8 km/h, at most 30
                minutes, ``transit_options``' ``access_walk_max_s``) to a stop, rides one or more
                vehicles and walks on, boarding only if
                at the stop a minute before a vehicle leaves; walks between stops are at most
                5 minutes. **Its journey is chosen by** ``choice_model`` among the
                competitive ones: for each number of vehicles, the earliest arrival that beats
                every journey with fewer (the attributes ``walk_min``, ``wait_min``,
                ``ride_min`` and ``transfers`` say what it costs). It is chosen on the times
                expected — the schedule at first, the last iteration's after — and ridden on
                the times the run makes: the traveller boards the first vehicle of each line
                chosen they can catch. The run's clock counts seconds after the service day's
                midnight and starts at the window.
                **Buses ride the roads**, 2 PCU each, among the cars: routed stop to stop at
                free flow, dwelling 20 s at a stop without blocking the traffic behind, never
                leaving a stop before their scheduled time, so congestion makes them late
                and their passengers with them. A bus line the roads cannot carry plausibly
                runs by the schedule (``Run.transit_summary`` counts them). Rail, metro, trams
                and ferries run by the schedule. These values are defaults, not a calibration:
                ``transit_options`` overrides them.
            transit_options: Transit's parameters by name, each replacing its default:
                ``board_slack_s`` (60), ``max_rides`` (8), ``access_walk_max_s`` (1800),
                ``journey_max_s`` (7200: the longest journey with transit offered),
                ``transfer_walk_max_s`` (300), ``stop_walk_snap_m`` (300),
                ``stop_transfer_s`` (0), ``bus_dwell_s`` (20), ``bus_pcu`` (2),
                ``bus_plausibility_ratio`` (1.6), ``bus_stop_snap_m`` (300). Unknown names
                and out-of-range values are refused.
            parkings: Park-and-ride car parks and bike parkings
                (``openmobisim.parking_read_osm``, ``openmobisim.parking_read_table``), for
                the ``"car_transit"`` and ``"bike_transit"`` trips; needs ``transit``. **The
                parking is chosen by** ``choice_model``: a trip whose car (or bike) is at its
                origin drives (rides) to one of the parkings within reach, parks, walks to a
                stop and rides on; the alternatives are the parking × the route to it × the
                journey from it, and ``parking_min`` says what parking costs. A trip whose
                vehicle is parked goes back by transit to where it is, fetches it and drives
                (rides) on. **Soft capacity:** a full parking refuses nobody; parking there
                takes longer (``parking_options``), the overflow is counted
                (``Run.parking_bins()``), and the choice sees how full each parking was in
                the iterations so far.
            parking_options: Parking's parameters by name, each replacing its default:
                ``walk_max_s`` (300: the longest walk from a parking to a stop),
                ``reach_car_s`` (1800) and ``reach_bike_s`` (1200: the longest drive or ride
                to a parking), ``candidates`` (3: the most parkings a trip chooses among),
                ``rank_speed_km_h`` (30: ranks candidates before they are tried),
                ``floor_car_s`` (120) and ``floor_bike_s`` (30: the time to park when there
                is room, and to fetch), ``slope_car_s`` (600) and ``slope_bike_s`` (180: what
                a parking full for a whole time bin adds), ``snap_m`` (300), ``bin_s``
                (900) and ``pr_min_km`` (3: the shortest trip, as the crow flies, that mode
                choice offers park-and-ride and bike-and-ride to). Uncalibrated defaults.
                For compute, ``reach_car_step_s`` (0: each park-and-ride trip searches its
                drive to the car parks at its own departure; above 0, the trips leaving one
                road node within one step of this many seconds share one search, made at the
                step's middle, each trip's drive timed at its own departure).
                Unknown names and out-of-range values are refused.
            modes: **Mode choice.** The modes a trip without a stated mode chooses among,
                with its route, by ``choice_model``: any of ``"walk"``, ``"bike"``,
                ``"car"``, ``"transit"``, ``"car_transit"`` and ``"bike_transit"``. Each is
                offered where the traveller can use it: their car or bike where the trip
                starts (a car left at a car park is fetched on the way back), the walk and
                bike layers, the timetable (``transit``) and the parkings (``parkings``);
                park-and-ride and bike-and-ride only for trips longer than
                ``parking_options["pr_min_km"]``. A traveller's trips choose in order, since
                each may move a vehicle the next needs; a trip with a stated mode keeps it.
                With ``choice_model="nested_logit"`` the alternatives of a mode share a nest
                (``choice_options["mu"]``), and ``beta_mode_<mode>`` is a mode's constant (0
                by default: time decides). One mode is no choice: such trips take it
                (``("car",)`` is the run without ``modes``, exactly). ``None`` (the default):
                such trips are car trips. A walk or a ride is offered only up to
                ``mode_options``' times (30 minutes' walk, 60 minutes' ride), or the
                trip's class's (``classes``).
            mode_options: Mode choice's parameters by name, each replacing its default:
                ``walk_max_s`` (1800) and ``bike_max_s`` (3600), the longest walk and ride
                offered to a trip choosing its mode (a trip given the mode takes it at any
                length; a class may give its own, ``classes``); ``float("inf")`` offers
                every walk or ride. Uncalibrated defaults. ``fallback_car`` (0; experimental):
                1 sends a trip with no alternative at all by car (a lift or a taxi for a
                traveller without one), counted under the car and listed by
                ``Run.fallback_car``; at 0 such a trip is not simulated, and the run warns
                (``NoAlternativeWarning``) and lists it (``Run.report_no_alternative()``).
                ``car_access_s`` (120) and ``car_egress_s`` (180): what a car trip takes before
                its drive (reaching the car) and after it (parking and walking from it), counted
                in a car alternative's door-to-door time, walk and parking time in mode choice
                and in the drive to a park-and-ride car park — every other mode pays its walks
                and waits; uncalibrated, 0 switches either off. The loading moves the car from
                the trip's departure. ``car_snap_motorway_m`` (50): a trip's end joins the roads
                at the nearest street node (one with a road a car can enter from; motorways,
                trunk roads and their ramps are not), unless it lies within this distance of a
                node that has only such roads, which it then takes (through traffic placed on
                the motorway keeps its node; an address beside it does not start on it);
                uncalibrated, 0 sends every trip to a street, ``inf`` takes the nearest drivable
                node of any class. Unknown names and negative values are refused (the limits
                must be above 0).
            loading_options: The loading's rules by name, for ``flow_level`` 2–4 (gridlock
                remedies; at ``flow_level`` 2, where nothing waits for room, ``reroute`` is off
                unless asked for, except in a loading with road disruptions, where a closed or
                narrowed link holds vehicles and it is on): ``priority`` (0 or 1, off by
                default: without per-turn queues a vehicle giving way holds up everything
                behind it, which made locks worse) —
                at an unsignalised merge a vehicle gives way to an approach of higher road
                class, or of the same class and at least 1.5 times the capacity, whose front
                vehicle is bound for the same link, and a departing vehicle gives way to every
                approach; ``reroute`` (0 or 1, **on** by default at levels 3 and 4, and at 2
                under road disruptions) — a vehicle
                blocked at the front of its link for ``reroute_after_s`` (300) re-routes from
                where it is, at
                most ``reroute_max`` (3) times, if the new route is at least
                ``reroute_min_gain`` (0.1) faster; ``pocket_length_m`` (50 by default; 0 turns
                turn pockets off) — on an approach of two lanes or more, a vehicle passes those
                ahead of it that wait for another movement while they fit in their turn pockets,
                this many metres per lane, split among the approach's movements; on by default,
                because without them one queue per link locks a dense network in the full model
                (their known cost: an iterated run under heavy congestion misprices the waiting
                movements).
                Uncalibrated defaults; unknown names and values out of range are refused.
            price_options: What travel costs, in euros, by name: ``car_eur_km``
                (the fuel a driver pays per km), ``bike_eur_km``, a transit
                journey's fare ``fare_base_eur`` (once) + ``fare_km_eur`` (per km from
                each boarding stop to its alighting stop as the crow flies) +
                ``fare_transfer_eur`` (per transfer), and the fee per stay of a car park or
                bike parking whose table gives none (``parking_car_eur``,
                ``parking_bike_eur``; a parking table's ``fee_eur`` column gives each its
                own). **Every one is 0 by default**: having a car, a bike, a pass or
                the means to pay a fare is access, which ``classes`` give, and no money is
                counted for it; give prices here to weigh them. **A traveller whose class or
                person has a transit pass pays no fare** (``cost_fare_eur`` is 0), whatever the
                fare prices. A toll is a road link value
                named ``toll_eur`` (``link_values``), paid at each passage. Every alternative
                carries what it costs: ``cost_eur`` and its
                parts ``cost_running_eur``, ``cost_toll_eur``, ``cost_parking_eur`` (park-and-ride
                and bike-and-ride, on the trip that parks) and ``cost_fare_eur``. The built-in
                models weigh money by ``beta_cost_eur`` (or a part's coefficient), **-1.2 per euro
                by default** (0 drops it): a value of time of ``V`` euros per hour is
                ``beta_cost_eur = beta_time_min * 60 / V``, so -1.2 is 10 €/h at the default
                -0.2; a class's own ``beta_cost_eur`` (``classes``) is its own value of time.
                Uncalibrated defaults (a city's own, to weigh fares: a single ticket in Paris
                2.55 € in 2026,
                https://www.iledefrance-mobilites.fr/tarifs-titre-de-transport-en-commun-2026;
                in Lyon 2.10 € in 2025,
                https://www.lebonbon.fr/lyon/news/augmentation-prix-tcl-1er-janvier-2025/;
                Amsterdam's GVB 1.16 € plus 0.217 € per km in 2026, Vervoerregio Amsterdam's
                note BBV/2025/15361, p. 2,
                https://vervoerregio.bestuurlijkeinformatie.nl/Document/View/abddeeb0-be54-4eae-9622-4ac0935df151);
                unknown names and values below 0 are refused.
                Recorded in the fingerprint and the manifest when the choice model reads money.
            link_values: The user's own numbers per link, for choice models:
                ``{layer: {name: values}}``, the layer ``"road"``, ``"bike"`` or ``"walk"``,
                the values either one per link of that layer in link order (an array as long as
                ``network.layer("bike").link_count``, aligned with its other per-link arrays) or
                a dict from link id (``network.layer("bike").link_ids()``) to value, a link not
                in it taking 0. Names are lower-case letters, digits and ``_``. Each column
                becomes two attributes of every alternative with a leg on that layer:
                ``<layer>_<name>_km``, the sum of the value times each link's length in km
                (greenery along a route, divided by ``length_km`` for its mean), and
                ``<layer>_<name>_sum``, the plain sum (a toll, a count of crossings); 0 for an
                alternative with no leg there (a transit itinerary's walks to and from stops
                carry none). The built-in models weigh them through ``choice_options``
                (``{"beta_bike_greenery_km": 0.2}``), a model of one's own reads them from the
                batch. For example, the greenery around each bike link computed from
                OpenStreetMap, or the car flow on the road beside each bike link taken from a
                previous run's ``link_bins`` (``link_ids()`` ties a bike link to its street's
                road links).
                Recorded in the fingerprint, and their names in the manifest.
            person_values: The user's own numbers per traveller, for choice models:
                ``{name: {traveller_id: value}}`` (a dict, or a pandas Series indexed by id); a
                traveller not in it takes 0, and an id the demand does not have is ignored (a
                sampled demand keeps a share of the people). Each becomes the attribute
                ``person_<name>`` of every alternative of that traveller's choices: income, age,
                a household's children.
            trip_values: The same per trip: ``{name: values}``, the values one per row of
                ``demand`` in its order (a list, an array, a Series), or a dict from
                ``(traveller_id, trip_seq)`` to value (a trip not in it taking 0); the attribute
                ``trip_<name>``: a purpose, a time budget. **``parking_eur``** is what parking a car
                at the trip's destination costs, in euros: it is added to ``cost_parking_eur`` of
                every alternative that arrives by car (see ``price_options``). Beside them every
                alternative carries ``trip_departure_h``, the trip's departure in hours after
                midnight. Being the same for every alternative of a choice, such a value weighs in
                the built-in models **in a product with another attribute**, a coefficient named
                ``beta_<a>*<b>`` in ``choice_options`` or a class: ``beta_mode_car*person_age``, a
                car constant growing with age; ``beta_cost_eur*person_income_inv``, money weighing
                less as income grows (give ``income_inv`` as one over the income); a model of one's
                own reads them from the batch. Names are lower-case letters, digits and ``_``.
                Recorded in the fingerprint, and their names in the manifest.
            disruptions: Things that happen at a time of day, a list of dicts: on roads,
                ``{"links": [...], "capacity_factor": 0.0, "from_s": 8 * 3600, "to_s": 9 * 3600}``
                — the links (by id, ``network.link_ids()``, or index; each direction of a road
                is a link) keep that share of their capacity from ``from_s`` to ``to_s``
                (seconds after midnight): 0 closes them, nothing entering or leaving, 0.5
                halves it; overlapping ones multiply; on lines, ``{"line": "T1", "delay_s": 600,
                "from_s": ..., "to_s": ...}`` or ``{"line": "T1", "cancel": True, ...}`` — the
                line's runs leaving their first stop in that window are late by ``delay_s`` at
                every call (a bus on the roads leaves late and drives on among the cars) or do
                not run. Road disruptions act through capacity, so not at ``flow_level`` 0. At
                the point queue (``flow_level`` 2) a closure's queue takes no road space: it
                delays only the vehicles in it, blocks nobody behind it and, holding them back,
                eases the roads beyond; with spillback (``flow_level`` 3) it backs up across the
                junctions behind it, as a real one does.
            disruptions_known: Whether travellers know of the disruptions in advance. ``False``
                (the default: an accident, a breakdown): the run reaches its equilibrium without
                them, then loads the day once more with them, every choice kept — only cars
                stuck behind a closure re-route (at ``flow_level`` 3 and 4) and passengers whose
                run never comes plan again at the stop; that last loading is the run's result
                (the convergence report stays the undisrupted one's), so the impact is the
                **zero-adaptation** one. ``True`` (works announced, a strike): every loading has
                them and choices adapt over the iterations. Both are in the fingerprint and the
                manifest (``disruptions``).
            preset: Named settings for speed (``PRESETS``). ``"fast"`` is what the starter cases
                use: the route update's searches shared by origin and half hour
                (``route_update_options`` ``tree_step_s`` 1 800) and the park-and-ride drives
                likewise (``parking_options`` ``reach_car_step_s`` 1 800), slightly approximate
                and several times faster on a city. ``"complete"`` or ``None`` (the default): the
                core's defaults, every search made on its own. An option given by name wins over
                the preset's; the run's options, not the preset's name, are in the fingerprint.

        Returns:
            A ``Scenario``, ready to ``.run()``.
        """
        if isinstance(demand, Path):
            demand = str(demand)
        if isinstance(demand, str) and demand.lower().endswith(".csv"):
            demand = demand_read_trips(demand)
        trips = demand if not isinstance(demand, str) else None
        trips_path = demand if isinstance(demand, str) else None
        persons_rows = persons if not isinstance(persons, str) else None
        persons_path = persons if isinstance(persons, str) else None
        return cls(
            network,
            trips=trips,
            trips_path=trips_path,
            persons=persons_rows,
            persons_path=persons_path,
            classes=classes,
            default_weight=default_weight,
            window=window,
            flow_level=flow_level,
            flow_step_s=flow_step_s,
            link_bin_s=link_bin_s,
            route_method=route_method,
            route_options=route_options,
            master_seed=master_seed,
            choice_model=choice_model,
            choice_options=choice_options,
            equilibration=equilibration,
            equilibration_options=equilibration_options,
            route_update=route_update,
            route_update_options=route_update_options,
            choice_detour_limit=choice_detour_limit,
            choice_max_routes=choice_max_routes,
            route_cache=route_cache,
            bike_cost=bike_cost,
            transit=transit,
            transit_options=transit_options,
            parkings=parkings,
            parking_options=parking_options,
            modes=modes,
            mode_options=mode_options,
            loading_options=loading_options,
            price_options=price_options,
            link_values=link_values,
            person_values=person_values,
            trip_values=trip_values,
            disruptions=disruptions,
            disruptions_known=disruptions_known,
            preset=preset,
        )

    @property
    def parts(self) -> dict[str, Any]:
        """The arguments this scenario was built from, by the names of ``from_parts``: a copy.

        ``Scenario.from_parts(**scenario.parts)`` builds the same scenario again (the same
        fingerprint); ``replace`` builds one with some of them changed. A demand read from a
        CSV file is here as the rows read.
        """
        return dict(self._parts)

    def replace(self, **changes: Any) -> Scenario:
        """A new scenario: this one with the arguments in ``changes`` changed.

        The names are those of ``from_parts``; everything not named stays as it is. This
        scenario is not changed. For example ``base.replace(flow_level=4)`` or
        ``base.replace(choice_options={"beta_time_min": -0.1})`` (an options dict replaces the
        whole dict: give every option you set).

        Raises:
            TypeError: If a name is not an argument of ``from_parts``, with the list of them.
        """
        unknown = sorted(set(changes) - set(self._parts))
        if unknown:
            raise TypeError(
                f"replace() takes the arguments of from_parts; no {unknown}. They are: "
                f"{sorted(self._parts)}"
            )
        return Scenario.from_parts(**{**self._parts, **changes})

    def _pipeline(self, run_id: str, output_dir: str, **extra: Any) -> Any:
        """The core's run, with this scenario's parts and ``extra``."""
        return _core.run_pipeline(
            self._network,
            run_id,
            output_dir,
            trips=self._trips,
            trips_path=self._trips_path,
            persons=self._persons,
            persons_path=self._persons_path,
            class_defaults=self._class_defaults,
            class_modes=self._class_modes,
            class_options=self._class_options,
            class_limits=self._class_limits,
            default_weight=self._default_weight,
            window_start_s=self._window[0],
            window_end_s=self._window[1],
            flow_level=self._flow_level,
            flow_step_s=self._flow_step_s,
            link_bin_s=self._link_bin_s,
            route_method=self._route_method,
            route_options=self._route_options,
            master_seed=self._master_seed,
            choice_model=self._choice_model,
            choice_options=self._choice_options,
            equilibration=self._equilibration,
            equilibration_options=self._equilibration_options,
            route_update=self._route_update,
            route_update_options=self._route_update_options,
            choice_detour_limit=self._choice_detour_limit,
            choice_max_routes=self._choice_max_routes,
            route_cache=self._route_cache,
            bike_cost=self._bike_cost,
            transit=self._transit,
            parkings=self._parkings,
            parking_options=self._parking_options,
            transit_options=self._transit_options,
            modes=self._modes,
            mode_options=self._mode_options,
            loading_options=self._loading_options,
            price_options=self._price_options,
            link_values=self._link_values,
            person_values=self._person_values,
            trip_values=self._trip_values or None,
            trip_values_keyed=self._trip_values_keyed or None,
            road_disruptions=self._road_disruptions,
            transit_disruptions=self._transit_disruptions,
            disruptions_known=self._disruptions_known,
            **extra,
        )

    def _fingerprint(self) -> str:
        """The fingerprint a run of this scenario would have, without running it.

        Everything a run prepares before it starts is prepared (the layers, the timetable,
        the parkings), so it takes a run's setup time, not its run time.
        """
        out = str(Path(tempfile.gettempdir()) / "openmobisim-runs" / "fingerprint")
        return str(self._pipeline("fingerprint", out, fingerprint_only=True))

    def run(
        self,
        run_id: str = "run",
        output_dir: str | None = None,
        quiet: bool = False,
        threads: int | None = None,
        overwrite: bool = False,
        design_id: str = "default",
        replication: int = 0,
        outputs: str = "all",
        hub_group_m: float = 150.0,
    ) -> Run:
        """Run the scenario and write its output artifacts.

        Args:
            run_id: Recorded in every output row; also names the default
                output directory.
            output_dir: Where ``kpis.parquet`` etc. are written. Defaults to
                ``./openmobisim-runs/<run_id>`` (the command line's default folder), under the
                current directory.
            quiet: If true, do not print the run's wall-clock time by stage when it ends
                (``Run.timings()`` and ``timings.csv`` have it either way).
            threads: How many threads the run's parallel work uses; ``None`` (the default),
                every logical core. Results do not depend on it (each is the same, bit for bit,
                at any count), so it is recorded in ``manifest.json`` (``threads``) and not in
                the fingerprint. The run uses a pool of its own: ``RAYON_NUM_THREADS`` does not
                change it.
            overwrite: Whether a folder that already holds a run (a ``manifest.json``) may be
                written into again. ``False`` (the default) refuses, so a second run with the
                same ``run_id`` cannot silently replace the first one's files.
            design_id: The design this run belongs to, written in every row of ``kpis.parquet``
                (``design_id``): a study comparing designs gives each its name. ``"default"``
                for a single run.
            replication: Which replication of its design this run is (``kpis.parquet``'s
                ``replication``), from 0: runs of one design that differ only in their
                ``master_seed``. Like ``run_id``, both are labels, not in the fingerprint.
            outputs: Which files the run writes in ``output_dir``. ``"all"`` (the default):
                ``kpis.parquet``, ``diagnostics.parquet``, ``events.parquet``, ``manifest.json``,
                ``timings.csv`` and, as the run has them, the link bins, ``transit_calls.parquet``
                and ``parking_bins.parquet``. ``"minimal"``, for mass runs (thousands of
                scenarios, a design space): ``kpis.csv`` (the rows of ``kpis.parquet``) and
                ``manifest.json`` only, a few kilobytes; the run's results in memory, its
                fingerprint and ``Run.open`` are the same, and the files it left out are
                refused by name. No page or figure is ever written by ``run``.
            hub_group_m: How far a parking may be from a stop, in metres in a straight line, to
                share its hub group (``hub_group`` in ``transit_calls`` and ``parking_bins``: a
                stop's station, or the stop itself without one; a parking whose table names a
                hub keeps it). A label of the outputs: results do not depend on it, so it is not
                in the fingerprint. 150 by default, a short walk across a station's forecourt (an
                assumption).

        Returns:
            A ``Run``: the results in memory, and the output files in ``output_dir``.

        Raises:
            FileExistsError: If ``output_dir`` already holds a run and ``overwrite`` is false.
        """
        if output_dir is None:
            output_dir = str(Path.cwd() / "openmobisim-runs" / run_id)
        if not overwrite and (Path(output_dir) / "manifest.json").exists():
            raise FileExistsError(
                f"{output_dir!r} already holds a run (manifest.json); give another run_id or "
                "output_dir, or run(..., overwrite=True) to replace its files"
            )
        if not isinstance(design_id, str) or not design_id:
            raise ValueError(f"design_id must be a non-empty string, got {design_id!r}")
        if not isinstance(replication, int) or replication < 0:
            raise ValueError(f"replication must be a whole number from 0, got {replication!r}")
        if outputs not in ("all", "minimal"):
            raise ValueError(f'outputs must be "all" or "minimal", got {outputs!r}')
        summary = self._pipeline(
            run_id,
            output_dir,
            threads=threads,
            design_id=design_id,
            replication=replication,
            outputs=outputs,
            hub_group_m=hub_group_m,
        )
        run = Run(
            summary,
            network=self._network,
            run_id=run_id,
            flow_level=self._flow_level,
            flow_step_s=self._flow_step_s,
            transit=self._transit,
            parkings=self._parkings,
            car_snap_motorway_m=(self._mode_options or {}).get("car_snap_motorway_m"),
        )
        run._outputs = outputs
        if not quiet:
            logger.info(run._summary_line())
            logger.debug(run._timings_text())
        left_out = run._window_text()
        if left_out is not None:
            warnings.warn(left_out, WindowWarning, stacklevel=2)
        fell_back = run._fallback_text()
        if fell_back is not None:
            warnings.warn(fell_back, FallbackWarning, stacklevel=2)
        stranded = run._no_alternative_text()
        if stranded is not None:
            warnings.warn(stranded, NoAlternativeWarning, stacklevel=2)
        return run
