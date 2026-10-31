"""The interactive map.

What is defended: the file is one self-contained page (no network, no library),
its embedded arrays are exactly the run's (geometry to 0.05 m, results and
routes as they are), a build is repeatable byte for byte, the ``view`` and
``max_route_pairs`` limits keep every remaining route whole and its links
consistent, and a run without traffic or routes still makes a valid page. It
needs no matplotlib.
"""

import base64
import json
import re
import shutil
import subprocess
import sys
import zlib

import numpy as np
import openmobisim as ms
import pytest
from openmobisim import viz
from openmobisim.viz import _geometry as geo

CAR = {"commuter": (True, False, False)}
DTYPES = {"u8": "<u1", "u32": "<u4", "i32": "<i4", "f32": "<f4"}
NONE = 2**32 - 1


def grid_run(n=8, trips=120, seed=5, **kwargs):
    net = ms.examples.manhattan_grid(n=n, block_metres=200.0, signals=False)
    rows = ms.examples.trips_random(net, trips, seed=seed, min_m=300.0, max_m=1200.0)
    settings = {"classes": CAR, "window": (0, 3_600), "flow_level": 4, "link_bin_s": 300}
    settings.update(kwargs)
    return net, ms.Scenario.from_parts(net, rows, **settings).run(
        "interactive-test", overwrite=True
    )


def decode(path):
    """The page's meta, and its arrays by name, exactly as the browser unpacks them."""
    text = path.read_text(encoding="utf-8")
    meta = json.loads(
        re.search(r'<script id="meta" type="application/json">(.*?)</script>', text, re.S)[1]
    )
    blob = re.search(r'<script id="blob" type="text/plain">(.*?)</script>', text, re.S)[1]
    raw = zlib.decompress(base64.b64decode(blob))
    arrays = {
        a["n"]: np.frombuffer(raw, dtype=DTYPES[a["t"]], count=a["c"], offset=a["o"])
        for a in meta["arrays"]
    }
    return text, meta, arrays


def test_the_page_is_one_self_contained_file(tmp_path):
    _, run = grid_run()
    out = viz.map_interactive(run, tmp_path / "map.html")
    text = out.read_text(encoding="utf-8")
    assert out.suffix == ".html" and text.startswith("<!doctype html>")
    # Nothing is fetched: no address at all, no resource tags, no network calls.
    assert not re.search(r"https?://", text)
    assert not re.search(r"<(link|img|iframe|script)\b[^>]*\b(src|href)=", text)
    assert not re.search(r"\b(fetch|XMLHttpRequest|WebSocket|importScripts)\b", text)
    assert "@import" not in text and "url(" not in text
    assert 'id="map"' in text and "DecompressionStream" in text


def test_embedded_geometry_is_the_networks_to_five_centimetres(tmp_path):
    net, run = grid_run()
    _, meta, a = decode(viz.map_interactive(run, tmp_path / "map.html"))
    coords, offsets = net.link_geometry()
    expect = geo.project_lonlat(coords, float(coords[:, 0].mean()), float(coords[:, 1].mean()))
    assert meta["n_links"] == net.link_count and meta["n_verts"] == len(coords)
    # One name per road class, busways (the last, S200) included.
    assert len(meta["class_names"]) == 19 and meta["class_names"][18] == "busway"
    assert (a["vs"] == offsets).all()
    x, y = np.cumsum(a["dx"].astype(np.int64)) * 0.1, np.cumsum(a["dy"].astype(np.int64)) * 0.1
    assert np.abs(x - expect[:, 0]).max() < 0.06 and np.abs(y - expect[:, 1]).max() < 0.06
    for name, want in (
        ("len", net.link_length_m()),
        ("ff", net.link_free_flow_s()),
        ("cap", net.link_capacity_pcu_h()),
    ):
        assert a[name] == pytest.approx(want, rel=1e-6)
    assert (a["cls"] == net.link_class()).all()


def test_embedded_results_are_the_runs_bins(tmp_path):
    _, run = grid_run()
    _, meta, a = decode(viz.map_interactive(run, tmp_path / "map.html"))
    bins = run.link_bins()
    assert meta["bin_seconds"] == bins.bin_seconds and meta["n_bins"] == bins.bins().max() + 1
    assert (a["rl"] == bins.links()).all()
    assert a["rp"] == pytest.approx(bins.pcu(), rel=1e-6)
    assert a["rs"] == pytest.approx(bins.pcu_seconds(), rel=1e-6)
    # Each bin's rows sit between its two markers.
    assert a["bs"][0] == 0 and a["bs"][-1] == len(bins) and (np.diff(a["bs"]) >= 0).all()
    for k in range(meta["n_bins"]):
        assert (bins.bins()[a["bs"][k] : a["bs"][k + 1]] == k).all()


