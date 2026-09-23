"""``network_read_table``: a ``Network`` from a plain link table.

The only network source before this was OpenStreetMap (or the two synthetic
fixtures, ``examples.manhattan_grid`` and ``examples.toy_network``). A link
table — a CSV, TSV, or a TNTP-style benchmark file (Sioux Falls, Anaheim and
the other standard networks of the transport-research literature) — is
turned into the same ``Network`` a scenario runs on, through the same
class-keyed defaults table an OSM import uses for anything the table itself
does not state.
"""

from __future__ import annotations

import math
import warnings
from collections.abc import Mapping, Sequence
from pathlib import Path
from typing import Any

from openmobisim import _core

__all__ = ["network_read_table"]

#: `RoadClass::Unclassified`'s position in `RoadClass::ALL` (Rust,
#: `core-graph/src/defaults.rs`) — the same order `openmobisim.viz`'s
#: `_interactive._CLASS_NAMES` mirrors, for the same reason: there is no
#: Python-facing name for it to look up by.
_UNCLASSIFIED_INDEX = 10

Row = Mapping[str, Any]
Rows = Sequence[Row]

#: Aliases recognised for each canonical link column (case-insensitive,
#: checked in this order — the first alias present in a row wins). TNTP's own
#: names (``init_node``, ``term_node``, ``capacity``, ``length``,
#: ``free_flow_time``, ``link_type``) are among them, so a TNTP ``~`` header
#: needs no renaming.
_LINK_ALIASES: dict[str, tuple[str, ...]] = {
    "id": ("id", "link_id"),
    "from": ("from", "init_node", "from_node", "source", "tail"),
    "to": ("to", "term_node", "to_node", "target", "head"),
    "length": ("length", "length_m", "dist", "distance"),
    "lanes": ("lanes",),
    # "speed" is deliberately not an alias: the standard TNTP files carry a
    # column of that exact name whose meaning is inconsistent across networks
    # (sometimes a speed limit, sometimes unused and zero) — reading it as
    # free-flow speed silently zeroed the whole diagram on a real file. Prefer
    # `free_flow_time`, which every TNTP file states reliably; give a
    # `free_flow_speed_km_h` column explicitly if that is what you have.
    "free_flow_speed": ("free_flow_speed", "free_flow_speed_km_h"),
    "free_flow_time": ("free_flow_time", "fftt", "free_flow_time_min"),
    "capacity": ("capacity", "cap", "capacity_veh_h"),
    "class": ("class", "type", "link_type", "highway", "road_class"),
    "roundabout": ("roundabout",),
}

#: Aliases recognised for each canonical node column.
_NODE_ALIASES: dict[str, tuple[str, ...]] = {
    "id": ("id", "node", "node_id"),
    "x": ("x", "lon", "longitude"),
    "y": ("y", "lat", "latitude"),
    "signalised": ("signalised", "signal", "signals"),
}

#: Latitude a table's arbitrary x/y is laid out at, and the fallback grid's
#: spacing is measured from — the same convention `core_graph::examples`
#: (`manhattan_grid`) uses for a network that represents nowhere in
#: particular: a mid-latitude so the longitude/latitude metre ratio is
#: representative of nowhere specific, fixed so two reads of the same table
#: place its nodes identically.
_REFERENCE_LATITUDE_DEG = 45.7
_METRES_PER_DEGREE_LAT = 110_574.0


def _metres_per_degree_lon(lat_deg: float) -> float:
    return 111_320.0 * math.cos(math.radians(lat_deg))


_LENGTH_TO_M = {"m": 1.0, "km": 1000.0, "mi": 1609.344}
_SPEED_TO_KM_H = {"km_h": 1.0, "mi_h": 1.609344}
_TIME_TO_S = {"s": 1.0, "min": 60.0, "h": 3600.0}

_TRUE = {"1", "true", "yes", "y"}


