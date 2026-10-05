"""The loading against a reference: the macroscopic link transmission model (Yperman 2007).

The vehicle LTM moves whole vehicles; the macroscopic model moves flows, from cumulative
counts at each link's two ends, with a sending and a receiving flow per link and a node model
per node. On the textbook cases — a bottleneck whose queue spills back over two links, a
saturated merge, a diverge whose blocked branch holds back the other (first in, first out) —
both must count the same traffic leaving every link at every second, to within the whole
vehicles one moves and the other splits. The reference is written here, from the papers,
and reads only the network's public arrays: each link's length, free-flow time, capacity
and storage, from which its jam density and backward wave speed follow (the triangle
``C = v·w·k_j / (v + w)``).

Node models: a series node passes ``min(sending, receiving)``; a merge gives each approach
room in proportion to its capacity (Daganzo 1995; the loading's rule, S77), what one leaves
going to the other; a diverge is first in, first out, its flow ``min(S, R_b / p_b, R_c / p_c)``
for turning shares ``p`` (Daganzo 1995).
"""

from __future__ import annotations

import math

import numpy as np
import openmobisim as ms

LAT = 52.0
HORIZON = 3000


def lonlat(x: float, y: float = 0.0) -> tuple[float, float]:
    return 4.0 + x / (111_320.0 * math.cos(math.radians(LAT))), LAT + y / 110_574.0


class Link:
    """One link of the macroscopic model.

    Cumulative counts in at its upstream end and out at its downstream end, at whole seconds.
    """

    def __init__(self, length: float, free_flow_s: float, capacity_h: float, storage: float):
        """A link from its length (m), free-flow time (s), capacity (PCU/h) and storage (PCU)."""
        self.length, self.v = length, length / free_flow_s
        self.capacity, self.storage = capacity_h / 3600.0, storage
        self.w = self.capacity / (storage / length - self.capacity / self.v)
        self.up, self.down = [0.0], [0.0]

    @staticmethod
    def at(counts: list[float], t: float) -> float:
        """A cumulative count at time ``t``, linear between whole seconds."""
        if t <= 0:
            return 0.0
        i = math.floor(t)
        if i + 1 >= len(counts):
            return counts[-1]
        return counts[i] + (counts[i + 1] - counts[i]) * (t - i)

    def sending(self, k: int) -> float:
        """What the link can send in second ``k``: what has had time to reach its end."""
        free = self.at(self.up, k + 1 - self.length / self.v) - self.down[k]
        return max(0.0, min(free, self.capacity))

    def receiving(self, k: int) -> float:
        """What it can take in second ``k``: the room that has had time to reach its start."""
        room = self.at(self.down, k + 1 - self.length / self.w) + self.storage - self.up[k]
        return max(0.0, min(room, self.capacity))


def reference(net: object, nodes: list[tuple], demand: dict[int, np.ndarray]) -> np.ndarray:
    """Every link's cumulative outflow, second by second, by the macroscopic LTM."""
    arrays = (
        net.link_length_m,
        net.link_free_flow_s,
        net.link_capacity_pcu_h,
        net.link_storage_pcu,
    )
    links = [Link(*row) for row in zip(*(np.asarray(f()) for f in arrays), strict=True)]
    for k in range(HORIZON):
        inflow, outflow = [0.0] * len(links), [0.0] * len(links)
        for kind, ins, outs, share in nodes:
            if kind == "origin":
                (o,) = outs
                inflow[o] = max(0.0, min(demand[o][k + 1] - links[o].up[k], links[o].receiving(k)))
            elif kind == "series":
                (i,), (o,) = ins, outs
                inflow[o] = outflow[i] = min(links[i].sending(k), links[o].receiving(k))
            elif kind == "sink":
                (i,) = ins
                outflow[i] = links[i].sending(k)
            elif kind == "merge":
                (a, b), (o,) = ins, outs
                sa, sb, r = links[a].sending(k), links[b].sending(k), links[o].receiving(k)
                qa, qb = sa, sb
                if sa + sb > r:
                    ca, cb = links[a].capacity, links[b].capacity
                    qa = min(sa, max(r * ca / (ca + cb), r - sb))
                    qb = min(sb, max(r * cb / (ca + cb), r - sa))
                outflow[a], outflow[b], inflow[o] = qa, qb, qa + qb
            elif kind == "diverge":
                (i,), (b, c) = ins, outs
                q = min(
                    links[i].sending(k),
                    links[b].receiving(k) / share,
                    links[c].receiving(k) / (1 - share),
                )
                outflow[i], inflow[b], inflow[c] = q, q * share, q * (1 - share)
        for n, link in enumerate(links):
            link.up.append(link.up[k] + inflow[n])
            link.down.append(link.down[k] + outflow[n])
    return np.array([link.down for link in links])


def build(node_xy: dict[str, tuple[float, float]], rows: list[tuple]) -> object:
    nodes = [{"id": n, "lon": lonlat(*xy)[0], "lat": lonlat(*xy)[1]} for n, xy in node_xy.items()]
    links = [
        {"id": lid, "from": a, "to": b, "length": 1000.0, "free_flow_speed": 50, "lanes": lanes,
         "capacity": cap, "class": "primary"}
        for lid, a, b, lanes, cap in rows
    ]  # fmt: skip
    return ms.network_read_table(links, nodes)


