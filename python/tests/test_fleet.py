"""A fleet day without a run: docked bikes and taxis on a 5 x 5 grid of 200 m blocks."""

from __future__ import annotations

import math

import openmobisim as ms
import pytest


def grid() -> object:
    return ms.examples.manhattan_grid(5, 200.0, False)


def at(net: object, row: int, col: int) -> tuple[float, float]:
    return ms._core.grid_node_lonlat(net, row, col)


def trip(net, ident, t, a, b, service=None, weight=None) -> dict:
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
    if weight is not None:
        row["weight"] = weight
    return row


def docks(net, bikes=(1, 0)) -> object:
    places = {"a": (0, 0), "z": (4, 4)}
    return ms.dock_read_table(
        [
            {"dock_id": k, "lon": at(net, *p)[0], "lat": at(net, *p)[1], "capacity": 4, "bikes": b}
            for (k, p), b in zip(places.items(), bikes, strict=True)
        ]
    )


def test_two_travellers_ask_for_the_one_bike_and_the_second_fails():
    net = grid()
    bikes = ms.service_bike_docked(docks(net))
    day = ms.fleet_day(
        net,
        [bikes],
        [trip(net, "t1", "08:00", (0, 0), (4, 4)), trip(net, "t2", "08:00:10", (0, 0), (4, 4))],
    )
    t = day.trip_results()
    assert day.fidelity == "frozen_day"
    assert t["status"] == ["served", "failed"]
    assert t["dock_from"] == ["a", None] and t["dock_to"] == ["z", None]
    assert t["requested_s"][0] == pytest.approx(8 * 3600), "the dock is at the origin's node"
    assert t["arrived_s"][0] > t["requested_s"][0]
    assert day.counts["invariant_violations"] == 0
    assert sum(day.dock_bins["bike_share"]["pickups"]) == 1
    assert day.seconds["day_s"] >= 0 and day.plan == {
        "trips": 2,
        "legs": 2,
        "no_dock": 0,
        "same_dock": 0,
        "no_ride": 0,
    }


def test_a_trip_without_a_dock_in_reach_is_not_planned_and_said_so():
    net = grid()
    bikes = ms.service_bike_docked(docks(net), options={"walk_max_s": 1.0})
    day = ms.fleet_day(net, [bikes], [trip(net, "far", "08:00", (2, 2), (4, 4))])
    assert day.trip_results()["status"] == ["not_planned"]
    assert day.plan["no_dock"] == 1


def test_a_taxi_drives_its_traveller_and_waits_are_counted():
    net = grid()
    v = ms.vehicle_read_table(
        [{"vehicle_id": "v1", "lon": at(net, 0, 0)[0], "lat": at(net, 0, 0)[1]}]
    )
    taxis = ms.service_taxi(v)
    day = ms.fleet_day(net, [taxis], [trip(net, "t1", "08:00", (0, 1), (4, 4))])
    t = day.trip_results()
    assert t["status"] == ["served"] and t["vehicle_id"] == ["v1"]
    assert t["waited_s"][0] > 0, "the vehicle drives one block to the pickup"
    assert math.isfinite(t["arrived_s"][0])


def test_inputs_that_are_not_valid_are_refused_with_why():
    net = grid()
    bikes = ms.service_bike_docked(docks(net))
    with pytest.raises(ValueError, match="service names repeat"):
        ms.fleet_day(net, [bikes, bikes], [])
    with pytest.raises(ValueError, match="unknown bike_docked option"):
        ms.fleet_day(net, [ms.service_bike_docked(docks(net), options={"walk": 1})], [])
    with pytest.raises(ValueError, match="departure is required"):
        row = trip(net, "t", "08:00", (0, 0), (4, 4))
        del row["departure"]
        ms.fleet_day(net, [bikes], [row])
    with pytest.raises(ValueError, match="is not one of"):
        ms.fleet_day(net, [bikes], [trip(net, "t", "08:00", (0, 0), (4, 4), service="nope")])


def test_an_environment_steps_through_the_day_and_a_fork_leaves_it_alone():
    from openmobisim.operator import Move

    net = grid()
    bikes = ms.service_bike_docked(docks(net, bikes=(3, 0)))
    trips = [trip(net, f"t{k}", 27_000 + 600 * k, (0, 0), (4, 4)) for k in range(6)]
    env = ms.fleet_environment(net, [bikes], trips, step_s=900)
    view, info = env.reset(seed=1)
    assert info["fidelity"] == "frozen_day" and env.t == 27_000
    # The first trip asks at 07:30 itself: a second's requests come before its step.
    assert list(view["bike_share"]["bikes"]) == [2, 0]
    total, steps, done = 0.0, 0, False
    ahead = None
    while not done:
        if steps == 2:
            ahead = env.fork()
            t_before = env.t
        view, reward, done, truncated, info = env.step([])
        total += reward
        steps += 1
        assert not truncated
    assert steps == 8, "two hours of 15 minutes"
    outcome = info["outcome"]["trips"]
    failed = sum(1 for s in outcome["status"] if s == "failed")
    assert total == pytest.approx(-failed - 0.1 * sum(outcome["extra_walk_s"]) / 60)
    assert ahead.t == t_before, "the fork stayed where it was"
    # The fork goes on alone, with a decision of its own; a view ends at its own day's next step.
    first, _ = ahead.reset(seed=1)
    _, _, _, _, info = ahead.step([Move("bike_share", "a", "z", 1)])
    assert info["t"] == 27_900
    with pytest.raises(RuntimeError, match="has ended"):
        first["bike_share"]


