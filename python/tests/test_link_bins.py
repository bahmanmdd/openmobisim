"""Per-link results and the network's arrays, across the boundary (S163).

What is defended: the arrays a figure reads describe the network (shapes,
dtypes, indexing), and the per-link table agrees with the demand that made it.
"""

import os

import numpy as np
import openmobisim as ms
import pytest

CAR = {"commuter": (True, False, False)}


def toy_rows(network, streams):
    """Trips for `streams`: ``(origin_node, destination_node, count, headway_s)``."""
    rows = []
    for origin, destination, count, headway in streams:
        (olon, olat), (dlon, dlat) = network.node_lonlat(origin), network.node_lonlat(destination)
        for k in range(count):
            rows.append((f"t{len(rows)}", 0, olon, olat, dlon, dlat, headway * k, "commuter", None))
    return rows


def test_a_network_reads_as_flat_arrays():
    net = ms.examples.toy_network()
    assert (net.node_count, net.link_count) == (16, 16)
    coords, offsets = net.link_geometry()
    assert coords.dtype == np.float64 and coords.shape[1] == 2
    assert offsets.dtype == np.uint32 and len(offsets) == 17 and offsets[0] == 0
    assert offsets[-1] == len(coords)
    for array in (
        net.link_length_m(),
        net.link_free_flow_s(),
        net.link_storage_pcu(),
        net.link_capacity_pcu_h(),
    ):
        assert array.dtype == np.float64 and array.shape == (16,) and (array > 0).all()
    assert net.link_class().dtype == np.uint8 and net.link_lanes().dtype == np.uint8
    # A network built from nodes alone has straight two-point links.
    assert (np.diff(offsets) == 2).all()
    assert net.source == "synthetic"


def test_a_named_node_has_a_position_and_an_unknown_one_is_an_error():
    net = ms.examples.toy_network()
    lon, lat = net.node_lonlat("W")
    assert 4.0 < lon < 5.5 and 45.0 < lat < 46.5
    with pytest.raises(ValueError, match="no such node"):
        net.node_lonlat("nowhere")


def test_a_run_reports_per_link_bins_that_match_its_demand():
    net = ms.examples.toy_network()
    rows = toy_rows(net, [("W", "D1", 30, 10)])
    scenario = ms.Scenario.from_parts(
        net, rows, class_defaults=CAR, window_hours=1, flow_level=4, link_bin_s=300
    )
    run = scenario.run("bins-test")
    bins = run.link_bins()
    assert bins is not None and bins.bin_seconds == 300 and len(bins) > 0
    assert run.completion["completed"] == 30
    # W -> D1 crosses seven links: each is crossed by all thirty cars, once.
    links = bins.links()
    for link in np.unique(links):
        assert bins.crossings()[links == link].sum() == 30
        assert bins.pcu()[links == link].sum() == pytest.approx(30.0)
    assert len(np.unique(links)) == 7
    # Sorted by bin, then link.
    order = np.lexsort((links, bins.bins()))
    assert (order == np.arange(len(bins))).all()


def test_per_link_bins_are_off_unless_asked_for():
    net = ms.examples.toy_network()
    scenario = ms.Scenario.from_parts(net, toy_rows(net, [("W", "D1", 3, 10)]), class_defaults=CAR)
    assert scenario.run("no-bins").link_bins() is None


def test_free_flow_bins_carry_free_flow_times():
    net = ms.examples.toy_network()
    rows = toy_rows(net, [("W", "D1", 5, 60)])
    run = ms.Scenario.from_parts(
        net, rows, class_defaults=CAR, window_hours=1, flow_level=0, link_bin_s=600
    ).run("free")
    bins = run.link_bins()
    mean = bins.pcu_seconds() / bins.pcu()
    assert mean == pytest.approx(net.link_free_flow_s()[bins.links()])


def test_the_table_does_not_depend_on_the_loading_step():
    net = ms.examples.toy_network()
    rows = toy_rows(net, [("W", "D1", 40, 6), ("N2", "D2", 40, 5), ("N1", "D1", 20, 9)])
    tables = []
    for step in (60, 300, 3600):
        run = ms.Scenario.from_parts(
            net,
            rows,
            class_defaults=CAR,
            window_hours=2,
            flow_level=4,
            flow_step_s=step,
            link_bin_s=120,
        ).run(f"step-{step}")
        b = run.link_bins()
        tables.append(
            (b.bins().tolist(), b.links().tolist(), b.pcu().tolist(), b.pcu_seconds().tolist())
        )
    assert tables[0] == tables[1] == tables[2]


def test_bad_flow_settings_are_rejected():
    net = ms.examples.toy_network()
    rows = toy_rows(net, [("W", "D1", 1, 1)])
    with pytest.raises(ValueError, match="flow_level"):
        ms.Scenario.from_parts(net, rows, flow_level=1)
    with pytest.raises(ValueError, match="flow_step_s"):
        ms.Scenario.from_parts(net, rows, flow_level=4, flow_step_s=0)
    with pytest.raises(ValueError, match="link_bin_s"):
        ms.Scenario.from_parts(net, rows, link_bin_s=0)


def test_random_trips_are_seeded_and_within_range():
    net = ms.examples.manhattan_grid(n=6, block_metres=200.0, signals=False)
    a = ms.examples.trips_random(net, 50, seed=3, min_m=200.0, max_m=800.0)
    b = ms.examples.trips_random(net, 50, seed=3, min_m=200.0, max_m=800.0)
    c = ms.examples.trips_random(net, 50, seed=4, min_m=200.0, max_m=800.0)
    assert a == b and a != c and len(a) == 50
    assert all(0 <= row[6] < 3600 for row in a)
    with pytest.raises(ValueError, match="widen"):
        ms.examples.trips_random(net, 5, min_m=50_000.0, max_m=60_000.0)


@pytest.mark.skipif("OPENMOBISIM_TEST_PBF" not in os.environ, reason="needs a .osm.pbf extract")
def test_a_real_extract_runs_and_reports_link_bins():
    net = ms.network_read_osm(os.environ["OPENMOBISIM_TEST_PBF"])
    assert net.source == "osm" and net.link_count > 1000
    coords, offsets = net.link_geometry()
    assert (np.diff(offsets) >= 2).all() and offsets[-1] == len(coords)
    rows = ms.examples.trips_random(net, 500, seed=1)
    run = ms.Scenario.from_parts(
        net, rows, class_defaults=CAR, window_hours=1, flow_level=4, link_bin_s=300
    ).run("osm-test")
    bins = run.link_bins()
    assert len(bins) > 0 and run.completion["completed"] > 0
    assert bins.links().max() < net.link_count


def test_link_capacity_follows_class_and_lanes():
    net = ms.examples.toy_network()
    capacity = net.link_capacity_pcu_h()
    assert capacity.dtype == np.float64
    # The toy network: residential one-lane links carry 1400/h, the two-lane
    # link twice that, and the service road 800/h.
    assert sorted(set(np.round(capacity).astype(int))) == [800, 1400, 2800]
    lanes = net.link_lanes()
    assert (capacity[lanes == 2] > capacity[lanes == 1].min()).all()
