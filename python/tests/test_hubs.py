"""The run's hubs in the outputs: a station's stops and the parkings beside it share one hub id."""

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


def test_a_parking_joins_the_hub_its_table_names_or_the_nearest_stop_s(tmp_path) -> None:
    run = _run(tmp_path, "hubs")
    stops = ms.examples.toy_network_transit().stops()
    rows = ms.examples.toy_network_parkings().rows()
    table = dict(zip(rows["parking_id"], rows["hub_id"], strict=True))
    places = run.parking_places()
    for i, parking in enumerate(places["parking_id"]):
        here = (places["lon"][i], places["lat"][i])
        near = sorted(
            (_metres(here, (lon, lat)), sid)
            for sid, lon, lat in zip(stops["stop_id"], stops["lon"], stops["lat"], strict=True)
        )
        if table[parking] is not None:
            assert places["hub_id"][i] == table[parking], "the hub its table names"
        elif near[0][0] <= 150.0:
            assert places["hub_id"][i] == near[0][1], "the nearest stop's hub"
        else:
            assert places["hub_id"][i] == parking, "a hub of its own"
    calls = run.transit_calls()
    assert calls["hub_id"] == calls["stop_id"], "the toy's stops have no parent station"


def test_the_hubs_label_the_outputs_and_change_no_result(tmp_path) -> None:
    pytest.importorskip("pyarrow")
    import pyarrow.parquet as pq

    wide = _run(tmp_path, "wide", hub_join_m=5000.0)
    none = _run(tmp_path, "none", hub_join_m=0.0)
    assert wide.fingerprint == none.fingerprint and wide.completion == none.completion
    rows = ms.examples.toy_network_parkings().rows()
    table = dict(zip(rows["parking_id"], rows["hub_id"], strict=True))
    places = none.parking_places()
    assert places["hub_id"] == [table[p] or p for p in places["parking_id"]], (
        "at 0 m a parking is in the hub its table names, or its own"
    )
    assert pq.read_table(tmp_path / "wide" / "transit_calls.parquet").column_names[-1] == "hub_id"
    assert "hub_group" not in pq.read_table(tmp_path / "wide" / "parking_bins.parquet").column_names
    assert set(
        pq.read_table(tmp_path / "wide" / "parking_bins.parquet")["hub_id"].to_pylist()
    ) <= set(ms.examples.toy_network_transit().stops()["stop_id"]) | set(table) | {
        h for h in table.values() if h
    }
    with pytest.raises(ValueError, match="hub_join_m"):
        _run(tmp_path, "bad", hub_join_m=-1.0)
