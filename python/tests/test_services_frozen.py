"""A run's day frozen (``Run.freeze``) and replayed on the fleet engine (``frozen=``).

The network and docks of ``test_services_in_runs``: a 10 x 10 grid of 200 m blocks, docks of eight
slots; travellers own nothing and choose between walking and a shared bike (or the line and the
shared bike to its stop). Few bikes, so that pickups fail and travellers are redirected: a replay
that reproduces the run then reproduces the engine's harder paths too.
"""

from __future__ import annotations

import json
import time
from pathlib import Path

import numpy as np
import openmobisim as ms
import pytest
from test_services_in_runs import (
    CHAIN_MODES,
    chain_docks,
    docks,
    grid,
    line_feed,
    scenario,
    trips,
)


def same_columns(a: dict, b: dict) -> list[str]:
    """The columns of `a` that differ from `b`'s (floats bit for bit, NaN equal to NaN)."""
    differ = []
    for name, x in a.items():
        x, y = np.asarray(x), np.asarray(b[name])
        if x.dtype.kind == "f":
            equal = x.shape == y.shape and np.array_equal(
                x.view(np.uint64), y.astype(np.float64).view(np.uint64)
            )
        else:
            equal = list(x) == list(y)
        if not equal:
            differ.append(name)
    return differ


def assert_replays(run: ms.Run, network: object, services: list, folder: Path) -> ms.FleetDay:
    """Freeze `run` into `folder`, replay it, and check it is the run's last loading's day."""
    run.freeze(folder)
    mine = run._share_day()
    day = ms.fleet_day(network, services, frozen=folder)
    assert day.fidelity == "frozen_day" and day.frozen["run_fingerprint"] == run.fingerprint
    assert same_columns(mine["trips"], day.trips) == []
    for name, bins in mine["dock_bins"].items():
        assert same_columns(bins, day.dock_bins[name]) == [], name
    assert day.counts["invariant_violations"] == 0
    assert day.plan["legs"] == len(day.trips["status"]) == day.frozen["legs"]
    return day


def test_a_run_s_frozen_day_replays_the_run_s_last_loading_exactly(tmp_path: Path) -> None:
    net = grid()
    bikes = ms.service_bike_docked(docks(net, bikes=2))
    rows = trips(net, 30, (0, 0), (9, 9)) + trips(net, 20, (9, 9), (0, 1), "back")
    classes = {"c": (False, False, False), "back": (False, False, False)}
    s = scenario(
        net, rows, services=[bikes], classes=classes, choice_model="logit", equilibration="msa"
    )
    run = s.run("frozen", quiet=True)
    day = assert_replays(run, net, [bikes], tmp_path / "day")
    status = list(day.trips["status"])
    assert "served" in status and "failed" in status, "pickups fail: the replay is not trivial"
    assert max(day.trips["redirections"]) > 0, "travellers are redirected"
    # The folder: its three files, and what setup.json says.
    assert sorted(p.name for p in (tmp_path / "day").iterdir()) == [
        "legs.parquet",
        "road_times.bin",
        "setup.json",
    ]
    setup = json.loads((tmp_path / "day" / "setup.json").read_text())
    assert setup["seed"] == run.master_seed and setup["services"][0]["docks"] == 5
    assert setup["window"] == [run.window["start_s"], run.window["end_s"]]
    with pytest.raises(ValueError, match="overwrite"):
        run.freeze(tmp_path / "day")


def test_the_chain_s_frozen_day_replays_too(tmp_path: Path) -> None:
    net = grid()
    transit = line_feed(net, tmp_path / "line")
    bikes = ms.service_bike_docked(chain_docks(net))
    rows = trips(net, 30, (0, 0), (9, 9))
    s = scenario(net, rows, transit=transit, modes=CHAIN_MODES, services=[bikes])
    run = s.run("frozen-chain", quiet=True)
    assert "bike_share_transit" in set(run.trip_modes()["mode"])
    assert_replays(run, net, [bikes], tmp_path / "day")


def test_a_run_s_taxis_replay_too(tmp_path: Path) -> None:
    """The service day of a run with both services replays, taxis too (S303).

    Its frozen day holds the taxi legs after the shared bikes' and the road times its taxis drove
    on; a replay with no policy plays the same day, leg by leg, a failed request included.
    """
    from test_services_taxi_runs import FARE, fleet

    net = grid()
    bikes = ms.service_bike_docked(docks(net, bikes=2))
    taxis = fleet(net, n=2, max_wait_s=300)
    rows = trips(net, 30, (0, 0), (9, 9)) + trips(net, 10, (9, 9), (0, 1), "back")
    classes = {"c": (False, False, False), "back": (False, False, False)}
    s = scenario(
        net,
        rows,
        modes=["walk", "bike_share", "taxi"],
        services=[bikes, taxis],
        classes=classes,
        choice_model="logit",
        price_options=FARE,
        equilibration="msa",
    )
    run = s.run("frozen-taxi", quiet=True)
    day = assert_replays(run, net, [bikes, taxis], tmp_path / "day")
    services = list(day.trips["service"])
    assert "taxi" in services and "bike_share" in services, "both services in the day"
    taxi = np.asarray(services) == "taxi"
    status = set(np.asarray(day.trips["status"])[taxi])
    assert "served" in status and "failed" in status, "the replay is not trivial: {status}"
    assert set(np.asarray(day.trips["status"])[~taxi]) >= {"served"}


