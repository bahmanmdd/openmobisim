"""The select-link analysis (``Run.link_select``) and its page (``viz.map_link_select``).

What is defended, each from a hand count or a property, never from an earlier run:

* on a 5 x 5 grid of 200 m blocks, where a trip along one row of nodes has one shortest route
  (any other is at least two blocks longer), two pairs sharing a link are listed with their
  hand-counted trips and shares and a third pair on another row is not; a departure window
  that leaves one pair's trips out drops it; a bike trip is listed under the bike on the bike
  layer's link and not on the road's;
* conservation: on a grid where every trip finishes, the trips the table counts on a link are
  the crossings the run's per-link results count there, on the road, bike and walk layers;
* a vehicle that re-routed counts on the links it actually took, not on those it left;
* the page is one self-contained file holding the users' paths and groups.
"""

from __future__ import annotations

import base64
import json
import re
import zlib

import numpy as np
import openmobisim as ms
import pytest
from openmobisim import _link_select, viz

DRIVES_ONLY = {"car_access_s": 0.0, "car_egress_s": 0.0, "bike_access_s": 0.0, "bike_egress_s": 0.0}
EIGHT = 8 * 3600


def grid():
    return ms.examples.manhattan_grid(5, 200.0, False)


def lonlat(net, i, j):
    return ms._core.grid_node_lonlat(net, i, j)


def trip(net, who, frm, to, departure, mode):
    o, d = lonlat(net, *frm), lonlat(net, *to)
    return (who, 0, o[0], o[1], d[0], d[1], departure, "everyone", None, mode)


def link_between(layer, a, b):
    """The link of `layer` from node `a` to node `b`."""
    frm, to = np.asarray(layer.link_from()), np.asarray(layer.link_to())
    found = np.nonzero((frm == a) & (to == b))[0]
    assert len(found) == 1
    return int(found[0])


@pytest.fixture(scope="module")
def rows_run():
    """Three pairs on a grid; A and B share the links from (1,0) to (3,0), C shares none.

    Pair A (0,0)->(4,0): three cars and a bike at 8:00. Pair B (1,0)->(3,0): two cars at 8:30.
    Pair C (0,4)->(4,4): two cars at 8:00, on another row.
    """
    net = grid()
    rows = [trip(net, f"a{k}", (0, 0), (4, 0), EIGHT, "car") for k in range(3)]
    rows += [trip(net, "a3", (0, 0), (4, 0), EIGHT, "bike")]
    rows += [trip(net, f"b{k}", (1, 0), (3, 0), EIGHT + 1800, "car") for k in range(2)]
    rows += [trip(net, f"c{k}", (0, 4), (4, 4), EIGHT, "car") for k in range(2)]
    run = ms.Scenario.from_parts(
        network=net,
        demand=rows,
        classes={"everyone": (True, True, False)},
        link_bin_s=300,
        mode_options=DRIVES_ONLY,
    ).run("link-select-rows", overwrite=True, quiet=True)
    zones = {
        "A": lonlat(net, 0, 0),
        "A'": lonlat(net, 4, 0),
        "B": lonlat(net, 1, 0),
        "B'": lonlat(net, 3, 0),
        "C": lonlat(net, 0, 4),
        "C'": lonlat(net, 4, 4),
    }
    return net, run, zones


def road_link(net, a, b):
    node = lambda p: net.node_nearest(*lonlat(net, *p))  # noqa: E731
    return link_between(net, node(a), node(b))


