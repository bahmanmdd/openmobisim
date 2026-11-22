"""A link's shape and road class through the tables (the optional columns `shape` and `road_class`
of the links table), for drawing only.

The hand run (test_hand_run.py) with ab a cycleway, st1 a primary road bent through (750, 300) m
(the shape "750 300"), st2 a residential street:
1. Nothing a run reads changes: the network's and the demand's fingerprints are the plain hand
   run's, and so are its trips.
2. The map holds the shape and the classes: st1's one point 7 500 and 3 000 decimetres from a
   (the origin, the nodes' least x and y); the drawing classes by version 1's order (cycleway 16,
   primary 4, residential 11); each link's name among the sorted names. A file and the same table
   in memory give the same map data.
3. In longitude/latitude the shape's point is projected as the nodes are: a point at node m's
   position is held at m's decimetres.
4. The summary's maps follow the shape. By hand: the frame 2 000 units wide, a margin of 20, the
   network 2 500 m wide and 300 m high, so 0.784 units a metre and 276 units high (300 x 0.784 +
   40, up); a point (x, y) m at (20 + 0.784 x, 20 + 0.784 (300 - y)): a (20, 255.2), the bend
   (608, 20), b (1 196, 255.2), c (1 980, 255.2). The base network: ab and st1 (two lines between a
   and b), st2. St1's car ribbon, 8 wide, moved 5 units to the right of its travel: at a and b
   along their piece's normal, (0.371, 0.928) and (-0.371, 0.928) (the pieces (588, -235.2) and
   (588, 235.2) long), to (21.9, 259.8) and (1 194.1, 259.8); at the bend along the two normals'
   sum, (0, 1.857), by 5 x min(2, 2 / 1.857) = 5.39, to (608, 25.4).
5. A shape that is not points "x y;x y", with a point not finite, or not text (3.5), draws
   the link straight (the map's data that of st1 without a shape), counted (`shapes_unread`,
   naming st1) and warned once; a road class that is not text (True) is none, the link kept
   (`road_classes_unread`); a column the table does not read (`shapes`) is ignored, its shape absent,
   counted (`columns_ignored`, naming it) and warned once; an empty `road_class` is no class.

For bikes: ab, the cyclists' path, is classed a cycleway in the map's data, as a road is classed.
The pages are checked as files; their behaviour in a browser is not tested automatically.
"""

import base64
import json
import re
import struct
import tomllib
import zlib
from pathlib import Path

import pyarrow as pa
import pyarrow.parquet as pq
import pytest

import openmobisim as oms
from test_hand_run import START, hand_run, hand_tables


def drawn_tables():
    nodes, links, trips = hand_tables()
    links = {**links, "shape": [None, "750 300", None], "road_class": ["cycleway", "primary", "residential"]}
    return nodes, links, trips


def decoded(page):
    """The map's table of contents and arrays, read from the file."""
    html = Path(page).read_text(encoding="utf-8")
    meta = json.loads(re.search(r'<script id="meta" type="application/json">(.*?)</script>', html, re.S).group(1))
    raw = zlib.decompress(base64.b64decode(re.search(r'<script id="blob" type="text/plain">(.*?)</script>', html, re.S).group(1).strip()))
    kinds = {"u8": "B", "u32": "I", "i32": "i", "f32": "f"}
    arrays = {a["n"]: list(struct.unpack_from(f"<{a['c']}{kinds[a['t']]}", raw, a["o"])) for a in meta["arrays"]}
    return meta, arrays


def fingerprints(record):
    with open(record, "rb") as f:
        r = tomllib.load(f)
    return r["network"]["fingerprint"], r["demand"]["fingerprint"]


def test_a_shape_and_classes_change_nothing_a_run_reads(tmp_path):
    plain = hand_run(*hand_tables(), tmp_path / "plain")
    drawn = hand_run(*drawn_tables(), tmp_path / "drawn")
    assert fingerprints(drawn.record) == fingerprints(plain.record)
    assert pq.read_table(drawn.trips).equals(pq.read_table(plain.trips))


def test_the_map_holds_the_shape_and_classes_from_a_table_and_a_file_alike(tmp_path):
    nodes, links, trips = drawn_tables()
    memory = hand_run(nodes, links, trips, tmp_path / "memory").map(tmp_path / "memory.html")
    meta, a = decoded(memory)
    assert (a["vs"], a["dx"], a["dy"]) == ([0, 0, 1, 1], [7_500], [3_000])
    assert a["cls"] == [16, 4, 11]
    assert meta["road_classes"] == ["cycleway", "primary", "residential"] and a["rc"] == [0, 1, 2]
    file = tmp_path / "links.parquet"
    pq.write_table(pa.table(links), file)
    from_file = hand_run(nodes, file, trips, tmp_path / "file").map(tmp_path / "file.html")
    assert decoded(from_file)[1] == a
    assert decoded(from_file)[0]["road_classes"] == meta["road_classes"]


