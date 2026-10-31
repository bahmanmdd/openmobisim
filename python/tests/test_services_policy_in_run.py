"""An operator steering a run's service day (``Scenario.run(policy=...)``).

The grid and travellers of ``test_services_in_runs``; docks left unbalanced (some empty, some
full) so that the threshold rule has bikes to move. The run's day with a policy is the day its
frozen copy replays with the same policy: the same decisions, the same trips, the same docks.
"""

from __future__ import annotations

import json
import math
from pathlib import Path

import openmobisim as ms
import pytest
from test_operator import PyThreshold
from test_services_frozen import same_columns
from test_services_in_runs import at, grid, scenario, trips


def unbalanced(net: object) -> object:
    places = {
        "a": ((0, 0), 0),
        "b": ((0, 1), 8),
        "y": ((9, 8), 0),
        "z": ((9, 9), 8),
        "m": ((5, 5), 4),
    }
    return ms.dock_read_table(
        [
            {"dock_id": k, "lon": at(net, *p)[0], "lat": at(net, *p)[1], "capacity": 8, "bikes": b}
            for k, (p, b) in places.items()
        ]
    )


def setup() -> tuple[object, object, ms.Scenario]:
    net = grid()
    bikes = ms.service_bike_docked(unbalanced(net))
    rows = trips(net, 20, (0, 0), (9, 9)) + trips(net, 10, (9, 9), (0, 1), "back")
    classes = {"c": (False, False, False), "back": (False, False, False)}
    return net, bikes, scenario(net, rows, services=[bikes], classes=classes)


def same_decisions(a: ms.Columns, b: ms.Columns) -> None:
    assert list(a) == list(b)
    for name in a:
        x, y = list(a[name]), list(b[name])
        assert len(x) == len(y), name
        for u, v in zip(x, y, strict=True):
            assert u == v or (isinstance(u, float) and math.isnan(u) and math.isnan(v)), name


def replays(run: ms.Run, net: object, bikes: object, folder: Path, policy: object) -> ms.FleetDay:
    """The run's frozen day replayed with `policy`, checked equal to the run's own day."""
    run.freeze(folder)
    day = ms.fleet_day(net, [bikes], frozen=folder, policy=policy)
    mine = run._share_day()
    assert same_columns(mine["trips"], day.trips) == []
    for name, bins in mine["dock_bins"].items():
        assert same_columns(bins, day.dock_bins[name]) == [], name
    same_decisions(run.operator_decisions(), day.decisions())
    return day


def test_a_built_in_policy_steers_the_run_s_day_as_it_steers_its_frozen_day(
    tmp_path: Path,
) -> None:
    net, bikes, s = setup()
    rule = ms.operator.policy_threshold(0.3, 0.7)
    run = s.run("steered", output_dir=str(tmp_path / "run"), quiet=True, policy=rule)
    replays(run, net, bikes, tmp_path / "day", rule)
    d = run.operator_decisions()
    assert d.row_count > 0 and any(d["accepted"]), "the rule moved bikes"
    assert run.service_policy() == {
        "name": "threshold",
        "parameters": {"low": 0.3, "high": 0.7, "step_s": 900.0},
    }
    manifest = json.loads((tmp_path / "run" / "manifest.json").read_text())
    assert manifest["service_policy"] == run.service_policy()
    # Without a policy, the same run's day moves no bike.
    plain = s.run("plain", output_dir=str(tmp_path / "plain"), quiet=True)
    assert plain.operator_decisions() is None and plain.service_policy() is None
    assert "service_policy" not in json.loads((tmp_path / "plain" / "manifest.json").read_text())
    # The decisions are written to operator_tasks.parquet, by a run with an operator only.
    assert (tmp_path / "run" / "operator_tasks.parquet").exists()
    assert not (tmp_path / "plain" / "operator_tasks.parquet").exists()
    assert plain.operator_tasks_table() is None
    # Opening a run's folder reads its parquet tables, the operator's decisions among them:
    # the same columns, in the same order, of the same types, with the same values.
    pq = pytest.importorskip("pyarrow.parquet")
    opened = ms.Run.open(tmp_path / "run")
    assert opened.service_policy() == run.service_policy()
    back, live = opened.operator_decisions(), run.operator_decisions()
    same_decisions(back, live)
    for name in live:
        assert type(back[name]) is type(live[name]), name
        assert getattr(back[name], "dtype", None) == getattr(live[name], "dtype", None), name
    table = pq.read_table(run.operator_tasks_table().path)
    assert table.column_names == ["run_id", *live]
    assert set(table.column("run_id").to_pylist()) == {"steered"}
    assert ms.Run.open(tmp_path / "plain").operator_decisions() is None


def test_a_python_policy_in_a_run_takes_the_decisions_it_takes_on_the_frozen_day(
    tmp_path: Path,
) -> None:
    net, bikes, s = setup()
    run = s.run("py", output_dir=str(tmp_path / "run"), quiet=True, policy=PyThreshold(0.3, 0.7))
    replays(run, net, bikes, tmp_path / "day", PyThreshold(0.3, 0.7))
    assert run.service_policy()["name"] == "py_threshold"
    # The built-in rule it mirrors takes the same decisions.
    built_in = s.run(
        "rs",
        output_dir=str(tmp_path / "rs"),
        quiet=True,
        policy="threshold",
        policy_options={"low": 0.3, "high": 0.7},
    )
    a, b = run.operator_decisions(), built_in.operator_decisions()
    assert list(a["kind"]) == list(b["kind"]) and list(a["accepted"]) == list(b["accepted"])


def test_the_policy_and_its_options_are_in_the_fingerprint(tmp_path: Path) -> None:
    _, _, s = setup()
    runs = {
        name: s.run(name, output_dir=str(tmp_path / name), quiet=True, **kw).fingerprint
        for name, kw in {
            "none": {},
            "low_02": {"policy": "threshold", "policy_options": {"low": 0.2}},
            "low_03": {"policy": "threshold", "policy_options": {"low": 0.3}},
            "reset": {"policy": "reset"},
        }.items()
    }
    assert len(set(runs.values())) == 4


def test_a_policy_without_a_service_or_a_failing_one_is_refused(tmp_path: Path) -> None:
    net = grid()
    plain = scenario(net, trips(net, 3, (0, 0), (9, 9)), modes=["walk"])
    with pytest.raises(ValueError, match="give services too"):
        plain.run("x", output_dir=str(tmp_path / "x"), quiet=True, policy="threshold")
    with pytest.raises(ValueError, match="unknown policy"):
        plain.run("y", output_dir=str(tmp_path / "y"), quiet=True, policy="no_such_rule")

    class Broken:
        def step(self, view: object) -> list:
            raise RuntimeError("the operator's own bug")

    _, _, s = setup()
    with pytest.raises(RuntimeError, match="the operator's own bug"):
        s.run("z", output_dir=str(tmp_path / "z"), quiet=True, policy=Broken())
