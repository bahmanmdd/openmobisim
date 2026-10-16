"""The simulation window: a run starts at its start and stops at its end.

What is defended: trips departing outside the window are left out, not simulated and not
counted, with a warning that says how many and when; without a window the run takes three hours
from the earliest departure; a run started at the window's start finishes trips as one started
at midnight; a timetable keeps the calls inside the window and, for planning, those up to the
longest journey past its end, while the calls table shows the window's alone.
"""

import openmobisim as ms
import pytest

CAR = {"commuter": (True, False, False)}
EVERYONE = {"everyone": (True, True, False)}


def car_rows(net, departures):
    (olon, olat), (dlon, dlat) = net.node_lonlat("W"), net.node_lonlat("D1")
    return [
        (f"t{k}", 0, olon, olat, dlon, dlat, at, "commuter", None)
        for k, at in enumerate(departures)
    ]


def run(rows, run_id, **kwargs):
    net = kwargs.pop("network")
    return ms.Scenario.from_parts(net, rows, classes=CAR, **kwargs).run(run_id, quiet=True)


def test_trips_departing_outside_the_window_are_left_out_with_a_warning():
    net = ms.examples.toy_network()
    rows = car_rows(net, [6 * 3600, 7 * 3600 + 1800, 8 * 3600, 9 * 3600 + 1800, 10 * 3600])
    with pytest.warns(ms.WindowWarning) as caught:
        r = run(rows, "window-left-out", network=net, window=("07:30", "09:30"))
    text = str(caught[0].message)
    assert "3 of the demand's 5 trips" in text
    assert "07:30–09:30" in text
    assert "1 depart before it (06:00–06:00)" in text
    assert "2 at or after its end (09:30–10:00)" in text
    assert "window=('06:00', '11:00')" in text, "it says how to widen the window"
    assert r.completion["total_trips"] == 2, "left out: not counted"
    assert r.completion["completed"] == 2
    w = r.window
    assert (w["start_s"], w["end_s"], w["start_given"], w["end_given"]) == (
        27000,
        34200,
        True,
        True,
    )
    assert (w["before_trips"], w["after_trips"], w["trips"]) == (1, 2, 5)
    assert w["timetable_runs_kept"] is None, "no timetable"
    manifest = r.manifest()
    assert manifest["window_start_seconds"] == 27000 and manifest["window_end_seconds"] == 34200
    assert "window_seconds" not in manifest, "renamed: it was the window's end, not its length"
    assert manifest["trips_outside_window"] == 3


def test_a_demand_inside_the_window_runs_without_a_warning():
    import warnings

    net = ms.examples.toy_network()
    with warnings.catch_warnings():
        warnings.simplefilter("error", ms.WindowWarning)
        r = run(
            car_rows(net, [7 * 3600 + 1800]),
            "window-inside",
            network=net,
            window=("07:30", "09:30"),
        )
    assert r.completion["completed"] == 1


def test_without_a_window_the_run_takes_three_hours_from_the_earliest_departure():
    net = ms.examples.toy_network()
    rows = car_rows(net, [7 * 3600 + 2400, 9 * 3600, 11 * 3600])
    with pytest.warns(ms.WindowWarning, match="as no window was given"):
        r = run(rows, "window-default", network=net)
    w = r.window
    assert (w["start_s"], w["end_s"]) == (7 * 3600 + 1800, 10 * 3600 + 1800), "07:30, three hours"
    assert not w["start_given"] and not w["end_given"]
    assert w["after_trips"] == 1 and r.completion["total_trips"] == 2


def test_a_run_started_at_the_window_start_finishes_trips_as_one_started_at_midnight():
    net = ms.examples.toy_network()
    rows = car_rows(net, [7 * 3600 + 1800 + 60 * k for k in range(20)])
    late = run(rows, "window-late", network=net, window=("07:30", "09:30"))
    whole = run(rows, "window-whole", network=net, window=(0, "09:30"))
    assert late.total_travel_time_s == whole.total_travel_time_s
    assert late.completion == whole.completion


def test_a_window_is_given_as_clock_times_or_seconds():
    net = ms.examples.toy_network()
    rows = car_rows(net, [8 * 3600])
    a = run(rows, "window-clock", network=net, window=("07:30", "09:30:00"))
    b = run(rows, "window-seconds", network=net, window=(27000, 34200))
    assert (a.window["start_s"], a.window["end_s"]) == (b.window["start_s"], b.window["end_s"])
    for bad in (("7h30", "09:30"), ("09:30", "07:30"), ("07:30",), "07:30", (-5, 100)):
        with pytest.raises(ValueError, match="window"):
            ms.Scenario.from_parts(net, rows, classes=CAR, window=bad)


def test_a_timetable_keeps_only_its_calls_inside_the_window():
    net = ms.examples.toy_network()
    o, d = net.node_lonlat("N1"), net.node_lonlat("D2")
    rows = [("a", 0, o[0], o[1], d[0], d[1], 0, "everyone", None, "transit")]
    r = ms.Scenario.from_parts(
        net,
        rows,
        classes=EVERYONE,
        transit=ms.examples.toy_network_transit(),
        equilibration="free_flow",
        window=(0, 1800),
    ).run("window-timetable", quiet=True)
    w = r.window
    assert w["timetable_runs_kept"] > 0 and w["timetable_runs_dropped"] > 0
    calls = r.transit_calls()
    assert max(calls["scheduled_departure_s"]) <= 1800, "no call after the window's end"
    assert r.completion_by_mode["transit"]["completed"] == 1


def test_a_journey_that_ends_after_the_window_is_offered_and_left_unfinished():
    # The toy tram leaves N1 at 600 and is at H at 900; W → N3 at 200 walks 318 s to the stop,
    # rides, and walks 345 s on: at N3 at 1245. With the window closing at 800 the tram's call
    # at 900 is past it. Before S279 the timetable was cut at the window's end and the trip had
    # no alternative; now the journey is planned on (the timetable reaches past the end) and the
    # trip is unfinished at 800, as a car trip arriving at 1245 would be. The calls table still
    # shows the window's calls alone.
    import warnings

    net = ms.examples.toy_network()
    o, d = net.node_lonlat("W"), net.node_lonlat("N3")
    rows = [("p", 0, o[0], o[1], d[0], d[1], 200, "everyone", None, "transit")]
    with warnings.catch_warnings():
        warnings.simplefilter("error", ms.NoAlternativeWarning)
        r = ms.Scenario.from_parts(
            net,
            rows,
            classes=EVERYONE,
            transit=ms.examples.toy_network_transit(),
            equilibration="free_flow",
            window=(0, 800),
        ).run("window-horizon", quiet=True)
    assert r.completion["total_trips"] == 1 and r.completion["completed"] == 0, r.completion
    calls = r.transit_calls()
    assert len(calls["scheduled_departure_s"]) > 0
    assert max(calls["scheduled_departure_s"]) <= 800, "nothing after the window is shown"
    assert r.window["timetable_runs_kept"] >= 1
