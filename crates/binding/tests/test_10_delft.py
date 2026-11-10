"""Test 10: Delft, signals ignored. Release only, by hand on the Mac (not in CI): it runs about
a minute. Run with `TEST_10=1 pytest crates/binding/tests/test_10_delft.py -s` after
`maturin develop --release`; without `TEST_10=1` it is skipped.

The fixture (`fixtures/delft-tables/`, its README says where it comes from): Delft's network as
tables, `nodes.csv.gz` and `links.csv.gz` (21 566 nodes; 12 460 car, 20 652 bike and 55 400
walk links, each with one mode; no signals: a table carries none), `trips.csv.gz` (65 959 trips
with both ends inside Delft, leaving 07:30 to 08:30, four classes). Transit is the repository's
Delft feed. Every traveller owns a bike: the two car-owning classes choose among
car, bike, walk and transit, the other two among bike, walk and transit. The window is 07:30 to
09:30, every parameter at its default.

The five parts, each a test below:
  1. free-flow routes equal networkx's shortest times on the same tables: 1 000 pairs of nodes per
     mode (car, bike, walk), both ends on a node, car capacities unlimited, all leaving at 07:30;
  2. every trip has an alternative and finishes by 09:30 (each unfinished trip is printed);
  3. the run under 15 s (the run alone; the reads are printed);
  4. the trips and links files identical over two runs and at 1 and 8 threads (the one
     self-comparison);
  5. the stopped state within 1 % of a 60-iteration run: the mean trip time of each mode and
     each mode's share (relative differences).
For bikes: bikes are in every part: their routes in part 1 (length over 15 km/h on the bike links),
their trips counted in parts 2 and 4, their mean time and share judged in part 5.
"""

import csv
import gzip
import math
import os
import random
import subprocess
import sys
import time
from pathlib import Path

import networkx as nx
import pyarrow.parquet as pq
import pytest

import openmobisim as oms

FIXTURE = Path(__file__).resolve().parent / "fixtures" / "delft-tables"
pytestmark = pytest.mark.skipif(os.environ.get("TEST_10") != "1", reason="release only, by hand: TEST_10=1")
GTFS = Path(__file__).resolve().parents[2] / "io-gtfs" / "tests" / "fixtures" / "delft" / "gtfs.zip"
WINDOW = (27_000, 34_200)
MODES = {"car_and_bike": "car;bike;walk;transit", "car_only": "car;bike;walk;transit",
         "bike_only": "bike;walk;transit", "no_vehicle": "bike;walk;transit"}
FILES = ("trips.parquet", "links.parquet")
SPEED_KM_H = {"bike": 15.0, "walk": 4.8}  # PARAMETERS.md: bike_speed, walk_speed, their defaults
PAIRS, ORIGINS = 1_000, 50


def table(path, numbers=(), counts=(), optional=()):
    with gzip.open(path, "rt", newline="") as f:
        rows = list(csv.DictReader(f))
    out = {k: [r[k] for r in rows] for k in rows[0]}
    for k in numbers:
        out[k] = [float(x) if x else None for x in out[k]] if k in optional else [float(x) for x in out[k]]
    for k in counts:
        out[k] = [int(x) if x else None for x in out[k]]
    return out


def network():
    folder = FIXTURE
    nodes = table(folder / "nodes.csv.gz", numbers=("lon_deg", "lat_deg"))
    links = table(folder / "links.csv.gz", numbers=("length_m", "car_speed_km_h", "capacity_veh_h"),
                  counts=("lanes",), optional=("car_speed_km_h", "capacity_veh_h"))
    return oms.Network.from_tables(nodes, links)


def delft_trips():
    rows = table(FIXTURE / "trips.csv.gz", counts=("departure_s",))
    return {
        "trip_id": rows["trip_id"],
        "origin_lon_deg": [float(x) for x in rows["origin_lon"]],
        "origin_lat_deg": [float(x) for x in rows["origin_lat"]],
        "destination_lon_deg": [float(x) for x in rows["destination_lon"]],
        "destination_lat_deg": [float(x) for x in rows["destination_lat"]],
        "departure_s": rows["departure_s"], "class_id": rows["class_id"],
        "modes": [MODES[c] for c in rows["class_id"]],
    }


