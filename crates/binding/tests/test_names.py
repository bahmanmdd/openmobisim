"""The names beside a timetable reaching the run's pages: a hand GTFS feed of one tram, line "1"
(route_type 0), from the stop "Markt" to the stop "Station" 1 000 m east, at 08:30 to 08:40, and two
transit trips between the two stops' places at 08:00.

By hand: the map holds the two stops by name, the kinds of service with tram the third (index 2),
the line "1" of kind tram, one hop Markt -> Station of kind tram ridden by 2 travellers, 2 boardings
at Markt; the pair's places are the stops themselves ("Markt", "Station", within 60 m); its journey
is named by its line, "1". The summary's transit map draws the hop in the tram's colour (class
k-k2) with a legend entry "tram". In Chrome, the hop, clicked on its stroke, opens the panel
"tram · Markt → Station" with 2 passengers on board; the stop Markt, hovered, reads "stop Markt"
with 2 boardings; a click on the stop closes the link panel (no pair is open); the legend lists
"Transit lines" and "tram".
"""

import base64
import json
import re
import struct
import zipfile
import zlib
from pathlib import Path

import pytest

import openmobisim as oms
from test_defaults import END, LINK_COLUMNS, START, lon_lat, table
from test_map import chrome

STOPS = {"S1": ("Markt", 0.0), "S2": ("Station", 1_000.0)}


def tram_run(tmp_path):
    """The run of the module's header, its map and summary written; their texts and the map's path."""
    positions = {n: lon_lat(x, 0.0) for n, x in (("a", 0.0), ("b", 1_000.0))}
    nodes = {"node_id": list(positions), "lon_deg": [p[0] for p in positions.values()],
             "lat_deg": [p[1] for p in positions.values()]}
    links = table(LINK_COLUMNS, [("ab", "a", "b", "walk", 1_000.0, None, None, None),
                                 ("ba", "b", "a", "walk", 1_000.0, None, None, None)])
    network = oms.Network.from_tables(nodes, links)
    stop_rows = "".join(f"{s},{name},{lon_lat(x, 0.0)[1]!r},{lon_lat(x, 0.0)[0]!r}\n"
                        for s, (name, x) in STOPS.items())
    files = {
        "agency.txt": "agency_id,agency_name,agency_url,agency_timezone\na,A,https://example.org,Europe/Amsterdam\n",
        "routes.txt": "route_id,agency_id,route_short_name,route_type\nr,a,1,0\n",
        "stops.txt": "stop_id,stop_name,stop_lat,stop_lon\n" + stop_rows,
        "trips.txt": "route_id,service_id,trip_id\nr,weekdays,t1\n",
        "stop_times.txt": "trip_id,arrival_time,departure_time,stop_id,stop_sequence\n"
                          "t1,08:30:00,08:30:00,S1,1\nt1,08:40:00,08:40:00,S2,2\n",
        "calendar.txt": "service_id,monday,tuesday,wednesday,thursday,friday,saturday,sunday,"
                        "start_date,end_date\nweekdays,1,1,1,1,1,0,0,20261102,20261106\n",
    }
    feed = tmp_path / "gtfs.zip"
    with zipfile.ZipFile(feed, "w") as z:
        for name, text in files.items():
            z.writestr(name, text)
    transit = oms.Transit.from_gtfs(feed, network)
    trips = table(["trip_id", "origin_lon_deg", "origin_lat_deg", "destination_lon_deg",
                   "destination_lat_deg", "departure_s", "class_id", "modes"],
                  [(i, *positions["a"], *positions["b"], START, "all", "transit") for i in ("p", "q")])
    demand = oms.Demand.from_table(trips, ["all"], network, transit=transit)
    result = oms.run(network, demand, transit, window=(START, END), out=tmp_path / "run")
    page = Path(result.map(tmp_path / "tram.html"))
    summary = Path(result.summary(tmp_path / "tram-summary.html")).read_text(encoding="utf-8")
    return page, summary


def decoded(page):
    """The map's table of contents and arrays, read from the file."""
    html = page.read_text(encoding="utf-8")
    meta = json.loads(re.search(r'<script id="meta" type="application/json">(.*?)</script>', html, re.S).group(1))
    raw = zlib.decompress(base64.b64decode(re.search(r'<script id="blob" type="text/plain">(.*?)</script>', html, re.S).group(1).strip()))
    kinds = {"u8": "B", "u32": "I", "i32": "i", "f32": "f"}
    arrays = {a["n"]: list(struct.unpack_from(f"<{a['c']}{kinds[a['t']]}", raw, a["o"])) for a in meta["arrays"]}
    return meta, arrays


def test_names_and_kinds_reach_both_pages(tmp_path):
    page, summary = tram_run(tmp_path)
    meta, a = decoded(page)
    assert meta["stops"] == ["Markt", "Station"]
    assert meta["kinds"][2] == "tram" and meta["lines"] == ["1"] and meta["line_kinds"] == [2]
    assert (a["ta"], a["tb"], a["tk"], a["tl"], a["tsb"]) == ([0], [1], [2], [2], [2, 0])
    assert [[meta["place_names"][o], meta["place_names"][d]] for o, d in zip(a["qpo"], a["qpd"])] == [["Markt", "Station"]]
    rides = [g for g, k in enumerate(a["lk"]) if k == 3]
    assert [a["lline"][g] for g in rides] == [0]
    transit = summary[summary.index('data-k="map.transit"'):]
    assert 'class="k-k2"' in transit and 'k-k2"></span>tram' in transit


def test_a_hop_and_a_stop_clicked_and_the_legends_kinds(tmp_path):
    binary = chrome()
    if binary is None:
        pytest.skip("no Chrome or Chromium binary")
    from cdp import Page

    page_path, _ = tram_run(tmp_path)
    _, a = decoded(page_path)
    xs, ys = [v * 0.1 for v in a["tsx"]], [v * 0.1 for v in a["tsy"]]
    cx, cy, mpp = (xs[0] + xs[1]) / 2, (ys[0] + ys[1]) / 2, 2.0
    screen = [(750 + (x - cx) / mpp, 475 - (y - cy) / mpp) for x, y in zip(xs, ys)]
    # The hop's stroke: 7.2 px wide (1.2 + 6 √(2 / 2)), its centre 4.1 px to the right of its travel.
    (x0, y0), (x1, y1) = screen
    length = ((x1 - x0) ** 2 + (y1 - y0) ** 2) ** 0.5
    nx, ny = -(y1 - y0) / length, (x1 - x0) / length
    hop = ((x0 + x1) / 2 + nx * 4.1, (y0 + y1) / 2 + ny * 4.1)
    page = Page(binary, tmp_path / "profile", page_path.as_uri() + f"#ly=t&z={cx:.1f},{cy:.1f},{mpp:.3f}")
    try:
        page.wait("document.body && document.body.dataset.state === 'ready'")
        legend = " ".join(page.run("document.getElementById('legendbody').innerText").split())
        assert legend.startswith("Transit lines tram width = passengers on board"), legend
        page.click(*hop)
        panel = " ".join(page.run("document.getElementById('linkinfo').innerText").split())
        assert panel.startswith("tram · Markt → Station × 2 passengers on board"), panel
        page.move(*screen[0])
        tip = " ".join(page.run("document.getElementById('tip').innerText").split())
        assert tip == "stop Markt Boardings 2 over the run", tip
        page.click(*screen[0])
        assert page.run("document.getElementById('linkinfo').hidden")
    finally:
        page.close()