def options(meta, a):
    """Each pair's options as (mode, travellers, drive links, legs), as the page reads them."""
    out = []
    for p in range(meta["n_pairs"]):
        opts = []
        for o in range(a["qs"][p], a["qs"][p + 1]):
            legs = list(range(a["ol"][o], a["ol"][o + 1]))
            drive = [tuple(a["pl"][a["la"][g] : a["lb"][g]]) for g in legs if a["lk"][g] == 0]
            opts.append((meta["trips"]["modes"][a["om"][o]], float(a["on"][o]), drive, legs))
        out.append(opts)
    return out


def test_embedded_routes_are_the_run_route_sets(tmp_path):
    _, run = grid_run()
    sets = run.route_sets()
    _, meta, a = decode(viz.map_interactive(run, tmp_path / "map.html", max_route_pairs=10_000))
    assert meta["has_routes"] and meta["n_pairs"] <= sets.key_count
    # Every car option drives a route of the store, whole and in order.
    store = {
        tuple(sets.links()[sets.route_offsets()[r] : sets.route_offsets()[r + 1]])
        for r in range(sets.route_count)
    }
    every = options(meta, a)
    for opts in every:
        for mode, _, drive, _ in opts:
            assert mode == "car" and len(drive) == 1 and drive[0] in store
    assert len(a["qs"]) == meta["n_pairs"] + 1 and a["qs"][-1] == len(a["om"]) == len(
        meta["trips"]["labels"]
    )
    finite = np.isfinite(a["oo"])
    assert (a["oo"][finite] >= 0).all() and (a["oo"][finite] <= 1).all()
    # Within a pair: the most travelled first; the routes nobody took last; the pairs by travellers.
    for p, opts in enumerate(every):
        n = [t for _, t, _, _ in opts]
        assert n == sorted(n, reverse=True) and sum(n) <= a["qn"][p] + 1e-6
    assert (np.diff(a["qn"]) <= 1e-6).all()


def test_a_build_is_repeatable_byte_for_byte(tmp_path):
    _, run = grid_run()
    one = viz.map_interactive(run, tmp_path / "one.html").read_bytes()
    two = viz.map_interactive(run, tmp_path / "two.html").read_bytes()
    assert one == two


def test_a_view_keeps_whole_routes_and_consistent_links(tmp_path):
    net, run = grid_run(n=10, trips=200)
    coords, _ = net.link_geometry()
    lon0, lat0 = coords.min(axis=0)
    lon1, lat1 = coords.max(axis=0)
    half = ((lon0, lat0), ((lon0 + lon1) / 2, (lat0 + lat1) / 2))
    _, whole, _ = decode(viz.map_interactive(run, tmp_path / "whole.html"))
    _, part, ap = decode(viz.map_interactive(run, tmp_path / "part.html", view=half))
    assert 0 < part["n_links"] < whole["n_links"]
    # Every referenced link exists in the clipped network, and results follow the new numbering.
    drive = [link for opts in options(part, ap) for _, _, d, _ in opts for leg in d for link in leg]
    assert ap["rl"].max() < part["n_links"] and max(drive) < part["n_links"]
    assert len(ap["bs"]) == part["n_bins"] + 1 and ap["bs"][-1] == len(ap["rl"])
    assert part["n_pairs"] <= whole["n_pairs"] and len(part["trips"]["labels"]) <= len(
        whole["trips"]["labels"]
    )
    with pytest.raises(ValueError, match="no links"):
        viz.map_interactive(run, tmp_path / "none.html", view=((0.0, 0.0), (0.001, 0.001)))


def test_the_pair_budget_keeps_the_busiest_pairs(tmp_path):
    _, run = grid_run(trips=200)
    _, all_, a_all = decode(viz.map_interactive(run, tmp_path / "all.html", max_route_pairs=10_000))
    _, few, a_few = decode(viz.map_interactive(run, tmp_path / "few.html", max_route_pairs=5))
    assert few["n_pairs"] == 5 < all_["n_pairs"]
    assert np.allclose(a_few["qn"], a_all["qn"][:5])


def test_without_traffic_or_routes_the_page_still_stands(tmp_path):
    net, run = grid_run(link_bin_s=None)
    _, meta, a = decode(viz.map_interactive(run, tmp_path / "quiet.html", routes=False))
    assert meta["n_bins"] == 0 and not meta["has_routes"]
    assert "rl" not in a and "qs" not in a and len(a["vs"]) == net.link_count + 1
    assert meta["title"] == "Network and route alternatives"
    # The same run with its results and its route sets on.
    _, run2 = grid_run()
    _, m2, _ = decode(viz.map_interactive(run2, tmp_path / "routes.html"))
    assert m2["has_routes"] and m2["n_bins"] > 0


