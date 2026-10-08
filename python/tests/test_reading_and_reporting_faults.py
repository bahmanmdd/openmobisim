"""Faults in reading inputs and reporting results, each fixed and kept fixed.

What is defended: a link table naming a node its node table lacks is read without that link
and says so, instead of failing; a global capacity factor is not mistaken for capacities reduced
to fit the diagram; a road closed with ``network_edit`` stays closed through GMNS files; a NumPy
integer names a link by its index, as a Python one does; a link map of a run in which no car
crossed a link is drawn; a parking table's quoted field may hold the delimiter, and a blank cell
keeps the columns after it in place; a starter case asked for in a given folder is looked for
there only; and ``check`` calls a result that is not the same to the bit on the reference's own
platform and version "different", however close.
"""

from __future__ import annotations

import warnings
from pathlib import Path

import numpy as np
import openmobisim as ms
import pytest
from openmobisim import _cases

NODES = [{"id": "a", "x": 0, "y": 0}, {"id": "b", "x": 100, "y": 0}, {"id": "c", "x": 200, "y": 0}]


def read(rows: list[dict], **kwargs: object) -> tuple:
    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always")
        net = ms.network_read_table(rows, NODES, coordinates="xy", coordinate_scale_m=1.0, **kwargs)
    return net, [str(w.message) for w in caught]


def test_a_link_to_an_unknown_node_is_left_out_and_reported():
    rows = [
        {"id": "ab", "from": "a", "to": "b", "capacity": 1000},
        {"id": "bc", "from": "b", "to": "c", "capacity": 1000},
        {"id": "cz", "from": "c", "to": "z", "capacity": 1000},
    ]
    net, said = read(rows)
    assert net.link_ids() == ["ab", "bc"]
    assert any("left out" in s and "cz" in s for s in said), said


def test_a_capacity_factor_is_not_reported_as_capacities_reduced():
    rows = [{"from": "a", "to": "b", "capacity": 1000, "class": "primary", "lanes": 2}]
    plain, said_plain = read(rows)
    scaled, said_scaled = read(rows, network_options={"capacity_factor": 0.9})
    assert scaled.link_capacity_pcu_h()[0] == pytest.approx(0.9 * plain.link_capacity_pcu_h()[0])
    assert not [s for s in said_plain + said_scaled if "jam density cannot carry" in s]
    _, said_too_high = read([{**rows[0], "lanes": 1, "capacity": 9000}])
    assert any("jam density cannot carry" in s for s in said_too_high), "a real one still warns"


def test_a_closed_road_stays_closed_through_gmns(tmp_path: Path):
    net = ms.examples.toy_network()
    a2 = net.link_ids().index("a2")
    closed = ms.network_edit(net, close=["a2"])
    ms.network_write_gmns(closed, str(tmp_path))
    back = ms.network_read_gmns(str(tmp_path))
    assert np.array_equal(np.asarray(back.link_drivable()), np.asarray(closed.link_drivable()))
    assert not np.asarray(back.link_drivable())[a2]
    ms.network_write_gmns(net, str(tmp_path / "open"))
    assert np.asarray(ms.network_read_gmns(str(tmp_path / "open")).link_drivable()).all()


def test_a_numpy_integer_names_a_link_by_its_index():
    net = ms.examples.toy_network()
    a2 = net.link_ids().index("a2")
    by_int = ms.network_edit(net, close=[a2])
    by_numpy = ms.network_edit(net, close=[np.int64(a2)])
    assert np.array_equal(np.asarray(by_numpy.link_drivable()), np.asarray(by_int.link_drivable()))
    with pytest.raises(ValueError, match="no link"):
        ms.network_edit(net, close=[np.int64(net.link_count)])


def test_a_link_map_of_a_run_without_road_traffic_is_drawn():
    pytest.importorskip("matplotlib")
    from openmobisim import viz

    net = ms.examples.toy_network()
    o, d = net.node_lonlat("W"), net.node_lonlat("M")
    rows = [("a", 0, o[0], o[1], d[0], d[1], 0, "x", None, "walk")]
    run = ms.Scenario.from_parts(
        net, rows, classes={"x": (True, True, False)}, equilibration="free_flow", link_bin_s=300
    ).run("walk", quiet=True)
    viz.map_link(run, size_cm=(10, 6), dpi=50)


def test_a_parking_table_reads_quoted_fields_and_blank_cells():
    text = (
        "parking_id,lon,lat,vehicle,capacity,name,hub_id,initial_occupancy\n"
        'p1,4.9,52.3,car,500,"P+R Noord, Amsterdam",H1,5\n'
        "p2,4.9,52.3,bike,80,,H2,\n"
    )
    rows = ms.parking_read_table(text).rows()
    assert rows["hub_id"] == ["H1", "H2"]
    assert list(rows["initial_occupancy"]) == [5, 0]


def test_a_case_asked_for_in_a_folder_is_looked_for_there_only(tmp_path, monkeypatch):
    name = next(iter(_cases._manifest()["cases"]))
    cached = tmp_path / "cache" / "openmobisim" / "bundle" / _cases.BUNDLE_VERSION / name
    cached.mkdir(parents=True)
    (cached / "case.json").write_text("{}")  # a copy in the cache, which must not be used
    monkeypatch.setenv("XDG_CACHE_HOME", str(tmp_path / "cache"))
    elsewhere = tmp_path / "elsewhere"
    elsewhere.mkdir()
    with pytest.raises(FileNotFoundError, match="elsewhere"):
        ms.examples.case(name, root=elsewhere)


REF = {"fingerprint": "f", "platform": "p", "version": "v", "completed": 100,
       "mean_trip_s": 600.0, "mode_share_pct": {"car": 100.0}}  # fmt: skip


def test_check_judges_the_reference_platform_to_the_bit():
    assert _cases._compare(dict(REF), REF) == ("same", [])
    status, notes = _cases._compare(dict(REF, mean_trip_s=600.6), REF)
    assert status == "different" and notes
    elsewhere = dict(REF, platform="q", mean_trip_s=600.6)  # another platform: the tolerance
    assert _cases._compare(elsewhere, REF)[0] == "within tolerance"
    assert _cases._compare(dict(REF, platform="q"), REF)[0] == "same numbers (another platform)"
    assert _cases._compare(dict(elsewhere, mean_trip_s=700.0), REF)[0] == "different"
    assert _cases._compare(dict(REF, fingerprint="g"), REF)[0] == "different"
