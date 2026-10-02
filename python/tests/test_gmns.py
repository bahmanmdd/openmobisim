"""GMNS files: a network written and read back is the same network, layers included.

A file written by another tool is read by GMNS's own rules (units, per-lane capacity,
undirected links, signals, uses).
"""

from __future__ import annotations

import os
from pathlib import Path

import numpy as np
import openmobisim as ms
import pytest

ROAD = (
    "link_from", "link_to", "link_class", "link_lanes", "link_length_m", "link_free_flow_s",
    "link_capacity_pcu_h", "link_storage_pcu", "link_roundabout", "node_signalised",
    "link_drivable",
)  # fmt: skip
LAYER = (
    "link_from",
    "link_to",
    "link_class",
    "link_speed_km_h",
    "link_length_m",
    "link_free_flow_s",
)


def same(a: object, b: object, names: tuple[str, ...]) -> list[str]:
    differ = []
    for name in names:
        x, y = np.asarray(getattr(a, name)(), float), np.asarray(getattr(b, name)(), float)
        if x.shape != y.shape or not np.allclose(x, y, rtol=1e-8, atol=1e-5):
            differ.append(name)
    return differ


def round_trip(net: object, folder: Path) -> None:
    counts = ms.network_write_gmns(net, str(folder))
    assert counts["road"] == net.link_count
    back = ms.network_read_gmns(str(folder))
    assert same(net, back, ROAD) == []
    for layer in ("bike", "walk"):
        names = LAYER + (("link_infrastructure",) if layer == "bike" else ())
        assert same(net.layer(layer), back.layer(layer), names) == [], layer


def test_the_toy_network_comes_back_link_for_link(tmp_path: Path) -> None:
    round_trip(ms.examples.toy_network(), tmp_path)
    for name in ("node.csv", "link.csv", "config.csv"):
        assert (tmp_path / "road" / name).exists() and (tmp_path / "bike" / name).exists()
    header = (tmp_path / "road" / "link.csv").read_text().splitlines()[0].split(",")
    assert header[:4] == ["link_id", "from_node_id", "to_node_id", "directed"]
    assert "capacity" in header and "geometry" in header


@pytest.mark.skipif(not os.environ.get("OPENMOBISIM_TEST_PBF"), reason="needs an OSM extract")
def test_an_osm_network_and_its_own_layers_come_back(tmp_path: Path) -> None:
    round_trip(ms.network_read_osm(os.environ["OPENMOBISIM_TEST_PBF"]), tmp_path)


def write(folder: Path, name: str, text: str) -> None:
    folder.mkdir(parents=True, exist_ok=True)
    (folder / name).write_text(text)


def other_tool(folder: Path) -> None:
    """A GMNS network as another tool writes it.

    One folder, miles and mph, an undirected road, per-lane capacity, a signal and a footpath.
    """
    write(folder, "config.csv", "dataset_name,long_length,speed,crs\nsample,mi,mph,EPSG:4326\n")
    write(folder, "node.csv", "node_id,x_coord,y_coord,ctrl_type\n"
          "a,4.900,52.370,\nb,4.910,52.370,signal\nc,4.920,52.370,\n")  # fmt: skip
    write(folder, "link.csv", "link_id,from_node_id,to_node_id,directed,length,free_speed,"
          "lanes,capacity,facility_type,allowed_uses\n"
          "1,a,b,false,0.4225,30,2,900,secondary,auto\n"
          "2,b,c,true,0.4225,,,,,walk\n")  # fmt: skip


def test_a_file_from_another_tool_is_read_by_gmns_rules(tmp_path: Path) -> None:
    other_tool(tmp_path)
    net = ms.network_read_gmns(str(tmp_path))
    assert net.link_count == 3, "the undirected road is two links, the path one"
    cls = net.link_class()
    names = np.array(["secondary" if c == 6 else "footway" if c == 15 else str(c) for c in cls])
    road = names == "secondary"
    assert road.sum() == 2 and (names == "footway").sum() == 1
    length = net.link_length_m()
    assert np.allclose(length[road], 0.4225 * 1609.344)
    assert np.allclose(net.link_speed_km_h()[road], 30 * 1.609344)
    # 900 per lane, two lanes: 1 800 in all (the table reader's capacity is a link's total).
    assert np.allclose(net.link_capacity_pcu_h()[road], 1800.0)
    assert net.node_signalised().sum() == 1
    assert not net.link_drivable()[~road].any()
    # No layers in the folder: derived from the road network.
    assert net.layer("bike").link_count > 0 and net.layer("walk").link_count > 0


def test_what_is_not_read_is_refused(tmp_path: Path) -> None:
    with pytest.raises(ValueError, match="holds no GMNS network"):
        ms.network_read_gmns(str(tmp_path))
    other_tool(tmp_path)
    write(tmp_path, "config.csv", "long_length,speed,crs\nm,kmh,EPSG:28992\n")
    with pytest.raises(ValueError, match="EPSG:4326"):
        ms.network_read_gmns(str(tmp_path))
    write(tmp_path, "config.csv", "long_length,speed\nfurlong,kmh\n")
    with pytest.raises(ValueError, match="long_length"):
        ms.network_read_gmns(str(tmp_path))
    with pytest.raises(ValueError, match="road network"):
        ms.network_write_gmns(ms.examples.toy_network().layer("bike"), str(tmp_path / "x"))


def test_a_short_slow_link_such_as_a_ferry_keeps_its_time(tmp_path: Path) -> None:
    # A ferry's speed folds in its expected wait: 16 m in 6 minutes. Rounded lengths or speeds
    # move such a link's time by milliseconds (they did on Amsterdam's ferries); written in full,
    # it comes back within a tenth of a millisecond.
    node = "node_id,x_coord,y_coord\na,4.9000,52.3700\nb,4.9002,52.3701\n"
    km_h = 16.204408120996227 * 3.6 / 360.0
    for layer, uses in (("road", "auto"), ("bike", "bike"), ("walk", "walk")):
        write(tmp_path / "in" / layer, "node.csv", node)
        write(tmp_path / "in" / layer, "link.csv",
              "link_id,from_node_id,to_node_id,directed,length,free_speed,facility_type,allowed_uses\n"
              f"1,a,b,false,16.204408120996227,{km_h if layer != 'road' else 30},"
              f"{'ferry' if layer != 'road' else 'residential'},{uses}\n")  # fmt: skip
    first = ms.network_read_gmns(str(tmp_path / "in"))
    assert np.allclose(first.layer("bike").link_free_flow_s(), 360.0, rtol=1e-12)
    ms.network_write_gmns(first, str(tmp_path / "out"))
    back = ms.network_read_gmns(str(tmp_path / "out"))
    for layer in ("bike", "walk"):
        x = np.asarray(back.layer(layer).link_free_flow_s())
        assert np.abs(x - 360.0).max() < 1e-4, (layer, x)