def test_a_route_set_object_can_be_shown_instead(tmp_path):
    net, run = grid_run()
    at = lambda r, c: ms._core.grid_node_lonlat(net, r, c)  # noqa: E731
    rs = ms.route_sets_build(net, [(*at(0, 0), *at(7, 7)), (*at(0, 7), *at(7, 0))])
    _, meta, _ = decode(viz.map_interactive(run, tmp_path / "given.html", routes=rs))
    assert meta["has_routes"] and meta["n_pairs"] == 2
    assert rs.identity[:8] in meta["provenance"]


def test_text_is_escaped_and_bad_choices_are_refused(tmp_path):
    _, run = grid_run()
    nasty = "</script><script>alert(1)</script> & <!-- "
    out = viz.map_interactive(run, tmp_path / "nasty.html", title=nasty, note=nasty, credit=nasty)
    text, meta, _ = decode(out)
    assert text.count("<script") == 3 and "alert(1)" not in text.split("<title>")[0]
    assert meta["title"] == nasty and meta["note"] == nasty and meta["credit"] == nasty
    assert "<title>&lt;/script&gt;" in text
    with pytest.raises(ValueError, match="theme"):
        viz.map_interactive(run, tmp_path / "bad.html", theme="sepia")


def test_the_page_starts_in_the_asked_theme_and_names_its_source(tmp_path):
    # Pinned to the earlier defaults: this test is about the footer's theme
    # and provenance text, not about the choice model or equilibration (the library's own
    # defaults since 2026-09-23 are "logit"/"msa" — `test_the_footer_says_the_run_iterated`
    # in test_equilibration.py covers what the footer says once a run has iterated).
    _, run = grid_run(choice_model="deterministic", equilibration="free_flow")
    _, meta, _ = decode(viz.map_interactive(run, tmp_path / "n.html", theme="night", logo=False))
    assert meta["theme"] == "night" and meta["logo"] is False
    assert set(meta["tokens"]) >= {"paper", "night", "journeys"}
    assert "run interactive-test" in meta["provenance"] and "level 4 (full)" in meta["provenance"]
    assert (
        f"choice deterministic · seed 0 · fingerprint {run.fingerprint[:8]}" in meta["provenance"]
    )
    assert meta["provenance"].startswith(f"{ms.__version__} ·")
    assert meta["source"] == "synthetic network" and "synthetic" not in meta["provenance"]


def test_it_needs_no_matplotlib():
    code = (
        "import sys; sys.modules['matplotlib'] = None; "
        "from openmobisim import viz; assert callable(viz.map_interactive)"
    )
    subprocess.run([sys.executable, "-c", code], check=True)


@pytest.mark.skipif(shutil.which("node") is None, reason="needs node to check the script")
def test_the_script_is_valid_javascript(tmp_path):
    _, run = grid_run()
    text, _, _ = decode(viz.map_interactive(run, tmp_path / "map.html"))
    scripts = re.findall(r"<script>(.*?)</script>", text, re.S)
    assert scripts, "the page's own script"
    js = tmp_path / "page.js"
    js.write_text(max(scripts, key=len), encoding="utf-8")
    result = subprocess.run(["node", "--check", str(js)], capture_output=True, text=True)
    assert result.returncode == 0, result.stderr


def test_the_page_carries_how_many_travellers_took_each_route(tmp_path):
    for model in ("deterministic", "logit"):
        # One loading of everyone at once keeps the route sets at the "penalty" default's several
        # alternatives per pair — a run that loads more than once (iterations, or the free-flow
        # loading's groups, S223) starts from one route per pair (S179), which would leave the
        # logit nothing to spread over on this small a fixture.
        _, run = grid_run(
            choice_model=model,
            equilibration="free_flow",
            equilibration_options={"increments": 1, "warmup": 0},
            master_seed=3,
        )
        _, meta, a = decode(
            viz.map_interactive(run, tmp_path / f"{model}.html", max_route_pairs=10_000)
        )
        rc = run.route_choices()
        routed = rc.route >= 0
        assert np.isclose(a["on"].sum() + a["qr"].sum(), rc.weight[routed].sum())
        assert np.isclose(a["qn"].sum(), rc.weight[routed].sum())
        used = [sum(1 for _, t, _, _ in opts if t > 0) for opts in options(meta, a)]
        if model == "deterministic":
            assert max(used) == 1  # everyone takes the first route of their pair
        else:
            assert max(used) > 1, "someone takes a second route"