def _as_bool(value: Any) -> bool:
    if isinstance(value, bool):
        return value
    if value is None:
        return False
    return str(value).strip().lower() in _TRUE


def _as_float(value: Any) -> float | None:
    if value is None or value == "":
        return None
    try:
        f = float(value)
    except (TypeError, ValueError):
        return None
    return f if math.isfinite(f) else None


def _parse_delimited_text(text: str) -> list[dict[str, str]]:
    """A CSV/TSV file, or a TNTP-style ``~``-headed, ``;``-terminated one.

    TNTP `_net.tntp` files carry a ``<...>`` metadata block (``<NUMBER OF
    ZONES> 24``, skipped: zones are not modelled here), a header line
    prefixed with ``~``, and every row ending in ``;`` — otherwise they are
    tab-separated text. Stripping the ``~``/``;`` and detecting the delimiter
    handles that dialect and a plain CSV/TSV with the same code. **No quoting
    support**: a field may not itself contain the delimiter. Every network
    link table this project reads (TNTP or a hand-made CSV of ids, node
    references and numbers) is fine with that; a `nodes`/`links` argument that
    is not a plain delimited file should be given as in-memory rows instead.
    """
    lines = [ln for ln in text.splitlines() if ln.strip()]
    lines = [ln for ln in lines if not ln.lstrip().startswith("<")]
    if not lines:
        return []
    header_line = lines[0].lstrip()
    if header_line.startswith("~"):
        header_line = header_line[1:]
    delimiter = "\t" if "\t" in header_line else ("," if "," in header_line else None)

    def split(line: str) -> list[str]:
        # TNTP's header starts "~<tab>col1<tab>col2…" but every data row
        # starts "<tab><tab>value1<tab>value2…" — the tilde stands where a
        # tab would, so the two lines' leading blanks do not line up.
        # Dropping every empty field (no real column here is ever blank)
        # fixes the alignment for that dialect and is a no-op for a plain
        # CSV/TSV that has no blank fields to begin with.
        line = line.split(";", 1)[0]
        fields = line.split(delimiter) if delimiter else line.split()
        return [f.strip() for f in fields if f.strip() != ""]

    header = [h.lower() for h in split(header_line)]
    rows: list[dict[str, str]] = []
    for line in lines[1:]:
        fields = split(line)
        if not fields:
            continue
        rows.append(dict(zip(header, fields, strict=False)))
    return rows


def _load_rows(source: str | Rows) -> list[Row]:
    if isinstance(source, str):
        path = Path(source)
        # The string is the table's text, not a path, if there is no such file.
        text = path.read_text(encoding="utf-8") if path.exists() else source
        return _parse_delimited_text(text)
    return list(source)


def _column(row: Row, canonical: str, aliases: Mapping[str, tuple[str, ...]]) -> Any:
    lowered = {str(k).lower(): v for k, v in row.items()}
    for name in aliases[canonical]:
        if name in lowered and lowered[name] not in (None, ""):
            return lowered[name]
    return None


