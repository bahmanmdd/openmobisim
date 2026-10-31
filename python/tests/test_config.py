"""The run configuration file, ``run.toml``: every parameter in one file, read back the same.

What is defended: the defaults file lists every parameter of the registry (``parameters()``) a run
takes, each with its default and a meaning, and reads back to the defaults (the same
fingerprint); a changed value is written, read back and named in the header.
"""

from __future__ import annotations

import math
import tomllib
from pathlib import Path

import openmobisim as ms
import pytest
from openmobisim import config as cfg


def _toy(**kwargs: object) -> ms.Scenario:
    """A multimodal scenario of the toy network: classes, transit, parkings, mode choice."""
    net = ms.examples.toy_network()
    rows = ms.examples.trips_random(net, 60, seed=3, min_m=100.0, max_m=2000.0, spread_s=1800)
    classes = ms.examples.traveller_classes()
    rows = ms.demand_assign_classes(rows, classes, seed=1)
    return ms.Scenario.from_parts(
        net,
        rows,
        classes=classes,
        transit=ms.examples.toy_network_transit(),
        parkings=ms.examples.toy_network_parkings(),
        **kwargs,
    )


def test_the_defaults_file_lists_every_parameter_with_its_default_and_meaning(tmp_path) -> None:
    path = ms.config_write_defaults(tmp_path / "run.toml")
    text = path.read_text()
    data = tomllib.loads(text)
    t = ms.parameters()
    missing, meaningless = [], []
    lines = cfg._lines()
    for group, name in zip(t["group"], t["name"], strict=True):
        prefix = group.split(".")[0]
        if prefix in cfg.NOT_IN_A_RUN:
            continue
        table = cfg._SCENARIO_TABLES.get(name) if prefix == "scenario" else cfg.GROUP_TABLES[prefix]
        key = cfg._BIKE + name if prefix == "bike_route" else name
        found = [x for x in lines[table] if x.key == key]
        if not found:
            missing.append(f"{group}.{name}")
        elif not all(x.meaning for x in found):
            meaningless.append(f"{group}.{name}")
    assert not missing, f"registry parameters the file has no line for: {missing}"
    assert not meaningless, f"parameters without a meaning in the file's comments: {meaningless}"
    # Every table is in the file, every parameter line (the inputs aside) says its default.
    assert set(data) == set(cfg.TABLES) - {"services"}
    table = None
    for line in text.splitlines():
        if line.startswith("["):
            table = line.split("]")[0][1:]
        elif " = " in line and table not in (None, "inputs") and not line.startswith("#   "):
            assert "# default" in line, line
    assert "beta_time_min = -0.2" in text and "default -0.2 per min: weighs" in text
    assert "motorway.lanes = 2" in text and data["network"]["motorway"]["lanes"] == 2


def test_the_defaults_file_reads_back_to_the_defaults(tmp_path) -> None:
    path = ms.config_write_defaults(tmp_path / "run.toml")
    read = ms.config_read(path)
    assert read.differences() == []
    stated, run = read._arguments()
    stated.pop("network_options")
    # Every option is at its default, so none reaches the run: the same run, bit for bit.
    arguments = cfg._scenario_arguments(stated, {})
    assert all(v is None for k, v in arguments.items() if k.endswith("_options"))
    assert _toy(**arguments)._fingerprint() == _toy()._fingerprint()
    assert cfg._run_arguments(run) == {
        "hub_join_m": 150.0,
        "outputs": "all",
        "design_id": "default",
        "replication": 0,
        "service_assumptions": None,
    }
    # Written again, it is the same file.
    assert read.text() == ms.Config().text()


