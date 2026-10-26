"""Every trip and what became of it (`Run.trip_results()`, `trip_results.parquet`).

On a grid loaded beyond its capacity, with a window too short for everyone: the table has one
row per trip, its outcomes agree with the run's counts, the completed trips' times give the
run's mean trip, every car still on the roads at the end names its link, and the run read back
from its folder gives the same table. Walkers beyond the walk limit, with nothing to choose, are
the `no_alternative` trips. With the minimal outputs the table is in memory but not written.
"""

from __future__ import annotations

import collections
from pathlib import Path

import numpy as np
import openmobisim as ms
import pytest

#: Every run pays the fixed times of reaching and leaving a vehicle; these tests read the drives
#: and rides alone, so they set them to 0.
DRIVES_ONLY = {"car_access_s": 0.0, "car_egress_s": 0.0, "bike_access_s": 0.0, "bike_egress_s": 0.0}

CAR = {"commuter": (True, False, False)}


def jammed(run_id: str, output_dir: str, outputs: str = "all") -> ms.Run:
    net = ms.examples.manhattan_grid(n=6, block_metres=150.0, signals=False)
    rows = ms.examples.trips_random(net, 3000, seed=3, min_m=300.0, max_m=900.0, spread_s=600)
    settings = {
        "flow_level": 4,
        "equilibration": "free_flow",
        "equilibration_options": {"increments": 1, "warmup": 0},
        "window": (0, 600),  # departures in the first 600 s: the last cannot finish
        "mode_options": DRIVES_ONLY,
    }
    return ms.Scenario.from_parts(net, rows, classes=CAR, **settings).run(
        run_id, output_dir=output_dir, overwrite=True, quiet=True, outputs=outputs
    )


@pytest.fixture(scope="module")
def jammed_run(tmp_path_factory) -> tuple[ms.Run, Path]:
    folder = tmp_path_factory.mktemp("trips") / "jammed"
    return jammed("trips-jammed", output_dir=str(folder)), folder


def test_every_trip_has_a_row_and_the_outcomes_add_up_to_the_run_s_counts(jammed_run) -> None:
    run, _ = jammed_run
    t = run.trip_results()
    n = run.completion["total_trips"]
    assert list(t) == [
        "trip", "traveller_id", "trip_seq", "user_class", "weight", "origin_lon", "origin_lat",
        "destination_lon", "destination_lat", "departure_s", "mode", "mode_choice", "outcome",
        "arrival_s", "travel_time_s", "link_at_end", "vehicle_s", "access_egress_s",
    ]  # fmt: skip
    assert all(len(v) == n for v in t.values())
    assert list(t["trip"]) == list(range(n))
    counts = collections.Counter(t["outcome"])
    assert counts["truncated"] == run.completion["truncated"] > 0, "the window is too short"
    assert counts["completed"] == run.completion["completed"] > 0
    assert sum(counts.values()) == n

    done = np.array([o == "completed" for o in t["outcome"]])
    arrival, departure = t["arrival_s"], t["departure_s"].astype(float)
    assert np.all(np.isnan(arrival[~done])) and not np.any(np.isnan(arrival[done]))
    assert np.array_equal(t["travel_time_s"][done], arrival[done] - departure[done])
    # A stated car trip in a run without mode choice or a timetable pays no access or egress:
    # its time is its vehicle's; neither is known for a trip that did not finish.
    vehicle, fixed = t["vehicle_s"], t["access_egress_s"]
    assert np.array_equal(fixed[done], np.zeros(done.sum()))
    assert np.array_equal(vehicle[done] + fixed[done], t["travel_time_s"][done])
    assert np.all(np.isnan(vehicle[~done])) and np.all(np.isnan(fixed[~done]))
    assert np.all(arrival[done] <= 600)
    weight = t["weight"].astype(float)
    mean = np.sum(t["travel_time_s"][done] * weight[done]) / weight[done].sum()
    assert mean == pytest.approx(run.mean_travel_time_s)

    # Every car still travelling names a link of the network; nobody else does.
    links = len(run.network.link_ids())
    at_end = t["link_at_end"]
    assert np.all(np.isnan(at_end[done]))
    unfinished = at_end[~done]
    assert not np.any(np.isnan(unfinished)) and np.all((unfinished >= 0) & (unfinished < links))
    g = run.report_gridlock()
    assert g["vehicles_on_network"] + g["vehicles_outside"] == len(unfinished)


