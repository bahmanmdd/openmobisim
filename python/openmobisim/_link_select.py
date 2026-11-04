"""The select-link analysis: the origin-destination pairs whose trips cross chosen links.

A trip **uses** a link when the route it travelled crosses it: on the road layer the route of
its car (a car trip's route choice, the car leg of a park-and-ride trip, a taxi ride with its
passenger), the links it actually took where it re-routed on the way and arrived, and for a trip
still under way when the window ended its route up to the link it was on; on the bike and walk
layers every leg the run put there (a bike ride, a shared-bike ride, a walk trip, a walk to,
from or between transit stops), the legs those layers' per-link results count. Each layer's
users are held as **paths** (each a sequence of the layer's links) and **users** (a trip and
one of its paths): a car route chosen by many trips is one path.
"""

from __future__ import annotations

from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from typing import Any

import numpy as np

from openmobisim.table import Columns

#: The layers a select-link analysis reads.
LAYERS = ("road", "bike", "walk")
#: The modes whose own vehicle drives on the road layer.
_ROAD_MODES = ("car", "taxi", "car_transit")
_NONE = 2**32 - 1


@dataclass(frozen=True)
class Users:
    """One layer's users: the paths, and each user's trip row and path."""

    #: ``paths + 1`` offsets into `path_links`.
    path_offsets: np.ndarray
    #: Every path's links, one flat array of the layer's link indices.
    path_links: np.ndarray
    #: Each user's row in ``trip_results()``.
    user_row: np.ndarray
    #: Each user's path.
    user_path: np.ndarray


def _check_layer(layer: str) -> str:
    if layer not in LAYERS:
        raise ValueError(f"unknown layer {layer!r}; choose from {list(LAYERS)}")
    return layer


def _csr(paths: list[np.ndarray]) -> tuple[np.ndarray, np.ndarray]:
    lengths = np.array([len(p) for p in paths], dtype=np.int64)
    offsets = np.concatenate([[0], np.cumsum(lengths)]).astype(np.int64)
    links = (
        np.concatenate([np.asarray(p, dtype=np.int64) for p in paths])
        if paths and offsets[-1]
        else np.zeros(0, dtype=np.int64)
    )
    return offsets, links


