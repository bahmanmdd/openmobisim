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


# --- the results file, the run's fingerprint and its seed (S168) ------------------------


def toy_run(**kwargs):
    net = ms.examples.toy_network()
    rows = toy_rows(net, [("W", "D1", 30, 10)])
    # Pinned to the pre-car-ready-checkpoint defaults as this file's own fixture default (not
    # the library's, "logit"/"msa" since 2026-09-23): this file is about link-bin output and
    # fingerprint/manifest identity, not the choice model or equilibration.
    settings = {
        "class_defaults": CAR,
        "window_hours": 1,
        "flow_level": 4,
        "link_bin_s": 300,
        "choice_model": "deterministic",
        "equilibration": "none",
    }
    settings.update(kwargs)
    return net, rows, ms.Scenario.from_parts(net, rows, **settings).run("identity-test")


def test_per_link_results_are_written_to_a_parquet_file():
    _, _, run = toy_run()
    table = run.link_bins_table()
    assert table is not None
    path = run._summary.link_bins_path
    with open(path, "rb") as f:
        data = f.read()
    assert data[:4] == b"PAR1" and data[-4:] == b"PAR1", "a Parquet file"
    assert run.manifest()["link_bins_file"] == "link_bins.parquet"
    assert run.manifest()["link_bin_seconds"] == 300


def test_the_file_holds_the_runs_table_with_external_ids():
    pytest.importorskip("pandas")
    pq = pytest.importorskip("pyarrow.parquet")
    net, _, run = toy_run()
    bins = run.link_bins()
    df = run.link_bins_table().to_pandas()
    assert list(df.columns) == [
        "run_id", "bin", "start_s", "link", "link_index", "crossings", "pcu", "pcu_seconds",
    ]  # fmt: skip
    assert len(df) == len(bins) and (df["run_id"] == "identity-test").all()
    assert (df["bin"].to_numpy() == bins.bins()).all()
    assert (df["start_s"].to_numpy() == bins.bins() * bins.bin_seconds).all()
    assert (df["link_index"].to_numpy() == bins.links()).all()
    assert (df["crossings"].to_numpy() == bins.crossings()).all()
    assert np.allclose(df["pcu"], bins.pcu()) and np.allclose(df["pcu_seconds"], bins.pcu_seconds())
    # External ids: one per internal link, and they are what the network calls it.
    assert df["link"].map(str).str.len().min() > 0
    assert df.groupby("link_index")["link"].nunique().max() == 1
    # The file says which run and network it belongs to.
    meta = pq.read_metadata(run._summary.link_bins_path).metadata
    assert meta[b"openmobisim.run_fingerprint"].decode() == run.fingerprint
    assert meta[b"openmobisim.master_seed"] == b"0" and meta[b"openmobisim.bin_seconds"] == b"300"
    assert (
        meta[b"openmobisim.network_fingerprint"].decode() == run.manifest()["network_fingerprint"]
    )


def test_there_is_no_results_file_unless_asked_for():
    _, _, run = toy_run(link_bin_s=None)
    assert run.link_bins_table() is None
    assert run.manifest()["link_bin_seconds"] is None and run.manifest()["link_bins_file"] is None


def test_a_run_has_a_fingerprint_and_a_seed_and_the_manifest_agrees():
    _, _, run = toy_run()
    assert len(run.fingerprint) == 16 and int(run.fingerprint, 16) >= 0
    assert run.master_seed == 0
    manifest = run.manifest()
    assert manifest["run_fingerprint"] == run.fingerprint and manifest["master_seed"] == 0
    assert manifest["flow_level"] == 4 and manifest["flow_step_seconds"] == 300
    assert manifest["route_method"] == "penalty" and manifest["route_descriptor"].startswith(
        "penalty"
    )
    assert len(manifest["network_fingerprint"]) == 16


def test_the_fingerprint_follows_the_inputs_and_only_the_inputs():
    net, rows, base = toy_run()
    same = toy_run()[2]
    assert same.fingerprint == base.fingerprint
    # A different seed, step, level, bin length, method and demand each make a new fingerprint.
    variants = [
        toy_run(master_seed=7)[2],
        toy_run(flow_step_s=60)[2],
        toy_run(flow_level=3)[2],
        toy_run(link_bin_s=600)[2],
        toy_run(route_method="shortest")[2],
        toy_run(route_options={"max_paths": 3})[2],
    ]
    shifted = [(*r[:6], r[6] + 1, *r[7:]) for r in rows]
    variants.append(
        ms.Scenario.from_parts(
            net, shifted, class_defaults=CAR, window_hours=1, flow_level=4, link_bin_s=300
        ).run("identity-test")
    )
    prints = {v.fingerprint for v in variants} | {base.fingerprint}
    assert len(prints) == len(variants) + 1, "every change is a different run"
    assert toy_run(master_seed=7)[2].master_seed == 7
    # The step is bookkeeping (S88): the outcome is the same, yet it is an input, so a new run.
    assert toy_run(flow_step_s=60)[2].completion == base.completion


def test_a_seed_must_be_a_whole_number_in_range():
    net = ms.examples.toy_network()
    rows = toy_rows(net, [("W", "D1", 1, 1)])
    for bad in (-1, 2**64):
        with pytest.raises(ValueError, match="master_seed"):
            ms.Scenario.from_parts(net, rows, master_seed=bad)
    ms.Scenario.from_parts(net, rows, class_defaults=CAR, master_seed=2**64 - 1).run("max-seed")
