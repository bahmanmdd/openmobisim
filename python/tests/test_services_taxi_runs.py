"""A taxi fleet in a run (S302, S303): its choice, service day, files, rows and refusal.

Offered to trips choosing their mode on fixed expectations, its requests played on the fleet
engine once, after the run has stopped; its three results files, its KPI rows; the refusal of a
taxi without a constant or a fare; ``has_taxi``; the presets and ``Run.service_met()``.

The network: the 10 x 10 grid of ``test_services_in_runs``; four vehicles at its first corner,
one charger at its centre. Travellers own no car and no bike and choose between walking and a
taxi.
"""

from __future__ import annotations

from collections import Counter

import numpy as np
import openmobisim as ms
import pytest
from test_services_in_runs import at, docks, grid, scenario, trips

pq = pytest.importorskip("pyarrow.parquet")

MSA = {"equilibration": "msa", "equilibration_options": {"iterations": 4, "gap_tolerance": 0}}
FARE = {"taxi_eur_per_ride": 0.5, "taxi_eur_per_km": 0.2}


def fleet(net: object, n: int = 4, soc: float = 0.8, **options: float) -> object:
    o = at(net, 0, 0)
    vehicles = ms.vehicle_read_table(
        [{"vehicle_id": f"v{i}", "lon": o[0], "lat": o[1], "soc": soc} for i in range(n)]
    )
    c = at(net, 5, 5)
    chargers = ms.charger_read_table([{"charger_id": "c", "lon": c[0], "lat": c[1], "plugs": 1}])
    return ms.service_taxi(vehicles, chargers, options=options)


def taxi_scenario(net: object, rows: list, **kwargs: object) -> ms.Scenario:
    kwargs.setdefault("services", [fleet(net)])
    kwargs.setdefault("modes", ["walk", "taxi"])
    kwargs.setdefault("price_options", FARE)
    kwargs.setdefault("choice_model", "logit")
    return scenario(net, rows, **kwargs)


@pytest.fixture(scope="module")
def run(tmp_path_factory: pytest.TempPathFactory) -> ms.Run:
    net = grid()
    rows = trips(net, 30, (0, 0), (9, 9), "a")
    # No hailing time: each vehicle comes back from the far corner within the 300 s wait for a
    # second trip and so drops below the charging threshold (with the 120 s hail, its reach of
    # 180 s is too short for the drive back, and nobody charges).
    taxis = fleet(net, soc=0.22, max_wait_s=300, hail_s=0)
    s = taxi_scenario(net, rows, services=[taxis], **MSA)
    return s.run("taxi-results", output_dir=str(tmp_path_factory.mktemp("r")), quiet=True)


def test_a_taxi_without_a_constant_or_a_fare_is_refused() -> None:
    net = grid()
    rows = trips(net, 2, (0, 0), (9, 9))
    with pytest.raises(ValueError, match="needs a constant or a fare"):
        taxi_scenario(net, rows, price_options=None)
    # A constant, run-wide or every class's, or a fare: accepted.
    taxi_scenario(net, rows, price_options=None, choice_options={"beta_mode_taxi": -1.0})
    taxi_scenario(
        net,
        rows,
        price_options=None,
        classes={"c": {"owns_car": 0, "owns_bike": 0, "beta_mode_taxi": -1.0}},
    )
    taxi_scenario(net, rows, price_options={"taxi_eur_per_km": 1.0})
    # Without a fleet the mode is refused, naming what it needs.
    with pytest.raises(ValueError, match="service_taxi"):
        taxi_scenario(net, rows, services=None)


