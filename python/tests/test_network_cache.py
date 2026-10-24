"""The network cache (S294, I-as): what is written is read back the same, and runs the same.

A network written and read back is the same network, bit for bit, and a run on it has the same
fingerprint and the same results; a case reads its network from the cache after the first import,
imports again when the file is not its own, and caches nothing with
``OPENMOBISIM_NO_NETWORK_CACHE=1``.

The logic runs everywhere on the toy network, a grid and a case made up here; with the bundle at
hand (``OPENMOBISIM_DATA`` or ``OPENMOBISIM_TEST_BUNDLE``), on Delft as well.
"""

from __future__ import annotations

import hashlib
import json
import logging
import os
from pathlib import Path

import numpy as np
import openmobisim as ms
import pytest
from openmobisim import _cases, _core, _network_cache

BUNDLE = os.environ.get("OPENMOBISIM_TEST_BUNDLE") or os.environ.get("OPENMOBISIM_DATA")

#: Every per-link and per-node array a network handle gives.
ARRAYS = (
    "link_length_m",
    "link_free_flow_s",
    "link_speed_km_h",
    "link_class",
    "link_lanes",
    "link_capacity_pcu_h",
    "link_storage_pcu",
    "link_drivable",
    "link_roundabout",
    "link_signalised",
    "node_signalised",
    "link_from",
    "link_to",
)


def same_bytes(a: object, b: object) -> bool:
    x, y = np.asarray(a), np.asarray(b)
    return x.dtype == y.dtype and x.shape == y.shape and x.tobytes() == y.tobytes()


def assert_same_network(a: ms._core.Network, b: ms._core.Network) -> None:
    """Every array, id, shape, report and parameter of two road networks and their layers."""
    assert (a.node_count, a.link_count, a.source, repr(a)) == (
        b.node_count,
        b.link_count,
        b.source,
        repr(b),
    )
    assert a.link_ids() == b.link_ids()
    for name in ARRAYS:
        assert same_bytes(getattr(a, name)(), getattr(b, name)()), name
    assert all(same_bytes(x, y) for x, y in zip(a.link_geometry(), b.link_geometry(), strict=True))
    assert a.report_import() == b.report_import()
    dropped_a, dropped_b = a.report_dropped(), b.report_dropped()
    assert (dropped_a is None) == (dropped_b is None)
    if dropped_a is not None:
        assert all(same_bytes(x, y) for x, y in zip(dropped_a, dropped_b, strict=True))
    assert ms.network_options(a) == ms.network_options(b)
    assert a.report_connectivity() == b.report_connectivity()
    for name in ("bike", "walk"):
        la, lb = a.layer(name), b.layer(name)
        assert la.link_ids() == lb.link_ids(), name
        for array in ("link_length_m", "link_free_flow_s", "link_speed_km_h", "link_class"):
            assert same_bytes(getattr(la, array)(), getattr(lb, array)()), (name, array)
        assert same_bytes(la.link_from(), lb.link_from()) and same_bytes(la.link_to(), lb.link_to())
        assert same_bytes(la.link_infrastructure(), lb.link_infrastructure()), name
        assert all(
            same_bytes(x, y) for x, y in zip(la.link_geometry(), lb.link_geometry(), strict=True)
        )


def round_trip(network: ms._core.Network, path: Path) -> ms._core.Network:
    ms.network_write_cache(network, path)
    back = ms.network_read_cache(path)
    assert back.read_s is not None and back.read_s >= 0.0
    assert_same_network(network, back)
    return back


def assert_same_run(a: ms.Run, b: ms.Run) -> None:
    """The same fingerprint and, bit for bit, the same trips, convergence, link bins and KPIs."""
    assert a.fingerprint == b.fingerprint
    for got, want in ((a.trip_results(), b.trip_results()), (a.convergence(), b.convergence())):
        assert list(got) == list(want)
        for column in got:
            x, y = np.asarray(got[column]), np.asarray(want[column])
            if x.dtype.kind in "fiub":
                assert same_bytes(x, y), column
            else:
                assert list(got[column]) == list(want[column]), column
    bins_a, bins_b = a.link_bins(), b.link_bins()
    assert (bins_a is None) == (bins_b is None)
    for part in ("bins", "links", "crossings", "pcu", "pcu_seconds") if bins_a else ():
        assert same_bytes(getattr(bins_a, part)(), getattr(bins_b, part)()), part
    kpis_a, kpis_b = a.kpis().to_pandas(), b.kpis().to_pandas()
    assert kpis_a.drop(columns="run_id").equals(kpis_b.drop(columns="run_id"))


