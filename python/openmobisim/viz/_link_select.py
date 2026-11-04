"""``map_link_select``: the select-link analysis as one self-contained HTML page.

The page embeds, compressed, the road network (and the bike layer, when bikes ran), each layer's
**paths** (the links of each route a user travelled), and the **groups** of users: a path, an
origin-destination pair, a mode and a departure bin, with the trips and the people they stand
for. Clicking a link sums the groups whose path crosses it, by pair and mode, into the table
``Run.link_select`` gives; it draws where those users came from and went (each origin and
destination sized by people) and their flow on every other link.
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import numpy as np

from openmobisim import __version__
from openmobisim import _link_select as sl
from openmobisim._paths import show_path
from openmobisim.viz import _geometry as geo
from openmobisim.viz._canvas import ROAD_TIER_WIDTHS, road_tier
from openmobisim.viz._figure import CREATOR
from openmobisim.viz._interactive import _UNIT_M, _Blob, _html_text, _quantise, _tokens
from openmobisim.viz._interactive_page import PAGE_CSS
from openmobisim.viz._link_select_page import SELECT_CSS, SELECT_HTML, SELECT_JS
from openmobisim.viz._logo import LOGO_PATH
from openmobisim.viz._modes import _LABEL as _MODE_LABELS
from openmobisim.viz._provenance import network_source, run_identity
from openmobisim.viz._style import get_theme

__all__ = ["map_link_select"]

#: The departure bins without a run's own: a quarter hour.
_DEFAULT_BIN_S = 900


def _embed_geometry(blob: _Blob, prefix: str, layer: Any, lon0: float, lat0: float) -> None:
    coords, offsets = layer.link_geometry()
    pts = geo.project_lonlat(coords, lon0, lat0)
    q = _quantise(pts)
    delta = np.diff(q, axis=0, prepend=np.zeros((1, 2), dtype=np.int64))
    blob.add(prefix + "vs", np.asarray(offsets, dtype=np.int64), "u32")
    blob.add(prefix + "dx", delta[:, 0], "i32")
    blob.add(prefix + "dy", delta[:, 1], "i32")
    blob.add(prefix + "nf", np.asarray(layer.link_from()), "u32")
    blob.add(prefix + "nt", np.asarray(layer.link_to()), "u32")
    blob.add(prefix + "len", np.asarray(layer.link_length_m()), "f32")


def _embed_users(
    blob: _Blob,
    prefix: str,
    users: sl.Users,
    pair_of_row: np.ndarray,
    mode_of_row: np.ndarray,
    bin_of_row: np.ndarray,
    weight: np.ndarray,
) -> dict[str, Any]:
    """Embed one layer's used paths and its groups, sorted by path; return their counts."""
    used = np.unique(users.user_path)
    new = np.full(len(users.path_offsets) - 1, -1, dtype=np.int64)
    new[used] = np.arange(len(used))
    lengths = users.path_offsets[used + 1] - users.path_offsets[used]
    offsets = np.concatenate([[0], np.cumsum(lengths)])
    links = (
        np.concatenate(
            [users.path_links[users.path_offsets[p] : users.path_offsets[p + 1]] for p in used]
        )
        if len(used)
        else np.zeros(0, dtype=np.int64)
    )
    rows = users.user_row
    # A trip with two legs on one path (two walks along the same streets) is one user of it.
    keys = np.unique(np.column_stack([new[users.user_path], rows]), axis=0)
    path, rows = keys[:, 0], keys[:, 1]
    group_keys = np.column_stack([path, pair_of_row[rows], mode_of_row[rows], bin_of_row[rows]])
    groups, inverse = np.unique(group_keys, axis=0, return_inverse=True)
    inverse = inverse.reshape(-1)
    trips = np.bincount(inverse, minlength=len(groups))
    people = np.bincount(inverse, weights=weight[rows], minlength=len(groups))
    starts = np.searchsorted(groups[:, 0], np.arange(len(used) + 1))
    blob.add(prefix + "_po", offsets, "u32")
    blob.add(prefix + "_pl", links, "u32")
    blob.add(prefix + "_pg", starts, "u32")
    blob.add(prefix + "_gpath", groups[:, 0], "u32")
    blob.add(prefix + "_gpair", groups[:, 1], "u32")
    blob.add(prefix + "_gmode", groups[:, 2], "u8")
    blob.add(prefix + "_gbin", groups[:, 3], "u32")
    blob.add(prefix + "_gt", trips, "u32")
    blob.add(prefix + "_gp", people, "f32")
    return {"paths": int(len(used)), "groups": int(len(groups)), "people": float(people.sum())}


