"""Reference operator policies, one of each of Powell's four classes, for docked bikes and taxis.

Powell (2019, *A unified framework for stochastic optimization*, European Journal of
Operational Research 275(3) 795-821, https://doi.org/10.1016/j.ejor.2018.07.014) sorts the
policies of sequential decisions into four classes:

- **PFA**, a policy function approximation: a rule with tunable parameters. The built-in
  ``operator.policy_threshold`` and ``operator.policy_charge_rule``, tuned here by a grid over
  episodes (``tune_threshold``, ``tune_charge_rule``; ``tune`` for any policy).
- **CFA**, a cost function approximation: a deterministic optimisation each step with tuned
  terms. ``policy_flow_targets`` (docked bikes: a min-cost transportation problem from docks
  above their band to docks below it) and ``policy_assignment`` (taxis: waiting requests to
  idle vehicles, low charge penalised, low vehicles sent to charge).
- **VFA**, a value function approximation: decisions by the reward now plus an estimated value
  of the state reached. ``policy_value_table``: a lookup table of values per (zone, fill band,
  hour) for docks or (zone, charge band, hour) for taxis, learned over episodes
  (``train``), decisions by one-step greedy over a small set of candidates.
- **DLA**, a direct lookahead: an optimisation over a forecast horizon, re-solved each step,
  its first decisions applied. ``policy_lookahead`` (a mixed-integer program on
  ``view.expected_legs``) and ``policy_lookahead_simulated`` (a few candidate policies
  simulated ahead on ``environment.fork()``, the best one's decisions applied).

They are references, simple and readable, to compare one's own policies with: checks of sense,
not tuned for quality. Each is a policy object of ``openmobisim.operator`` (``step(view)``,
``step_s``, ``describe()``), usable in ``fleet_day`` and stepped on a ``fleet_environment``
by ``evaluate``. Every parameter's default is an assumption, not calibrated.

The CFA and the DLA need SciPy (``pip install "openmobisim[policies]"``), imported when such a
policy is made; the rest needs numpy only.

**Bikes and taxis.** Each class has a reference for each service kind, chosen by the service's
kind in the view; a policy steers every service of its kind in a day.
"""

from __future__ import annotations

import math
import time
from collections.abc import Callable, Iterable, Mapping, Sequence
from itertools import product
from typing import Any

import numpy as np

from openmobisim.fleet import REWARD_WEIGHTS
from openmobisim.operator import Charge, Move, PolicyBuiltin, Relocate, Target

__all__ = [
    "evaluate",
    "policy_assignment",
    "policy_flow_targets",
    "policy_lookahead",
    "policy_lookahead_simulated",
    "policy_value_table",
    "tune",
    "tune_charge_rule",
    "tune_threshold",
]

#: The speed a van or taxi is taken to drive at when a policy turns seconds into kilometres
#: for its own cost estimates, in metres a second: an assumption, not calibrated.
_SPEED_M_S = 8.0


def _scipy_optimize() -> Any:
    """``scipy.optimize``, or an ``ImportError`` naming the extra that installs it."""
    try:
        import scipy.optimize as opt
    except ImportError as e:
        raise ImportError(
            "this policy needs SciPy, which openmobisim does not install by default: "
            'pip install "openmobisim[policies]"'
        ) from e
    return opt


def _services(view: Any, kind: str) -> list[str]:
    """The names of the view's services of ``kind`` (``bike_docked`` or ``taxi``)."""
    return [s for s in view.services if view[s]["kind"] == kind]


# --- the built-in rules, in Python, so that an environment can step them -----------------


class _Noop:
    step_s = 900.0

    def describe(self) -> dict[str, Any]:
        return {"name": "noop", "parameters": {}}

    def step(self, view: Any) -> list[Any]:
        return []


class _Threshold:
    """``operator.policy_threshold``'s rule, deciding the same targets."""

    def __init__(self, low: float, high: float, step_s: float = 900.0) -> None:
        self.low, self.high, self.step_s = float(low), float(high), float(step_s)

    def describe(self) -> dict[str, Any]:
        return {"name": "threshold", "parameters": {"low": self.low, "high": self.high}}

    def step(self, view: Any) -> list[Any]:
        out = []
        for name in _services(view, "bike_docked"):
            d = view[name]
            for dock, bikes, slots in zip(d["dock_id"], d["bikes"], d["slots"], strict=True):
                mid = slots * (self.low + self.high) / 2
                if bikes < self.low * slots:
                    out.append(Target(name, dock, math.floor(mid)))
                elif bikes > self.high * slots:
                    out.append(Target(name, dock, math.ceil(mid)))
        return out


class _ChargeRule:
    """``operator.policy_charge_rule``'s rule, deciding the same charges."""

    def __init__(self, charge_below: float, charge_to: float, step_s: float = 900.0) -> None:
        self.charge_below, self.charge_to = float(charge_below), float(charge_to)
        self.step_s = float(step_s)

    def describe(self) -> dict[str, Any]:
        return {
            "name": "charge_rule",
            "parameters": {"charge_below": self.charge_below, "charge_to": self.charge_to},
        }

    def step(self, view: Any) -> list[Any]:
        out = []
        for name in _services(view, "taxi"):
            v = view[name]
            for vid, state, soc, charger in zip(
                v["vehicle_id"], v["state"], v["soc"], v["nearest_charger"], strict=True
            ):
                if state == "idle" and soc < self.charge_below and charger is not None:
                    out.append(Charge(name, vid, charger, self.charge_to))
        return out


def _python_policy(policy: Any) -> Any:
    """A policy an environment can step: a built-in one in its Python form."""
    if policy is None:
        return _Noop()
    if isinstance(policy, str):
        policy = PolicyBuiltin(policy, {})
    if isinstance(policy, PolicyBuiltin):
        p = dict(policy.parameters)
        step_s = p.get("step_s", 900.0)
        if policy.name == "noop":
            return _Noop()
        if policy.name == "threshold":
            return _Threshold(p.get("low", 0.2), p.get("high", 0.8), step_s)
        if policy.name == "charge_rule":
            return _ChargeRule(p.get("charge_below", 0.2), p.get("charge_to", 0.8), step_s)
        raise ValueError(
            f"the built-in policy {policy.name!r} has no Python form here; "
            "noop, threshold and charge_rule have"
        )
    if not callable(getattr(policy, "step", None)):
        raise ValueError(f"a policy has a step(view) method; {policy!r} has none")
    return policy


def _describe(policy: Any) -> dict[str, Any]:
    describe = getattr(policy, "describe", None)
    if not callable(describe):
        return {"name": f"python:{type(policy).__name__}", "parameters": {}}
    d = describe()
    return {"name": d, "parameters": {}} if isinstance(d, str) else dict(d)


