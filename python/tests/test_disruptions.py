"""Disruptions at a time of day (S238, roadmap I-az: timed events).

What is defended, on the toy network's hand values: a road closed for a while holds its cars
until it opens (``W → M`` has one road, ``a1`` then ``a2``: 80 s free); at free flow capacity
plays no part; a tram's runs delayed or cancelled change a passenger's trip — ``N1 → D2``
leaving at 100 s takes the 600 s tram (800 s); 120 s late, 920 s; cancelled and unannounced,
the passenger waits for it in vain and takes the 1 200 s tram (1 400 s); announced, they plan
another way from the start (1 181 s); the disruptions are in the fingerprint and the manifest;
and wrong ones are refused.
"""

from __future__ import annotations

import openmobisim as ms
import pytest

CLOSED = {"links": ["a2"], "capacity_factor": 0.0, "from_s": 60, "to_s": 300}


def cars(n: int = 30) -> list[tuple]:
    net = ms.examples.toy_network()
    o, d = net.node_lonlat("W"), net.node_lonlat("M")
    return [(f"c{k}", 0, o[0], o[1], d[0], d[1], 10 * k, "x", None, "car") for k in range(n)]


def run_cars(name: str, **kwargs: object) -> ms.Run:
    return ms.Scenario.from_parts(
        ms.examples.toy_network(), cars(), classes={"x": (True, True, False)}, **kwargs
    ).run(name, quiet=True)


def tram_trip(name: str, **kwargs: object) -> ms.Run:
    net = ms.examples.toy_network()
    o, d = net.node_lonlat("N1"), net.node_lonlat("D2")
    rows = [("p", 0, o[0], o[1], d[0], d[1], 100, "y", None, "transit")]
    return ms.Scenario.from_parts(
        net, rows, classes={"y": (False, False, True)},
        transit=ms.examples.toy_network_transit(), **kwargs,
    ).run(name, quiet=True)  # fmt: skip


def test_a_closed_road_holds_its_cars_until_it_opens() -> None:
    base, closed = run_cars("dis-base"), run_cars("dis-closed", disruptions=[CLOSED])
    assert base.mean_travel_time_s == pytest.approx(80.0)
    assert closed.completion["completed"] == 30, "everyone arrives once it opens"
    assert closed.mean_travel_time_s > base.mean_travel_time_s + 60
    assert closed.manifest()["disruptions"] == {"road": 1, "transit": 0, "known": False}
    assert closed.fingerprint != base.fingerprint
    known = run_cars("dis-known", disruptions=[CLOSED], disruptions_known=True)
    assert known.manifest()["disruptions"]["known"] and known.fingerprint != closed.fingerprint
    free = run_cars("dis-free", disruptions=[CLOSED], flow_level=0, equilibration="free_flow")
    assert free.mean_travel_time_s == pytest.approx(80.0), "no capacity at free flow"


def test_a_tram_late_or_cancelled_known_or_not() -> None:
    assert tram_trip("dis-tram").mean_travel_time_s == pytest.approx(800.0)
    late = {"line": "T1", "delay_s": 120, "from_s": 0, "to_s": 1000}
    assert tram_trip("dis-late", disruptions=[late]).mean_travel_time_s == pytest.approx(920.0)
    gone = {"line": "T1", "cancel": True, "from_s": 500, "to_s": 700}
    unknown = tram_trip("dis-gone", disruptions=[gone])
    assert unknown.mean_travel_time_s == pytest.approx(1400.0), "waits for the next, at 1 200 s"
    assert list(unknown.itinerary_choices()["expected_s"]) == [800.0], "planned on the 600 s tram"
    known = tram_trip("dis-gone-known", disruptions=[gone], disruptions_known=True)
    assert known.mean_travel_time_s == pytest.approx(1181.0), "another way, planned"


@pytest.mark.parametrize(
    ("disruption", "message"),
    [
        ({"links": ["zz"], "capacity_factor": 0.0, "from_s": 0, "to_s": 1}, "no link 'zz'"),
        ({"links": ["a2"], "capacity_factor": 0.0, "from_s": 0}, "from_s and to_s"),
        ({"links": ["a2"], "capacity_factor": -1.0, "from_s": 0, "to_s": 1}, "factor"),
        ({"links": ["a2"], "capacity_factor": 0.5, "from_s": 9, "to_s": 1}, "ends before"),
        ({"line": "T1", "delay_s": 60, "from_s": 0, "to_s": 1}, "needs transit"),
        ({"colour": "red", "from_s": 0, "to_s": 1}, "no such key"),
    ],
)
def test_wrong_disruptions_are_refused(disruption: dict, message: str) -> None:
    with pytest.raises(ValueError, match=message):
        run_cars("dis-wrong", disruptions=[disruption])


def test_a_line_s_disruption_names_a_line_of_the_timetable() -> None:
    with pytest.raises(ValueError, match="no line 'Z9'"):
        tram_trip(
            "dis-no-line", disruptions=[{"line": "Z9", "delay_s": 60, "from_s": 0, "to_s": 9}]
        )
    with pytest.raises(ValueError, match="delay_s or cancel"):
        tram_trip("dis-neither", disruptions=[{"line": "T1", "from_s": 0, "to_s": 9}])
