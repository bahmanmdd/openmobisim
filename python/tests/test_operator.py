"""Operators on a fleet day: built-in and Python policies on a 5 x 5 grid of 200 m blocks."""

from __future__ import annotations

import math

import numpy as np
import openmobisim as ms
import pytest
from openmobisim.operator import Move, Target


def grid() -> object:
    return ms.examples.manhattan_grid(5, 200.0, False)


def at(net: object, row: int, col: int) -> tuple[float, float]:
    return ms._core.grid_node_lonlat(net, row, col)


def trip(net, ident, t, a, b, service=None) -> dict:
    o, d = at(net, *a), at(net, *b)
    row = {
        "trip_id": ident,
        "origin_lon": o[0],
        "origin_lat": o[1],
        "destination_lon": d[0],
        "destination_lat": d[1],
        "departure": t,
    }
    if service is not None:
        row["service"] = service
    return row


PLACES = {"a": (0, 0), "b": (0, 4), "c": (4, 0), "z": (4, 4)}


def docks(net, bikes=(4, 0, 2, 0)) -> object:
    return ms.dock_read_table(
        [
            {"dock_id": k, "lon": at(net, *p)[0], "lat": at(net, *p)[1], "capacity": 4, "bikes": b}
            for (k, p), b in zip(PLACES.items(), bikes, strict=True)
        ]
    )


def day(policy, net=None, bikes=(4, 0, 2, 0), seed=0, **kw) -> ms.FleetDay:
    net = net or grid()
    rows = [
        trip(net, f"t{k}", 8 * 3600 + 120 * k, PLACES["abcz"[k % 4]], PLACES["zcba"[k % 4]])
        for k in range(12)
    ]
    service = ms.service_bike_docked(docks(net, bikes))
    return ms.fleet_day(
        net, [service], rows, window=("07:30", "09:30"), seed=seed, policy=policy, **kw
    )


class PyThreshold:
    """The built-in threshold rule, written in Python."""

    step_s = 900.0

    def __init__(self, low: float, high: float) -> None:
        """The rule's shares of a dock's slots."""
        self.low, self.high = low, high

    def describe(self) -> dict:
        """Its name and parameters."""
        return {"name": "py_threshold", "parameters": {"low": self.low, "high": self.high}}

    def step(self, view) -> list:
        """The targets of the docks past a threshold."""
        d = view["bike_share"]
        out = []
        for dock, bikes, slots in zip(d["dock_id"], d["bikes"], d["slots"], strict=True):
            mid = slots * (self.low + self.high) / 2
            if bikes < self.low * slots:
                out.append(Target("bike_share", dock, math.floor(mid)))
            elif bikes > self.high * slots:
                out.append(Target("bike_share", dock, math.ceil(mid)))
        return out


def same(a: ms.FleetDay, b: ms.FleetDay) -> None:
    da, db = a.decisions(), b.decisions()
    assert list(da) == list(db)
    for name in da:
        x, y = list(da[name]), list(db[name])
        assert len(x) == len(y), name
        for u, v in zip(x, y, strict=True):
            assert u == v or (isinstance(u, float) and math.isnan(u) and math.isnan(v)), name


def test_a_python_rule_equal_to_the_built_in_threshold_takes_the_same_decisions():
    rust = day(ms.operator.policy_threshold(0.3, 0.7))
    python = day(PyThreshold(0.3, 0.7))
    assert rust.policy == {
        "name": "threshold",
        "parameters": {"low": 0.3, "high": 0.7, "step_s": 900.0},
    }
    assert python.policy == {"name": "py_threshold", "parameters": {"low": 0.3, "high": 0.7}}
    d = rust.decisions()
    assert d.row_count > 0 and any(d["from_target"]) and any(d["moved"] > 0), "bikes moved"
    same(rust, python)
    assert rust.trips["status"] == python.trips["status"]
    assert rust.counts["invariant_violations"] == 0


