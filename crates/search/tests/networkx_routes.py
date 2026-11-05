"""Test 9's expected routes, from networkx: shortest paths by length on a jittered grid.

Kept for reproducibility; it does not run in CI. Standard library and networkx only:

    python networkx_routes.py > fixtures/networkx_routes.txt

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


if __name__ == "__main__":
    main()
