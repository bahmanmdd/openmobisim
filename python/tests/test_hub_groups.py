"""Hub groups (C4, D-6 b): a station's stops and the parkings beside it share one label."""

from __future__ import annotations

import math

import openmobisim as ms
import pytest


def _run(tmp_path, name: str, **run_options) -> ms.Run:
    net = ms.examples.toy_network()
    rows = ms.examples.trips_random(net, 60, seed=3, min_m=100.0, max_m=2000.0, spread_s=1800)
    classes = ms.examples.traveller_classes()
    rows = ms.demand_assign_classes(rows, classes, seed=1)
    scenario = ms.Scenario.from_parts(
        net,
        rows,
        classes=classes,
        transit=ms.examples.toy_network_transit(),
        parkings=ms.examples.toy_network_parkings(),
    )
    return scenario.run(name, output_dir=str(tmp_path / name), quiet=True, **run_options)


def _metres(a: tuple[float, float], b: tuple[float, float]) -> float:
    k = math.cos(math.radians((a[1] + b[1]) / 2))
    return 6_371_008.8 * math.radians(math.hypot((a[0] - b[0]) * k, a[1] - b[1]))


def test_a_parking_joins_the_group_of_the_nearest_stop_within_the_radius(tmp_path) -> None:
    run = _run(tmp_path, "groups")
    stops = ms.examples.toy_network_transit().stops()
    places = run.parking_places()
    for i, parking in enumerate(places["parking_id"]):
        here = (places["lon"][i], places["lat"][i])
        near = sorted(
            (_metres(here, (lon, lat)), sid)
            for sid, lon, lat in zip(stops["stop_id"], stops["lon"], stops["lat"], strict=True)
        )
        own = places["hub_id"][i]
        if own != parking:
            assert places["hub_group"][i] == own, "a hub its table named is kept"
        elif near[0][0] <= 150.0:
            assert places["hub_group"][i] == near[0][1]
        else:
            assert places["hub_group"][i] == own
    calls = run.transit_calls()
    assert calls["hub_group"] == calls["stop_id"], "the toy's stops have no parent station"


def test_the_groups_label_the_outputs_and_change_no_result(tmp_path) -> None:
    pytest.importorskip("pyarrow")
    import pyarrow.parquet as pq

    wide = _run(tmp_path, "wide", hub_group_m=5000.0)
    none = _run(tmp_path, "none", hub_group_m=0.0)
    assert wide.fingerprint == none.fingerprint and wide.completion == none.completion
    places = none.parking_places()
    assert places["hub_group"] == places["hub_id"], "at 0 m every parking is its own group"
    for name in ("transit_calls", "parking_bins"):
        table = pq.read_table(tmp_path / "wide" / f"{name}.parquet")
        assert table.column_names[-1] == "hub_group"
    assert set(
        pq.read_table(tmp_path / "wide" / "parking_bins.parquet")["hub_group"].to_pylist()
    ) <= set(ms.examples.toy_network_transit().stops()["stop_id"]) | set(
        wide.parking_places()["hub_id"]
    )
    with pytest.raises(ValueError, match="hub_group_m"):
        _run(tmp_path, "bad", hub_group_m=-1.0)
