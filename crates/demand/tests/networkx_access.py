"""Test 2's expected access and egress lists, from networkx: bounded Dijkstra plus the gaps.

Kept for reproducibility; it does not run in CI. Standard library and networkx only:

    python networkx_access.py > fixtures/networkx_access.txt

A 20 x 20 grid of nodes 100 m apart, each moved by up to 20 m in x and in y (seed 7); a link
each way between neighbours, its length the straight line rounded to 0.01 m; on 10 % of the
streets, drawn, walking is allowed one way only (the other link is car and bike). 6 stops and
25 trips (50 ends) at random points, off the nodes. Each stop and trip end is linked to its
nearest walk node by a scan of every node (ties to the lower name, which is the lower id); the
gap is the straight line to it. A trip's access list: each stop whose walk, origin gap plus
networkx's shortest walk by length from the origin's node to the stop's node
(single_source_dijkstra with a cutoff) plus the stop's gap, is at most 600 m; the egress list
the same from each stop's node to the destination's (on the reversed graph). No walk is within
0.01 m of 600 m (checked), so the lists do not hang on rounding.
Output, one record per line:
    node ID X Y                        (metres, repr)
    link ID FROM TO LENGTH MODES       (metres; MODES cbw or cb)
    stop ID X Y NODE GAP
    trip ID OX OY DX DY ONODE OGAP DNODE DGAP
    access TRIP STOP:METRES ...        (the whole walk, gaps included)
    egress TRIP STOP:METRES ...
"""

import math
import random

import networkx as nx

N, SPACING, JITTER, ONE_WAY_SHARE = 20, 100.0, 20.0, 0.1
STOPS, TRIPS, MAX, MARGIN = 6, 25, 600.0, 0.01


def nearest(pos, walk_nodes, p):
    return min(
        (math.hypot(p[0] - x, p[1] - y), name) for name, (x, y) in pos.items() if name in walk_nodes
    )


def main():
    rng = random.Random(7)
    name = lambda k: f"n{k:03}"
    pos = {}
    for r in range(N):
        for c in range(N):
            dx, dy = rng.uniform(-JITTER, JITTER), rng.uniform(-JITTER, JITTER)
            pos[name(r * N + c)] = (c * SPACING + dx, r * SPACING + dy)
    g = nx.DiGraph()
    links = []
    for r in range(N):
        for c in range(N):
            a = r * N + c
            for b in ([a + 1] if c + 1 < N else []) + ([a + N] if r + 1 < N else []):
                (xa, ya), (xb, yb) = pos[name(a)], pos[name(b)]
                length = round(math.hypot(xa - xb, ya - yb), 2)
                one_way = rng.random() < ONE_WAY_SHARE
                for i, (u, v) in enumerate(((name(a), name(b)), (name(b), name(a)))):
                    walk = not (one_way and i == 1)
                    if walk:
                        g.add_edge(u, v, length=length)
                    links.append((u, v, length, "cbw" if walk else "cb"))
    walk_nodes = set(g.nodes)
    reverse = g.reverse(copy=True)
    point = lambda lo, hi: (rng.uniform(lo, hi), rng.uniform(lo, hi))
    stops = []
    for s in range(STOPS):
        p = point(0.0, 1900.0)
        gap, node = nearest(pos, walk_nodes, p)
        stops.append((f"s{s}", p, node, gap))
    lines = [f"node {k} {x!r} {y!r}" for k, (x, y) in sorted(pos.items())]
    lines += [f"link {i} {u} {v} {ln!r} {m}" for i, (u, v, ln, m) in enumerate(links)]
    lines += [f"stop {s} {p[0]!r} {p[1]!r} {node} {gap!r}" for s, p, node, gap in stops]
    lists, one_way_matters = [], False
    for t in range(TRIPS):
        o, d = point(-50.0, 1950.0), point(-50.0, 1950.0)
        (og, on), (dg, dn) = nearest(pos, walk_nodes, o), nearest(pos, walk_nodes, d)
        trip = f"t{t:02}"
        lines.append(f"trip {trip} {o[0]!r} {o[1]!r} {d[0]!r} {d[1]!r} {on} {og!r} {dn} {dg!r}")
        for kind, node, gap, graph in (("access", on, og, g), ("egress", dn, dg, reverse)):
            every = nx.single_source_dijkstra_path_length(graph, node, weight="length")
            near = nx.single_source_dijkstra_path_length(
                graph, node, cutoff=MAX - gap, weight="length"
            )
            kept = []
            for s, _, s_node, s_gap in stops:
                whole = gap + every[s_node] + s_gap
                assert abs(whole - MAX) > MARGIN, f"{trip} {kind} {s}: {whole} m, near {MAX}"
                if whole <= MAX:
                    assert s_node in near
                    kept.append(f"{s}:{gap + near[s_node] + s_gap!r}")
            flipped = reverse if graph is g else g
            other = nx.single_source_dijkstra_path_length(flipped, node, weight="length")
            one_way_matters |= any(other[s_node] != every[s_node] for _, _, s_node, _ in stops)
            lists.append(len(kept))
            lines.append(f"{kind} {trip} " + " ".join(kept))
    assert one_way_matters, "the one-way walk links change no walk to a stop"
    assert lists.count(0) >= 5 and sum(n >= 2 for n in lists) >= 5, f"lists of {lists} stops"
    print("\n".join(lines))


if __name__ == "__main__":
    main()
