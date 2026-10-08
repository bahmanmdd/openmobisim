"""Starter cases (S240): found, checked and read; refused when changed or nowhere to be found.

The loader's logic runs everywhere on a case made up here (two links, two zones). With the bundle at
hand (``OPENMOBISIM_TEST_BUNDLE``, a folder holding ``<case>/``, as ``OPENMOBISIM_TEST_PBF`` holds
an extract), the real cases are read and the benchmarks' reference results checked: Sioux Falls at
base 11.72 min a trip, congested 20.86 min (S225, S229); Nguyen–Dupuis 53.43 min.
"""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path

import openmobisim as ms
import pytest
from openmobisim import _cases

BUNDLE = os.environ.get("OPENMOBISIM_TEST_BUNDLE")


def _sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


@pytest.fixture
def made_up(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    """A case ``line``: A → B → C, two links of 1 km; 100 trips A to C; a manifest naming it."""
    root = tmp_path / "bundle"
    folder = root / "line"
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
        "demand": {
            "od": "demand/od.csv", "zones": "demand/zones.csv", "classes": "demand/classes.csv",
            "from_od": {"start_s": 25200, "end_s": 28800, "profile": "uniform", "spread_m": 0.0},
        },
        "scenarios": {"base": {}, "double": {"multiple": 2.0}},
    }))  # fmt: skip
    paths = sorted(p for p in folder.rglob("*") if p.is_file())
    files = {p.relative_to(folder).as_posix(): _sha(p) for p in paths}  # "/" on every system
    manifest = tmp_path / "bundle_v1.json"
    manifest.write_text(json.dumps({"bundle": "v1", "cases": {"line": {
        "archive": {"name": "line.zip", "sha256": "0" * 64, "bytes": 0},
        "files": files}}}))  # fmt: skip
    monkeypatch.setattr(_cases, "_MANIFEST", manifest)
    monkeypatch.setenv("XDG_CACHE_HOME", str(tmp_path / "cache"))
    monkeypatch.delenv("OPENMOBISIM_DATA", raising=False)
    return root


def test_a_case_is_found_checked_and_read(made_up: Path) -> None:
    assert ms.examples.case_names() == ["line"]
    case = ms.examples.case("line", root=made_up)
    assert case.network().link_count == 2
    trips = case.demand()
    assert len(trips) == 100 and {t[7] for t in trips} == {"default"}
    assert len(case.demand(multiple=2.0)) == 200
    run = case.scenario(equilibration="free_flow").run("case-line", quiet=True)
    assert run.completion["completed"] == 100
    double = case.scenario("double", equilibration="free_flow").run("case-line-2", quiet=True)
    assert double.completion["total_trips"] == 200
    with pytest.raises(ValueError, match="no scenario 'busy'"):
        case.scenario("busy")