def test_a_refused_decision_comes_back_with_its_reason():
    class Once:
        def step(self, view):
            if view.step > 0:
                return []
            # b starts empty: refused; a holds 4: two leave.
            return [Move("bike_share", "b", "a", 1), ("move", "bike_share", "a", "z", 2)]

    d = day(Once()).decisions()
    assert d["kind"] == ["move", "move"]
    assert d["accepted"] == [False, True]
    assert d["reason"] == ["no bike at the first dock", None]
    assert d["from_dock"] == ["b", "a"] and d["to_dock"] == ["a", "z"]
    assert list(d["moved"]) == [0, 2]
    assert math.isnan(d["executed_s"][0]) and d["executed_s"][1] == 7.5 * 3600
    assert d["finished_s"][1] > d["executed_s"][1]


def test_a_view_kept_after_its_step_refuses_to_be_read():
    kept = []

    class Keeper:
        def step(self, view):
            assert view["bike_share"]["dock_id"] == list(PLACES)
            assert view.t == 7.5 * 3600 + 900 * view.step
            kept.append(view)
            return []

    day(Keeper())
    assert len(kept) == 8, "every 15 minutes over two hours"
    with pytest.raises(RuntimeError, match="step 0, which has ended"):
        kept[0]["bike_share"]
    with pytest.raises(RuntimeError, match="has ended"):
        _ = kept[-1].t


def test_a_policy_that_raises_stops_the_day_with_its_message():
    class Broken:
        def step(self, view):
            if view.step == 2:
                raise ArithmeticError("the van broke down")
            return []

    with pytest.raises(ArithmeticError, match="the van broke down"):
        day(Broken())
    with pytest.raises(ValueError, match='no dock "nowhere" in service'):
        day(type("Lost", (), {"step": lambda self, v: [Target("bike_share", "nowhere", 1)]})())


def test_a_registered_name_and_a_built_in_name_work():
    ms.policy_register("py_threshold", lambda o: PyThreshold(o.get("low", 0.3), o.get("high", 0.7)))
    by_name = day("py_threshold", policy_options={"low": 0.3, "high": 0.7})
    same(by_name, day(PyThreshold(0.3, 0.7)))
    same(day("threshold", policy_options={"low": 0.3, "high": 0.7}), by_name)
    assert day("noop").decisions().row_count == 0
    with pytest.raises(ValueError, match="unknown policy 'nope'"):
        day("nope")
    with pytest.raises(ValueError, match="built-in policy"):
        ms.policy_register("reset", PyThreshold)
    with pytest.raises(ValueError, match="low < high"):
        day(ms.operator.policy_threshold(0.8, 0.2))


def test_the_same_seed_gives_the_same_day():
    class Random:
        step_s = 600.0

        def step(self, view):
            rng = np.random.default_rng(view.rng_seed)
            d = view["bike_share"]
            i, j = rng.choice(len(d["dock_id"]), size=2, replace=False)
            return [Move("bike_share", d["dock_id"][i], d["dock_id"][j], int(rng.integers(1, 3)))]

    a, b, c = day(Random(), seed=1), day(Random(), seed=1), day(Random(), seed=2)
    same(a, b)
    assert a.trips["status"] == b.trips["status"]
    assert a.decisions()["from_dock"] != c.decisions()["from_dock"], "another seed, another day"
    assert a.policy == {"name": "python:Random", "parameters": {}}


def test_reset_and_charge_rule_run_by_name_on_a_fleet_day():
    net = grid()
    r = day(ms.operator.policy_reset(from_s=8.5 * 3600), net=net)
    d = r.decisions()
    assert d.row_count > 0 and min(d["t_s"]) >= 8.5 * 3600
    v = ms.vehicle_read_table(
        [{"vehicle_id": "v1", "lon": at(net, 0, 0)[0], "lat": at(net, 0, 0)[1], "soc": 0.3}]
    )
    chargers = ms.charger_read_table(
        [
            {"charger_id": "far", "lon": at(net, 4, 4)[0], "lat": at(net, 4, 4)[1], "plugs": 1},
            {"charger_id": "near", "lon": at(net, 0, 2)[0], "lat": at(net, 0, 2)[1], "plugs": 1},
        ]
    )
    taxis = ms.service_taxi(v, chargers)
    t = ms.fleet_day(net, [taxis], [], policy="charge_rule", policy_options={"charge_below": 0.5})
    d = t.decisions()
    assert d["kind"] == ["charge"] and d["vehicle"] == ["v1"] and d["charger"] == ["near"]
    assert d["accepted"] == [True] and list(d["to_soc"]) == [0.8]