def test_a_changed_value_is_written_read_back_and_named_in_the_header(tmp_path) -> None:
    taxis = ms.vehicle_read_table([{"vehicle_id": "v", "lon": 4.9, "lat": 52.37}])
    config = ms.Config.from_arguments(
        "delft-base",
        flow_level=4,
        window=("07:30", "09:30"),
        choice_options={"beta_time_min": -0.1},
        loading_options={"reroute": 1},
        modes=["car", "taxi"],
        services=[ms.service_taxi(taxis, options={"request_cancel_s": 900})],
        service_assumptions={"assumed_wait_s": 300},
        policy=ms.operator.policy_charge_rule(0.3),
    )
    text = config.write(tmp_path / "run.toml").read_text()
    header = text.splitlines()[1]
    assert header == (
        "# Differs from the defaults: run.flow_level, run.window, choice.beta_time_min, "
        "modes.modes, services.taxi, service_assumptions.assumed_wait_s, policy, loading.reroute."
    )
    assert 'name = "delft-base"' in text and "beta_time_min = -0.1" in text
    assert "[services.taxi]" in text and "request_cancel_s = 900" in text
    read = ms.config_read(tmp_path / "run.toml")
    assert read.differences() == config.differences()
    assert read.name == "delft-base"
    assert read.values["run"]["window"] == ("07:30", "09:30")
    assert read.values["choice"]["beta_time_min"] == -0.1
    assert read.values["services"]["taxi"]["request_cancel_s"] == 900
    assert read.values["policy"] == {"name": "charge_rule", "charge_below": 0.3,
                                     "charge_to": 0.8, "step_s": 900.0}  # fmt: skip
    assert read.text() == text


def test_the_writer_writes_every_kind_of_value_as_toml() -> None:
    values = {
        "text": 'a "quoted" name\\with a backslash',
        "whole": 300.0,
        "fraction": 0.18,
        "small": 1e-9,
        "large": 1e300,
        "infinite": math.inf,
        "list": ["car", "bike"],
        "times": ["07:30", 27000],
        "flag": True,
    }
    text = "\n".join(f"{cfg._key(k)} = {cfg._value(v)}" for k, v in values.items())
    text += '\n"a key with spaces" = 1\nmotorway.lanes = 3\n'
    data = tomllib.loads(text)
    assert data["text"] == values["text"] and data["whole"] == 300
    assert data["fraction"] == 0.18 and data["small"] == 1e-9 and data["large"] == 1e300
    assert data["infinite"] == math.inf and data["list"] == ["car", "bike"]
    assert data["times"] == ["07:30", 27000] and data["flag"] is True
    assert cfg._key("a key with spaces") == '"a key with spaces"'
    assert cfg._flat("network", data)["motorway.lanes"] == 3


def test_a_name_not_in_its_table_is_refused_with_the_table_names(tmp_path) -> None:
    path = Path(tmp_path / "run.toml")
    path.write_text("[choice]\nbeta_tme_min = -0.1\n")
    with pytest.raises(ValueError, match=r"\[choice\]: no parameter 'beta_tme_min'.*beta_time_min"):
        ms.config_read(path)
    path.write_text("[runn]\nflow_level = 2\n")
    with pytest.raises(ValueError, match=r"no table or name \['runn'\]"):
        ms.config_read(path)
    path.write_text('[equilibration]\nmethod = "frank_wolfe"\ncost_bin_s = 300\n')
    with pytest.raises(ValueError, match=r"no 'cost_bin_s' for method = 'frank_wolfe'"):
        ms.config_read(path)
    path.write_text('[run]\nflow_level = "two"\n')
    with pytest.raises(ValueError, match=r"\[run\] flow_level: a number"):
        ms.config_read(path)
    path.write_text("[run]\nflow_level = 2.5\n")
    with pytest.raises(ValueError, match="a whole number"):
        ms.config_read(path)
    path.write_text('[services.taxi]\nkind = "taxi"\nrequest_cancel = 600\n')
    with pytest.raises(ValueError, match=r"no 'request_cancel' for kind 'taxi'.*request_cancel_s"):
        ms.config_read(path)


def test_writing_the_file_costs_milliseconds(tmp_path) -> None:
    import time

    started = time.perf_counter()
    for i in range(20):
        ms.config_write_defaults(tmp_path / f"run{i}.toml")
    # A loose floor (each write is about a millisecond): a run's copy must cost nothing beside it.
    assert (time.perf_counter() - started) / 20 < 0.05
