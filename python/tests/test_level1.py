"""Flow level 1, the static user-equilibrium assignment (S294).

What is defended, and how each is checked:

* **one link, exact by hand**: every trip's time is the BPR time of the whole hour's flow, the
  hour read from the departures (not the three-hour window), the gap 0 at once;
* **the demand made static**: every departure moved to the window's start and every trip end
  to its zone's point, by the run and by the two helpers on their own, which change nothing on a
  demand already made that way;
* **the two solvers agree**, and the options of each level are refused where they do not apply;
* **the same run on 1 thread and 7** gives the same results and fingerprint;
* **Sioux Falls against TNTP's best-known solution** (when ``OPENMOBISIM_DATA`` points at the
  data bundle): the total travel time and the link flows of Frank–Wolfe at a relative gap of
  1e-4.
"""

from __future__ import annotations

import math
import os

import numpy as np
import openmobisim as ms
import pytest

CAR = {"commuter": (True, False, False)}
A, B = (4.8000, 45.7000), (4.8130, 45.7000)


def one_link() -> ms._core.Network:
    """``a → b``: one primary street of 1 000 veh/h, 1 km at 50 km/h."""
    nodes = [{"id": "a", "x": A[0], "y": A[1]}, {"id": "b", "x": B[0], "y": B[1]}]
    links = [
        {"id": "ab", "from": "a", "to": "b", "class": "primary", "length": 1000.0,
         "free_flow_speed": 50.0, "capacity": 1000.0, "lanes": 1},
    ]  # fmt: skip
    return ms.network_read_table(links, nodes)


