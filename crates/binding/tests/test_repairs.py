"""What the run sets right instead of refusing, through the binding: counted in the record and on
`Run.repairs`, warned once per kind, the dropped trips out of `trips.parquet` and in the
summary's first tile. The hand run (test_hand_run.py, from 07:00, START = 25 200 s), its values
derived there; the counts here by hand:

1. A window of 1 800 s: `walk-late` departs at START + 3 000 s, at or after the end, and is
   dropped (`trips_outside_window`, 1). trips.parquet holds the other 6 of the 7 given; the
   demand dropped none, so the record has no `trips_given` (its `trips`, 7, are all given) and
   `[result.repairs] trips_outside_window = 1`; the summary's tile 6, "of 7 given, 1 dropped";
   `Run.open` reads the run without a warning.
2. A window of 1 801 s in bins of 60 s: its end moved up to the next bin, START + 1 860 =
   27 060 s (`window_end_moved`, id "27060"); the record keeps `window_end_s = 27060` and
   `window_end_given_s = 27001`, and `run_file` on it (the tables as files) runs from 27 001 s:
   the same repair and count again.
3. A door 5 000 m from the nearest walk node: 5 000 / (4 800 / 3 600) = 3 750 s on foot, after a
   3 600 s window: walking taken out (`modes_after_window`), the trip dropped
   (`trips_after_window`).
4. Nothing left, said plainly, the work going on: a window from START + 3 600 s drops all 7
   ("run: no trip is left: 7 given, all dropped (by reason: 7 trips_outside_window)"; the tile
   "no trip is left: of 7 given, 7 dropped"); a demand of classes not given drops all 7 at
   `Demand.from_table`; links whose modes are all null leave no link ("no link is left"), and
   the trips asking for a car on a network of the cycle path alone lose it ("no car link": the
   3 cars); a feed whose one run calls at stops not held leaves no run ("no run is left").

For bikes: the hand run's bike, departing at START, is kept and rides its 1 500 m in 360 s in
case 1; on the cycle path alone of case 4 it keeps its bike.
"""

import tomllib
import warnings
from pathlib import Path

import pyarrow.parquet as pq
import pytest

import openmobisim as oms
from test_defaults import feed, lon_lat
from test_files import write
from test_hand_run import START, hand_tables
from test_summary import value

CLASSES = ["commute", "leisure"]


def run(tmp_path, trips, window, name="run", links=None):
    """The hand run on `trips` (and `links`) over `window`, one iteration, bins of 60 s; the
    run, its network and demand, and every warning's text."""
    nodes, hand_links, _ = hand_tables()
    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always")
        network = oms.Network.from_tables(nodes, links or hand_links)
        demand = oms.Demand.from_table(trips, CLASSES, network)
        result = oms.run(network, demand, window=window, out=tmp_path / name, link_bin=60,
                         iteration_cap=1, choice="deterministic")
    return result, (network, demand), [str(w.message) for w in caught]


def record(result):
    with open(result.record, "rb") as f:
        return tomllib.load(f)


def test_a_trip_outside_the_window_is_dropped_counted_warned_and_left_out(tmp_path):
    result, (network, demand), shown = run(tmp_path, hand_tables()[2], (START, START + 1_800))
    assert shown == ["run: 1 trips departing before the window's start or at or after its end: "
                     "dropped (trips_outside_window; first: walk-late)"]
    rows = {r["trip_id"]: r for r in pq.read_table(result.trips).to_pylist()}
    assert sorted(rows) == ["bike", "car1", "car2", "car3", "lost", "walk"]
    assert rows["bike"]["travel_time_s"] == 360.0
    r = record(result)
    assert (r["demand"]["trips"], "trips_given" in r["demand"]) == (7, False)
    assert r["result"]["repairs"] == {"trips_outside_window": 1}
    assert result.repairs == {"network": {}, "demand": {}, "run": {"trips_outside_window": 1}}
    text = Path(result.summary(tmp_path / "summary.html")).read_text(encoding="utf-8")
    assert (value(text, "trips"), value(text, "given"), value(text, "dropped")) == ("6", "7", "1")
    assert "1 departing outside the window" in text
    with warnings.catch_warnings():
        warnings.simplefilter("error")
        opened = oms.Run.open(result.record, network=network, demand=demand)
    assert opened.repairs["run"] == {"trips_outside_window": 1}


