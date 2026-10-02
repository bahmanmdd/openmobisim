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