def delft_run(out, **parameters):
    """The fixture read and run once; returns the Run and the seconds of the reads and the run."""
    started = time.perf_counter()
    net = network()
    transit = oms.Transit.from_gtfs(GTFS, net)
    demand = oms.Demand.from_table(delft_trips(), sorted(MODES), net, transit=transit)
    read_s = time.perf_counter() - started
    started = time.perf_counter()
    result = oms.run(net, demand, transit, window=WINDOW, out=out, **parameters)
    return result, read_s, time.perf_counter() - started


def by_mode(result):
    trips = pq.read_table(result.trips).to_pylist()
    out = {}
    for mode in ("car", "bike", "walk", "transit"):
        times = [t["travel_time_s"] for t in trips if t["mode"] == mode and t["finished"]]
        share = sum(t["mode"] == mode for t in trips) / len(trips)
        out[mode] = (share, sum(times) / len(times) if times else None)
    return trips, out


@pytest.fixture(scope="module")
def default_run(tmp_path_factory):
    folder = tmp_path_factory.mktemp("default")
    result, read_s, run_s = delft_run(folder)
    iterations = pq.read_table(result.iterations).to_pylist()
    print(f"\nDelft: reads {read_s:.2f} s, run {run_s:.2f} s, {len(iterations)} iterations, "
          f"stopped by the rule: {result.stopped_by_rule}")
    return folder, result, run_s