def _episode(policy: Any, environment: Any, seed: int) -> float:
    view, _ = environment.reset(int(seed))
    start = getattr(policy, "day_start", None)
    if callable(start):
        start({"window": environment.window, "seed": int(seed), "services": list(view.services)})
    total, done = 0.0, False
    while not done:
        decisions = policy.step(view) or []
        view, reward, terminated, truncated, _ = environment.step(decisions)
        total += float(reward)
        done = bool(terminated or truncated)
    return total


def evaluate(policy: Any, environment: Any, seeds: Iterable[int]) -> dict[str, Any]:
    """A policy's episode rewards on a fleet environment, one day per seed.

    Each seed's day is stepped from its start to its end, the policy deciding at every step of
    the environment (the environment's ``step_s`` rules, not the policy's). The reward is the
    environment's standard one (``fleet_environment``: minus the failed legs, the minutes
    waited and walked, the van kilometres, by their weights).

    Args:
        policy: A policy object (``step(view)``), a built-in ``noop``, ``threshold`` or
            ``charge_rule`` (``operator.policy_threshold(...)`` or its name; stepped in its
            Python form, deciding the same), or ``None``: no decisions.
        environment: A ``fleet_environment`` (``n=1``), or a callable returning one.
        seeds: The days' seeds.

    Returns:
        ``{"policy", "seeds", "rewards", "reward_mean", "reward_std", "seconds",
        "seconds_episode"}``: the policy's description, the seeds and each one's episode
        reward, their mean and standard deviation (0 for one seed), the wall-clock seconds in
        all and per episode.
    """
    env = _environment(environment)
    pol = _python_policy(policy)
    seeds = [int(s) for s in seeds]
    if not seeds:
        raise ValueError("evaluate needs at least one seed")
    started = time.perf_counter()
    rewards = [_episode(pol, env, s) for s in seeds]
    seconds = time.perf_counter() - started
    return {
        "policy": _describe(pol),
        "seeds": seeds,
        "rewards": rewards,
        "reward_mean": float(np.mean(rewards)),
        "reward_std": float(np.std(rewards)),
        "seconds": seconds,
        "seconds_episode": seconds / len(seeds),
    }


def _environment(environment: Any) -> Any:
    if callable(environment) and not hasattr(environment, "reset"):
        return environment()
    return environment


# --- PFA: tuning a rule over episodes ----------------------------------------------------


def tune(
    policy_make: Callable[..., Any],
    grid: Mapping[str, Sequence[float]],
    environment: Any,
    *,
    seeds_train: Sequence[int] = (0, 1, 2, 3),
    seeds_validate: Sequence[int] = (100, 101, 102, 103),
    parameters_untuned: Mapping[str, float] | None = None,
) -> dict[str, Any]:
    """A policy's parameters chosen by a grid search over training days, scored on others.

    Every combination of ``grid``'s values is evaluated on ``seeds_train``; the one of highest
    mean episode reward (the first in the grid's order among equals) is evaluated on
    ``seeds_validate``, days the search did not see, beside the untuned parameters'.

    Args:
        policy_make: Called with one combination as keywords; returns a policy, or ``None`` to
            skip a combination that is not valid.
        grid: The values to try per parameter name.
        environment: A ``fleet_environment`` (``n=1``) or a callable returning one; with
            ``legs_keep`` below 1 or a ``departure_jitter_s``, each seed is another day.
        seeds_train: The days the search scores on.
        seeds_validate: The days the chosen parameters are scored on.
        parameters_untuned: The parameters to compare with on the validation days, or
            ``None``: not compared.

    Returns:
        ``{"parameters", "policy", "train", "validate", "validate_untuned", "grid"}``: the
        chosen parameters and policy, ``evaluate``'s result for them on the training and the
        validation days, the untuned parameters' on the validation days (or ``None``), and per
        combination tried ``{"parameters", "reward_mean"}`` on the training days.
    """
    env = _environment(environment)
    names = list(grid)
    tried, best = [], None
    for values in product(*(grid[k] for k in names)):
        params = dict(zip(names, (float(v) for v in values), strict=True))
        policy = policy_make(**params)
        if policy is None:
            continue
        score = evaluate(policy, env, seeds_train)
        tried.append({"parameters": params, "reward_mean": score["reward_mean"]})
        if best is None or score["reward_mean"] > best[2]["reward_mean"]:
            best = (params, policy, score)
    if best is None:
        raise ValueError("no combination of the grid makes a policy")
    params, policy, train = best
    untuned = None
    if parameters_untuned is not None:
        untuned = evaluate(policy_make(**parameters_untuned), env, seeds_validate)
    return {
        "parameters": params,
        "policy": policy,
        "train": train,
        "validate": evaluate(policy, env, seeds_validate),
        "validate_untuned": untuned,
        "grid": tried,
    }


def tune_threshold(
    environment: Any,
    grid: Mapping[str, Sequence[float]] | None = None,
    *,
    seeds_train: Sequence[int] = (0, 1, 2, 3),
    seeds_validate: Sequence[int] = (100, 101, 102, 103),
    step_s: float = 900.0,
) -> dict[str, Any]:
    """``operator.policy_threshold``'s ``low`` and ``high`` tuned over training days.

    As ``tune``, the untuned being the rule's defaults (0.2, 0.8). The policy returned is the
    built-in one (``fleet_day`` runs it in Rust). ``low = 0`` never tops a dock up: the grid
    then holds doing nothing, and the training score can only beat it or equal it.

    Args:
        environment: As ``tune``.
        grid: ``{"low": [...], "high": [...]}``, or ``None``: ``low`` in 0, 0.1, 0.2, 0.3 and
            ``high`` in 0.6, 0.7, 0.8, 0.9, 1 (pairs with ``low >= high`` skipped).
        seeds_train: As ``tune``.
        seeds_validate: As ``tune``.
        step_s: The policy's step, as its built-in form records it.
    """
    from openmobisim.operator import policy_threshold

    grid = grid or {"low": (0.0, 0.1, 0.2, 0.3), "high": (0.6, 0.7, 0.8, 0.9, 1.0)}

    def make(low: float, high: float) -> Any:
        return policy_threshold(low, high, step_s=step_s) if low < high else None

    return tune(
        make,
        grid,
        environment,
        seeds_train=seeds_train,
        seeds_validate=seeds_validate,
        parameters_untuned={"low": 0.2, "high": 0.8},
    )