def choosing_run(pairs=(("W", "N1"), ("S", "D2"), ("W", "M"), ("W", "D1"))):
    """The toy network, everyone choosing among car, bike, walk, transit and park-and-ride."""
    net = ms.examples.toy_network()
    rows = []
    for k in range(80):
        frm, to = pairs[k % len(pairs)]
        o, d = net.node_lonlat(frm), net.node_lonlat(to)
        rows.append((f"t{k}", 0, o[0], o[1], d[0], d[1], 25 * k, "everyone", None, None))
    return ms.Scenario.from_parts(
        network=net,
        demand=rows,
        classes={"everyone": (True, True, False)},
        transit=ms.examples.toy_network_transit(),
        parkings=ms.examples.toy_network_parkings(),
        modes=["car", "bike", "walk", "transit", "car_transit"],
        choice_model="logit",
        equilibration="free_flow",
        link_bin_s=300,
    ).run(run_id="choosing", quiet=True)


def test_each_trip_keeps_its_chosen_journey_leg_by_leg():
    run = choosing_run()
    j, c = run.itinerary_journeys(), run.itinerary_choices()
    n = len(j["trip"])
    assert (
        n == len(c["trip"])
        and (j["trip"] == c["trip"]).all()
        and list(j["mode"]) == list(c["mode"])
    )
    assert len(j["link_offsets"]) == len(j["ride_offsets"]) == n + 1
    assert (np.diff(j["link_offsets"].astype(np.int64)) >= 0).all() and j["link_offsets"][
        -1
    ] == len(j["links"])
    assert j["ride_offsets"][-1] == len(j["ride_line"]) == len(j["ride_board"]) == len(j["ride_s"])
    layers = {
        "car": run.network,
        "bike": run.network.layer("bike"),
        "walk": run.network.layer("walk"),
    }
    for i in range(n):
        links = j["links"][j["link_offsets"][i] : j["link_offsets"][i + 1]]
        mode = j["mode"][i]
        layer = (
            layers["car" if mode in ("car", "car_transit") else mode]
            if mode in ("car", "car_transit", "bike", "walk")
            else None
        )
        if layer is not None and len(links):
            frm, to = layer.link_from(), layer.link_to()
            assert links.max() < layer.link_count
            assert (to[links[:-1]] == frm[links[1:]]).all(), "a leg's links join end to end"
        rides = range(j["ride_offsets"][i], j["ride_offsets"][i + 1])
        assert (mode in ("transit", "car_transit")) == (len(rides) > 0) or mode is None
        if len(rides):
            assert j["first_stop"][i] == j["ride_board"][rides[0]] or j["access_walk_s"][i] >= 0
    stops = run.transit.stops()
    assert (
        j["ride_board"].max() < len(stops["stop_id"])
        and j["ride_line"].max() < run.transit.route_count
    )
    assert run.network.link_count and set(j["mode"]) - {None} >= {"car", "transit"}


def test_the_page_shows_every_mode_a_pair_took(tmp_path):
    run = choosing_run()
    _, meta, a = decode(viz.map_interactive(run, tmp_path / "map.html"))
    taken = {m for opts in options(meta, a) for m, t, _, _ in opts if t > 0}
    assert len(taken) >= 3, taken
    n_stops = len(run.transit.stops()["stop_id"])
    for g in range(len(a["lk"])):
        if a["lk"][g] == 3:  # a ride: its line's stops, at least the two it joins
            stops = a["pst"][a["la"][g] : a["lb"][g]]
            assert (
                len(stops) >= 2
                and stops.max() < n_stops
                and a["lline"][g] < run.transit.route_count
            )
        if a["lk"][g] in (1, 2):  # a ride on a bike or a walk: links of its layer, as embedded
            prefix = "b" if a["lk"][g] == 1 else "w"
            assert a["pl"][a["la"][g] : a["lb"][g]].max() < len(a[prefix + "vs"]) - 1
        assert a["lsec"][g] >= 0
    # The travellers of a pair's options and its other ways add up to the pair's.
    for p in range(meta["n_pairs"]):
        assert np.isclose(a["on"][a["qs"][p] : a["qs"][p + 1]].sum() + a["qr"][p], a["qn"][p])
    assert len(meta["trips"]["places"]) == meta["n_pairs"] and meta["trips"]["has_journeys"]