def test_the_files_hold_the_tables_and_the_events_reconcile(run: ms.Run) -> None:
    for columns, table in (
        (run.taxi_events(), run.taxi_events_table()),
        (run.taxi_trips(), run.taxi_trips_table()),
        (run.taxi_vehicles(), run.taxi_vehicles_table()),
    ):
        t = pq.read_table(table.path)
        assert t.column_names[1:] == list(columns)
        assert t.num_rows == columns.row_count > 0
    ev, tr, vb = run.taxi_events(), run.taxi_trips(), run.taxi_vehicles()
    assert list(ev["time_s"]) == sorted(ev["time_s"])
    w = np.asarray(tr["travellers"], dtype=float)
    results: Counter[str] = Counter()
    for r, x in zip(tr["result"], w, strict=True):
        results[r] += x
    events = Counter(ev["event"])
    carried = results["as_planned"] + results["waited_longer_than_expected"]
    assert events["asked_for_taxi"] == w.sum()
    assert events["dropped_off"] == carried == np.sum(vb["passengers_dropped_off"])
    # A request with no vehicle within the longest wait ends there: no walk, no arrival.
    assert events["no_taxi_within_wait_limit"] == results["no_taxi"] > 0
    assert "reached_destination" not in events and "gave_up_walking_to_destination" not in events
    failed = np.asarray(tr["result"]) == "no_taxi"
    assert np.isnan(np.asarray(tr["arrived_s"], dtype=float)[failed]).all()
    km = Counter()
    for e, k in zip(ev["event"], ev["drive_km"], strict=True):
        if k == k:
            km[e] += k
    assert km["picked_up"] == pytest.approx(np.sum(vb["km_occupied"]))
    empty = km["taxi_assigned"] + km["vehicle_went_to_charge"]
    assert empty == pytest.approx(np.sum(vb["km_empty"]))
    seconds = sum(np.asarray(vb[c]) for c in vb if c.startswith("seconds_"))
    assert seconds == pytest.approx(np.asarray(vb["bin_end_s"]) - np.asarray(vb["bin_start_s"]))
    assert events["vehicle_went_to_charge"] > 0, "the vehicles start low and charge"


def test_the_kpi_rows_are_the_trips(run: ms.Run) -> None:
    k = pq.read_table(run._summary.kpis_path).to_pylist()
    rows = {(r["iteration"], r["metric"]): r["value"] for r in k if r["mode"] == "taxi"}
    last = max(i for i, _ in rows)
    # One row set of the service day, for the last iteration; one row per metric.
    assert len([r for r in k if r["mode"] == "taxi"]) == len(rows)
    assert {i for i, m in rows if m == "service_trips"} == {last}
    tr = run.taxi_trips()
    w = np.asarray(tr["travellers"], dtype=float)
    assert rows[(last, "service_trips")] == w.sum()
    result = np.asarray(tr["result"])
    assert rows[(last, "failed")] == np.sum(w * (result == "no_taxi"))
    served = np.isin(result, ["as_planned", "waited_longer_than_expected"])
    assert rows[(last, "served")] == np.sum(w * served)
    counts = sum(rows.get((last, m), 0.0) for m in ("served", "failed", "truncated"))
    assert rows[(last, "service_trips")] == counts
    waited = np.asarray(tr["waited_s"], dtype=float)
    assert rows[(last, "wait_mean_s")] == pytest.approx(
        np.sum((w * waited)[served]) / w[served].sum()
    )
    met = np.asarray(tr["met_minus_expected_s"], dtype=float)
    ok = ~np.isnan(met)
    assert rows[(last, "mismatch_mean_s")] == pytest.approx(np.sum((w * met)[ok]) / w[ok].sum())
    assert rows[(last, "mismatch_abs_mean_s")] >= abs(rows[(last, "mismatch_mean_s")])
    expected = np.asarray(tr["expected_door_to_door_s"], dtype=float)
    assert rows[(last, "expected_door_to_door_s")] == pytest.approx(np.sum(w * expected) / w.sum())
    assert 0 <= rows[(last, "km_empty_share")] <= 1
    for gone in ("taxi_travellers", "share_no_taxi", "expected_share_no_taxi"):
        assert (last, gone) not in rows
    # The trip rows: as planned, with the service day's outcome.
    t = run.trip_results()
    taxi = np.asarray(t["mode"], dtype=object) == "taxi"
    assert set(np.asarray(t["service_outcome"])[taxi]) <= {"served", "failed", "truncated"}
    assert set(np.asarray(t["service_outcome"])[~taxi]) <= {""}
    assert (np.asarray(t["service_outcome"])[taxi] == "failed").sum() == failed_count(tr)
    # A failed request: the trip completed as planned, its met arrival NaN.
    gave_up = taxi & (np.asarray(t["service_outcome"]) == "failed")
    assert np.isnan(np.asarray(t["service_met_arrival_s"])[gave_up]).all()
    assert (np.asarray(t["outcome"])[gave_up] == "completed").all()
    m = run.manifest()
    assert m["service_expectations"]["taxi_wait_s"] == 600.0
    assert "last loading" in m["service_day"] and "as planned" in m["service_day"]


def failed_count(tr: object) -> int:
    return int((np.asarray(tr["result"]) == "no_taxi").sum())