def tune_charge_rule(
    environment: Any,
    grid: Mapping[str, Sequence[float]] | None = None,
    *,
    seeds_train: Sequence[int] = (0, 1, 2, 3),
    seeds_validate: Sequence[int] = (100, 101, 102, 103),
    step_s: float = 900.0,
) -> dict[str, Any]:
    """``operator.policy_charge_rule``'s ``charge_below`` and ``charge_to`` tuned over days.

    As ``tune_threshold``, the untuned being the rule's defaults (0.2, 0.8); ``charge_below
    = 0`` never sends a vehicle (the fleet's own charging rule still acts).

    Args:
        environment: As ``tune``.
        grid: ``{"charge_below": [...], "charge_to": [...]}``, or ``None``: ``charge_below`` in
            0, 0.1, 0.2, 0.3, 0.4 and ``charge_to`` in 0.6, 0.8, 1.
        seeds_train: As ``tune``.
        seeds_validate: As ``tune``.
        step_s: The policy's step, as its built-in form records it.
    """
    from openmobisim.operator import policy_charge_rule

    grid = grid or {"charge_below": (0.0, 0.1, 0.2, 0.3, 0.4), "charge_to": (0.6, 0.8, 1.0)}

    def make(charge_below: float, charge_to: float) -> Any:
        if not charge_below < charge_to:
            return None
        return policy_charge_rule(charge_below, charge_to, step_s=step_s)

    return tune(
        make,
        grid,
        environment,
        seeds_train=seeds_train,
        seeds_validate=seeds_validate,
        parameters_untuned={"charge_below": 0.2, "charge_to": 0.8},
    )


# --- CFA: a deterministic optimisation each step ----------------------------------------


def _bands(d: Mapping[str, Any], low: float, high: float) -> tuple[np.ndarray, np.ndarray]:
    """Per dock, the bikes it can give and the bikes it lacks against the band [low, high].

    Bikes in vans on their way (``incoming``) count as there. A dock above ``high`` of its
    slots gives down to the band's middle rounded up; one below ``low`` lacks up to the middle
    rounded down (the threshold rule's levels).
    """
    bikes = np.asarray(d["bikes"], dtype=float) + np.asarray(d["incoming"], dtype=float)
    slots = np.asarray(d["slots"], dtype=float)
    mid = slots * (low + high) / 2
    give = np.where(bikes > high * slots, bikes - np.ceil(mid), 0.0)
    lack = np.where(bikes < low * slots, np.floor(mid) - bikes, 0.0)
    return np.maximum(give, 0.0), np.maximum(lack, 0.0)


class PolicyFlowTargets:
    """CFA for docked bikes: a transportation problem each step (``policy_flow_targets``)."""

    def __init__(self, low: float, high: float, holding_s: float, step_s: float) -> None:
        if not 0.0 <= low < high <= 1.0:
            raise ValueError(f"0 <= low < high <= 1, got low={low}, high={high}")
        self._opt = _scipy_optimize()
        self.low, self.high = float(low), float(high)
        self.holding_s, self.step_s = float(holding_s), float(step_s)

    def describe(self) -> dict[str, Any]:
        """Its name and parameters."""
        return {
            "name": "flow_targets",
            "parameters": {"low": self.low, "high": self.high, "holding_s": self.holding_s},
        }

    def step(self, view: Any) -> list[Any]:
        """The moves of the step's transportation problem, per docked service."""
        out: list[Any] = []
        for name in _services(view, "bike_docked"):
            d = view[name]
            give, lack = _bands(d, self.low, self.high)
            src, dst = np.flatnonzero(give > 0), np.flatnonzero(lack > 0)
            if len(src) == 0 or len(dst) == 0:
                continue
            nodes = d["node"]
            tt = view.travel_times(name, [nodes[i] for i in src], [nodes[j] for j in dst])
            flows = self._solve(tt, give[src], lack[dst])
            for a, b in zip(*np.nonzero(flows), strict=True):
                n = round(flows[a, b])
                if n > 0:
                    out.append(Move(name, d["dock_id"][src[a]], d["dock_id"][dst[b]], n))
        return out

    def _solve(self, tt: np.ndarray, supply: np.ndarray, demand: np.ndarray) -> np.ndarray:
        """Minimise sum(tt x) + holding_s sum(u): sum_j x_ij <= s_i, sum_i x_ij + u_j = d_j."""
        ns, nd = tt.shape
        cost = np.where(np.isfinite(tt), tt, 0.0)
        bounds = [(0.0, 0.0 if not np.isfinite(t) else None) for t in tt.ravel()]
        c = np.concatenate([cost.ravel(), np.full(nd, self.holding_s)])
        bounds += [(0.0, None)] * nd
        a_ub = np.zeros((ns, ns * nd + nd))
        for i in range(ns):
            a_ub[i, i * nd : (i + 1) * nd] = 1.0
        a_eq = np.zeros((nd, ns * nd + nd))
        for j in range(nd):
            a_eq[j, j : ns * nd : nd] = 1.0
            a_eq[j, ns * nd + j] = 1.0
        res = self._opt.linprog(
            c, A_ub=a_ub, b_ub=supply, A_eq=a_eq, b_eq=demand, bounds=bounds, method="highs"
        )
        if res.status != 0:
            return np.zeros((ns, nd))
        # A transportation problem with whole supplies and demands has whole vertex solutions.
        return np.round(res.x[: ns * nd]).reshape(ns, nd)


def policy_flow_targets(
    low: float = 0.2, high: float = 0.8, *, holding_s: float = 1800.0, step_s: float = 900.0
) -> PolicyFlowTargets:
    """CFA for docked bikes: bikes moved by a min-cost transportation problem each step.

    Each step, per docked service, docks holding (with the bikes on their way) more than
    ``high`` of their slots can give down to the middle of the band, and docks holding fewer
    than ``low`` lack up to it (the threshold rule's levels). The bikes moved minimise the
    seconds driven between docks (``view.travel_times`` at the step, per bike) plus
    ``holding_s`` per bike a dock still lacks: a bike is moved only when the drive is shorter
    than ``holding_s``, the tuned term. Solved by ``scipy.optimize.linprog`` (HiGHS); a
    transportation problem with whole supplies and demands has a whole optimum, so the moves
    are whole bikes. A move above the service's ``van_bikes`` is cut to it by the engine.

    Args:
        low: Below this share of its slots, a dock lacks bikes.
        high: Above this share, a dock gives bikes (``0 <= low < high <= 1``).
        holding_s: The cost of a bike a dock still lacks, in seconds of driving.
        step_s: The seconds between its steps.

    Raises:
        ImportError: Without SciPy (the ``policies`` extra).
    """
    return PolicyFlowTargets(low, high, holding_s, step_s)


