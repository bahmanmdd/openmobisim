"""Networks as GMNS files: ``network_write_gmns`` and ``network_read_gmns``.

GMNS, the General Modeling Network Specification (Zephyr Foundation), is a plain-CSV
standard for routable networks: a ``node.csv`` and a ``link.csv``, with units and the
coordinate system in an optional ``config.csv``. It is what DTALite, osm2gmns and path4gmns
read and write, so a network written here opens in those tools and theirs opens here.

**One folder per layer.** A network is written as ``road/``, ``bike/`` and ``walk/``, each a
GMNS network; read back, the road folder is the road network and the other two are its
bike and walk layers, exactly as they were (an OpenStreetMap-built layer keeps its cycle
infrastructure). A single GMNS folder, as other tools write, is read as the road network;
its bike and walk layers are then derived from it (bikes on every road but motorways, in the
road's direction; walking on every road but motorways and cycleways, both ways).

**What is written** — ``link.csv``: ``link_id``, ``from_node_id``, ``to_node_id``,
``directed`` (always true: a two-way street is two links), ``length`` (m), ``free_speed``
(km/h), and on the road layer ``lanes``, ``capacity`` (PCU/h **per lane**, GMNS's convention),
``facility_type`` (the road class: ``primary``, ``residential``, …), ``allowed_uses``
(``auto``, ``bus``, ``bike``, ``walk``), ``roundabout`` (an extra column), and ``geometry``
(WKT); on the bike layer ``bike_facility`` (``separated``, ``lane`` or ``mixed``).
``node.csv``: ``node_id``, ``x_coord``, ``y_coord`` (WGS84 longitude and latitude) and
``ctrl_type`` (``signal`` at a signalised node). Ids are the network's own indices,
zero-padded, so a network read back has its nodes and links in the same order.

**What is read** — the same columns, the ``config.csv`` units (``long_length`` ``m``, ``km``,
``mi`` or ``ft``; ``speed`` ``kmh`` or ``mph``) and, for a link without a ``facility_type``,
the class defaults, as a link table's (``network_read_table``). A link whose ``directed`` is
false becomes two links. A link's geometry is not read (links are straight between their
nodes for figures; lengths come from the ``length`` column). ``network_check`` checks a network
read from files before it is run.
"""

from __future__ import annotations

import csv
import math
import time
from collections.abc import Mapping
from pathlib import Path
from typing import Any

import numpy as np

from openmobisim import _core
from openmobisim.network import LINK_CLASSES, network_read_table

__all__ = ["network_read_gmns", "network_write_gmns"]

_INFRASTRUCTURE = ["mixed", "lane", "separated"]
_NOT_DRIVEN = {"pedestrian": "walk", "footway": "walk", "cycleway": "bike", "ferry": "walk"}
_LONG_LENGTH_TO_M = {
    "m": 1.0,
    "meter": 1.0,
    "meters": 1.0,
    "km": 1000.0,
    "kilometer": 1000.0,
    "mi": 1609.344,
    "mile": 1609.344,
    "miles": 1609.344,
    "ft": 0.3048,
    "feet": 0.3048,
}
_SPEED_TO_KM_H = {
    "kmh": 1.0,
    "km/h": 1.0,
    "kph": 1.0,
    "km_h": 1.0,
    "mph": 1.609344,
    "mi/h": 1.609344,
}
_SIGNAL = ("signal", "signalized", "signalised", "traffic_signals")


def _write(path: Path, header: list[str], rows: list[list[Any]]) -> None:
    with path.open("w", newline="", encoding="utf-8") as f:
        w = csv.writer(f, lineterminator="\n")
        w.writerow(header)
        w.writerows(rows)


def _read(path: Path) -> list[dict[str, str]]:
    with path.open(newline="", encoding="utf-8-sig") as f:
        return [
            {k.strip().lower(): (v or "").strip() for k, v in row.items() if k}
            for row in csv.DictReader(f)
        ]


def _wkt(points: np.ndarray) -> str:
    return "LINESTRING (" + ", ".join(f"{x:.7f} {y:.7f}" for x, y in points) + ")"