def test_two_pairs_sharing_a_link_are_listed_with_their_hand_counted_trips(rows_run):
    net, run, zones = rows_run
    link = road_link(net, (1, 0), (2, 0))
    t = run.link_select(link, zones=zones)
    # A's three cars and B's two cross it; C is on another row. Shares of the link's five
    # users: 3/5 and 2/5. Of A's four travellers three drove (3/4); both of B's did.
    assert t["origin"] == ["A", "B"] and t["destination"] == ["A'", "B'"]
    assert t["mode"] == ["car", "car"]
    assert t["trips"].tolist() == [3, 2] and t["people"].tolist() == [3.0, 2.0]
    assert np.allclose(t["share_link"], [3 / 5, 2 / 5])
    assert np.allclose(t["share_pair"], [3 / 4, 1.0])
    # Without zones, the pairs are road nodes: the same two groups, by node.
    plain = run.link_select(link)
    assert plain["trips"].tolist() == [3, 2] and len(set(plain["origin"])) == 2
    # The link from (0,0) to (1,0) is A's alone; the reverse direction nobody's.
    assert run.link_select(road_link(net, (0, 0), (1, 0)), zones=zones)["origin"] == ["A"]
    assert run.link_select(road_link(net, (2, 0), (1, 0))).row_count == 0
    # Several links: a trip crossing two of them counts once.
    both = run.link_select([link, road_link(net, (2, 0), (3, 0))], zones=zones)
    assert both["trips"].tolist() == [3, 2]


def test_a_departure_window_that_leaves_one_pair_out_drops_it(rows_run):
    net, run, zones = rows_run
    link = road_link(net, (1, 0), (2, 0))
    early = run.link_select(link, zones=zones, departure_s=(EIGHT - 900, EIGHT + 900))
    # B leaves at 8:30: out. A's cars are all the link's users then; A's bike still counts in
    # the pair's travellers.
    assert early["origin"] == ["A"] and early["trips"].tolist() == [3]
    assert np.allclose(early["share_link"], [1.0]) and np.allclose(early["share_pair"], [3 / 4])
    by_bin = run.link_select(link, zones=zones, bin_s=900)
    assert by_bin["departure_s"].tolist() == [EIGHT, EIGHT + 1800]
    assert np.allclose(by_bin["share_link"], [1.0, 1.0])


def test_a_bike_trip_is_listed_under_the_bike_on_the_bike_layer(rows_run):
    net, run, zones = rows_run
    bike = net.layer("bike")
    node = lambda p: bike.node_nearest(*lonlat(net, *p))  # noqa: E731
    t = run.link_select(link_between(bike, node((1, 0)), node((2, 0))), "bike", zones=zones)
    assert t["origin"] == ["A"] and t["mode"] == ["bike"] and t["trips"].tolist() == [1]
    assert np.allclose(t["share_link"], [1.0]) and np.allclose(t["share_pair"], [1 / 4])
    assert "bike" not in run.link_select(road_link(net, (1, 0), (2, 0)))["mode"]
    with pytest.raises(ValueError, match="not a link of the bike layer"):
        run.link_select(bike.link_count, "bike")
    with pytest.raises(ValueError, match="unknown layer"):
        run.link_select(0, "rail")


def test_the_trips_on_a_link_are_its_crossings_on_every_layer():
    """Conservation: the trips on a link are the crossings the per-link results count there.

    Every trip finishes, so each crossing is a trip the table counts on that link, on each layer
    (the routes have no repeated link).
    """
    net = ms.examples.manhattan_grid(n=6, block_metres=200.0, signals=False)
    rows = ms.examples.trips_random(net, 300, seed=7, min_m=300.0, max_m=1200.0)
    rows = [(*r[:7], "everyone", None) for r in rows]
    run = ms.Scenario.from_parts(
        network=net,
        demand=rows,
        classes={"everyone": (True, True, False)},
        modes=("car", "bike", "walk"),
        choice_model="logit",
        link_bin_s=300,
        window=(0, 6 * 3600),
        mode_options=DRIVES_ONLY,
    ).run("link-select-conservation", overwrite=True, quiet=True)
    assert set(run.trip_results()["outcome"]) == {"completed"}
    taken = set(run.trip_results()["mode"])
    assert {"car", "bike", "walk"} <= taken, taken
    for layer in ("road", "bike", "walk"):
        bins = run.link_bins(layer)
        crossings = np.bincount(bins.links().astype(np.int64), bins.crossings().astype(np.int64))
        busiest = np.argsort(-crossings, kind="stable")[:8]
        for link in busiest:
            t = run.link_select(int(link), layer)
            assert int(t["trips"].sum()) == int(crossings[link]), (layer, int(link))
            assert np.isclose(t["share_link"].sum(), 1.0)


