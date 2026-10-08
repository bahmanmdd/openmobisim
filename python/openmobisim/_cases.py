"""The starter cases (S240): small, versioned inputs for the tutorials and the default scenarios.

A **starter case** is a network with what runs on it — a timetable, parkings, a demand and its
traveller classes — prepared once, clipped to its study area and checked: Amsterdam, Paris and Lyon
(OpenStreetMap, their transit feeds, a synthetic morning commute), Sioux Falls and Nguyen–Dupuis
(the benchmarks). **Synthetic demand, not a forecast**: for learning the tool and comparing methods,
not for studying the cities.

``examples.case(name)`` finds a case's files — in ``root`` if given, else in the folder
``OPENMOBISIM_DATA`` names, else in the cache (``~/.cache/openmobisim``), else it downloads the
case's archive once — checks every file against the checksums this package carries, and reads them:
``network()``, ``transit()``, ``parkings()``, ``demand()``, ``classes()``, or all at once,
``scenario()``. Each case says how it is read in its ``case.json``.

``examples.case_check()`` runs the cases' default scenarios and compares them with the reference
results this version carries (the acceptance test of an installation; ``python -m openmobisim
check`` on the command line).
"""

from __future__ import annotations

import hashlib
import json
import os
import shutil
import tempfile
import urllib.request
import zipfile
from pathlib import Path
from typing import Any

from openmobisim import _core

#: The bundle's version: a case's files never change within one.
BUNDLE_VERSION = "v1"
#: Where the cases' archives (``<case>.zip``) are published, tried in order. Empty until they are.
SOURCES: tuple[str, ...] = ()

_MANIFEST = Path(__file__).with_name("data") / f"bundle_{BUNDLE_VERSION}.json"


def _region(given: list) -> Any:
    """A ``case.json`` region as the readers take it: a box tuple, or a polygon of points."""
    if len(given) == 4 and not isinstance(given[0], list):
        return tuple(given)
    return [tuple(p) for p in given]


def _manifest() -> dict[str, Any]:
    return json.loads(_MANIFEST.read_text(encoding="utf-8"))


def case_names() -> list[str]:
    """The starter cases this version of the package knows."""
    return sorted(_manifest()["cases"])


def _sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def _cache() -> Path:
    base = os.environ.get("XDG_CACHE_HOME") or str(Path.home() / ".cache")
    return Path(base) / "openmobisim" / "bundle" / BUNDLE_VERSION


def _download(name: str, entry: dict[str, Any], target: Path) -> None:
    """The case's archive from the first source that has it, checked, unpacked into ``target``."""
    if not SOURCES:
        raise FileNotFoundError(
            f"case {name!r}: not found locally, and this version knows no place to download it "
            f"from; give root= (or set OPENMOBISIM_DATA) to a folder holding the bundle's {name}/"
        )
    archive = entry["archive"]
    errors = []
    for source in SOURCES:
        url = f"{source.rstrip('/')}/{archive['name']}"
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / archive["name"]
            try:
                urllib.request.urlretrieve(url, path)  # noqa: S310 (a fixed https source)
            except OSError as e:
                errors.append(f"{url}: {e}")
                continue
            if _sha256(path) != archive["sha256"]:
                errors.append(f"{url}: the archive's checksum differs")
                continue
            target.parent.mkdir(parents=True, exist_ok=True)
            partial = target.with_name(target.name + ".partial")
            shutil.rmtree(partial, ignore_errors=True)
            with zipfile.ZipFile(path) as z:
                z.extractall(partial)
            partial.replace(target)
            return
    raise FileNotFoundError(f"case {name!r}: could not download it: " + "; ".join(errors))


