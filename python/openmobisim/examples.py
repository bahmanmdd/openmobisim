"""Built-in fixtures for getting started without any input data.

``toy_network`` is a small network whose every loading number can be checked by
hand. ``manhattan_grid`` is a square grid of two-way streets, optionally signalised.
``fixed_car_trips`` is deliberately thin: "car, fixed departure time, from
here to there" is not a different demand format from the general one — it
is what the same trips schema already expresses when ``classes`` give its
class a car and nothing else. This helper only saves writing out the
eight-column table by hand; it does not introduce a second demand shape.
``traveller_classes`` is an example class table: six classes, each with the
modes its travellers may use and its own mode constants. ``case`` gives a
**starter case** (S240): Amsterdam, Paris, Lyon, Sioux Falls or Nguyen–Dupuis,
its files fetched once and checked, read in one call each.
"""

from __future__ import annotations

import numpy as np

from openmobisim import _core
from openmobisim._cases import Case, case, case_names

__all__ = [
    "Case",
    "case",
    "case_names",
    "fixed_car_trips",
    "manhattan_grid",
    "toy_network",
    "toy_network_parkings",
    "toy_network_transit",
    "traveller_classes",
    "trips_random",
]

# A direct re-export, not a wrapper: PyO3 already carries the Rust doc
# comment as this function's `__doc__` (and `_core.pyi` is the hand-kept
# stub `mypy`/editors actually read), and a builtin function's `__doc__`
# cannot be reassigned from Python even if a different one were wanted.
manhattan_grid = _core.manhattan_grid

# The toy network's road part (I-m, S161): sixteen nodes and links, every number
# checkable by hand. Node names are "W", "N1", "S", "M", … (see the Rust docs).
toy_network = _core.toy_network

# The toy network's timetable (S199): the tram T1 (stop "N1" to stop "H" at D2
# and back) and the bus B1 ("W", "M", "D1" on the arterial), every 10 minutes.
toy_network_transit = _core.toy_network_transit

# The toy network's parkings: hub "H" at D2 with 6 car and 3 bike spaces, where
# the tram stop is, and car park "P2" at R2 with 20 spaces, 300 m on foot from it.
toy_network_parkings = _core.toy_network_parkings


def fixed_car_trips(
    network: _core.Network,
    trips: list[tuple[str, tuple[int, int], tuple[int, int], float, int | None]],
    user_class: str = "commuter",
) -> list[tuple]:
    """Build a trips table for car-only demand at a fixed departure time.

    A convenience over the general schema, for the common case in a demo or
    a test: give the mode a fixed availability (a car, via
    ``classes``) and a fixed departure time, rather than letting a
    choice model decide. Every row gets ``trip_seq = 0`` — this helper does
    not build multi-trip chains.

    Args:
        network: A grid built by ``manhattan_grid``.
        trips: ``(traveller_id, (origin_row, origin_col),
            (destination_row, destination_col), departure_hour, weight)``
            tuples. ``weight=None`` takes the scenario's default weight.
        user_class: The class every row is tagged with. A class that
            ``classes`` (passed to ``Scenario.from_parts``, not here) does not
            list owns a car (S243); one listed without a car reports every trip
            in this table as ``no_vehicle_available``.

    Returns:
        Rows in the schema ``Scenario.from_parts``' ``demand`` argument (or
        ``run_pipeline``) expects — the same table a ``trips.parquet`` file
        would produce.
    """
    rows = []
    for traveller_id, origin, destination, hour, weight in trips:
        origin_lon, origin_lat = _core.grid_node_lonlat(network, *origin)
        destination_lon, destination_lat = _core.grid_node_lonlat(network, *destination)
        departure_s = round(hour * 3600)
        rows.append(
            (
                traveller_id,
                0,
                origin_lon,
                origin_lat,
                destination_lon,
                destination_lat,
                departure_s,
                user_class,
                weight,
            )
        )
    return rows