def trips_over_the_hour(n: int, start: int = 25_200) -> list[tuple]:
    """``n`` car trips ``a → b`` departing evenly over the hour from ``start``."""
    return [(f"p{i:04d}", 0, *A, *B, start + i * 3600 // n, "commuter", None) for i in range(n)]


def bpr(t0: float, flow: float, capacity: float) -> float:
    return t0 * (1 + 0.15 * (flow / capacity) ** 4)


def test_one_link_takes_the_whole_hour_at_its_bpr_time() -> None:
    net = one_link()
    run = ms.Scenario.from_parts(
        net,
        trips_over_the_hour(1500),
        classes=CAR,
        flow_level=1,
        equilibration="frank_wolfe",
        link_bin_s=900,
    ).run("one-link", quiet=True)
    t0 = float(net.link_free_flow_s()[0])
    expected = bpr(t0, 1500.0, 1000.0)
    assert run.completion["completed"] == run.completion["total_trips"] == 1500
    # The hour of departures is one hour of flow, though the window is three hours long.
    assert run.mean_travel_time_s == pytest.approx(expected, abs=1.0)
    c = run.convergence()
    assert len(c["gap"]) == 1 and c["gap"][0] == pytest.approx(0.0, abs=1e-12)
    assert run.converged and run.convergence_verdict == "good"
    assert c["total_travel_time_s"][0] == pytest.approx(1500 * expected)
    bins = run.link_bins()
    assert bins is not None and bins.pcu().sum() == pytest.approx(1500.0)
    assert bins.bins().tolist() == [28, 29, 30, 31], "07:00 to 08:00 in quarter hours"
    trips = run.trip_results()
    assert set(trips["departure_s"].tolist()) == {25_200}, "everyone departs at the window's start"
    # A skim after the run reads the equilibrium's times, the same at any hour.
    for hour in (7, 12):
        skim = run.skim("car", [A], [B], departure_s=hour * 3600)
        assert skim[0, 0] == pytest.approx(expected, rel=1e-9)
    # Half the demand over the same hour: half the flow, its own BPR time.
    half = ms.Scenario.from_parts(
        net, trips_over_the_hour(750), classes=CAR, flow_level=1, equilibration="frank_wolfe"
    ).run("one-link-half", quiet=True)
    assert half.mean_travel_time_s == pytest.approx(bpr(t0, 750.0, 1000.0), abs=1.0)
    # The period given by name: the same 1 500 trips over half an hour are twice the flow.
    short = ms.Scenario.from_parts(
        net,
        trips_over_the_hour(1500),
        classes=CAR,
        flow_level=1,
        equilibration="frank_wolfe",
        loading_options={"period_s": 1800},
    ).run("one-link-short", quiet=True)
    assert short.mean_travel_time_s == pytest.approx(bpr(t0, 3000.0, 1000.0), abs=1.0)


def test_the_helpers_make_the_demand_static_and_change_nothing_on_a_static_one() -> None:
    zones = {"za": A, "zb": B}
    near = [
        ("p", 0, A[0] + 0.001, A[1], B[0] - 0.001, B[1] + 0.0005, 25_260, "commuter", None),
        ("q", 0, *A, *B, 26_000, "commuter", None, "car"),
    ]
    moved = ms.demand_aggregate_space(near, zones, quiet=True)
    assert [(t.origin_lon, t.origin_lat, t.destination_lon, t.destination_lat) for t in moved] == [
        (*A, *B),
        (*A, *B),
    ]
    assert moved[1].mode == "car" and moved[0].departure_time_s == 25_260, "nothing else moves"
    assert ms.demand_aggregate_space(moved, zones, quiet=True) == moved
    at_start = ms.demand_aggregate_time(near)
    assert [t.departure_time_s for t in at_start] == [25_200, 25_200], "07:01 rounds down to 07:00"
    assert [t.departure_time_s for t in ms.demand_aggregate_time(near, "07:30")] == [27_000] * 2
    with pytest.raises(ValueError, match="zone"):
        ms.demand_aggregate_space(near, {"z": (200.0, 0.0)})
    # A run at level 1 does the same itself: the scenario's zones move the trip ends.
    net = one_link()
    rows = [(f"p{i}", 0, A[0] + 0.0004, A[1], B[0] - 0.0004, B[1], 25_200 + 7 * i, "commuter",
             None) for i in range(200)]  # fmt: skip
    a = ms.Scenario.from_parts(net, rows, classes=CAR, flow_level=1, zones=zones)
    b = ms.Scenario.from_parts(
        net,
        ms.demand_aggregate_time(ms.demand_aggregate_space(rows, zones, quiet=True), 25_200),
        classes=CAR,
        flow_level=1,
        loading_options={"period_s": 1800},
    )
    # The same trips once aggregated (the period given, since b's departures are one instant).
    assert a.run("a", quiet=True).mean_travel_time_s == pytest.approx(
        b.run("b", quiet=True).mean_travel_time_s
    )
    # The zones are read at level 1 alone: another level's fingerprint does not see them.
    two = ms.Scenario.from_parts(net, rows, classes=CAR, flow_level=2)
    assert two._fingerprint() == two.replace(zones=zones)._fingerprint()


def test_frank_wolfe_and_msa_agree_and_each_level_refuses_what_it_cannot_run() -> None:
    net = ms.examples.toy_network()
    rows = ms.examples.trips_random(net, 400, seed=3, min_m=300.0, max_m=2500.0, spread_s=3600)
    fw = ms.Scenario.from_parts(
        net,
        rows,
        flow_level=1,
        equilibration="frank_wolfe",
        equilibration_options={"gap_tolerance": 1e-6, "iterations": 5000},
    ).run("fw", quiet=True)
    msa = ms.Scenario.from_parts(
        net,
        rows,
        flow_level=1,
        equilibration="msa",
        equilibration_options={"gap_tolerance": 0.0, "iterations": 1000},
    ).run("msa", quiet=True)
    assert fw.converged and fw.convergence()["gap"][-1] < 1e-6
    assert msa.convergence()["gap"][-1] < 1e-2
    assert msa.mean_travel_time_s == pytest.approx(fw.mean_travel_time_s, rel=0.01)
    # Free flow, the static equilibrium: level 1 is no faster than level 0.
    free = ms.Scenario.from_parts(net, rows, flow_level=0, equilibration="free_flow").run(
        "free", quiet=True
    )
    assert fw.mean_travel_time_s >= free.mean_travel_time_s - 1.0
    with pytest.raises(ValueError, match="flow_level=1"):
        ms.Scenario.from_parts(net, rows, flow_level=2, equilibration="frank_wolfe")
    with pytest.raises(ValueError, match="'sra' does not apply"):
        ms.Scenario.from_parts(net, rows, flow_level=1, equilibration="sra")
    with pytest.raises(ValueError, match="cannot simulate a timetable"):
        ms.Scenario.from_parts(net, rows, flow_level=1, transit=ms.examples.toy_network_transit())
    with pytest.raises(ValueError, match="cannot simulate mode choice"):
        ms.Scenario.from_parts(net, rows, flow_level=1, modes=["car", "walk"])
    closed = [{"links": [0], "capacity_factor": 0.0, "from_s": 25_200, "to_s": 28_800}]
    with pytest.raises(ValueError, match="do not know of"):
        ms.Scenario.from_parts(net, rows, flow_level=1, disruptions=closed).run(
            "unknown", quiet=True
        )
    with pytest.raises(ValueError, match="bpr_beta"):
        ms.Scenario.from_parts(net, rows, flow_level=1, loading_options={"bpr_beta": 0.5}).run(
            "steep", quiet=True
        )


def test_level_one_gives_the_same_results_on_one_thread_and_on_seven(tmp_path) -> None:
    net = ms.examples.toy_network()
    rows = ms.examples.trips_random(net, 300, seed=5, min_m=300.0, max_m=2500.0, spread_s=3600)
    scenario = ms.Scenario.from_parts(
        net, rows, flow_level=1, equilibration="frank_wolfe", link_bin_s=900
    )
    one = scenario.run("t1", str(tmp_path / "one"), quiet=True, threads=1)
    seven = scenario.run("t7", str(tmp_path / "seven"), quiet=True, threads=7)
    assert one.fingerprint == seven.fingerprint
    for name in ("trip", "travel_time_s", "arrival_s"):
        np.testing.assert_array_equal(one.trip_results()[name], seven.trip_results()[name])
    a, b = one.link_bins(), seven.link_bins()
    assert a is not None and b is not None and len(a) > 0
    for name in ("links", "bins", "pcu", "pcu_seconds", "crossings"):
        np.testing.assert_array_equal(getattr(a, name)(), getattr(b, name)())


#: TNTP's best-known user equilibrium of Sioux Falls (bstabler/TransportationNetworks,
#: SiouxFalls_flow.tntp): each link's flow in veh/h, in the network's link order, and the total
#: travel time, veh·min.
TNTP_FLOWS = [
    4494.7, 8119.1, 4519.1, 5967.3, 8094.7, 14006.4, 10022.3, 14030.6, 18006.4, 5200.0, 18030.6,
    8798.3, 15780.8, 5991.8, 8806.5, 12492.9, 12101.5, 15794.0, 12525.6, 12040.9, 6882.7, 8388.7,
    15796.7, 6836.7, 21744.1, 21814.1, 17726.6, 23125.8, 11047.1, 8100.0, 5300.0, 17604.2, 8365.3,
    9776.1, 9973.7, 8404.9, 12287.6, 12378.6, 11121.4, 9814.1, 9036.3, 8400.4, 23192.3, 9079.8,
    19083.3, 18409.9, 8406.7, 11073.0, 11695.0, 15278.3, 8100.0, 11683.8, 9953.0, 15854.6,
    15333.4, 18976.8, 19116.7, 9941.9, 8688.4, 18992.5, 8710.6, 6302.0, 7000.0, 6240.0, 8619.5,
    10309.4, 18386.5, 7000.0, 8607.4, 9661.8, 8394.9, 9626.2, 7903.0, 11112.4, 10259.5, 7861.8,
]  # fmt: skip
TNTP_TSTT_VEH_MIN = 7_480_225.3


@pytest.mark.skipif(
    not os.environ.get("OPENMOBISIM_DATA"), reason="set OPENMOBISIM_DATA to the data bundle"
)
def test_sioux_falls_matches_tntp_s_best_known_equilibrium() -> None:
    case = ms.examples.case("siouxfalls")
    # The bundle's demand is half TNTP's over one hour: twice it is TNTP's.
    trips = case.demand(multiple=2.0)
    run = ms.Scenario.from_parts(
        case.network(),
        trips,
        classes=case.classes(),
        zones=case.zones(),
        flow_level=1,
        equilibration="frank_wolfe",
        equilibration_options={"gap_tolerance": 1e-4, "iterations": 100_000},
        link_bin_s=3600,
    ).run("sf-tntp", quiet=True)
    assert run.converged
    tstt = run.convergence()["total_travel_time_s"][-1] / 60
    assert tstt == pytest.approx(TNTP_TSTT_VEH_MIN, rel=0.01)
    bins = run.link_bins()
    assert bins is not None
    flows = np.zeros(len(TNTP_FLOWS))
    np.add.at(flows, bins.links(), bins.pcu())
    tntp = np.asarray(TNTP_FLOWS)
    assert np.abs(flows - tntp).sum() / tntp.sum() < 0.01, "within 1 % over the links"
    assert math.isclose(len(trips), run.completion["completed"])