class Case:
    """One starter case, its files checked: see :func:`openmobisim.examples.case`."""

    def __init__(self, name: str, folder: Path) -> None:
        self.name = name
        self.folder = folder
        self.info: dict[str, Any] = json.loads((folder / "case.json").read_text(encoding="utf-8"))

    def __repr__(self) -> str:
        return f"Case({self.name!r}, {self.info.get('title', '')!r}, {str(self.folder)!r})"

    def path(self, relative: str) -> Path:
        """A file of the case."""
        return self.folder / relative

    def network(self) -> _core.Network:
        """The road network, with its bike and walk layers."""
        n = self.info["network"]
        if "pbf" in n:
            return _core.network_read_osm(str(self.path(n["pbf"])), region=_region(n["region"]))
        from openmobisim.network import network_read_table

        return network_read_table(
            str(self.path(n["links"])), str(self.path(n["nodes"])), **n.get("read", {})
        )

    def transit(self, network: _core.Network | None = None) -> _core.Transit | None:
        """The timetable of the case's service day, its stops reached on ``network``'s walks."""
        t = self.info.get("transit")
        if not t:
            return None
        return _core.transit_read_gtfs(
            str(self.path(t["gtfs"])),
            network=network if network is not None else self.network(),
            date=t["date"],
        )

    def parkings(self) -> _core.Parkings | None:
        """The parkings of the study area, from OpenStreetMap."""
        n = self.info["network"]
        if not self.info.get("parkings") or "pbf" not in n:
            return None
        return _core.parking_read_osm(str(self.path(n["pbf"])), region=_region(n["region"]))

    def zones(self) -> dict[str, tuple[float, float]]:
        """The demand's zones: each its point."""
        from openmobisim.demand import demand_read_zones

        return demand_read_zones(self.path(self.info["demand"]["zones"]))

    def classes(self) -> dict[str, dict]:
        """The traveller classes, with their shares."""
        from openmobisim.demand import demand_read_classes

        return demand_read_classes(self.path(self.info["demand"]["classes"]))

    def demand(self, multiple: float = 1.0, seed: int = 0) -> list[tuple]:
        """The trips: the OD matrix times ``multiple``, each traveller given a class by share."""
        from openmobisim.demand import demand_assign_classes, demand_from_od, demand_read_od

        d = self.info["demand"]
        od = demand_read_od(self.path(d["od"]))
        if multiple != 1.0:
            od = [{**r, "trips": float(r["trips"]) * multiple} for r in od]
        trips = demand_from_od(od, self.zones(), seed=seed, **d.get("from_od", {}))
        return demand_assign_classes(trips, self.classes(), seed=seed)

    def scenario(self, scenario: str = "base", seed: int = 0, **options: Any) -> Any:
        """The case's scenario ``scenario`` (its ``case.json``), with ``options`` added."""
        from openmobisim.scenario import Scenario

        settings = dict(self.info.get("scenarios", {}).get(scenario, {}))
        if scenario not in self.info.get("scenarios", {"base": {}}):
            raise ValueError(
                f"case {self.name!r} has no scenario {scenario!r}; it has "
                f"{sorted(self.info.get('scenarios', {}))}"
            )
        multiple = float(settings.pop("multiple", 1.0))
        network = self.network()
        transit = self.transit(network)
        parts: dict[str, Any] = {
            "classes": self.classes(),
            "transit": transit,
            **settings,
            **options,
        }
        if transit is not None:
            parts.setdefault("parkings", self.parkings())
        return Scenario.from_parts(network, self.demand(multiple, seed), **parts)