def test_a_walk_to_from_or_between_stops_runs_along_its_streets(tmp_path):
    """Each walk to, from or between stops, docks and parkings (S306) runs along its streets.

    It is drawn along walk links that join end to end, from the place the journey's previous leg
    ends at to the place its next leg starts from (the pair's origin and destination at either
    end); its links begin and end within a few metres of those places (on this network each is a
    node, or a stop 3 m from one).
    """
    # Origins and destinations away from the stops, so that the journeys by transit walk.
    run = choosing_run((("S", "D1"), ("N2", "D1"), ("S", "D2"), ("W", "N1"), ("N2", "D2")))
    _, meta, a = decode(viz.map_interactive(run, tmp_path / "map.html"))

    def geometry(prefix):
        return a[prefix + "vs"], np.column_stack(
            [np.cumsum(a[prefix + "dx"]) * 0.1, np.cumsum(a[prefix + "dy"]) * 0.1]
        )

    layer = {0: geometry(""), 1: geometry("b"), 2: geometry("w")}

    def link_points(kind, links):
        vs, xy = layer[kind]
        return np.concatenate([xy[vs[x] : vs[x + 1]] for x in links])

    def point(i):
        return np.array([a["ptx"][i], a["pty"][i]]) * 0.1

    def stop(s):
        return np.array([a["tsx"][s], a["tsy"][s]]) * 0.1

    def ends(g):
        """Where a drawn leg starts and ends (None for a wait, a park or a strip-only walk)."""
        k = a["lk"][g]
        if k in (0, 1, 2) and a["lb"][g] > a["la"][g]:
            pts = link_points(k, a["pl"][a["la"][g] : a["lb"][g]])
            first = point(a["lpa"][g]) if a["lpa"][g] != NONE else pts[0]
            last = point(a["lpb"][g]) if a["lpb"][g] != NONE else pts[-1]
            return first, last
        if k == 3:
            return stop(a["pst"][a["la"][g]]), stop(a["pst"][a["lb"][g] - 1])
        if k == 4 and a["la"][g] != NONE:
            return point(a["la"][g]), point(a["lb"][g])
        return None

    near = 5.0  # metres
    walked = 0
    for p in range(meta["n_pairs"]):
        origin = np.array([a["qox"][p], a["qoy"][p]]) * 0.1
        destination = np.array([a["qdx"][p], a["qdy"][p]]) * 0.1
        for o in range(a["qs"][p], a["qs"][p + 1]):
            legs = [g for g in range(a["ol"][o], a["ol"][o + 1]) if ends(g) is not None]
            for i, g in enumerate(legs):
                if a["lk"][g] != 2 or a["lpa"][g] == NONE:
                    continue  # not a walk between places
                links = a["pl"][a["la"][g] : a["lb"][g]]
                pieces = [link_points(2, [x]) for x in links]
                for one, two in zip(pieces[:-1], pieces[1:], strict=True):
                    assert np.hypot(*(one[-1] - two[0])) < 0.2, "the links join end to end"
                start, end = point(a["lpa"][g]), point(a["lpb"][g])
                assert np.hypot(*(pieces[0][0] - start)) < near, "the links start at the place left"
                assert np.hypot(*(pieces[-1][-1] - end)) < near, "and end at the place reached"
                before = ends(legs[i - 1])[1] if i > 0 else origin
                after = ends(legs[i + 1])[0] if i + 1 < len(legs) else destination
                assert np.hypot(*(start - before)) < near, "where the previous leg ends"
                assert np.hypot(*(end - after)) < near, "where the next leg starts"
                walked += 1
    assert walked >= 4, walked
    # The legend no longer says the walks to stops are straight.
    assert "drawn straight" not in (tmp_path / "map.html").read_text(encoding="utf-8")


def test_the_page_offers_png_and_vector_exports(tmp_path):
    path = tmp_path / "map.html"
    viz.map_interactive(grid_run()[1], str(path))
    page = path.read_text(encoding="utf-8")
    assert 'id="exportpng"' in page and 'id="exportsvg"' in page
    assert "function exportSVG" in page and "function exportPNG" in page


BROWSER = next(
    (b for b in ("chromium", "google-chrome", "google-chrome-stable") if shutil.which(b)), None
)


@pytest.mark.skipif(BROWSER is None, reason="needs a Chromium to run the page")
@pytest.mark.parametrize("bins", [300, None])
def test_the_exports_run_in_a_browser_and_carry_the_credit(tmp_path, bins):
    import html
    import xml.dom.minidom

    path = tmp_path / "map.html"
    viz.map_interactive(grid_run(link_bin_s=bins)[1], str(path))

    def exported(kind):
        dom = subprocess.run(
            [BROWSER, "--headless=new", "--disable-gpu", "--no-sandbox",
             "--virtual-time-budget=3000", "--window-size=900,700", "--dump-dom",
             f"{path.as_uri()}#export={kind}"],
            capture_output=True, text=True, timeout=90,
        ).stdout  # fmt: skip
        found = re.search(r'<textarea id="exported"[^>]*>(.*?)</textarea>', dom, re.S)
        assert found, f"no {kind} export in the page"
        return html.unescape(found.group(1))

    svg = exported("svg")
    xml.dom.minidom.parseString(svg)  # well-formed
    assert svg.startswith("<svg") and "<path" in svg and ">openmobisim</text>" in svg
    png = base64.b64decode(exported("png").split(",", 1)[1])
    assert png[:8] == b"\x89PNG\r\n\x1a\n" and len(png) > 10_000


