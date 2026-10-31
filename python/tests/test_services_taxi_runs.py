"""A taxi fleet in a run (S302, S303): its choice, service day, files, rows and refusal.

Offered to trips choosing their mode on fixed assumptions, its requests played on the fleet
engine once, after the run has stopped; its three results files, its KPI rows; the refusal of a
taxi without a constant or a fare; ``has_taxi``; the assumptions by name, the choice's time of a
taxi with a chance of no taxi, and ``Run.service_actual()``.

The network: the 10 x 10 grid of ``test_services_in_runs``; four vehicles at its first corner,
one charger at its centre. Travellers own no car and no bike and choose between walking and a
taxi.
"""

from __future__ import annotations

from collections import Counter
from pathlib import Path

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
    # No dispatch delay: each vehicle comes back from the far corner within the 300 s cancel
    # limit for a second trip and so drops below the charging threshold (with the 120 s delay,
    # its reach of 180 s is too short for the drive back, and nobody charges).
    taxis = fleet(net, soc=0.22, request_cancel_s=300, dispatch_delay_s=0)
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
    outcomes: Counter[str] = Counter()
    for r, x in zip(tr["outcome"], w, strict=True):
        outcomes[r] += x
    events = Counter(ev["event"])
    assert events["asked_for_taxi"] == w.sum()
    assert events["dropped_off"] == outcomes["completed"] == np.sum(vb["passengers_dropped_off"])
    # A request no taxi could pick up in time is cancelled and ends there: no walk, no drop-off.
    assert events["request_cancelled"] == outcomes["cancelled"] > 0
    assert "reached_destination" not in events and "gave_up_walking_to_destination" not in events
    cancelled = np.asarray(tr["outcome"]) == "cancelled"
    for column in ("pickup_s", "dropoff_s", "waiting_s", "in_vehicle_s", "ride_km"):
        assert np.isnan(np.asarray(tr[column], dtype=float)[cancelled]).all(), column
    # A ride: its waiting and in-vehicle times are its pickup's and drop-off's differences.
    ride = np.asarray(tr["outcome"]) == "completed"
    pickup, dropoff = (np.asarray(tr[c], dtype=float)[ride] for c in ("pickup_s", "dropoff_s"))
    requested = np.asarray(tr["requested_s"], dtype=float)[ride]
    assert np.asarray(tr["waiting_s"], dtype=float)[ride] == pytest.approx(pickup - requested)
    assert np.asarray(tr["in_vehicle_s"], dtype=float)[ride] == pytest.approx(dropoff - pickup)
    assert set(tr["assumed_wait_s"]) == {600.0}
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


TAXI_ROWS = [
    "requests",
    "rides",
    "cancelled",
    "unfinished",
    "waiting_mean_s",
    "waiting_median_s",
    "waiting_p90_s",
    "assumed_wait_s",
    "in_vehicle_mean_s",
    "ride_km_mean",
    "empty_km_per_ride",
    "utilisation",
    "rides_per_taxi",
    "kwh_charged",
]


def _weighted_quantile(values: np.ndarray, weights: np.ndarray, q: float) -> float:
    """The smallest value with at least the share ``q`` of the people at or below it."""
    order = np.argsort(values, kind="stable")
    v, w = values[order], weights[order]
    seen = np.cumsum(w)
    return float(v[np.searchsorted(seen, q * w.sum() - 1e-9 * w.sum())])


