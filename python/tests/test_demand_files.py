"""Demand from files: ``zones.csv``, ``od.csv`` and ``trips.csv``, and an OD matrix made trips.

``demand_from_od`` keeps each group's total, never depends on the order of the rows, keeps its
departures in their window and its ends within ``spread_m`` of their zone's point.
"""

from __future__ import annotations

import math
from collections import Counter
from pathlib import Path

import numpy as np
import openmobisim as ms
import pytest

ZONES = {"a": (4.90, 52.37), "b": (4.92, 52.37), "c": (4.91, 52.38)}


def write(path: Path, text: str) -> Path:
    path.write_text(text)
    return path


def od_rows(scale: float = 1.0) -> list[dict]:
    cells = [("a", "b", 10.4), ("a", "c", 3.3), ("b", "c", 2.2), ("c", "a", 0.6), ("b", "a", 0.5)]
    return [{"origin_zone": o, "destination_zone": d, "trips": n * scale} for o, d, n in cells]


def metres(p: tuple[float, float], q: tuple[float, float]) -> float:
    dx = (p[0] - q[0]) * 111_320.0 * math.cos(math.radians(q[1]))
    dy = (p[1] - q[1]) * 110_574.0
    return math.hypot(dx, dy)


def test_zones_and_od_files_are_read_by_their_columns(tmp_path: Path) -> None:
    zones = write(
        tmp_path / "zones.csv", "zone_id,name,x_coord,y_coord\nA,x,4.9,52.37\nB,,4.92,52.37\n"
    )
    assert ms.demand_read_zones(zones) == {"A": (4.9, 52.37), "B": (4.92, 52.37)}
    other = write(tmp_path / "z2.csv", "zone_id,lon,lat\n1,4.9,52.37\n")
    assert ms.demand_read_zones(other) == {"1": (4.9, 52.37)}
    od = write(
        tmp_path / "od.csv",
        "origin_zone,destination_zone,trips,start_s,end_s,user_class,mode\n"
        "A,B,12.5,25200,28800,commuter,bike\nB,A,3,,,,\n",
    )
    rows = ms.demand_read_od(od)
    assert rows[0] == {
        "origin_zone": "A", "destination_zone": "B", "trips": 12.5, "start_s": 25200,
        "end_s": 28800, "user_class": "commuter", "mode": "bike",
    }  # fmt: skip
    assert rows[1]["start_s"] is None and rows[1]["mode"] is None and rows[1]["trips"] == 3.0


def test_bad_files_are_refused_with_the_row(tmp_path: Path) -> None:
    with pytest.raises(ValueError, match="row 2 needs zone_id"):
        ms.demand_read_zones(write(tmp_path / "z.csv", "zone_id,x_coord,y_coord\nA,1,2\nB,,2\n"))
    with pytest.raises(ValueError, match="listed twice"):
        ms.demand_read_zones(write(tmp_path / "z.csv", "zone_id,x_coord,y_coord\nA,1,2\nA,1,2\n"))
    with pytest.raises(ValueError, match="row 1 needs origin_zone"):
        ms.demand_read_od(write(tmp_path / "o.csv", "origin_zone,destination_zone,trips\nA,,3\n"))
    with pytest.raises(ValueError, match="negative"):
        ms.demand_read_od(write(tmp_path / "o.csv", "origin_zone,destination_zone,trips\nA,B,-1\n"))
    with pytest.raises(ValueError, match="row 1"):
        ms.demand_read_trips(write(tmp_path / "t.csv", "traveller_id,trip_seq\nx,0\n"))


def test_a_trips_file_comes_back_as_written(tmp_path: Path) -> None:
    rows = [
        ("p1", 0, 4.9, 52.37, 4.92, 52.37, 25200, "default", None, None),
        ("p1", 1, 4.92, 52.37, 4.9, 52.37, 61200, "default", 3, "bike"),
    ]
    path = tmp_path / "trips.csv"
    assert ms.demand_write_trips(rows, path) == 2
    assert path.read_text().splitlines()[0] == ",".join(ms.demand.TRIP_COLUMNS)
    assert ms.demand_read_trips(path) == rows
    # Nine fields (no mode) are written with an empty mode.
    ms.demand_write_trips([rows[0][:9]], path)
    assert ms.demand_read_trips(path) == [rows[0]]


