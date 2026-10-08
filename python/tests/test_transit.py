"""Transit (S199): reading a GTFS feed, transit trips in a scenario, buses on the roads.

The toy network's timetable (``examples.toy_network_transit``) carries the tram ``T1``
(``N1`` ↔ ``H`` at ``D2``) and the bus ``B1`` (``W``, ``M``, ``D1``); the hand values are
the Rust suite's (TR1, B1, B2), checked here across the Python boundary.
"""

from __future__ import annotations

import math
import os
from pathlib import Path

import openmobisim as ms
import pytest

FEED = {
    "stops.txt": "stop_id,stop_name,stop_lat,stop_lon,location_type,parent_station\n"
    "A,Alpha,52.37,4.90,0,\nB,Bravo,52.37,4.91,0,\nZ,Far,40.0,-3.7,0,\n",
    "routes.txt": "route_id,route_short_name,route_long_name,route_type\nR,1,One,3\n",
    "trips.txt": "route_id,service_id,trip_id\nR,WK,T1\nR,WK,T2\n",
    "calendar.txt": "service_id,monday,tuesday,wednesday,thursday,friday,saturday,sunday,"
    "start_date,end_date\nWK,1,1,1,1,1,0,0,20261005,20261009\n",
    "stop_times.txt": "trip_id,arrival_time,departure_time,stop_id,stop_sequence\n"
    "T1,08:00:00,08:00:00,A,1\nT1,08:05:00,08:05:00,B,2\n"
    "T2,09:00:00,09:00:00,A,1\nT2,09:30:00,09:30:00,Z,2\n",
}


def write_feed(folder: Path) -> Path:
    folder.mkdir(parents=True, exist_ok=True)
    for name, text in FEED.items():
        (folder / name).write_text(text, encoding="utf-8")
    return folder


def toy_trip(net: object, who: str, frm: str, to: str, depart: int, mode: str) -> tuple:
    o, d = net.node_lonlat(frm), net.node_lonlat(to)
    return (who, 0, o[0], o[1], d[0], d[1], depart, "everyone", None, mode)


def toy_run(rows: list[tuple], run_id: str, **kwargs: object) -> ms.Run:
    return ms.Scenario.from_parts(
        network=kwargs.pop("network", None) or ms.examples.toy_network(),
        demand=rows,
        classes={"everyone": (True, True, False)},
        transit=ms.examples.toy_network_transit(),
        equilibration="free_flow",
        link_bin_s=300,
        **kwargs,
    ).run(run_id=run_id)


# --- reading a feed ------------------------------------------------------------------------------


def test_a_feed_is_read_for_its_busiest_weekday(tmp_path: Path) -> None:
    transit = ms.transit_read_gtfs(str(write_feed(tmp_path / "feed")))
    assert transit.date == "2026-10-05", "every weekday alike: the earliest"
    assert (transit.run_count, transit.stop_count, transit.route_count) == (2, 3, 1)
    assert transit.runs_by_kind() == {"bus": 2}
    report = transit.read_report()
    assert report is not None and report["date_chosen"] is True
    assert report["trips_on_date"] == 2
    stops = transit.stops()
    assert stops["stop_id"] == ["A", "B", "Z"]
    assert stops["lon"][0] == pytest.approx(4.90)


def test_a_feed_is_clipped_to_a_network_and_a_day_can_be_asked_for(tmp_path: Path) -> None:
    feed = str(write_feed(tmp_path / "feed"))
    # The toy network is near 4.8 E, 52.37 N: A and B are too far from its walk layer
    # for a pedestrian (over 300 m), so nothing is kept and nothing runs.
    with pytest.raises(ValueError, match="stop|run"):
        ms.transit_read_gtfs(feed, network=ms.examples.toy_network(), date="2026-10-07")
    transit = ms.transit_read_gtfs(feed, date="2026-10-07")
    assert transit.date == "2026-10-07"
    with pytest.raises(ValueError, match="nothing runs"):
        ms.transit_read_gtfs(feed, date="2026-10-10")
    with pytest.raises(ValueError, match="not a date"):
        ms.transit_read_gtfs(feed, date="10/07/2026")


def test_a_missing_feed_is_an_error(tmp_path: Path) -> None:
    with pytest.raises(ValueError):
        ms.transit_read_gtfs(str(tmp_path / "nowhere.zip"))


# --- transit trips in a scenario -----------------------------------------------------------------


def test_the_toy_timetable() -> None:
    transit = ms.examples.toy_network_transit()
    assert transit.runs_by_kind() == {"tram": 36, "bus": 18}
    assert transit.stop_count == 5
    assert transit.read_report() is None