class PolicyAssignment:
    """CFA for taxis: waiting requests to idle vehicles each step (``policy_assignment``)."""

    def __init__(
        self, charge_below: float, charge_to: float, soc_penalty_s: float, step_s: float
    ) -> None:
        if not 0.0 <= charge_below < charge_to <= 1.0:
            raise ValueError(f"0 <= charge_below < charge_to <= 1, got {charge_below}, {charge_to}")
        self._opt = _scipy_optimize()
        self.charge_below, self.charge_to = float(charge_below), float(charge_to)
        self.soc_penalty_s, self.step_s = float(soc_penalty_s), float(step_s)

    def describe(self) -> dict[str, Any]:
        """Its name and parameters."""
        return {
            "name": "assignment",
            "parameters": {
                "charge_below": self.charge_below,
                "charge_to": self.charge_to,
                "soc_penalty_s": self.soc_penalty_s,
            },
        }

    def step(self, view: Any) -> list[Any]:
        """Charges for the low vehicles, relocations of the others to the waiting requests."""
        out: list[Any] = []
        for name in _services(view, "taxi"):
            v = view[name]
            free = []
            for k, (vid, state, soc, charger) in enumerate(
                zip(v["vehicle_id"], v["state"], v["soc"], v["nearest_charger"], strict=True)
            ):
                if state != "idle":
                    continue
                if soc < self.charge_below and charger is not None:
                    out.append(Charge(name, vid, charger, self.charge_to))
                else:
                    free.append(k)
            waiting = list(v["waiting_node"])
            if not free or not waiting:
                continue
            tt = view.travel_times(name, [v["node"][k] for k in free], waiting)
            soc = np.asarray(v["soc"])[free]
            # Low charge costs: a vehicle near charge_below is used last.
            cost = tt + self.soc_penalty_s * np.clip(1.0 - soc, 0.0, 1.0)[:, None]
            big = 1e9
            rows, cols = self._opt.linear_sum_assignment(np.where(np.isfinite(cost), cost, big))
            for r, c in zip(rows, cols, strict=True):
                if np.isfinite(tt[r, c]):
                    out.append(Relocate(name, v["vehicle_id"][free[r]], waiting[c]))
        return out


def policy_assignment(
    charge_below: float = 0.2,
    charge_to: float = 0.8,
    *,
    soc_penalty_s: float = 300.0,
    step_s: float = 300.0,
) -> PolicyAssignment:
    """CFA for taxis: waiting requests matched to idle vehicles by an assignment each step.

    The engine matches a request to the nearest idle vehicle that can reach it within the
    fleet's ``max_wait_s`` less its hailing time ``hail_s`` the moment it is made; a request no
    vehicle can reach in time **waits**
    (``waiting_node`` in the view). Each step, this policy (1) sends every idle vehicle below
    ``charge_below`` to its nearest charger, to charge to ``charge_to``, and (2) assigns the
    other idle vehicles to the waiting requests, one each, minimising the seconds to the pickup
    (``view.travel_times``) plus ``soc_penalty_s`` × the share of battery used, by
    ``scipy.optimize.linear_sum_assignment``; each assigned vehicle is relocated to its
    request's road node. The engine offers waiting requests, oldest first, to a vehicle that
    ends a relocation: the request is served if the vehicle, hailing time included, arrives
    within the request's ``max_wait_s``. The engine does the match, so nothing is assigned
    twice; the assignment
    chooses which vehicle drives, not which request it takes on arrival.

    Args:
        charge_below: Below this share of its battery, an idle vehicle is sent to charge.
        charge_to: The share it charges to (``0 <= charge_below < charge_to <= 1``).
        soc_penalty_s: The cost of a battery used up, in seconds of driving.
        step_s: The seconds between its steps (300: a request waits ``max_wait_s``, 600 by
            default, so a longer step reaches few in time).

    Raises:
        ImportError: Without SciPy (the ``policies`` extra).
    """
    return PolicyAssignment(charge_below, charge_to, soc_penalty_s, step_s)


# --- VFA: a lookup table of values, learned over episodes --------------------------------


