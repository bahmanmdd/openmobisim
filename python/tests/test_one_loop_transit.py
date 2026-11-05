"""Transit trips whose mode is stated, on the one-loop engine: journeys on the timetable.

What is defended, each from a value derived by hand or a property that must hold. Three stops A, X
and B, 6 km apart on one street (nobody walks between them); leaving A at 08:00, with a boarding
slack of 60 s: line L1 A 08:05 → B 08:30, or line L2 A 08:02 → X 08:10 then line L3 X 08:12 →
B 08:22. Both journeys are in every trip's set (the second arrives earlier with one vehicle more);
by the default coefficients (−0.2 a minute in a vehicle, −0.33 a minute walking, −0.29 a minute
waiting, −1 a transfer) V = −0.2·18 − 0.29·4 − 1 = −5.76 for the two-vehicle journey and
V = −0.2·25 − 0.29·5 = −6.45 for the direct one; within one nest of scale 0.5 the two-vehicle one
has probability 1/(1 + e^(−0.69/0.5)) = 0.798991, and 4 000 travellers take it in that share within
three standard deviations of sampling. Transit trips added to a car run change no car result, to
the bit; the same seed gives the same run on one, two and every thread; relabelling the stops,
lines and nodes changes nothing; moving the departures, the window and the timetable an hour
later moves every arrival an hour later, exactly.
"""

import math
import tempfile
from pathlib import Path

import numpy as np
import openmobisim as ms
import pytest

pytest.importorskip("pyarrow")

LAT = 45.70
KM = 1 / (111.32 * math.cos(math.radians(LAT)))
H8 = 8 * 3600
#: The three lines: stops, ride seconds between them, departures from the first stop.
LINES = {
    "L1": (["A", "B"], [25 * 60], [H8 + 5 * 60]),
    "L2": (["A", "X"], [8 * 60], [H8 + 2 * 60]),
    "L3": (["X", "B"], [10 * 60], [H8 + 12 * 60]),
}


def street(names=("n0", "n1", "n2"), link_names=None):
    """A street through A, X and B, both ways, 6 km between stops."""
    nodes = [{"id": n, "x": 4.80 + 6 * i * KM, "y": LAT} for i, n in enumerate(names)]
    links = []
    for i in range(2):
        for a, b in ((i, i + 1), (i + 1, i)):
            name = f"l{a}_{b}" if link_names is None else link_names[(a, b)]
            links.append(
                {
                    "id": name,
                    "from": names[a],
                    "to": names[b],
                    "class": "residential",
                    "length": 6000,
                }
            )
    return ms.network_read_table(links, nodes), nodes


def hhmmss(s):
    s = int(s)
    return f"{s // 3600:02d}:{s % 3600 // 60:02d}:{s % 60:02d}"


def feed(lines, stop_names=None, line_names=None, shift_s=0):
    """A one-day GTFS feed of ``lines`` (route type 1: by the timetable).

    The stops are at the street's nodes; ``stop_names`` and ``line_names`` rename stops and lines.
    """
    stop_names = stop_names or {s: s for s in "AXB"}
    line_names = line_names or {name: name for name in lines}
    folder = Path(tempfile.mkdtemp()) / "feed"
    folder.mkdir(parents=True)
    (folder / "agency.txt").write_text(
        "agency_id,agency_name,agency_url,agency_timezone\nA,V,http://x,Europe/Paris\n"
    )
    stops = "".join(
        f"{stop_names[s]},{stop_names[s]},{LAT},{4.80 + 6 * i * KM}\n" for i, s in enumerate("AXB")
    )
    (folder / "stops.txt").write_text("stop_id,stop_name,stop_lat,stop_lon\n" + stops)
    (folder / "routes.txt").write_text(
        "route_id,route_short_name,route_long_name,route_type\n"
        + "".join(f"{line_names[r]},{line_names[r]},{line_names[r]},1\n" for r in lines)
    )
    (folder / "calendar.txt").write_text(
        "service_id,monday,tuesday,wednesday,thursday,friday,saturday,sunday,start_date,end_date\n"
        "S,1,1,1,1,1,1,1,20261007,20261007\n"
    )
    trips, times = (
        ["route_id,service_id,trip_id\n"],
        ["trip_id,arrival_time,departure_time,stop_id,stop_sequence\n"],
    )
    for r, (ids, rides, deps) in lines.items():
        for k, dep in enumerate(deps):
            trips.append(f"{line_names[r]},S,{line_names[r]}_{k}\n")
            t = dep + shift_s
            for seq, sid in enumerate(ids):
                t += rides[seq - 1] if seq else 0
                stop = stop_names[sid]
                times.append(f"{line_names[r]}_{k},{hhmmss(t)},{hhmmss(t)},{stop},{seq + 1}\n")
    (folder / "trips.txt").write_text("".join(trips))
    (folder / "stop_times.txt").write_text("".join(times))
    return folder


