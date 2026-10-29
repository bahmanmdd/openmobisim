"""A fleet environment as a Gymnasium environment.

``gymnasium`` is needed, and not installed with openmobisim: ``pip install gymnasium``.

``FleetGymEnv(environment, spec)`` wraps a ``fleet_environment`` (``n=1``) for one docked
service: the observation is ``environment.observation(view, spec)`` and the action one target
level per dock, as Gymnasium's ``MultiDiscrete`` counts from 0: entry ``a`` asks for ``a - 1``
bikes, 0 setting no target. ``reset(seed=k)`` starts the day of seed ``k``; without a seed, the
day after the last one.
"""

from __future__ import annotations

from collections.abc import Mapping
from typing import Any

import numpy as np

try:
    import gymnasium
    from gymnasium import spaces
except ImportError as e:  # pragma: no cover - depends on the environment
    raise ImportError(
        "openmobisim.contrib.gymnasium needs gymnasium, which openmobisim does not install: "
        "pip install gymnasium"
    ) from e

__all__ = ["FleetGymEnv"]


class FleetGymEnv(gymnasium.Env):
    """One docked service of a fleet day, stepped by Gymnasium's interface."""

    metadata: dict[str, Any] = {"render_modes": []}  # noqa: RUF012 (Gymnasium's attribute)

    def __init__(self, environment: Any, spec: Mapping[str, Any]) -> None:
        """Wrap ``environment`` (a ``FleetEnvironment``) for the service ``spec["service"]``."""
        self.environment = environment
        self.spec_encoding = dict(spec)
        layout = environment.layout()[spec["service"]]
        if layout["kind"] != "bike_docked":
            raise ValueError("FleetGymEnv steers a docked service; a taxi fleet takes decisions")
        slots = np.asarray(layout["slots"], dtype=np.int64)
        docks = spaces.Box(0.0, np.inf, (len(slots), 5), np.float32)
        clock = spaces.Box(-1.0, 1.0, (3,), np.float32)
        if spec.get("flat", False):
            low = np.concatenate([np.zeros(docks.shape[0] * 5), clock.low]).astype(np.float32)
            high = np.concatenate([np.full(docks.shape[0] * 5, np.inf), clock.high])
            self.observation_space = spaces.Box(low, high.astype(np.float32), dtype=np.float32)
        else:
            self.observation_space = spaces.Dict({"docks": docks, "time": clock})
        self.action_space = spaces.MultiDiscrete(slots + 2)
        self._seed = -1

    def reset(
        self, *, seed: int | None = None, options: dict[str, Any] | None = None
    ) -> tuple[Any, dict[str, Any]]:
        """Start the day of ``seed`` (or the next one): ``(observation, info)``."""
        super().reset(seed=seed)
        self._seed = seed if seed is not None else self._seed + 1
        view, info = self.environment.reset(self._seed)
        return self.environment.observation(view, self.spec_encoding), info

    def step(self, action: Any) -> tuple[Any, float, bool, bool, dict[str, Any]]:
        """Set the docks' target levels (``action - 1``) and run one step."""
        levels = np.asarray(action, dtype=np.int64) - 1
        decisions = self.environment.action(levels, self.spec_encoding)
        view, reward, terminated, truncated, info = self.environment.step(decisions)
        obs = self.environment.observation(view, self.spec_encoding)
        return obs, float(reward), bool(terminated), bool(truncated), info
