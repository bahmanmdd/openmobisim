"""Bike and walk trips on a jittered grid, built as tables, against networkx's shortest paths.

The derivation of a trip's door-to-door time (the loop's README: a walk off the network, a route
on it, a walk off it; the route entered at the instant the first walk ends):

    travel_time_s = g_o / w + L / v + g_d / w

where g_o and g_d are the straight lines from the trip's origin and destination to their nodes
(each end placed at most 8 m from a node, and every other node at least 60 m from that one, so
the nearest node is that node), w = 4.8 km/h = 4 800 / 3 600 m/s the walking speed (gaps are
walked by every mode), L networkx's shortest length (Dijkstra by length) between the two nodes
on the mode's graph, and v the mode's speed: 15 km/h = 15 000 / 3 600 m/s for a bike, w for a
walker. Bikes and walkers ride and walk at free flow, so no one else changes these times.

For bikes: 15 % of the streets are one-way for bikes and two-way for walkers, so a bike's route
is its own (on the bike graph) and differs from a walker's on the same pair; the trips table is a
pyarrow table, the second way of giving a table.
"""

import math
import random

import pyarrow as pa
import pyarrow.parquet as pq
import networkx as nx

import openmobisim as oms
from grid import grid

W = 4.8 * 1_000 / 3_600
SPEED = {"bike": 15.0 * 1_000 / 3_600, "walk": W}
START = 28_800


def test_bike_and_walk_times_equal_networkx_plus_the_walked_gaps(tmp_path):
    nodes, links, pos, graphs = grid(12, seed=9)
    rng = random.Random(11)
    ids = list(pos)
    rows, want = [], {}
    differ = 0
    for i in range(60):
        mode = "bike" if i % 2 == 0 else "walk"
        o, d = rng.sample(ids, 2)
        ends = []
        for node in (o, d):
            r, a = rng.uniform(2.0, 8.0), rng.uniform(0.0, 2 * math.pi)
            x, y = pos[node][0] + r * math.cos(a), pos[node][1] + r * math.sin(a)
            ends.append((x, y, math.hypot(x - pos[node][0], y - pos[node][1])))
        (ox, oy, g_o), (dx, dy, g_d) = ends
        first = g_o / W
        length = nx.shortest_path_length(graphs[mode], o, d, weight="length")
        differ += length != nx.shortest_path_length(graphs["walk" if mode == "bike" else "bike"],
                                                    o, d, weight="length")
        trip = f"t{i:02d}"
        want[trip] = (mode, first + length / SPEED[mode] + g_d / W)
        rows.append((trip, ox, oy, dx, dy, START + rng.randrange(600), "all", mode))
    assert differ > 0, "the bike and walk graphs give the same length on every pair"
    columns = ["trip_id", "origin_x_m", "origin_y_m", "destination_x_m", "destination_y_m",
               "departure_s", "class_id", "modes"]
    trips = pa.table({c: [r[k] for r in rows] for k, c in enumerate(columns)})

    network = oms.Network.from_tables(nodes, links, bike_speed=15.0, walk_speed=4.8)
    demand = oms.Demand.from_table(trips, ["all"], network)
    result = oms.run(network, demand, window=(START, START + 7_200), out=tmp_path)

    got = pq.read_table(result.trips).to_pylist()
    assert len(got) == len(want)
    worst = 0.0
    for row in got:
        mode, seconds = want[row["trip_id"]]
        assert row["finished"] and row["mode"] == mode, row
        worst = max(worst, abs(row["travel_time_s"] - seconds))
        assert abs(row["travel_time_s"] - seconds) <= 1e-6, (row, seconds)
    print(f"60 trips (30 by bike, 30 on foot): largest difference from networkx {worst:.1e} s; "
          f"{differ} pairs where the bike's length is not the walker's")
