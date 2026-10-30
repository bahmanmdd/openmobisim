"""The reference policies of Powell's four classes, on toy days of a 10 x 10 grid of 200 m blocks.

Checks of sense, not of quality: each policy runs a toy day, the same seed gives the same
decisions, and on the toy it does no worse than deciding nothing (``noop``) on the standard
reward. The cost function and lookahead policies need SciPy and are skipped without it.
"""

from __future__ import annotations

import builtins
import sys

import numpy as np
import openmobisim as ms
import pytest
from openmobisim.contrib import policies as P

#: Days the policies are scored on; the tuning and training see other seeds.
SEEDS_VALIDATE = range(100, 106)
SEEDS_TRAIN = range(10)


def grid() -> object:
    return ms.examples.manhattan_grid(10, 200.0, False)


def at(net: object, row: int, col: int) -> tuple[float, float]:
    return ms._core.grid_node_lonlat(net, row, col)


def trip(net, ident, t, a, b) -> dict:
    o, d = at(net, *a), at(net, *b)
    return {
        "trip_id": ident,
        "origin_lon": o[0],
        "origin_lat": o[1],
        "destination_lon": d[0],
        "destination_lat": d[1],
        "departure": t,
    }


def bike_parts() -> tuple:
    """Nine docks of 10 slots, 5 bikes each; 80 trips from the south rows to the north row."""
    net = grid()
    places = [(r, c) for r in (0, 4, 9) for c in (0, 4, 9)]
    docks = ms.dock_read_table(
        [
            {"dock_id": f"d{r}{c}", "lon": at(net, r, c)[0], "lat": at(net, r, c)[1]}
            | {"capacity": 10, "bikes": 5}
            for r, c in places
        ]
    )
    rng = np.random.default_rng(7)
    trips = []
    for k in range(80):
        row = 0 if k % 4 else 4
        a, b = (row, int(rng.choice([0, 4, 9]))), (9, int(rng.choice([0, 4, 9])))
        trips.append(trip(net, f"t{k}", 27_000 + 90 * k, a, b))
    return net, [ms.service_bike_docked(docks)], trips


def bike_environment() -> object:
    """The bike toy; each seed keeps 80 % of the trips, each moved by up to 5 minutes."""
    net, services, trips = bike_parts()
    return ms.fleet_environment(
        net,
        services,
        trips,
        legs_keep=0.8,
        departure_jitter_s=300,
        zone_grid_m=1000,
    )


def taxi_environment() -> object:
    """Four taxis of 2 kWh along the south edge, two chargers; 30 requests far from them."""
    net = grid()
    socs = [0.12, 0.3, 0.5, 0.9]
    vehicles = ms.vehicle_read_table(
        [
            {"vehicle_id": f"v{k}", "lon": at(net, 0, k)[0], "lat": at(net, 0, k)[1]} | {"soc": s}
            for k, s in enumerate(socs)
        ]
    )
    chargers = ms.charger_read_table(
        [
            {"charger_id": c, "lon": at(net, *p)[0], "lat": at(net, *p)[1], "plugs": 1}
            for c, p in (("c1", (5, 5)), ("c2", (0, 9)))
        ]
    )
    rng = np.random.default_rng(3)
    trips = [
        trip(
            net,
            f"q{k}",
            27_000 + 240 * k,
            [(9, 9), (9, 0), (0, 9), (9, 5)][k % 4],
            (int(rng.integers(10)), int(rng.integers(10))),
        )
        for k in range(30)
    ]
    taxis = ms.service_taxi(vehicles, chargers, options={"max_wait_s": 120, "battery_kwh": 2, "hail_s": 0})
    return ms.fleet_environment(
        net,
        [taxis],
        trips,
        step_s=300,
        legs_keep=0.8,
        departure_jitter_s=300,
        zone_grid_m=1000,
    )


def decisions(policy, environment, seed: int) -> list:
    """Every decision a policy takes over one day, by step."""
    view, _ = environment.reset(seed)
    if hasattr(policy, "day_start"):
        policy.day_start({})
    out, done = [], False
    while not done:
        ds = list(policy.step(view) or [])
        out.append(ds)
        view, _, done, _, _ = environment.step(ds)
    return out


def bike_policies(env) -> dict:
    vfa = P.policy_value_table()
    vfa.train(env, 300, seed=0)
    out = {"vfa": vfa}
    try:
        import scipy  # noqa: F401
    except ImportError:
        return out
    out["cfa"] = P.policy_flow_targets()
    out["dla"] = P.policy_lookahead()
    out["dla_simulated"] = P.policy_lookahead_simulated(
        env, [None, ms.operator.policy_threshold()], horizon_s=1800
    )
    return out


