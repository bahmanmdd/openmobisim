"""A Python choice model registered by name (C5): chosen like a built-in one."""

from __future__ import annotations

import openmobisim as ms
import pytest
from openmobisim import choice


class _Timid:
    """A logit on time alone, its weight from the options."""

    def __init__(self, options: dict[str, float]) -> None:
        self.beta = float(options.get("beta_time_min", -0.2))
        self.descriptor = f"timid;beta_time_min={self.beta}"

    def choose(self, batch):
        return choice.sample_random_utility(batch, self.beta * batch.attributes["time_min"])


@pytest.fixture
def timid(monkeypatch: pytest.MonkeyPatch) -> str:
    monkeypatch.setattr(choice, "_REGISTERED", {})
    ms.choice_model_register("timid", _Timid)
    return "timid"


def test_a_registered_model_runs_by_name_with_its_options(timid: str, tmp_path) -> None:
    assert ms.choice_models()[-1] == timid
    net = ms.examples.toy_network()
    rows = ms.examples.trips_random(net, 40, seed=3, min_m=100.0, max_m=2000.0, spread_s=1800)
    base = ms.Scenario.from_parts(net, rows, choice_model=timid, route_method="penalty")
    assert base.parts["choice_model"] == timid, "the scenario keeps the name"
    steeper = base.replace(choice_options={"beta_time_min": -0.5})
    assert steeper._fingerprint() != base._fingerprint(), "the options reach the fingerprint"
    run = base.run("timid", output_dir=str(tmp_path / "timid"), quiet=True)
    assert run.manifest()["choice_model"] == timid
    assert run.manifest()["choice_descriptor"] == "timid;beta_time_min=-0.2"


def test_a_built_in_or_odd_name_is_refused(timid: str) -> None:
    with pytest.raises(ValueError, match="built-in"):
        ms.choice_model_register("logit", _Timid)
    with pytest.raises(ValueError, match="identifier|letters"):
        ms.choice_model_register("my model", _Timid)
    ms.choice_model_register("broken", lambda options: object())
    net = ms.examples.toy_network()
    with pytest.raises(ValueError, match="choose"):
        ms.Scenario.from_parts(net, [], choice_model="broken")
