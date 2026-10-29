"""The results of a run with a docked bike service: its legs, its docks bin by bin, its KPI rows.

``service_legs.parquet`` (one row per shared-bike leg of the last loading) and
``hub_bins.parquet`` (one row per dock and time bin) are written only by a run with a service,
and their counts agree with each other and with ``kpis.parquet``'s service rows. A run without a
service writes neither and has no service rows. The network: the 10 x 10 grid of
``test_services_in_runs``, with few bikes, so that pickups fail and travellers are redirected.
"""

from __future__ import annotations

from pathlib import Path

import numpy as np
import openmobisim as ms
import pytest
from test_services_in_runs import docks, grid, scenario, trips

pq = pytest.importorskip("pyarrow.parquet")

MSA = {"equilibration": "msa", "equilibration_options": {"iterations": 4, "gap_tolerance": 0}}


@pytest.fixture(scope="module")
def run(tmp_path_factory: pytest.TempPathFactory) -> ms.Run:
    net = grid()
    rows = trips(net, 12, (0, 0), (9, 9), "a") + trips(net, 6, (9, 9), (0, 1), "b")
    classes = {"a": (False, False, False), "b": (False, False, False)}
    s = scenario(
        net,
        rows,
        services=[ms.service_bike_docked(docks(net, bikes=2))],
        classes=classes,
        choice_model="logit",
        **MSA,
    )
    return s.run("share-results", output_dir=str(tmp_path_factory.mktemp("r")), quiet=True)


def _kpi(run: ms.Run, metric: str) -> list[tuple[int, float]]:
    k = pq.read_table(run._summary.kpis_path).to_pylist()
    return [(r["iteration"], r["value"]) for r in k if r["metric"] == metric]


def test_the_files_hold_the_tables_and_their_counts_agree(run: ms.Run) -> None:
    legs, bins = run.service_legs(), run.hub_bins()
    assert legs.row_count > 0 and bins.row_count > 0
    # The files hold the same tables, after run_id.
    for columns, table in ((legs, run.service_legs_table()), (bins, run.hub_bins_table())):
        t = pq.read_table(table.path)
        assert t.column_names[1:] == list(columns)
        assert t.num_rows == columns.row_count
    w = np.asarray(legs["weight"], dtype=float)
    picked = np.isfinite(legs["picked_s"])
    failed = np.array([o == "failed" for o in legs["outcome"]])
    returned = np.array([d is not None for d in legs["dock_to"]])
    assert failed.any() and picked.any()
    # Pickups, returns and failures: the docks' counts are the legs'.
    assert bins["pickups"].sum() == pytest.approx(w[picked].sum())
    assert bins["returns"].sum() == pytest.approx(w[returned].sum())
    assert bins["failed"].sum() == pytest.approx(w[failed].sum())
    # Every traveller sent on from one dock is sent to another.
    assert bins["redirected_in"].sum() == pytest.approx(bins["redirected_out"].sum())
    assert bins["redirected_out"].sum() > 0
    # No operator in a run: nothing moved.
    assert not bins["moved_in"].any() and not bins["moved_out"].any()
    # A failed leg took no bike; a served one took and returned it.
    assert not picked[failed].any()
    served = np.array([o == "served" for o in legs["outcome"]])
    assert picked[served].all() and returned[served].all()
    assert set(legs["mode"]) == {"bike_share"}
    assert set(bins["resource_kind"]) == {"dock"}