def network_read_table(
    links: str | Rows,
    nodes: str | Rows | None = None,
    *,
    length_unit: str = "m",
    speed_unit: str = "km_h",
    time_unit: str = "min",
    coordinates: str = "lonlat",
    coordinate_scale_m: float = 1.0,
    layout_spacing_m: float = 200.0,
) -> _core.Network:
    """Read a road network from a plain table of links.

    Args:
        links: A path to a delimited text file (CSV, TSV, or a TNTP-style
            ``~``-headed, ``;``-terminated one — the dialect is detected), or
            an in-memory sequence of row mappings, column name to value (for
            example ``df.to_dict("records")``). Recognised columns, by
            canonical name (aliases in parentheses; matching is
            case-insensitive): ``from`` (``init_node``, ``from_node``),
            ``to`` (``term_node``, ``to_node``) — **required**; ``id``
            (auto-numbered if absent); ``length`` (``length_m``, ``dist``);
            ``lanes``; ``free_flow_speed`` (``speed``) or ``free_flow_time``
            (``fftt``) — at most one is read, ``free_flow_speed`` first;
            ``capacity`` (``cap``); ``class`` (``type``, ``link_type``,
            ``highway``) — a name from ``openmobisim.viz``'s road classes
            (``"primary"``, ``"residential"``, …; TNTP's numeric
            ``link_type`` is not one, so it is treated as unrecognised, like
            an unfamiliar OSM tag, with a diagnostic — pass a ``class``
            column of your own if you have the mapping); ``roundabout``. A
            column not given, for a row that does not state it, defaults from
            the class table exactly as an unstated OSM tag does, with the
            same diagnostics for what was filled in — a two-way street is two
            rows, each is one directed link, and every trip's demand is
            already routed on the network this way.
        nodes: A node table in the same two shapes, or ``None`` if `links`
            carries no coordinates at all — then nodes are laid out on a
            deterministic grid (`layout_spacing_m` apart, in the order they
            are first named by a link) purely so a figure has something to
            draw; the layout carries no meaning and is never read by the
            loading. Recognised columns: ``id`` (``node``) — **required**;
            ``x``/``y`` or ``lon``/``lat`` depending on `coordinates`;
            ``signalised`` (``signal``) — every approach to a signalised node
            gets a control delay and a green-time fraction.
        length_unit: The unit of a `links` ``length`` column: ``"m"``
            (the default), ``"km"`` or ``"mi"``.
        speed_unit: The unit of a `links` ``free_flow_speed`` column:
            ``"km_h"`` (the default) or ``"mi_h"``.
        time_unit: The unit of a `links` ``free_flow_time`` column (read only
            when ``free_flow_speed`` is absent, and converted to a speed
            using ``length`` — so a `links` row with a time but no length has
            no free-flow speed and gets the class default instead, with a
            diagnostic the same way a degenerate length does): ``"s"``,
            ``"min"`` (the default — the convention of the standard TNTP
            benchmark networks) or ``"h"``.
        coordinates: What a `nodes` table's position columns are:
            ``"lonlat"`` (the default — real-world ``lon``/``lat`` degrees,
            read and projected exactly as an OSM import's are) or ``"xy"`` (an
            arbitrary local coordinate system, as the standard TNTP benchmark
            networks use for their node files — placed on a flat,
            representative-of-nowhere-in-particular layout, the same
            construction `examples.manhattan_grid` uses for its own grid, so
            it can be drawn but carries no real-world meaning).
        coordinate_scale_m: Metres per unit of a `nodes` table's ``x``/``y``,
            under ``coordinates="xy"``. The default, ``1.0``, assumes the
            table's units already are metres; the standard TNTP node files do
            not document their unit, so check a network's own extent (a
            printed figure, or `report_connectivity`'s counts) before trusting
            it for anything but a schematic plot.
        layout_spacing_m: Grid spacing for the coordinate-free fallback layout
            (`nodes` is ``None`` and `links` carries no coordinates).

    Returns:
        A ``Network``, exactly as ``network_read_osm`` or
        ``examples.manhattan_grid`` return one — usable by a ``Scenario``,
        `report_connectivity`, and every ``openmobisim.viz`` figure.

    Raises:
        ValueError: If `links` has no usable rows, a row's ``from``/``to`` is
            missing, `length_unit`/`speed_unit`/`time_unit`/`coordinates` is
            not one of the values above, or the resulting network holds no
            usable links (the same failures ``network_read_osm`` can raise).

    Example:
        Sioux Falls, from its published TNTP files (``sioux_falls_net.tntp``,
        tab-separated, lengths in miles, times in minutes; a node file with
        arbitrary ``x``/``y``)::

            net = network_read_table(
                "data/sioux_falls_net.tntp",
                "data/sioux_falls_node.tntp",
                length_unit="mi",
                coordinates="xy",
            )
    """
    if length_unit not in _LENGTH_TO_M:
        raise ValueError(f"length_unit must be one of {sorted(_LENGTH_TO_M)}, got {length_unit!r}")
    if speed_unit not in _SPEED_TO_KM_H:
        raise ValueError(f"speed_unit must be one of {sorted(_SPEED_TO_KM_H)}, got {speed_unit!r}")
    if time_unit not in _TIME_TO_S:
        raise ValueError(f"time_unit must be one of {sorted(_TIME_TO_S)}, got {time_unit!r}")
    if coordinates not in ("lonlat", "xy"):
        raise ValueError(f'coordinates must be "lonlat" or "xy", got {coordinates!r}')

    link_rows = _load_rows(links)
    if not link_rows:
        raise ValueError("links has no usable rows")

    link_ids: list[str] = []
    link_from: list[str] = []
    link_to: list[str] = []
    link_class: list[str] = []
    link_lanes: list[int | None] = []
    link_length_m: list[float | None] = []
    link_capacity_veh_h: list[float | None] = []
    link_free_flow_km_h: list[float | None] = []
    link_roundabout: list[bool] = []
    seen_node_ids: dict[str, None] = {}  # insertion-ordered set, for the fallback layout
    width = len(str(len(link_rows)))

    for i, row in enumerate(link_rows):
        origin = _column(row, "from", _LINK_ALIASES)
        destination = _column(row, "to", _LINK_ALIASES)
        if origin is None or destination is None:
            raise ValueError(f"links row {i} has no from/to (or init_node/term_node)")
        origin, destination = str(origin), str(destination)
        seen_node_ids.setdefault(origin, None)
        seen_node_ids.setdefault(destination, None)

        given_id = _column(row, "id", _LINK_ALIASES)
        link_ids.append(str(given_id) if given_id is not None else f"L{i:0{width}d}")
        link_from.append(origin)
        link_to.append(destination)
        link_class.append(str(_column(row, "class", _LINK_ALIASES) or ""))
        lanes = _column(row, "lanes", _LINK_ALIASES)
        link_lanes.append(int(lanes) if lanes is not None else None)

        length_m = _as_float(_column(row, "length", _LINK_ALIASES))
        if length_m is not None:
            length_m *= _LENGTH_TO_M[length_unit]
        link_length_m.append(length_m)

        link_capacity_veh_h.append(_as_float(_column(row, "capacity", _LINK_ALIASES)))

        speed_km_h = _as_float(_column(row, "free_flow_speed", _LINK_ALIASES))
        if speed_km_h is not None:
            speed_km_h *= _SPEED_TO_KM_H[speed_unit]
        elif length_m is not None:
            time_s = _as_float(_column(row, "free_flow_time", _LINK_ALIASES))
            if time_s is not None and time_s > 0:
                time_s *= _TIME_TO_S[time_unit]
                speed_km_h = (length_m / 1000.0) / (time_s / 3600.0)
        link_free_flow_km_h.append(speed_km_h)

        link_roundabout.append(_as_bool(_column(row, "roundabout", _LINK_ALIASES)))

    node_rows = _load_rows(nodes) if nodes is not None else None
    node_ids: list[str] = []
    node_lon: list[float] = []
    node_lat: list[float] = []
    node_signalised: list[bool] = []

    if node_rows is not None:
        for row in node_rows:
            given_id = _column(row, "id", _NODE_ALIASES)
            if given_id is None:
                continue
            node_ids.append(str(given_id))
            x = _as_float(_column(row, "x", _NODE_ALIASES)) or 0.0
            y = _as_float(_column(row, "y", _NODE_ALIASES)) or 0.0
            if coordinates == "lonlat":
                node_lon.append(x)
                node_lat.append(y)
            else:
                lat = _REFERENCE_LATITUDE_DEG + y * coordinate_scale_m / _METRES_PER_DEGREE_LAT
                lon = 4.8 + x * coordinate_scale_m / _metres_per_degree_lon(_REFERENCE_LATITUDE_DEG)
                node_lon.append(lon)
                node_lat.append(lat)
            node_signalised.append(_as_bool(_column(row, "signalised", _NODE_ALIASES)))
    else:
        # No node table and the links carry no coordinates either: a
        # deterministic placeholder grid, in first-appearance order, so a
        # figure has something to draw. Never read by the loading.
        cols = max(1, math.ceil(math.sqrt(len(seen_node_ids))))
        for i, node_id in enumerate(seen_node_ids):
            node_ids.append(node_id)
            row_i, col_i = divmod(i, cols)
            lat = _REFERENCE_LATITUDE_DEG + row_i * layout_spacing_m / _METRES_PER_DEGREE_LAT
            lon = 4.8 + col_i * layout_spacing_m / _metres_per_degree_lon(_REFERENCE_LATITUDE_DEG)
            node_lon.append(lon)
            node_lat.append(lat)
            node_signalised.append(False)

    network = _core.network_from_columns(
        node_ids=node_ids,
        node_lon=node_lon,
        node_lat=node_lat,
        node_signalised=node_signalised,
        link_ids=link_ids,
        link_from=link_from,
        link_to=link_to,
        link_class=link_class,
        link_lanes=link_lanes,
        link_length_m=link_length_m,
        link_capacity_veh_h=link_capacity_veh_h,
        link_free_flow_km_h=link_free_flow_km_h,
        link_signalised=[False] * len(link_ids),
        link_roundabout=link_roundabout,
    )
    # A table-read network has no `report_import`-style diagnostics yet (the
    # Rust side records an unrecognised `class` value the same way an
    # unfamiliar OSM tag is recorded, but nothing surfaces that count to
    # Python for this source). Catch the common case — a `link_type` column
    # of numbers, as the standard TNTP files carry, none of which is a road
    # class name — with one aggregate warning rather than none at all.
    given_empty = sum(1 for c in link_class if not c)
    fell_back = int((network.link_class() == _UNCLASSIFIED_INDEX).sum())
    if fell_back > given_empty:
        warnings.warn(
            f"{fell_back - given_empty} link(s) had a `class` value not recognised as a road "
            "class and were treated as unclassified (the design's fallback rule for an "
            'unfamiliar tag) — pass recognised names ("primary", "residential", …) if that '
            "was not intended; TNTP's numeric `link_type` is not one of them.",
            stacklevel=2,
        )

    # A stated capacity can ask for more than the class row's jam density can
    # carry at the link's free-flow speed (design §12.1: no triangular diagram
    # exists past that point) — common precisely where `class` fell back to
    # unclassified's single, modest lane, as a high-capacity TNTP corridor
    # does. The core resolves it (capacity is reduced to what the diagram can
    # hold, recorded as a diagnostic) rather than failing, but nothing in
    # `network_from_columns` surfaces that diagnostic to Python yet, so a
    # requested capacity silently becoming a different number needs its own
    # check here: sort by id, the same rule `RoadNetworkBuilder::build` orders
    # links by, so this lines up with `link_capacity_pcu_h()` regardless of
    # what the caller's own ids look like.
    order = sorted(range(len(link_ids)), key=lambda i: link_ids[i])
    actual_capacity = network.link_capacity_pcu_h()
    reduced = sum(
        1
        for rank, i in enumerate(order)
        if link_capacity_veh_h[i] is not None
        and abs(actual_capacity[rank] - link_capacity_veh_h[i]) > 1.0
    )
    if reduced:
        warnings.warn(
            f"{reduced} link(s) asked for a capacity the class row's jam density cannot carry "
            "at the link's free-flow speed, and were reduced to the largest value that diagram "
            "admits (the rule for an inconsistent triangular diagram — not a failure, "
            "but the capacity used is not the one asked for). A `class` that fell back to "
            "unclassified assumes a single modest lane; give a `lanes` value, or a `class` this "
            "table recognises, if you know the real one.",
            stacklevel=2,
        )
    return network