def test_a_longitude_latitude_shape_is_projected_as_the_nodes_are(tmp_path):
    nodes = {"node_id": ["a", "b", "m"], "lon_deg": [4.36, 4.38, 4.37], "lat_deg": [52.0, 52.0, 52.003]}
    links = {"link_id": ["ab"], "upstream": ["a"], "downstream": ["b"], "modes": ["walk"],
             "length_m": [1_400.0], "lanes": [None], "car_speed_km_h": [None], "capacity_veh_h": [None],
             "shape": ["4.37 52.003"]}
    trips = {"trip_id": ["w"], "origin_lon_deg": [4.36], "origin_lat_deg": [52.0], "destination_lon_deg": [4.38],
             "destination_lat_deg": [52.0], "departure_s": [START], "class_id": ["all"], "modes": ["walk"]}
    network = oms.Network.from_tables(nodes, links)
    demand = oms.Demand.from_table(trips, ["all"], network)
    result = oms.run(network, demand, window=(START, START + 3_600), out=tmp_path / "run")
    meta, a = decoded(result.map(tmp_path / "lonlat.html"))
    assert meta["road_classes"] is None  # no class: drawn by capacity
    assert (a["dx"], a["dy"]) == ([a["nx"][2]], [a["ny"][2]])


def test_the_summary_maps_follow_the_shape(tmp_path):
    result = hand_run(*drawn_tables(), tmp_path / "run")
    html = Path(result.summary(tmp_path / "summary.html")).read_text(encoding="utf-8")
    assert '<path id="net" d="M20 255L1196 255M20 255L608 20L1196 255M1196 255L1980 255"/>' in html
    car = html[html.index('data-k="map.car"'):]
    car = car[:car.index("</svg>")]
    assert '<svg viewBox="0 0 2000 276" class="map" data-k="map.car"' in html
    assert '<path class="k-b0" stroke-width="8" d="M22 260L608 25L1194 260"/>' in car, car


def straight_map(tmp_path, nodes, links, trips, name):
    """The map's data of the hand run on `links`."""
    return decoded(hand_run(nodes, links, trips, tmp_path / name).map(tmp_path / f"{name}.html"))[1]


@pytest.mark.parametrize("shape", ["750", "750 300;a b", "750 nan", 3.5])
def test_a_shape_not_points_is_drawn_straight_counted_and_warned(tmp_path, shape):
    nodes, links, trips = drawn_tables()
    links["shape"] = [None, None, None]
    want = straight_map(tmp_path, nodes, links, trips, "straight")
    links["shape"] = [None, shape, None]
    with pytest.warns(UserWarning) as caught:
        network = oms.Network.from_tables(nodes, links)
    shown = [str(w.message) for w in caught if "Network.from_tables" in str(w.message)]
    assert shown == ["Network.from_tables: 1 shapes that cannot be read as points of the nodes' "
                     "kind, finite and projectable: drawn straight (shapes_unread; first: st1)"]
    assert network.repairs == {"shapes_unread": 1}
    assert straight_map(tmp_path, nodes, links, trips, "bad") == want


def test_a_road_class_that_is_not_text_is_none_and_the_link_kept(tmp_path):
    nodes, links, trips = drawn_tables()
    links["road_class"] = ["cycleway", True, "residential"]
    with pytest.warns(UserWarning, match=re.escape("(road_classes_unread; first: st1)")):
        network = oms.Network.from_tables(nodes, links)
    assert network.repairs == {"road_classes_unread": 1}
    meta, _ = decoded(hand_run(nodes, links, trips, tmp_path / "run").map(tmp_path / "map.html"))
    assert meta["road_classes"] is None  # st1 without a class: drawn by capacity, as with ""


def test_a_column_the_links_table_does_not_read_is_ignored_counted_and_warned(tmp_path):
    nodes, links, trips = drawn_tables()
    links["shape"] = [None, None, None]
    want = straight_map(tmp_path, nodes, links, trips, "straight")
    links["shapes"] = ["", "750 300", ""]
    del links["shape"]
    with pytest.warns(UserWarning, match=re.escape("columns_ignored; first: links: shapes)")):
        network = oms.Network.from_tables(nodes, links)
    assert network.repairs == {"columns_ignored": 1}
    assert straight_map(tmp_path, nodes, links, trips, "misspelt") == want


def test_an_empty_class_is_none(tmp_path):
    nodes, links, trips = drawn_tables()
    links["road_class"] = ["", "primary", "residential"]
    meta, _ = decoded(hand_run(nodes, links, trips, tmp_path / "run").map(tmp_path / "map.html"))
    assert meta["road_classes"] is None  # one link without a class: drawn by capacity