def toy_run(network: ms._core.Network, run_id: str) -> ms.Run:
    """Cars, bikes and walkers across the toy network, the default equilibration."""
    w, e = network.node_lonlat("W"), network.node_lonlat("D1")
    rows = [
        (f"t{i}", 0, *w, *e, 8 * 3600 + 20 * i, "everyone", None, mode)
        for i, mode in enumerate(["car"] * 40 + ["bike"] * 5 + ["walk"] * 5)
    ]
    scenario = ms.Scenario.from_parts(
        network=network, demand=rows, classes={"everyone": (True, True, False)}, link_bin_s=300
    )
    return scenario.run(run_id=run_id, quiet=True)


def test_the_toy_network_comes_back_the_same_and_runs_the_same(tmp_path: Path) -> None:
    toy = ms.examples.toy_network()
    back = round_trip(toy, tmp_path / "toy.network")
    assert_same_run(toy_run(toy, "from-import"), toy_run(back, "from-cache"))


def test_a_grid_an_edited_network_and_a_table_network_come_back_the_same(tmp_path: Path) -> None:
    grid = ms.examples.manhattan_grid(5, 200.0, True)
    round_trip(grid, tmp_path / "grid.network")
    grid.layer("bike")  # the layers derived now are cached with it
    round_trip(grid, tmp_path / "grid-layers.network")
    edited = ms.network_edit(grid, close=[0, 3], lanes={5: 2}, capacity_factor={7: 0.5})
    back = round_trip(edited, tmp_path / "edited.network")
    assert not back.link_drivable()[0] and grid.link_drivable()[0]
    table = ms.network_read_table(
        [{"from": "A", "to": "B", "length": 1000, "capacity": 1800, "lanes": 1}],
        [{"id": "A", "lon": 4.90, "lat": 52.37}, {"id": "B", "lon": 4.915, "lat": 52.37}],
        network_options={"capacity_factor": 0.9},
    )
    round_trip(table, tmp_path / "table.network")


