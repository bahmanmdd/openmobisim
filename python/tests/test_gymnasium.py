"""The Gymnasium adapter: skipped without ``gymnasium`` (not a dependency of openmobisim)."""

from __future__ import annotations

import openmobisim as ms
import pytest
from test_fleet import docks, grid, trip


def test_the_adapter_passes_gymnasiums_checker_and_steps_a_day():
    pytest.importorskip("gymnasium")
    from gymnasium.utils.env_checker import check_env
    from openmobisim.contrib.gymnasium import FleetGymEnv

    net = grid()
    bikes = ms.service_bike_docked(docks(net, bikes=(3, 0)))
    trips = [trip(net, f"t{k}", 27_000 + 600 * k, (0, 0), (4, 4)) for k in range(6)]
    env = FleetGymEnv(ms.fleet_environment(net, [bikes], trips), {"service": "bike_share"})
    assert env.action_space.nvec.tolist() == [6, 6], "levels -1 to 4, counted from 0"
    check_env(env, skip_render_check=True)
    obs, _ = env.reset(seed=1)
    assert env.observation_space.contains(obs)
    total, done = 0.0, False
    while not done:
        obs, reward, done, truncated, info = env.step(env.action_space.sample())
        total += reward
        assert not truncated and env.observation_space.contains(obs)
    assert total <= 0.0 and info["fidelity"] == "frozen_day"
    flat = FleetGymEnv(
        ms.fleet_environment(net, [bikes], trips), {"service": "bike_share", "flat": True}
    )
    obs, _ = flat.reset(seed=0)
    assert obs.shape == (13,) and flat.observation_space.contains(obs)


def test_without_gymnasium_the_adapter_says_what_to_install():
    try:
        import gymnasium  # noqa: F401
    except ImportError:
        with pytest.raises(ImportError, match="gymnasium"):
            import openmobisim.contrib.gymnasium  # noqa: F401
    else:
        pytest.skip("gymnasium is installed")
