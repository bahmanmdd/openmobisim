"""The run configuration file, ``run.toml``: every parameter in one file, read back the same.

What is defended: the defaults file lists every parameter of the registry (``parameters()``) a run
takes, each with its default and a meaning, and reads back to the defaults (the same
fingerprint); a changed value is written, read back and named in the header.
"""

from __future__ import annotations

import json
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
    # The same file again, but for the header's note on the taxis' table, given in memory.
    note = "# Not in this file (given in Python; a run from this file alone lacks them): "
    assert text.splitlines()[3] == note + "services.taxi.vehicles (in memory)."
    assert read.text().splitlines() == text.splitlines()[:3] + text.splitlines()[4:]


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


# --- (c) the reader and the run ------------------------------------------------------------

#: A run's arguments, from both calls: the scenario's and the run's.
SETTINGS = {
    "flow_level": 2,
    "choice_options": {"beta_time_min": -0.15, "beta_wait_min": -0.1},
    "equilibration_options": {"iterations": 3},
    "loading_options": {"reroute_after_s": 240},
    "mode_options": {"car_egress_s": 200},
    "price_options": {"car_eur_km": 0.2},
    "modes": ["car", "bike", "walk", "transit"],
}


def _same_results(a: Path, b: Path) -> None:
    for name in ("kpis.parquet", "trip_results.parquet", "events.parquet"):
        assert (a / name).read_bytes() == (b / name).read_bytes(), name


def test_a_run_from_a_written_file_is_the_run_from_the_arguments(tmp_path) -> None:
    direct = _toy(**SETTINGS).run("r", output_dir=str(tmp_path / "direct"), quiet=True,
                                  service_assumptions=None, design_id="d")  # fmt: skip
    path = ms.Config.from_arguments("r", design_id="d", **SETTINGS).write(tmp_path / "run.toml")
    filed = _toy().run(output_dir=str(tmp_path / "filed"), quiet=True, config=path)
    assert filed.fingerprint == direct.fingerprint
    assert filed.run_id == "r"
    _same_results(tmp_path / "direct", tmp_path / "filed")
    # The scenario's own values stay where the file says nothing; the file's replace them.
    partial = tmp_path / "partial.toml"
    partial.write_text("[equilibration]\niterations = 3\n")
    built = _toy(**{k: v for k, v in SETTINGS.items() if k != "equilibration_options"})
    again = built.run("r", output_dir=str(tmp_path / "partial"), quiet=True, config=partial,
                      design_id="d")  # fmt: skip
    assert again.fingerprint == direct.fingerprint


def test_a_name_given_in_the_file_and_the_call_is_refused(tmp_path) -> None:
    path = ms.Config.from_arguments("r", design_id="a").write(tmp_path / "run.toml")
    with pytest.raises(ValueError, match=r"\['design_id'\] given both in the call and in the file"):
        _toy().run(config=path, design_id="b", quiet=True)
    with pytest.raises(ValueError, match=r"run_id \(the file's name\)"):
        _toy().run("other", config=path, quiet=True)


def test_a_file_whose_inputs_are_not_the_scenario_s_is_refused(tmp_path) -> None:
    (tmp_path / "run.toml").write_text('[inputs]\ndemand = "trips.csv"\n')
    with pytest.raises(ValueError, match=r"\[inputs\] \['demand'\] are not this scenario's"):
        _toy().run(config=tmp_path / "run.toml", quiet=True)
    (tmp_path / "run.toml").write_text("[network]\nwalk_km_h = 5.5\n")
    with pytest.raises(ValueError, match=r"\[network\] is applied when the network is read"):
        _toy().run(config=tmp_path / "run.toml", quiet=True)
    (tmp_path / "run.toml").write_text("[routes]\nmax_paths = 4\n")
    with pytest.raises(ValueError, match=r"\[routes\]: options without a method"):
        _toy().run(config=tmp_path / "run.toml", quiet=True)
    (tmp_path / "run.toml").write_text('[services.taxi]\nkind = "taxi"\n')
    with pytest.raises(ValueError, match=r"this scenario has no service 'taxi'"):
        _toy().run(config=tmp_path / "run.toml", quiet=True)


def _files(folder: Path) -> Path:
    """The toy scenario's inputs as files: a GMNS network and a trips.csv."""
    net = ms.examples.toy_network()
    ms.network_write_gmns(net, str(folder / "network"))
    rows = ms.examples.trips_random(net, 40, seed=5, min_m=100.0, max_m=2000.0, spread_s=1200)
    ms.demand_write_trips(rows, folder / "trips.csv")
    return folder


def test_the_inputs_are_read_relative_to_the_file(tmp_path, monkeypatch) -> None:
    _files(tmp_path)
    (tmp_path / "run.toml").write_text(
        'name = "files"\n[inputs]\nnetwork = "network"\ndemand = "trips.csv"\n'
        "[network]\nwalk_km_h = 5.0\n[equilibration]\niterations = 2\n"
    )
    monkeypatch.chdir(tmp_path.parent)
    run = ms.config_read(tmp_path / "run.toml").run(str(tmp_path / "out"), quiet=True)
    net = ms.network_read_gmns(str(tmp_path / "network"), network_options={"walk_km_h": 5.0})
    direct = ms.Scenario.from_parts(
        net, str(tmp_path / "trips.csv"), equilibration_options={"iterations": 2}
    ).run("files", output_dir=str(tmp_path / "direct"), quiet=True)
    assert run.fingerprint == direct.fingerprint


