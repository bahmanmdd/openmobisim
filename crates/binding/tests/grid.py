"""A jittered grid written as the binding's tables, with networkx's graph of each mode on it, and a
small logit run on it. Shared by test_networkx.py and test_determinism.py.

The grid: n x n nodes 100 m apart, each moved by up to 20 m in x and in y (so two nodes are at
least 60 m apart), positions in metres; a street each way between neighbours, its length the
straight line rounded to 0.01 m. Walkers use every street both ways. On a share of the streets,
drawn, bikes may ride one way only (a one-way street for bikes): the bike graph differs from the
walk graph. With cars, every link also carries cars: one lane at 30 km/h passing 600 veh/h.
"""

import random
import sys

import networkx as nx

SPACING, JITTER = 100.0, 20.0
LINK_COLUMNS = [
    "link_id", "upstream", "downstream", "modes", "length_m", "lanes", "car_speed_km_h",
    "capacity_veh_h",
]


def grid(n, seed, one_way_bike_share=0.15, cars=False):
    """The nodes and links tables (dicts of lists), the positions by node id, and networkx's
    directed graph per mode ("bike", "walk") with each link's length."""
    rng = random.Random(seed)
    pos = {}
    for r in range(n):
        for c in range(n):
            dx, dy = rng.uniform(-JITTER, JITTER), rng.uniform(-JITTER, JITTER)
            pos[f"n{r * n + c}"] = (c * SPACING + dx, r * SPACING + dy)
    nodes = {
        "node_id": list(pos),
        "x_m": [x for x, _ in pos.values()],
        "y_m": [y for _, y in pos.values()],
    }
    links = {c: [] for c in LINK_COLUMNS}
    graphs = {"bike": nx.DiGraph(), "walk": nx.DiGraph()}
    for r in range(n):
        for c in range(n):
            a = r * n + c
            for b in ([a + 1] if c + 1 < n else []) + ([a + n] if r + 1 < n else []):
                (xa, ya), (xb, yb) = pos[f"n{a}"], pos[f"n{b}"]
                length = round(((xa - xb) ** 2 + (ya - yb) ** 2) ** 0.5, 2)
                one_way = rng.random() < one_way_bike_share
                for back, (u, v) in enumerate(((f"n{a}", f"n{b}"), (f"n{b}", f"n{a}"))):
                    modes = ["walk"]
                    graphs["walk"].add_edge(u, v, length=length)
                    if not (one_way and back):
                        modes.append("bike")
                        graphs["bike"].add_edge(u, v, length=length)
                    if cars:
                        modes.append("car")
                    row = [f"{u}-{v}", u, v, ";".join(modes), length]
                    row += [1, 30.0, 600.0] if cars else [None, None, None]
                    for column, value in zip(LINK_COLUMNS, row):
                        links[column].append(value)
    return nodes, links, pos, graphs


def logit_run(out):
    """The determinism test's run: an 8 x 8 grid with cars, 400 trips open to car, bike and walk
    between drawn nodes, two classes, the default choice model (the nested logit), 8 iterations
    at most, written into `out`."""
    import openmobisim as oms

    nodes, links, pos, _ = grid(8, seed=3, cars=True)
    rng = random.Random(4)
    ids = list(pos)
    trips = {c: [] for c in ["trip_id", "origin_x_m", "origin_y_m", "destination_x_m",
                             "destination_y_m", "departure_s", "class_id", "modes"]}
    for i in range(400):
        o, d = rng.sample(ids, 2)
        row = [f"t{i:03d}", *pos[o], *pos[d], 28_800 + rng.randrange(900), "ab"[i % 2],
               "car;bike;walk"]
        for column, value in zip(trips, row):
            trips[column].append(value)
    network = oms.Network.from_tables(nodes, links)
    demand = oms.Demand.from_table(trips, ["a", "b"], network)
    return oms.run(network, demand, window=(28_800, 32_400), out=out, iteration_cap=8)


if __name__ == "__main__":
    logit_run(sys.argv[1])
