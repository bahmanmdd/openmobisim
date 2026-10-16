"""The package's messages through ``logging``: one line per run, shown once."""

from __future__ import annotations

import logging

import openmobisim as ms
import pytest


def _scenario() -> ms.Scenario:
    net = ms.examples.toy_network()
    rows = ms.examples.trips_random(net, 20, seed=3, min_m=100.0, max_m=2000.0, spread_s=1800)
    return ms.Scenario.from_parts(net, rows)


def test_a_run_says_one_line_and_its_timings_at_debug(tmp_path, caplog) -> None:
    with caplog.at_level(logging.INFO, logger="openmobisim"):
        _scenario().run("said", output_dir=str(tmp_path / "said"))
    assert len(caplog.messages) == 1 and caplog.messages[0].startswith("Run 'said': ")
    assert "trips completed" in caplog.messages[0]
    caplog.clear()
    with caplog.at_level(logging.DEBUG, logger="openmobisim"):
        _scenario().run("debug", output_dir=str(tmp_path / "debug"))
    assert any("wall-clock seconds by stage" in m for m in caplog.messages)
    caplog.clear()
    with caplog.at_level(logging.DEBUG, logger="openmobisim"):
        _scenario().run("quiet", output_dir=str(tmp_path / "quiet"), quiet=True)
    assert caplog.messages == []


def test_without_logging_set_up_the_line_goes_to_standard_output(
    tmp_path, capsys: pytest.CaptureFixture, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(logging.getLogger(), "handlers", [])  # as in a plain script or notebook
    _scenario().run("plain", output_dir=str(tmp_path / "plain"))
    out = capsys.readouterr().out
    assert out.startswith("Run 'plain': ") and out.count("\n") == 1
