"""Every network parameter by name (S225, roadmap I-ae).

What is defended: the readers take ``network_options`` by name and refuse an unknown one with the
list; each kind of parameter acts (a class row, a multiplier, a layer speed); the network keeps
what it was built with, and a run's fingerprint follows it.
"""

import os

import numpy as np
import openmobisim as ms
import pytest

CAR = {"commuter": (True, False, False)}

LINKS = [
    {"from": "a", "to": "b", "length": 400.0, "class": "residential"},
    {"from": "b", "to": "a", "length": 400.0, "class": "residential"},
    {"from": "b", "to": "c", "length": 600.0, "class": "primary"},
    {"from": "c", "to": "b", "length": 600.0, "class": "primary"},
]
NODES = [
    {"node": "a", "lon": 4.900, "lat": 52.370},
    {"node": "b", "lon": 4.906, "lat": 52.370},
    {"node": "c", "lon": 4.915, "lat": 52.370},
]


def read(**options):
    return ms.network_read_table(LINKS, NODES, network_options=options or None)


def test_every_parameter_is_listed_with_its_shipped_value():
    shipped = ms.network_options()
    assert len(shipped) == 14 + 4 * 19
    assert shipped["residential.free_flow_km_h"] > 0 and shipped["capacity_factor"] == 1.0
    assert {"signal_cycle_s", "walk_km_h", "bike_mixed_km_h", "primary.lanes"} <= set(shipped)
    assert ms.network_options(read()) == shipped


def test_a_class_row_a_multiplier_and_a_layer_speed_each_act():
    base = read()
    speed = np.asarray(base.link_speed_km_h())
    slow = read(**{"residential.free_flow_km_h": 20.0})
    assert np.allclose(np.asarray(slow.link_speed_km_h())[:2], 20.0), "the residential links"
    assert np.allclose(np.asarray(slow.link_speed_km_h())[2:], speed[2:]), (
        "the primary ones are not"
    )
    half = read(capacity_factor=0.5)
    assert np.allclose(
        np.asarray(half.link_capacity_pcu_h()), 0.5 * np.asarray(base.link_capacity_pcu_h())
    )
    walking = read(walk_km_h=4.0)
    assert ms.network_options(walking)["walk_km_h"] == 4.0
    assert np.allclose(np.asarray(walking.layer("walk").link_speed_km_h()), 4.0)


def test_unknown_names_and_bad_values_are_refused():
    with pytest.raises(ValueError, match="no network option called.*residential.free_flow_km_h"):
        read(**{"residential.speed": 30.0})
    with pytest.raises(ValueError, match="positive"):
        read(capacity_factor=-1.0)
    with pytest.raises(ValueError, match="whole number from 1 to 20"):
        read(**{"primary.lanes": 2.5})


def test_a_runs_fingerprint_follows_the_network_parameters():
    def run(net, name):
        rows = [("t0", 0, 4.900, 52.370, 4.915, 52.370, 0, "commuter", None)]
        return ms.Scenario.from_parts(net, rows, class_defaults=CAR).run(name, quiet=True)

    plain, same, other = (
        run(read(), "no-1"),
        run(read(), "no-2"),
        run(read(capacity_factor=0.8), "no-3"),
    )
    assert plain.fingerprint == same.fingerprint
    assert other.fingerprint != plain.fingerprint


@pytest.mark.skipif("OPENMOBISIM_TEST_PBF" not in os.environ, reason="needs an OSM extract")
def test_an_osm_import_takes_the_options_too():
    path = os.environ["OPENMOBISIM_TEST_PBF"]
    net = ms.network_read_osm(path, network_options={"walk_km_h": 4.0, "bike_mixed_km_h": 12.0})
    assert ms.network_options(net)["walk_km_h"] == 4.0
    walk = np.asarray(net.layer("walk").link_speed_km_h())
    assert np.median(walk) == pytest.approx(4.0)
