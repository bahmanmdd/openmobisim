"""Scenario edits (S238, roadmap I-az): roads closed or narrowed, bike links upgraded.

A changed network, for a study to compare with the original.

What is defended: the original is left as it was; links keep their indices and ids, so per-link
results line up; a closed road is used by no route (a trip with no other way has no path, and is
counted); lanes scale capacity and storage, a capacity factor capacity alone; a bike link's new
infrastructure changes its speed and so the ride; the changes are in the run's fingerprint; and
wrong edits are refused. Hand values on the toy network: ``W → M`` by car takes ``a1`` and
``a2`` (80 s), the only road; ``W → N1`` by bike takes 144 s in mixed traffic.
"""

from __future__ import annotations

import numpy as np
import openmobisim as ms
import pytest


def trip(net: ms._core.Network, frm: str, to: str, mode: str) -> tuple:
    o, d = net.node_lonlat(frm), net.node_lonlat(to)
    return ("a", 0, o[0], o[1], d[0], d[1], 0, "x", None, mode)


def run(net: ms._core.Network, rows: list[tuple], run_id: str) -> ms.Run:
    return ms.Scenario.from_parts(
        net, rows, classes={"x": (True, True, False)}, equilibration="free_flow", flow_level=0
    ).run(run_id, quiet=True)


def test_a_closed_road_is_used_by_no_route_and_links_keep_their_indices() -> None:
    net = ms.examples.toy_network()
    closed = ms.network_edit(net, close=["a2"])
    assert closed.link_ids() == net.link_ids() and closed.link_count == net.link_count
    a2 = net.link_ids().index("a2")
    assert not np.asarray(closed.link_drivable())[a2] and np.asarray(net.link_drivable())[a2]
    base = run(net, [trip(net, "W", "M", "car")], "edit-base")
    shut = run(closed, [trip(net, "W", "M", "car")], "edit-closed")
    assert base.completion["completed"] == 1 and base.mean_travel_time_s == pytest.approx(80.0)
    assert shut.completion["no_feasible_path"] == 1, "a2 is the only road from W to M"
    assert shut.fingerprint != base.fingerprint
    assert ms.network_edit(net, close=[a2]).link_ids() == closed.link_ids(), "by index too"


def test_lanes_scale_capacity_and_storage_a_factor_capacity_alone() -> None:
    net = ms.examples.toy_network()
    ids = net.link_ids()
    m1, a1 = ids.index("m1"), ids.index("a1")
    cap, storage = np.asarray(net.link_capacity_pcu_h()), np.asarray(net.link_storage_pcu())
    assert np.asarray(net.link_lanes())[m1] == 2
    edited = ms.network_edit(net, lanes={"m1": 1}, capacity_factor={"a1": 0.5})
    cap2, storage2 = np.asarray(edited.link_capacity_pcu_h()), np.asarray(edited.link_storage_pcu())
    assert cap2[m1] == pytest.approx(cap[m1] / 2) and storage2[m1] == pytest.approx(storage[m1] / 2)
    assert np.asarray(edited.link_lanes())[m1] == 1
    assert cap2[a1] == pytest.approx(cap[a1] / 2) and storage2[a1] == pytest.approx(storage[a1])
    others = [i for i in range(net.link_count) if i not in (m1, a1)]
    assert np.array_equal(cap2[others], cap[others]), "nothing else changes"
    assert np.array_equal(np.asarray(net.link_capacity_pcu_h()), cap), "the original is kept"


def test_a_bike_link_s_new_infrastructure_changes_its_speed_and_the_ride() -> None:
    net = ms.examples.toy_network()
    streets = ["a1", "a2", "c1", "c2", "c3", "e2", "s1"]
    upgraded = ms.network_edit(net, bike_facility={s: "separated" for s in streets})
    bike, bike2 = net.layer("bike"), upgraded.layer("bike")
    a1 = bike.link_ids().index("a1")
    assert np.asarray(bike.link_infrastructure())[a1] == 0, "mixed"
    assert np.asarray(bike2.link_infrastructure())[a1] == 2, "separated"
    km_h = ms.network_options()
    assert np.asarray(bike2.link_speed_km_h())[a1] == pytest.approx(km_h["bike_dedicated_km_h"])
    base = run(net, [trip(net, "W", "N1", "bike")], "edit-bike-base")
    better = run(upgraded, [trip(net, "W", "N1", "bike")], "edit-bike-upgraded")
    assert base.mean_travel_time_s == pytest.approx(144.0)
    assert better.mean_travel_time_s < base.mean_travel_time_s
    assert better.fingerprint != base.fingerprint
    lane = ms.network_edit(net, bike_facility={"t1": "lane"})
    assert np.asarray(lane.layer("bike").link_infrastructure())[bike.link_ids().index("t1")] == 1


@pytest.mark.parametrize(
    ("edit", "message"),
    [
        ({"close": ["no-such-link"]}, "has no link 'no-such-link'"),
        ({"close": [999]}, "there is no link 999"),
        ({"lanes": {"m1": 0}}, "0 lanes"),
        ({"capacity_factor": {"a1": 0.0}}, "above 0"),
        ({"bike_facility": {"a1": "painted"}}, "separated"),
    ],
)
def test_wrong_edits_are_refused(edit: dict, message: str) -> None:
    with pytest.raises(ValueError, match=message):
        ms.network_edit(ms.examples.toy_network(), **edit)


def test_a_layer_is_edited_through_its_road_network() -> None:
    with pytest.raises(ValueError, match="road network"):
        ms.network_edit(ms.examples.toy_network().layer("bike"), close=["a1"])