def test_od_totals_are_kept_and_the_largest_remainders_round_up() -> None:
    trips = ms.demand_from_od(od_rows(), ZONES)
    per_cell = Counter((t[2] < 4.905, t[4] > 4.915) for t in trips)
    assert len(trips) == round(17.0), "10.4 + 3.3 + 2.2 + 0.6 + 0.5 = 17"
    # Whole parts 10, 3, 2, 0, 0 = 15; the two largest remainders (0.6 and 0.5) get one each.
    assert per_cell[(True, True)] == 10  # a -> b
    assert sum(1 for t in trips if t[0] and metres((t[4], t[5]), ZONES["a"]) < 400) == 2
    assert len({t[0] for t in trips}) == len(trips) and trips[0][0] == "od0000001"
    assert all(t[1] == 0 and t[7] == "default" and t[8] is None and t[9] is None for t in trips)


def test_od_trips_never_depend_on_the_order_of_the_rows() -> None:
    rows = od_rows(7.3)
    a = ms.demand_from_od(rows, ZONES, seed=4)
    b = ms.demand_from_od(list(reversed(rows)), dict(reversed(list(ZONES.items()))), seed=4)
    assert a == b
    assert ms.demand_from_od(rows, ZONES, seed=5) != a


def test_departures_stay_in_their_window_and_ends_near_their_zone() -> None:
    rows = od_rows(40.0) + [
        {"origin_zone": "c", "destination_zone": "b", "trips": 50, "start_s": 0, "end_s": 600,
         "user_class": "night", "mode": "bike"},
    ]  # fmt: skip
    trips = ms.demand_from_od(rows, ZONES, profile="am_peak", spread_m=250.0)
    night = [t for t in trips if t[7] == "night"]
    assert len(night) == 50 and {t[9] for t in night} == {"bike"}
    assert all(0 <= t[6] < 600 for t in night)
    day = np.array([t[6] for t in trips if t[7] == "default"])
    assert day.min() >= 7 * 3600 and day.max() < 9 * 3600
    # A bell: two thirds in the window's middle 40% (Beta(3, 3) puts 0.6742 there).
    middle = ((day >= 7 * 3600 + 2160) & (day < 9 * 3600 - 2160)).mean()
    assert 0.62 < middle < 0.73
    for t in trips:
        o = min(ZONES.values(), key=lambda z: metres((t[2], t[3]), z))
        assert metres((t[2], t[3]), o) <= 250.0 + 1e-6
    uniform = np.array([t[6] for t in ms.demand_from_od(od_rows(40.0), ZONES)])
    assert 0.33 < ((uniform >= 7 * 3600 + 2160) & (uniform < 9 * 3600 - 2160)).mean() < 0.47


def test_od_refusals() -> None:
    with pytest.raises(ValueError, match="profile"):
        ms.demand_from_od(od_rows(), ZONES, profile="pm_peak")
    with pytest.raises(ValueError, match="zone 'x'"):
        ms.demand_from_od([{"origin_zone": "x", "destination_zone": "a", "trips": 1}], ZONES)
    with pytest.raises(ValueError, match="empty departure window"):
        ms.demand_from_od(od_rows(), ZONES, start_s=100, end_s=100)
    with pytest.raises(ValueError, match="spread_m"):
        ms.demand_from_od(od_rows(), ZONES, spread_m=-1)


def test_od_files_run_on_the_toy_network(tmp_path: Path) -> None:
    net = ms.examples.toy_network()
    zones = {name: net.node_lonlat(name) for name in ("W", "M", "D2")}
    lines = ["zone_id,x_coord,y_coord"] + [f"{z},{p[0]},{p[1]}" for z, p in zones.items()]
    write(tmp_path / "zones.csv", "\n".join(lines) + "\n")
    write(tmp_path / "od.csv", "origin_zone,destination_zone,trips\nW,M,6\nW,D2,4\n")
    trips = ms.demand_from_od(tmp_path / "od.csv", tmp_path / "zones.csv", spread_m=0.0)
    ms.demand_write_trips(trips, tmp_path / "trips.csv")
    run = ms.Scenario.from_parts(
        network=net,
        demand=tmp_path / "trips.csv",
        classes={"default": (True, False, False)},
        equilibration="free_flow",
        flow_level=0,
    ).run(run_id="od-toy")
    assert run.completion["completed"] == 10
