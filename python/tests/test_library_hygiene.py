"""Small library defaults and refusals that a user of the package meets (review B5).

What is defended: importing the package leaves the warning filters alone (P-6); a figure shows
itself in a notebook without a rule registered for every figure (P-43); a hover card escapes the
run's text (P-44); a table path that does not exist is a missing file, not an empty table (P-23);
a headway window may be a list (P-24); a run never silently replaces another's files (P-20); a
mode asked for that the run cannot simulate is refused (CM-22); the old ``class_defaults`` keyword
is gone (P-8); the summary and the map return a ``Path`` and default into the run's folder
(P-19); a network option where 0 means off accepts 0 (G-5).
"""

import re
import shutil
import subprocess
import sys
from pathlib import Path

import openmobisim as ms
import pytest
from openmobisim import viz

CAR = {"commuter": (True, False, False)}


def grid_scenario(**kwargs):
    net = ms.examples.manhattan_grid(n=4, block_metres=200.0, signals=False)
    rows = ms.examples.trips_random(net, 20, seed=5, min_m=300.0, max_m=800.0)
    settings = {"classes": CAR, "window": (0, 3_600), "link_bin_s": 300}
    settings.update(kwargs)
    return ms.Scenario.from_parts(net, rows, **settings)


def test_importing_the_package_leaves_the_warning_filters_alone() -> None:
    # P-6: three "always" filters at import sat in front of the user's own `-W ignore`.
    code = (
        "import warnings; before = list(warnings.filters); import openmobisim as ms; "
        "mine = (ms.WindowWarning, ms.FallbackWarning, ms.NoAlternativeWarning); "
        "added = [f for f in warnings.filters if f not in before]; "
        "assert not [f for f in added if f[2] in mine], added"
    )
    subprocess.run([sys.executable, "-c", code], check=True)


def test_a_figure_shows_itself_in_a_notebook_and_a_users_figure_is_left_alone() -> None:
    matplotlib = pytest.importorskip("matplotlib")
    from matplotlib.figure import Figure

    fig = viz.map_link(grid_scenario().run("hyg-fig", quiet=True), size_cm=(10, 6), dpi=50)
    png = fig._repr_png_()
    assert png[:8] == b"\x89PNG\r\n\x1a\n" and isinstance(fig, Figure)
    assert not hasattr(Figure(), "_repr_png_"), f"{matplotlib.__version__}: nothing registered"


@pytest.mark.skipif(shutil.which("node") is None, reason="needs node to run the page's script")
def test_a_hover_card_escapes_the_text_of_the_run(tmp_path) -> None:
    # P-44: a stop named `A&B <x>` must not become markup.
    from openmobisim.viz._interactive_page import PAGE_JS

    helpers = [
        re.search(rf"^function {name}\(.*$", PAGE_JS, re.M)[0] for name in ("escHtml", "tipHtml")
    ]
    script = "\n".join(helpers) + (
        '\nconsole.log(tipHtml("stop " + "A&B <x>", [["Name", \'"q" <img onerror=1>\']]));'
    )
    js = tmp_path / "tip.js"
    js.write_text(script, encoding="utf-8")
    out = subprocess.run(["node", str(js)], capture_output=True, text=True, check=True).stdout
    assert "A&amp;B &lt;x&gt;" in out and "&lt;img onerror=1&gt;" in out and "<img" not in out
    assert out.startswith("<b>stop ") and "<div><span>Name</span>" in out
    # Both cards build their HTML through it.
    assert PAGE_JS.count("innerHTML = tipHtml(") == 2


@pytest.mark.parametrize("reader", [ms.network_read_table, ms.parking_read_table])
def test_a_table_path_that_does_not_exist_is_a_missing_file(reader) -> None:
    with pytest.raises(FileNotFoundError, match="missing.csv"):
        reader("missing.csv")


def test_a_table_given_as_text_is_still_read() -> None:
    parkings = ms.parking_read_table("parking_id,lon,lat,vehicle,capacity\np1,4.9,52.37,car,10\n")
    assert parkings is not None


def test_a_headway_window_may_be_a_list() -> None:
    t = ms.examples.toy_network_transit()
    as_tuple = ms.transit_edit(t, headway={"T1": (1200, 3600, 7200)}).lines()["runs"]
    as_list = ms.transit_edit(t, headway={"T1": [1200, 3600, 7200]}).lines()["runs"]
    assert as_list == as_tuple
    with pytest.raises(ValueError, match="seconds, from_s, to_s"):
        ms.transit_edit(t, headway={"T1": [1200, 3600]})


def test_a_run_writes_to_a_folder_of_its_own_and_never_replaces_another(tmp_path) -> None:
    # P-20: the default folder is ./openmobisim-runs/<run_id>; a second run with the same id,
    # in a folder that holds a run, is refused unless asked.
    scenario = grid_scenario()
    first = scenario.run("hyg-twice", quiet=True)
    assert Path(first.timings_path).parent == tmp_path / "openmobisim-runs" / "hyg-twice"
    with pytest.raises(FileExistsError, match="overwrite=True"):
        scenario.run("hyg-twice", quiet=True)
    again = scenario.run("hyg-twice", quiet=True, overwrite=True)
    assert again.fingerprint == first.fingerprint
    elsewhere = scenario.run("hyg-twice", str(tmp_path / "other"), quiet=True)
    assert (Path(elsewhere.timings_path).parent / "manifest.json").exists()


def test_the_summary_and_the_map_default_into_the_runs_folder_and_return_a_path() -> None:
    pytest.importorskip("matplotlib")
    run = grid_scenario().run("hyg-pages", quiet=True)
    folder = Path(run.timings_path).parent
    page, map_page = viz.summary(run, quiet=True), viz.map_interactive(run, quiet=True)
    assert (page, map_page) == (folder / "summary.html", folder / "map.html")
    assert isinstance(page, Path) and isinstance(map_page, Path)
    assert page.exists() and map_page.exists()


def test_a_mode_the_run_cannot_simulate_is_refused_with_what_it_needs() -> None:
    # CM-22: `modes=` naming transit with no timetable left it out silently.
    with pytest.raises(ValueError, match=r'"transit" needs transit='):
        grid_scenario(modes=["car", "transit"])
    with pytest.raises(ValueError, match=r'"car_transit" needs transit=… and parkings='):
        grid_scenario(modes=["car", "car_transit"])
    grid_scenario(modes=["car", "walk"])  # what it can simulate is accepted


def test_the_old_class_defaults_keyword_is_gone() -> None:
    with pytest.raises(TypeError, match="class_defaults"):
        grid_scenario(class_defaults={"commuter": (True, False, False)})


def test_a_network_option_where_0_means_off_accepts_0() -> None:
    # G-5: no signal delay and no ferry wait are 0; others stay above 0.
    net = ms.examples.manhattan_grid(n=3, block_metres=200.0, signals=True)
    assert net is not None
    rows = "from,to,length,speed\na,b,100,50\nb,a,100,50\n"
    off = ms.network_read_table(
        rows, network_options={"control_delay_factor": 0, "ferry_wait_s": 0}
    )
    assert ms.network_options(off)["control_delay_factor"] == 0
    with pytest.raises(ValueError, match="capacity_factor"):
        ms.network_read_table(rows, network_options={"capacity_factor": 0})
