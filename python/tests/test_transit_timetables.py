"""A journey's time on transit is the timetable's, to the second.

What is defended: travellers who leave at known times and take the fastest journey (the
deterministic model) arrive exactly when the timetable says — counted here independently, by
brute force over the timetable — on one line at regular and irregular headways, with and
without the boarding slack, and on Spiess and Florian's four-line network (A, X, Y, B; changes
at X and Y), which exercises the transfers. The reader, the journey search, the waits and the
slack are all on that path; a difference of one second is a fault.

The networks are written here as small GTFS feeds of metro lines (route type 1: their vehicles
keep to the timetable whatever the roads carry), on a straight street that lets travellers
reach each stop on foot.
"""

from __future__ import annotations

from pathlib import Path

import numpy as np
import openmobisim as ms
import pytest

LAT = 45.70
DLON_PER_KM = 1 / (111.32 * np.cos(np.radians(LAT)))
MINUTE = 60.0


def street(stops_km: list[float]) -> tuple:
    nodes = [
        {"id": f"n{i}", "x": 4.80 + km * DLON_PER_KM, "y": LAT} for i, km in enumerate(stops_km)
    ]
    links = [
        {"id": f"l{a}_{b}", "from": f"n{a}", "to": f"n{b}", "class": "residential",
         "length": abs(stops_km[b] - stops_km[a]) * 1000}
        for i in range(len(stops_km) - 1)
        for a, b in ((i, i + 1), (i + 1, i))
    ]  # fmt: skip
    return ms.network_read_table(links, nodes), [n["x"] for n in nodes]


def hhmmss(s: float) -> str:
    s = int(round(s))
    return f"{s // 3600:02d}:{s % 3600 // 60:02d}:{s % 60:02d}"


def write_feed(folder: Path, stops: dict[str, float], lines: dict) -> Path:
    """One day's feed; ``lines``: route → (stop ids, ride seconds between them, departures)."""
    folder.mkdir(parents=True)
    files = {
        "agency.txt": "agency_id,agency_name,agency_url,agency_timezone\nA,A,http://a,Europe/Paris\n",
        "stops.txt": "stop_id,stop_name,stop_lat,stop_lon\n"
        + "".join(f"{s},{s},{LAT},{lon}\n" for s, lon in stops.items()),
        "routes.txt": "route_id,route_short_name,route_long_name,route_type\n"
        + "".join(f"{r},{r},{r},1\n" for r in lines),
        "calendar.txt": "service_id,monday,tuesday,wednesday,thursday,friday,saturday,sunday,"
        "start_date,end_date\nS,1,1,1,1,1,1,1,20261007,20261007\n",
    }
    trips = ["route_id,service_id,trip_id\n"]
    times = ["trip_id,arrival_time,departure_time,stop_id,stop_sequence\n"]
    for route, (ids, rides, departures) in lines.items():
        for k, departure in enumerate(departures):
            trips.append(f"{route},S,{route}_{k}\n")
            at = departure
            for seq, stop in enumerate(ids):
                at += rides[seq - 1] if seq else 0
                times.append(f"{route}_{k},{hhmmss(at)},{hhmmss(at)},{stop},{seq + 1}\n")
    files["trips.txt"], files["stop_times.txt"] = "".join(trips), "".join(times)
    for name, text in files.items():
        (folder / name).write_text(text, encoding="utf-8")
    return folder


def every(headways: list[float], first: float, last: float) -> np.ndarray:
    out, at, k = [], first, 0
    while at <= last:
        out.append(at)
        at += headways[k % len(headways)]
        k += 1
    return np.asarray(out)


def next_departure(departures: np.ndarray, at: np.ndarray) -> np.ndarray:
    return departures[np.searchsorted(departures, at, side="left")]