def test_a_vehicle_that_re_routed_counts_on_the_links_it_took():
    net = ms.examples.manhattan_grid(n=6, block_metres=150.0, signals=False)
    rows = ms.examples.trips_random(net, 4000, seed=3, min_m=300.0, max_m=900.0, spread_s=600)
    run = ms.Scenario.from_parts(
        net,
        rows,
        classes={"commuter": (True, False, False)},
        window=(0, 10_800),
        flow_level=4,
        equilibration="free_flow",
        equilibration_options={"increments": 1, "warmup": 0},
        loading_options={"reroute": 1, "reroute_after_s": 60},
    ).run("link-select-reroute", overwrite=True, quiet=True)
    real = run.route_realised()
    assert real["trip"].size > 0, "vehicles re-route on a jammed grid"
    sets, rc = run.route_sets(), run.route_choices()
    offsets, links = sets.route_offsets().astype(np.int64), sets.links().astype(np.int64)
    checked = 0
    for t, taken in zip(real["trip"].tolist()[:20], real["links"], strict=False):
        r = int(rc.route[t])
        planned = set(links[offsets[r] : offsets[r + 1]].tolist())
        took = set(np.asarray(taken).tolist())
        for link, expect in ((min(took - planned, default=None), True),
                             (min(planned - took, default=None), False)):  # fmt: skip
            if link is None:
                continue
            listed = run.link_select(link)
            users = _link_select.layer_users(run, "road")
            hit = _link_select.paths_crossing(users, np.array([link]))[users.user_path]
            trips = set(np.asarray(run.trip_results()["trip"])[users.user_row[hit]].tolist())
            assert (t in trips) is expect
            assert int(listed["trips"].sum()) == len(trips)
            checked += 1
    assert checked > 0


def test_the_page_is_written_and_holds_the_users(rows_run, tmp_path):
    net, run, zones = rows_run
    out = viz.map_link_select(run, tmp_path / "select.html", zones=zones, quiet=True)
    text = out.read_text(encoding="utf-8")
    assert text.startswith("<!doctype html>") and not re.search(r"https?://", text)
    meta = json.loads(
        re.search(r'<script id="meta" type="application/json">(.*?)</script>', text, re.S)[1]
    )
    blob = re.search(r'<script id="blob" type="text/plain">(.*?)</script>', text, re.S)[1]
    raw = zlib.decompress(base64.b64decode(blob))
    kinds = {"u8": "<u1", "u32": "<u4", "i32": "<i4", "f32": "<f4"}
    a = {
        x["n"]: np.frombuffer(raw, dtype=kinds[x["t"]], count=x["c"], offset=x["o"])
        for x in meta["arrays"]
    }
    # Seven car trips and one bike trip: every user is in a group, with its people.
    assert a["r_gp"].sum() == 7 and a["b_gp"].sum() == 1
    assert sorted(meta["places"]) == sorted(zones)
    # The groups' paths are the users' paths: A's road route crosses (1,0)->(2,0).
    link = road_link(net, (1, 0), (2, 0))
    po, pl = a["r_po"].astype(np.int64), a["r_pl"].astype(np.int64)
    crossing = [p for p in range(len(po) - 1) if link in pl[po[p] : po[p + 1]].tolist()]
    on_link = sum(a["r_gp"][g] for g in range(len(a["r_gp"])) if a["r_gpath"][g] in crossing)
    assert on_link == 5