def layered_run():
    """The toy network with every kind of traveller: cars, transit, park-and-ride, bikes, walks."""
    net = ms.examples.toy_network()

    def trip(who, frm, to, depart, mode):
        o, d = net.node_lonlat(frm), net.node_lonlat(to)
        return (who, 0, o[0], o[1], d[0], d[1], depart, "everyone", None, mode)

    rows = (
        [trip(f"a{i}", "W", "N1", 20 * i, "car_transit") for i in range(10)]
        + [trip(f"c{i}", "S", "D2", 10 * i, "bike") for i in range(20)]
        + [trip(f"d{i}", "W", "M", 15 * i, "walk") for i in range(10)]
        + [trip(f"e{i}", "W", "D1", 5 * i, "car") for i in range(50)]
    )
    return ms.Scenario.from_parts(
        network=net,
        demand=rows,
        classes={"everyone": (True, True, False)},
        transit=ms.examples.toy_network_transit(),
        parkings=ms.examples.toy_network_parkings(),
        choice_model="deterministic",
        equilibration="free_flow",
        link_bin_s=300,
    ).run(run_id="layers", quiet=True)


def test_the_page_carries_each_layer_the_run_has_and_shows_only_road_traffic_at_first(tmp_path):
    """T-4: transit, parkings, cyclists and walkers, each a toggle, off at first."""
    run = layered_run()
    _, meta, a = decode(viz.map_interactive(run, tmp_path / "map.html"))
    assert set(meta["layers"]) == {"transit", "parking", "bike", "walk"}
    stops = run.transit.stops()
    assert len(a["tsx"]) == len(stops["stop_id"]) and len(
        meta["layers"]["transit"]["stops"]
    ) == len(a["tsx"])
    assert len(a["ta"]) == len(a["tb"]) == len(a["tk"]) == len(a["tl"]) > 0
    assert len(a["pc"]) == len(run.parking_places()["parking_id"])
    assert (a["po"] >= 0).all() and a["pv"].sum() == 1  # one bike parking in the toy's three
    bike = run.link_bins("bike")
    # Every bike link (S269: the unused ones drawn faint), the used ones those with traffic.
    assert len(a["bv"]) == run.network.layer("bike").link_count and a["bvs"][-1] == len(a["bdx"])
    assert (a["bv"] > 0).sum() == len(set(bike.links().tolist())) == meta["layers"]["bike"]["used"]
    assert np.isclose(a["bv"].sum(), bike.pcu().sum())
    assert len(a["bi"]) == len(a["bv"]) and len(a["bl"]) == len(a["bv"])
    text = (tmp_path / "map.html").read_text(encoding="utf-8")
    assert "ly: { road: true, transit: false, parking: false, bike: false, walk: false }" in text
    # A run with none of them carries none.
    _, plain, _ = decode(viz.map_interactive(grid_run()[1], tmp_path / "plain.html"))
    assert plain["layers"] == {}


@pytest.mark.skipif(BROWSER is None, reason="needs a Chromium to run the page")
def test_a_pair_s_trips_show_in_a_browser(tmp_path):
    path = tmp_path / "map.html"
    viz.map_interactive(choosing_run(), str(path))
    dom = subprocess.run(
        [BROWSER, "--headless=new", "--disable-gpu", "--no-sandbox",
         "--virtual-time-budget=3000", "--window-size=1400,900", "--dump-dom",
         f"{path.as_uri()}#p=0&ly=rtpbw&export=svg"],
        capture_output=True, text=True, timeout=90,
    ).stdout  # fmt: skip
    assert 'data-state="ready"' in dom, "the page started"
    card = re.search(r'<div id="tripcard">(.*?)<div class="foot">', dom, re.S)
    assert card and "travellers" in card.group(1) and 'class="strip"' in card.group(1)
    assert "Trips between two places" in dom  # the legend's section, shown with the pair


def pair_view(path, hash_):
    """A pair's card and map in a browser.

    Returns the page, its rows (class, journey, dot colour, row text, share-bar colour), the
    colours drawn on the map (an SVG export of the view) and those in the export's legend.
    """
    import html

    dom = subprocess.run(
        [BROWSER, "--headless=new", "--disable-gpu", "--no-sandbox",
         "--virtual-time-budget=3000", "--window-size=1400,900", "--dump-dom",
         f"{path.as_uri()}#{hash_}&export=svg"],
        capture_output=True, text=True, timeout=90,
    ).stdout  # fmt: skip
    rows = re.findall(
        r'<div class="(opt[^"]*)" data-o="(\d+)"><div class="top"><i class="dot" '
        r'style="background:(#[0-9a-f]{6})"></i>(.*?)<div class="share"><span style="width:[\d.]+%;'
        r'background:(#[0-9a-f]{6})"',
        dom,
        re.S,
    )
    svg = html.unescape(re.search(r'<textarea id="exported"[^>]*>(.*?)</textarea>', dom, re.S)[1])
    drawn = re.search(r'<g transform="translate\(0 \d+\)">(.*?)</g>', svg, re.S)[1]
    strokes = set(re.findall(r'stroke="(#[0-9a-f]{6})"', drawn))
    legend = set(re.findall(r'<line [^>]*stroke="(#[0-9a-f]{6})"', svg))
    return dom, rows, strokes, legend


