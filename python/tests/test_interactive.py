"""The interactive map (S167).

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


def grid_run(n=8, trips=120, seed=5, **kwargs):
    net = ms.examples.manhattan_grid(n=n, block_metres=200.0, signals=False)
    rows = ms.examples.trips_random(net, trips, seed=seed, min_m=300.0, max_m=1200.0)
    settings = {"class_defaults": CAR, "window_hours": 1, "flow_level": 4, "link_bin_s": 300}
    settings.update(kwargs)
    return net, ms.Scenario.from_parts(net, rows, **settings).run("interactive-test")


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


def test_embedded_routes_are_the_run_route_sets(tmp_path):
    _, run = grid_run()
    sets = run.route_sets()
    _, meta, a = decode(viz.map_interactive(run, tmp_path / "map.html", max_route_pairs=10_000))
    assert meta["has_routes"] and meta["n_pairs"] <= sets.key_count
    # Every embedded route is a route of the store, whole and in order.
    store = {
        tuple(sets.links()[sets.route_offsets()[r] : sets.route_offsets()[r + 1]])
        for r in range(sets.route_count)
    }
    for r in range(len(a["rst"]) - 1):
        assert tuple(a["rlk"][a["rst"][r] : a["rst"][r + 1]]) in store
    assert len(a["rc"]) == len(a["ro"]) == len(a["rst"]) - 1 == meta["n_routes"]
    assert a["ss"][-1] == meta["n_routes"] and len(a["ss"]) == meta["n_pairs"] + 1
    assert (a["ro"] >= 0).all() and (a["ro"] <= 1).all()
    # Within a pair: the first route is the cheapest.
    for k in range(meta["n_pairs"]):
        costs = a["rc"][a["ss"][k] : a["ss"][k + 1]]
        assert costs[0] == costs.min()


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
    assert ap["rl"].max() < part["n_links"] and ap["rlk"].max() < part["n_links"]
    assert len(ap["bs"]) == part["n_bins"] + 1 and ap["bs"][-1] == len(ap["rl"])
    assert part["n_pairs"] <= whole["n_pairs"] and part["n_routes"] <= whole["n_routes"]
    with pytest.raises(ValueError, match="no links"):
        viz.map_interactive(run, tmp_path / "none.html", view=((0.0, 0.0), (0.001, 0.001)))


def test_the_route_budget_keeps_the_richest_pairs(tmp_path):
    _, run = grid_run(trips=200)
    _, all_, a_all = decode(viz.map_interactive(run, tmp_path / "all.html", max_route_pairs=10_000))
    _, few, a_few = decode(viz.map_interactive(run, tmp_path / "few.html", max_route_pairs=5))
    assert few["n_pairs"] == 5 < all_["n_pairs"]
    per_pair_all = np.diff(a_all["ss"])
    assert np.diff(a_few["ss"]).min() >= np.sort(per_pair_all)[-5]


def test_without_traffic_or_routes_the_page_still_stands(tmp_path):
    net, run = grid_run(link_bin_s=None)
    _, meta, a = decode(viz.map_interactive(run, tmp_path / "quiet.html", routes=False))
    assert meta["n_bins"] == 0 and not meta["has_routes"]
    assert "rl" not in a and "ss" not in a and len(a["vs"]) == net.link_count + 1
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
    _, run = grid_run()
    _, meta, _ = decode(viz.map_interactive(run, tmp_path / "n.html", theme="night", logo=False))
    assert meta["theme"] == "night" and meta["logo"] is False
    assert set(meta["tokens"]) >= {"paper", "night", "route"}
    assert "run interactive-test" in meta["provenance"] and "level 4 (full)" in meta["provenance"]
    assert (
        f"choice deterministic · seed 0 · fingerprint {run.fingerprint[:8]}" in meta["provenance"]
    )
    assert meta["provenance"].startswith(f"openmobisim {ms.__version__}")
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
        _, run = grid_run(choice_model=model, master_seed=3)
        _, meta, a = decode(
            viz.map_interactive(run, tmp_path / f"{model}.html", max_route_pairs=10_000)
        )
        assert "ru" in a and len(a["ru"]) == meta["n_routes"]
        rc, sets = run.route_choices(), run.route_sets()
        routed = rc.route >= 0
        taken = np.bincount(rc.route[routed], weights=rc.weight[routed], minlength=sets.route_count)
        assert a["ru"].sum() <= taken.sum() + 1e-6 and a["ru"].sum() > 0
        if model == "deterministic":
            # Everyone takes the first route of their pair.
            for k in range(meta["n_pairs"]):
                use = a["ru"][a["ss"][k] : a["ss"][k + 1]]
                assert (use[1:] == 0).all()
        else:
            second = [
                a["ru"][a["ss"][k] + 1 : a["ss"][k + 1]].sum() for k in range(meta["n_pairs"])
            ]
            assert max(second) > 0, "someone takes a second route"
