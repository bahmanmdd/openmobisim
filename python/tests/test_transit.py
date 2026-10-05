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
    plain = ms.Scenario.from_parts(
        network=net,
        demand=[toy_trip(net, "a", "N1", "D2", 0, "walk")],
        classes={"everyone": (False, False, False)},
        equilibration="free_flow",
    ).run(run_id="map-none")
    with pytest.raises(ValueError, match="transit"):
        viz.map_transit(plain)