def busiest_choosing_pair(tmp_path):
    """The choosing run's map, its meta and arrays, and its pair with the most journeys."""
    path = tmp_path / "map.html"
    _, meta, a = decode(viz.map_interactive(choosing_run(), path))
    p = int(np.argmax(np.diff(a["qs"].astype(np.int64))))
    assert a["qs"][p + 1] - a["qs"][p] >= 3
    return path, meta, a, p


@pytest.mark.skipif(BROWSER is None, reason="needs a Chromium to run the page")
@pytest.mark.parametrize("theme", ["paper", "night"])
def test_each_journey_s_colour_in_the_list_is_its_colour_on_the_map(tmp_path, theme):
    """One colour per journey, from one value: the page's palette by the journey's rank in its pair.

    The list's dot and share bar, the map's lines (an SVG export of the view) and the export's
    legend all carry the same colour for each journey, and no two journeys shown share one.
    """
    path, meta, a, p = busiest_choosing_pair(tmp_path)
    _, rows, strokes, legend = pair_view(path, f"t={theme}&p={p}")
    assert len(rows) >= 3, "the pair's journeys are listed"
    palette = meta["tokens"]["journeys"][theme]
    for _, o, dot, _, bar in rows:
        assert dot == bar == palette[(int(o) - int(a["qs"][p])) % len(palette)]
    colours = [dot for _, _, dot, _, _ in rows]
    assert len(set(colours)) == len(colours), "no two journeys shown share a colour"
    for c in colours:
        assert c in strokes, f"{c} is drawn on the map"
        assert c in legend, f"{c} is in the export's legend"


@pytest.mark.skipif(BROWSER is None, reason="needs a Chromium to run the page")
def test_the_list_holds_every_mode_and_the_modes_ticked_are_drawn(tmp_path):
    """The list holds every mode's journeys, whatever layers are on.

    Each row names its mode and its share of the pair; a mode unticked stays listed, marked,
    and is not drawn.
    """
    path, meta, a, p = busiest_choosing_pair(tmp_path)
    names = meta["trips"]["mode_names"]
    top = int(a["om"][a["qs"][p]])  # the most travelled journey's mode
    dom, rows, strokes, legend = pair_view(path, f"p={p}&ly=-&hm={top}")
    assert "Every way this pair" in dom and "whatever layers are on" in dom
    assert len(rows) >= 3, "the layers are all off, and the journeys are still listed"
    for cls, o, dot, text, _ in rows:
        mode = int(a["om"][int(o)])
        assert f"<span>{names[mode]} · " in text, "the row names its mode first"
        assert "% of the pair" in text or "not taken" in text
        hidden = mode == top
        assert ("off" in cls.split()) == hidden and ("not drawn" in text) == hidden
        assert (dot in strokes) != hidden, "drawn on the map only if its mode is ticked"
        assert (dot in legend) != hidden
    assert any(int(a["om"][int(o)]) != top for _, o, *_ in rows)


@pytest.mark.skipif(BROWSER is None, reason="needs a Chromium to run the page")
def test_the_widest_journey_is_drawn_first_and_the_thinnest_on_top(tmp_path):
    """A walk or any thinner journey along the same streets as a wider one stays in sight."""
    path, _, a, p = busiest_choosing_pair(tmp_path)
    import html

    dom, rows, _, _ = pair_view(path, f"p={p}&ly=-")
    svg = html.unescape(re.search(r'<textarea id="exported"[^>]*>(.*?)</textarea>', dom, re.S)[1])
    drawn = re.search(r'<g transform="translate\(0 \d+\)">(.*?)</g>', svg, re.S)[1]
    first = {c: drawn.index(f'stroke="{c}"') for _, _, c, _, _ in rows}
    order = sorted(rows, key=lambda r: first[r[2]])
    travellers = [float(a["on"][int(o)]) for _, o, *_ in order]
    assert len(order) >= 3 and travellers == sorted(travellers, reverse=True)