def case(name: str, root: str | Path | None = None, *, check: bool = True) -> Case:
    """A starter case (S240): its files found or fetched, checked, ready to read.

    The cases: ``amsterdam``, ``paris``, ``lyon`` (OpenStreetMap clipped to the study area, the
    transit feed of one service day, the parkings, a synthetic morning commute 07:00–09:00 with six
    traveller classes), ``siouxfalls`` and ``nguyendupuis`` (the benchmarks: link tables, an OD
    matrix, cars). ``case_names()`` lists them.

    Args:
        name: The case.
        root: A folder holding the bundle (``<root>/<name>/``); else the folder
            ``OPENMOBISIM_DATA`` names, else the cache (``~/.cache/openmobisim/bundle/<version>``),
            where a case is downloaded once.
        check: Check every file against the checksums this package carries (a changed or broken
            file is refused).

    Returns:
        The case: ``network()``, ``transit()``, ``parkings()``, ``zones()``, ``classes()``,
        ``demand()``, ``scenario()``, and ``info`` (its ``case.json``: sources, licences, how it was
        made).

    Raises:
        ValueError: For an unknown case, or a file that fails its check.
        FileNotFoundError: If the case is nowhere to be found and cannot be downloaded.
    """
    cases = _manifest()["cases"]
    if name not in cases:
        raise ValueError(f"no starter case {name!r}; the cases: {sorted(cases)}")
    entry = cases[name]
    if root is not None:  # a folder given is the only place looked in
        candidates = [Path(root)]
    else:
        data = os.environ.get("OPENMOBISIM_DATA")
        candidates = ([Path(data)] if data else []) + [_cache()]
    folder = next((c / name for c in candidates if (c / name / "case.json").exists()), None)
    if folder is None:
        if root is not None:
            raise FileNotFoundError(f"case {name!r}: no {Path(root) / name / 'case.json'}")
        folder = _cache() / name
        _download(name, entry, folder)
    if check:
        for relative, digest in entry["files"].items():
            path = folder / relative
            if not path.exists():
                raise ValueError(f"case {name!r}: {relative} is missing from {folder}")
            if _sha256(path) != digest:
                raise ValueError(
                    f"case {name!r}: {relative} in {folder} is not the bundle's ({BUNDLE_VERSION})"
                )
    return Case(name, folder)


_REFERENCE = Path(__file__).with_name("data") / f"reference_{BUNDLE_VERSION}.json"

#: How far a run on another platform may be from the reference and still match (S243): trips
#: completed, in % of the reference's; the mean trip, in %; each mode's share of the people, in
#: percentage points. Judgement calls: floating-point results are the same to the bit only on the
#: platform that made the reference (Foundations §3.5).
CHECK_TOLERANCE = {"completed_pct": 0.5, "mean_trip_pct": 1.0, "mode_share_points": 0.5}


def case_summary(run: Any) -> dict[str, Any]:
    """What a starter case's run is compared on: its fingerprint, trips, mean trip and modes.

    The mode shares are of people (traveller weights), ``"none"`` for trips with nothing to
    choose from; ``platform`` is the build's target, ``version`` the package's.
    """
    modes = run.trip_modes()
    people: dict[str, float] = {}
    for mode, weight in zip(modes["mode"], modes["weight"], strict=True):
        people[mode or "none"] = people.get(mode or "none", 0.0) + float(weight)
    total = sum(people.values()) or 1.0
    info = _core.build_info()
    return {
        "fingerprint": run.fingerprint,
        "trips": int(run.completion["total_trips"]),
        "completed": int(run.completion["completed"]),
        "mean_trip_s": float(run.mean_travel_time_s),
        "mode_share_pct": {m: 100.0 * p / total for m, p in sorted(people.items())},
        "loadings": len(run.convergence()["gap"]),
        "platform": info["target"],
        "version": info["version"],
    }


