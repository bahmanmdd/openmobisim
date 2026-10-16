"""A finished run read back from its folder (C2, D-5 a): ``Run.open``."""

from __future__ import annotations

import math

import openmobisim as ms
import pytest

pytest.importorskip("pyarrow")

SAME = (
    "fingerprint",
    "master_seed",
    "choice_model",
    "equilibration",
    "route_update",
    "converged",
    "completion",
    "completion_by_mode",
    "total_travel_time_s",
    "mean_travel_time_s",
    "convergence_gap",
    "convergence_verdict",
    "run_id",
)


def _live(folder) -> ms.Run:
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
        flow_level=4,
    ).run("opened", output_dir=str(folder), quiet=True)


def _equal(a, b) -> bool:
    if isinstance(a, dict):
        return a.keys() == b.keys() and all(_equal(a[k], b[k]) for k in a)
    if isinstance(a, float):
        return (math.isnan(a) and math.isnan(b)) or math.isclose(a, b, rel_tol=1e-12)
    return a == b


def test_an_opened_run_gives_what_its_files_hold_as_the_run_did(tmp_path) -> None:
    folder = tmp_path / "opened"
    live = _live(folder)
    back = ms.Run.open(folder)
    for name in SAME:
        live_value = getattr(live, name)
        if name == "completion_by_mode":  # the files keep the counts, not the people outside
            live_value = {
                m: {k: r[k] for k in back.completion_by_mode[m]} for m, r in live_value.items()
            }
        assert _equal(live_value, getattr(back, name)), name
    for name, values in back.convergence().items():
        theirs = live.convergence()[name]
        assert all(_equal(float(a), float(b)) for a, b in zip(values, theirs, strict=True)), name
    assert back.timings()["stage"] == live.timings()["stage"]
    assert back.kpis().to_pandas().equals(live.kpis().to_pandas())
    assert back.manifest() == live.manifest()


def test_what_only_the_run_in_memory_holds_is_refused_by_name(tmp_path) -> None:
    folder = tmp_path / "opened"
    _live(folder)
    back = ms.Run.open(folder)
    with pytest.raises(RuntimeError, match="route_sets"):
        back.route_sets()
    with pytest.raises(FileNotFoundError, match="manifest.json"):
        ms.Run.open(tmp_path)