def test_the_delay_is_named_as_the_delay_over_free_flow(tmp_path):
    """The colour is mean time / free flow - 1, in four bands up to 100 % and over; v/c alike."""
    text, meta, _ = decode(viz.map_interactive(grid_run()[1], tmp_path / "map.html"))
    assert '<option value="delay">Delay over free flow</option>' in text
    assert "Delay over free flow: mean time ÷ free-flow time − 1" in text
    assert '"% over free flow"' in text
    assert "lost to delay" not in text and "of the crossing time" not in text
    assert meta["delay_bands"] == [0.15, 0.5, 1.0]
    assert meta["delay_band_labels"] == ["0–15%", "15–50%", "50–100%", "100%+"]
    assert meta["vc_bands"] == [0.6, 0.8, 1.0] and len(meta["vc_band_labels"]) == 4


def test_the_page_knows_each_link_s_direction_and_its_other_direction(tmp_path):
    """A two-way road is two links between the same nodes; the page carries both ends of each."""
    net, run = grid_run()
    _, _, a = decode(viz.map_interactive(run, tmp_path / "map.html"))
    assert (a["nf"] == net.link_from()).all() and (a["nt"] == net.link_to()).all()
    ends = set(zip(a["nf"].tolist(), a["nt"].tolist(), strict=True))
    assert all((t, f) in ends for f, t in ends)  # the grid's roads are all two-way


def ribbons(svg):
    """The road-traffic ribbons of an SVG export: (x0, y0, x1, y1, width) of each straight piece."""
    out = []
    for attrs in re.findall(r"<path ([^>]*)/>", svg):
        stroke = re.search(r'stroke="([^"]*)"', attrs)
        if not stroke or not stroke[1].startswith("rgb(") or 'stroke-linecap="round"' not in attrs:
            continue  # ribbons are in the colour ramp's rgb(); the base, casings and halos are not
        width = float(re.search(r'stroke-width="([^"]*)"', attrs)[1])
        d = re.search(r'(?:^| )d="([^"]*)"', attrs)[1]
        for sub in re.findall(r"M[^M]*", d):
            pts = [tuple(map(float, p.split())) for p in re.findall(r"[ML]([-\d.]+ [-\d.]+)", sub)]
            out += [(*pts[i], *pts[i + 1], width) for i in range(len(pts) - 1)]
    return out


@pytest.mark.skipif(BROWSER is None, reason="needs a Chromium to run the page")
def test_the_two_directions_of_a_road_are_drawn_apart(tmp_path):
    """No two ribbons overlap; a two-way street shows two, one on each side of its centre line."""
    path = tmp_path / "map.html"
    viz.map_interactive(grid_run()[1], str(path))
    dom = subprocess.run(
        [BROWSER, "--headless=new", "--disable-gpu", "--no-sandbox",
         "--virtual-time-budget=3000", "--window-size=1200,900", "--dump-dom",
         f"{path.as_uri()}#export=svg"],
        capture_output=True, text=True, timeout=90,
    ).stdout  # fmt: skip
    import html

    svg = html.unescape(re.search(r'<textarea id="exported"[^>]*>(.*?)</textarea>', dom, re.S)[1])
    flat = [r for r in ribbons(svg) if abs(r[1] - r[3]) < 0.01 and abs(r[0] - r[2]) > 5]
    assert len(flat) > 20, "the east-west ribbons"
    side_by_side = 0
    for i, (ax0, ay, ax1, _, aw) in enumerate(flat):
        for bx0, by, bx1, _, bw in flat[i + 1 :]:
            overlap = min(max(ax0, ax1), max(bx0, bx1)) - max(min(ax0, ax1), min(bx0, bx1))
            if overlap < 2:
                continue
            apart = abs(ay - by) - (aw + bw) / 2  # the space between their edges, px
            assert apart >= 1.0, f"two ribbons overlap or touch: {apart:.2f} px apart"
            side_by_side += apart < 12
    assert side_by_side > 10, "the two directions of the streets, side by side"
    # The widths grow linearly with volume up to the widest the room on screen allows (S306).
    wmax = float(re.search(r'data-wmax="([\d.]+)"', dom)[1])
    widths = [w for *_, w in ribbons(svg)]
    assert 2.5 <= wmax <= 30 and abs(max(widths) - wmax) <= 0.5
    assert max(widths) / min(widths) > 4, "the least and the most loaded read apart"


def test_the_controls_legend_compass_and_scale_share_the_right_column(tmp_path):
    """Everything the reader uses on the right (S306): only the title stays on the left."""
    text = viz.map_interactive(grid_run()[1], tmp_path / "map.html").read_text(encoding="utf-8")
    side = re.search(r'<div id="side">(.*?)\n</div>\n<footer>', text, re.S)
    assert side, "the right column"
    for part in ('<aside id="panel">', 'id="legendgroup"', 'id="compass"', 'id="scalebar"'):
        assert part in side[1]
    assert "left: 12px" not in text  # nothing pinned to the left edge but the title