def test_a_frozen_day_is_refused_with_other_services_another_network_or_window(
    tmp_path: Path,
) -> None:
    net = grid()
    bikes = ms.service_bike_docked(docks(net, bikes=2))
    rows = trips(net, 10, (0, 0), (9, 9))
    run = scenario(net, rows, services=[bikes], choice_model="deterministic").run(
        "frozen-refused", quiet=True
    )
    folder = run.freeze(tmp_path / "day")
    more = ms.service_bike_docked(docks(net, bikes=3))
    with pytest.raises(ValueError, match="other services"):
        ms.fleet_day(net, [more], frozen=folder)
    with pytest.raises(ValueError, match="services.*'bike_share'.*'other'"):
        ms.fleet_day(net, [ms.service_bike_docked(docks(net, 2), name="other")], frozen=folder)
    with pytest.raises(ValueError, match="walk_max_s"):
        ms.fleet_day(
            net,
            [ms.service_bike_docked(docks(net, 2), options={"walk_max_s": 300})],
            frozen=folder,
        )
    with pytest.raises(ValueError, match="network"):
        ms.fleet_day(ms.examples.manhattan_grid(9, 200.0, False), [bikes], frozen=folder)
    with pytest.raises(ValueError, match="window"):
        ms.fleet_day(net, [bikes], frozen=folder, window=("07:00", "10:00"))
    with pytest.raises(ValueError, match="without trips"):
        ms.fleet_environment(net, [bikes], rows, frozen=folder)
    with pytest.raises(FileNotFoundError, match="setup.json"):
        ms.fleet_day(net, [bikes], frozen=tmp_path)
    # A run without a service has no day to freeze, and writes nothing.
    plain = scenario(net, rows, modes=["walk", "bike"]).run("frozen-none", quiet=True)
    with pytest.raises(ValueError, match="no service"):
        plain.freeze(tmp_path / "none")
    assert not (tmp_path / "none").exists()


def rewards(env: ms.FleetEnvironment, seed: int) -> list[float]:
    env.reset(seed)
    out, done = [], False
    while not done:
        _, reward, done, _, info = env.step([])
        out.append(reward)
    env.last_info = info
    return out


def test_an_environment_on_a_frozen_day(tmp_path: Path) -> None:
    net = grid()
    bikes = ms.service_bike_docked(docks(net, bikes=2))
    rows = trips(net, 40, (0, 0), (9, 9))
    s = scenario(net, rows, services=[bikes], choice_model="logit", equilibration="msa")
    run = s.run("frozen-env", quiet=True)
    folder = run.freeze(tmp_path / "day")
    env = ms.fleet_environment(net, [bikes], frozen=folder, step_s=600)
    assert env.frozen["run_fingerprint"] == run.fingerprint
    assert env.window == tuple(env.frozen["window"])
    first = rewards(env, 3)
    assert rewards(env, 3) == first and sum(first) < 0
    # With neither variation every seed replays the run's day, to its outcome.
    assert rewards(env, 4) == first
    assert same_columns(run._share_day()["trips"], env.last_info["outcome"]["trips"]) == []
    # Thinned and shifted legs still vary days by seed, and replay by seed.
    for kw in ({"legs_keep": 0.5}, {"departure_jitter_s": 600.0}):
        varied = ms.fleet_environment(net, [bikes], frozen=folder, step_s=600, **kw)
        a, b = rewards(varied, 1), rewards(varied, 2)
        assert a != b, kw
        assert rewards(varied, 1) == a, kw


def test_freezing_and_reading_back_the_grid_run_s_day(tmp_path: Path) -> None:
    """The 600 trips of the grid's run-time measurement: seconds and bytes (``pytest -s``)."""
    net = grid()
    bikes = ms.service_bike_docked(docks(net, bikes=8))
    rows = [
        r
        for k, (frm, to) in enumerate(
            [((0, 0), (9, 9)), ((0, 1), (5, 5)), ((9, 8), (0, 0)), ((5, 5), (9, 9))]
        )
        for r in trips(net, 150, frm, to, f"g{k}")
    ]
    classes = {f"g{k}": (False, True, False) for k in range(4)}
    s = scenario(
        net,
        rows,
        modes=["walk", "bike", "bike_share"],
        services=[bikes],
        classes=classes,
        choice_model="logit",
        equilibration="msa",
    )
    run = s.run("frozen-time", quiet=True)
    started = time.perf_counter()
    folder = run.freeze(tmp_path / "day")
    t_freeze = time.perf_counter() - started
    started = time.perf_counter()
    day = ms.fleet_day(net, [bikes], frozen=folder)
    t_read = time.perf_counter() - started
    size = sum(p.stat().st_size for p in folder.iterdir())
    print(
        f"\ngrid 10 x 10, 600 trips, {day.frozen['legs']} legs: freeze {t_freeze:.3f} s, "
        f"read back and replay {t_read:.3f} s (day {day.seconds['day_s']:.3f} s), "
        f"folder {size} bytes"
    )
    assert day.frozen["legs"] > 0
