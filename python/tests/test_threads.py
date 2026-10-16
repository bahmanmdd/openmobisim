"""A run's results do not depend on how many threads it uses.

The guarantee: reductions in a fixed order, random draws keyed to the seed,
the traveller and the iteration, never to a thread. Each run below is made with one thread and
with several, and everything it wrote is compared, bit for bit: the fingerprint, the KPIs, the
events, the per-link bins of every layer, the transit calls and the parkings.
"""

from __future__ import annotations

import os

import openmobisim as ms
import pytest

pq = pytest.importorskip("pyarrow.parquet")  # the results are compared as tables

BUNDLE = os.environ.get("OPENMOBISIM_TEST_BUNDLE")

#: Every file a run writes that holds results, by the summary's path attribute.
RESULTS = (
    "kpis_path",
    "events_path",
    "link_bins_path",
    "link_bins_bike_path",
    "link_bins_walk_path",
    "transit_calls_path",
    "parking_bins_path",
)


def _results(run: ms.Run) -> dict[str, object]:
    """Every result file's table, without the run's id (which names the run, not its result)."""
    out = {}
    for name in RESULTS:
        path = getattr(run._summary, name, None)
        if path and os.path.exists(path):
            table = pq.read_table(path)
            if "run_id" in table.column_names:
                table = table.drop(["run_id"])
            out[name] = table.replace_schema_metadata(None)
    return out


def _same(one: ms.Run, many: ms.Run) -> None:
    assert one.fingerprint == many.fingerprint
    assert one.manifest()["threads"] == 1 and many.manifest()["threads"] > 1
    a, b = _results(one), _results(many)
    assert a.keys() == b.keys() and "kpis_path" in a
    for name in a:
        assert a[name].equals(b[name]), name
    assert one.completion == many.completion


def test_a_multimodal_run_is_the_same_at_one_thread_and_at_four(tmp_path) -> None:
    """Mode choice among car, bike and walk, classes, iterations: 1 and 4 threads agree."""
    net = ms.examples.manhattan_grid(n=8, block_metres=250.0, signals=True)
    rows = ms.examples.trips_random(net, 3000, seed=11, min_m=300.0, max_m=3000.0, spread_s=1800)
    classes = ms.examples.traveller_classes()
    rows = ms.demand_assign_classes(rows, classes, seed=2)
    scenario = ms.Scenario.from_parts(
        net,
        rows,
        classes=classes,
        modes=["car", "bike", "walk"],
        choice_model="logit",
        equilibration="msa",
        equilibration_options={"iterations": 4},
        link_bin_s=600,
    )
    one = scenario.run("threads-1", str(tmp_path / "one"), quiet=True, threads=1)
    four = scenario.run("threads-4", str(tmp_path / "four"), quiet=True, threads=4)
    _same(one, four)


def test_threads_must_be_at_least_one() -> None:
    net = ms.examples.manhattan_grid(n=3, block_metres=200.0, signals=False)
    rows = ms.examples.trips_random(net, 10, seed=1, min_m=200.0, max_m=600.0)
    scenario = ms.Scenario.from_parts(net, rows)
    with pytest.raises(ValueError, match="at least 1"):
        scenario.run("threads-0", quiet=True, threads=0)


@pytest.mark.skipif(BUNDLE is None, reason="set OPENMOBISIM_TEST_BUNDLE to the bundle's folder")
@pytest.mark.parametrize("weight", [None, 1], ids=["fast", "complete"])
def test_delft_is_the_same_at_one_thread_and_at_every_core(tmp_path, weight) -> None:
    """The user's check: Delft, its fast and its complete run, 1 thread and all."""
    if (os.cpu_count() or 1) < 2:
        pytest.skip("one core: nothing to compare")
    scenario = ms.examples.case("delft", root=BUNDLE).scenario("base", weight=weight)
    one = scenario.run("delft-1", str(tmp_path / "one"), quiet=True, threads=1)
    every = scenario.run("delft-all", str(tmp_path / "all"), quiet=True)
    _same(one, every)
