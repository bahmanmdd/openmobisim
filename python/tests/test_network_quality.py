"""Auditing a network from Python: connectivity, what an import did, study areas (S172).

What is defended: the arrays that let a user audit a network agree with each
other and with the network; the connectivity verdict is right on networks whose
answer is known, and is checked here by an independent reachability search rather
than by the core's own; bad options say what is wrong before any file is read;
and, when a real extract is pointed at, a strongly connected import really is
one, removes only what the report says, and changes nothing else.

The real-extract tests need ``OPENMOBISIM_TEST_PBF`` (any ``.osm.pbf``; data is
never committed) and are skipped without it.
"""

import os

import numpy as np
import openmobisim as ms
import pytest


def reaches_all(n_nodes, frm, to, start, forward=True):
    """The nodes reachable from `start`, along the links or against them.

    A plain search: the slow, obviously right way.
    """
    a, b = (frm, to) if forward else (to, frm)
    order = np.argsort(a, kind="stable")
    a, b = a[order], b[order]
    seen = np.zeros(n_nodes, bool)
    seen[start] = True
    stack = [start]
    while stack:
        u = stack.pop()
        lo, hi = np.searchsorted(a, [u, u + 1])
        for v in b[lo:hi]:
            if not seen[v]:
                seen[v] = True
                stack.append(v)
    return seen


def strongly_connected_by_search(net):
    """Every node reaches node 0 and is reached from it (the definition)."""
    n = net.node_count
    frm, to = net.link_from(), net.link_to()
    return bool(
        reaches_all(n, frm, to, 0).all() and reaches_all(n, frm, to, 0, forward=False).all()
    )


def test_the_link_arrays_agree_with_the_network():
    net = ms.examples.manhattan_grid(n=4, block_metres=100.0, signals=True)
    frm, to = net.link_from(), net.link_to()
    assert frm.dtype == to.dtype == np.uint32
    assert len(frm) == len(to) == net.link_count == len(net.link_length_m())
    assert frm.max() < net.node_count and to.max() < net.node_count
    assert (frm != to).all(), "no link runs from a node to itself"
    assert net.link_drivable().all() and len(net.link_drivable()) == net.link_count
    assert len(net.link_roundabout()) == net.link_count and not net.link_roundabout().any()
    sig = net.node_signalised()
    assert len(sig) == net.node_count and sig.sum() == 4, "the four interior nodes of a 4x4 grid"


def test_a_grid_is_strongly_connected_and_the_search_agrees():
    net = ms.examples.manhattan_grid(n=5, block_metres=100.0, signals=False)
    report = net.report_connectivity()
    assert report["strongly_connected"] and report["node_components"] == 1
    assert (report["sources"], report["sinks"]) == (0, 0)
    assert (
        report["node_largest"] == net.node_count
        and report["links_in_node_largest"] == net.link_count
    )
    assert strongly_connected_by_search(net)


def test_the_toy_network_is_not_strongly_connected_and_says_where():
    # Made for hand-checking the loading, with one-way links and streams that
    # start and end inside it; it was never meant to be all-to-all.
    net = ms.examples.toy_network()
    report = net.report_connectivity()
    assert not report["strongly_connected"]
    assert report["sources"] > 0 and report["sinks"] > 0
    assert report["node_largest"] < net.node_count
    assert not strongly_connected_by_search(net), "and the search agrees"


def test_the_report_has_the_numbers_a_reader_needs():
    report = ms.examples.manhattan_grid(
        n=3, block_metres=100.0, signals=False
    ).report_connectivity()
    for key in (
        "strongly_connected", "nodes", "links", "node_components", "node_largest",
        "links_in_node_largest", "link_components", "link_largest", "sources", "sinks",
        "node_component_sizes_top", "turns", "u_turns", "max_turns_per_node",
    ):  # fmt: skip
        assert key in report, key
    assert report["turns"] > 0 and report["u_turns"] == 0


def test_an_unknown_mode_says_which_are_known():
    net = ms.examples.manhattan_grid(n=3, block_metres=100.0, signals=False)
    with pytest.raises(ValueError, match="car.*all"):
        net.report_connectivity("walk")
    assert net.report_connectivity("all") == net.report_connectivity("car"), (
        "a network of drivable links only: the two modes see the same thing"
    )


def test_a_network_not_read_from_osm_has_no_import_to_report():
    net = ms.examples.manhattan_grid(n=3, block_metres=100.0, signals=False)
    assert net.report_import() is None and net.report_dropped() is None


@pytest.mark.parametrize(
    ("kwargs", "message"),
    [
        ({"connectivity": "bogus"}, "connectivity must be"),
        ({"region": (1, 2, 3)}, "region must be"),
        ({"region": (3, 2, 1, 4)}, "west < east"),
        ({"region": (1, 2, float("nan"), 4)}, "finite"),
        ({"region": [(0, 0), (1, 1)]}, "three vertices"),
        ({"region": "the middle"}, "region must be"),
    ],
)
def test_bad_options_are_refused_before_any_file_is_read(kwargs, message):
    # The file does not exist: the option is what is complained about, not the file.
    with pytest.raises(ValueError, match=message):
        ms.network_read_osm("does-not-exist.osm.pbf", **kwargs)


