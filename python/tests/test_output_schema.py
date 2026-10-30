"""The output files keep their schema: a contract checked against a versioned file.

``openmobisim/data/schema_v4.json`` lists, for every file a run writes, its columns in order with
their Arrow type, whether they may be null and their unit, and the Parquet metadata keys it
carries; for ``manifest.json`` its top-level keys; for ``timings.csv`` its header. A run of the toy
network with transit, parkings, a docked bike service, a taxi fleet, classes and every mode writes
them all,
and each must match: a column renamed, removed, reordered, retyped or made nullable fails here.
Such a change is deliberate only with ``SCHEMA_VERSION`` raised (``io-parquet``) and the file
rewritten: ``python python/tests/test_output_schema.py --write`` (the units of new columns are
then to fill).
"""

from __future__ import annotations

import csv
import json
import sys
import tempfile
from pathlib import Path
from typing import Any

import openmobisim as ms
import pytest

SCHEMA = Path(ms.__file__).parent / "data" / "schema_v4.json"
VERSION_KEY = b"openmobisim.schema_version"


def _run(folder: Path) -> Path:
    net = ms.examples.toy_network()
    rows = ms.examples.trips_random(net, 80, seed=3, min_m=100.0, max_m=2000.0, spread_s=1800)
    classes = ms.examples.traveller_classes()
    rows = ms.demand_assign_classes(rows, classes, seed=1)
    out = folder / "schema-run"
    # A docked bike service, so the files only a run with a service writes are checked too:
    # three docks on the network's vertices, the first at a stop's hub.
    coords, _ = net.link_geometry()
    docks = ms.dock_read_table(
        [
            {"dock_id": f"d{i}", "lon": p[0], "lat": p[1], "capacity": 6, "bikes": 3}
            for i, p in enumerate(coords[[0, len(coords) // 2, -1]])
        ]
    )
    # And a taxi fleet (S302): two vehicles and a charger, so its three files are checked too.
    vehicles = ms.vehicle_read_table(
        [{"vehicle_id": f"v{i}", "lon": p[0], "lat": p[1]} for i, p in enumerate(coords[[0, -1]])]
    )
    chargers = ms.charger_read_table(
        [{"charger_id": "c", "lon": coords[0][0], "lat": coords[0][1], "plugs": 1}]
    )
    ms.Scenario.from_parts(
        net,
        rows,
        classes=classes,
        transit=ms.examples.toy_network_transit(),
        parkings=ms.examples.toy_network_parkings(),
        services=[ms.service_bike_docked(docks), ms.service_taxi(vehicles, chargers)],
        link_bin_s=300,
    ).run("schema-run", output_dir=str(out), quiet=True)
    return out


def _written(out: Path) -> dict[str, Any]:
    import pyarrow.parquet as parquet

    files: dict[str, Any] = {}
    for path in sorted(out.glob("*.parquet")):
        f = parquet.ParquetFile(path)
        meta = f.metadata.metadata or {}
        files[path.name] = {
            "columns": [
                {"name": c.name, "type": str(c.type), "nullable": c.nullable}
                for c in f.schema_arrow
            ],
            "metadata": sorted(k.decode() for k in meta if not k.startswith(b"ARROW")),
            "schema_version": meta.get(VERSION_KEY, b"").decode(),
        }
    files["manifest.json"] = {"keys": sorted(json.loads((out / "manifest.json").read_text()))}
    with (out / "timings.csv").open() as f:
        files["timings.csv"] = {"columns": next(csv.reader(f))}
    return files


def _columns(spec: dict[str, Any]) -> list[dict[str, Any]]:
    return [c for c in spec.get("columns", []) if isinstance(c, dict)]


def test_every_output_file_matches_the_schema_file(tmp_path: Path) -> None:
    pytest.importorskip("pyarrow")
    expected = json.loads(SCHEMA.read_text())
    written = _written(_run(tmp_path))
    assert sorted(written) == sorted(expected["files"]), "the files a run writes"
    for name, spec in expected["files"].items():
        got = written[name]
        if "keys" in spec:
            assert got["keys"] == spec["keys"], f"{name}: its top-level keys"
        elif name.endswith(".csv"):
            assert got["columns"] == spec["columns"], f"{name}: its header"
        else:
            columns = [{k: c[k] for k in ("name", "type", "nullable")} for c in spec["columns"]]
            assert got["columns"] == columns, f"{name}: its columns (name, type, nullable)"
            assert got["metadata"] == spec["metadata"], f"{name}: its Parquet metadata keys"
            version = str(expected["schema_version"])
            assert got["schema_version"] == version, f"{name}: its schema version"


def test_every_column_has_a_unit() -> None:
    expected = json.loads(SCHEMA.read_text())
    missing = [
        f"{name}:{c['name']}"
        for name, spec in expected["files"].items()
        for c in _columns(spec)
        if not c.get("unit")
    ]
    assert not missing, f"columns without a unit in {SCHEMA.name}: {missing}"


def _write() -> None:
    """Rewrite the schema file from a run, keeping the units already given."""
    old = json.loads(SCHEMA.read_text()) if SCHEMA.exists() else {"files": {}}
    units = {
        (n, c["name"]): c.get("unit", "") for n, s in old["files"].items() for c in _columns(s)
    }
    with tempfile.TemporaryDirectory() as folder:
        written = _written(_run(Path(folder)))
    versions = {v.pop("schema_version") for v in written.values() if "schema_version" in v}
    assert len(versions) == 1, f"the files disagree on their schema version: {versions}"
    for name, spec in written.items():
        for c in _columns(spec):
            c["unit"] = units.get((name, c["name"]), "")
    text = json.dumps(
        {"schema_version": int(versions.pop()), "files": written}, indent=1, ensure_ascii=False
    )
    SCHEMA.write_text(text + "\n")
    blank = [f"{n}:{c['name']}" for n, s in written.items() for c in _columns(s) if not c["unit"]]
    print(f"written {SCHEMA}; units to fill: {blank or 'none'}")


if __name__ == "__main__" and sys.argv[1:] == ["--write"]:
    _write()
