"""Network.tables(): a network gives back the two tables Network.from_tables reads.

By hand: a network built from tables of longitude/latitude (three nodes near Delft, a car street
with a bend, the same street for bikes, a footpath) gives back the nodes at the positions given
(to 1e-9 degree, a tenth of a millimetre: they pass through the projection and back) and each
link's row as given, the car fields None on a link without cars, the shape's points in degrees.
A network built in metres gives its x and y back as they are. A property, on Delft read from the
extract of crates/io-osm in version 1's box: the tables build a network whose own tables are the
same tables (the reading is a fixed point), with the link count the reader reports.
For bikes: a bike link comes back as a bike link, with its length, shape and class, no car field.
"""

from pathlib import Path

import pytest

import openmobisim as oms

OSM = Path(__file__).resolve().parents[2] / "io-osm" / "tests" / "fixtures" / "delft" / "network.osm.pbf"
REGION = (4.315, 51.965, 4.410, 52.035)

NODES = {"node_id": ["a", "b", "c"], "lon_deg": [4.36, 4.37, 4.365], "lat_deg": [52.0, 52.0, 52.01]}
LINKS = {
    "link_id": ["bike:ab", "car:ab", "car:ba", "walk:bc"],
    "upstream": ["a", "a", "b", "b"],
    "downstream": ["b", "b", "a", "c"],
    "modes": ["bike", "car", "car", "walk"],
    "length_m": [700.0, 700.0, 690.5, 1200.0],
    "lanes": [None, 2, 1, None],
    "car_speed_km_h": [None, 30.0, 50.0, None],
    "capacity_veh_h": [None, 1620.0, 810.0, None],
    "shape": ["4.365 52.001", "4.365 52.001", None, None],
    "road_class": ["tertiary", "tertiary", "tertiary", None],
}


def rows(table):
    names = list(table)
    return {r[0]: dict(zip(names, r)) for r in zip(*(table[n] for n in names))}


def points(text):
    return [tuple(float(v) for v in p.split()) for p in text.split(";")]


def test_a_network_from_tables_gives_its_rows_back():
    nodes, links = oms.Network.from_tables(NODES, LINKS).tables()
    assert list(nodes) == ["node_id", "lon_deg", "lat_deg"]
    assert list(links) == list(LINKS)
    got = rows(nodes)
    for node, lon, lat in zip(NODES["node_id"], NODES["lon_deg"], NODES["lat_deg"]):
        assert got[node]["lon_deg"] == pytest.approx(lon, abs=1e-9)
        assert got[node]["lat_deg"] == pytest.approx(lat, abs=1e-9)
    got, given = rows(links), rows(LINKS)
    assert sorted(got) == sorted(given)
    for link, row in given.items():
        for name in ("upstream", "downstream", "modes", "length_m", "lanes", "car_speed_km_h",
                     "capacity_veh_h", "road_class"):
            assert got[link][name] == row[name], (link, name)
    for link in ("bike:ab", "car:ab"):
        (lon, lat), = points(got[link]["shape"])
        assert (lon, lat) == pytest.approx((4.365, 52.001), abs=1e-9)
    assert got["car:ba"]["shape"] is None and got["walk:bc"]["shape"] is None


def test_a_network_in_metres_gives_its_metres_back():
    nodes = {"node_id": ["a", "b"], "x_m": [0.0, 300.25], "y_m": [10.0, -40.5]}
    links = {name: [LINKS[name][k] for k in (1, 2)] for name in LINKS}
    links["shape"] = ["100 5.5;200 -7", None]
    got_nodes, got_links = oms.Network.from_tables(nodes, links).tables()
    assert got_nodes == nodes
    assert got_links["shape"] == ["100 5.5;200 -7", None]
    assert got_links["length_m"] == [700.0, 690.5]


def test_delft_read_from_its_extract_is_the_network_of_its_own_tables():
    network = oms.Network.from_osm(OSM, region=REGION)
    nodes, links = network.tables()
    again = oms.Network.from_tables(nodes, links)
    assert again.tables() == (nodes, links)
    assert again.repairs == {}
    modes = links["modes"]
    assert set(modes) == {"car", "bike", "walk"}
    assert any(m == "bike" and c == "residential" for m, c in zip(modes, links["road_class"]))
    car = [k for k, m in enumerate(modes) if m == "car"]
    assert all(links["lanes"][k] >= 1 and links["capacity_veh_h"][k] > 0 for k in car)
    assert all(links["lanes"][k] is None for k, m in enumerate(modes) if m != "car")