def trips_random(
    network: _core.Network,
    count: int,
    *,
    seed: int = 0,
    spread_s: int = 3600,
    min_m: float = 500.0,
    max_m: float = 5000.0,
    user_class: str = "commuter",
    mode: str | None = None,
) -> list[tuple]:
    """A seeded load fixture: `count` trips between random places on `network`.

    **Not a forecast.** Origins and destinations are the start points of random
    links, a crow-flies distance of ``min_m``-``max_m`` apart, departing at
    uniformly random times in the first ``spread_s`` seconds. It exists to put
    a realistic *amount* of traffic on a real network, so a run and its
    figures can be demonstrated and its cost measured. The same ``seed`` always
    gives the same trips.

    A class that ``classes`` does not list owns a car (S243); one listed
    without a car reports every trip as ``no_vehicle_available``.

    Args:
        network: Any network.
        count: How many trips.
        seed: The random seed.
        spread_s: Departures fall in ``[0, spread_s)`` seconds.
        min_m: The nearest a destination may be to its origin, in metres.
        max_m: The farthest, in metres.
        user_class: The class every row is tagged with.
        mode: Every trip's mode (``"car"``, ``"bike"``, ``"walk"``, …); ``None``
            (the default) leaves it out, which is a car trip.

    Returns:
        Rows in the trips schema ``Scenario.from_parts`` takes.

    Raises:
        ValueError: If no destination can be found within ``min_m``-``max_m``.
    """
    coords, offsets = network.link_geometry()
    starts = coords[offsets[:-1].astype(np.int64)]
    rng = np.random.default_rng(seed)
    n = len(starts)
    cos_lat = float(np.cos(np.radians(starts[:, 1].mean())))
    xy = np.column_stack([starts[:, 0] * cos_lat * 111_320.0, starts[:, 1] * 110_574.0])
    # A grid of `max_m` cells: every place within `max_m` of a point is in its
    # own cell or one of the eight around it, so a destination is drawn from
    # those nine cells only, however large the network.
    cell = np.floor((xy - xy.min(axis=0)) / max_m).astype(np.int64) + 1
    width = int(cell[:, 1].max()) + 2
    key = cell[:, 0] * width + cell[:, 1]
    order = np.argsort(key, kind="stable")
    sorted_key = key[order]
    around = np.array([dx * width + dy for dx in (-1, 0, 1) for dy in (-1, 0, 1)])

    origin = rng.integers(0, n, size=count)
    destination = np.full(count, -1, dtype=np.int64)
    todo = np.arange(count)
    for round_ in range(100):
        if not len(todo):
            break
        if round_ > 0:  # an origin with nothing in range is redrawn, not retried
            origin[todo] = rng.integers(0, n, size=len(todo))
        neighbours = key[origin[todo]][:, None] + around[None, :]
        lo = np.searchsorted(sorted_key, neighbours, side="left")
        hi = np.searchsorted(sorted_key, neighbours, side="right")
        count_in = hi - lo
        total = count_in.sum(axis=1)
        pick = np.floor(rng.random(len(todo)) * total).astype(np.int64)
        after = np.cumsum(count_in, axis=1)
        which = (after > pick[:, None]).argmax(axis=1)
        rows = np.arange(len(todo))
        within = pick - (after[rows, which] - count_in[rows, which])
        position = np.minimum(lo[rows, which] + within, n - 1)
        candidate = order[position]
        metres = np.hypot(*(xy[candidate] - xy[origin[todo]]).T)
        ok = (total > 0) & (metres >= min_m) & (metres <= max_m)
        destination[todo[ok]] = candidate[ok]
        todo = todo[~ok]
    if len(todo):
        raise ValueError(
            f"no destination {min_m:g}-{max_m:g} m from {len(todo)} of {count} origins; "
            "widen min_m/max_m"
        )
    departure = rng.integers(0, max(spread_s, 1), size=count)
    return [
        (
            f"r{i}",
            0,
            float(starts[origin[i], 0]),
            float(starts[origin[i], 1]),
            float(starts[destination[i], 0]),
            float(starts[destination[i], 1]),
            int(departure[i]),
            user_class,
            None,
        )
        + (() if mode is None else (mode,))
        for i in range(count)
    ]


def traveller_classes() -> dict[str, dict]:
    """Six example traveller classes (S231), in the shape of ``demand_read_classes``.

    ==========================  =====  ==================================  ===========  ===========
    Class                       Share  Modes                               Constants    Own limits
    ==========================  =====  ==================================  ===========  ===========
    ``car_captive``             0.05   car                                 car 0
    ``bike_enthusiast``         0.45   bike, walk, transit, bike-and-ride  bike 1.5     ride 90 min
    ``transit_only``            0.15   walk, transit                       transit 0.5
    ``open_to_all``             0.25   all six                             all 0
    ``walker``                  0.05   walk, transit                       walk 1.0     walk 45 min
    ``park_and_ride_commuter``  0.05   car, transit, park-and-ride         transit 0.3
    ==========================  =====  ==================================  ===========  ===========

    Each class owns what its modes need (a car for one that may drive or park and ride, a bike
    for one that may cycle, a transit pass for one that may take transit); a constant not shown
    is 0. A class's own limits (S235) replace the run's for its trips choosing their mode
    (``bike_max_s`` 5400, ``walk_max_s`` 2700); the others take the run's (30 minutes' walk,
    60 minutes' ride, 30 minutes' walk to a stop). **Illustrative, not calibrated**: a starting
    point to replace with a study's own. The shares were chosen (S234, refitted under these
    limits in S235) so that a synthetic morning commute in Amsterdam comes near the city's split
    between cycling, public transport and driving (about 38 : 17 : 23); walking, mostly short
    trips, needs a demand that has them.
    ``demand_assign_classes(trips, traveller_classes())`` draws them for a demand's travellers;
    ``Scenario.from_parts(..., classes=traveller_classes())`` runs them.
    """
    from openmobisim.demand import _class_table

    every = ["car", "bike", "walk", "transit", "car_transit", "bike_transit"]
    return _class_table({
        "car_captive": {"share": 0.05, "modes": ["car"], "beta_mode_car": 0.0},
        "bike_enthusiast": {
            "share": 0.45, "modes": ["bike", "walk", "transit", "bike_transit"],
            "beta_mode_bike": 1.5, "beta_mode_walk": 0.0, "beta_mode_transit": 0.0,
            "bike_max_s": 5400,
        },
        "transit_only": {
            "share": 0.15, "modes": ["walk", "transit"], "beta_mode_walk": 0.0,
            "beta_mode_transit": 0.5,
        },
        "open_to_all": {
            "share": 0.25, "modes": every, "beta_mode_car": 0.0, "beta_mode_bike": 0.0,
            "beta_mode_walk": 0.0, "beta_mode_transit": 0.0,
        },
        "walker": {
            "share": 0.05, "modes": ["walk", "transit"], "beta_mode_walk": 1.0,
            "beta_mode_transit": 0.0, "walk_max_s": 2700,
        },
        "park_and_ride_commuter": {
            "share": 0.05, "modes": ["car", "transit", "car_transit"], "beta_mode_car": 0.0,
            "beta_mode_transit": 0.3,
        },
    })  # fmt: skip