def node_pairs(links, positions, mode, rng):
    """networkx's graph of the mode's links (time = length over the mode's free-flow speed; MultiDiGraph,
    so of parallel links the fastest counts), and PAIRS pairs inside its largest strongly connected
    component with networkx's shortest time: ORIGINS origins, each with PAIRS / ORIGINS destinations."""
    graph = nx.MultiDiGraph()
    for up, down, modes, length, car in zip(links["upstream"], links["downstream"], links["modes"],
                                            links["length_m"], links["car_speed_km_h"]):
        if mode in modes.split(";"):
            graph.add_edge(up, down, time=length / ((car if mode == "car" else SPEED_KM_H[mode]) / 3.6))
    largest = sorted(max(nx.strongly_connected_components(graph), key=len))
    pairs = []
    for origin in rng.sample(largest, ORIGINS):
        times = nx.single_source_dijkstra_path_length(graph, origin, weight="time")
        for destination in rng.sample([n for n in largest if n != origin], PAIRS // ORIGINS):
            pairs.append((positions[origin], positions[destination], times[destination]))
    return pairs


def test_1_free_flow_routes_of_every_mode_equal_networkx(tmp_path):
    folder = FIXTURE
    nodes = table(folder / "nodes.csv.gz", numbers=("lon_deg", "lat_deg"))
    links = table(folder / "links.csv.gz", numbers=("length_m", "car_speed_km_h", "capacity_veh_h"),
                  counts=("lanes",), optional=("car_speed_km_h", "capacity_veh_h"))
    positions = dict(zip(nodes["node_id"], zip(nodes["lon_deg"], nodes["lat_deg"])))
    rng, rows, want = random.Random(10), {k: [] for k in ("trip_id", "origin_lon_deg", "origin_lat_deg",
                                         "destination_lon_deg", "destination_lat_deg", "modes")}, {}
    started = time.perf_counter()
    for mode in ("car", "bike", "walk"):
        for i, (a, b, seconds) in enumerate(node_pairs(links, positions, mode, rng)):
            trip = f"{mode}-{i:04d}"
            for key, value in zip(rows, (trip, *a, *b, mode)):
                rows[key].append(value)
            want[trip] = (mode, seconds)
    networkx_s = time.perf_counter() - started
    n = len(want)
    rows.update(departure_s=[WINDOW[0]] * n, class_id=["all"] * n)
    # Unlimited capacity: no car waits behind another, so each time is its route's free-flow time (test 1).
    links["capacity_veh_h"] = [math.inf if c is not None else None for c in links["capacity_veh_h"]]
    started = time.perf_counter()
    net = oms.Network.from_tables(nodes, links)
    result = oms.run(net, oms.Demand.from_table(rows, ["all"], net), window=(WINDOW[0], WINDOW[0] + 6 * 3_600),
                     out=tmp_path, iteration_cap=1)
    got = {t["trip_id"]: t for t in pq.read_table(result.trips).to_pylist()}
    print(f"\n{n} trips: networkx {networkx_s:.2f} s, read and run {time.perf_counter() - started:.2f} s")
    off = []
    for mode in ("car", "bike", "walk"):
        diffs = [abs(got[t]["travel_time_s"] - s) for t, (m, s) in want.items() if m == mode and got[t]["finished"]]
        print(f"{mode}: {len(diffs)} finished of {PAIRS}, largest difference {max(diffs):.2e} s, "
              f"mean networkx time {sum(s for m, s in want.values() if m == mode) / PAIRS:.0f} s")
    for trip, (mode, seconds) in want.items():
        t = got[trip]
        if t["mode"] != mode or not t["finished"] or abs(t["travel_time_s"] - seconds) > 1e-6 * seconds:
            off.append((trip, t["mode"], t["travel_time_s"], round(seconds, 3)))
    assert not off, f"{len(off)} trips off networkx's time, e.g. {off[:5]}"


def test_2_and_3_every_trip_finishes_and_the_run_takes_under_15_s(default_run):
    _, result, run_s = default_run
    trips, modes = by_mode(result)
    for mode, (share, mean) in modes.items():
        print(f"{mode}: share {share:.3f}, mean trip {mean or 0:.0f} s")
    without = [t["trip_id"] for t in trips if t["mode"] is None]
    unfinished = [t for t in trips if t["mode"] is not None and not t["finished"]]
    for t in unfinished:
        print(f"unfinished: {t['trip_id']} {t['class_id']} leaves {t['departure_s']} by {t['mode']}, cost "
              f"{t['cost']:.2f}, {t['alternatives']} alternatives")
    assert not without, f"{len(without)} trips without an alternative, e.g. {without[:5]}"
    assert not unfinished, f"{len(unfinished)} trips unfinished at 09:30"
    assert run_s < 15.0, f"the run took {run_s:.2f} s (budget 15 s)"


def test_4_identical_files_twice_and_at_1_and_8_threads(default_run, tmp_path):
    first, _, _ = default_run
    folders = [first]
    _, _, run_s = delft_run(tmp_path / "second")
    print(f"\nsecond run {run_s:.2f} s")
    folders.append(tmp_path / "second")
    for threads in ("1", "8"):
        folder = tmp_path / f"threads-{threads}"
        started = time.perf_counter()
        subprocess.run([sys.executable, __file__, str(folder)], check=True,
                       env={**os.environ, "RAYON_NUM_THREADS": threads})
        print(f"{threads} thread(s): {time.perf_counter() - started:.2f} s, reads included")
        folders.append(folder)
    for name in FILES:
        files = [(folder / name).read_bytes() for folder in folders]
        assert all(f == files[0] for f in files), name


def test_5_the_stopped_state_within_1_percent_of_60_iterations(default_run, tmp_path):
    _, stopped, _ = default_run
    long, _, run_s = delft_run(tmp_path, iteration_floor=60, iteration_cap=60)
    assert len(pq.read_table(long.iterations)) == 60
    print(f"\n60 iterations in {run_s:.2f} s\nmode     share_stopped  share_60  mean_s_stopped  mean_s_60")
    off = []
    _, a = by_mode(stopped)
    _, b = by_mode(long)
    for mode in a:
        (sa, ma), (sb, mb) = a[mode], b[mode]
        print(f"{mode:8} {sa:13.4f}  {sb:8.4f}  {ma:14.1f}  {mb:9.1f}")
        for what, x, y in (("share", sa, sb), ("mean trip", ma, mb)):
            if abs(x - y) > 0.01 * abs(y):
                off.append((mode, what, round((x - y) / y * 100, 2)))
    assert not off, f"stopped state more than 1 % from the 60-iteration run (mode, quantity, %): {off}"


if __name__ == "__main__":
    delft_run(sys.argv[1])
