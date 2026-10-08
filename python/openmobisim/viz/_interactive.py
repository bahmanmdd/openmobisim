"""``map_interactive``: the run, and its route sets, as one self-contained HTML file (S167).

The page needs no server, no network and no library. Everything is embedded:
the network's street shapes (coordinates in tenths of a metre, delta-encoded),
the per-link, per-time-bin results, and the route sets, as typed arrays,
compressed once and unpacked by the browser. The script draws with a canvas,
works out what to show from the zoom (major roads first, then the rest), and
lets the reader move through time, colour links three ways, hover to read a
link, and click to see which routes use it or to pick an origin-destination pair
and compare its alternatives.
"""

from __future__ import annotations

import base64
import json
import zlib
from pathlib import Path
from typing import Any

import numpy as np

from openmobisim import __version__
from openmobisim.network import LINK_CLASSES
from openmobisim.viz import _geometry as geo
from openmobisim.viz._figure import CREATOR
from openmobisim.viz._interactive_page import PAGE_CSS, PAGE_HTML, PAGE_JS
from openmobisim.viz._logo import LOGO_PATH
from openmobisim.viz._provenance import network_source, run_identity
from openmobisim.viz._route import _ROUTE_NIGHT, _ROUTE_PAPER
from openmobisim.viz._style import THEMES, get_theme

__all__ = ["map_interactive"]

#: The road classes' names as the page shows them, in the order of their numbers (X-23).
_CLASS_NAMES = [name.replace("_", " ") for name in LINK_CLASSES]
_LEVEL_NAMES = {0: "free flow", 2: "point queue", 3: "spatial queue", 4: "full"}
#: Coordinates are stored in tenths of a metre as 32-bit integers: good to ~214 km.
_UNIT_M = 0.1
_LIMIT = 2**31 - 1


class _Blob:
    """Typed arrays laid end to end, each aligned to 8 bytes, with a table of contents."""

    def __init__(self) -> None:
        self.parts: list[bytes] = []
        self.table: list[dict[str, Any]] = []
        self.size = 0

    def add(self, name: str, array: np.ndarray, kind: str) -> None:
        dtype = {"u8": "<u1", "u32": "<u4", "i32": "<i4", "f32": "<f4"}[kind]
        data = np.ascontiguousarray(array, dtype=dtype).tobytes()
        self.table.append({"n": name, "t": kind, "o": self.size, "c": int(array.size)})
        pad = (-len(data)) % 8
        self.parts.append(data + b"\0" * pad)
        self.size += len(data) + pad

    def packed(self) -> str:
        return base64.b64encode(zlib.compress(b"".join(self.parts), 6)).decode("ascii")


def _tokens() -> dict[str, Any]:
    themes = {
        name: {
            "surface": t.surface, "ink": t.ink, "ink2": t.ink2, "muted": t.muted,
            "base": t.base, "mark": t.mark, "glow": t.glow,
            "kinds": list(t.kinds), "modes": list(t.modes), "ramp_delay": list(t.ramp_delay),
            "ramp_volume": list(t.ramp_volume),
        }
        for name, t in THEMES.items()
    }  # fmt: skip
    return {**themes, "route": {"paper": list(_ROUTE_PAPER), "night": list(_ROUTE_NIGHT)}}


def _quantise(points: np.ndarray) -> np.ndarray:
    """Points in metres as the page stores them: tenths of a metre, 32-bit."""
    q = np.round(points / _UNIT_M).astype(np.int64)
    if len(q) and np.abs(q).max() > _LIMIT:
        raise ValueError("the region is too large to store (over about 200 km); pass view=")
    return q


def _inside(points: np.ndarray, box: tuple[np.ndarray, np.ndarray] | None) -> np.ndarray:
    if box is None:
        return np.ones(len(points), dtype=bool)
    return ((points >= box[0]) & (points <= box[1])).all(axis=1)


