"""Checking a network before running it: ``network_check``.

For a network a user brings (a GMNS folder, a link table) or builds: what would make trips
fail, and what is worth a look on the map. It reads only the network's public arrays and
reports, so anything it flags can be looked at with the same arrays.
"""

from __future__ import annotations

from typing import Any

import numpy as np

from openmobisim import _core

__all__ = ["network_check"]

#: Road classes by index, as the core numbers them (`RoadClass::ALL`).
_CLASSES = (
    "motorway", "motorway_link", "trunk", "trunk_link", "primary", "primary_link",
    "secondary", "secondary_link", "tertiary", "tertiary_link", "unclassified",
    "residential", "living_street", "service", "pedestrian", "footway", "cycleway",
    "ferry", "busway",
)  # fmt: skip

#: Plausible median free-flow speeds (km/h) by road class, on links that do not end at a
#: signal (a signal's delay is part of a link's free-flow time). Judgement calls (S172).
_ROAD_SPEED = {
    "motorway": (50, 130), "trunk": (35, 110), "primary": (20, 90), "secondary": (18, 80),
    "tertiary": (15, 70), "unclassified": (10, 70), "residential": (8, 50),
    "living_street": (4, 30), "service": (4, 40),
}  # fmt: skip

#: Plausible median speeds (km/h) on the bike and walk layers, ferries left out.
_LAYER_SPEED = {"bike": (8.0, 30.0), "walk": (2.5, 7.0)}

_EARTH_M = 6_371_008.8


def _straight_m(coords: np.ndarray, offsets: np.ndarray) -> np.ndarray:
    """Great-circle metres between each link's first and last point."""
    a, b = coords[offsets[:-1]], coords[offsets[1:] - 1]
    lon1, lat1, lon2, lat2 = map(np.radians, (a[:, 0], a[:, 1], b[:, 0], b[:, 1]))
    h = (
        np.sin((lat2 - lat1) / 2) ** 2
        + np.cos(lat1) * np.cos(lat2) * np.sin((lon2 - lon1) / 2) ** 2
    )
    return 2 * _EARTH_M * np.arcsin(np.sqrt(h))


class _Checks:
    def __init__(self) -> None:
        self.rows: list[dict[str, Any]] = []

    def add(self, layer: str, name: str, ok: bool, value: str, rule: str, *, fail: bool = False):
        status = "pass" if ok else ("fail" if fail else "warn")
        self.rows.append({"layer": layer, "check": name, "status": status, "value": value,
                          "rule": rule})  # fmt: skip


def _share(x: float) -> str:
    return f"{x:.1%}"


def _common(checks: _Checks, net: _core.Network, layer: str, links: np.ndarray) -> None:
    """The checks every layer gets.

    Connected, no self-loops, positive lengths and times, lengths against the straight line.
    """
    mode = "car" if layer == "road" else "all"
    who = {"road": "car", "bike": "bike", "walk": "walk"}[layer]
    if not links.any():
        checks.add(layer, "has links", False, "none", f"at least one {who} link", fail=True)
        return
    conn = net.report_connectivity(mode)
    checks.add(
        layer, "strongly connected", bool(conn["strongly_connected"]),
        f"{conn['node_components']} piece(s), the largest {conn['node_largest']} of "
        f"{conn['nodes']} nodes; {conn['sources']} sources, {conn['sinks']} sinks",
        f"every node reaches every other by {who}: a {who} trip between two pieces has no route",
        fail=True,
    )  # fmt: skip
    frm, to = np.asarray(net.link_from())[links], np.asarray(net.link_to())[links]
    loops = int(np.sum(frm == to))
    checks.add(layer, "no self-loops", loops == 0, str(loops), "none", fail=True)
    length = np.asarray(net.link_length_m())[links]
    ff = np.asarray(net.link_free_flow_s())[links]
    bad = int(np.sum(~(length > 0) | ~(ff > 0) | ~np.isfinite(ff)))
    checks.add(layer, "lengths and times positive", bad == 0, f"{bad} links",
               "every link longer than 0 m and taking more than 0 s", fail=True)  # fmt: skip
    coords, offsets = net.link_geometry()
    straight = _straight_m(np.asarray(coords, float), np.asarray(offsets, np.int64))[links]
    far = straight > 5.0
    ratio = length[far] / straight[far]
    shorter = float(np.mean(ratio < 0.99)) if far.any() else 0.0
    checks.add(
        layer, "length not below the straight line", shorter < 0.005,
        f"{_share(shorter)} of links more than 1% shorter than the straight line between "
        "their ends",
        "under 0.5% (a shorter link is a length or a unit error)",
    )  # fmt: skip


