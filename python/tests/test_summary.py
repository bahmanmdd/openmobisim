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


def test_the_people_rows_of_kpis_and_the_mode_limits_in_the_manifest():
    # S232 (K-1): kpis.parquet counts people beside simulated trips. S233: a run where trips
    # choose their mode records the walk and bike limits it offered them.
    pq = pytest.importorskip("pyarrow.parquet")
    tripled = run("sum-kpis", default_weight=3)
    rows = pq.read_table(tripled.kpis().path).to_pylist()
    last = max(r["iteration"] for r in rows)
    got = {(r["mode"], r["metric"]): r["value"] for r in rows if r["iteration"] == last}
    assert got[("all", "people")] == 3 * got[("all", "trips")]
    assert got[("all", "completed_people")] == 3 * got[("all", "completed_trips")]
    assert got[("car", "completed_people")] == 3 * got[("car", "completed_trips")]
    assert tripled.manifest()["walk_max_s"] is None, "no mode choice, no limit to record"
    choosing = run("sum-modes", modes=["car", "walk"], mode_options={"walk_max_s": 600.0})
    assert (choosing.manifest()["walk_max_s"], choosing.manifest()["bike_max_s"]) == (600, 3600)


def test_trips_with_no_alternative_are_counted_by_class_with_the_limits(tmp_path):
    # S233: walkers whose trips are longer than the walk limit have nothing to choose; the
    # page counts them, by class, and says what the limits were.
    walker = {"modes": ["walk"]}
    net = ms.examples.manhattan_grid(n=6, block_metres=200.0, signals=False)
    rows = ms.examples.trips_random(net, 200, seed=5, min_m=300.0, max_m=1500.0, spread_s=600)
    rows = ms.demand_assign_classes(rows, {"walker": {**walker, "share": 1}, "driver": {
        "modes": ["car"], "share": 1}}, seed=1)  # fmt: skip
    short = ms.Scenario.from_parts(
        net, rows, classes={"walker": walker, "driver": {"modes": ["car"]}},
        mode_options={"walk_max_s": 600.0, "fallback_car": 0}, equilibration="free_flow",
    ).run("sum-none", quiet=True)  # fmt: skip
    ch = short.itinerary_choices()
    stuck = [c for c, m in zip(ch["user_class"], ch["mode"], strict=True) if m is None]
    assert stuck and set(stuck) == {"walker"}, "only walkers beyond 10 minutes are stuck"
    page = Path(viz.summary(short, str(tmp_path / "none.html"))).read_text(encoding="utf-8")
    from openmobisim.viz._summary import _fmt

    assert "Trips with no alternative" in page and f"<b>{_fmt(len(stuck), 0)}</b>" in page
    assert "up to 10 min and 60 min" in page and '<td class="">walker</td>' in page


def test_trips_with_no_alternative_fall_back_to_the_car_only_when_asked(tmp_path):
    # S271 (W-1), off by default since S276: with the fallback on, the walkers beyond the walk
    # limit travel by car instead, counted under the car, listed, warned about, and named on
    # the page; off, they are warned about and listed with their reasons.
    walker = {"modes": ["walk"], "owns_car": False}
    net = ms.examples.manhattan_grid(n=6, block_metres=200.0, signals=False)
    rows = ms.examples.trips_random(net, 200, seed=5, min_m=300.0, max_m=1500.0, spread_s=600)
    rows = ms.demand_assign_classes(rows, {"walker": {**walker, "share": 1}, "driver": {
        "modes": ["car"], "share": 1}}, seed=1)  # fmt: skip
    classes = {"walker": walker, "driver": {"modes": ["car"]}}

    def scenario(**options):
        return ms.Scenario.from_parts(
            net, rows, classes=classes, mode_options={"walk_max_s": 600.0, **options},
            equilibration="free_flow",
        )  # fmt: skip

    with pytest.warns(ms.NoAlternativeWarning, match="were not simulated"):
        off = scenario().run("sum-fallback-off", quiet=True)
    stuck = sum(m is None for m in off.itinerary_choices()["mode"])
    assert len(off.report_no_alternative()["traveller_id"]) == stuck > 0
    assert not off.fallback_car["traveller_id"], "off by default"
    with pytest.warns(ms.FallbackWarning, match="had no alternative") as caught:
        on = scenario(fallback_car=1).run("sum-fallback-on", quiet=True)
    assert not [w for w in caught if w.category is ms.NoAlternativeWarning], "all went by car"
    fell = on.fallback_car
    assert len(fell["traveller_id"]) == stuck, "the same trips, now by car"
    assert all(on.report_no_alternative()["fallback_car"]), "and still explained"
    assert "travelled by car instead" in str(caught[0].message)
    assert on.completion["completed"] == off.completion["completed"] + stuck
    assert "none" not in on.completion_by_mode, "nobody left without a mode"
    assert on.fingerprint != off.fingerprint, "the fallback is an input"
    page = Path(viz.summary(on, str(tmp_path / "fallback.html"))).read_text(encoding="utf-8")
    assert "travelled by car instead" in page


def test_trips_with_no_alternative_are_listed_with_each_mode_s_reason():
    # S276 (I-ca): a walker beyond the walk limit, and a would-be cyclist without a bike whose
    # walk is too long too: each mode offered says why it gave nothing, and what to change.
    net = ms.examples.manhattan_grid(n=6, block_metres=200.0, signals=False)
    rows = ms.examples.trips_random(net, 200, seed=5, min_m=300.0, max_m=1500.0, spread_s=600)
    classes = {
        "walker": {"modes": ["walk"]},
        "no_bike": {"modes": ["bike", "walk"], "owns_bike": False},
    }
    rows = ms.demand_assign_classes(
        rows, {name: {**c, "share": 1} for name, c in classes.items()}, seed=1
    )
    with pytest.warns(ms.NoAlternativeWarning) as caught:
        run = ms.Scenario.from_parts(
            net, rows, classes=classes, mode_options={"walk_max_s": 600.0},
            equilibration="free_flow",
        ).run("noalt-reasons", quiet=True)  # fmt: skip
    t = run.report_no_alternative()
    n = len(t["traveller_id"])
    assert n == sum(m is None for m in run.itinerary_choices()["mode"]) > 0
    columns = ["traveller_id", "trip_seq", "user_class", "departure_s", "weight", "origin_lon",
               "origin_lat", "destination_lon", "destination_lat", "mode_stated",
               "fallback_car"]  # fmt: skip
    columns += [f"reason_{m}" for m in ms.MODES] + ["change"]
    assert list(t) == columns and all(len(t[c]) == n for c in columns)
    assert set(t["user_class"]) == {"walker", "no_bike"}
    assert all(m is None for m in t["mode_stated"]), "they chose their mode"
    for i in range(n):
        assert t["reason_walk"][i] == "walk_too_long"
        assert t["reason_bike"][i] == ("no_vehicle" if t["user_class"][i] == "no_bike" else "")
        assert t["reason_car"][i] == t["reason_transit"][i] == "", "not offered"
        assert "walk_max_s" in t["change"][i]
    text = str(caught[0].message)
    assert f"{n:,} trips" in text and "walk: walk_too_long" in text and "bike: no_vehicle" in text
    assert "mode_options['walk_max_s']" in text and "owns_car or owns_bike" in text
    assert "Run.report_no_alternative()" in text
    assert "not simulated" in run._timings_text()
