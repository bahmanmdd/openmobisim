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