def test_a_layer_handle_is_refused_and_a_foreign_file_names_what_is_wrong(tmp_path: Path) -> None:
    toy = ms.examples.toy_network()
    with pytest.raises(ValueError, match="write the road network's handle"):
        ms.network_write_cache(toy.layer("bike"), tmp_path / "bike.network")
    path = tmp_path / "toy.network"
    ms.network_write_cache(toy, path, source="made from these")
    assert _core.network_cache_header(path)["source"] == "made from these"
    ms.network_read_cache(path, source="made from these")
    with pytest.raises(ValueError, match="another source"):
        ms.network_read_cache(path, source="made from others")
    data = bytearray(path.read_bytes())
    data[len(data) // 2] ^= 1
    path.write_bytes(bytes(data))
    with pytest.raises(ValueError, match="damaged"):
        ms.network_read_cache(path)
    (tmp_path / "not.network").write_bytes(b"PAR1 not a network")
    with pytest.raises(ValueError, match="not a network file"):
        ms.network_read_cache(tmp_path / "not.network")
    with pytest.raises(OSError):
        ms.network_read_cache(tmp_path / "missing.network")


# --- a case's network ----------------------------------------------------------------------------


@pytest.fixture
def line(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    """A case ``line`` (A → B → C, two 1 km links, 100 trips), its manifest, a cache of its own."""
    folder = tmp_path / "bundle" / "line"
    (folder / "network").mkdir(parents=True)
    (folder / "demand").mkdir()
    (folder / "network" / "links.csv").write_text(
        "from,to,length,free_flow_time,capacity,lanes\nA,B,1000,1,1800,1\nB,C,1000,1,1800,1\n"
    )
    (folder / "network" / "nodes.csv").write_text(
        "id,lon,lat\nA,4.90,52.37\nB,4.915,52.37\nC,4.93,52.37\n"
    )
    (folder / "demand" / "zones.csv").write_text("zone,lon,lat\nA,4.90,52.37\nC,4.93,52.37\n")
    (folder / "demand" / "od.csv").write_text("origin,destination,trips\nA,C,100\n")
    (folder / "demand" / "classes.csv").write_text("class,share,modes\ndefault,1,car\n")
    (folder / "case.json").write_text(json.dumps({
        "case": "line", "title": "A line", "kind": "table",
        "network": {"links": "network/links.csv", "nodes": "network/nodes.csv",
                    "read": {"length_unit": "m", "time_unit": "min"}},
        "transit": None, "parkings": False,
        "demand": {"od": "demand/od.csv", "zones": "demand/zones.csv",
                   "classes": "demand/classes.csv",
                   "from_od": {"start_s": 25200, "end_s": 28800, "profile": "uniform",
                               "spread_m": 0.0}},
        "scenarios": {"base": {}},
    }))  # fmt: skip
    files = {
        p.relative_to(folder).as_posix(): hashlib.sha256(p.read_bytes()).hexdigest()
        for p in sorted(folder.rglob("*"))
        if p.is_file()
    }
    manifest = tmp_path / "bundle_v1.json"
    manifest.write_text(json.dumps({"bundle": "v1", "cases": {"line": {
        "archive": {"name": "line.zip", "sha256": "0" * 64, "bytes": 0},
        "files": files}}}))  # fmt: skip
    monkeypatch.setattr(_cases, "_MANIFEST", manifest)
    monkeypatch.setenv("XDG_CACHE_HOME", str(tmp_path / "cache"))
    monkeypatch.delenv(_network_cache.NO_CACHE_VARIABLE, raising=False)
    monkeypatch.delenv("OPENMOBISIM_DATA", raising=False)
    return tmp_path


def cached_files(root: Path) -> list[Path]:
    return sorted((root / "cache" / "openmobisim" / "networks" / "line").glob("*.network"))


def test_a_case_imports_its_network_once_then_reads_it_from_the_cache(
    line: Path, caplog: pytest.LogCaptureFixture, monkeypatch: pytest.MonkeyPatch
) -> None:
    case = ms.examples.case("line", root=line / "bundle")
    with caplog.at_level(logging.INFO, logger="openmobisim"):
        first = case.network()
    assert "network imported" in caplog.text and "cached for the next time" in caplog.text
    (path,) = cached_files(line)
    caplog.clear()
    with caplog.at_level(logging.INFO, logger="openmobisim"):
        again = ms.examples.case("line", root=line / "bundle").network()
    assert "network read from the cache" in caplog.text and str(path) in caplog.text
    assert_same_network(first, again)
    cached = case.scenario(equilibration="free_flow").run("line-cache", quiet=True)
    monkeypatch.setenv(_network_cache.NO_CACHE_VARIABLE, "1")
    imported = case.scenario(equilibration="free_flow").run("line-import", quiet=True)
    assert_same_run(imported, cached)


def test_a_file_of_other_data_or_a_damaged_one_is_imported_again(line: Path) -> None:
    case = ms.examples.case("line", root=line / "bundle")
    case.network()
    (path,) = cached_files(line)
    # The same name, another source in its header (other data, another state of the code).
    ms.network_write_cache(case.network(), path, source="another source")
    case.network()
    assert _core.network_cache_header(path)["source"] != "another source", "replaced"
    path.write_bytes(path.read_bytes()[:-5])  # cut short
    network = case.network()
    assert network.link_count == 2
    ms.network_read_cache(path)  # written anew, whole
    # A changed link table is other data: another file, the old one kept among the last three.
    links = line / "bundle" / "line" / "network" / "links.csv"
    links.write_text(links.read_text().replace("1000,1,1800", "2000,2,1800"))
    changed = ms.examples.case("line", root=line / "bundle", check=False).network()
    assert len(cached_files(line)) == 2
    assert changed.link_length_m()[0] == 2000.0


def test_a_file_another_build_wrote_is_imported_again(
    line: Path, caplog: pytest.LogCaptureFixture, monkeypatch: pytest.MonkeyPatch
) -> None:
    case = ms.examples.case("line", root=line / "bundle")
    case.network()
    real = _core.network_cache_header
    # What the header says of a file of another version or build of openmobisim.
    monkeypatch.setattr(_core, "network_cache_header", lambda p: {**real(p), "current": False})
    with caplog.at_level(logging.INFO, logger="openmobisim"):
        case.network()
    assert "network imported" in caplog.text and "read from the cache" not in caplog.text


def test_the_variable_turns_the_cache_off(line: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv(_network_cache.NO_CACHE_VARIABLE, "1")
    case = ms.examples.case("line", root=line / "bundle")
    case.network()
    case.network()
    assert cached_files(line) == []
    monkeypatch.setenv(_network_cache.NO_CACHE_VARIABLE, "0")  # "0" is on
    case.network()
    assert len(cached_files(line)) == 1


# --- Delft ---------------------------------------------------------------------------------------


@pytest.mark.skipif(BUNDLE is None, reason="set OPENMOBISIM_DATA to the bundle's folder")
def test_delft_s_network_comes_back_the_same_and_runs_the_same(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv(_network_cache.NO_CACHE_VARIABLE, "1")
    case = ms.examples.case("delft", root=BUNDLE)
    imported = case.network()
    back = round_trip(imported, tmp_path / "delft.network")
    assert back.read_s < imported.read_s
    # A small day on both: cars, bikes and walkers between the case's zones.
    zones = list(case.zones().values())
    rows = []
    for i in range(120):
        o, d = zones[i % len(zones)], zones[(7 * i + 3) % len(zones)]
        mode = ("car", "car", "bike", "walk")[i % 4]
        rows.append((f"t{i}", 0, *o, *d, 8 * 3600 + 10 * i, "everyone", None, mode))
    runs = [
        ms.Scenario.from_parts(
            network=net, demand=rows, classes={"everyone": (True, True, False)}, link_bin_s=900
        ).run(run_id=f"delft-{name}", quiet=True)
        for name, net in (("import", imported), ("cache", back))
    ]
    assert_same_run(*runs)