def taxi_policies(env) -> dict:
    vfa = P.policy_value_table(step_s=300)
    vfa.train(env, 300, seed=0)
    out = {"vfa": vfa}
    try:
        import scipy  # noqa: F401
    except ImportError:
        return out
    # The cost function's terms tuned on the training days (a CFA is tuned by definition).
    out["cfa"] = P.tune(
        lambda charge_below, soc_penalty_s: P.policy_assignment(
            charge_below, soc_penalty_s=soc_penalty_s
        ),
        {"charge_below": (0.1, 0.2, 0.3, 0.4), "soc_penalty_s": (0.0, 300.0, 1200.0)},
        env,
        seeds_train=SEEDS_TRAIN,
        seeds_validate=SEEDS_VALIDATE,
    )["policy"]
    out["dla"] = P.policy_lookahead(step_s=300)
    out["dla_simulated"] = P.policy_lookahead_simulated(
        env, [None, P.policy_assignment()], horizon_s=1800
    )
    return out


@pytest.mark.parametrize("service", ["bike", "taxi"])
def test_each_reference_policy_runs_a_toy_day_and_does_no_worse_than_none(service):
    env = bike_environment() if service == "bike" else taxi_environment()
    policies = bike_policies(env) if service == "bike" else taxi_policies(env)
    none = P.evaluate(None, env, SEEDS_VALIDATE)
    assert none["policy"]["name"] == "noop" and len(none["rewards"]) == len(SEEDS_VALIDATE)
    for name, policy in policies.items():
        got = P.evaluate(policy, env, SEEDS_VALIDATE)
        assert got["reward_mean"] >= none["reward_mean"], (name, got, none)
        assert got["seconds"] > 0 and "name" in got["policy"]
        assert policy.step_s > 0 and "name" in policy.describe()


@pytest.mark.parametrize("service", ["bike", "taxi"])
def test_the_same_seed_gives_the_same_decisions(service):
    env = bike_environment() if service == "bike" else taxi_environment()
    policies = bike_policies(env) if service == "bike" else taxi_policies(env)
    for name, policy in policies.items():
        first, again = decisions(policy, env, 3), decisions(policy, env, 3)
        assert first == again, name
        assert any(first), f"{name} decided nothing on the toy"


def test_the_tuned_rules_beat_the_untuned_on_days_the_tuning_did_not_see():
    bikes = P.tune_threshold(
        bike_environment(), seeds_train=SEEDS_TRAIN, seeds_validate=SEEDS_VALIDATE
    )
    assert bikes["parameters"] != {"low": 0.2, "high": 0.8}
    assert bikes["validate"]["reward_mean"] > bikes["validate_untuned"]["reward_mean"]
    assert isinstance(bikes["policy"], ms.operator.PolicyBuiltin), "fleet_day runs it in Rust"
    best = max(g["reward_mean"] for g in bikes["grid"])
    assert bikes["train"]["reward_mean"] == best
    taxis = P.tune_charge_rule(
        taxi_environment(), seeds_train=SEEDS_TRAIN, seeds_validate=SEEDS_VALIDATE, step_s=300
    )
    assert taxis["validate"]["reward_mean"] > taxis["validate_untuned"]["reward_mean"]


def test_the_python_forms_of_the_built_in_rules_decide_as_the_rules_do():
    net, services, trips = bike_parts()
    rule = ms.operator.policy_threshold(0.3, 0.6)
    rust = ms.fleet_day(net, services, trips, policy=rule)
    python = ms.fleet_day(net, services, trips, policy=P._python_policy(rule))
    assert rust.decisions().row_count > 0
    for column in ("step", "kind", "dock", "level", "from_dock", "to_dock", "moved"):
        a, b = list(rust.decisions()[column]), list(python.decisions()[column])
        assert [str(x) for x in a] == [str(x) for x in b], column
    assert rust.trips["status"] == python.trips["status"]
    assert P._python_policy("noop").describe()["name"] == "noop"


@pytest.mark.parametrize("service", ["bike", "taxi"])
def test_the_value_table_changes_over_training_and_the_same_seed_gives_the_same_table(service):
    env = bike_environment() if service == "bike" else taxi_environment()
    step_s = 900.0 if service == "bike" else 300.0
    a, b = P.policy_value_table(step_s=step_s), P.policy_value_table(step_s=step_s)
    assert a.table == {}
    short = dict(a.train(env, 10, seed=5))
    long = a.train(env, 100, seed=5)
    assert short and short != long, "the table moves with more episodes"
    assert b.train(env, 10, seed=5) == short, "the same seed, the same table"
    assert P.policy_value_table(step_s=step_s).train(env, 10, seed=6) != short


def test_without_scipy_the_programs_say_which_extra_to_install(monkeypatch):
    real = builtins.__import__

    def no_scipy(name, *args, **kwargs):
        if name == "scipy" or name.startswith("scipy."):
            raise ImportError(f"No module named {name!r}")
        return real(name, *args, **kwargs)

    for mod in [m for m in sys.modules if m == "scipy" or m.startswith("scipy.")]:
        monkeypatch.delitem(sys.modules, mod)
    monkeypatch.setattr(builtins, "__import__", no_scipy)
    for make in (P.policy_flow_targets, P.policy_assignment, P.policy_lookahead):
        with pytest.raises(ImportError, match=r"openmobisim\[policies\]"):
            make()
    # The rest makes and runs without SciPy.
    env = bike_environment()
    assert P.evaluate(P.policy_value_table(), env, [0])["rewards"]
    assert P.evaluate(ms.operator.policy_threshold(), env, [0])["rewards"]
