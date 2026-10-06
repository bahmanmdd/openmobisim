"""Skims (S238, roadmap I-az): travel-time matrices between points by mode, after a run.

What is defended: each mode's time is the one its trips would take — the car's expected time
on the run's link times, the bike's least-cost route, the walk, the tram with its wait — on the
toy network's hand values (``W → M`` by car 80.55 s expected, by bike 120 s; ``W → N1`` by
bike 144 s, on foot 318.2 s; ``N1 → D2`` by tram leaving 100 s after one, 800 s); a point to
itself is 0; nothing within reach is ``nan``; a skim of a layer the run did not prepare is made
anyway; and wrong requests are refused.
"""

from __future__ import annotations

import numpy as np
import openmobisim as ms
import pytest


@pytest.fixture(scope="module")
def run() -> ms.Run:
    net = ms.examples.toy_network()
    o, d = net.node_lonlat("W"), net.node_lonlat("M")
    rows = [("a", 0, o[0], o[1], d[0], d[1], 0, "x", None, "car")]
    return ms.Scenario.from_parts(
        net, rows, classes={"x": (True, True, True)}, transit=ms.examples.toy_network_transit(),
        equilibration="free_flow", flow_level=0,
    ).run("skims", quiet=True)  # fmt: skip


def at(*names: str) -> dict[str, tuple[float, float]]:
    net = ms.examples.toy_network()
    return {n: net.node_lonlat(n) for n in names}


def test_each_mode_takes_the_time_its_trips_would(run: ms.Run) -> None:
    car = run.skim("car", at("W"), at("M", "W"))
    assert car.shape == (1, 2)
    assert car[0, 0] == pytest.approx(80.554054, abs=1e-3) and car[0, 1] == 0.0
    bike = run.skim("bike", at("W"), at("M", "N1"))
    assert bike[0].tolist() == pytest.approx([120.0, 144.0])
    walk = run.skim("walk", at("W"), at("N1"))
    assert walk[0, 0] == pytest.approx(300.0 * 2**0.5 / (4.8 / 3.6), abs=1e-6)
    tram = run.skim("transit", at("N1"), at("D2"), departure_s=100)
    assert tram[0, 0] == pytest.approx(800.0), "waits until 600 s, rides 300 s"
    # A minute's slack to board (``board_slack_s``): at 540 s the 600 s tram is caught.
    assert run.skim("transit", at("N1"), at("D2"), departure_s=540)[0, 0] == pytest.approx(360.0)
    assert run.skim("transit", at("N1"), at("D2"), departure_s=600)[0, 0] == pytest.approx(900.0)


def test_a_square_skim_has_zeros_on_its_diagonal_and_nan_out_of_reach(run: ms.Run) -> None:
    points = at("W", "M", "N1", "D2")
    for mode in ("car", "bike", "walk", "transit"):
        m = run.skim(mode, points)
        assert m.shape == (4, 4) and np.all(np.diag(m) == 0.0), mode
        assert not np.signbit(np.diag(m)).any(), "0, not -0"
    short = run.skim("walk", points, max_s=60.0)
    assert np.isnan(short[0, 1]), "W → M on foot takes over a minute"


def test_a_layer_the_run_did_not_prepare_is_made_for_its_skim() -> None:
    net = ms.examples.toy_network()
    o, d = net.node_lonlat("W"), net.node_lonlat("M")
    rows = [("a", 0, o[0], o[1], d[0], d[1], 0, "x", None, "car")]
    cars = ms.Scenario.from_parts(net, rows, classes={"x": (True, False, False)}).run(
        "skims-cars", quiet=True
    )
    assert cars.skim("bike", at("W"), at("N1"))[0, 0] == pytest.approx(144.0)
    with pytest.raises(ValueError, match="no timetable"):
        cars.skim("transit", at("W"), at("N1"))


@pytest.mark.parametrize(
    ("kwargs", "message"),
    [
        ({"mode": "car_transit"}, "car, bike, walk and transit"),
        ({"mode": "plane"}, "plane"),
        ({"mode": "car", "max_s": 0}, "max_s"),
    ],
)
def test_wrong_skims_are_refused(run: ms.Run, kwargs: dict, message: str) -> None:
    mode = kwargs.pop("mode")
    with pytest.raises(ValueError, match=message):
        run.skim(mode, at("W"), **kwargs)
