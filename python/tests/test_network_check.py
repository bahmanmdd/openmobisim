"""``network_check``: a network a user brings, checked before it is run.

A clean grid passes; the toy network's one-way spokes fail connectivity, as they are meant to
(it is a hand-built test case, not a city); a unit mistake in a file (lengths in kilometres
read as metres, speeds ten times too high) is flagged; a self-loop fails.
"""

from __future__ import annotations

import csv
import os
from pathlib import Path

import openmobisim as ms
import pytest


def statuses(report: dict) -> dict[tuple[str, str], str]:
    return {(c["layer"], c["check"]): c["status"] for c in report["checks"]}


def test_a_clean_grid_passes_on_every_layer() -> None:
    report = ms.network_check(ms.examples.manhattan_grid(5, 200.0, True))
    assert report["verdict"] == "pass", [c for c in report["checks"] if c["status"] != "pass"]
    layers = {c["layer"] for c in report["checks"]}
    assert layers == {"road", "bike", "walk"}
    assert report["summary"]["road"]["drivable_links"] == 80
    assert set(report["summary"]["bike"]) >= {"separated_share", "lane_share"}
    only_road = ms.network_check(ms.examples.manhattan_grid(5, 200.0, True), layers=False)
    assert {c["layer"] for c in only_road["checks"]} == {"road"}


def test_the_toy_networks_one_way_spokes_fail_connectivity() -> None:
    report = ms.network_check(ms.examples.toy_network())
    s = statuses(report)
    assert report["verdict"] == "fail"
    assert s[("road", "strongly connected")] == "fail"
    assert s[("walk", "strongly connected")] == "pass", "walkers go both ways"


def edit(path: Path, column: str, scale: float) -> None:
    with path.open(newline="") as f:
        rows = list(csv.DictReader(f))
    for row in rows:
        row[column] = str(float(row[column]) * scale)
    with path.open("w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=list(rows[0]), lineterminator="\n")
        w.writeheader()
        w.writerows(rows)


def test_unit_mistakes_in_a_file_are_flagged(tmp_path: Path) -> None:
    ms.network_write_gmns(ms.examples.manhattan_grid(6, 200.0, False), tmp_path, layers=False)
    edit(tmp_path / "road" / "link.csv", "length", 0.001)
    s = statuses(ms.network_check(ms.network_read_gmns(tmp_path / "road")))
    assert s[("road", "length not below the straight line")] == "warn"
    assert s[("road", "very short links")] == "warn"
    edit(tmp_path / "road" / "link.csv", "length", 1000.0)
    edit(tmp_path / "road" / "link.csv", "free_speed", 10.0)
    s = statuses(ms.network_check(ms.network_read_gmns(tmp_path / "road")))
    assert s[("road", "length not below the straight line")] == "pass"
    assert s[("road", "speeds plausible by class")] == "warn"


def test_a_self_loop_fails_and_a_layer_is_refused(tmp_path: Path) -> None:
    (tmp_path / "node.csv").write_text("node_id,x_coord,y_coord\na,4.90,52.37\nb,4.91,52.37\n")
    (tmp_path / "link.csv").write_text(
        "link_id,from_node_id,to_node_id,directed,length,free_speed,facility_type\n"
        "1,a,b,false,700,50,residential\n2,a,a,true,50,50,residential\n"
    )
    net = ms.network_read_gmns(tmp_path)
    s = statuses(ms.network_check(net))
    assert s[("road", "no self-loops")] == "fail" and s[("bike", "no self-loops")] == "fail"
    with pytest.raises(ValueError, match="road network"):
        ms.network_check(net.layer("bike"))


@pytest.mark.skipif(not os.environ.get("OPENMOBISIM_TEST_PBF"), reason="needs an OSM extract")
def test_an_osm_import_is_connected_on_every_layer() -> None:
    report = ms.network_check(ms.network_read_osm(os.environ["OPENMOBISIM_TEST_PBF"]))
    assert report["verdict"] != "fail", [c for c in report["checks"] if c["status"] == "fail"]
    s = statuses(report)
    for layer in ("road", "bike", "walk"):
        assert s[(layer, "strongly connected")] == "pass"


def _junction_with_a_stop_line(both_ways: bool):
    """A four-arm junction J with a stop line S 10 m west of it, on a two-way street (S282, H6)."""
    step = 10.0 / 111_320.0 / 0.69  # 10 m of longitude at 46° N, in degrees (cos 46° ≈ 0.69)
    nodes = [
        {"id": "J", "x": 4.80, "y": 46.0},
        {"id": "S", "x": 4.80 - step, "y": 46.0},
        {"id": "W", "x": 4.80 - 20 * step, "y": 46.0},
        {"id": "E", "x": 4.80 + 20 * step, "y": 46.0},
        {"id": "N", "x": 4.80, "y": 46.0 + 0.0018},
        {"id": "X", "x": 4.80, "y": 46.0 - 0.0018},
    ]
    if both_ways:
        nodes[1]["signalised"] = True  # every link into S stops: the far side too
    else:
        nodes[1]["signal_location"] = True  # S stands here; its links say what it stops
    links = []
    for a, b in (("W", "S"), ("S", "J"), ("J", "E"), ("J", "N"), ("J", "X")):
        for f, t in ((a, b), (b, a)):
            stop = (not both_ways) and (f, t) == ("W", "S")
            links.append({"id": f"{f}{t}", "from": f, "to": t, "class": "residential",
                          "signalised": stop})  # fmt: skip
    return ms.network_read_table(links, nodes)


def test_a_stop_line_that_stops_both_directions_is_flagged() -> None:
    # S282, H6 (DR-129): the reader bug of S265 (a stop line's direction ignored) went unnoticed
    # for months; this check would have shown it.
    check = ("road", "stop lines stop one direction")
    assert statuses(ms.network_check(_junction_with_a_stop_line(both_ways=True)))[check] == "warn"
    assert statuses(ms.network_check(_junction_with_a_stop_line(both_ways=False)))[check] == "pass"
