"""Test 9's expected routes and reaches, from networkx: shortest paths by length on jittered grids.

Kept for reproducibility; it does not run in CI. Standard library and networkx only:

    python networkx_routes.py > fixtures/networkx_routes.txt
    python networkx_routes.py reach > fixtures/networkx_reach.txt

The routes:

A 12 x 12 grid of nodes 100 m apart, each moved by up to 20 m in x and in y; a link each way
between neighbours, its length the straight line rounded to 0.01 m (all modes, 1 lane,
30 km/h in the Rust test). 20 origin-destination pairs at least 4 rows and 4 columns apart,
each with exactly one shortest path by length (checked), written with its length.
Output, one record per line:
    node ID X Y                  (metres, repr: read back exactly)
    link ID FROM TO LENGTH       (metres)
    pair ORIGIN DESTINATION LENGTH NODE NODE ...
"""

import random
import sys

import networkx as nx

N, SPACING, JITTER, PAIRS = 12, 100.0, 20.0, 20


def main():
    rng = random.Random(9)
    pos = {}
    for r in range(N):
        for c in range(N):
            dx, dy = rng.uniform(-JITTER, JITTER), rng.uniform(-JITTER, JITTER)
            pos[r * N + c] = (c * SPACING + dx, r * SPACING + dy)
    g = nx.DiGraph()
    links = []
    for r in range(N):
        for c in range(N):
            a = r * N + c
            for b in ([a + 1] if c + 1 < N else []) + ([a + N] if r + 1 < N else []):
                (xa, ya), (xb, yb) = pos[a], pos[b]
                length = round(((xa - xb) ** 2 + (ya - yb) ** 2) ** 0.5, 2)
                for u, v in ((a, b), (b, a)):
                    g.add_edge(u, v, length=length)
                    links.append((u, v, length))
    lines = [f"node {n} {x!r} {y!r}" for n, (x, y) in sorted(pos.items())]
    lines += [f"link {i} {u} {v} {length!r}" for i, (u, v, length) in enumerate(links)]
    found = 0
    while found < PAIRS:
        o, d = rng.randrange(N * N), rng.randrange(N * N)
        if abs(o // N - d // N) < 4 or abs(o % N - d % N) < 4:
            continue
        paths = list(nx.all_shortest_paths(g, o, d, weight="length"))
        assert len(paths) == 1, f"pair {o} {d} has {len(paths)} shortest paths"
        length = nx.path_weight(g, paths[0], weight="length")
        lines.append(f"pair {o} {d} {length!r} " + " ".join(map(str, paths[0])))
        found += 1
    print("\n".join(lines))


REACH_N, ONE_WAY_SHARE, ORIGINS, MARGIN = 10, 0.15, 8, 0.01
CUTOFF = {"walk": 450.0, "bike": 600.0}


def reach():
    """The reaches: a 10 x 10 grid jittered as above (seed 10), a link each way between
    neighbours; on 15 % of the streets, drawn, walking is allowed one way only (the other link
    is car and bike). 8 origins drawn; for each, networkx's single_source_dijkstra by length with
    a cutoff of 450 m on foot and 600 m by bike from it, and on foot on the reversed graph to it
    (the distance from each node to it). No node is within 0.01 m of a cutoff (checked), so
    the sets do not hang on rounding; forward and backward differ for some origin (checked).
    Output, one record per line:
        node ID X Y                     (metres, repr)
        link ID FROM TO LENGTH MODES    (metres; MODES cbw or cb)
        reach ORIGIN MODE CUTOFF NODE:METRES ...       (forward, METRES from the origin)
        back DESTINATION MODE CUTOFF NODE:METRES ...   (backward, METRES to the destination)
    """
    n = REACH_N
    rng = random.Random(10)
    pos = {}
    for r in range(n):
        for c in range(n):
            dx, dy = rng.uniform(-JITTER, JITTER), rng.uniform(-JITTER, JITTER)
            pos[r * n + c] = (c * SPACING + dx, r * SPACING + dy)
    graphs = {"walk": nx.DiGraph(), "bike": nx.DiGraph()}
    links = []
    for r in range(n):
        for c in range(n):
            a = r * n + c
            for b in ([a + 1] if c + 1 < n else []) + ([a + n] if r + 1 < n else []):
                (xa, ya), (xb, yb) = pos[a], pos[b]
                length = round(((xa - xb) ** 2 + (ya - yb) ** 2) ** 0.5, 2)
                one_way = rng.random() < ONE_WAY_SHARE
                for i, (u, v) in enumerate(((a, b), (b, a))):
                    walk = not (one_way and i == 1)
                    graphs["bike"].add_edge(u, v, length=length)
                    if walk:
                        graphs["walk"].add_edge(u, v, length=length)
                    links.append((u, v, length, "cbw" if walk else "cb"))
    lines = [f"node {k} {x!r} {y!r}" for k, (x, y) in sorted(pos.items())]
    lines += [f"link {i} {u} {v} {ln!r} {m}" for i, (u, v, ln, m) in enumerate(links)]
    differ = False
    for o in rng.sample(range(n * n), ORIGINS):
        found = {}
        for kind, mode, graph in (
            ("reach", "walk", graphs["walk"]),
            ("reach", "bike", graphs["bike"]),
            ("back", "walk", graphs["walk"].reverse(copy=True)),
        ):
            cutoff = CUTOFF[mode]
            every = nx.single_source_dijkstra_path_length(graph, o, weight="length")
            near = [d for d in every.values() if abs(d - cutoff) <= MARGIN]
            assert not near, f"{kind} {o} {mode}: a node within {MARGIN} m of the cutoff"
            dist = nx.single_source_dijkstra_path_length(graph, o, cutoff=cutoff, weight="length")
            assert dist == {k: d for k, d in every.items() if d <= cutoff}
            found[(kind, mode)] = dist
            body = " ".join(f"{k}:{d!r}" for k, d in sorted(dist.items()))
            lines.append(f"{kind} {o} {mode} {cutoff!r} {body}")
        differ |= found[("reach", "walk")] != found[("back", "walk")]
    assert differ, "forward and backward reaches on foot agree for every origin"
    print("\n".join(lines))


if __name__ == "__main__":
    reach() if sys.argv[1:] == ["reach"] else main()