def test_tr1_a_tram_trip_and_its_passenger_counts() -> None:
    net = ms.examples.toy_network()
    run = toy_run([toy_trip(net, "a", "N1", "D2", 0, "transit")], "tr1", network=net)
    by_mode = run.completion_by_mode["transit"]
    assert by_mode["completed"] == 1
    assert by_mode["total_travel_time_s"] == 900.0, "the 600 tram: the 0 one leaves too soon"
    calls = run.transit_calls()
    assert calls is not None
    i = calls["run"].index("T1:out:01")
    assert (calls["stop_id"][i], calls["boardings"][i], calls["on_board"][i]) == ("N1", 1.0, 1.0)
    assert calls["alightings"][i + 1] == 1.0
    assert run.transit_summary["boardings"] == 1.0
    pq = pytest.importorskip("pyarrow.parquet")
    kpis = pq.read_table(run.kpis().path).to_pylist()
    transit = {r["metric"]: r["value"] for r in kpis if r["mode"] == "transit"}
    assert transit["completed_trips"] == 1.0
    assert transit["boardings"] == 1.0


def test_b2_a_bus_behind_cars_is_late_and_so_is_its_passenger() -> None:
    net = ms.examples.toy_network()
    rows = [toy_trip(net, f"c{i}", "W", "M", 0, "car") for i in range(10)]
    rows.append(toy_trip(net, "p", "M", "D1", 0, "transit"))
    run = toy_run(rows, "b2", network=net, choice_model="deterministic")
    calls = run.transit_calls()
    i = calls["run"].index("B1:00")
    assert calls["stop_id"][i : i + 3] == ["W", "M", "D1"]
    assert list(calls["arrival_s"][i : i + 3]) == [0.0, 139.0, 244.0]
    assert list(calls["scheduled_arrival_s"][i : i + 3]) == [0.0, 120.0, 240.0]
    assert calls["departure_s"][i + 1] == 159.0
    p = run.completion_by_mode["transit"]
    assert p["total_travel_time_s"] == 244.0
    summary = run.transit_summary
    assert summary["bus_runs_on_roads"] == 18
    assert summary["bus_groups_on_roads"] == 1
    assert summary["bus_delay_mean_s"] == pytest.approx((4.0 - 17.0 * 35.0) / 18.0)


def test_without_a_timetable_transit_is_not_available() -> None:
    net = ms.examples.toy_network()
    row = toy_trip(net, "a", "N1", "D2", 0, "transit")
    run = ms.Scenario.from_parts(
        network=net,
        demand=[row],
        classes={"everyone": (False, False, False)},
        equilibration="free_flow",
    ).run(run_id="no-transit")
    assert run.completion["mode_not_available"] == 1
    assert run.transit_calls() is None and run.transit_summary is None


def test_a_timetable_changes_the_fingerprint_and_runs_are_reproducible() -> None:
    net = ms.examples.toy_network()
    rows = [toy_trip(net, "a", "N1", "D2", 0, "transit")]
    a = toy_run(rows, "fp-a", network=net)
    b = toy_run(rows, "fp-b", network=net)
    assert a.fingerprint == b.fingerprint
    assert list(a.transit_calls()["arrival_s"]) == list(b.transit_calls()["arrival_s"])
    plain = ms.Scenario.from_parts(
        network=net,
        demand=[toy_trip(net, "a", "N1", "D2", 0, "walk")],
        classes={"everyone": (False, False, False)},
        equilibration="free_flow",
    ).run(run_id="fp-c")
    assert plain.fingerprint != a.fingerprint


@pytest.mark.skipif(
    not os.environ.get("OPENMOBISIM_TEST_GTFS"),
    reason="set OPENMOBISIM_TEST_GTFS to a GTFS feed to read a real one",
)
def test_a_real_feed_reads() -> None:
    transit = ms.transit_read_gtfs(os.environ["OPENMOBISIM_TEST_GTFS"])
    assert transit.run_count > 0 and transit.stop_count > 0
    report = transit.read_report()
    assert report["runs_kept"] == transit.run_count
    assert not math.isnan(sum(transit.stops()["lat"]))


# --- the transit file and figure -----------------------------------------------------------------


def test_the_transit_calls_file_is_written() -> None:
    net = ms.examples.toy_network()
    run = toy_run([toy_trip(net, "a", "N1", "D2", 0, "transit")], "file", network=net)
    table = run.transit_calls_table()
    assert table is not None and Path(table.path).exists()
    pq = pytest.importorskip("pyarrow.parquet")
    rows = pq.read_table(table.path).to_pylist()
    assert len(rows) == ms.examples.toy_network_transit().call_count
    out = [r for r in rows if r["run"] == "T1:out:01"]
    assert (out[0]["stop_id"], out[0]["boardings"], out[1]["alightings"]) == ("N1", 1.0, 1.0)
    assert out[1]["arrival_s"] == 900


