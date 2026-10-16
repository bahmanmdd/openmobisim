"""The minimal outputs for mass runs: ``kpis.csv`` and ``manifest.json``, the same run."""

from __future__ import annotations

import csv
import json

import openmobisim as ms
import pytest


def _scenario() -> ms.Scenario:
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
        link_bin_s=300,
    )


def test_minimal_outputs_are_two_text_files_of_the_same_run(tmp_path) -> None:
    pytest.importorskip("pyarrow")
    import pyarrow.parquet as pq

    scenario = _scenario()
    full = scenario.run("full", output_dir=str(tmp_path / "full"), quiet=True)
    small = scenario.run(
        "run-0001", output_dir=str(tmp_path / "run-0001"), quiet=True, outputs="minimal"
    )
    assert sorted(p.name for p in (tmp_path / "run-0001").iterdir()) == [
        "kpis.csv",
        "manifest.json",
    ]
    manifest = json.loads((tmp_path / "run-0001" / "manifest.json").read_text())
    assert manifest["outputs"] == "minimal" and manifest["link_bins_file"] is None
    assert small.fingerprint == full.fingerprint and small.completion == full.completion
    rows = list(csv.DictReader((tmp_path / "run-0001" / "kpis.csv").open()))
    table = pq.read_table(tmp_path / "full" / "kpis.parquet").to_pylist()
    assert [(r["iteration"], r["mode"], r["metric"], float(r["value"])) for r in rows] == [
        (str(r["iteration"]), r["mode"], r["metric"], r["value"]) for r in table
    ], "the same rows, every value exact"


def test_a_minimal_run_reads_back_and_refuses_what_it_did_not_write(tmp_path) -> None:
    scenario = _scenario()
    small = scenario.run(
        "run-0002", output_dir=str(tmp_path / "run-0002"), quiet=True, outputs="minimal"
    )
    back = ms.Run.open(tmp_path / "run-0002")
    assert back.fingerprint == small.fingerprint
    assert back.completion == small.completion
    assert back.convergence_verdict == small.convergence_verdict
    for refused in (small.events, small.diagnostics, small.link_bins_table, back.events):
        with pytest.raises(RuntimeError, match="minimal outputs"):
            refused()
    with pytest.raises(ValueError, match="outputs"):
        scenario.run("bad", output_dir=str(tmp_path / "bad"), quiet=True, outputs="none")