def map_link_select(
    run: Any,
    path: str | Path | None = None,
    *,
    zones: Any = None,
    link: int | None = None,
    theme: str = "paper",
    title: str | None = None,
    bin_s: int | None = None,
    quiet: bool = False,
) -> Path:
    """Write the select-link analysis of a run as one interactive HTML file.

    **Click a link** (a road, or a bike link with the bike layer chosen) to see the
    origin-destination pairs whose trips cross it, as ``Run.link_select`` counts them: each
    pair's trips by mode, their share of the link's users and their share of the pair's
    travellers. The map draws where those users came from (filled discs) and went (rings), each
    sized by people, and their flow on every other link, so the roads that feed the link and
    those it feeds read at a glance. **Departures**: all of them together (the default), or one
    departure bin at a time with the slider. **Hover** a row of the table to see that pair
    alone. The page needs no server, no network and no library.

    Args:
        run: A ``Run`` from ``Scenario.run()``. The bike layer is offered when the run has
            per-link results (``link_bin_s``) and bikes ran.
        path: Where to write the ``.html`` file; ``link_select.html`` in the run's output folder
            by default.
        zones: ``{zone: (lon, lat)}``: pairs of zones (each trip end at the nearest zone point),
            as ``Run.link_select``'s; ``None``: pairs of road nodes.
        link: The road link shown first; the one with the most people by default.
        theme: The starting theme, ``"paper"`` or ``"night"``; the page has a button to switch.
        title: The page title.
        bin_s: The departure bins' length in seconds: the run's ``link_bin_s``, or 900.
        quiet: Do not say where the file was written.

    Returns:
        The path written.

    Raises:
        ValueError: If `theme` is unknown, the run does not carry its network or keep its
            journeys, or the region is too large to store (over about 200 km across).
    """
    get_theme(theme)
    network = run.network
    if network is None:
        raise ValueError("this run does not carry its network")
    coords, _ = network.link_geometry()
    lon0, lat0 = float(coords[:, 0].mean()), float(coords[:, 1].mean())
    bins = run.link_bins()
    if bin_s is None:
        bin_s = int(bins.bin_seconds) if bins is not None else _DEFAULT_BIN_S
    if bin_s <= 0:
        raise ValueError("bin_s must be a positive number of seconds")

    tr = run.trip_results()
    departure = np.asarray(tr["departure_s"], dtype=float)
    weight = np.asarray(tr["weight"], dtype=float)
    window = getattr(run, "window", None)
    start_s = int(window["start_s"]) if window else int(departure.min()) if len(departure) else 0
    end_s = int(window["end_s"]) if window else int(departure.max()) + 1 if len(departure) else 1
    first_bin = start_s // bin_s
    n_bins = max(1, -(-end_s // bin_s) - first_bin)
    bin_of_row = np.clip((departure // bin_s).astype(np.int64) - first_bin, 0, n_bins - 1)
    modes = sorted({m for m in tr["mode"] if m is not None})
    mode_index = {m: i for i, m in enumerate(modes)}
    mode_of_row = np.array([mode_index.get(m, 0) for m in tr["mode"]], dtype=np.int64)
    origin, destination, names, place_ll = sl.places(run, zones)
    pair_keys, pair_of_row = np.unique(
        np.column_stack([origin, destination]), axis=0, return_inverse=True
    )
    pair_of_row = pair_of_row.reshape(-1)

    blob = _Blob()
    _embed_geometry(blob, "r", network, lon0, lat0)
    tier = road_tier(network)
    if tier is not None:
        blob.add("rt", tier, "u8")
    layers: dict[str, Any] = {}
    layers["road"] = _embed_users(
        blob, "r", sl.layer_users(run, "road"), pair_of_row, mode_of_row, bin_of_row, weight
    )
    if bins is not None:
        bike = sl.layer_users(run, "bike")
        if len(bike.user_row):
            _embed_geometry(blob, "b", network.layer("bike"), lon0, lat0)
            layers["bike"] = _embed_users(
                blob, "b", bike, pair_of_row, mode_of_row, bin_of_row, weight
            )
    # The places, and each pair's people by departure bin (every mode): share_pair's denominators.
    xy = _quantise(geo.project_lonlat(place_ll, lon0, lat0))
    blob.add("px", xy[:, 0], "i32")
    blob.add("py", xy[:, 1], "i32")
    blob.add("q_o", pair_keys[:, 0], "u32")
    blob.add("q_d", pair_keys[:, 1], "u32")
    cells, inverse = np.unique(
        np.column_stack([pair_of_row, bin_of_row]), axis=0, return_inverse=True
    )
    blob.add("t_pair", cells[:, 0], "u32")
    blob.add("t_bin", cells[:, 1], "u32")
    blob.add("t_p", np.bincount(inverse.reshape(-1), weights=weight, minlength=len(cells)), "f32")

    start = link
    if start is None and layers["road"]["groups"]:
        users = sl.layer_users(run, "road")
        rows = users.user_row
        counts = np.zeros(int(network.link_count))
        lengths = np.diff(users.path_offsets)
        per_path = np.bincount(users.user_path, weights=weight[rows], minlength=len(lengths))
        np.add.at(counts, users.path_links, np.repeat(per_path, lengths))
        start = int(np.argmax(counts))
    provenance = (
        f"{__version__} · run {run.run_id} · level {run.flow_level}"
        + f" · departure bins {bin_s} s · {run_identity(run)}"
    )
    meta: dict[str, Any] = {
        "version": __version__, "theme": theme, "n_links": int(network.link_count),
        "title": title or "Select link: who uses a link",
        "provenance": provenance, "source": network_source(network),
        "arrays": blob.table, "tokens": _tokens(), "layers": layers,
        "road_tiers": list(ROAD_TIER_WIDTHS), "unit_m": _UNIT_M,
        "places": [str(n) if zones is not None else f"node {n}" for n in names],
        "modes": modes, "mode_names": [_MODE_LABELS.get(m, m) for m in modes],
        "bin_s": bin_s, "first_bin": first_bin, "n_bins": n_bins,
        "start": {"layer": "road", "link": start if start is not None else -1},
        "zones": zones is not None,
    }  # fmt: skip

    def escape(text: str) -> str:
        return text.replace("<", "\\u003c")

    page = (
        SELECT_HTML.replace("__CSS__", PAGE_CSS + SELECT_CSS)
        .replace("__TITLE__", _html_text(meta["title"]))
        .replace("__LOGO__", LOGO_PATH)
        .replace("__GENERATOR__", CREATOR.format(version=__version__))
        .replace("__META__", escape(json.dumps(meta, separators=(",", ":"))))
        .replace("__BLOB__", blob.packed())
        .replace("__JS__", SELECT_JS)
    )
    out = Path(run.timings_path).parent / "link_select.html" if path is None else Path(path)
    out.write_bytes(page.encode("utf-8"))
    if not quiet:
        show_path("Select-link map", out)
    return out
