"""The map's trips between two places: each origin-destination pair's journeys, by every mode.

A pair is two nodes of the road layer (the pairs of the car route sets). Its **options** are the
journeys its travellers took, one per distinct way (a car route; a sequence of transit lines
from the same stops; a car or bike to a parking, then transit; a bike route; a walk; a shared
bike; a taxi), each with its travellers and its expected times, plus the car routes of the
pair's route set nobody took. Each option is a list of **legs** the page draws and lays out as
a strip: a drive, a ride on a bike or a walk along the links of its layer, a ride on a transit
line from stop to stop, a wait, and the time to park or fetch the vehicle.

**A walk to or from a stop, a dock or a parking** (and between two stops) is found when the map
is written, on the walk layer, by the search the run walks with: from the walk node nearest the
place it leaves to the one nearest the place it reaches (the pair's origin or destination, the
stop, the parking, the dock), as the run snaps them. The walk layer's times are fixed for a run,
so this is the path the run's walkers took (for a pair's origin and destination, from the pair's
road nodes, which stand for its travellers' doors). It is drawn along its links, from the place
it leaves to the place it reaches, and as a straight line only where the walk layer has no path
(or its links are not on the page).
"""

from __future__ import annotations

from collections import defaultdict
from typing import Any

import numpy as np

from openmobisim.viz import _geometry as geo

#: Leg kinds, as the page reads them.
LEG_DRIVE, LEG_BIKE, LEG_WALK, LEG_RIDE, LEG_CONNECT, LEG_WAIT, LEG_PARK = range(7)
#: The options' modes, in the order the page groups them.
OPTION_MODES = (
    "car",
    "taxi",
    "car_transit",
    "transit",
    "bike_transit",
    "bike_share_transit",
    "bike",
    "bike_share",
    "walk",
)
#: A pair keeps its options with at least this share of its travellers (and at most
#: `_MAX_OPTIONS` of them); the rest are counted as "other journeys".
_MIN_SHARE = 0.01
_MAX_OPTIONS = 14
_NONE = 2**32 - 1
#: How far a walk between places is searched on the walk layer: twice its mean seconds and this
#: slack (a pair's road node stands for its travellers' doors, a little off them), at most an
#: hour. A walk not found within it is drawn straight.
_WALK_SEARCH_SLACK_S = 600.0
_WALK_SEARCH_MAX_S = 3600.0


def _node_lonlat(network: Any) -> np.ndarray:
    """Each road node's WGS84 ``(lon, lat)``, from the links' first and last vertices."""
    coords, offsets = network.link_geometry()
    first = coords[offsets[:-1].astype(np.int64)]
    last = coords[offsets[1:].astype(np.int64) - 1]
    n = int(max(network.link_from().max(), network.link_to().max())) + 1
    out = np.full((n, 2), np.nan)
    out[network.link_from()] = first
    out[network.link_to()] = last
    return out


def _labels(points: np.ndarray, stops: Any, stop_xy: np.ndarray | None) -> list[str]:
    """A name for each place: its nearest stop's name within 400 m, else its coordinates' grid."""
    out = []
    for x, y in points:
        name = ""
        if stop_xy is not None and len(stop_xy):
            d = np.hypot(stop_xy[:, 0] - x, stop_xy[:, 1] - y)
            k = int(np.argmin(d))
            if d[k] <= 400:
                name = str((stops.get("name") or stops["stop_id"])[k] or stops["stop_id"][k])
                if d[k] > 60:
                    name = "near " + name
        if not name:  # no stop near: where it lies from the network's centre (the page's origin)
            km = float(np.hypot(x, y)) / 1000
            compass = ("E", "NE", "N", "NW", "W", "SW", "S", "SE")[
                int(round(np.arctan2(y, x) / (np.pi / 4))) % 8
            ]
            name = f"{km:.1f} km {compass} of the centre" if km >= 0.3 else "the centre"
        out.append(name)
    return out