def test_the_kpi_rows_are_the_trips(run: ms.Run) -> None:
    k = pq.read_table(run._summary.kpis_path).to_pylist()
    # The service's rows (under its name, here the mode's too, beside the mode's trip rows):
    # one row set of the service day, for the last iteration, in this order.
    taxi = [r for r in k if r["mode"] == "taxi" and r["metric"] in TAXI_ROWS]
    last = max(r["iteration"] for r in k)
    assert {r["iteration"] for r in taxi} == {last}
    assert [r["metric"] for r in taxi] == TAXI_ROWS
    gone = {"service_trips", "served", "failed", "truncated", "wait_mean_s", "km_empty_share"}
    assert not gone & {r["metric"] for r in k if r["mode"] == "taxi"}
    rows = {r["metric"]: r["value"] for r in taxi}
    tr = run.taxi_trips()
    w = np.asarray(tr["travellers"], dtype=float)
    outcome = np.asarray(tr["outcome"])
    assert rows["requests"] == w.sum()
    assert rows["rides"] == np.sum(w * (outcome == "completed"))
    assert rows["cancelled"] == np.sum(w * (outcome == "cancelled"))
    assert rows["requests"] == rows["rides"] + rows["cancelled"] + rows["unfinished"]
    ride = outcome == "completed"
    waiting = np.asarray(tr["waiting_s"], dtype=float)[ride]
    assert rows["waiting_mean_s"] == pytest.approx(np.sum(w[ride] * waiting) / w[ride].sum())
    assert rows["waiting_median_s"] == _weighted_quantile(waiting, w[ride], 0.5)
    assert rows["waiting_p90_s"] == _weighted_quantile(waiting, w[ride], 0.9)
    assert rows["assumed_wait_s"] == 600.0
    in_vehicle = np.asarray(tr["in_vehicle_s"], dtype=float)[ride]
    assert rows["in_vehicle_mean_s"] == pytest.approx(np.sum(w[ride] * in_vehicle) / w[ride].sum())
    km = np.asarray(tr["ride_km"], dtype=float)[ride]
    assert rows["ride_km_mean"] == pytest.approx(np.sum(w[ride] * km) / w[ride].sum())
    vb = run.taxi_vehicles()
    assert rows["empty_km_per_ride"] == pytest.approx(np.sum(vb["km_empty"]) / rows["rides"])
    seconds = sum(np.sum(vb[c]) for c in vb if c.startswith("seconds_"))
    assert rows["utilisation"] == pytest.approx(np.sum(vb["seconds_occupied"]) / seconds)
    assert rows["rides_per_taxi"] == rows["rides"] / 4
    # The trip rows: the planned trip, with the service day's actual outcome.
    t = run.trip_results()
    by_taxi = np.asarray(t["mode"], dtype=object) == "taxi"
    service = np.asarray(t["service_outcome"])
    assert set(service[by_taxi]) <= {"completed", "cancelled", "unfinished"}
    assert set(service[~by_taxi]) <= {""}
    assert (service[by_taxi] == "cancelled").sum() == int(cancelled_count(tr))
    # A cancelled request: the trip completed as planned, its actual arrival NaN.
    gave_up = by_taxi & (service == "cancelled")
    assert np.isnan(np.asarray(t["service_arrival_s"])[gave_up]).all()
    assert (np.asarray(t["outcome"])[gave_up] == "completed").all()
    done = by_taxi & (service == "completed")
    assert np.isfinite(np.asarray(t["service_arrival_s"])[done]).all()
    m = run.manifest()
    assert m["service_assumptions"]["assumed_wait_s"] == 600.0
    assert "last loading" in m["service_day"] and "fixed assumptions" in m["service_day"]


# The rows of a mode that only a planned trip has: under a mode a service serves, the service's
# own rows say how its trips went instead (S305).
PLANNED_ROWS = {
    "completed_trips",
    "completed_people",
    "completion_rate",
    "truncated_trips",
    "total_travel_time_s",
}


def test_the_taxi_mode_keeps_its_choice_rows_and_none_the_service_day_contradicts(
    run: ms.Run,
) -> None:
    # A cheap fare and a short cancel limit: many choose a taxi and many requests are cancelled,
    # so a planned-trip row under `taxi` would count as completed what the fleet cancelled.
    k = pq.read_table(run._summary.kpis_path).to_pylist()
    last = max(r["iteration"] for r in k)
    taxi = {r["metric"]: r["value"] for r in k if r["mode"] == "taxi" and r["iteration"] == last}
    assert taxi["cancelled"] > 0.25 * taxi["requests"], "the fixture cancels many requests"
    assert not PLANNED_ROWS & set(taxi), sorted(PLANNED_ROWS & set(taxi))
    for kept in ("trips", "people", "mode_share", "mode_not_available_trips"):
        assert kept in taxi, kept
    assert taxi["people"] == taxi["requests"], "every person who chose a taxi requested one"
    # The whole run keeps them (its trips counted as planned, the taxi's included).
    whole = {r["metric"] for r in k if r["mode"] == "all" and r["iteration"] == last}
    assert not PLANNED_ROWS - whole
    # Read back, the taxi's entry holds what the file holds, and the mean trip is the run's.
    back = ms.Run.open(Path(run._summary.kpis_path).parent)
    entry = back.completion_by_mode["taxi"]
    assert set(entry) == {
        "total_trips",
        "no_vehicle_available",
        "no_feasible_path",
        "mode_not_available",
        "people",
    }
    assert back.mean_travel_time_s == pytest.approx(run.mean_travel_time_s, rel=1e-12)


def cancelled_count(tr: object) -> int:
    return int((np.asarray(tr["outcome"]) == "cancelled").sum())