def layer_users(run: Any, layer: str) -> Users:
    """The users of `layer`'s links in `run`: see the module's docstring."""
    _check_layer(layer)
    tr = run.trip_results()
    trip = np.asarray(tr["trip"], dtype=np.int64)
    row_of = np.full(int(trip.max()) + 1 if len(trip) else 0, -1, dtype=np.int64)
    row_of[trip] = np.arange(len(trip))
    if layer != "road":
        if run.link_bins() is None:
            raise ValueError(
                f"the {layer} layer's users are kept with the per-link results: build the "
                "scenario with link_bin_s"
            )
        legs = getattr(run._summary, "layer_legs", None) or {}
        cols = legs.get(layer)
        if cols is None:
            raise ValueError(f"this run does not keep its {layer} legs (a run read from a folder)")
        offsets = np.asarray(cols["link_offsets"], dtype=np.int64)
        return Users(
            path_offsets=offsets,
            path_links=np.asarray(cols["links"], dtype=np.int64),
            user_row=row_of[np.asarray(cols["trip"], dtype=np.int64)],
            user_path=np.arange(len(offsets) - 1, dtype=np.int64),
        )

    # The road layer: the route sets' routes first (a stated car trip's route choice), then a
    # path of its own for each trip that drove another way.
    sets = run.route_sets()
    if sets is not None and sets.key_count:
        base_offsets = sets.route_offsets().astype(np.int64)
        base_links = sets.links().astype(np.int64)
    else:
        base_offsets, base_links = np.zeros(1, dtype=np.int64), np.zeros(0, dtype=np.int64)
    n_base = len(base_offsets) - 1
    path_of_row = np.full(len(trip), -1, dtype=np.int64)
    rc = run.route_choices()
    if rc is not None and len(rc):
        route = np.asarray(rc.route, dtype=np.int64)
        routed = np.nonzero(route >= 0)[0]
        path_of_row[row_of[routed]] = route[routed]
    own: dict[int, np.ndarray] = {}  # trip row -> its links, where not a route set's route
    journeys = run.itinerary_journeys()
    if journeys is None and run.itinerary_choices() is not None:
        raise ValueError(
            "this run's journeys are not kept (a run read from a folder): select links on the "
            "run Scenario.run() returned"
        )
    if journeys is not None:
        lo = np.asarray(journeys["link_offsets"], dtype=np.int64)
        links = np.asarray(journeys["links"], dtype=np.int64)
        shape = np.asarray(journeys["shape"])
        for i, mode in enumerate(journeys["mode"]):
            if mode in _ROAD_MODES and shape[i] in (1, 2, 3, 6) and lo[i + 1] > lo[i]:
                own[int(row_of[int(journeys["trip"][i])])] = links[lo[i] : lo[i + 1]]
    realised = run.route_realised()
    for t, links in zip(realised["trip"], realised["links"], strict=True):
        own[int(row_of[int(t)])] = np.asarray(links, dtype=np.int64)
    # A trip still under way at the window's end: its route up to the link it was on.
    at_end = np.asarray(tr["link_at_end"], dtype=float)
    for r in np.nonzero(np.isfinite(at_end))[0]:
        r = int(r)
        links = own.get(r)
        if links is None:
            p = int(path_of_row[r])
            if p < 0:
                continue
            links = base_links[base_offsets[p] : base_offsets[p + 1]]
        hits = np.nonzero(links == int(at_end[r]))[0]
        if len(hits):
            own[r] = links[: int(hits[0]) + 1]
    rows = sorted(own)
    extra_offsets, extra_links = _csr([own[r] for r in rows])
    for k, r in enumerate(rows):
        path_of_row[r] = n_base + k
    offsets = np.concatenate([base_offsets, base_offsets[-1] + extra_offsets[1:]])
    users = np.nonzero(path_of_row >= 0)[0]
    return Users(
        path_offsets=offsets,
        path_links=np.concatenate([base_links, extra_links]),
        user_row=users,
        user_path=path_of_row[users],
    )


def paths_crossing(users: Users, links: np.ndarray) -> np.ndarray:
    """Whether each path crosses at least one of `links`."""
    hit = np.isin(users.path_links, links)
    count = np.concatenate([[0], np.cumsum(hit, dtype=np.int64)])
    return (count[users.path_offsets[1:]] - count[users.path_offsets[:-1]]) > 0


def places(
    run: Any, zones: Mapping[str, tuple[float, float]] | None
) -> tuple[np.ndarray, np.ndarray, list[Any], np.ndarray]:
    """Each trip's origin and destination place: codes into a list of places, and their points.

    With `zones`, a place is the zone whose point is nearest the trip's end; without, the road
    node the trip is routed from or to (``Network.node_nearest``). Returns the origin and
    destination codes by trip row, the places (zone names or node indices) and their
    ``(lon, lat)``.
    """
    tr = run.trip_results()
    lon = np.concatenate([np.asarray(tr["origin_lon"]), np.asarray(tr["destination_lon"])])
    lat = np.concatenate([np.asarray(tr["origin_lat"]), np.asarray(tr["destination_lat"])])
    n = len(tr["trip"])
    points, inverse = np.unique(np.column_stack([lon, lat]), axis=0, return_inverse=True)
    inverse = inverse.reshape(-1)
    if zones is not None:
        names = list(zones)
        if not names:
            raise ValueError("zones is empty")
        z = np.array([zones[k] for k in names], dtype=float).reshape(-1, 2)
        scale = np.cos(np.radians(float(np.mean(z[:, 1]))))
        nearest = np.empty(len(points), dtype=np.int64)
        for a in range(0, len(points), 4096):  # chunks keep the distance matrix small
            dx = (points[a : a + 4096, None, 0] - z[None, :, 0]) * scale
            dy = points[a : a + 4096, None, 1] - z[None, :, 1]
            nearest[a : a + 4096] = np.argmin(dx * dx + dy * dy, axis=1)
        place_of_point = nearest
        place_names: list[Any] = names
        place_ll = z
    else:
        network = run.network
        if network is None:
            raise ValueError("give zones: this run does not carry its network")
        within = run._car_snap_motorway_m
        nodes = np.array(
            [network.node_nearest(float(x), float(y), within) for x, y in points], dtype=np.int64
        )
        place_names_arr, place_of_point = np.unique(nodes, return_inverse=True)
        place_of_point = place_of_point.reshape(-1)
        place_names = [int(v) for v in place_names_arr]
        from openmobisim.viz._trips import _node_lonlat

        place_ll = _node_lonlat(network)[place_names_arr]
    code = place_of_point[inverse]
    return code[:n], code[n:], place_names, np.asarray(place_ll, dtype=float)