def test_the_table_read_back_from_the_run_s_folder_is_the_same(jammed_run) -> None:
    pytest.importorskip("pyarrow")  # Run.open reads the Parquet files through it
    run, folder = jammed_run
    t = run.trip_results()
    opened = ms.Run.open(folder)
    back = opened.trip_results()
    for name, column in t.items():
        if isinstance(column, list):
            assert back[name] == column, name
        else:
            assert np.array_equal(np.asarray(back[name]), column, equal_nan=True), name
    modes = run.trip_modes()
    for name in modes:
        assert list(modes[name]) == list(t[name])
        assert list(opened.trip_modes()[name]) == list(t[name])


def test_a_car_still_travelling_at_free_flow_names_the_link_it_is_on() -> None:
    g = ms.examples.manhattan_grid(n=3, block_metres=100.0, signals=False)
    trips = ms.examples.fixed_car_trips(g, [("carol", (0, 0), (2, 2), 8.0, None)])
    for level in (0, 2):
        run = ms.Scenario.from_parts(
            g, trips, classes=CAR, window=(0, 28_801), flow_level=level, mode_options=DRIVES_ONLY
        ).run(f"trips-one-{level}", overwrite=True, quiet=True)
        t = run.trip_results()
        assert t["outcome"] == ["truncated"]
        # One second after leaving, the car is on a link out of its origin.
        link = g.link_ids()[int(t["link_at_end"][0])]
        assert link.startswith("l_n_000_000__"), (level, link)


def test_walkers_with_nothing_to_choose_are_the_no_alternative_trips() -> None:
    walker = {"modes": ["walk"]}
    net = ms.examples.manhattan_grid(n=6, block_metres=200.0, signals=False)
    rows = ms.examples.trips_random(net, 200, seed=5, min_m=300.0, max_m=1500.0, spread_s=600)
    rows = ms.demand_assign_classes(rows, {"walker": {**walker, "share": 1}, "driver": {
        "modes": ["car"], "share": 1}}, seed=1)  # fmt: skip
    with pytest.warns(ms.NoAlternativeWarning):
        run = ms.Scenario.from_parts(
            net, rows, classes={"walker": walker, "driver": {"modes": ["car"]}},
            mode_options={"walk_max_s": 600.0, "fallback_car": 0}, equilibration="free_flow",
        ).run("trips-none", overwrite=True, quiet=True)  # fmt: skip
    t = run.trip_results()
    none = [i for i, m in enumerate(t["mode"]) if m is None]
    assert none and all(t["outcome"][i] == "no_alternative" for i in none)
    assert collections.Counter(t["outcome"])["no_alternative"] == len(none)
    assert {t["user_class"][i] for i in none} == {"walker"}
    assert len(none) == run.completion_by_mode["none"]["total_trips"]
    # The same trips the run reports with no alternative (none of them went by car here).
    assert sorted(i for i, o in enumerate(t["outcome"]) if o == "no_alternative") == none
    assert len(none) == len(run.report_no_alternative()["traveller_id"])


def test_the_minimal_outputs_keep_the_table_in_memory_only(tmp_path) -> None:
    run = jammed("trips-minimal", output_dir=str(tmp_path / "minimal"), outputs="minimal")
    assert len(run.trip_results()["trip"]) == run.completion["total_trips"]
    assert not (tmp_path / "minimal" / "trip_results.parquet").exists()
    with pytest.raises(RuntimeError, match="trip_results.parquet"):
        ms.Run.open(tmp_path / "minimal").trip_results()