def _add_layers(
    blob: _Blob,
    run: Any,
    network: Any,
    lon0: float,
    lat0: float,
    box: tuple[np.ndarray, np.ndarray] | None,
) -> dict[str, Any]:
    """Add the optional layers the run has data for; return what the page needs to know of them.

    **Transit** (stop-to-stop hops, as wide as the passengers on board over the day, coloured by
    the kind of service; stops sized by boardings), **parkings** (each a mark sized by capacity,
    coloured by the busiest share of it in use), and the **cyclists'** and **walkers'** flows on
    their own networks (only the links someone used: whole-run totals). Each is small beside the
    road layer; the page shows none of them until asked (T-4: the main things on, the rest one
    click away).
    """
    layers: dict[str, Any] = {}
    calls = run.transit_calls() if hasattr(run, "transit_calls") else None
    transit = getattr(run, "transit", None)
    if calls is not None and transit is not None and len(calls["run"]):
        stops = transit.stops()
        index = {sid: i for i, sid in enumerate(stops["stop_id"])}
        xy = geo.project_lonlat(np.column_stack([stops["lon"], stops["lat"]]), lon0, lat0)
        kinds = ("rail", "metro", "tram", "bus", "ferry", "other")
        runs, kind, stop_ids = calls["run"], calls["kind"], calls["stop_id"]
        on_board = np.asarray(calls["on_board"], dtype=float)
        loads: dict[tuple[int, int, int], float] = {}
        for i in range(len(runs) - 1):
            a, b = index[stop_ids[i]], index[stop_ids[i + 1]]
            if runs[i] == runs[i + 1] and a != b:
                key = (kinds.index(kind[i]) if kind[i] in kinds else 5, a, b)
                loads[key] = loads.get(key, 0.0) + float(on_board[i])
        keep = _inside(xy, box)
        keys = [k for k in sorted(loads, key=lambda k: (loads[k], k)) if keep[k[1]] and keep[k[2]]]
        if keys:
            boardings = np.zeros(len(index))
            np.add.at(boardings, [index[sid] for sid in stop_ids], np.asarray(calls["boardings"]))
            q = _quantise(xy)
            blob.add("tsx", q[:, 0], "i32")
            blob.add("tsy", q[:, 1], "i32")
            blob.add("tsb", boardings, "f32")
            blob.add("ta", np.array([k[1] for k in keys]), "u32")
            blob.add("tb", np.array([k[2] for k in keys]), "u32")
            blob.add("tk", np.array([k[0] for k in keys]), "u8")
            blob.add("tl", np.array([loads[k] for k in keys]), "f32")
            names = stops.get("name") or stops["stop_id"]
            layers["transit"] = {
                "kinds": list(kinds),
                "stops": [str(n or stops["stop_id"][i]) for i, n in enumerate(names)],
                "shown": keep.tolist() if box is not None else None,
            }
    places = run.parking_places() if hasattr(run, "parking_places") else None
    bins = run.parking_bins() if hasattr(run, "parking_bins") else None
    if places is not None and len(places["parking_id"]):
        xy = geo.project_lonlat(np.column_stack([places["lon"], places["lat"]]), lon0, lat0)
        keep = _inside(xy, box)
        if keep.any():
            capacity = np.asarray(places["capacity"], dtype=float)
            peak = np.zeros(len(capacity))
            if bins is not None and len(bins["parking"]):
                np.maximum.at(
                    peak,
                    np.asarray(bins["parking"], dtype=np.int64),
                    np.asarray(bins["occupancy_max"]),
                )
            q = _quantise(xy[keep])
            blob.add("px", q[:, 0], "i32")
            blob.add("py", q[:, 1], "i32")
            blob.add("pc", capacity[keep], "f32")
            blob.add("po", (peak / np.maximum(capacity, 1.0))[keep], "f32")
            blob.add("pv", (np.asarray(places["vehicle"]) == "bike")[keep], "u8")
            ids = np.asarray(places["parking_id"])[keep]
            layers["parking"] = {"ids": [str(i) for i in ids]}
    for name in ("bike", "walk"):
        lb = run.link_bins(name) if hasattr(run, "link_bins") else None
        if lb is None or not len(lb):
            continue
        layer = network.layer(name)
        total = np.zeros(int(layer.link_count))
        np.add.at(total, lb.links().astype(np.int64), lb.pcu())
        coords, offsets = layer.link_geometry()
        pts = geo.project_lonlat(coords, lon0, lat0)
        used = np.flatnonzero(total > 0)
        if box is not None:
            lo_box, hi_box = _link_boxes(pts, offsets)
            used = used[((hi_box[used] >= box[0]) & (lo_box[used] <= box[1])).all(axis=1)]
        if not len(used):
            continue
        parts = [pts[int(offsets[i]) : int(offsets[i + 1])] for i in used]
        counts = np.array([len(x) for x in parts])
        q = _quantise(np.concatenate(parts))
        delta = np.diff(q, axis=0, prepend=np.zeros((1, 2), dtype=np.int64))
        prefix = name[0]
        blob.add(prefix + "vs", np.concatenate([[0], np.cumsum(counts)]), "u32")
        blob.add(prefix + "dx", delta[:, 0], "i32")
        blob.add(prefix + "dy", delta[:, 1], "i32")
        blob.add(prefix + "v", total[used], "f32")
        layers[name] = {"links": int(len(used))}
    return layers