def link_select(
    run: Any,
    links: int | Sequence[int] | np.ndarray,
    layer: str = "road",
    *,
    zones: Mapping[str, tuple[float, float]] | None = None,
    departure_s: tuple[float, float] | None = None,
    bin_s: int | None = None,
) -> Columns:
    """The table of :meth:`Run.link_select`; see there."""
    _check_layer(layer)
    selected = np.unique(np.atleast_1d(np.asarray(links, dtype=np.int64)))
    network = run.network
    if network is not None:
        count = int((network if layer == "road" else network.layer(layer)).link_count)
        bad = selected[(selected < 0) | (selected >= count)]
        if len(bad):
            raise ValueError(
                f"link {int(bad[0])} is not a link of the {layer} layer (0..{count - 1})"
            )
    if bin_s is not None and bin_s <= 0:
        raise ValueError("bin_s must be a positive number of seconds")
    users = layer_users(run, layer)
    tr = run.trip_results()
    rows = np.unique(users.user_row[paths_crossing(users, selected)[users.user_path]])
    departure = np.asarray(tr["departure_s"], dtype=float)
    weight = np.asarray(tr["weight"], dtype=float)
    in_window = np.ones(len(departure), dtype=bool)
    if departure_s is not None:
        start, end = departure_s
        in_window = (departure >= start) & (departure < end)
    rows = rows[in_window[rows]]
    origin, destination, names, _ = places(run, zones)
    modes = np.array([m if m is not None else "" for m in tr["mode"]], dtype=object)
    bins = (departure // bin_s).astype(np.int64) if bin_s else np.zeros(len(departure), np.int64)
    # The pairs' travellers in the window (by bin): the denominators of share_pair.
    everyone = np.nonzero(in_window)[0]
    pair_people: dict[tuple[int, int, int], float] = {}
    for o, d, b, w in zip(
        origin[everyone], destination[everyone], bins[everyone], weight[everyone], strict=True
    ):
        key = (int(o), int(d), int(b))
        pair_people[key] = pair_people.get(key, 0.0) + float(w)
    groups: dict[tuple[int, int, str, int], list[float]] = {}
    for r in rows:
        key = (int(origin[r]), int(destination[r]), str(modes[r]), int(bins[r]))
        g = groups.setdefault(key, [0.0, 0.0])
        g[0] += 1
        g[1] += float(weight[r])
    # The link's users (by bin): the denominators of share_link.
    total: dict[int, float] = {}
    for k, g in groups.items():
        total[k[3]] = total.get(k[3], 0.0) + g[1]
    order = sorted(groups, key=lambda k: (-groups[k][1], k[3], k[0], k[1], k[2]))
    out = Columns()
    if bin_s:
        out["departure_s"] = np.array([k[3] * bin_s for k in order], dtype=np.int64)
    out["origin"] = [names[k[0]] for k in order]
    out["destination"] = [names[k[1]] for k in order]
    out["mode"] = [k[2] for k in order]
    out["trips"] = np.array([int(groups[k][0]) for k in order], dtype=np.int64)
    out["people"] = np.array([groups[k][1] for k in order], dtype=float)
    out["share_link"] = np.array([groups[k][1] / total[k[3]] for k in order], dtype=float)
    out["share_pair"] = np.array(
        [groups[k][1] / pair_people[(k[0], k[1], k[3])] for k in order], dtype=float
    )
    return out
