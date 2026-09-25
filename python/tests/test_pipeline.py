"""The Phase 1 pipeline, from Python: grid through `Scenario` through `Run`.

These are the Python-side counterpart to `crates/core-sim/tests/run.rs` and
`crates/io-parquet/tests/writers.rs` — this file is about the boundary and
the ergonomic wrapper (`Scenario`/`Run`/`Table`), not re-proving the Rust
logic those already cover.
"""

from __future__ import annotations

import openmobisim as ms
import pytest


def small_grid() -> object:
    return ms.examples.manhattan_grid(n=3, block_metres=100.0, signals=False)


def car_owning_commuters() -> dict[str, tuple[bool, bool, bool]]:
    return {"commuter": (True, False, False)}


def test_manhattan_grid_needs_at_least_two_by_two() -> None:
    with pytest.raises(ValueError, match="2x2"):
        ms.examples.manhattan_grid(n=1, block_metres=100.0, signals=False)


def test_a_single_car_trip_completes() -> None:
    g = small_grid()
    trips = ms.examples.fixed_car_trips(g, [("alice", (0, 0), (2, 2), 8.0, None)])
    sc = ms.Scenario.from_parts(
        network=g, demand=trips, class_defaults=car_owning_commuters(), window_hours=24.0
    )
    run = sc.run(run_id="pytest-single-trip")

    assert run.completion == {
        "total_trips": 1,
        "completed": 1,
        "truncated": 0,
        "no_vehicle_available": 0,
        "no_feasible_path": 0,
        "mode_not_available": 0,
    }
    assert run.total_travel_time_s > 0.0


def test_a_traveller_without_a_car_is_not_simulated() -> None:
    g = small_grid()
    trips = ms.examples.fixed_car_trips(
        g, [("bob", (0, 0), (2, 2), 8.0, None)], user_class="pedestrian"
    )
    # No class default declares ownership for "pedestrian".
    sc = ms.Scenario.from_parts(network=g, demand=trips, class_defaults=car_owning_commuters())
    run = sc.run(run_id="pytest-no-car")

    assert run.completion["no_vehicle_available"] == 1
    assert run.completion["completed"] == 0
    assert run.total_travel_time_s == 0.0


def test_a_short_window_truncates_rather_than_completing() -> None:
    g = small_grid()
    trips = ms.examples.fixed_car_trips(g, [("carol", (0, 0), (2, 2), 8.0, None)])
    sc = ms.Scenario.from_parts(
        network=g, demand=trips, class_defaults=car_owning_commuters(), window_hours=8.0001
    )
    run = sc.run(run_id="pytest-truncated")

    assert run.completion["truncated"] == 1
    assert run.completion["completed"] == 0


def test_kpis_table_round_trips_through_pandas() -> None:
    pytest.importorskip("pandas")
    g = small_grid()
    trips = ms.examples.fixed_car_trips(g, [("dana", (0, 0), (2, 2), 8.0, None)])
    # Pinned to the pre-car-ready-checkpoint defaults (S187 flipped the library's bare default
    # to "logit"/"msa"): this test is about the plain, single-loading kpis shape.
    sc = ms.Scenario.from_parts(
        network=g,
        demand=trips,
        class_defaults=car_owning_commuters(),
        choice_model="deterministic",
        equilibration="none",
    )
    run = sc.run(run_id="pytest-kpis-pandas")

    df = run.kpis().to_pandas()
    expected = {
        "trips",
        "total_travel_time_s",
        "completed_trips",
        "truncated_trips",
        "no_vehicle_available_trips",
        "no_feasible_path_trips",
        "mode_not_available_trips",
        "completion_rate",
    }
    # The run's rows, and the same for its one mode (S195).
    assert set(df[df["mode"] == "all"]["metric"]) == expected
    assert set(df[df["mode"] == "car"]["metric"]) == expected
    assert set(df["mode"]) == {"all", "car"}
    assert (df["run_id"] == "pytest-kpis-pandas").all()


def test_manifest_reports_the_kpi_weighting() -> None:
    g = small_grid()
    trips = ms.examples.fixed_car_trips(g, [("erin", (0, 0), (2, 2), 8.0, None)])
    sc = ms.Scenario.from_parts(network=g, demand=trips, class_defaults=car_owning_commuters())
    run = sc.run(run_id="pytest-manifest")

    manifest = run.manifest()
    assert manifest["kpi_weighting"] == "traveller_weight_scaled"
    assert manifest["simulated_travellers"] == 1
    assert manifest["car_owning_travellers"] == 1


def test_a_run_is_deterministic() -> None:
    def run_once() -> dict[str, int]:
        g = ms.examples.manhattan_grid(n=3, block_metres=100.0, signals=False)
        trips = ms.examples.fixed_car_trips(
            g,
            [("alice", (0, 0), (2, 2), 8.0, None), ("bob", (0, 2), (2, 0), 8.5, 2)],
        )
        sc = ms.Scenario.from_parts(network=g, demand=trips, class_defaults=car_owning_commuters())
        run = sc.run(run_id="pytest-determinism")
        return {"completion": run.completion, "total_travel_time_s": run.total_travel_time_s}

    first = run_once()
    for _ in range(3):
        assert run_once() == first