def _link_boxes(points: np.ndarray, offsets: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
    start = offsets[:-1].astype(np.int64)
    return (
        np.minimum.reduceat(points, start, axis=0),
        np.maximum.reduceat(points, start, axis=0),
    )


def map_interactive(
    run: Any,
    path: str | Path,
    *,
    routes: Any = True,
    theme: str = "paper",
    title: str | None = None,
    note: str | None = None,
    credit: str | None = None,
    logo: bool = True,
    view: tuple[tuple[float, float], tuple[float, float]] | None = None,
    max_route_pairs: int = 2000,
) -> Path:
    """Write a run and its route sets as one interactive HTML file.

    Open the file in any modern browser: it needs no server, no network and no
    library. **Zoom** with the wheel, the buttons or the five level buttons
    (Overview, Region, District, Street, Detail): a large network shows its
    major roads first and adds minor streets as you zoom in, so it stays fast
    and readable at every scale. **Time**: a slider over the run's time bins,
    all bins together, or play. **Colour** links by delay, volume, or volume
    over capacity. **Hover** a link to read it; **click** it to see how many
    routes and pairs use it and jump to one of them. **Pick a pair** to see
    its alternatives side by side, with each route's time, detour and overlap.
    **Layers**: besides road traffic, the run's transit lines and stops, its
    parkings, and the cyclists' and walkers' flows on their own networks, each
    with a switch and each off at first (the road traffic is the main view;
    transit over busy roads reads only when asked for). Only the layers the run
    has data for are offered.
    The page's whole state is in its URL after the ``#``, so a view can be
    shared and reproduced. The figure is drawn at the screen's own resolution:
    it stays sharp at any zoom.

    **Size.** A file holds one copy of the network's street shapes, the
    per-bin results and the route sets, compressed. Andorra (23 000 links) with
    3 000 route pairs is a few megabytes; a whole country is tens. To keep a
    file small, pass ``view`` to include only a region, or lower
    ``max_route_pairs``.

    Args:
        run: A ``Run`` from ``Scenario.run()``. Built with ``link_bin_s`` it
            shows traffic; without, the network and routes only.
        path: Where to write the ``.html`` file.
        routes: ``True`` (default) to include the run's route sets, ``False`` or
            ``None`` for none, or a ``RouteSets`` object (from
            ``openmobisim.route_sets_build``) to show instead.
        theme: The starting theme, ``"paper"`` or ``"night"``; the page has a
            button to switch.
        title: The page title; defaults to a description of what is drawn.
        note: A third, italic line under the title.
        credit: A line of your own (a name, an institution) shown before the logo.
        logo: Show the openmobisim logo at the bottom right.
        view: ``((lon_min, lat_min), (lon_max, lat_max))``: include only the
            links that touch this box, and only routes that lie wholly inside it.
        max_route_pairs: At most this many origin-destination pairs are
            embedded, those with the most alternatives first (ties by order).

    Returns:
        The path written.

    Raises:
        ValueError: If `theme` is unknown, `view` is empty, or the region is too
            large to store (over about 200 km across; pass `view`).
    """
    get_theme(theme)
    network = run.network
    if network is None:
        raise ValueError("this run does not carry its network")
    coords, offsets = network.link_geometry()
    n_all = int(network.link_count)
    lon0, lat0 = float(coords[:, 0].mean()), float(coords[:, 1].mean())
    points = geo.project_lonlat(coords, lon0, lat0)

    keep = np.ones(n_all, dtype=bool)
    if view is not None:
        lo = geo.project_lonlat(np.array([view[0]]), lon0, lat0)[0]
        hi = geo.project_lonlat(np.array([view[1]]), lon0, lat0)[0]
        bmin, bmax = _link_boxes(points, offsets)
        keep = ((bmax >= lo) & (bmin <= hi)).all(axis=1)
        if not keep.any():
            raise ValueError("view holds no links")
    new_id = np.cumsum(keep) - 1
    n_links = int(keep.sum())
    vertex_keep = np.repeat(keep, np.diff(offsets).astype(np.int64))
    pts = points[vertex_keep]
    counts = np.diff(offsets).astype(np.int64)[keep]
    vs = np.concatenate([[0], np.cumsum(counts)])
    quantised = np.round(pts / _UNIT_M).astype(np.int64)
    if np.abs(quantised).max() > _LIMIT:
        raise ValueError("the region is too large to store (over about 200 km); pass view=")
    delta = np.diff(quantised, axis=0, prepend=np.zeros((1, 2), dtype=np.int64))

    blob = _Blob()
    blob.add("vs", vs, "u32")
    blob.add("dx", delta[:, 0], "i32")
    blob.add("dy", delta[:, 1], "i32")
    blob.add("cls", network.link_class()[keep], "u8")
    blob.add("len", network.link_length_m()[keep], "f32")
    blob.add("ff", network.link_free_flow_s()[keep], "f32")
    blob.add("cap", network.link_capacity_pcu_h()[keep], "f32")

    bins = run.link_bins()
    n_bins = bin_seconds = 0
    if bins is not None and len(bins):
        b, link = bins.bins().astype(np.int64), bins.links().astype(np.int64)
        rows = keep[link]
        b, link = b[rows], new_id[link[rows]]
        n_bins = int(b.max()) + 1 if len(b) else 0
        bin_seconds = int(bins.bin_seconds)
        blob.add("bs", np.searchsorted(b, np.arange(n_bins + 1)), "u32")
        blob.add("rl", link, "u32")
        blob.add("rp", bins.pcu()[rows], "f32")
        blob.add("rs", bins.pcu_seconds()[rows], "f32")

    # The links where a queue outgrew the road (S229): one optional layer.
    n_spill = 0
    spill = run.report_spillback() if hasattr(run, "report_spillback") else None
    if spill is not None and len(spill["link"]):
        flag = np.zeros(n_all, dtype=np.uint8)
        flag[np.asarray(spill["link"], dtype=np.int64)] = 1
        n_spill = int(flag[keep].sum())
        if n_spill:
            blob.add("sb", flag[keep], "u8")

    # The other layers (T-4): each off at first, each a toggle; only what the run has.
    box = None
    if view is not None:
        box = (
            geo.project_lonlat(np.array([view[0]]), lon0, lat0)[0],
            geo.project_lonlat(np.array([view[1]]), lon0, lat0)[0],
        )
    layers = _add_layers(blob, run, network, lon0, lat0, box)

    route_sets = run.route_sets() if routes is True else (routes or None)
    n_pairs = n_routes = 0
    if route_sets is not None and route_sets.key_count:
        usage = _route_usage(run) if routes is True else None
        n_pairs, n_routes = _add_routes(blob, route_sets, keep, new_id, max_route_pairs, usage)
    has_routes = n_pairs > 0

    level = _LEVEL_NAMES.get(run.flow_level, str(run.flow_level))
    source = network_source(network)
    provenance = (
        f"{__version__} · run {run.run_id} · level {run.flow_level} ({level})"
        + (f" · step {run.flow_step_s} s" if run.flow_level else "")
        + (f" · bin {bin_seconds} s" if bin_seconds else "")
        + f" · {run_identity(run)}"
        + (f" · route set {route_sets.identity[:8]} ({route_sets.method})" if has_routes else "")
    )
    payload = blob.packed()
    meta: dict[str, Any] = {
        "version": __version__, "theme": theme, "n_links": n_links, "n_verts": int(len(pts)),
        "n_bins": n_bins, "bin_seconds": bin_seconds, "has_routes": has_routes,
        "n_pairs": n_pairs, "n_routes": n_routes, "class_names": _CLASS_NAMES,
        "title": title or _default_title(bins is not None and n_bins > 0, has_routes),
        "note": note, "credit": credit, "logo": bool(logo),
        "provenance": provenance, "source": source, "n_spillback": n_spill,
        "arrays": blob.table, "tokens": _tokens(), "layers": layers,
    }  # fmt: skip
    body = len(payload) + len(PAGE_JS) + len(PAGE_CSS) + len(PAGE_HTML)
    meta["size_note"] = f"{n_links:,} links · {body / 1e6:.1f} MB"

    def escape(text: str) -> str:
        # "<" as a JSON escape: no "</script>" or "<!--" can end or confuse the script element.
        return text.replace("<", "\\u003c")

    page = (
        PAGE_HTML.replace("__CSS__", PAGE_CSS)
        .replace("__TITLE__", _html_text(meta["title"]))
        .replace("__LOGO__", LOGO_PATH)
        .replace("__GENERATOR__", CREATOR.format(version=__version__))
        .replace("__META__", escape(json.dumps(meta, separators=(",", ":"))))
        .replace("__BLOB__", payload)
        .replace("__JS__", PAGE_JS.replace("__LOGO__", LOGO_PATH))
    )
    out = Path(path)
    # Bytes, not text: the same file on every platform (no newline translation).
    out.write_bytes(page.encode("utf-8"))
    return out


def _route_usage(run: Any) -> np.ndarray | None:
    """How many travellers took each route of the run's route sets, or ``None``."""
    choices = run.route_choices()
    if choices is None:
        return None
    routed = choices.route >= 0
    return np.bincount(
        choices.route[routed],
        weights=choices.weight[routed],
        minlength=run.route_sets().route_count,
    )


def _default_title(has_traffic: bool, has_routes: bool) -> str:
    if has_traffic and has_routes:
        return "Traffic and route alternatives"
    return "Traffic by direction" if has_traffic else "Network and route alternatives"


def _html_text(text: str) -> str:
    return text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def _add_routes(
    blob: _Blob,
    sets: Any,
    keep: np.ndarray,
    new_id: np.ndarray,
    max_pairs: int,
    usage: np.ndarray | None = None,
) -> tuple[int, int]:
    """Embed the route sets: only routes wholly inside `keep`, the richest pairs first."""
    set_off = sets.set_offsets().astype(np.int64)
    route_off = sets.route_offsets().astype(np.int64)
    links = sets.links().astype(np.int64)
    n_keys, n_routes = len(set_off) - 1, len(route_off) - 1
    route_len = np.diff(route_off)
    key_of_route = np.repeat(np.arange(n_keys), np.diff(set_off))
    inside = np.add.reduceat(keep[links].astype(np.int64), route_off[:-1]) == route_len
    inside &= route_len > 0
    per_key = np.bincount(key_of_route[inside], minlength=n_keys)
    candidates = np.nonzero(per_key > 0)[0]
    if not len(candidates):
        return 0, 0
    richest = candidates[np.argsort(-per_key[candidates], kind="stable")][: max(max_pairs, 1)]
    chosen = np.zeros(n_keys, dtype=bool)
    chosen[richest] = True
    take = inside & chosen[key_of_route]
    lengths = route_len[take]
    link_of_route = np.repeat(np.arange(n_routes), route_len)
    blob.add("ss", np.concatenate([[0], np.cumsum(per_key[chosen])]), "u32")
    blob.add("rst", np.concatenate([[0], np.cumsum(lengths)]), "u32")
    blob.add("rlk", new_id[links[take[link_of_route]]], "u32")
    blob.add("rc", sets.costs()[take], "f32")
    blob.add("ro", sets.overlaps()[take], "f32")
    if usage is not None:
        blob.add("ru", usage[take], "f32")
    return int(chosen.sum()), int(take.sum())