def riders(nodes, n, depart_s, prefix="r", mode="transit"):
    a, b = nodes[0], nodes[2]
    return [
        (f"{prefix}{i:05d}", 0, a["x"], LAT, b["x"], LAT, depart_s, "c", None, mode)
        for i in range(n)
    ]


def run(path, net, rows, timetable, **kw):
    kw.setdefault("classes", {"c": (True, False, True)})
    kw.setdefault("choice_model", "nested_logit")
    kw.setdefault("window", (7 * 3600, 10 * 3600))
    kw.setdefault("equilibration", "free_flow")
    s = ms.Scenario.from_parts(net, rows, transit=timetable, **kw)
    return s.run(path.name, output_dir=str(path), quiet=True, engine="one-loop")


def two_journeys(tmp_path, n=4000, **kw):
    net, nodes = street()
    timetable = ms.transit_read_gtfs(str(feed(LINES)), network=net, date="2026-10-07")
    return run(tmp_path / "x", net, riders(nodes, n, H8), timetable, **kw)


def test_both_journeys_are_chains_with_the_attributes_and_split_derived_by_hand(tmp_path):
    r = two_journeys(tmp_path)
    c = r.chains()
    trip = np.asarray(c["trip"])
    assert set(np.bincount(trip)) == {2}  # both journeys in every trip's set
    first = trip == 0
    by_rides = {len(c["ride_line"][i]): i for i in np.flatnonzero(first)}
    two, one = by_rides[2], by_rides[1]
    assert c["ride_line"][two] == ["L2", "L3"] and c["ride_line"][one] == ["L1"]
    assert c["ride_board_stop"][two] == ["A", "X"] and c["ride_alight_stop"][two] == ["X", "B"]
    assert c["ride_board_s"][two] == [H8 + 120, H8 + 720]
    assert c["ride_alight_s"][two] == [H8 + 600, H8 + 1320]
    assert (c["in_vehicle_s"][two], c["wait_s"][two], c["walk_s"][two]) == (1080, 240, 0)
    assert (c["in_vehicle_s"][one], c["wait_s"][one], c["transfers"][one]) == (1500, 300, 0)
    assert c["transfers"][two] == 1
    assert c["cost"][two] == pytest.approx(5.76, abs=1e-12)
    assert c["cost"][one] == pytest.approx(6.45, abs=1e-12)
    p = 1 / (1 + math.exp(-0.69 / 0.5))
    assert p == pytest.approx(0.798991, abs=1e-6)
    assert np.asarray(c["probability"])[two] == pytest.approx(p, abs=1e-12)
    taken = np.asarray(c["taken"])
    rides = np.array([len(x) for x in c["ride_line"]])
    share = (taken & (rides == 2)).sum() / 4000
    assert abs(share - p) < 3 * math.sqrt(p * (1 - p) / 4000)
    # Door to door: each trip arrives at its journey's arrival.
    t = r.trip_results()
    arrival = np.asarray(t["arrival_s"])
    assert set(arrival) <= {H8 + 1320.0, H8 + 1800.0}
    assert (arrival == H8 + 1320).sum() == (taken & (rides == 2)).sum()
    assert list(set(t["mode"])) == ["transit"]
    # The boardings: by line, and at each call.
    lines = r.transit_lines()
    by = dict(zip(lines["line"], lines["boardings"], strict=True))
    n2 = float((taken & (rides == 2)).sum())
    assert by == {"L1": 4000 - n2, "L2": n2, "L3": n2}
    calls = r.transit_calls()
    assert float(np.asarray(calls["boardings"]).sum()) == 4000 + n2


def test_the_deterministic_choice_takes_the_cheaper_journey(tmp_path):
    r = two_journeys(tmp_path, n=10, choice_model="deterministic")
    # Time-based: 22 min against 30 min door to door.
    assert np.all(np.asarray(r.trip_results()["arrival_s"]) == H8 + 1320)
    assert r.mean_travel_time_s == pytest.approx(1320.0)


def test_transit_trips_change_no_car_result(tmp_path):
    net, nodes = street()
    timetable = ms.transit_read_gtfs(str(feed(LINES)), network=net, date="2026-10-07")
    cars = riders(nodes, 300, H8, prefix="c", mode=None)
    cars = [(*row[:6], H8 + 4 * i, *row[7:]) for i, row in enumerate(cars)]
    options = {
        "flow_level": 2,
        "equilibration": "msa",
        "equilibration_options": {"iterations": 5},
        "choice_model": "logit",
    }
    alone = run(tmp_path / "a", net, cars, timetable, **options)
    both = run(tmp_path / "b", net, cars + riders(nodes, 200, H8), timetable, **options)
    ta, tb = alone.trip_results(), both.trip_results()
    car = np.asarray(tb["mode"]) == "car"
    assert car.sum() == 300
    for col in ("arrival_s", "travel_time_s", "vehicle_s"):
        assert np.array_equal(np.asarray(ta[col]), np.asarray(tb[col])[car])
    ca, cb = alone.convergence(), both.convergence()
    for col in ("switch_share", "gap", "found_share", "switch_excess_expected"):
        assert np.array_equal(np.asarray(ca[col]), np.asarray(cb[col])), col


