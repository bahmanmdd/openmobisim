"""A run's summary page (S227, roadmap I-ad).

What is defended: one self-contained HTML file with the run's identity and headline numbers, its
sections, the flow map when the run recorded per-link results (and none otherwise), the mode-choice
measure where trips choose their mode, and the default place next to the run's files.
"""

from pathlib import Path

import openmobisim as ms
import pytest
from openmobisim import viz

pytest.importorskip("matplotlib")

CAR = {"commuter": (True, True, False)}


def run(name, **kwargs):
    net = ms.examples.manhattan_grid(n=6, block_metres=200.0, signals=False)
    rows = ms.examples.trips_random(net, 400, seed=5, min_m=300.0, max_m=1000.0, spread_s=600)
    settings = {"classes": CAR, "equilibration_options": {"iterations": 3}}
    settings.update(kwargs)
    return ms.Scenario.from_parts(net, rows, **settings).run(name, quiet=True)


def test_the_page_holds_the_runs_identity_numbers_and_sections(tmp_path):
    r = run("sum-1", link_bin_s=900)
    path = viz.summary(r, str(tmp_path / "page.html"), title="A grid", note="a test")
    text = Path(path).read_text(encoding="utf-8")
    assert text.startswith("<!doctype html>") and "</html>" in text
    for piece in (
        "A grid",
        "a test",
        r.fingerprint,
        "sum-1",
        "Flows",
        "Modes",
        "Convergence",
        "Where the time went",
        "Settings",
        "400",
    ):
        assert piece in text, piece
    assert text.count("data:image/png;base64,") == 2, "the flow map, by day and by night"
    assert "http" not in text.replace("http://www.w3.org", ""), "self-contained: nothing fetched"


def test_without_per_link_results_there_is_no_map_and_the_default_place_is_the_runs_folder():
    r = run("sum-2")
    path = viz.summary(r)
    assert Path(path) == Path(r.timings_path).parent / "summary.html"
    text = Path(path).read_text(encoding="utf-8")
    assert "data:image/png" not in text and "Flows" not in text


def test_a_mode_choice_run_shows_the_share_whose_mode_changed(tmp_path):
    r = run("sum-3", modes=("car", "bike", "walk"))
    text = Path(viz.summary(r, str(tmp_path / "m.html"))).read_text(encoding="utf-8")
    assert "mode changed (%)" in text and "by chance (%)" in text and "Verdict on the last" in text


def test_a_sampled_run_reports_its_mean_trip_per_person(tmp_path):
    # S232: a traveller standing for three people adds three times its travel time to the
    # total (and its car counts as 3 PCU); the mean trip is per person, on the run and the page.
    plain, tripled = run("sum-w1"), run("sum-w3", default_weight=3)
    assert plain.mean_travel_time_s == pytest.approx(
        plain.total_travel_time_s / plain.completion["completed"]
    )
    assert tripled.mean_travel_time_s == pytest.approx(
        tripled.total_travel_time_s / (3 * tripled.completion["completed"])
    )
    car = tripled.completion_by_mode["car"]
    assert (car["people"], car["completed_people"]) == (
        3 * car["total_trips"],
        3 * car["completed"],
    )
    c = tripled.convergence()
    assert list(c["completed_people"]) == [3.0 * x for x in c["completed"]]
    page = Path(viz.summary(tripled, str(tmp_path / "w3.html"))).read_text(encoding="utf-8")
    assert f"{tripled.mean_travel_time_s / 60:.1f}" in page
    from openmobisim.viz._summary import _fmt

    assert f">{_fmt(3 * tripled.completion['total_trips'], 0)}<" in page, "trips counted as people"
