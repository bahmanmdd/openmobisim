"""Every result given as columns is a ``Columns`` (C2, D-5 a): a dict, and a data frame too."""

from __future__ import annotations

import openmobisim as ms
import pytest


@pytest.fixture(scope="module")
def run(tmp_path_factory: pytest.TempPathFactory) -> ms.Run:
    net = ms.examples.toy_network()
    rows = ms.examples.trips_random(net, 40, seed=3, min_m=100.0, max_m=2000.0, spread_s=1800)
    classes = ms.examples.traveller_classes()
    rows = ms.demand_assign_classes(rows, classes, seed=1)
    out = tmp_path_factory.mktemp("columns") / "run"
    return ms.Scenario.from_parts(
        net,
        rows,
        classes=classes,
        transit=ms.examples.toy_network_transit(),
        parkings=ms.examples.toy_network_parkings(),
    ).run("columns", output_dir=str(out), quiet=True)


RESULTS = (
    "convergence",
    "timings",
    "report_no_alternative",
    "transit_calls",
    "parking_bins",
    "parking_places",
    "itinerary_choices",
    "route_changes",
    "route_realised",
    "trip_modes",
)


def test_column_results_are_columns_of_one_length(run: ms.Run) -> None:
    for name in RESULTS:
        columns = getattr(run, name)()
        assert isinstance(columns, ms.Columns) and isinstance(columns, dict), name
        assert {len(v) for v in columns.values()} <= {columns.row_count}, name
    assert isinstance(run.fallback_car, ms.Columns)
    assert isinstance(ms.parameters(), ms.Columns)


def test_columns_convert_to_data_frames(run: ms.Run) -> None:
    pd = pytest.importorskip("pandas")
    pa = pytest.importorskip("pyarrow")
    for name in RESULTS:
        columns = getattr(run, name)()
        frame = columns.to_pandas()
        assert isinstance(frame, pd.DataFrame) and list(frame.columns) == list(columns), name
        assert len(frame) == columns.row_count, name
        table = columns.to_arrow()
        assert isinstance(table, pa.Table) and table.num_rows == columns.row_count, name
    assert run.kpis().to_arrow().num_rows == len(run.kpis().to_pandas())


def test_columns_behave_as_the_dicts_they_were() -> None:
    columns = ms.Columns(a=[1, 2], b=["x", "y"])
    assert columns == {"a": [1, 2], "b": ["x", "y"]}
    assert columns.row_count == 2 and ms.Columns().row_count == 0
    assert repr(columns) == "Columns(2 rows; a, b)"