def test_map_transit_draws_a_run_with_a_timetable(tmp_path: Path) -> None:
    pytest.importorskip("matplotlib")
    from openmobisim import viz

    net = ms.examples.toy_network()
    run = toy_run([toy_trip(net, "a", "N1", "D2", 0, "transit")], "map", network=net)
    assert run.transit is not None
    out = tmp_path / "transit.png"
    for theme in ("paper", "night"):
        fig = viz.map_transit(run, theme=theme, path=str(out))
        assert fig is not None and out.stat().st_size > 10_000
        # X-59: the footer names where the network came from; the toy's is not OpenStreetMap's.
        footer = " ".join(t.get_text() for t in fig.texts)
        assert f"{net.source} network" in footer and "OpenStreetMap" not in footer
    # By line (T-8): one colour per line, in the legend by name; the stops named on a small network.
    fig = viz.map_transit(run, colour="line")
    texts = [t.get_text() for t in fig.texts] + [t.get_text() for t in fig.axes[0].texts]
    for line in set(run.transit_calls()["line"]):
        assert any(t.startswith(line) for t in texts), line
    names = set(run.transit.stops()["name"])
    assert names <= set(texts), "every stop named"
    unnamed = viz.map_transit(run, stop_labels=False)
    assert not names & {t.get_text() for t in unnamed.axes[0].texts}
    with pytest.raises(ValueError, match="colour"):
        viz.map_transit(run, colour="mode")
    plain = ms.Scenario.from_parts(
        network=net,
        demand=[toy_trip(net, "a", "N1", "D2", 0, "walk")],
        classes={"everyone": (False, False, False)},
        equilibration="free_flow",
    ).run(run_id="map-none")
    with pytest.raises(ValueError, match="transit"):
        viz.map_transit(plain)


# --- scenario edits of a timetable (S238) -----------------------------------------------------


def _tram_trip(transit: ms._core.Transit, run_id: str) -> ms.Run:
    # N1 → D2 by tram, leaving 100 s after a tram: it waits for the next.
    net = ms.examples.toy_network()
    o, d = net.node_lonlat("N1"), net.node_lonlat("D2")
    rows = [("p", 0, o[0], o[1], d[0], d[1], 100, "x", None, "transit")]
    return ms.Scenario.from_parts(
        net, rows, classes={"x": (False, False, True)}, transit=transit,
        equilibration="free_flow", flow_level=0,
    ).run(run_id, quiet=True)  # fmt: skip


def test_a_line_at_another_headway_changes_the_wait_and_a_cancelled_one_is_gone() -> None:
    t = ms.examples.toy_network_transit()
    lines = t.lines()
    assert lines["route_id"] == ["B1", "T1"] and lines["kind"] == ["bus", "tram"]
    assert lines["runs"] == [18, 36] and lines["first_s"] == [0, 0]
    fast = ms.transit_edit(t, headway={"T1": 300})
    # The tram every 10 min, each way: out 0 … 10 200 s, back 300 … 10 500 s. Every 5 min over
    # the same day: 36 out, 35 back.
    assert fast.lines()["runs"] == [18, 71] and fast.lines()["last_s"] == lines["last_s"]
    assert t.lines()["runs"] == [18, 36], "the original is kept"
    base, quicker = _tram_trip(t, "edit-tram-base"), _tram_trip(fast, "edit-tram-fast")
    assert base.mean_travel_time_s == pytest.approx(800.0), "waits 500 s, rides 300 s"
    assert quicker.mean_travel_time_s == pytest.approx(500.0), "waits 200 s"
    assert quicker.fingerprint != base.fingerprint
    gone = ms.transit_edit(t, cancel=["T1"])
    assert gone.lines()["runs"] == [18, 0]
    assert _tram_trip(gone, "edit-tram-gone").mean_travel_time_s > 800.0


def test_a_headway_in_a_window_leaves_the_rest_of_the_day() -> None:
    t = ms.examples.toy_network_transit()
    # Every 20 min between 01:00 and 02:00 only: the tram's 12 runs there become 6.
    edited = ms.transit_edit(t, headway={"T1": (1200, 3600, 7200)})
    assert edited.lines()["runs"] == [18, 36 - 12 + 6]


@pytest.mark.parametrize(
    ("edit", "message"),
    [
        ({"cancel": ["Z9"]}, "no line 'Z9'"),
        ({"headway": {"T1": 0}}, "above 0"),
        ({"headway": {"T1": (300, 7200, 3600)}}, "ends before"),
    ],
)
def test_wrong_timetable_edits_are_refused(edit: dict, message: str) -> None:
    with pytest.raises(ValueError, match=message):
        ms.transit_edit(ms.examples.toy_network_transit(), **edit)