def test_an_environment_is_deterministic_and_refuses_bad_input():
    net = grid()
    bikes = ms.service_bike_docked(docks(net, bikes=(1, 0)))
    trips = [trip(net, f"t{k}", 27_000 + 60 * k, (0, 0), (4, 4)) for k in range(4)]

    def run(seed):
        env = ms.fleet_environment(net, [bikes], trips, step_s=600)
        env.reset(seed)
        rewards, done = [], False
        while not done:
            _, r, done, _, _ = env.step()
            rewards.append(r)
        return rewards

    assert run(3) == run(3)
    env = ms.fleet_environment(net, [bikes], trips)
    with pytest.raises(RuntimeError, match="reset"):
        env.step()
    with pytest.raises(ValueError, match="unknown reward weight"):
        ms.fleet_environment(net, [bikes], trips, reward_weights={"w_nope": 1.0})


def test_each_seed_is_another_day_when_legs_are_thinned_or_shifted():
    net = grid()
    bikes = ms.service_bike_docked(docks(net, bikes=(2, 0)))
    trips = [trip(net, f"t{k}", 27_000 + 120 * k, (0, 0), (4, 4)) for k in range(30)]

    def total(env, seed):
        env.reset(seed)
        reward, done = 0.0, False
        while not done:
            _, r, done, _, _ = env.step()
            reward += r
        return reward

    same = ms.fleet_environment(net, [bikes], trips)
    assert total(same, 1) == total(same, 2), "nothing drawn: one day"
    varied = ms.fleet_environment(net, [bikes], trips, legs_keep=0.5, departure_jitter_s=300)
    days = {total(varied, seed) for seed in range(6)}
    assert len(days) > 1, "the seeds give other days"
    assert total(varied, 4) == total(varied, 4), "a seed replays its day"
    with pytest.raises(ValueError, match="legs_keep"):
        ms.fleet_environment(net, [bikes], trips, legs_keep=0.0)


def run_vector(env, seeds, decide=None):
    """Every day of a vector environment to its end: per day, its rewards."""
    views, _ = env.reset(seeds)
    rewards = [[] for _ in seeds]
    done = False
    while not done:
        lists = [decide(v) if decide else [] for v in views]
        views, r, terminated, truncated, _ = env.step(lists)
        assert len(views) == len(seeds) and r.shape == (len(seeds),)
        assert not any(truncated)
        for k, x in enumerate(r):
            rewards[k].append(float(x))
        done = all(terminated)
        assert done == any(terminated), "the days end together"
    return rewards


def test_a_vector_of_days_gives_each_seed_its_own_day_and_refuses_a_wrong_count():
    import time

    net = grid()
    bikes = ms.service_bike_docked(docks(net, bikes=(2, 0)))
    trips = [trip(net, f"t{k}", 27_000 + 120 * k, (0, 0), (4, 4)) for k in range(30)]
    kw = {"legs_keep": 0.5, "departure_jitter_s": 300, "step_s": 600}
    seeds = [3, 1, 4, 1]

    def decide(view):
        # A decision depending on the view, so the days' decisions differ.
        return [Target("bike_share", "z", 0)] if view["bike_share"]["bikes"][0] == 0 else []

    from openmobisim.operator import Target

    single = ms.fleet_environment(net, [bikes], trips, **kw)
    alone = []
    for s in seeds:
        view, _ = single.reset(s)
        rewards, done = [], False
        while not done:
            view, r, done, _, _ = single.step(decide(view))
            rewards.append(r)
        alone.append(rewards)
    vector = ms.fleet_environment(net, [bikes], trips, n=4, **kw)
    assert isinstance(vector, ms.fleet.FleetVectorEnvironment) and len(vector) == 4
    assert run_vector(vector, seeds, decide) == alone, "seed by seed, the same day"
    assert len({sum(r) for r in alone}) > 1, "the seeds are other days"
    vector.reset(seeds)
    with pytest.raises(ValueError, match="step takes 4 decision lists"):
        vector.step([[], []])
    with pytest.raises(ValueError, match="reset takes 4 seeds"):
        vector.reset([1])
    with pytest.raises(ValueError, match="n must be 1 or more"):
        ms.fleet_environment(net, [bikes], trips, n=0)

    # The time of n episodes, n = 1, 4 and 8, one after another and as a vector (printed; a
    # speed-up is expected, not asserted).
    for n in (1, 4, 8):
        started = time.perf_counter()
        for s in range(n):
            single.reset(s)
            done = False
            while not done:
                _, _, done, _, _ = single.step([])
        one_by_one = time.perf_counter() - started
        # A vector of one is timed alike (fleet_environment gives a single one for n=1).
        env = ms.fleet.FleetVectorEnvironment(single._inner.vector(n), net)  # noqa: SLF001
        started = time.perf_counter()
        run_vector(env, list(range(n)))
        together = time.perf_counter() - started
        print(f"fleet n={n}: one by one {one_by_one:.4f} s, as a vector {together:.4f} s")