def _write_layer(network: _core.Network, folder: Path, layer: str, geometry: bool) -> int:
    folder.mkdir(parents=True, exist_ok=True)
    n_links, n_nodes = network.link_count, network.node_count
    coords, offsets = network.link_geometry()
    coords, offsets = np.asarray(coords, dtype=float), np.asarray(offsets, dtype=np.int64)
    frm, to = np.asarray(network.link_from()), np.asarray(network.link_to())
    lw, nw = len(str(max(n_links - 1, 0))), len(str(max(n_nodes - 1, 0)))
    # Node positions: where their links start and end.
    pos = np.full((n_nodes, 2), np.nan)
    if n_links:
        pos[frm] = coords[offsets[:-1]]
        pos[to] = coords[offsets[1:] - 1]
    signal = np.asarray(network.node_signalised(), dtype=bool) if layer == "road" else None
    node_rows = []
    for i in range(n_nodes):
        if math.isnan(pos[i, 0]):
            continue
        row = [f"{i:0{nw}d}", f"{pos[i, 0]:.7f}", f"{pos[i, 1]:.7f}"]
        if signal is not None:
            row.append("signal" if signal[i] else "")
        node_rows.append(row)
    node_header = ["node_id", "x_coord", "y_coord"] + (["ctrl_type"] if signal is not None else [])
    _write(folder / "node.csv", node_header, node_rows)
    length = np.asarray(network.link_length_m())
    speed = np.asarray(network.link_speed_km_h())
    cls = np.asarray(network.link_class())
    header = [
        "link_id",
        "from_node_id",
        "to_node_id",
        "directed",
        "length",
        "free_speed",
        "facility_type",
    ]
    if layer == "road":
        lanes = np.asarray(network.link_lanes())
        cap = np.asarray(network.link_capacity_pcu_h())
        roundabout = np.asarray(network.link_roundabout(), dtype=bool)
        drivable = np.asarray(network.link_drivable(), dtype=bool)
        header += ["lanes", "capacity", "allowed_uses", "roundabout"]
    elif layer == "bike":
        infra = np.asarray(network.link_infrastructure())
        header += ["bike_facility", "allowed_uses"]
    else:
        header += ["allowed_uses"]
    if geometry:
        header.append("geometry")
    rows = []
    for i in range(n_links):
        name = LINK_CLASSES[cls[i]] if cls[i] < len(LINK_CLASSES) else "unclassified"
        row: list[Any] = [
            f"{i:0{lw}d}",
            f"{frm[i]:0{nw}d}",
            f"{to[i]:0{nw}d}",
            "true",
            f"{length[i]:.6f}",
            f"{speed[i]:.10g}",
            name,
        ]
        if layer == "road":
            n = max(int(lanes[i]), 1)
            uses = (
                "auto"
                if drivable[i]
                else ("bus" if name == "busway" else _NOT_DRIVEN.get(name, "walk"))
            )
            row += [n, f"{cap[i] / n:.10g}", uses, "true" if roundabout[i] else ""]
        elif layer == "bike":
            row += [
                _INFRASTRUCTURE[infra[i]] if infra[i] < len(_INFRASTRUCTURE) else "mixed",
                "bike",
            ]
        else:
            row += ["walk"]
        if geometry:
            row.append(_wkt(coords[offsets[i] : offsets[i + 1]]))
        rows.append(row)
    _write(folder / "link.csv", header, rows)
    _write(
        folder / "config.csv",
        ["dataset_name", "long_length", "speed", "crs", "geometry_field_format"],
        [[f"openmobisim {layer}", "m", "kmh", "EPSG:4326", "WKT"]],
    )
    return n_links


def network_write_gmns(
    network: _core.Network, folder: str, *, layers: bool = True, geometry: bool = True
) -> dict[str, int]:
    """Write ``network`` as GMNS files: ``road/``, and ``bike/`` and ``walk/`` with ``layers``.

    See the module docs for the columns. Every file is plain CSV; the street geometry
    (``geometry``, WKT) makes the files larger and lets GIS tools draw them, and is not needed
    to read the network back.

    Args:
        network: The road network (a handle from ``network_read_osm``,
            ``network_read_table``, ``network_read_gmns`` or ``examples``).
        folder: Where to write; made if missing.
        layers: Also write the bike and walk layers.
        geometry: Write each link's polyline.

    Returns:
        Links written per layer.
    """
    if network.layer_name != "road":
        raise ValueError("write the road network's handle; its layers are written with it")
    out = Path(folder)
    counts = {"road": _write_layer(network, out / "road", "road", geometry)}
    if layers:
        for layer in ("bike", "walk"):
            counts[layer] = _write_layer(network.layer(layer), out / layer, layer, geometry)
    return counts


def _units(folder: Path) -> tuple[float, float]:
    """Metres per length unit and km/h per speed unit, from ``config.csv``.

    Metres and km/h if there is no ``config.csv``.
    """
    config = folder / "config.csv"
    if not config.exists():
        return 1.0, 1.0
    rows = _read(config)
    row = rows[0] if rows else {}
    length_unit = row.get("long_length", "m").lower() or "m"
    speed_unit = row.get("speed", "kmh").lower().replace(" ", "") or "kmh"
    if length_unit not in _LONG_LENGTH_TO_M:
        known = sorted(_LONG_LENGTH_TO_M)
        raise ValueError(f"config.csv long_length {length_unit!r} is not one of {known}")
    if speed_unit not in _SPEED_TO_KM_H:
        raise ValueError(f"config.csv speed {speed_unit!r} is not one of {sorted(_SPEED_TO_KM_H)}")
    crs = row.get("crs", "").upper()
    if crs and crs not in ("EPSG:4326", "WGS84", "WGS 84", "LONLAT"):
        raise ValueError(
            f"config.csv crs {crs!r}: only longitude and latitude (EPSG:4326) are read; "
            "reproject the node coordinates first"
        )
    return _LONG_LENGTH_TO_M[length_unit], _SPEED_TO_KM_H[speed_unit]