def test_the_assumptions_move_the_taxi_share_and_service_actual_round_trips(
    tmp_path: object,
) -> None:
    # Trips short enough to walk, a fare so that some but not all take a taxi.
    net = grid()
    rows = trips(net, 30, (0, 0), (3, 3))
    fare = {"taxi_eur_per_ride": 2.0, "taxi_eur_per_km": 0.5}
    s = taxi_scenario(net, rows, price_options=fare)
    share = {}
    for name, given in (
        ("300", {"assumed_wait_s": 300}),
        ("600", None),
        ("900", {"assumed_wait_s": 900}),
    ):
        r = s.run(name, output_dir=str(tmp_path / name), quiet=True, service_assumptions=given)
        share[name] = float(np.sum(np.asarray(r.trip_results()["mode"], dtype=object) == "taxi"))
        assert r.service_assumptions() == r.manifest()["service_assumptions"]
    print(f"\ntaxi trips of 30 by assumed wait: {share}")
    # A shorter assumed wait draws more; one above the cancel limit (600 s) fewer: no cap.
    assert share["300"] > share["600"] > share["900"]
    assert 0 < share["600"] < 30
    # The fingerprint holds the assumptions; a misspelt name is refused with the names.
    base = s.run("d", output_dir=str(tmp_path / "d"), quiet=True)
    assert base.service_assumptions() == {
        "assumed_wait_s": 600.0,
        "assumed_no_taxi_share": 0.0,
        "assumed_extra_walk_at_pickup_s": None,
        "assumed_extra_walk_at_return_s": None,
    }
    actual = base.service_actual()
    assert set(actual) == {"assumed_wait_s", "assumed_no_taxi_share"}
    assert 0.0 <= actual["assumed_no_taxi_share"] <= 1.0
    again = s.run("a", output_dir=str(tmp_path / "a"), quiet=True, service_assumptions=actual)
    assert again.service_assumptions()["assumed_wait_s"] == actual["assumed_wait_s"]
    assert again.fingerprint != base.fingerprint
    with pytest.raises(ValueError, match="assumed_wait_s.*assumed_no_taxi_share"):
        s.run("bad", output_dir=str(tmp_path / "bad"), service_assumptions={"taxi_wait": 60})
    with pytest.raises(ValueError, match="a dict of values by name"):
        s.run("bad2", output_dir=str(tmp_path / "bad2"), service_assumptions="optimistic")
    # Read back from its folder, the run gives the same.
    opened = ms.Run.open(tmp_path / "d")
    assert opened.service_assumptions() == base.service_assumptions()
    assert opened.service_actual() == pytest.approx(actual)


class _Recorder:
    """The fastest alternative by ``time_min``, keeping every situation's taxi and best other."""

    name = "recorder"
    descriptor = "recorder;v1"
    attributes = ["time_min", "wait_min", "car_min", "parking_min", "fail_prob", "mode_taxi"]

    def __init__(self) -> None:
        self.seen: list[tuple[float, float, float, float, float, float]] = []

    def choose(self, batch: object) -> object:
        a = {k: np.asarray(v, dtype=float) for k, v in batch.attributes.items()}
        offsets = np.asarray(batch.offsets)
        chosen = np.zeros(len(offsets) - 1, dtype=np.int64)
        for i in range(len(offsets) - 1):
            lo, hi = int(offsets[i]), int(offsets[i + 1])
            chosen[i] = int(np.argmin(a["time_min"][lo:hi]))
            taxi = np.flatnonzero(a["mode_taxi"][lo:hi] == 1)
            if len(taxi) == 1 and hi - lo > 1:
                t = lo + int(taxi[0])
                best = min(float(a["time_min"][j]) for j in range(lo, hi) if j != t)
                self.seen.append(
                    (
                        float(a["time_min"][t]),
                        float(a["wait_min"][t]),
                        float(a["car_min"][t]),
                        float(a["parking_min"][t]),
                        float(a["fail_prob"][t]),
                        best,
                    )
                )
        return chosen


def test_a_chance_of_no_taxi_adds_the_expected_waiting_of_cancelled_requests(
    tmp_path: object,
) -> None:
    # A cancelled request followed by the best other alternative, under the random utility
    # model: the ride with the cancel limit x p / (1 - p) added to its waiting.
    net = grid()
    rows = trips(net, 6, (0, 0), (3, 3))
    model = _Recorder()
    s = taxi_scenario(net, rows, choice_model=model)
    s.run(
        "p",
        output_dir=str(tmp_path),
        quiet=True,
        service_assumptions={"assumed_no_taxi_share": 0.25},
    )
    assert model.seen, "the taxi was offered"
    penalty = 600 / 60 * 0.25 / 0.75
    for time, wait, car, boarding, p, _ in model.seen:
        assert p == 0.25 and boarding == 1.0
        assert wait == pytest.approx(10.0 + penalty)
        assert time == pytest.approx(10.0 + boarding + car + penalty)
    # With no chance of no taxi the time is the planned one.
    model0 = _Recorder()
    taxi_scenario(net, rows, choice_model=model0).run(
        "p0", output_dir=str(tmp_path / "0"), quiet=True
    )
    for time, wait, car, boarding, p, _ in model0.seen:
        assert p == 0.0 and time == pytest.approx(wait + boarding + car)


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