def test_the_data_folder_variable_is_read_and_a_changed_file_refused(
    made_up: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("OPENMOBISIM_DATA", str(made_up))
    assert ms.examples.case("line").folder == made_up / "line"
    od = made_up / "line" / "demand" / "od.csv"
    od.write_text(od.read_text().replace("100", "101"))
    with pytest.raises(ValueError, match="demand/od.csv .* is not the bundle's"):
        ms.examples.case("line")
    assert ms.examples.case("line", check=False).demand()[0][0], "unchecked, on request"


def test_an_unknown_or_missing_case_is_refused(made_up: Path, tmp_path: Path) -> None:
    with pytest.raises(ValueError, match="no starter case 'atlantis'"):
        ms.examples.case("atlantis")
    with pytest.raises(FileNotFoundError, match="no place to download it from"):
        ms.examples.case("line")  # not in the cache, no root, no folder variable
    with pytest.raises(FileNotFoundError, match="no .*case.json"):
        ms.examples.case("line", root=tmp_path / "elsewhere")


def test_a_check_compares_with_the_reference_and_the_command_line_runs_it(
    made_up: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture
) -> None:
    from openmobisim.__main__ import main

    monkeypatch.setattr(_cases, "_REFERENCE", tmp_path / "reference_v1.json")
    rows = ms.examples.case_check(root=made_up, quiet=True)
    assert [(r["scenario"], r["status"]) for r in rows] == [
        ("base", "no reference"), ("double", "no reference"),
    ]  # fmt: skip
    got = rows[0]["result"]
    assert got["completed"] == 100 and got["mode_share_pct"] == {"car": 100.0}
    reference = {"cases": {f"line/{r['scenario']}": r["result"] for r in rows}}
    (tmp_path / "reference_v1.json").write_text(json.dumps(reference))
    assert {r["status"] for r in ms.examples.case_check(root=made_up, quiet=True)} == {"same"}
    # Another platform's run: the same within the tolerance, or not; other inputs: never.
    elsewhere = {**got, "platform": "elsewhere"}
    assert _cases._compare(got, elsewhere)[0] == "same numbers (another platform)"
    near = {**got, "mean_trip_s": got["mean_trip_s"] * 1.001}
    assert _cases._compare(near, elsewhere)[0] == "within tolerance"
    assert _cases._compare(got, {**elsewhere, "mean_trip_s": got["mean_trip_s"] * 1.05})[0] == (
        "different"
    )
    assert _cases._compare(got, {**got, "fingerprint": "0" * 16})[0] == "different"
    # The command line: a check passes, then fails against a changed reference; a run prints.
    assert main(["check", "--root", str(made_up)]) == 0
    reference["cases"]["line/base"]["completed"] = 90
    (tmp_path / "reference_v1.json").write_text(json.dumps(reference))
    assert main(["check", "line", "--root", str(made_up)]) == 1
    assert main(["run", "line", "--root", str(made_up)]) == 0
    assert f"fingerprint {got['fingerprint']}" in capsys.readouterr().out


# --- the real bundle ------------------------------------------------------------------------------

needs_bundle = pytest.mark.skipif(
    BUNDLE is None, reason="set OPENMOBISIM_TEST_BUNDLE to the bundle's folder"
)


def test_the_package_s_references_are_of_its_cases() -> None:
    reference = json.loads(_cases._REFERENCE.read_text(encoding="utf-8"))
    assert reference["bundle"] == _cases.BUNDLE_VERSION
    assert {key.split("/")[0] for key in reference["cases"]} <= set(ms.examples.case_names())
    for summary in reference["cases"].values():
        assert set(summary) == {"fingerprint", "trips", "completed", "mean_trip_s",
                                "mode_share_pct", "loadings", "platform", "version"}  # fmt: skip


@needs_bundle
def test_the_benchmarks_match_the_package_s_references() -> None:
    rows = ms.examples.case_check(["nguyendupuis", "siouxfalls"], root=BUNDLE, quiet=True)
    assert len(rows) == 3
    matching = {"same", "same numbers (another platform)", "within tolerance"}
    assert {r["status"] for r in rows} <= matching, rows


@needs_bundle
@pytest.mark.parametrize(
    ("scenario", "mean_min"), [("base", 11.72), ("congested", 20.86)]
)  # fmt: skip
def test_sioux_falls_gives_its_reference_results(scenario: str, mean_min: float) -> None:
    run = (
        ms.examples.case("siouxfalls", root=BUNDLE)
        .scenario(scenario)
        .run("sf-" + scenario, quiet=True)
    )
    assert run.completion["completed"] == run.completion["total_trips"]
    assert run.mean_travel_time_s / 60 == pytest.approx(mean_min, abs=0.01)


@needs_bundle
def test_nguyen_dupuis_gives_its_reference_result() -> None:
    run = ms.examples.case("nguyendupuis", root=BUNDLE).scenario().run("nd", quiet=True)
    assert run.completion["completed"] == 2000
    assert run.mean_travel_time_s / 60 == pytest.approx(53.43, abs=0.01)


@needs_bundle
@pytest.mark.parametrize("name", ["amsterdam", "paris", "lyon"])
def test_a_city_reads_with_its_timetable_parkings_and_classes(name: str) -> None:
    if name not in ms.examples.case_names():
        pytest.skip(f"{name} is not in this bundle yet")
    case = ms.examples.case(name, root=BUNDLE)
    net = case.network()
    transit = case.transit(net)
    assert net.link_count > 10_000 and transit is not None and transit.run_count > 1000
    assert case.parkings() is not None
    assert set(case.classes()) >= {"bike_enthusiast", "car_captive"}
    assert transit.date == case.info["transit"]["date"]


@needs_bundle
def test_a_case_is_downloaded_once_checked_and_unpacked(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # The bundle's archives served from a folder, as they will be from a release.
    monkeypatch.setattr(_cases, "SOURCES", (f"file://{tmp_path / 'nowhere'}", f"file://{BUNDLE}"))
    monkeypatch.setenv("XDG_CACHE_HOME", str(tmp_path / "cache"))
    monkeypatch.delenv("OPENMOBISIM_DATA", raising=False)
    case = ms.examples.case("nguyendupuis")
    assert case.folder == tmp_path / "cache" / "openmobisim" / "bundle" / "v1" / "nguyendupuis"
    assert case.network().link_count == 19
    again = ms.examples.case("nguyendupuis")
    assert again.folder == case.folder, "found in the cache, not downloaded again"
