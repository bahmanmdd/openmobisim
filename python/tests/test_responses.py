"""Responses in the direction theory gives, for a population of travellers.

What is defended, everything else held (the same travellers, the same draws, the same
settings): a toll on the roads, a dearer fare and a dearer kilometre by car never attract more
of the travellers they charge, and a large rise takes most of them away; separated bike tracks
draw cyclists who dislike riding in mixed traffic; halving every road's capacity lengthens the
mean trip and a third of the demand shortens it; and values a run is given but no model weighs
(a traveller's, a trip's, a link's, a price) change no result. The steps are large on purpose:
each assertion is about a direction, which sampling noise at these sizes cannot reverse.

The choice runs load at free flow (``flow_level=0``, one loading), so a response is the choice
model's alone and not the congestion's; the capacity and demand runs are congested on purpose.
"""

from __future__ import annotations

from pathlib import Path

import numpy as np
import openmobisim as ms
import pytest

CLASSES = ms.examples.traveller_classes()
MONEY = {"beta_cost_eur": -0.2}  # 6 € an hour at the default time weight
TRANSIT = ("transit", "car_transit", "bike_transit")
DRIVE = ("car", "car_transit")


def toy_trips() -> tuple:
    net = ms.examples.toy_network()
    trips = ms.examples.trips_random(net, 3000, seed=3, spread_s=1800, min_m=300, max_m=3000)
    return net, ms.demand_assign_classes(trips, CLASSES, seed=0)


def choose(net: object, trips: list[tuple], **settings: object) -> dict[str, float]:
    """Each mode's share of the people (%) in a run of every mode at free flow."""
    parts = {
        "classes": CLASSES,
        "transit": ms.examples.toy_network_transit(),
        "parkings": ms.examples.toy_network_parkings(),
        "modes": ms.MODES,
        "parking_options": {"pr_min_km": 0},
        "equilibration": "free_flow",
        "flow_level": 0,
    }
    run = ms.Scenario.from_parts(net, trips, **{**parts, **settings}).run("responses", quiet=True)
    return shares(run)


def shares(run: ms.Run) -> dict[str, float]:
    m = run.trip_modes()
    weight = np.asarray(m["weight"], dtype=float)
    out: dict[str, float] = {}
    for mode, w in zip(m["mode"], weight, strict=True):
        out[mode] = out.get(mode, 0.0) + float(w)
    return {mode: 100.0 * w / weight.sum() for mode, w in out.items()}


def share(s: dict[str, float], modes: tuple[str, ...]) -> float:
    return sum(s.get(mode, 0.0) for mode in modes)


def test_a_toll_takes_drivers_off_the_road():
    net, trips = toy_trips()
    driving = [
        share(choose(net, trips, choice_options=MONEY,
                     link_values={"road": {"toll_eur": [toll] * net.link_count}}), DRIVE)
        for toll in (0.0, 0.5, 5.0)
    ]  # fmt: skip
    assert driving[0] > driving[1] > driving[2], driving
    assert driving[2] < 0.25 * driving[0], "five euros on every link empties the roads"


def test_a_dearer_fare_takes_riders_off_transit():
    net, trips = toy_trips()
    riding = [
        share(choose(net, trips, choice_options=MONEY, price_options={"fare_base_eur": fare}),
              TRANSIT)
        for fare in (0.0, 3.0, 20.0)
    ]  # fmt: skip
    assert riding[0] > riding[1] > riding[2], riding
    assert riding[2] < 0.1 * riding[0], "a 20 € fare leaves almost no rider"


def test_a_dearer_kilometre_takes_drivers_off_the_road():
    net, trips = toy_trips()
    cheap = choose(net, trips, choice_options=MONEY, price_options={"car_eur_km": 0.0})
    dear = choose(net, trips, choice_options=MONEY, price_options={"car_eur_km": 10.0})
    assert share(dear, DRIVE) < share(cheap, DRIVE)


def test_separated_tracks_draw_cyclists_who_dislike_mixed_traffic(tmp_path: Path):
    path = tmp_path / "classes.csv"
    path.write_text("class,share,modes,beta_bike_mixed_km\neveryone,1,walk;bike;car,-1.0\n")
    classes = ms.demand_read_classes(path)
    net = ms.examples.toy_network()
    trips = ms.examples.trips_random(net, 3000, seed=3, spread_s=1800, min_m=300, max_m=3000)
    trips = ms.demand_assign_classes(trips, classes, seed=0)
    bike = net.layer("bike")
    mixed = [i for i, kind in zip(bike.link_ids(), bike.link_infrastructure(), strict=True)
             if kind == 0]  # fmt: skip
    assert mixed, "the toy has streets without a bike facility"
    tracks = ms.network_edit(net, bike_facility={i: "separated" for i in mixed})

    def run(network: object) -> dict[str, float]:
        return shares(ms.Scenario.from_parts(
            network, trips, classes=classes, modes=("walk", "bike", "car"),
            equilibration="free_flow", flow_level=0).run("tracks", quiet=True))  # fmt: skip

    before, after = run(net), run(tracks)
    assert after["bike"] > before["bike"] + 1.0, (before, after)
    assert after.get("car", 0.0) < before.get("car", 0.0)


def congested(network: object, trips: list[tuple], equilibration: str) -> float:
    run = ms.Scenario.from_parts(network, trips, equilibration=equilibration).run(
        "congested", quiet=True
    )
    assert run.completion["completed"] == len(trips)
    return run.mean_travel_time_s


@pytest.mark.parametrize("equilibration", ["msa", "free_flow"])
def test_less_capacity_lengthens_trips_and_less_demand_shortens_them(equilibration):
    grid = ms.examples.manhattan_grid(6, 200.0, True)
    cars = ms.examples.trips_random(grid, 6000, seed=5, spread_s=900, min_m=400, max_m=2000,
                                    mode="car")  # fmt: skip
    half = ms.network_edit(grid, capacity_factor={i: 0.5 for i in range(grid.link_count)})
    base = congested(grid, cars, equilibration)
    assert congested(half, cars, equilibration) > 1.5 * base
    assert congested(grid, cars[:2000], equilibration) < base


def test_values_no_model_weighs_change_no_result():
    net, trips = toy_trips()

    def run(**settings: object) -> ms.Run:
        return ms.Scenario.from_parts(
            net, trips, classes=CLASSES, transit=ms.examples.toy_network_transit(),
            parkings=ms.examples.toy_network_parkings(), modes=ms.MODES,
            parking_options={"pr_min_km": 0}, link_bin_s=300, **settings,
        ).run("unweighed", quiet=True)  # fmt: skip

    base = run()
    given = {
        "person_values": {"income": {t[0]: float(i % 7) for i, t in enumerate(trips)}},
        "trip_values": {"urgency": [float(i % 3) for i in range(len(trips))]},
        "link_values": {"road": {"noise": [1.0] * net.link_count}},
        "price_options": {"car_eur_km": 5.0, "fare_base_eur": 9.0},
    }
    for name, value in given.items():
        other = run(**{name: value})
        assert list(other.trip_modes()["mode"]) == list(base.trip_modes()["mode"]), name
        assert other.total_travel_time_s == base.total_travel_time_s, name
        assert other.completion == base.completion, name
        assert np.array_equal(other.link_bins("road").pcu(), base.link_bins("road").pcu()), name