def _compare(got: dict[str, Any], ref: dict[str, Any]) -> tuple[str, list[str]]:
    """The verdict and what differed.

    ``"same"``, ``"same numbers (another platform)"``, ``"within tolerance"`` or ``"different"``.
    """
    notes = []
    if got["fingerprint"] != ref["fingerprint"]:
        notes.append(f"fingerprint {got['fingerprint']} against {ref['fingerprint']}: other inputs")
    exact = got["platform"] == ref["platform"] and got["version"] == ref["version"]
    shares = set(got["mode_share_pct"]) | set(ref["mode_share_pct"])
    if exact and not notes:
        same = (
            got["completed"] == ref["completed"]
            and got["mean_trip_s"] == ref["mean_trip_s"]
            and all(got["mode_share_pct"].get(m) == ref["mode_share_pct"].get(m) for m in shares)
        )
        if same:
            return "same", []
        # The reference's own platform and version must repeat it to the bit: anything else is
        # a fault, however close the numbers.
        return "different", ["not the same to the bit on the reference's platform"]
    tol = CHECK_TOLERANCE
    off_completed = 100 * abs(got["completed"] - ref["completed"]) / max(ref["completed"], 1)
    off_mean = 100 * abs(got["mean_trip_s"] - ref["mean_trip_s"]) / max(ref["mean_trip_s"], 1e-9)
    got_s, ref_s = got["mode_share_pct"], ref["mode_share_pct"]
    off_share = max((abs(got_s.get(m, 0.0) - ref_s.get(m, 0.0)) for m in shares), default=0.0)
    if off_completed > tol["completed_pct"]:
        notes.append(f"completed {got['completed']} against {ref['completed']}")
    if off_mean > tol["mean_trip_pct"]:
        notes.append(f"mean trip {got['mean_trip_s']:.1f} s against {ref['mean_trip_s']:.1f} s")
    if off_share > tol["mode_share_points"]:
        notes.append(f"a mode's share differs by {off_share:.2f} points")
    if notes:
        return "different", notes
    identical = (
        got["completed"] == ref["completed"]
        and got["mean_trip_s"] == ref["mean_trip_s"]
        and all(got_s.get(m) == ref_s.get(m) for m in shares)
    )
    return ("same numbers (another platform)" if identical else "within tolerance"), notes


def case_check(
    names: list[str] | None = None,
    root: str | Path | None = None,
    *,
    quiet: bool = False,
) -> list[dict[str, Any]]:
    """Run starter cases' default scenarios and compare them with the reference results (S243).

    The acceptance test of an installation: every scenario of each case (its ``case.json``) is run
    with the package's defaults, and compared with the results this version of the package
    carries. **Same**: the same fingerprint and, on the platform that made the reference, the same
    results to the bit. **Same numbers (another platform)**: the same fingerprint and, on another
    platform, every compared number the same to the bit. **Within tolerance**: the same
    fingerprint, and on another platform results within ``CHECK_TOLERANCE``. **Different**:
    anything else (another fingerprint means other inputs or settings: another bundle or package
    version).

    Args:
        names: The cases (``case_names()``); ``None``: every case.
        root: Where the bundle is, as for :func:`case`.
        quiet: If false, print a line per scenario as it finishes.

    Returns:
        One row per scenario: ``case``, ``scenario``, ``status`` (``"same"``,
        ``"same numbers (another platform)"``, ``"within tolerance"``, ``"different"`` or
        ``"no reference"``), ``notes`` (what differed),
        ``run_s`` (wall-clock seconds, reading included) and ``result`` (``case_summary``).
    """
    import time

    reference = json.loads(_REFERENCE.read_text(encoding="utf-8")) if _REFERENCE.exists() else {}
    rows = []
    for name in names or case_names():
        c = case(name, root)
        for scenario in c.info.get("scenarios", {"base": {}}):
            t = time.perf_counter()
            run = c.scenario(scenario).run(f"check-{name}-{scenario}", quiet=True)
            got = case_summary(run)
            ref = reference.get("cases", {}).get(f"{name}/{scenario}")
            status, notes = ("no reference", []) if ref is None else _compare(got, ref)
            row = {"case": name, "scenario": scenario, "status": status, "notes": notes,
                   "run_s": time.perf_counter() - t, "result": got}  # fmt: skip
            rows.append(row)
            if not quiet:
                print(f"{name:14s} {scenario:10s} {status:31s} {row['run_s']:7.1f} s  "
                      + "; ".join(notes), flush=True)  # fmt: skip
    return rows
