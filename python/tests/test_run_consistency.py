"""A run's results agree with each other, whatever the case and the settings.

What is defended: every trip is accounted for once; the modes add up to the run in trips,
people and travel time; the mean trip is the total over the people who arrived;
``trip_modes`` has one row per trip, with the weights given; a chosen alternative's probability
lies in (0, 1], its logsum is finite, and the choices agree with ``trip_modes``; no layer carries
a negative flow and no link-bin's mean road time is below free flow; every transit run boards as
many as alight, nobody is below zero on board, and the boardings add up; parkings are never
below empty; an iterated run gets a verdict. Each holds by construction, so a failure is a fault
in the accounting, never a modelling question.

They are checked over the loadings (the point queue and the full model), the equilibration
(msa and the free-flow loading), the built-in choice models, with money weighed and with
weighted travellers, on the toy network with its timetable and parkings (every mode, the example
traveller classes), on a signalised grid, and under congestion, where the fixture is checked to
congest so that the queues are exercised.
"""

from __future__ import annotations

import collections
import itertools
import math

import numpy as np
import openmobisim as ms
import pytest

CLASSES = ms.examples.traveller_classes()
PRICES = {"car_eur_km": 0.2, "fare_base_eur": 1.5, "fare_km_eur": 0.2, "parking_car_eur": 2.0}
OUTCOMES = ("completed", "truncated", "no_vehicle_available", "no_feasible_path",
            "mode_not_available")  # fmt: skip


def case(name: str, weight: int = 1) -> tuple:
    """A network, its trips and the scenario's other parts."""
    if name == "toy":  # every mode, the example classes, the toy's timetable and parkings
        net = ms.examples.toy_network()
        trips = ms.examples.trips_random(net, 3000, seed=3, spread_s=1800, min_m=50, max_m=3000)
        trips = ms.demand_assign_classes(trips, CLASSES, seed=0)
        parts = {
            "classes": CLASSES,
            "transit": ms.examples.toy_network_transit(),
            "parkings": ms.examples.toy_network_parkings(),
            "modes": ms.MODES,
            "parking_options": {"pr_min_km": 0},
        }
    elif name == "grid":  # cars, bikes and walking on a signalised grid
        net = ms.examples.manhattan_grid(6, 200.0, True)
        trips = ms.examples.trips_random(net, 3000, seed=3, spread_s=1800, min_m=200, max_m=2000)
        trips = ms.demand_assign_classes(trips, CLASSES, seed=0)
        parts = {"classes": CLASSES, "modes": ("car", "bike", "walk")}
    else:  # "congested": cars only, a quarter of an hour's heavy load on the grid
        net = ms.examples.manhattan_grid(6, 200.0, True)
        trips = ms.examples.trips_random(
            net, 6000, seed=5, spread_s=900, min_m=400, max_m=2000, mode="car"
        )
        parts = {}
    if weight > 1:
        trips = ms.demand_sample(trips, weight, seed=0)
    return net, trips, parts


