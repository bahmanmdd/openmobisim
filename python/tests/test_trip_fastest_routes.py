"""Every trip's fastest route by car after a run: ``Run.trip_fastest_routes``.

What is defended, on two roads from O to D whose times are known by hand: a short one (2 km
to P, its capacity ``c1``, then 2 km to D, at 50 km/h: 144 + 144 = 288 s) and a long one (two
links of 3.2 km at 50 km/h: 230.4 + 230.4 = 460.8 s):

* at free flow every trip's fastest route is the short road, 288 s, from the second its car
  enters the roads (its departure plus ``car_access_s``), and after a run of one loading the
  last loading's times are free flow too;
* under a queue, properties that cannot fail: no trip's drive is shorter than its fastest route
  at free flow (a point queue only adds time), no fastest route on the last loading's times is
  faster than at free flow, and each route is a route from O to D;
* the same seed gives the same routes; the same network with other ids, its links given in
  another order, gives every trip the same seconds and the same roads;
* a departure bin that is not a positive number of seconds is refused.
"""

from __future__ import annotations

import numpy as np
import openmobisim as ms
import pytest

LAT = 45.70
KM = 1 / (111.32 * np.cos(np.radians(LAT)))
#: The links as (id, from, to, metres); the short road's first link takes the capacity asked for.
LINKS = [
    ("op", "O", "P", 2000.0),
    ("pd", "P", "D", 2000.0),
    ("oq", "O", "Q", 3200.0),
    ("qd", "Q", "D", 3200.0),
]
NODES = {
    "O": (4.80, LAT),
    "P": (4.80 + 2 * KM, LAT + 0.004),
    "Q": (4.80 + 2 * KM, LAT - 0.004),
    "D": (4.80 + 4 * KM, LAT),
}
DRIVES_ONLY = {"car_access_s": 0.0, "car_egress_s": 0.0, "bike_access_s": 0.0, "bike_egress_s": 0.0}


def two_roads(c1: float, rename: dict[str, str] | None = None, reverse: bool = False) -> ms.Network:
    rename = rename or {}
    name = lambda x: rename.get(x, x)  # noqa: E731
    links = [
        {"id": name(i), "from": name(a), "to": name(b), "class": "primary", "lanes": 3, "length": m,
         "free_flow_speed": 50, "capacity": c1 if i == "op" else 6000.0}
        for i, a, b, m in LINKS
    ]  # fmt: skip
    nodes = [{"id": name(n), "x": x, "y": y} for n, (x, y) in NODES.items()]
    if reverse:
        links, nodes = links[::-1], nodes[::-1]
    return ms.network_read_table(links, nodes)


def run(
    net: ms.Network, departures: list[int], iterations: int = 1, seed: int = 0, **kwargs
) -> ms.Run:
    o, d = NODES["O"], NODES["D"]
    rows = [
        (f"t{i}", 0, o[0], o[1], d[0], d[1], s, "commuter", None, None)
        for i, s in enumerate(departures)
    ]
    eq = (
        ("free_flow", {})
        if iterations == 1
        else ("msa", {"iterations": iterations, "gap_tolerance": 0.0})
    )
    return ms.Scenario.from_parts(
        net, rows, equilibration=eq[0], equilibration_options=eq[1], master_seed=seed, **kwargs
    ).run("fastest", quiet=True, overwrite=True)


def ids(net: ms.Network, links: np.ndarray, back: dict[str, str] | None = None) -> list[str]:
    back = back or {}
    every = net.link_ids()
    return [back.get(every[int(k)], every[int(k)]) for k in links]


def test_the_roads_take_the_times_derived_by_hand() -> None:
    net = two_roads(6000.0)
    ff = dict(zip(net.link_ids(), np.asarray(net.link_free_flow_s()), strict=True))
    assert ff["op"] == pytest.approx(144.0) and ff["pd"] == pytest.approx(144.0)
    assert ff["oq"] == pytest.approx(230.4) and ff["qd"] == pytest.approx(230.4)