def simulated(
    net: object, trips: list[tuple], loading_options: dict[str, float] | None = None
) -> np.ndarray:
    """Every link's cumulative outflow, second by second, by the vehicle LTM (level 4)."""
    run = ms.Scenario.from_parts(
        net,
        trips,
        class_defaults={"x": (True, False, False)},
        # The plain single loading: everyone at once on free flow, the full model (S223).
        equilibration="free_flow",
        equilibration_options={"increments": 1, "warmup": 0},
        flow_level=4,
        choice_model="deterministic",
        link_bin_s=1,
        window_hours=HORIZON / 3600,
        loading_options=loading_options,
    ).run("ltm-reference")
    bins = run.link_bins()
    out = np.zeros((net.link_count, HORIZON + 1))
    for b, link, pcu in zip(bins.bins(), bins.links(), bins.pcu(), strict=True):
        if b < HORIZON:
            out[link, b + 1] += pcu
    return np.cumsum(out, axis=1)


def departures(n: int, headway: float) -> list[int]:
    return [math.floor(i * headway) for i in range(n)]


def counts(deps: list[int]) -> np.ndarray:
    d = np.zeros(HORIZON + 1)
    for t in deps:
        d[t + 1 :] += 1
    return d


def trip(who: str, o: tuple[float, float], d: tuple[float, float], t: int) -> tuple:
    return (who, 0, o[0], o[1], d[0], d[1], t, "x", None, "car")


def test_a_bottleneck_queue_spills_back_over_two_links_as_the_reference_has_it() -> None:
    # 2 400 PCU/h for 15 min into a 600 PCU/h link: its queue fills the link before it and
    # spills into the first, and the origin waits.
    node_xy = {n: (1000.0 * i, 0.0) for i, n in enumerate("ABCDE")}
    rows = [("1", "A", "B", 2, 3600), ("2", "B", "C", 2, 3600), ("3", "C", "D", 1, 600),
            ("4", "D", "E", 2, 3600)]  # fmt: skip
    net = build(node_xy, rows)
    deps = departures(600, 1.5)
    o, d = lonlat(0), lonlat(4000)
    sim = simulated(net, [trip(f"v{i}", o, d, t) for i, t in enumerate(deps)])
    nodes = [("origin", (), (0,), None), ("series", (0,), (1,), None), ("series", (1,), (2,), None),
             ("series", (2,), (3,), None), ("sink", (3,), (), None)]  # fmt: skip
    ref = reference(net, nodes, {0: counts(deps)})
    assert sim[2, -1] < 600, "the bottleneck is still draining at the horizon"
    assert np.abs(sim - ref).max() <= 1.0, np.abs(sim - ref).max(axis=1)


def test_a_saturated_merge_shares_room_in_proportion_to_capacity_as_the_reference_does() -> None:
    node_xy = {"A": (0, 500), "B": (0, -500), "M": (1000, 0), "E": (2000, 0)}
    net = build(
        node_xy, [("1", "A", "M", 2, 3600), ("2", "B", "M", 1, 1200), ("3", "M", "E", 1, 1800)]
    )
    deps = departures(250, 2.4)
    a, b, d = lonlat(0, 500), lonlat(0, -500), lonlat(2000)
    trips = [trip(f"a{i}", a, d, t) for i, t in enumerate(deps)]
    trips += [trip(f"b{i}", b, d, t) for i, t in enumerate(deps)]
    sim = simulated(net, trips)
    nodes = [("origin", (), (0,), None), ("origin", (), (1,), None), ("merge", (0, 1), (2,), None),
             ("sink", (2,), (), None)]  # fmt: skip
    ref = reference(net, nodes, {0: counts(deps), 1: counts(deps)})
    assert np.abs(sim - ref).max() <= 1.0, np.abs(sim - ref).max(axis=1)


def test_a_blocked_branch_holds_back_the_other_first_in_first_out_as_the_reference_has_it() -> None:
    # Half the traffic turns towards a 600 PCU/h bottleneck; its queue fills the branch and the
    # vehicles waiting for it at the diverge hold back those bound for the free branch. The
    # reference's diverge is first in, first out: turn pockets (S217) off.
    node_xy = {"A": (0, 0), "B": (1000, 0), "C": (2000, 500), "D": (2000, -500), "E": (3000, -500)}
    rows = [("1", "A", "B", 2, 3600), ("2", "B", "C", 2, 3600), ("3", "B", "D", 1, 1800),
            ("4", "D", "E", 1, 600)]  # fmt: skip
    net = build(node_xy, rows)
    deps = departures(400, 1.5)
    o, c, e = lonlat(0), lonlat(2000, 500), lonlat(3000, -500)
    trips = [trip(f"v{i}", o, c if i % 2 == 0 else e, t) for i, t in enumerate(deps)]
    sim = simulated(net, trips, {"pocket_length_m": 0})
    nodes = [("origin", (), (0,), None), ("diverge", (0,), (1, 2), 0.5), ("sink", (1,), (), None),
             ("series", (2,), (3,), None), ("sink", (3,), (), None)]  # fmt: skip
    ref = reference(net, nodes, {0: counts(deps)})
    # The free branch's traffic leaves late: unblocked, its last vehicle (leaving at 598 s) would
    # be through at 742 s, two free-flow links later; held back first in, first out, it is not.
    assert np.argmax(sim[1] >= 200) > 742 + 50
    # Whole vehicles alternating between branches against a flow split in half: within two.
    assert np.abs(sim - ref).max() <= 2.0 + 1e-6, np.abs(sim - ref).max(axis=1)