def _road(checks: _Checks, net: _core.Network) -> dict[str, Any]:
    drivable = np.asarray(net.link_drivable())
    _common(checks, net, "road", drivable)
    if not drivable.any():
        return {"links": net.link_count, "nodes": net.node_count, "drivable_km": 0.0}
    frm, to = np.asarray(net.link_from()), np.asarray(net.link_to())
    length = np.asarray(net.link_length_m())
    ff = np.asarray(net.link_free_flow_s())
    cls = np.asarray(net.link_class())
    lanes = np.asarray(net.link_lanes()).astype(float)
    cap = np.asarray(net.link_capacity_pcu_h())
    signal = np.asarray(net.node_signalised())
    d = drivable
    short = float(np.mean(length[d] < 5.0))
    checks.add("road", "very short links", short < 0.03, f"{_share(short)} under 5 m",
               "under 3% of drivable links (control delay and storage are least reliable on "
               "short links)")  # fmt: skip
    key = frm[d].astype(np.int64) * net.node_count + to[d]
    parallel = (len(key) - len(np.unique(key))) / len(key)
    checks.add("road", "parallel links", parallel < 0.01, _share(parallel),
               "under 1% of drivable links share both their nodes with another")  # fmt: skip
    odd = []
    for c in np.unique(cls[d]):
        name = _CLASSES[int(c)] if int(c) < len(_CLASSES) else str(int(c))
        m = d & (cls == c)
        free = m & ~signal[to]
        m = free if free.sum() >= 20 else m
        base = name.replace("_link", "")
        if base in _ROAD_SPEED and m.sum() >= 20:
            speed = float(np.median(3.6 * length[m] / ff[m]))
            lo, hi = _ROAD_SPEED[base]
            if not lo <= speed <= hi:
                odd.append(f"{name} {speed:.0f} km/h")
    checks.add("road", "speeds plausible by class", not odd, ", ".join(odd) or "every class",
               "each class's median free-flow speed in a plausible range (a unit mix-up, mph "
               "for km/h, shows here)")  # fmt: skip
    per_lane = cap[d] / np.maximum(lanes[d], 1.0)
    out = float(np.mean((per_lane < 500) | (per_lane > 2400)))
    checks.add("road", "capacity per lane plausible", out < 0.02,
               f"{_share(out)} outside 500-2400 PCU/h per lane",
               "under 2% of drivable links")  # fmt: skip
    wide = int(np.sum(lanes[d] > 6))
    checks.add("road", "lanes per direction", wide == 0, f"{wide} links with more than 6",
               "none above 6")  # fmt: skip
    indeg = np.bincount(to[d], minlength=net.node_count)
    outdeg = np.bincount(frm[d], minlength=net.node_count)
    used = (indeg + outdeg) > 0
    ids = np.flatnonzero(d)
    in_of, out_of = np.zeros(net.node_count, np.int64), np.zeros(net.node_count, np.int64)
    in_of[to[ids]], out_of[frm[ids]] = ids, ids
    one = (indeg == 1) & (outdeg == 1)
    dead = int(np.sum(one & (frm[in_of] == to[out_of])))
    share = dead / max(int(used.sum()), 1)
    checks.add("road", "dead ends", share < 0.25, f"{_share(share)} of nodes",
               "under 25% of nodes are a street's end (many is a sign of over-mapping)",
               )  # fmt: skip
    return {
        "links": net.link_count, "nodes": net.node_count, "drivable_links": int(d.sum()),
        "drivable_km": round(float(length[d].sum()) / 1000, 1),
        "signalised_nodes": int(signal.sum()),
    }  # fmt: skip


def _layer(checks: _Checks, net: _core.Network, layer: str) -> dict[str, Any]:
    links = np.ones(net.link_count, bool)
    _common(checks, net, layer, links)
    length = np.asarray(net.link_length_m())
    out: dict[str, Any] = {"links": net.link_count, "nodes": net.node_count,
                           "km": round(float(length.sum()) / 1000, 1)}  # fmt: skip
    if net.link_count == 0:
        return out
    cls = np.asarray(net.link_class())
    speed = np.asarray(net.link_speed_km_h())
    land = cls != _CLASSES.index("ferry")
    if land.any():
        median = float(np.median(speed[land]))
        lo, hi = _LAYER_SPEED[layer]
        checks.add(layer, "speed plausible", lo <= median <= hi, f"median {median:.1f} km/h",
                   f"between {lo:g} and {hi:g} km/h")  # fmt: skip
    if layer == "bike":
        infra = np.asarray(net.link_infrastructure())
        total = max(float(length.sum()), 1e-9)
        out["separated_share"] = round(float(length[infra == 2].sum()) / total, 3)
        out["lane_share"] = round(float(length[infra == 1].sum()) / total, 3)
    return out


def network_check(network: _core.Network, *, layers: bool = True) -> dict[str, Any]:
    """Check a network before running it: what would make trips fail, and what to look at.

    For a network you bring (``network_read_gmns``, ``network_read_table``) or build. Each check
    is **pass**, **warn** (worth a look on the map, not necessarily wrong) or **fail** (trips
    will fail or a run is refused). On the road network, over the links a car may use: strongly
    connected, no self-loops, every length and free-flow time above 0, no link more than 1%
    shorter than the straight line between its ends, few very short or parallel links, each
    class's median speed plausible (a unit mix-up shows here), capacity per lane and lanes per
    direction plausible, few dead ends. On the bike and walk layers: connected, no self-loops,
    lengths and times above 0, lengths against the straight line, and a plausible median speed;
    the bike layer's summary says how much of it is separated or a painted lane. The thresholds
    are judgement calls, written next to each check (``rule``).

    Args:
        network: A road network.
        layers: Also check its bike and walk layers.

    Returns:
        ``{"verdict": "pass" | "warn" | "fail", "checks": [...], "summary": {...}}``: one row
        per check (``layer``, ``check``, ``status``, ``value``, ``rule``), the worst status as
        the verdict, and each layer's size.

    Raises:
        ValueError: If ``network`` is a layer, not a road network.
    """
    if network.layer_name != "road":
        raise ValueError(f"network_check needs a road network, not the {network.layer_name} layer")
    checks = _Checks()
    summary = {"road": _road(checks, network)}
    if layers:
        for name in ("bike", "walk"):
            summary[name] = _layer(checks, network.layer(name), name)
    statuses = {row["status"] for row in checks.rows}
    verdict = "fail" if "fail" in statuses else "warn" if "warn" in statuses else "pass"
    return {"verdict": verdict, "checks": checks.rows, "summary": summary}