def test_the_presets_order_the_taxi_share_and_service_met_round_trips(tmp_path: object) -> None:
    # Trips short enough to walk, a fare so that some but not all take a taxi.
    net = grid()
    rows = trips(net, 30, (0, 0), (3, 3))
    fare = {"taxi_eur_per_ride": 2.0, "taxi_eur_per_km": 0.5}
    s = taxi_scenario(net, rows, price_options=fare)
    share = {}
    for preset in ("optimistic", "default", "pessimistic"):
        r = s.run(
            preset, output_dir=str(tmp_path / preset), quiet=True, service_expectations=preset
        )
        share[preset] = float(np.sum(np.asarray(r.trip_results()["mode"], dtype=object) == "taxi"))
        assert r.service_expectations() == r.manifest()["service_expectations"]
    print(f"\ntaxi trips of 30 by preset: {share}")
    assert share["optimistic"] >= share["default"] >= share["pessimistic"]
    assert share["optimistic"] > share["pessimistic"]
    assert 0 < share["default"] < 30
    # The fingerprint holds the expectations; a misspelt name is refused with the names.
    base = s.run("d", output_dir=str(tmp_path / "d"), quiet=True)
    assert base.service_expectations() == {
        "taxi_wait_s": 600.0,
        "taxi_fail_share": 0.0,
        "dock_pickup_extra_s": None,
        "dock_return_extra_s": None,
        "dock_worst_case": 0.0,
    }
    met = base.service_met()
    assert set(met) == {"taxi_wait_s", "taxi_fail_share"}
    assert 0.0 <= met["taxi_fail_share"] <= 1.0
    again = s.run("met", output_dir=str(tmp_path / "met"), quiet=True, service_expectations=met)
    assert again.service_expectations()["taxi_wait_s"] == met["taxi_wait_s"]
    assert again.fingerprint != base.fingerprint
    with pytest.raises(ValueError, match="taxi_wait_s.*dock_worst_case"):
        s.run("bad", output_dir=str(tmp_path / "bad"), service_expectations={"taxi_wait": 60})
    with pytest.raises(ValueError, match="optimistic"):
        s.run("bad2", output_dir=str(tmp_path / "bad2"), service_expectations="cheerful")
    # Read back from its folder, the run gives the same.
    opened = ms.Run.open(tmp_path / "d")
    assert opened.service_expectations() == base.service_expectations()
    assert opened.service_met() == pytest.approx(met)


def test_without_a_fleet_no_taxi_file_nor_row(tmp_path: object) -> None:
    net = grid()
    rows = trips(net, 6, (0, 0), (9, 9))
    plain = scenario(
        net, rows, modes=["walk", "bike_share"], services=[ms.service_bike_docked(docks(net))]
    )
    r = plain.run("plain", output_dir=str(tmp_path), quiet=True)
    assert r.taxi_events() is None and r.taxi_trips() is None and r.taxi_vehicles() is None
    assert not list(tmp_path.glob("taxi_*.parquet"))
    k = pq.read_table(r._summary.kpis_path).to_pylist()
    assert not [x for x in k if x["mode"] == "taxi"]


def test_a_class_without_taxis_never_takes_one(tmp_path: object) -> None:
    net = grid()
    rows = trips(net, 6, (0, 0), (9, 9), "a") + trips(net, 6, (0, 0), (9, 9), "b")
    classes = {
        "a": {"owns_car": 0, "owns_bike": 0, "has_taxi": 0},
        "b": {"owns_car": 0, "owns_bike": 0},
    }
    r = taxi_scenario(net, rows, classes=classes).run("c", output_dir=str(tmp_path), quiet=True)
    tr = r.taxi_trips()
    assert tr.row_count > 0
    assert all(t.startswith("b") for t in tr["traveller_id"])


def test_same_seed_same_results(tmp_path: object) -> None:
    net = grid()
    rows = trips(net, 12, (0, 0), (9, 9))
    a = taxi_scenario(net, rows, **MSA).run("a", output_dir=str(tmp_path / "a"), quiet=True)
    b = taxi_scenario(net, rows, **MSA).run("b", output_dir=str(tmp_path / "b"), quiet=True)
    ta, tb = pq.read_table(a._summary.taxi_trips_path), pq.read_table(b._summary.taxi_trips_path)
    assert ta.drop(["run_id"]).equals(tb.drop(["run_id"]))
    assert a.fingerprint == b.fingerprint


def test_chart_service_draws_a_taxi_fleet(run: ms.Run) -> None:
    pytest.importorskip("matplotlib")
    import openmobisim.viz as viz

    for theme in ("paper", "night"):
        fig = viz.chart_service(run, "taxi", theme=theme)
        assert fig is not None