def _directed(row: dict[str, str]) -> bool:
    return row.get("directed", "true").lower() not in ("false", "0", "no", "n")


def _road_rows(folder: Path) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    to_m, to_km_h = _units(folder)
    links: list[dict[str, Any]] = []
    for row in _read(folder / "link.csv"):
        lanes = row.get("lanes", "")
        n = int(float(lanes)) if lanes else None
        cap = row.get("capacity", "")
        length = row.get("length", "")
        speed = row.get("free_speed", "")
        uses = {u.strip() for u in row.get("allowed_uses", "").split(",") if u.strip()}
        cls = row.get("facility_type", "")
        if cls not in LINK_CLASSES:
            cls = ""
        if uses and not uses & {"auto", "car", "all"} and not cls:
            cls = "busway" if "bus" in uses else ("cycleway" if uses == {"bike"} else "footway")
        base = {
            "from": row["from_node_id"],
            "to": row["to_node_id"],
            "class": cls,
            "lanes": n,
            # GMNS capacity is per lane; the table reader takes a link's total.
            "capacity": float(cap) * (n or 1) if cap else None,
            "length": float(length) * to_m if length else None,
            "free_flow_speed": float(speed) * to_km_h if speed else None,
            "roundabout": row.get("roundabout", ""),
        }
        links.append({"id": row["link_id"], **base})
        if not _directed(row):
            links.append(
                {"id": f"{row['link_id']}_r", **base, "from": base["to"], "to": base["from"]}
            )
    nodes = [
        {
            "id": r["node_id"],
            "x": r["x_coord"],
            "y": r["y_coord"],
            "signalised": r.get("ctrl_type", "").lower() in _SIGNAL,
        }
        for r in _read(folder / "node.csv")
    ]
    return links, nodes


def _attach_layer(road: _core.Network, folder: Path, layer: str) -> _core.Network:
    to_m, to_km_h = _units(folder)
    nodes = _read(folder / "node.csv")
    links = _read(folder / "link.csv")
    cols: dict[str, list[Any]] = {
        k: [] for k in ("ids", "frm", "to", "cls", "speed", "infra", "length")
    }
    for row in links:
        pairs = [(row["link_id"], row["from_node_id"], row["to_node_id"])]
        if not _directed(row):
            pairs.append((f"{row['link_id']}_r", row["to_node_id"], row["from_node_id"]))
        for link_id, a, b in pairs:
            cols["ids"].append(link_id)
            cols["frm"].append(a)
            cols["to"].append(b)
            cols["cls"].append(
                row.get("facility_type") or ("cycleway" if layer == "bike" else "footway")
            )
            speed = row.get("free_speed", "")
            if not speed:
                raise ValueError(f"{folder / 'link.csv'}: link {link_id} has no free_speed")
            cols["speed"].append(float(speed) * to_km_h)
            cols["infra"].append(row.get("bike_facility", "").lower())
            length = row.get("length", "")
            cols["length"].append(float(length) * to_m if length else None)
    return _core.network_with_layer(
        road,
        layer,
        [r["node_id"] for r in nodes],
        [float(r["x_coord"]) for r in nodes],
        [float(r["y_coord"]) for r in nodes],
        cols["ids"],
        cols["frm"],
        cols["to"],
        cols["cls"],
        cols["speed"],
        cols["infra"],
        cols["length"],
    )


def network_read_gmns(
    folder: str, network_options: Mapping[str, float] | None = None
) -> _core.Network:
    """Read a network from GMNS files.

    ``folder`` is either one GMNS network (``node.csv``, ``link.csv``, optionally
    ``config.csv``), read as the road network with its bike and walk layers derived from it,
    or a folder of ``road/``, ``bike/`` and ``walk/`` GMNS networks as ``network_write_gmns``
    writes them, read as the road network and its own layers. See the module docs for the
    columns read.

    Args:
        folder: The folder.
        network_options: Network parameters by name, as ``network_read_table`` takes them.

    Returns:
        A ``Network``, as ``network_read_osm`` returns one.

    Raises:
        ValueError: If the folder holds no GMNS network, a ``config.csv`` names units or a
            coordinate system that is not read, or a table is malformed.
    """
    started = time.perf_counter()
    root = Path(folder)
    road_folder = root / "road" if (root / "road" / "link.csv").exists() else root
    if not (road_folder / "link.csv").exists() or not (road_folder / "node.csv").exists():
        raise ValueError(
            f"{folder} holds no GMNS network (node.csv and link.csv, or road/ with them)"
        )
    links, nodes = _road_rows(road_folder)
    network = network_read_table(
        links, nodes, length_unit="m", speed_unit="km_h", network_options=network_options
    )
    if road_folder != root:
        for layer in ("bike", "walk"):
            if (root / layer / "link.csv").exists():
                network = _attach_layer(network, root / layer, layer)
    network.read_s = time.perf_counter() - started
    return network