# --- (d) the command line ------------------------------------------------------------------


def test_the_command_line_runs_a_file_as_python_does(tmp_path, capsys) -> None:
    from openmobisim.__main__ import main

    _files(tmp_path)
    (tmp_path / "run.toml").write_text(
        'name = "cli"\n[inputs]\nnetwork = "network"\ndemand = "trips.csv"\n'
        "[equilibration]\niterations = 2\n[choice]\nbeta_time_min = -0.25\n"
    )
    out = tmp_path / "cli"
    assert main(["run", "--config", str(tmp_path / "run.toml"), "--output", str(out),
                 "--no-pages"]) == 0  # fmt: skip
    assert "cli: " in capsys.readouterr().out
    python = ms.config_read(tmp_path / "run.toml").run(str(tmp_path / "python"), quiet=True)
    manifest = json.loads((out / "manifest.json").read_text())
    assert manifest["run_fingerprint"] == python.fingerprint
    # The defaults file, printed.
    assert main(["config", "--defaults"]) == 0
    assert capsys.readouterr().out == ms.Config().text()
    # A case and a file at once, or neither, is refused.
    assert main(["run", "delft", "--config", str(tmp_path / "run.toml")]) == 2
    assert main(["run"]) == 2
    assert main(["config", "--from", str(tmp_path / "network")]) == 1


# --- (e) the run folder's copy -------------------------------------------------------------


def test_every_run_writes_the_file_it_ran_with_and_it_repeats_the_run(tmp_path) -> None:
    first = _toy(**SETTINGS).run("copy", output_dir=str(tmp_path / "first"), quiet=True)
    text = (tmp_path / "first" / "run.toml").read_text()
    assert 'name = "copy"' in text and "beta_time_min = -0.15" in text
    assert "network (in memory)" in text.splitlines()[3]  # the inputs were objects
    assert "this run: " in text  # what the rules chose, in the comments
    copy = first.config()
    assert copy.values["equilibration"]["iterations"] == 3
    # On a scenario of the same inputs at its defaults, the copy is the whole run.
    again = _toy().run(output_dir=str(tmp_path / "again"), quiet=True, config=copy)
    assert again.fingerprint == first.fingerprint
    _same_results(tmp_path / "first", tmp_path / "again")
    assert (tmp_path / "again" / "run.toml").read_text() == text
    try:
        import pyarrow  # noqa: F401  (Run.open reads the results with it)
    except ImportError:
        return
    assert ms.Run.open(tmp_path / "first").config().values == copy.values


def test_a_run_from_files_repeats_from_its_folder_alone(tmp_path, capsys) -> None:
    from openmobisim.__main__ import main

    _files(tmp_path)
    (tmp_path / "run.toml").write_text(
        'name = "files"\n[inputs]\nnetwork = "network"\ndemand = "trips.csv"\n'
        '[equilibration]\nmethod = "sra"\niterations = 3\n'
    )
    first = ms.config_read(tmp_path / "run.toml").run(str(tmp_path / "first"), quiet=True)
    assert main(["config", "--from", str(tmp_path / "first")]) == 0
    printed = capsys.readouterr().out
    assert printed == (tmp_path / "first" / "run.toml").read_text()
    assert f'network = "{tmp_path.resolve() / "network"}"' in printed
    assert main(["run", "--config", str(tmp_path / "first" / "run.toml"), "--output",
                 str(tmp_path / "second"), "--no-pages"]) == 0  # fmt: skip
    second = json.loads((tmp_path / "second" / "manifest.json").read_text())
    assert second["run_fingerprint"] == first.fingerprint


def test_a_run_with_services_and_a_policy_repeats_from_its_copy(tmp_path) -> None:
    net = ms.examples.toy_network()
    rows = ms.examples.trips_random(net, 60, seed=3, min_m=100.0, max_m=2000.0, spread_s=1800)
    coords, _ = net.link_geometry()
    docks = ms.dock_read_table(
        [{"dock_id": f"d{i}", "lon": p[0], "lat": p[1], "capacity": 6, "bikes": 3}
         for i, p in enumerate(coords[[0, len(coords) // 2, -1]])]
    )  # fmt: skip
    taxis = ms.vehicle_read_table(
        [{"vehicle_id": f"v{i}", "lon": p[0], "lat": p[1]} for i, p in enumerate(coords[[0, -1]])]
    )

    def scenario(**options: object) -> ms.Scenario:
        return ms.Scenario.from_parts(
            net,
            rows,
            modes=["car", "walk", "bike", "bike_share", "taxi"],
            price_options={"taxi_eur_per_km": 1.0},
            services=[
                ms.service_bike_docked(docks, options=options.get("bikes")),
                ms.service_taxi(taxis, options=options.get("taxis")),
            ],  # fmt: skip
        )

    first = scenario(taxis={"request_cancel_s": 900}).run(
        "services",
        output_dir=str(tmp_path / "first"),
        quiet=True,
        service_assumptions={"assumed_wait_s": 300},
        policy="threshold",
        policy_options={"low": 0.1},
    )
    copy = first.config()
    assert copy.values["services"]["taxi"]["request_cancel_s"] == 900
    assert copy.values["policy"] == {"name": "threshold", "low": 0.1, "high": 0.8, "step_s": 900}
    assert "policy" in copy.differences() and "services.taxi" in copy.differences()
    again = scenario().run(output_dir=str(tmp_path / "again"), quiet=True, config=copy)
    assert again.fingerprint == first.fingerprint