def test_at_free_flow_every_trip_takes_the_short_road_in_288_s() -> None:
    net = two_roads(6000.0)
    r = run(net, [0, 100, 1000, 2000])
    f = r.trip_fastest_routes()
    assert f["trip"].tolist() == [0, 1, 2, 3]
    # The car enters the roads `car_access_s` (120 s by default) after the departure.
    assert f["car_departure_s"].tolist() == [120.0, 220.0, 1120.0, 2120.0]
    assert f["free_flow_s"] == pytest.approx([288.0] * 4)
    assert f["final_s"] == pytest.approx([288.0] * 4), "a run of one loading: free flow"
    for links in list(f["free_flow_links"]) + list(f["final_links"]):
        assert ids(net, links) == ["op", "pd"]
    assert (
        len(set(f["origin_node"].tolist())) == 1 and len(set(f["destination_node"].tolist())) == 1
    )
    veh = np.asarray(r.trip_results()["vehicle_s"], dtype=float)
    assert np.all(veh >= f["free_flow_s"] - 1e-9)


@pytest.fixture(scope="module")
def queued() -> tuple[ms.Network, ms.Run]:
    # 300 trips in 5 minutes onto a road of 600 veh/h: a queue that sends some to the long road.
    net = two_roads(600.0)
    return net, run(net, [int(i) for i in np.arange(300)], iterations=4, mode_options=DRIVES_ONLY)


def test_under_a_queue_no_drive_and_no_route_beats_free_flow(
    queued: tuple[ms.Network, ms.Run],
) -> None:
    net, r = queued
    f = r.trip_fastest_routes()
    veh = np.asarray(r.trip_results()["vehicle_s"], dtype=float)
    done = np.isfinite(veh)
    assert done.sum() > 0
    assert np.all(veh[done] >= f["free_flow_s"][done] - 1e-9), "a point queue only adds time"
    assert np.all(f["final_s"] >= f["free_flow_s"] - 1e-9)
    assert f["final_s"].max() > 288.0 + 1.0, "the queue shows in the last loading's times"
    for links in f["final_links"]:
        assert ids(net, links) in (["op", "pd"], ["oq", "qd"])
    # A finer departure bin is a choice of the caller; every route is still one of the two.
    fine = r.trip_fastest_routes(departure_bin_s=60)
    assert np.all(fine["final_s"] >= fine["free_flow_s"] - 1e-9)


def test_the_same_seed_gives_the_same_routes(queued: tuple[ms.Network, ms.Run]) -> None:
    net, r = queued
    again = run(net, [int(i) for i in np.arange(300)], iterations=4, mode_options=DRIVES_ONLY)
    a, b = r.trip_fastest_routes(), again.trip_fastest_routes()
    for name in ("free_flow_s", "final_s", "car_departure_s"):
        assert np.array_equal(a[name], b[name]), name
    assert all(
        np.array_equal(x, y) for x, y in zip(a["final_links"], b["final_links"], strict=True)
    )


def test_other_ids_and_another_order_change_nothing(queued: tuple[ms.Network, ms.Run]) -> None:
    net, r = queued
    rename = {
        "op": "z1",
        "pd": "z2",
        "oq": "a1",
        "qd": "a2",
        "O": "n9",
        "P": "n1",
        "Q": "n0",
        "D": "n5",
    }
    back = {v: k for k, v in rename.items()}
    other_net = two_roads(600.0, rename, reverse=True)
    other = run(other_net, [int(i) for i in np.arange(300)], iterations=4, mode_options=DRIVES_ONLY)
    a, b = r.trip_fastest_routes(), other.trip_fastest_routes()
    assert np.array_equal(a["free_flow_s"], b["free_flow_s"])
    assert np.array_equal(a["final_s"], b["final_s"])
    for x, y in zip(a["final_links"], b["final_links"], strict=True):
        assert ids(net, x) == ids(other_net, y, back)


def test_a_bin_that_is_not_a_positive_number_of_seconds_is_refused(
    queued: tuple[ms.Network, ms.Run],
) -> None:
    _, r = queued
    for bad in (0, -60, float("nan")):
        with pytest.raises(ValueError, match="departure_bin_s"):
            r.trip_fastest_routes(departure_bin_s=bad)