def test_the_kpi_rows_are_the_legs(run: ms.Run) -> None:
    legs = run.service_legs()
    w = np.asarray(legs["weight"], dtype=float)
    met = np.asarray(legs["met_dock_s"])
    arrived = np.isfinite(met)
    diff = met[arrived] - np.asarray(legs["expected_dock_s"])[arrived]
    last = max(i for i, _ in _kpi(run, "service_legs"))
    rows = {m: dict(_kpi(run, m)) for m in ("service_legs", "service_bias_s", "service_mismatch_s")}
    # One row per iteration for these three; the last loading's are the legs'.
    assert len(rows["service_legs"]) == 4
    assert rows["service_legs"][last] == pytest.approx(w.sum())
    assert rows["service_bias_s"][last] == pytest.approx(
        np.sum(w[arrived] * diff) / w[arrived].sum()
    )
    assert rows["service_mismatch_s"][last] == pytest.approx(
        np.sum(w[arrived] * np.abs(diff)) / w[arrived].sum()
    )
    failed = w[np.array([o == "failed" for o in legs["outcome"]])].sum()
    assert dict(_kpi(run, "service_failed"))[last] == pytest.approx(failed)
    assert dict(_kpi(run, "service_failed_share"))[last] == pytest.approx(failed / w.sum())
    assert dict(_kpi(run, "service_extra_walk_mean_s"))[last] == pytest.approx(
        np.sum(w * legs["extra_walk_s"]) / w.sum()
    )
    assert dict(_kpi(run, "service_wait_mean_s"))[last] == 0.0
    assert dict(_kpi(run, "service_fleet"))[last] == 10.0  # five docks of two bikes
    # The service's rows are under its name.
    k = pq.read_table(run._summary.kpis_path).to_pylist()
    assert {r["mode"] for r in k if r["metric"].startswith("service_")} == {"bike_share"}


def test_the_docks_keep_their_bikes(run: ms.Run) -> None:
    """At each bin's end, the bikes docked and ridden are the fleet; docked never below 0."""
    bins = run.hub_bins()
    assert (np.asarray(bins["level_min"]) >= 0).all()
    assert (np.asarray(bins["level_mean"]) >= np.asarray(bins["level_min"]) - 1e-9).all()
    places = run.hub_places()
    assert sorted(places["resource_id"]) == ["a", "b", "m", "y", "z"]
    assert int(np.sum(places["bikes_start"])) == 10


def test_a_run_without_a_service_writes_none_of_it(tmp_path: Path) -> None:
    net = grid()
    rows = trips(net, 3, (0, 0), (1, 0))
    out = tmp_path / "none"
    r = scenario(net, rows, modes=["walk"]).run("none", output_dir=str(out), quiet=True)
    assert r.service_legs() is None and r.hub_bins() is None and r.hub_places() is None
    assert r.service_legs_table() is None and r.hub_bins_table() is None
    assert not (out / "service_legs.parquet").exists() and not (out / "hub_bins.parquet").exists()
    k = pq.read_table(out / "kpis.parquet").to_pylist()
    assert not [r for r in k if r["metric"].startswith("service_")]


def test_a_run_with_a_service_and_no_leg_keeps_its_docks(tmp_path: Path) -> None:
    """A service nobody took: no leg, the docks at their start-of-day bikes all day."""
    net = grid()
    rows = trips(net, 2, (0, 0), (1, 0))  # 200 m: walking is faster
    s = scenario(
        net,
        rows,
        services=[ms.service_bike_docked(docks(net, bikes=3))],
        choice_model="deterministic",
    )
    r = s.run("idle", output_dir=str(tmp_path / "idle"), quiet=True)
    assert r.service_legs().row_count == 0
    bins = r.hub_bins()
    assert bins.row_count > 0 and (np.asarray(bins["level_mean"]) == 3.0).all()
    assert pq.read_table(r.service_legs_table().path).num_rows == 0


def test_the_figures_draw(run: ms.Run, tmp_path: Path) -> None:
    pytest.importorskip("matplotlib")
    from openmobisim import viz

    for theme in ("paper", "night"):
        viz.map_hubs(run, "08:05", theme=theme, path=str(tmp_path / f"hubs_{theme}.png"))
        viz.chart_service(run, "bike_share", theme=theme, path=str(tmp_path / f"day_{theme}.png"))
        assert (tmp_path / f"hubs_{theme}.png").stat().st_size > 0
        assert (tmp_path / f"day_{theme}.png").stat().st_size > 0
    with pytest.raises(ValueError, match="outside"):
        viz.map_hubs(run, "23:00")
    with pytest.raises(ValueError, match="no service"):
        viz.chart_service(run, "taxi")