def test_a_window_not_of_whole_bins_ends_at_the_next_bin_and_a_rerun_repeats_it(tmp_path):
    files = [write(tmp_path / "in", f"{n}.parquet", t)
             for n, t in zip(("nodes", "links", "trips"), hand_tables())]
    network = oms.Network.from_tables(*files[:2])
    demand = oms.Demand.from_table(files[2], CLASSES, network)
    with pytest.warns(UserWarning, match="window_end_moved; first: 27060"):
        result = oms.run(network, demand, window=(START, START + 1_801), out=tmp_path / "run",
                         link_bin=60, iteration_cap=1)
    r = record(result)
    assert (r["window_end_s"], r["window_end_given_s"]) == (27_060, 27_001)
    assert result.repairs["run"] == {"window_end_moved": 1, "trips_outside_window": 1}
    with pytest.warns(UserWarning, match="window_end_moved; first: 27060"):
        again = oms.run_file(result.record, out=tmp_path / "again")
    assert again.repairs["run"] == result.repairs["run"]
    assert Path(again.record).read_bytes() == Path(result.record).read_bytes()


def test_a_trip_reaching_the_network_after_the_window_loses_the_mode_and_is_dropped(tmp_path):
    trips = {k: v + v[-1:] for k, v in hand_tables()[2].items()}
    trips["trip_id"][-1], trips["origin_y_m"][-1] = "far", 5_000.0
    trips["modes"][-1], trips["destination_y_m"][-1] = "walk", 5_000.0
    result, (_, demand), shown = run(tmp_path, trips, (START, START + 3_600))
    assert demand.trips_given == 8
    assert result.repairs["run"] == {"modes_after_window": 1, "trips_after_window": 1}
    assert [s for s in shown if "far" in s] == [
        "run: 1 trips that would reach the network by a mode at or after the window's end: the "
        "mode taken out (modes_after_window; first: far)",
        "run: 1 trips left with no mode the window can hold: dropped (trips_after_window; "
        "first: far)",
    ]
    assert "far" not in pq.read_table(result.trips).column("trip_id").to_pylist()


def test_nothing_left_is_said_plainly_and_the_work_goes_on(tmp_path):
    trips = hand_tables()[2]
    result, _, shown = run(tmp_path, trips, (START + 3_600, START + 7_200), "late")
    assert shown[-1] == ("run: no trip is left: 7 given, all dropped (by reason: 7 "
                         "trips_outside_window)")
    assert pq.read_table(result.trips).num_rows == 0
    text = Path(result.summary(tmp_path / "late.html")).read_text(encoding="utf-8")
    assert "no trip is left: of <span data-k=\"given\">7</span> given" in text
    lost = {**trips, "class_id": ["other"] * 7}
    _, demand, shown = run(tmp_path, lost, (START, START + 3_600), "lost")
    assert ("Demand.from_table: no trip is left: 7 given, all dropped (by reason: 7 "
            "trips_of_unknown_class)") in shown
    links = {**hand_tables()[1], "modes": [None, None, None]}
    _, _, shown = run(tmp_path, trips, (START, START + 3_600), "no-link", links)
    assert any(s.startswith("Network.from_tables: no link is left: the network holds 3 nodes")
               for s in shown)
    path_only = {k: v[:1] for k, v in hand_tables()[1].items()}
    _, _, shown = run(tmp_path, trips, (START, START + 3_600), "no-car", path_only)
    assert ("Demand.from_table: the network has no car link: 3 trips that ask for a car cannot "
            "drive (trips_without_car_network)") in shown
    stops = {"S1": lon_lat(0.0, 0.0)}
    network = oms.Network.from_tables(
        {"node_id": ["a", "b"], "lon_deg": [lon_lat(0, 0)[0], lon_lat(1_000, 0)[0]],
         "lat_deg": [lon_lat(0, 0)[1]] * 2}, {**path_only, "upstream": ["a"], "length_m": [1_000.0]})
    with pytest.warns(UserWarning, match="Transit.from_gtfs: no run is left"):
        oms.Transit.from_gtfs(feed(tmp_path / "gtfs.zip", stops,
                                   {"bus": [("Q1", "08:00:00"), ("Q2", "08:10:00")]}), network)