def add_trips(
    blob: Any,
    run: Any,
    network: Any,
    keep: np.ndarray,
    new_id: np.ndarray,
    max_pairs: int,
    lon0: float,
    lat0: float,
    layer_index: dict[str, np.ndarray],
    route_sets: Any = None,
) -> dict[str, Any] | None:
    """Embed the pairs' options; return what the page needs to know of them (``None``: none).

    `layer_index` maps a bike or walk layer link to its position among the embedded links of that
    layer (-1: not embedded); `route_sets`, when given, replaces the run's (its car routes only).
    """
    sets = run.route_sets() if route_sets is None else route_sets
    journeys = run.itinerary_journeys() if route_sets is None else None
    trips = run.trip_results()
    weight_of = np.zeros(int(np.max(trips["trip"])) + 1 if len(trips["trip"]) else 0)
    weight_of[np.asarray(trips["trip"], dtype=np.int64)] = np.asarray(trips["weight"], dtype=float)

    # The route sets' pairs, and their routes by links.
    key_of: dict[tuple[int, int], int] = {}
    route_links: dict[int, list[tuple[int, ...]]] = {}
    if sets is not None and sets.key_count:
        ko, kd = sets.keys()
        key_of = {(int(o), int(d)): k for k, (o, d) in enumerate(zip(ko, kd, strict=True))}
    costs = sets.costs() if sets is not None and sets.key_count else np.zeros(0)
    overlaps = sets.overlaps() if sets is not None and sets.key_count else np.zeros(0)
    set_off = sets.set_offsets().astype(np.int64) if sets is not None and sets.key_count else None
    route_off = sets.route_offsets().astype(np.int64) if set_off is not None else None
    links_all = sets.links().astype(np.int64) if set_off is not None else None

    def routes_of(k: int) -> list[tuple[int, ...]]:
        if k not in route_links:
            assert set_off is not None and route_off is not None and links_all is not None
            route_links[k] = [
                tuple(int(x) for x in links_all[route_off[r] : route_off[r + 1]])
                for r in range(int(set_off[k]), int(set_off[k + 1]))
            ]
        return route_links[k]

    # Every journey, as (pair, signature, weight, components, legs).
    by_pair: dict[tuple[int, int], dict[tuple, dict[str, Any]]] = defaultdict(dict)
    travellers: dict[tuple[int, int], float] = defaultdict(float)
    transit = getattr(run, "transit", None)
    lines = transit.lines() if transit is not None else None
    patterns = transit.line_patterns() if transit is not None else None
    stops = transit.stops() if transit is not None else None
    stop_xy = (
        geo.project_lonlat(np.column_stack([stops["lon"], stops["lat"]]), lon0, lat0)
        if stops is not None and len(stops["lon"])
        else None
    )
    places = run.parking_places()
    parking_xy, parking_index, parking_name = None, {}, {}
    if places is not None and len(places["parking_id"]):
        parking_xy = geo.project_lonlat(np.column_stack([places["lon"], places["lat"]]), lon0, lat0)
        parking_index = {str(p): i for i, p in enumerate(places["parking_id"])}
        try:
            names = places["name"]
        except KeyError:
            names = None
        parking_name = {
            str(p): str(names[i]) if names is not None and names[i] else str(p)
            for i, p in enumerate(places["parking_id"])
        }
    docks = run.bike_share_dock_places()
    dock_xy = (
        geo.project_lonlat(np.column_stack([docks["lon"], docks["lat"]]), lon0, lat0)
        if docks is not None and len(docks["lon"])
        else None
    )
    line_stops: dict[int, list[list[int]]] = defaultdict(list)
    if patterns is not None:
        for line, seq in zip(patterns["line"], patterns["stops"], strict=True):
            line_stops[int(line)].append([int(s) for s in seq])

    def ride_stops(line: int, board: int, alight: int) -> list[int]:
        for seq in line_stops.get(line, []):
            if board in seq:
                i = seq.index(board)
                if alight in seq[i + 1 :]:
                    return seq[i : seq.index(alight, i + 1) + 1]
        return [board, alight]

    def add(
        pair: tuple[int, int], sig: tuple, w: float, parts: dict[str, float], make: Any
    ) -> None:
        travellers[pair] += w
        o = by_pair[pair].get(sig)
        if o is None:
            o = by_pair[pair][sig] = {"w": 0.0, "n": 0, "sum": defaultdict(float), "make": make}
        o["w"] += w
        o["n"] += 1
        for k, v in parts.items():
            if np.isfinite(v):
                o["sum"][k] += v * w

    if journeys is not None:
        j = journeys
        choices = run.itinerary_choices()
        parking_ids = choices["parking_id"] if choices is not None else None
        lo, ro = j["link_offsets"].astype(np.int64), j["ride_offsets"].astype(np.int64)
        for i in range(len(j["trip"])):
            shape, mode = int(j["shape"][i]), j["mode"][i]
            o, d = int(j["road_origin"][i]), int(j["road_destination"][i])
            if shape == 255 or mode is None or _NONE in (o, d) or o == d:
                continue
            w = float(weight_of[int(j["trip"][i])])
            links = tuple(int(x) for x in j["links"][lo[i] : lo[i + 1]])
            rides = tuple(
                (int(j["ride_line"][k]), int(j["ride_board"][k]), int(j["ride_alight"][k]))
                for k in range(ro[i], ro[i + 1])
            )
            parking = parking_ids[i] if parking_ids is not None else None
            docks_used = (int(j["dock_pickup"][i]), int(j["dock_return"][i]))
            sig = (mode, shape, links, rides, parking, docks_used)
            parts = {
                "total": float(j["expected_s"][i]),
                "walk": float(j["walk_s"][i]),
                "wait": float(j["wait_s"][i]),
                "ride": float(j["ride_total_s"][i]),
                "vehicle": float(j["vehicle_s"][i]),
                "parking": float(j["parking_s"][i]),
                "access": float(j["access_walk_s"][i]),
                "egress": float(j["egress_walk_s"][i]),
            }
            for k in range(ro[i], ro[i + 1]):
                parts[f"ride{k - ro[i]}"] = float(j["ride_s"][k])
                parts[f"transfer{k - ro[i]}"] = float(j["ride_transfer_walk_s"][k])
            add((o, d), sig, w, parts, (mode, shape, links, rides, parking, docks_used))

    # Trips given the car (no mode choice): their route, from the route choices.
    if set_off is not None and route_sets is None:
        rc = run.route_choices()
        ko, kd = sets.keys()
        for t in np.nonzero(rc.route >= 0)[0]:
            k, r = int(rc.pair[t]), int(rc.route[t])
            pair = (int(ko[k]), int(kd[k]))
            links = routes_of(k)[r - int(set_off[k])]
            sig = ("car", 3, links, (), None, (_NONE, _NONE))
            parts = {
                "vehicle": float(costs[r]),
                "total": float(costs[r]),
            }  # the drive, at the route's cost
            add(pair, sig, float(rc.weight[t]), parts, ("car", 3, links, (), None, (_NONE, _NONE)))

    # Pairs: the most travellers first; without journeys (a route set given), the route sets' pairs.
    if not travellers and key_of:
        for o, d in key_of:
            travellers[(o, d)] = 0.0
            by_pair[(o, d)]  # noqa: B018  (the pair, with its routes only)
    if not travellers:
        return None
    node_ll = _node_lonlat(network)
    nodes = geo.project_lonlat(node_ll, lon0, lat0)
    order = sorted(travellers, key=lambda p: (-travellers[p], key_of.get(p, 1 << 62), p))
    order = [p for p in order if np.isfinite(nodes[p[0]]).all() and np.isfinite(nodes[p[1]]).all()]
    if not keep.all():
        # With a view, only pairs whose car routes and drives lie in it.
        order = [
            p
            for p in order
            if all(
                keep[list(sig[2])].all()
                for sig in by_pair[p]
                if sig[1] in (1, 2, 3, 6) and sig[0] in ("car", "taxi", "car_transit")
            )
        ]
    order = order[: max(max_pairs, 1)]

    kinds_of_line = list(lines["kind"]) if lines is not None else []
    names_of_line = (
        [str(n or lines["route_id"][i]) for i, n in enumerate(lines["name"])]
        if lines is not None
        else []
    )

    # The places a walk leaves or reaches, as (kind, index): a road node (a pair's origin or
    # destination), a stop, a parking, a dock. Their positions on the page and on the map.
    place_xy = {"node": nodes, "stop": stop_xy, "parking": parking_xy, "dock": dock_xy}
    place_ll = {
        "node": node_ll,
        "stop": np.column_stack([stops["lon"], stops["lat"]]) if stop_xy is not None else None,
        "parking": np.column_stack([places["lon"], places["lat"]])
        if parking_xy is not None
        else None,
        "dock": np.column_stack([docks["lon"], docks["lat"]]) if dock_xy is not None else None,
    }

    def xy_of(place: tuple[str, int] | None) -> np.ndarray | None:
        if place is None or place_xy[place[0]] is None:
            return None
        return place_xy[place[0]][place[1]]

    # The walk layer, for the walks to, from and between stops, docks and parkings.
    walk_layer = None
    if "walk" in layer_index:
        try:
            walk_layer = network.layer("walk")
        except (KeyError, ValueError):
            walk_layer = None

    # The arrays: pairs, options, legs.
    pair_xy, pair_n, pair_opt, pair_other = [], [], [0], []
    opt_mode, opt_n, opt_t, opt_cost, opt_overlap, opt_label, opt_leg = [], [], [], [], [], [], [0]
    leg_kind, leg_s, leg_a, leg_b, leg_line, leg_from, leg_to = [], [], [], [], [], [], []
    pool_links: list[int] = []
    pool_stops: list[int] = []
    pts: list[tuple[float, float]] = []
    # The walks between places, found on the walk layer once every leg is known: (leg, from, to).
    walks: list[tuple[int, tuple[str, int], tuple[str, int]]] = []

    def point(xy: np.ndarray | tuple[float, float]) -> int:
        pts.append((float(xy[0]), float(xy[1])))
        return len(pts) - 1

    def leg(kind: int, seconds: float, a: int = 0, b: int = 0, line: int = _NONE) -> None:
        leg_kind.append(kind)
        leg_s.append(seconds if np.isfinite(seconds) else 0.0)
        leg_a.append(a)
        leg_b.append(b)
        leg_line.append(line)
        leg_from.append(_NONE)
        leg_to.append(_NONE)

    def link_leg(kind: int, seconds: float, layer: str | None, links: tuple[int, ...]) -> bool:
        if layer is None:
            ids = [int(new_id[x]) for x in links if keep[x]]
            if len(ids) != len(links):
                return False
        else:
            index = layer_index.get(layer)
            if index is None:
                return False
            ids = [int(index[x]) for x in links]
            if any(i < 0 for i in ids):
                return False
        a = len(pool_links)
        pool_links.extend(ids)
        leg(kind, seconds, a, len(pool_links))
        return True

    def connect(seconds: float, a: Any, b: Any) -> None:
        if a is None or b is None:  # an end not on the page: a walk in the strip only
            leg(LEG_CONNECT, seconds, _NONE, _NONE)
            return
        leg(LEG_CONNECT, seconds, point(a), point(b))

    def walk(seconds: float, a: tuple[str, int] | None, b: tuple[str, int] | None) -> None:
        """A walk from place `a` to place `b`: along the walk layer once found, else straight."""
        if walk_layer is None or xy_of(a) is None or xy_of(b) is None:
            connect(seconds, xy_of(a), xy_of(b))
            return
        assert a is not None and b is not None
        leg(LEG_WALK, seconds)
        walks.append((len(leg_kind) - 1, a, b))

    def rides_legs(
        rides: tuple, mean: dict[str, float], start: tuple[str, int] | None
    ) -> tuple[str, int] | None:
        """The rides and the walks between them; returns the stop the last ride ends at."""
        at = start
        if mean.get("wait", 0) > 0:
            leg(LEG_WAIT, mean["wait"])
        for k, (line, board, alight) in enumerate(rides):
            if k > 0 and mean.get(f"transfer{k}", 0) > 0:
                walk(mean[f"transfer{k}"], at, ("stop", board))
            seq = ride_stops(line, board, alight)
            a = len(pool_stops)
            pool_stops.extend(seq)
            leg(LEG_RIDE, mean.get(f"ride{k}", 0.0), a, len(pool_stops), line)
            at = ("stop", alight)
        return at

    for p in order:
        o_xy, d_xy = nodes[p[0]], nodes[p[1]]
        total = travellers[p]
        options = by_pair[p]
        k = key_of.get(p)
        # The car routes of the pair's set nobody took, after the taken ones.
        taken_car = {sig[2] for sig in options if sig[0] == "car" and sig[1] == 3}
        if k is not None and set_off is not None:
            for r, links in enumerate(routes_of(k)):
                if links not in taken_car:
                    sig = ("car", 3, links, (), None, (_NONE, _NONE))
                    options[sig] = {
                        "w": 0.0,
                        "n": 0,
                        "sum": {},
                        "make": sig,
                        "cost": float(costs[int(set_off[k]) + r]),
                    }
        ranked = sorted(
            options.items(),
            key=lambda kv: (
                -kv[1]["w"],
                OPTION_MODES.index(kv[0][0]) if kv[0][0] in OPTION_MODES else 99,
                kv[0][2],
            ),
        )
        shown, other = [], 0.0
        for sig, o in ranked:
            unused_car = o["w"] == 0 and sig[0] == "car"
            if unused_car or (o["w"] >= _MIN_SHARE * total and len(shown) < _MAX_OPTIONS):
                shown.append((sig, o))
            else:
                other += o["w"]
        n_before = len(opt_mode)
        for _, o in shown:
            mode, shape, links, rides, parking, docks_used = o["make"]
            w = o["w"]
            mean = {key: v / w for key, v in o["sum"].items()} if w > 0 else {}
            start = len(leg_kind)
            ok = True
            park = (
                ("parking", parking_index[parking])
                if parking is not None and parking in parking_index
                else None
            )
            origin, destination = ("node", p[0]), ("node", p[1])
            first = ("stop", rides[0][1]) if rides else None
            vehicle_layer = None if mode in ("car", "taxi", "car_transit") else "bike"
            vehicle_kind = LEG_DRIVE if vehicle_layer is None else LEG_BIKE
            if shape == 0:  # walk, ride(s), walk
                walk(mean.get("access", 0.0), origin, first)
                end = rides_legs(rides, mean, first)
                walk(mean.get("egress", 0.0), end, destination)
            elif shape == 1:  # the vehicle to a parking, then transit
                ok = link_leg(vehicle_kind, mean.get("vehicle", 0.0), vehicle_layer, links)
                if mean.get("parking", 0) > 0:
                    leg(LEG_PARK, mean["parking"])
                walk(mean.get("access", 0.0), park, first)
                end = rides_legs(rides, mean, first)
                walk(mean.get("egress", 0.0), end, destination)
            elif shape == 2:  # transit to the parked vehicle, then home
                walk(mean.get("access", 0.0), origin, first)
                end = rides_legs(rides, mean, first)
                walk(mean.get("egress", 0.0), end, park)
                if mean.get("parking", 0) > 0:
                    leg(LEG_PARK, mean["parking"])
                ok = link_leg(vehicle_kind, mean.get("vehicle", 0.0), vehicle_layer, links)
            elif shape == 3:  # one leg
                if mode == "walk":
                    ok = link_leg(LEG_WALK, mean.get("walk", 0.0), "walk", links)
                else:
                    if mean.get("walk", 0) > 0:
                        leg(LEG_PARK, mean["walk"])
                    secs = mean.get("vehicle", o.get("cost", 0.0))
                    ok = link_leg(vehicle_kind, secs, vehicle_layer, links)
                    if mean.get("parking", 0) > 0:
                        leg(LEG_PARK, mean["parking"])
            elif shape in (4, 5):  # a shared bike (then transit)
                pick = ("dock", docks_used[0]) if docks_used[0] != _NONE else None
                drop = ("dock", docks_used[1]) if docks_used[1] != _NONE else None
                walk_left = max(
                    mean.get("walk", 0.0) - mean.get("access", 0.0) - mean.get("egress", 0.0), 0.0
                )
                walk(walk_left / 2 if shape == 4 else walk_left, origin, pick)
                ok = link_leg(LEG_BIKE, mean.get("vehicle", 0.0), "bike", links)
                if shape == 4:
                    walk(walk_left / 2, drop, destination)
                else:
                    walk(mean.get("access", 0.0), drop, first)
                    end = rides_legs(rides, mean, first)
                    walk(mean.get("egress", 0.0), end, destination)
            elif shape == 6:  # a taxi
                if mean.get("wait", 0) > 0:
                    leg(LEG_WAIT, mean["wait"])
                ok = link_leg(LEG_DRIVE, mean.get("vehicle", 0.0), None, links)
            if not ok:  # its geometry is not on the page: leave the option out
                for column in (leg_kind, leg_s, leg_a, leg_b, leg_line, leg_from, leg_to):
                    del column[start:]
                while walks and walks[-1][0] >= start:
                    walks.pop()
                other += w
                continue
            opt_leg.append(len(leg_kind))
            opt_mode.append(OPTION_MODES.index(mode) if mode in OPTION_MODES else 0)
            opt_n.append(w)
            opt_t.append(mean.get("total", np.nan) if w > 0 else o.get("cost", np.nan))
            r_index = (
                routes_of(k).index(links)
                if k is not None
                and set_off is not None
                and mode == "car"
                and shape == 3
                and links in routes_of(k)
                else -1
            )
            opt_cost.append(float(costs[int(set_off[k]) + r_index]) if r_index >= 0 else np.nan)
            opt_overlap.append(
                float(overlaps[int(set_off[k]) + r_index]) if r_index >= 0 else np.nan
            )
            names = " → ".join(names_of_line[r[0]] for r in rides) if rides else ""
            opt_label.append(
                _label(mode, shape, names, parking_name.get(parking, parking) if parking else None)
            )
        if len(opt_mode) == n_before:
            continue
        pair_xy.append((o_xy, d_xy))
        pair_n.append(total)
        pair_other.append(other)
        pair_opt.append(len(opt_mode))
    if not pair_n:
        return None

    # The walks between places, on the walk layer: each place at its nearest walk node, as the run
    # snaps it, and the path the run's own search finds between them (one search per place left).
    if walks:
        assert walk_layer is not None
        index = layer_index["walk"]
        snapped: dict[tuple[str, int], int] = {}

        def node_of(place: tuple[str, int]) -> int:
            if place not in snapped:
                lon, lat = place_ll[place[0]][place[1]]
                snapped[place] = int(walk_layer.node_nearest(float(lon), float(lat)))
            return snapped[place]

        walk_ends = [(node_of(a), node_of(b)) for _, a, b in walks]
        bound: dict[tuple[int, int], float] = {}
        for (g, _, _), pair in zip(walks, walk_ends, strict=True):
            bound[pair] = max(bound.get(pair, 0.0), 2 * leg_s[g] + _WALK_SEARCH_SLACK_S)
        pairs = sorted(bound)
        links, offsets, seconds = walk_layer.path_shortest(
            [a for a, _ in pairs],
            [b for _, b in pairs],
            bound_s=[min(_WALK_SEARCH_MAX_S, bound[pair]) for pair in pairs],
        )
        offsets = offsets.astype(np.int64)
        found = {pair: i for i, pair in enumerate(pairs)}
        for (g, a, b), pair in zip(walks, walk_ends, strict=True):
            i = found[pair]
            ids = index[links[offsets[i] : offsets[i + 1]].astype(np.int64)]
            a_xy, b_xy = xy_of(a), xy_of(b)
            assert a_xy is not None and b_xy is not None
            if len(ids) and np.isfinite(seconds[i]) and (ids >= 0).all():
                leg_a[g] = len(pool_links)
                pool_links.extend(int(x) for x in ids)
                leg_b[g] = len(pool_links)
                leg_from[g], leg_to[g] = point(a_xy), point(b_xy)
            else:  # no path on the walk layer (or one leaving the page): a straight line
                leg_kind[g] = LEG_CONNECT
                leg_a[g], leg_b[g] = point(a_xy), point(b_xy)

    q = lambda a: np.round(np.asarray(a, dtype=float) / 0.1).astype(np.int64)  # noqa: E731
    ends = np.array([[a[0], a[1], b[0], b[1]] for a, b in pair_xy])
    blob.add("qox", q(ends[:, 0]), "i32")
    blob.add("qoy", q(ends[:, 1]), "i32")
    blob.add("qdx", q(ends[:, 2]), "i32")
    blob.add("qdy", q(ends[:, 3]), "i32")
    blob.add("qn", np.asarray(pair_n), "f32")
    blob.add("qr", np.asarray(pair_other), "f32")
    blob.add("qs", np.asarray(pair_opt), "u32")
    blob.add("om", np.asarray(opt_mode), "u8")
    blob.add("on", np.asarray(opt_n), "f32")
    blob.add("ot", np.asarray(opt_t), "f32")
    blob.add("oc", np.asarray(opt_cost), "f32")
    blob.add("oo", np.asarray(opt_overlap), "f32")
    blob.add("ol", np.asarray(opt_leg), "u32")
    blob.add("lk", np.asarray(leg_kind), "u8")
    blob.add("lsec", np.asarray(leg_s), "f32")
    blob.add("la", np.asarray(leg_a), "u32")
    blob.add("lb", np.asarray(leg_b), "u32")
    blob.add("lline", np.asarray(leg_line), "u32")
    blob.add("lpa", np.asarray(leg_from), "u32")
    blob.add("lpb", np.asarray(leg_to), "u32")
    blob.add("pl", np.asarray(pool_links or [0]), "u32")
    blob.add("pst", np.asarray(pool_stops or [0]), "u32")
    pq = np.asarray(pts or [(0.0, 0.0)])
    blob.add("ptx", q(pq[:, 0]), "i32")
    blob.add("pty", q(pq[:, 1]), "i32")
    if stop_xy is not None and not blob.has("tsx"):
        blob.add("tsx", q(stop_xy[:, 0]), "i32")
        blob.add("tsy", q(stop_xy[:, 1]), "i32")
    origins = _labels(ends[:, :2], stops, stop_xy)
    destinations = _labels(ends[:, 2:], stops, stop_xy)
    return {
        "modes": list(OPTION_MODES),
        "labels": opt_label,
        "places": [[a, b] for a, b in zip(origins, destinations, strict=True)],
        "lines": names_of_line,
        "line_kinds": kinds_of_line,
        "stops": [
            str((stops.get("name") or stops["stop_id"])[i] or stops["stop_id"][i])
            for i in range(len(stops["stop_id"]))
        ]
        if stops is not None
        else [],
        "n_pairs": len(pair_n),
        "has_journeys": journeys is not None,
    }


def _label(mode: str, shape: int, lines: str, parking: str | None) -> str:
    """An option's name in the panel."""
    if mode == "car":
        return "Car"
    if mode == "taxi":
        return "Taxi"
    if mode == "walk":
        return "Walk"
    if mode == "bike":
        return "Bike"
    if mode == "bike_share":
        return "Shared bike"
    if mode == "transit":
        return lines or "Transit"
    if mode == "bike_share_transit":
        return "Shared bike → " + lines
    vehicle = "Car" if mode == "car_transit" else "Bike"
    if shape == 2:
        return f"{lines} → {vehicle.lower()} from {parking or 'the parking'}"
    return f"{vehicle} to {parking or 'a parking'} → {lines}"