def _hour(view: Any) -> int:
    return int(view.t // 3600) % 24


class _ZoneNodes:
    """Per zone, a road node seen there (the smallest id), for relocations to a zone."""

    def __init__(self) -> None:
        self.nodes: dict[int, str] = {}

    def see(self, zones: Iterable[int], nodes: Iterable[str]) -> None:
        for z, n in zip(zones, nodes, strict=True):
            z = int(z)
            if z >= 0 and (z not in self.nodes or n < self.nodes[z]):
                self.nodes[z] = n

    def see_view(self, v: Mapping[str, Any]) -> None:
        self.see(v["zone"], v["node"])
        self.see(v["waiting_zone"], v["waiting_node"])


def _greedy_moves(
    view: Any, name: str, d: Mapping[str, Any], low: float, high: float
) -> list[tuple[int, int, int, float]]:
    """The threshold band's moves paired nearest first: ``(from, to, bikes, seconds)``."""
    give, lack = _bands(d, low, high)
    src, dst = np.flatnonzero(give > 0), np.flatnonzero(lack > 0)
    if len(src) == 0 or len(dst) == 0:
        return []
    nodes = d["node"]
    tt = view.travel_times(name, [nodes[i] for i in src], [nodes[j] for j in dst])
    give, lack = give[src].copy(), lack[dst].copy()
    moves = []
    for flat in np.argsort(tt, axis=None, kind="stable"):
        a, b = divmod(int(flat), len(dst))
        if not np.isfinite(tt[a, b]):
            break
        n = int(min(give[a], lack[b]))
        if n > 0:
            moves.append((int(src[a]), int(dst[b]), n, float(tt[a, b])))
            give[a] -= n
            lack[b] -= n
    return moves


_CHARGING = ("to_charger", "queued_at_charger", "charging")


class PolicyValueTable:
    """VFA: a value per (zone, band, hour), learned over episodes (``policy_value_table``)."""

    def __init__(
        self,
        bands: int,
        learning_rate: float,
        explore: float,
        candidates: Sequence[tuple[float, float]],
        charge_to: float,
        step_s: float,
        weights: Mapping[str, float],
    ) -> None:
        if bands < 2:
            raise ValueError(f"bands must be 2 or more, got {bands}")
        self.bands, self.learning_rate, self.explore = int(bands), float(learning_rate), explore
        self.candidates = [(float(a), float(b)) for a, b in candidates]
        self.charge_to, self.step_s = float(charge_to), float(step_s)
        self.weights = {**REWARD_WEIGHTS, **dict(weights)}
        self.table: dict[tuple[str, int, int, int], float] = {}
        self._rng: np.random.Generator | None = None
        self._zones = _ZoneNodes()
        self._last: dict[str, list[tuple[int, int, int]]] = {}
        self._busy: dict[str, np.ndarray] = {}

    def describe(self) -> dict[str, Any]:
        """Its name and parameters (the table's size counted)."""
        return {
            "name": "value_table",
            "parameters": {
                "bands": self.bands,
                "learning_rate": self.learning_rate,
                "entries": len(self.table),
            },
        }

    def day_start(self, day: Mapping[str, Any]) -> None:
        """Forget the last step's states (a new day)."""
        self._last = {}
        self._busy = {}

    def _band(self, share: float) -> int:
        return min(max(int(share * self.bands), 0), self.bands - 1)

    def _value(self, name: str, key: tuple[int, int, int]) -> float:
        return self.table.get((name, *key), 0.0)

    def _learn(self, name: str, keys: list[tuple[int, int, int]], gains: np.ndarray) -> None:
        """Each unit's last post-decision value moved toward its gain since plus its value now."""
        last = self._last.get(name)
        if self._rng is None or last is None or len(last) != len(keys):
            return
        a = self.learning_rate
        for old, new, g in zip(last, keys, gains, strict=True):
            k = (name, *old)
            self.table[k] = (1 - a) * self.table.get(k, 0.0) + a * (g + self._value(name, new))

    def step(self, view: Any) -> list[Any]:
        """One-step greedy on the reward now plus the value of the state reached, per service."""
        out: list[Any] = []
        for name in _services(view, "bike_docked"):
            out += self._bikes(view, name)
        for name in _services(view, "taxi"):
            out += self._taxis(view, name)
        return out

    def _bikes(self, view: Any, name: str) -> list[Any]:
        d = view[name]
        hour = _hour(view)
        bikes = np.asarray(d["bikes"], dtype=float) + np.asarray(d["incoming"], dtype=float)
        slots = np.maximum(np.asarray(d["slots"], dtype=float), 1.0)
        zones = [int(z) for z in d["zone"]]
        # The gain of each dock since the last step: its failed pickups, by the reward's weight.
        gains = -self.weights["w_fail"] * np.asarray(d["failed"], dtype=float)
        now = [(z, self._band(b / s), hour) for z, b, s in zip(zones, bikes, slots, strict=True)]
        self._learn(name, now, gains)
        options: list[tuple[float, list[tuple[int, int, int, float]], np.ndarray]] = []
        for low, high in [(0.0, 1.0), *self.candidates]:
            moves = [] if low == 0.0 and high == 1.0 else _greedy_moves(view, name, d, low, high)
            after = bikes.copy()
            km = 0.0
            for a, b, n, s in moves:
                after[a] -= n
                after[b] += n
                km += s * _SPEED_M_S / 1000.0
            value = sum(
                self._value(name, (z, self._band(x / s), hour))
                for z, x, s in zip(zones, after, slots, strict=True)
            )
            options.append((value - self.weights["w_km"] * km, moves, after))
        pick = max(range(len(options)), key=lambda k: (options[k][0], -k))
        if self._rng is not None and self._rng.random() < self.explore:
            pick = int(self._rng.integers(len(options)))
        _, moves, after = options[pick]
        self._last[name] = [
            (z, self._band(x / s), hour) for z, x, s in zip(zones, after, slots, strict=True)
        ]
        return [Move(name, d["dock_id"][a], d["dock_id"][b], n) for a, b, n, _ in moves]

    def _taxis(self, view: Any, name: str) -> list[Any]:
        v = view[name]
        self._zones.see_view(v)
        hour = _hour(view)
        soc = np.asarray(v["soc"], dtype=float)
        zones = [int(z) for z in v["zone"]]
        # The gain of each vehicle since the last step: a vehicle that has taken a traveller
        # since (fetching or carrying one now, not at the last step) stands for a request
        # served, by the reward's weight of a failed one (a proxy: requests taken and dropped
        # within one step are missed).
        busy = np.array([s in ("to_pickup", "occupied") for s in v["state"]], dtype=bool)
        before = self._busy.get(name, np.zeros(len(busy), dtype=bool))
        gains = self.weights["w_fail"] * (busy & ~before).astype(float)
        self._busy[name] = busy
        # A vehicle on its way to charge, queued or charging is in one state of its own:
        # (-1, bands, hour), its zone and charge set aside.
        charging = (-1, self.bands, hour)
        now = [
            charging if s in _CHARGING else (z, self._band(c), hour)
            for z, c, s in zip(zones, soc, v["state"], strict=True)
        ]
        self._learn(name, now, gains)
        after = list(now)
        out: list[Any] = []
        idle = [k for k, s in enumerate(v["state"]) if s == "idle"]
        targets = sorted(self._zones.nodes.items())
        times = None
        if idle and targets:
            times = view.travel_times(name, [v["node"][k] for k in idle], [n for _, n in targets])
        # A zone is a relocation's candidate only while its requests expected in the next hour
        # outnumber the vehicles idle or heading there (the table's values do not fall as
        # vehicles gather: without this, every vehicle drives to the best zone).
        lack = self._lack(view, name, v) if idle else {}
        for row, k in enumerate(idle):
            # Candidates: stay; charge at the nearest charger; drive to a zone seen.
            opts: list[tuple[float, Any, tuple[int, int, int]]] = [
                (self._value(name, now[k]), None, now[k])
            ]
            charger = v["nearest_charger"][k]
            if charger is not None and soc[k] < self.charge_to:
                key = charging
                opts.append(
                    (
                        self._value(name, key),
                        Charge(name, v["vehicle_id"][k], charger, self.charge_to),
                        key,
                    )
                )
            for col, (z, node) in enumerate(targets):
                if times is None or z == zones[k] or not np.isfinite(times[row, col]):
                    continue
                if lack.get(z, 0.0) < 1.0:
                    continue
                km = times[row, col] * _SPEED_M_S / 1000.0
                key = (z, self._band(soc[k]), hour)
                opts.append(
                    (
                        self._value(name, key) - self.weights["w_km"] * km,
                        Relocate(name, v["vehicle_id"][k], node),
                        key,
                    )
                )
            pick = max(range(len(opts)), key=lambda j: (opts[j][0], -j))
            if self._rng is not None and self._rng.random() < self.explore:
                pick = int(self._rng.integers(len(opts)))
            _, decision, key = opts[pick]
            after[k] = key
            if isinstance(decision, Relocate):
                lack[key[0]] = lack.get(key[0], 0.0) - 1.0
                lack[zones[k]] = lack.get(zones[k], 0.0) + 1.0
            if decision is not None:
                out.append(decision)
        self._last[name] = after
        return out

    def _lack(self, view: Any, name: str, v: Mapping[str, Any]) -> dict[int, float]:
        """Per zone, the requests expected in the next hour less the vehicles idle there."""
        legs = view.expected_legs(name, 900.0)
        first = int((view.t - view.window[0]) // 900.0)
        ahead = legs[:, first : first + 4].sum(axis=1)
        lack = {int(z): float(q) for z, q in zip(view.zones["id"], ahead, strict=True)}
        for z, state in zip(v["zone"], v["state"], strict=True):
            if state in ("idle", "relocating"):
                lack[int(z)] = lack.get(int(z), 0.0) - 1.0
        return lack

    def train(self, environment: Any, episodes: int, seed: int = 0) -> dict:
        """Learn the table over ``episodes`` days of ``environment``, the seeds from ``seed``.

        Approximate dynamic programming with a lookup table: at each step, each dock's (or
        vehicle's) value of the state it was left in at the last step moves, by the learning
        rate, toward its gain since (minus ``w_fail`` × its failed pickups; for a taxi, ``w_fail``
        if it is fetching or carrying a traveller: a proxy for a request served) plus the value
        of its state now. Decisions explore (a random candidate) with the share ``explore``,
        drawn by ``numpy.random.default_rng(seed)``: the same seed gives the same table.
        Episode ``k`` is the environment's day ``seed + k``.

        Returns:
            A copy of the table: ``{(service, zone, band, hour): value}``.
        """
        env = _environment(environment)
        self._rng = np.random.default_rng(int(seed))
        try:
            for k in range(int(episodes)):
                self.day_start({})
                _episode(self, env, int(seed) + k)
        finally:
            self._rng = None
        return dict(self.table)


def policy_value_table(
    *,
    bands: int = 5,
    learning_rate: float = 0.1,
    explore: float = 0.1,
    candidates: Sequence[tuple[float, float]] = ((0.1, 0.9), (0.2, 0.8), (0.3, 0.7)),
    charge_to: float = 0.8,
    step_s: float = 900.0,
    weights: Mapping[str, float] | None = None,
) -> PolicyValueTable:
    """VFA: decisions by the reward now plus a learned value of the state reached.

    The value of a state is a sum over the service's units (docks, or vehicles) of a lookup
    table's entry for the unit's (zone, band, hour): the band of a dock's fill (bikes, with
    those on their way, over its slots) or of a vehicle's charge, in ``bands`` equal bands,
    and the hour of the day; an entry not yet learned is 0. Each step, per docked service, the
    candidates are no moves and the threshold rule's moves for each ``(low, high)`` of
    ``candidates`` (paired nearest first); the one of highest value of the fills reached less
    ``w_km`` × the van kilometres (the seconds driven at 8 m/s, an assumption) is taken. Per
    idle taxi the candidates are staying, charging at the nearest charger to ``charge_to``, and
    driving empty to a zone where a vehicle or request was seen (its node of smallest id).
    ``train(environment, episodes, seed)`` learns the table; before, every value is 0 and the
    policy does nothing (no moves, vehicles stay).

    Args:
        bands: The fill or charge bands.
        learning_rate: The step of each update (a fixed rate).
        explore: The share of decisions taken at random while training.
        candidates: The threshold bands a docked service's candidate moves come from.
        charge_to: The share a taxi sent to charge charges to.
        step_s: The seconds between its steps.
        weights: Reward weights by name in place of ``fleet.REWARD_WEIGHTS``'.
    """
    return PolicyValueTable(
        bands, learning_rate, explore, candidates, charge_to, step_s, weights or {}
    )


# --- DLA: optimisation over a forecast horizon, re-solved each step ----------------------


class PolicyLookahead:
    """DLA: a mixed-integer program over the next ``horizon_s`` (``policy_lookahead``)."""

    def __init__(
        self,
        horizon_s: float,
        bin_s: float,
        neighbours: int,
        van_bikes: int,
        vehicle_requests_bin: float,
        charge_below: float,
        charge_to: float,
        step_s: float,
        weights: Mapping[str, float],
    ) -> None:
        self._opt = _scipy_optimize()
        self.horizon_s, self.bin_s = float(horizon_s), float(bin_s)
        self.neighbours, self.van_bikes = int(neighbours), int(van_bikes)
        self.vehicle_requests_bin = float(vehicle_requests_bin)
        self.charge_below, self.charge_to = float(charge_below), float(charge_to)
        self.step_s = float(step_s)
        self.weights = {**REWARD_WEIGHTS, **dict(weights)}
        self._returns: dict[str, np.ndarray] = {}
        self._zones = _ZoneNodes()
        self.solve_s: list[float] = []

    def describe(self) -> dict[str, Any]:
        """Its name and parameters."""
        return {
            "name": "lookahead",
            "parameters": {
                "horizon_s": self.horizon_s,
                "bin_s": self.bin_s,
                "neighbours": self.neighbours,
                "van_bikes": self.van_bikes,
            },
        }

    def day_start(self, day: Mapping[str, Any]) -> None:
        """Forget the returns counted and the zones' nodes seen (a new day)."""
        self._returns = {}
        self._zones = _ZoneNodes()

    def _bins(self, view: Any, legs: np.ndarray) -> tuple[np.ndarray, int]:
        """The expected legs of the horizon's bins, from the bin the step is in."""
        first = int((view.t - view.window[0]) // self.bin_s)
        count = max(1, math.ceil(self.horizon_s / self.bin_s))
        out = np.zeros((legs.shape[0], count))
        part = legs[:, first : first + count]
        out[:, : part.shape[1]] = part
        return out, count

    def step(self, view: Any) -> list[Any]:
        """The first bin's decisions of the horizon's program, per service."""
        out: list[Any] = []
        for name in _services(view, "bike_docked"):
            out += self._bikes(view, name)
        for name in _services(view, "taxi"):
            out += self._taxis(view, name)
        return out

    def _bikes(self, view: Any, name: str) -> list[Any]:
        d = view[name]
        n = len(d["dock_id"])
        returns = self._returns.setdefault(name, np.zeros(n))
        returns += np.asarray(d["returns"], dtype=float)
        elapsed = max(view.t - view.window[0], self.bin_s)
        # Returns forecast at the day's rate so far per dock (expected_legs counts pickups).
        r = returns * self.bin_s / elapsed
        p, nb = self._bins(view, view.expected_legs(name, self.bin_s))
        b0 = np.asarray(d["bikes"], dtype=float) + np.asarray(d["incoming"], dtype=float)
        cap = np.asarray(d["slots"], dtype=float)
        nodes = list(d["node"])
        tt = view.travel_times(name, nodes, nodes)
        k = min(self.neighbours, n - 1)
        if k <= 0:
            return []
        pairs = []
        for i in range(n):
            order = [
                j for j in np.argsort(tt[i], kind="stable") if j != i and np.isfinite(tt[i, j])
            ]
            pairs += [(i, int(j)) for j in order[:k]]
        moves = self._solve_bikes(n, nb, pairs, tt, b0, cap, p, r)
        return [Move(name, d["dock_id"][i], d["dock_id"][j], m) for (i, j), m in moves]

    def _solve_bikes(self, n, nb, pairs, tt, b0, cap, p, r):
        """Moves x_pt (bikes) and van trips y_pt (0 to 1) per pair p and bin t, whole in bin 0.

        Stock b_it = b_i(t-1) + r_it - p_it + inflow - outflow + u_it - o_it in [0, slots]:
        u_it the pickups that fail, o_it the returns that find the dock full. Minimise
        w_fail sum(u) + w_walk·5 min sum(o) + w_km sum(km_p y_pt) + 1e-3 sum(x).
        """
        opt = self._opt
        from scipy.sparse import lil_matrix

        npair = len(pairs)
        nx = npair * nb
        ix = lambda q, t: q * nb + t  # noqa: E731
        iy = lambda q, t: nx + q * nb + t  # noqa: E731
        base = 2 * nx
        ib = lambda i, t: base + i * nb + t  # noqa: E731
        iu = lambda i, t: base + n * nb + i * nb + t  # noqa: E731
        io = lambda i, t: base + 2 * n * nb + i * nb + t  # noqa: E731
        nv = base + 3 * n * nb
        c = np.zeros(nv)
        lo, hi = np.zeros(nv), np.full(nv, np.inf)
        integ = np.zeros(nv)
        w = self.weights
        for q, (i, j) in enumerate(pairs):
            km = tt[i, j] * _SPEED_M_S / 1000.0
            for t in range(nb):
                c[ix(q, t)] = 1e-3
                c[iy(q, t)] = w["w_km"] * km + 1e-3
                hi[iy(q, t)] = 1.0
                # Whole bikes and vans now; later bins relaxed (they are re-solved then).
                if t == 0:
                    integ[ix(q, t)] = integ[iy(q, t)] = 1
        for i in range(n):
            for t in range(nb):
                c[iu(i, t)] = w["w_fail"]
                c[io(i, t)] = w["w_walk"] * 5.0
                hi[ib(i, t)] = cap[i]
                # Failures and overflows only of the people expected (no bikes made or lost).
                hi[iu(i, t)] = p[i, t]
                hi[io(i, t)] = r[i]
        rows = n * nb + nx
        a = lil_matrix((rows, nv))
        lb, ub = np.zeros(rows), np.zeros(rows)
        row = 0
        for i in range(n):
            for t in range(nb):
                a[row, ib(i, t)] = 1.0
                if t > 0:
                    a[row, ib(i, t - 1)] = -1.0
                a[row, iu(i, t)] = -1.0
                a[row, io(i, t)] = 1.0
                rhs = r[i] - p[i, t] + (b0[i] if t == 0 else 0.0)
                for q, (s, e) in enumerate(pairs):
                    if s == i:
                        a[row, ix(q, t)] = 1.0
                    if e == i:
                        a[row, ix(q, t)] = -1.0
                lb[row] = ub[row] = rhs
                row += 1
        for q in range(npair):
            for t in range(nb):
                a[row, ix(q, t)] = 1.0
                a[row, iy(q, t)] = -float(self.van_bikes)
                lb[row], ub[row] = -np.inf, 0.0
                row += 1
        started = time.perf_counter()
        res = opt.milp(
            c,
            constraints=opt.LinearConstraint(a.tocsr(), lb, ub),
            integrality=integ,
            bounds=opt.Bounds(lo, hi),
            options={"time_limit": 10.0, "mip_rel_gap": 0.01},
        )
        self.solve_s.append(time.perf_counter() - started)
        if res.x is None:
            return []
        out = []
        for q, pair in enumerate(pairs):
            m = round(res.x[ix(q, 0)])
            if m > 0:
                out.append((pair, m))
        return out

    def _taxis(self, view: Any, name: str) -> list[Any]:
        v = view[name]
        self._zones.see_view(v)
        zones_seen = sorted(self._zones.nodes.items())
        if not zones_seen:
            return []
        zone_ids = [z for z, _ in zones_seen]
        legs = view.expected_legs(name, self.bin_s)
        q_all, nb = self._bins(view, legs)
        rows_by_zone = {int(z): k for k, z in enumerate(view.zones["id"])}
        q = np.array(
            [q_all[rows_by_zone[z]] if z in rows_by_zone else np.zeros(nb) for z in zone_ids]
        )
        state, soc = list(v["state"]), np.asarray(v["soc"], dtype=float)
        idle = [k for k, s in enumerate(state) if s == "idle"]
        if not idle:
            return []
        tt = view.travel_times(name, [v["node"][k] for k in idle], [n for _, n in zones_seen])
        # Vehicles busy now serve from the next bin in their zone (an approximation).
        busy = np.zeros(len(zone_ids))
        zpos = {z: k for k, z in enumerate(zone_ids)}
        for k, s in enumerate(state):
            if s in ("to_pickup", "occupied", "relocating") and int(v["zone"][k]) in zpos:
                busy[zpos[int(v["zone"][k])]] += 1
        decisions = self._solve_taxis(v, name, idle, soc, tt, q, busy, zone_ids, nb)
        return decisions

    def _solve_taxis(self, v, name, idle, soc, tt, q, busy, zone_ids, nb):
        """Each idle vehicle stays, drives to a zone, or charges: a 0/1 choice; served s_zt.

        Maximise w_fail sum(s) - w_km sum(km a) - w_fail·(below charge_below and not charging)
        with s_zt <= q_zt and s_zt <= cap (the vehicles there by the bin's end, each
        ``vehicle_requests_bin`` requests a bin).
        """
        opt = self._opt
        nveh, nz = len(idle), len(zone_ids)
        w = self.weights
        ia = lambda k, z: k * nz + z  # noqa: E731
        ich = lambda k: nveh * nz + k  # noqa: E731
        isv = lambda z, t: nveh * nz + nveh + z * nb + t  # noqa: E731
        nv = nveh * nz + nveh + nz * nb
        c = np.zeros(nv)
        hi = np.ones(nv)
        integ = np.zeros(nv)
        integ[: nveh * nz + nveh] = 1
        for k, veh in enumerate(idle):
            low = soc[veh] < self.charge_below
            has_charger = v["nearest_charger"][veh] is not None
            # Not charging while low costs a failed request's weight.
            c[ich(k)] = -w["w_fail"] if low else w["w_fail"]
            if not has_charger:
                hi[ich(k)] = 0.0
            for z in range(nz):
                s = tt[k, z]
                if not np.isfinite(s):
                    hi[ia(k, z)] = 0.0
                    continue
                c[ia(k, z)] = w["w_km"] * s * _SPEED_M_S / 1000.0
        for z in range(nz):
            for t in range(nb):
                c[isv(z, t)] = -w["w_fail"]
                hi[isv(z, t)] = q[z, t]
        rows: list[np.ndarray] = []
        lb: list[float] = []
        ub: list[float] = []
        for k in range(nveh):
            r = np.zeros(nv)
            r[[ia(k, z) for z in range(nz)]] = 1.0
            r[ich(k)] = 1.0
            rows.append(r)
            lb.append(1.0)
            ub.append(1.0)
        for z in range(nz):
            for t in range(nb):
                r = np.zeros(nv)
                r[isv(z, t)] = 1.0
                for k in range(nveh):
                    if np.isfinite(tt[k, z]) and tt[k, z] <= (t + 1) * self.bin_s:
                        r[ia(k, z)] = -self.vehicle_requests_bin
                rows.append(r)
                lb.append(-np.inf)
                ub.append(self.vehicle_requests_bin * (busy[z] if t > 0 else 0.0))
        started = time.perf_counter()
        res = opt.milp(
            c,
            constraints=opt.LinearConstraint(np.array(rows), lb, ub),
            integrality=integ,
            bounds=opt.Bounds(np.zeros(nv), hi),
            options={"time_limit": 10.0, "mip_rel_gap": 0.01},
        )
        self.solve_s.append(time.perf_counter() - started)
        if res.x is None:
            return []
        out: list[Any] = []
        for k, veh in enumerate(idle):
            vid = v["vehicle_id"][veh]
            if res.x[ich(k)] > 0.5:
                out.append(Charge(name, vid, v["nearest_charger"][veh], self.charge_to))
                continue
            for z in range(nz):
                if res.x[ia(k, z)] > 0.5 and zone_ids[z] != int(v["zone"][veh]):
                    out.append(Relocate(name, vid, self._zones.nodes[zone_ids[z]]))
        return out


def policy_lookahead(
    *,
    horizon_s: float = 7200.0,
    bin_s: float = 900.0,
    neighbours: int = 5,
    van_bikes: int = 20,
    vehicle_requests_bin: float = 2.0,
    charge_below: float = 0.2,
    charge_to: float = 0.8,
    step_s: float = 900.0,
    weights: Mapping[str, float] | None = None,
) -> PolicyLookahead:
    """DLA: a mixed-integer program over the next ``horizon_s``, re-solved every step.

    **Docked bikes.** Over the horizon's bins of ``bin_s``, each dock's stock follows the
    pickups expected (``view.expected_legs``), the returns forecast at the rate the dock has
    seen so far in the day, and the bikes moved; the stock stays within ``[0, slots]``, the
    pickups that find it empty fail and the returns that find it full go elsewhere. Moves run
    between each dock and its ``neighbours`` nearest by road (``view.travel_times``), whole
    bikes, at most ``van_bikes`` a van trip. The program minimises ``w_fail`` × the failed
    pickups, ``w_walk`` × 5 minutes per return gone elsewhere (an assumed walk), and ``w_km``
    × the van kilometres (seconds driven at 8 m/s, an assumption); the first bin's moves are
    applied. Moves arrive within their bin (the drive's time is left out).

    **Taxis.** Each idle vehicle stays, drives empty to a zone where a vehicle or request was
    seen, or charges (when below ``charge_below`` not charging costs ``w_fail``); per zone and
    bin the requests served are at most those expected (``view.expected_legs``) and
    ``vehicle_requests_bin`` per vehicle there by the bin's end (busy vehicles count from the
    second bin, in their zone). The program maximises ``w_fail`` × the requests served less
    the kilometres driven.

    Both solved by ``scipy.optimize.milp`` (HiGHS, at most 10 s a solve); the seconds of each
    solve are in ``solve_s``. In a day without variation the expected legs are the day's own
    (``operator``): the program then knows the future.

    Args:
        horizon_s: The seconds looked ahead (7 200).
        bin_s: The program's bins, in seconds.
        neighbours: The docks each dock may send bikes to.
        van_bikes: The bikes a van carries (the service's option of that name, 20 by default).
        vehicle_requests_bin: The requests a taxi serves per bin, in the program.
        charge_below: Below this charge a taxi not sent to charge costs a failed request.
        charge_to: The share a taxi sent to charge charges to.
        step_s: The seconds between its steps.
        weights: Reward weights by name in place of ``fleet.REWARD_WEIGHTS``'.

    Raises:
        ImportError: Without SciPy (the ``policies`` extra).
    """
    return PolicyLookahead(
        horizon_s,
        bin_s,
        neighbours,
        van_bikes,
        vehicle_requests_bin,
        charge_below,
        charge_to,
        step_s,
        weights or {},
    )


class PolicyLookaheadSimulated:
    """DLA by simulation: candidates simulated ahead on a fork (``policy_lookahead_simulated``)."""

    def __init__(self, environment: Any, candidates: Sequence[Any], horizon_s: float) -> None:
        if not candidates:
            raise ValueError("policy_lookahead_simulated needs at least one candidate policy")
        self.environment = environment
        self.candidates = [_python_policy(c) for c in candidates]
        self.horizon_s = float(horizon_s)
        self.step_s = float(getattr(self.candidates[0], "step_s", 900.0))
        self.chosen: list[int] = []

    def describe(self) -> dict[str, Any]:
        """Its name and parameters."""
        return {
            "name": "lookahead_simulated",
            "parameters": {"horizon_s": self.horizon_s, "candidates": len(self.candidates)},
        }

    def day_start(self, day: Mapping[str, Any]) -> None:
        """Forget the choices made (a new day)."""
        self.chosen = []

    def step(self, view: Any) -> list[Any]:
        """The decisions now of the candidate whose simulated horizon earns the most."""
        firsts = [c.step(view) or [] for c in self.candidates]
        scores = []
        for cand, first in zip(self.candidates, firsts, strict=True):
            ahead = self.environment.fork()
            start = ahead.t
            v, total, done = None, 0.0, False
            decisions = first
            while not done:
                v, reward, terminated, truncated, _ = ahead.step(decisions)
                total += float(reward)
                done = bool(terminated or truncated) or ahead.t - start >= self.horizon_s
                if not done:
                    decisions = cand.step(v) or []
            scores.append(total)
        best = max(range(len(scores)), key=lambda k: (scores[k], -k))
        self.chosen.append(best)
        return firsts[best]


def policy_lookahead_simulated(
    environment: Any, candidates: Sequence[Any], *, horizon_s: float = 7200.0
) -> PolicyLookaheadSimulated:
    """DLA by simulation: at each step, each candidate policy is run ahead on a copy of the day.

    At each step every candidate decides on the view; each candidate's decisions are applied
    on ``environment.fork()`` (a copy of the day at its clock) and the candidate then decides
    alone over the next ``horizon_s``; the candidate whose copy earns the highest reward
    (the first among equals) has its decisions of now applied. The index chosen per step is in
    ``chosen``. The policy steps ``environment`` itself (it forks it), so it runs with
    ``evaluate(policy, environment, seeds)`` on that same environment, not in ``fleet_day``.

    **The copy simulates the day being stepped, with its seed's own legs: the lookahead knows
    the day's future**, as a perfect forecast. It is a bound on what the candidates can earn
    by switching between them, not a policy that could run without that knowledge.

    Args:
        environment: The ``fleet_environment`` (``n=1``) the policy is stepped on.
        candidates: Policy objects or built-in ``noop``, ``threshold``, ``charge_rule``.
        horizon_s: The seconds simulated ahead (7 200).
    """
    return PolicyLookaheadSimulated(environment, candidates, horizon_s)