def test_observation_and_action_have_fixed_shapes_and_the_engine_refuses_a_level_too_high():
    import numpy as np
    from openmobisim.operator import Target

    net = grid()
    bikes = ms.service_bike_docked(docks(net, bikes=(3, 0)))
    trips = [trip(net, f"t{k}", 27_000 + 60 * k, (0, 0), (4, 4)) for k in range(2)]
    env = ms.fleet_environment(net, [bikes], trips, step_s=900)
    spec = {"service": "bike_share"}
    assert env.layout()["bike_share"] == {
        "kind": "bike_docked",
        "dock_id": ["a", "z"],
        "slots": [4, 4],
    }
    view, _ = env.reset(0)
    obs = env.observation(view, spec)
    assert obs["docks"].dtype == np.float32 and obs["docks"].shape == (2, 5)
    # The first trip asks at 07:30 itself: one pickup at "a" already counted.
    assert obs["docks"].tolist() == [[2, 4, 1, 0, 0], [0, 4, 0, 0, 0]]
    hour = 2 * math.pi * 27_000 / 86_400
    assert obs["time"] == pytest.approx([0.0, math.sin(hour), math.cos(hour)], abs=1e-6)
    flat = env.observation(view, {**spec, "flat": True})
    assert flat.shape == (13,) and flat[:10].tolist() == obs["docks"].ravel().tolist()

    assert env.action([-1, 1], spec) == [Target("bike_share", "z", 1)]
    assert env.action(np.array([2, -1]), spec) == [Target("bike_share", "a", 2)]
    with pytest.raises(ValueError, match="2 docks"):
        env.action([1], spec)
    with pytest.raises(ValueError, match="unknown spec keys"):
        env.observation(view, {**spec, "size": 3})
    with pytest.raises(ValueError, match="must be one of"):
        env.action([1, 1], {"service": "nope"})
    _, _, _, _, info = env.step(env.action([-1, 9], spec))
    assert len(info["refused"]) == 1 and "9" in info["refused"][0], info["refused"]
    _, _, _, _, info = env.step(env.action([0, 1], spec))
    assert info["refused"] == [None, None]
    view, *_ = env.step([])
    t = env.observation(view, spec)["time"]
    assert t[0] == pytest.approx(3 * 900 / 7200)


def test_a_taxi_observation_counts_the_requests_waiting_within_reach():
    import numpy as np

    net = grid()
    v = ms.vehicle_read_table(
        [{"vehicle_id": "v1", "lon": at(net, 0, 0)[0], "lat": at(net, 0, 0)[1]}]
    )
    taxis = ms.service_taxi(v, options={"max_wait_s": 3600})
    # Asked before the window: two requests one block from the vehicle, then two at the far
    # corner. The vehicle takes the first; its node in the view is where it is heading.
    trips = [trip(net, f"a{k}", 26_000, (0, 1), (2, 2)) for k in range(2)]
    trips += [trip(net, f"b{k}", 26_010, (4, 3), (2, 2)) for k in range(2)]
    env = ms.fleet_environment(net, [taxis], trips)
    view, _ = env.reset(0)
    assert view["taxi"]["state"] == ["to_pickup"] and len(view["taxi"]["waiting_node"]) == 3
    obs = env.observation(view, {"service": "taxi", "reach_m": 300})
    assert obs["vehicles"].shape == (1, 3) and obs["vehicles"].dtype == np.float32
    code = ms.fleet.TAXI_STATES.index("to_pickup")
    assert obs["vehicles"][0].tolist() == pytest.approx([code, view["taxi"]["soc"][0], 1])
    wide = env.observation(view, {"service": "taxi", "reach_m": 5000, "flat": True})
    assert wide.shape == (6,) and wide[2] == 3
    with pytest.raises(ValueError, match="docked services only"):
        env.action([1], {"service": "taxi"})