def inconsistencies(net: object, trips: list[tuple], run: ms.Run) -> list[str]:
    """Every rule the results break, as a readable line; empty when they agree."""
    bad = []
    c = run.completion
    n = len(trips)
    people = sum(r[8] or 1 for r in trips)
    if c["total_trips"] != n or sum(c[k] for k in OUTCOMES) != n:
        bad.append(f"{n} trips, {c['total_trips']} counted, outcomes {[c[k] for k in OUTCOMES]}")

    by_mode = run.completion_by_mode
    for key in ("total_trips", "completed"):
        total = sum(row[key] for row in by_mode.values())
        if total != c[key]:
            bad.append(f"the modes' {key} add to {total}, the run's is {c[key]}")
    if not math.isclose(sum(row["people"] for row in by_mode.values()), people):
        bad.append("the modes' people do not add to the run's")
    time_s = sum(row["total_travel_time_s"] for row in by_mode.values())
    if not math.isclose(time_s, run.total_travel_time_s, rel_tol=1e-9):
        bad.append(f"the modes' travel time {time_s} against the run's {run.total_travel_time_s}")
    arrived = sum(row["completed_people"] for row in by_mode.values())
    if arrived and not math.isclose(run.mean_travel_time_s, time_s / arrived, rel_tol=1e-9):
        bad.append("the mean trip is not the total over the people who arrived")

    m = run.trip_modes()
    if len(m["mode"]) != n:
        bad.append(f"trip_modes has {len(m['mode'])} rows for {n} trips")
    if not math.isclose(float(np.sum(m["weight"])), people):
        bad.append("trip_modes' weights are not the people given")
    if not set(m["mode"]) <= set(ms.MODES) | {None}:
        bad.append(f"unknown modes {set(m['mode']) - set(ms.MODES)}")

    ch = run.itinerary_choices()
    if ch is not None and len(ch["mode"]):
        chose = np.asarray([x is not None for x in ch["mode"]])
        p = np.asarray(ch["probability"], dtype=float)[chose]
        if not np.all((p > 0) & (p <= 1 + 1e-12)):
            bad.append(f"a chosen probability outside (0, 1]: {p.min()}, {p.max()}")
        if not np.all(np.asarray(ch["alternatives"])[chose] >= 1):
            bad.append("a trip chose among no alternatives")
        if not np.all(np.isfinite(np.asarray(ch["logsum"], dtype=float)[chose])):
            bad.append("a chosen trip's logsum is not finite")
        took = dict(zip(zip(m["traveller_id"], m["trip_seq"], strict=True), m["mode"], strict=True))
        rows = zip(ch["traveller_id"], ch["trip_seq"], ch["mode"], strict=True)
        if any(took[(t, s)] != mode for t, s, mode in rows):
            bad.append("a choice disagrees with trip_modes")

    for layer in ("road", "bike", "walk"):
        bins = run.link_bins(layer)
        if bins is not None and len(bins) and np.any(np.asarray(bins.pcu()) < 0):
            bad.append(f"a negative flow on the {layer} layer")
    road = run.link_bins("road")
    if road is not None and len(road):
        flow = np.asarray(road.pcu())
        seen = flow > 0
        mean_s = np.asarray(road.pcu_seconds())[seen] / flow[seen]
        free_s = np.asarray(net.link_free_flow_s())[np.asarray(road.links())[seen]]
        if np.any(mean_s < free_s * 0.999 - 1e-6):
            bad.append(f"a link-bin faster than free flow: {np.min(mean_s / free_s):.4f}")

    if run.transit is not None:
        calls = run.transit_calls()
        boarded, alighted = collections.Counter(), collections.Counter()
        for r, on, off in zip(calls["run"], calls["boardings"], calls["alightings"], strict=True):
            boarded[r] += on
            alighted[r] += off
        if any(not math.isclose(boarded[r], alighted[r], abs_tol=1e-6) for r in boarded):
            bad.append("a transit run's boardings differ from its alightings")
        on_board = np.asarray(calls["on_board"], dtype=float)
        if np.any(on_board[np.isfinite(on_board)] < -1e-9):
            bad.append("fewer than none on board")
        if not math.isclose(sum(boarded.values()), run.transit_summary["boardings"]):
            bad.append("the calls' boardings do not add to the summary's")

    if run.parkings is not None:
        bins = run.parking_bins()
        if bins is not None and np.any(np.asarray(bins["occupancy_mean"], dtype=float) < -1e-9):
            bad.append("a parking below empty")

    if run.equilibration == "msa" and run.convergence_verdict not in ("good", "acceptable", "poor"):
        bad.append(f"an iterated run's verdict is {run.convergence_verdict!r}")
    return bad


def run_case(name: str, weight: int = 1, money: bool = False, **settings: object) -> tuple:
    """A case run with ``settings``, money weighed if asked; the network, trips and run."""
    net, trips, parts = case(name, weight)
    if money:
        parts = {**parts, "price_options": PRICES}
        settings = {**settings, "choice_options": {"beta_cost_eur": -0.2}}
    run = ms.Scenario.from_parts(net, trips, link_bin_s=300, **parts, **settings).run(
        f"consistency-{name}", quiet=True
    )
    return net, trips, run


SETTINGS = itertools.product(
    ("toy", "grid"), (2, 4), ("msa", "free_flow"), ("logit", "nested_logit"), (False, True), (1, 3)
)
MULTIMODAL = [
    pytest.param(*s, id="-".join(map(str, s)))
    for s in SETTINGS
    if not (s[0] == "grid" and s[3] == "nested_logit" and s[5] == 3)  # a lean matrix
]


@pytest.mark.parametrize(("name", "level", "equilibration", "model", "money", "weight"), MULTIMODAL)
def test_a_multimodal_run_s_results_agree(name, level, equilibration, model, money, weight):
    net, trips, run = run_case(
        name, weight, money, flow_level=level, equilibration=equilibration, choice_model=model
    )
    assert inconsistencies(net, trips, run) == []


@pytest.mark.parametrize("level", [2, 4])
@pytest.mark.parametrize("equilibration", ["msa", "free_flow"])
def test_a_congested_run_s_results_agree(level, equilibration):
    net, trips, run = run_case("congested", flow_level=level, equilibration=equilibration)
    assert inconsistencies(net, trips, run) == []


def test_the_congested_case_congests_so_its_queues_are_exercised():
    _, _, light = run_case("congested", flow_level=0, equilibration="free_flow")
    _, _, queue = run_case("congested", flow_level=2, equilibration="msa")
    _, _, full = run_case("congested", flow_level=4, equilibration="msa")
    assert queue.mean_travel_time_s > 1.2 * light.mean_travel_time_s, "queues lengthen trips"
    assert full.mean_travel_time_s != queue.mean_travel_time_s, "spillback changes the loading"