@pytest.mark.parametrize("threads", [1, 2, None])
def test_the_same_seed_gives_the_same_run_on_any_thread_count(tmp_path, threads):
    net, nodes = street()
    timetable = ms.transit_read_gtfs(str(feed(LINES)), network=net, date="2026-10-07")
    rows = riders(nodes, 500, H8)
    s = ms.Scenario.from_parts(
        net,
        rows,
        transit=timetable,
        classes={"c": (True, False, True)},
        choice_model="logit",
        window=(7 * 3600, 10 * 3600),
        equilibration="msa",
        equilibration_options={"iterations": 4},
    )
    a = s.run("a", output_dir=str(tmp_path / "a"), quiet=True, engine="one-loop", threads=threads)
    b = s.run("b", output_dir=str(tmp_path / "b"), quiet=True, engine="one-loop", threads=1)
    assert a.fingerprint == b.fingerprint
    for col in ("arrival_s", "travel_time_s"):
        assert np.array_equal(np.asarray(a.trip_results()[col]), np.asarray(b.trip_results()[col]))
    assert np.array_equal(np.asarray(a.chains()["taken"]), np.asarray(b.chains()["taken"]))


def test_relabelled_stops_lines_and_nodes_change_nothing(tmp_path):
    net, nodes = street()
    timetable = ms.transit_read_gtfs(str(feed(LINES)), network=net, date="2026-10-07")
    a = run(tmp_path / "a", net, riders(nodes, 1000, H8), timetable, choice_model="logit")
    # Every id renamed so that the sorted orders reverse.
    names = ("z9", "m5", "a1")
    renamed_links = {(0, 1): "zz", (1, 0): "yy", (1, 2): "bb", (2, 1): "aa"}
    net2, nodes2 = street(names, renamed_links)
    stops = {"A": "s9", "X": "s5", "B": "s1"}
    lines = {"L1": "R3", "L2": "R2", "L3": "R1"}
    path = str(feed(LINES, stops, lines))
    timetable2 = ms.transit_read_gtfs(path, network=net2, date="2026-10-07")
    b = run(tmp_path / "b", net2, riders(nodes2, 1000, H8), timetable2, choice_model="logit")
    for col in ("arrival_s", "travel_time_s"):
        assert np.array_equal(np.asarray(a.trip_results()[col]), np.asarray(b.trip_results()[col]))
    ca, cb = a.chains(), b.chains()
    assert np.array_equal(np.asarray(ca["probability"]), np.asarray(cb["probability"]))
    assert np.array_equal(np.asarray(ca["taken"]), np.asarray(cb["taken"]))
    assert [[lines[x] for x in rides] for rides in ca["ride_line"]] == cb["ride_line"]


def test_an_hour_later_everything_arrives_an_hour_later(tmp_path):
    net, nodes = street()
    timetable = ms.transit_read_gtfs(str(feed(LINES)), network=net, date="2026-10-07")
    a = run(tmp_path / "a", net, riders(nodes, 500, H8), timetable, choice_model="logit")
    later = ms.transit_read_gtfs(str(feed(LINES, shift_s=3600)), network=net, date="2026-10-07")
    b = run(
        tmp_path / "b",
        net,
        riders(nodes, 500, H8 + 3600),
        later,
        choice_model="logit",
        window=(8 * 3600, 11 * 3600),
    )
    ta, tb = a.trip_results(), b.trip_results()
    assert np.array_equal(np.asarray(ta["arrival_s"]) + 3600, np.asarray(tb["arrival_s"]))
    assert np.array_equal(np.asarray(ta["travel_time_s"]), np.asarray(tb["travel_time_s"]))


def test_what_the_engine_lacks_for_transit_is_refused_by_name(tmp_path):
    net, nodes = street()
    timetable = ms.transit_read_gtfs(str(feed(LINES)), network=net, date="2026-10-07")
    rows = riders(nodes, 5, H8)
    with pytest.raises(ms.EngineNotSupportedError, match="buses_on_roads"):
        run(tmp_path / "a", net, rows, timetable, transit_options={"buses_on_roads": 1})
    with pytest.raises(ms.EngineNotSupportedError, match="transit"):
        s = ms.Scenario.from_parts(net, rows, classes={"c": (True, False, True)})
        s.run("b", output_dir=str(tmp_path / "b"), quiet=True, engine="one-loop")