def test_a_missing_file_is_an_error_not_an_empty_network():
    with pytest.raises(ValueError, match="could not"):
        ms.network_read_osm("does-not-exist.osm.pbf", region=(0, 0, 1, 1), connectivity="strong")


needs_pbf = pytest.mark.skipif(
    "OPENMOBISIM_TEST_PBF" not in os.environ, reason="needs a .osm.pbf extract"
)


@needs_pbf
def test_a_strong_import_is_strongly_connected_by_an_independent_search():
    path = os.environ["OPENMOBISIM_TEST_PBF"]
    net = ms.network_read_osm(path, connectivity="strong")
    drivable = net.link_drivable()
    frm, to = net.link_from()[drivable], net.link_to()[drivable]
    nodes = np.unique(np.concatenate([frm, to]))
    dense = {int(n): i for i, n in enumerate(nodes)}
    f = np.array([dense[int(x)] for x in frm])
    t = np.array([dense[int(x)] for x in to])
    assert reaches_all(len(nodes), f, t, 0).all() and reaches_all(len(nodes), f, t, 0, False).all()
    assert net.report_connectivity("car")["strongly_connected"]


@needs_pbf
def test_the_filter_removes_what_it_reports_and_changes_nothing_else():
    path = os.environ["OPENMOBISIM_TEST_PBF"]
    keep = ms.network_read_osm(path, connectivity="keep")
    strong = ms.network_read_osm(path, connectivity="strong")
    r = strong.report_import()
    assert r["connectivity"] == "strong" and keep.report_import()["connectivity"] == "keep"
    assert r["links_disconnected"] > 0, "a real extract is never quite connected"
    # Fewer links, and by more than the removed ones alone (contraction merges across
    # the gaps they leave) but never by less.
    assert strong.link_count < keep.link_count
    # Links a car cannot use are never removed, at most merged where a removed road no
    # longer holds them apart.
    assert (~strong.link_drivable()).sum() <= (~keep.link_drivable()).sum()
    coords, offsets, ways, reasons, classes = strong.report_dropped()
    assert len(ways) == len(reasons) == len(classes) == len(offsets) - 1
    assert (classes[reasons == 1] < 15).all(), (
        "only roads a car may use are removed for connectivity"
    )
    assert offsets[0] == 0 and offsets[-1] == len(coords)
    assert (reasons == 1).sum() == r["links_disconnected"]
    assert (reasons == 0).sum() == r["closed_loops_dropped"]
    # The default keeps everything, so its report has nothing removed for connectivity.
    kr = keep.report_import()
    assert kr["links_disconnected"] == 0 and kr["components_before"] == 0


@needs_pbf
def test_a_region_is_a_subset_and_a_polygon_equal_to_the_box_gives_the_same_network():
    path = os.environ["OPENMOBISIM_TEST_PBF"]
    whole = ms.network_read_osm(path)
    coords, _ = whole.link_geometry()
    (w, s), (e, n) = coords.min(axis=0), coords.max(axis=0)
    box = (w + (e - w) * 0.25, s + (n - s) * 0.25, w + (e - w) * 0.75, s + (n - s) * 0.75)
    part = ms.network_read_osm(path, region=box)
    assert 0 < part.link_count < whole.link_count
    pc, _ = part.link_geometry()
    assert (pc[:, 0] >= box[0] - 1e-9).all() and (pc[:, 0] <= box[2] + 1e-9).all()
    assert (pc[:, 1] >= box[1] - 1e-9).all() and (pc[:, 1] <= box[3] + 1e-9).all()
    # A polygon that is the box gives the same network as the box.
    poly = [(box[0], box[1]), (box[2], box[1]), (box[2], box[3]), (box[0], box[3])]
    assert ms.network_read_osm(path, region=poly).link_count == part.link_count
    assert "region" in part.report_import()


@needs_pbf
def test_contracting_past_footways_keeps_the_streets_as_long_as_they_were():
    path = os.environ["OPENMOBISIM_TEST_PBF"]
    plain = ms.network_read_osm(path)
    merged = ms.network_read_osm(path, contract_drivable=True)
    assert merged.link_count <= plain.link_count
    length = lambda n: n.link_length_m()[n.link_drivable()].sum()  # noqa: E731
    assert length(merged) == pytest.approx(length(plain), rel=1e-9)
    footway = lambda n: n.link_length_m()[~n.link_drivable()].sum()  # noqa: E731
    assert footway(merged) == pytest.approx(footway(plain), rel=1e-9)
    assert merged.report_import()["contract_drivable"] is True