def ride(tmp_path, net, stops, lines, frm, to, leave, slack) -> ms.Run:
    transit = ms.transit_read_gtfs(
        str(write_feed(tmp_path / "feed", stops, lines)), network=net, date="2026-10-07"
    )
    rows = [
        (f"p{i}", 0, stops[frm], LAT, stops[to], LAT, int(t), "rider", None, "transit")
        for i, t in enumerate(leave)
    ]
    return ms.Scenario.from_parts(
        net, rows, classes={"rider": (False, False, True)}, transit=transit,
        transit_options={"board_slack_s": slack}, choice_model="deterministic",
        equilibration="free_flow",
    ).run("timetable", quiet=True)  # fmt: skip


LEAVE = np.arange(7 * 3600, 9 * 3600, 10.0)  # a traveller every 10 s for two hours


@pytest.mark.parametrize("headways", [[600.0], [240.0, 960.0], [120.0, 1080.0]])
@pytest.mark.parametrize("slack", [0.0, 60.0])
def test_one_line_s_journeys_wait_for_the_next_departure_after_the_slack(tmp_path, headways, slack):
    net, x = street([0.0, 5.0])
    stops = {"A": x[0], "B": x[1]}
    departures = every(headways, 6 * 3600, 10 * 3600)
    run = ride(
        tmp_path, net, stops, {"L": (["A", "B"], [600.0], departures)}, "A", "B", LEAVE, slack
    )
    by_hand = np.mean(next_departure(departures, LEAVE + slack) - LEAVE) + 600.0
    assert run.completion["completed"] == len(LEAVE)
    assert run.mean_travel_time_s == pytest.approx(by_hand, abs=1e-6)


def spiess_florian(offsets: dict[str, float]) -> dict:
    first, last = 5 * 3600, 11 * 3600
    return {
        "L1": (["A", "B"], [25 * MINUTE], every([12 * MINUTE], first + offsets["L1"], last)),
        "L2": (["A", "X", "Y"], [7 * MINUTE, 6 * MINUTE],
               every([12 * MINUTE], first + offsets["L2"], last)),
        "L3": (["X", "Y", "B"], [4 * MINUTE, 4 * MINUTE],
               every([30 * MINUTE], first + offsets["L3"], last)),
        "L4": (["Y", "B"], [10 * MINUTE], every([6 * MINUTE], first + offsets["L4"], last)),
    }  # fmt: skip


def earliest_arrival(lines: dict, leave: np.ndarray, slack: float) -> np.ndarray:
    """At B, leaving A at ``leave``, over every journey this network has."""

    def board(line: str, stop: str, at: np.ndarray) -> np.ndarray:
        ids, rides, departures = lines[line]
        return next_departure(departures + sum(rides[: ids.index(stop)]), at + slack)

    direct = board("L1", "A", leave) + 25 * MINUTE
    two = board("L2", "A", leave)
    at_x, at_y = two + 7 * MINUTE, two + 13 * MINUTE
    via_x = board("L3", "X", at_x) + 8 * MINUTE
    via_y_l3 = board("L3", "Y", at_y) + 4 * MINUTE
    via_y_l4 = board("L4", "Y", at_y) + 10 * MINUTE
    l3_then_l4 = board("L4", "Y", board("L3", "X", at_x) + 4 * MINUTE) + 10 * MINUTE
    return np.minimum.reduce([direct, via_x, via_y_l3, via_y_l4, l3_then_l4])


@pytest.mark.parametrize("slack", [0.0, 60.0])
def test_spiess_and_florian_s_network_is_crossed_by_its_fastest_journey(tmp_path, slack):
    net, x = street([0.0, 6.0, 12.0, 18.0])  # 6 km apart: nobody walks between the stops
    stops = dict(zip("AXYB", x, strict=True))
    lines = spiess_florian({"L1": 0.0, "L2": 300.0, "L3": 600.0, "L4": 120.0})
    run = ride(tmp_path, net, stops, lines, "A", "B", LEAVE, slack)
    by_hand = np.mean(earliest_arrival(lines, LEAVE, slack) - LEAVE)
    assert run.completion["completed"] == len(LEAVE)
    assert run.mean_travel_time_s == pytest.approx(by_hand, abs=1e-6)
    assert by_hand > 25 * MINUTE, "nobody beats line 1's ride alone"
