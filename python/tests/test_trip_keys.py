"""A trip is keyed by ``(traveller_id, trip_seq)``: a repeated key is refused, never dropped (X-68).

Dropped, a second row with the same key would make a run simulate fewer trips than the demand
given, and every total would be of a smaller demand than the one given.
"""

from __future__ import annotations

import openmobisim as ms
import pytest

CAR = {"commuter": (True, False, False)}


def trips(net: ms.Network) -> list[tuple]:
    """Four car trips W to D1 at 07:00, two of them given as the same traveller's trip 0."""
    o, d = net.node_lonlat("W"), net.node_lonlat("D1")
    return [
        ("a", 0, *o, *d, 25200, "commuter", None),
        ("same", 0, *o, *d, 25200, "commuter", None),
        ("b", 0, *o, *d, 25200, "commuter", None),
        ("same", 0, *o, *d, 25200, "commuter", None),
    ]


def test_a_repeated_traveller_id_and_trip_seq_is_refused_with_the_rows_named() -> None:
    net = ms.examples.toy_network()
    with pytest.raises(ValueError, match=r"row 3 \(from 0\) repeats row 1's") as caught:
        ms.Scenario.from_parts(net, trips(net), classes=CAR, flow_level=2)
    assert "('same', 0)" in str(caught.value) and "1 row(s)" in str(caught.value)


def test_distinct_rows_are_all_simulated() -> None:
    net = ms.examples.toy_network()
    rows = trips(net)
    rows[3] = ("same", 1, *rows[3][2:])  # the same traveller's second trip
    run = ms.Scenario.from_parts(
        net, rows, classes=CAR, flow_level=2, equilibration="free_flow"
    ).run("distinct")
    assert run.completion["total_trips"] == len(rows) == 4
    assert len(run.trip_results()["trip"]) == 4


def test_a_file_read_by_its_path_is_checked_the_same_way(tmp_path) -> None:
    pa = pytest.importorskip("pyarrow")
    pq = pytest.importorskip("pyarrow.parquet")

    net = ms.examples.toy_network()
    rows = trips(net)
    names = ms.Trip._fields[:8]  # weight left out: the default weight
    table = pa.table({n: [r[i] for r in rows] for i, n in enumerate(names)})
    path = tmp_path / "trips.parquet"
    pq.write_table(table, path)
    with pytest.raises(ValueError, match=r"trip row 3 \(from 0\) repeats row 1's"):
        ms.Scenario.from_parts(net, str(path), classes=CAR, flow_level=2).run("from_file")
