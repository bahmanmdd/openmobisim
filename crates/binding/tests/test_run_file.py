"""The settings file as an input: run_file(folder, out=folder) runs from run.toml alone, its
inputs read from the files it names (paths relative to its folder). A finished run is run again
from its record's part one, every line before [result], written as run.toml beside the record
(`settings` below). The answers come from outside the simulator: Python's os.path and the file system (where a recorded path
leads), PARAMETERS.md (the defaults, parsed), hand numbers (test_defaults.py's derivations) and
the bytes of a first run of the same inputs and values (a rerun's byte equality, the one
self-comparison allowed).

1. A recorded path joined to the record's folder by os.path.join is the file given
   (os.path.samefile), each path relative with "/" between components: inputs given by absolute
   path, by relative path, through a symbolic link, out at another depth, and out reached
   through a symbolic link to a folder at another depth; no file key for a table given in
   memory. On Windows, which removes a ".." before it follows a link, the two symbolic-link
   parts check what the tool does instead of the text join: each path relative with "/", the
   run again by run_file from its settings (the tables the first run's bytes) and opened by
   Run.open; for
   a user who may not create a symbolic link (OSError) they are skipped, with the reason.
2. A run from files with transit (test_record.py's case of a 6 x 6 grid in longitude/latitude,
   two bus lines, 120 trips) and a non-default value for every row of PARAMETERS.md a record
   holds, its settings fed to run_file into a sibling folder: trips.parquet, links.parquet and
   run_record.toml byte-identical, iterations.parquet equal but for its five wall-clock columns. The same after
   the whole tree (inputs and folders) is moved. Into a folder at another depth: the tables
   identical, the record differing only in its file lines.
3. The same on Delft's OSM extract and feed (one walk-and-transit trip and one bike trip, one
   iteration; about a second in release, so not gated).
4. A hand-written settings file holding only the window, the files and the classes runs test_defaults.py's
   case 4 (stop_walk_max 1 500 m): "near" walks 1 499 m to its stop, rides 08:30 to 08:40 and
   walks 100 m: 2 475 s; "far" (1 501 m) has no journey; the record written has every parameter
   at the default parsed from PARAMETERS.md.
5. A record renamed run.toml: one UserWarning saying so; with one link's car speed, one trip's
   departure or one call's time changed in its file, one more, for that input alone, naming both
   fingerprints; the run completes and only that input's fingerprint differs. The record's part
   one alone (no fingerprint): no warning, warnings raised as errors.
6. An out whose trips.parquet (or another of the four files, or run_record.toml) is an input
   file: refused by name for run and run_file, the inputs byte-identical after; inputs of other
   names beside the run's files are kept; run_file into the settings' own folder writes the
   record beside them and leaves run.toml as it was.
7. Each refusal of a settings file by name (ValueError), one case each (a boolean for each kind
   of number included: TOML's true would read as 1), and a file that cannot be read (OSError).
8. Ignored, one UserWarning naming it, the run the minimal settings' (its record the same
   bytes): a key or table a settings file never holds outside [parameters] (at the top, in
   [network], [demand], [transit], a reader's report under [network] or [transit]), and
   stop_walk_max or transfer_walk_max under [parameters] with no [transit]; [result] (a record
   renamed) told in one UserWarning, its part two not read.
9. The two files by their fixed names: run_file given the folder, the path of its run.toml or no
   folder (the current one) writes the same record; a folder itself named run.toml is a folder
   (the same trips.parquet). Refused, saying what to give instead, and nothing written: a folder
   holding a run's record and no run.toml (given by its path, or as "run.toml" from inside it,
   the folder then named "."), the path of a record, a file of another name (run_file and
   Run.open), run.toml given to Run.open, a folder holding neither file (both). A run into a
   folder that holds a finished run: one UserWarning naming it, then its files replaced (the
   record the new run's); a folder holding run.toml alone: no warning. A stray top key `result`
   that is not a table: ignored, named in one warning; an empty [result], or one whose
   fingerprint is not text: the renamed record's one warning and nothing more; each run the
   minimal settings' record, byte for byte. Run.open does not read the end given: a record
   without window_end_s opens.
10. One window end: a settings file ending at 35 950 s, not a whole number of 300 s bins from
   28 800 s: the record keeps window_end_s = 35950 and [result] window_end_used_s = 36000 (24
   bins by hand), window_end_moved counted; the settings edited to 36 300 s (25 whole bins): the
   next run ends there, window_end_used_s = 36300, nothing moved.

For bikes: bike_speed is 17 km/h in case 2 and read back from the record; the Delft rerun holds a
bike trip; the bike's permissions travel in the links and trips files.
"""

import os
import re
import shutil
import time
import tomllib
import warnings
from pathlib import Path

import pyarrow as pa
import pyarrow.parquet as pq
import pytest

import openmobisim as oms
from test_defaults import feed, lon_lat
from test_parameters import PARAMETERS, rows
from test_record import (CLASSES, COLUMNS, DEFAULTS, DEMAND_KW, NETWORK_KW, RUN_KW, TRANSIT_KW,
                         tables)

CRATES = Path(__file__).resolve().parents[2]
NETWORK_FILES = ("nodes_file", "links_file")
CLOCKED = {"search_s", "choose_s", "load_s", "measure_s", "assess_s"}
# Every row a record holds away from its default; choice too (the default is the logit).
EVERY_RUN_KW = {**RUN_KW, "choice": "deterministic"}


def read(path):
    with open(path, "rb") as f:
        return tomllib.load(f)


def inputs(folder):
    """test_record.py's case written into `folder` as three Parquet files and a GTFS feed."""
    folder.mkdir(parents=True, exist_ok=True)
    nodes, links, trips, stops, runs = tables()
    paths = {}
    for name, columns in (("nodes", nodes), ("links", links), ("trips", trips)):
        paths[name] = folder / f"{name}.parquet"
        pq.write_table(pa.table(columns), paths[name])
    paths["gtfs"] = feed(folder / "gtfs.zip", stops, runs)
    return paths


def run_case(files, out, run_kw=EVERY_RUN_KW):
    """The case from its files (str or Path), every entry point's keywords away from the
    default, written into `out`."""
    network = oms.Network.from_tables(files["nodes"], files["links"], **NETWORK_KW)
    transit = oms.Transit.from_gtfs(files["gtfs"], network, **TRANSIT_KW)
    demand = oms.Demand.from_table(files["trips"], CLASSES, network, transit, **DEMAND_KW)
    return oms.run(network, demand, transit, window=(28_800, 36_000), out=out, **run_kw)


def leads_to(record, table, key, given):
    """Whether the path recorded under [table] key, joined to the record's folder, is `given`."""
    value = read(record)[table][key]
    assert not os.path.isabs(value) and "\\" not in value, value
    return os.path.samefile(os.path.join(os.path.dirname(record), value), given)


def all_lead_to(record, files):
    pairs = [("network", "nodes_file", "nodes"), ("network", "links_file", "links"),
             ("demand", "trips_file", "trips"), ("transit", "gtfs_file", "gtfs")]
    return all(leads_to(record, t, k, files[f]) for t, k, f in pairs)


def symlink(target, link, skipped):
    """os.symlink(target, link), True. On Windows a user without the right to create a symbolic
    link (a standard user, developer mode off) is refused with an OSError: then False, the
    reason kept in `skipped`."""
    if os.name != "nt":
        os.symlink(target, link)
        return True
    try:
        os.symlink(target, link)
    except OSError as e:
        skipped.append(f"{link.name}: {e}")
        return False
    return True


def through_link(run, files, again):
    """A run whose inputs or folder are reached through a symbolic link. On POSIX: its recorded
    paths, joined to the record's folder, lead to `files`. On Windows os.path removes a recorded
    ".." before it follows the link, so that text join is no measure there; what the tool does
    is checked instead: each path relative with "/", the record, reached through the link, run
    again by run_file into `again` (trips.parquet and links.parquet the first run's bytes), and
    opened by Run.open."""
    if os.name != "nt":
        assert all_lead_to(run.record, files)
        return
    record = read(run.record)
    for table, key in (("network", "nodes_file"), ("network", "links_file"),
                       ("demand", "trips_file"), ("transit", "gtfs_file")):
        assert not os.path.isabs(record[table][key]) and "\\" not in record[table][key]
    rerun = oms.run_file(settings(run.record), out=again)
    same_run(Path(run.record).parent, Path(rerun.record).parent, record=False)
    assert oms.Run.open(run.record).stopped_by_rule == run.stopped_by_rule


def test_a_recorded_path_joined_to_the_records_folder_is_the_file_given(tmp_path, monkeypatch):
    files = inputs(tmp_path / "inputs")
    one = {"iteration_cap": 1}
    absolute = run_case({k: str(p.resolve()) for k, p in files.items()}, tmp_path / "abs", one)
    assert all_lead_to(absolute.record, files)
    monkeypatch.chdir(tmp_path)
    relative = {k: os.path.join("inputs", p.name) for k, p in files.items()}
    assert all_lead_to(run_case(relative, "rel", one).record, files)
    skipped = []
    if symlink(tmp_path / "inputs", tmp_path / "link", skipped):
        linked = {k: tmp_path / "link" / p.name for k, p in files.items()}
        through_link(run_case(linked, tmp_path / "via-link", one), linked, tmp_path / "again-1")
    deep = run_case(files, tmp_path / "a" / "b" / "c" / "run", one)
    assert all_lead_to(deep.record, files)
    assert read(deep.record)["network"]["nodes_file"] == "../../../../inputs/nodes.parquet"
    # `out` through a symbolic link to a folder at another depth: the path is relative to the
    # folder the record is really in, and os.path resolves it from the link (on POSIX).
    (tmp_path / "real" / "one" / "two").mkdir(parents=True)
    if symlink(tmp_path / "real" / "one" / "two", tmp_path / "short", skipped):
        through_link(run_case(files, tmp_path / "short" / "run", one), files, tmp_path / "again-2")
    # In memory: no file key; a network half from memory cannot run again.
    nodes, links, trips, _, _ = tables()
    network = oms.Network.from_tables(nodes, files["links"])
    transit = oms.Transit.from_gtfs(files["gtfs"], network)
    demand = oms.Demand.from_table(trips, CLASSES, network, transit)
    mixed = oms.run(network, demand, transit, window=(28_800, 36_000), out="mixed", **one)
    record = read(mixed.record)
    assert "nodes_file" not in record["network"] and "trips_file" not in record["demand"]
    assert leads_to(mixed.record, "network", "links_file", files["links"])
    with pytest.raises(ValueError, match="network.nodes_file is missing"):
        oms.run_file(settings(mixed.record), out="again")
    if skipped:
        pytest.skip("the symbolic-link parts (every other part passed): " + "; ".join(skipped))


def settings(record):
    """The record's part one, every line before [result], written as the settings file run.toml
    beside it; their folder, which run_file reads."""
    record = Path(record)
    part_one = record.read_text(encoding="utf-8").split("\n[result]\n")[0] + "\n"
    (record.parent / "run.toml").write_text(part_one, encoding="utf-8")
    return record.parent


def same_run(a, b, record=True):
    """Two run folders: trips.parquet, links.parquet (and run_record.toml) byte-identical,
    iterations.parquet equal but for its wall-clock columns."""
    names = ["trips.parquet", "links.parquet"] + (["run_record.toml"] if record else [])
    for name in names:
        assert (a / name).read_bytes() == (b / name).read_bytes(), name
    x, y = (pq.read_table(f / "iterations.parquet").to_pylist() for f in (a, b))
    assert [{k: v for k, v in r.items() if k not in CLOCKED} for r in x] == \
        [{k: v for k, v in r.items() if k not in CLOCKED} for r in y]


def test_a_run_from_files_runs_again_from_its_record_moved_and_at_another_depth(tmp_path):
    tree = tmp_path / "tree"
    files = inputs(tree / "inputs")
    started = time.perf_counter()
    first = run_case(files, tree / "first")
    seconds = time.perf_counter() - started
    p = read(first.record)["parameters"]
    assert set(p) == set(DEFAULTS) | {"coefficients"}
    assert all(p[k] != v for k, v in DEFAULTS.items()), \
        [k for k, v in DEFAULTS.items() if p[k] == v]
    for class_id, row in p["coefficients"].items():
        assert [row[c] for c in COLUMNS] == EVERY_RUN_KW["coefficients"][class_id]
    started = time.perf_counter()
    oms.run_file(settings(first.record), out=tree / "second")
    rerun = time.perf_counter() - started
    same_run(tree / "first", tree / "second")
    shutil.move(tree, tmp_path / "moved")
    moved = tmp_path / "moved"
    oms.run_file(moved / "first" / "run.toml", out=moved / "third")
    same_run(moved / "first", moved / "third")
    deeper = moved / "deeper" / "still" / "run"
    oms.run_file(moved / "first" / "run.toml", out=deeper)
    same_run(moved / "first", deeper, record=False)
    a = (moved / "first" / "run_record.toml").read_text().splitlines()
    b = (deeper / "run_record.toml").read_text().splitlines()
    differ = [(x, y) for x, y in zip(a, b) if x != y]
    assert len(a) == len(b) and len(differ) == 4, differ
    assert all(x.split(" = ")[0].endswith("_file") and x.split(" = ")[0] == y.split(" = ")[0]
               for x, y in differ), differ
    print(f"the run from files: {seconds:.2f} s; run again from its record: {rerun:.2f} s")


def test_delft_from_its_osm_extract_runs_again_from_its_record(tmp_path):
    osm = CRATES / "io-osm" / "tests" / "fixtures" / "delft" / "network.osm.pbf"
    gtfs = CRATES / "io-gtfs" / "tests" / "fixtures" / "delft" / "gtfs.zip"
    trips = {"trip_id": ["station-markt", "ride"], "origin_lon_deg": [4.3566, 4.3566],
             "origin_lat_deg": [52.0067, 52.0067], "destination_lon_deg": [4.3584, 4.3584],
             "destination_lat_deg": [52.0116, 52.0116], "departure_s": [28_920, 28_920],
             "class_id": ["all", "all"], "modes": ["walk;transit", "bike"]}
    trips_file = tmp_path / "trips.parquet"
    pq.write_table(pa.table(trips), trips_file)
    started = time.perf_counter()
    region = (4.315, 51.965, 4.41, 52.035)
    network = oms.Network.from_osm(osm, region=region)
    transit = oms.Transit.from_gtfs(gtfs, network)
    demand = oms.Demand.from_table(trips_file, ["all"], network, transit)
    first = oms.run(network, demand, transit, window=(28_800, 32_400), out=tmp_path / "first",
                    iteration_cap=1)
    seconds = time.perf_counter() - started
    assert read(first.record)["result"]["network"]["source"] == "osm"
    assert read(first.record)["network"]["region"] == list(region), "the region, run again"
    assert leads_to(first.record, "network", "osm_file", osm)
    assert leads_to(first.record, "transit", "gtfs_file", gtfs)
    started = time.perf_counter()
    oms.run_file(settings(first.record), out=tmp_path / "second")
    rerun = time.perf_counter() - started
    same_run(tmp_path / "first", tmp_path / "second")
    modes = pq.read_table(first.trips).column("mode").to_pylist()
    print(f"Delft from OSM: {seconds:.2f} s; run again from its record: {rerun:.2f} s; "
          f"modes {modes}")


def parsed_defaults():
    """The Default column of PARAMETERS.md for each row a record holds, as numbers."""
    def number(text):
        text = text.replace(" ", "").replace("−", "-")
        return float(text) if "." in text else int(text)

    cells = {}
    for line in PARAMETERS.read_text(encoding="utf-8").splitlines():
        parts = [p.strip() for p in line.split("|")]
        if len(parts) > 4 and parts[1] in rows() and parts[1] != "osm_class_defaults":
            cells[parts[1]] = parts[3].split(" (")[0]
    defaults = {k: number(v) for k, v in cells.items() if k not in ("choice", "coefficients")}
    defaults["choice"] = cells["choice"]
    defaults["coefficients"] = [number(x) for x in cells["coefficients"].split(", ")]
    return defaults


MINIMAL = """window_start_s = 28800
window_end_s = 36000

[network]
nodes_file = "inputs/nodes.parquet"
links_file = "inputs/links.parquet"

[demand]
trips_file = "inputs/trips.parquet"
classes = ["all"]

[transit]
gtfs_file = "inputs/gtfs.zip"
"""


def minimal_case(folder):
    """test_defaults.py's case 4 as files under `folder/inputs` and the minimal settings naming
    them as `folder/run.toml`."""
    xs = {"near": 0.0, "s1": 1_000.0, "far": 2_000.0, "s2": 3_000.0, "door": 3_050.0}
    pos = {n: lon_lat(x, 0.0) for n, x in xs.items()}
    nodes = {"node_id": list(pos), "lon_deg": [p[0] for p in pos.values()],
             "lat_deg": [p[1] for p in pos.values()]}
    walks = [(u, v, m) for a, b, m in [("near", "s1", 1_499.0), ("far", "s1", 1_501.0),
                                       ("s2", "door", 100.0)] for u, v in ((a, b), (b, a))]
    links = pa.table({"link_id": [f"{u}-{v}" for u, v, _ in walks],
                      "upstream": [u for u, _, _ in walks], "downstream": [v for _, v, _ in walks],
                      "modes": ["walk"] * len(walks), "length_m": [m for _, _, m in walks],
                      "lanes": pa.nulls(len(walks), pa.int64()),
                      "car_speed_km_h": pa.nulls(len(walks), pa.float64()),
                      "capacity_veh_h": pa.nulls(len(walks), pa.float64())})
    trips = {"trip_id": ["near", "far"], "origin_lon_deg": [pos["near"][0], pos["far"][0]],
             "origin_lat_deg": [pos["near"][1], pos["far"][1]],
             "destination_lon_deg": [pos["door"][0]] * 2, "destination_lat_deg": [pos["door"][1]] * 2,
             "departure_s": [28_800] * 2, "class_id": ["all"] * 2, "modes": ["transit"] * 2}
    (folder / "inputs").mkdir(parents=True)
    for name, columns in (("nodes", pa.table(nodes)), ("links", links), ("trips", pa.table(trips))):
        pq.write_table(columns, folder / "inputs" / f"{name}.parquet")
    feed(folder / "inputs" / "gtfs.zip", {"S1": pos["s1"], "S2": pos["s2"]},
         {"bus": [("S1", "08:30:00"), ("S2", "08:40:00")]})
    (folder / "run.toml").write_text(MINIMAL)
    return folder / "run.toml"


def test_minimal_settings_run_a_hand_case_at_the_defaults_of_parameters_md(tmp_path):
    result = oms.run_file(minimal_case(tmp_path / "case"), out=tmp_path / "run")
    trips = {r["trip_id"]: r for r in pq.read_table(result.trips).to_pylist()}
    near, far = trips["near"], trips["far"]
    assert (near["mode"], near["travel_time_s"], near["arrival_s"]) == ("transit", 2_475.0, 31_275.0)
    assert (far["mode"], far["alternatives"], far["finished"]) == (None, 0, False)
    p, want = read(result.record)["parameters"], parsed_defaults()
    coefficients = p.pop("coefficients")
    assert list(coefficients) == ["all"]
    assert [coefficients["all"][c] for c in COLUMNS] == want.pop("coefficients")
    assert p == want, {k: (p.get(k), want.get(k)) for k in set(p) | set(want) if p.get(k) != want.get(k)}


def change(files, what):
    """Change one input file: a link's car speed, a trip's departure, a call's time."""
    nodes, links, trips, stops, runs = tables()
    if what == "network":
        pq.write_table(pa.table({**links, "car_speed_km_h": [31.0] + links["car_speed_km_h"][1:]}),
                       files["links"])
    elif what == "demand":
        trips["departure_s"][0] += 1
        pq.write_table(pa.table(trips), files["trips"])
    else:
        feed(files["gtfs"], stops, {**runs, "A0": [runs["A0"][0], ("S2", "08:02:30"), runs["A0"][2]]})


RENAMED = "run.toml: it holds [result]: it is a run's record renamed, not a settings file"


@pytest.mark.parametrize("changed", ["network", "demand", "transit"])
def test_a_changed_input_warns_for_itself_alone_and_unchanged_inputs_do_not_warn(tmp_path,
                                                                                   changed):
    files = inputs(tmp_path / "inputs")
    first = read(run_case(files, tmp_path / "first", {"iteration_cap": 1}).record)["result"]
    with warnings.catch_warnings():
        warnings.simplefilter("error")
        oms.run_file(settings(tmp_path / "first" / "run_record.toml"), out=tmp_path / "part-one")
    renamed = tmp_path / "first" / "run.toml"
    shutil.copyfile(tmp_path / "first" / "run_record.toml", renamed)
    with pytest.warns(UserWarning) as caught:
        oms.run_file(renamed, out=tmp_path / "unchanged")
    assert [str(w.message).startswith(RENAMED) for w in caught] == [True], \
        [str(w.message) for w in caught]
    change(files, changed)
    with pytest.warns(UserWarning) as caught:
        result = oms.run_file(renamed, out=tmp_path / "changed")
    again = read(result.record)["result"]
    said = [str(w.message) for w in caught]
    assert [w.category for w in caught] == [UserWarning] * 2 and said[0].startswith(RENAMED), said
    message = said[1]
    inputs_named = [t for t in ("network", "demand", "transit") if f"the {t} " in message]
    assert inputs_named == [changed], message
    old, new = first[changed]["fingerprint"], again[changed]["fingerprint"]
    assert old != new and old in message and new in message, message
    for other in {"network", "demand", "transit"} - {changed}:
        assert again[other]["fingerprint"] == first[other]["fingerprint"], other
    assert again["iterations"] == 1


def contents(folder):
    return {p.name: p.read_bytes() for p in sorted(folder.iterdir()) if p.is_file()}


# Each output name, and the input file given that name in `out`, the run's other inputs named
# apart: the nodes file as iterations.parquet, the GTFS zip as run_record.toml.
AS_OUTPUT = {"trips.parquet": "trips", "links.parquet": "links", "iterations.parquet": "nodes",
             "run_record.toml": "gtfs"}


def inputs_named(tmp_path, name):
    """The case's inputs in `tmp_path/inputs`, the one of AS_OUTPUT[name] named `name`, the
    others `input-<name>`; their paths by input."""
    made = inputs(tmp_path / "made")
    folder = tmp_path / "inputs"
    folder.mkdir()
    given = {k: folder / (name if k == AS_OUTPUT[name] else f"input-{p.name}")
             for k, p in made.items()}
    for k, p in made.items():
        shutil.copyfile(p, given[k])
    return given, folder


@pytest.mark.parametrize("name", list(AS_OUTPUT))
def test_an_out_holding_an_input_of_an_output_name_is_refused_and_the_input_kept(tmp_path, name):
    given, folder = inputs_named(tmp_path, name)
    before = contents(folder)
    refusal = re.escape(f"{os.sep}{name} is one of the run's input files")
    with pytest.raises(ValueError, match=refusal):
        run_case(given, folder, {"iteration_cap": 1})
    assert contents(folder) == before
    first = run_case(given, tmp_path / "first", {"iteration_cap": 1})
    with pytest.raises(ValueError, match=refusal):
        oms.run_file(settings(first.record), out=folder)
    assert contents(folder) == before


def test_the_overwrite_refusal_comes_before_the_loop(tmp_path):
    # A window ending before it starts is refused by the loop, which runs after the inputs are
    # built; given with an out holding the trips file, the error raised must be the overwrite
    # refusal, so the overwrite check ran before the loop. Without the overwrite (a fresh
    # out), the same call raises the loop's refusal: both refusals apply.
    given, folder = inputs_named(tmp_path, "trips.parquet")
    network = oms.Network.from_tables(given["nodes"], given["links"])
    demand = oms.Demand.from_table(given["trips"], CLASSES, network)
    backwards = {"window": (36_000, 28_800), "iteration_cap": 1}
    with pytest.raises(ValueError) as loop:
        oms.run(network, demand, out=tmp_path / "fresh", **backwards)
    assert "is one of the run's input files" not in str(loop.value), loop.value
    with pytest.raises(ValueError, match="trips.parquet is one of the run's input files"):
        oms.run(network, demand, out=folder, **backwards)
    # run_file: the same, the record's window backwards.
    record = minimal_case(tmp_path / "case")
    record.write_text(MINIMAL.replace("window_end_s = 36000", "window_end_s = 20000"))
    with pytest.raises(ValueError) as loop:
        oms.run_file(record, out=tmp_path / "fresh-file")
    assert "is one of the run's input files" not in str(loop.value), loop.value
    with pytest.raises(ValueError, match="trips.parquet is one of the run's input files"):
        oms.run_file(record, out=tmp_path / "case" / "inputs")
    print(f"the loop's refusal of the window: {loop.value}")


def test_inputs_of_other_names_beside_the_run_are_kept_and_the_settings_folder_is_an_out(tmp_path):
    files = inputs(tmp_path / "inputs")
    before = contents(tmp_path / "inputs")
    beside = tmp_path / "beside"
    beside.mkdir()
    named = {k: beside / f"input-{p.name}" for k, p in files.items()}
    for k, p in files.items():
        shutil.copyfile(p, named[k])
    run_case(named, beside, {"iteration_cap": 1})
    assert {p.name: p.read_bytes() for p in named.values()} == \
        {f"input-{k}": v for k, v in before.items()}
    assert all_lead_to(beside / "run_record.toml", named)
    # run_file into the settings' own folder: the record beside them, the settings as they were.
    record = minimal_case(tmp_path / "case")
    oms.run_file(record, out=tmp_path / "case")
    assert record.read_text() == MINIMAL
    assert read(tmp_path / "case" / "run_record.toml")["result"]["iterations"] >= 1


GTFS_LINE = "gtfs_file = \"inputs/gtfs.zip\"\n"
REFUSALS = [  # (what replaces what in the minimal record, the message)
    (("[network]", "[parameters]\nwalk_sped = 5.0\n\n[network]"),
     r"walk_sped is not a key of the settings file \[parameters\]"),
    (("[network]", "[parameters.coefficients.\"all\"]\nper_km = 1.0\n\n[network]"),
     "per_km is not a column of the choice row"),
    (("[network]", "[parameters.coefficients.\"all\"]\ncar_drive_min = 1.0\n\n[network]"),
     r"parameters.coefficients.\"all\".bike_ride_min is missing"),
    (("window_start_s", "format = 2\nwindow_start_s"), "format 2 is not read"),
    (("window_end_s = 36000\n", ""), "window_end_s is missing"),
    (("window_start_s = 28800", "window_start_s = \"eight\""), "window_start_s: "),
    (("classes = [\"all\"]\n", ""), "demand.classes is missing"),
    (("trips_file = \"inputs/trips.parquet\"\n", ""), "demand.trips_file is missing"),
    (("[network]\nnodes_file = \"inputs/nodes.parquet\"\nlinks_file = \"inputs/links.parquet\"\n",
      "network = 3\n"), "network is not a table"),
    (("[network]\nnodes_file = \"inputs/nodes.parquet\"\nlinks_file = \"inputs/links.parquet\"\n",
      ""), r"\[network\] is missing"),
    (("links_file = \"inputs/links.parquet\"\n", ""), "network.links_file is missing"),
    (("[network]\n", "[network]\nosm_file = \"inputs/x.osm.pbf\"\n"), "are two networks"),
    (("gtfs_file = \"inputs/gtfs.zip\"\n", "stops = 2\n"), "transit.gtfs_file is missing"),
    (("[network]", "[parameters]\nseed = -1\n\n[network]"), "seed: "),
] + [  # a boolean for each kind of number the record holds
    (("[network]", f"{table}\n{key} = true\n\n[network]"), f"{name}: a boolean")
    for table, key, name in [
        ("[parameters]", "iteration_cap", "parameters.iteration_cap"),
        ("[parameters]", "bike_speed", "parameters.bike_speed"),
        ("[parameters]", "seed", "parameters.seed"),
        ("[parameters]", "link_bin", "parameters.link_bin"),
        ("[parameters]", "nest_scale", "parameters.nest_scale"),
        ("[parameters.coefficients.\"all\"]", "car_drive_min",
         "parameters.coefficients.\"all\".car_drive_min"),
    ]
] + [
    (("window_start_s", "format = true\nwindow_start_s"), "format: a boolean"),
    (("window_start_s = 28800", "window_start_s = false"), "window_start_s: a boolean"),
    (("window_end_s = 36000", "window_end_s = true"), "window_end_s: a boolean"),
]


IGNORED = [  # (what replaces what in the minimal settings, the warning)
    (("window_start_s", "speed = 3\nwindow_start_s"), "ignored: speed"),
    (("[network]", "[scenario]\nname = 1\n\n[network]"), "ignored: scenario"),
    (("[network]\n", "[network]\ncrs = 1\n"), r"ignored: crs \[network\]"),
    (("classes = [\"all\"]\n", "classes = [\"all\"]\nzones = 3\n"), r"ignored: zones \[demand\]"),
    ((GTFS_LINE, GTFS_LINE + "agency = 1\n"), r"ignored: agency \[transit\]"),
    ((GTFS_LINE, GTFS_LINE + "\n[result]\nseconds = 1.0\n"), re.escape(RENAMED)),
    (("[demand]", "[network.osm_report]\nnodez = 1\n\n[demand]"),
     r"ignored: osm_report \[network\]"),
    ((GTFS_LINE, GTFS_LINE + "\n[transit.gtfs_report]\nroutes = 1\n"),
     r"ignored: gtfs_report \[transit\]"),
]
WITHOUT_TRANSIT = [  # the same, the settings without [transit]
    (("[transit]\n" + GTFS_LINE, "[parameters]\ntransfer_walk_max = 400.0\n"),
     "parameters.transfer_walk_max is ignored: it is read only with a transit"),
    (("[transit]\n" + GTFS_LINE, "[parameters]\nstop_walk_max = 1500.0\n"),
     "parameters.stop_walk_max is ignored: it is read only with a transit"),
]


@pytest.mark.parametrize("case", range(len(IGNORED + WITHOUT_TRANSIT)),
                         ids=[m for _, m in IGNORED + WITHOUT_TRANSIT])
def test_a_key_the_settings_do_not_read_is_ignored_and_warned(tmp_path, case):
    (old, new), match = (IGNORED + WITHOUT_TRANSIT)[case]
    record = minimal_case(tmp_path)
    baseline_text = MINIMAL if case < len(IGNORED) else MINIMAL.replace("[transit]\n" + GTFS_LINE, "")
    record.write_text(baseline_text)
    baseline = Path(oms.run_file(record, out=tmp_path / "baseline").record).read_bytes()
    assert MINIMAL.count(old) == 1, old
    record.write_text(MINIMAL.replace(old, new))
    with pytest.warns(UserWarning, match=match):
        result = oms.run_file(record, out=tmp_path / "ignored")
    assert Path(result.record).read_bytes() == baseline


@pytest.mark.parametrize("case", range(len(REFUSALS)), ids=[m for _, m in REFUSALS])
def test_a_settings_file_is_refused_by_name(tmp_path, case):
    (old, new), match = REFUSALS[case]
    record = minimal_case(tmp_path)
    assert MINIMAL.count(old) == 1, old
    record.write_text(MINIMAL.replace(old, new))
    with pytest.raises(ValueError, match=match):
        oms.run_file(record, out=tmp_path / "refused")
    assert not (tmp_path / "refused").exists(), match


def test_a_settings_file_not_toml_or_not_read_and_a_file_it_names_absent(tmp_path):
    record = minimal_case(tmp_path)
    record.write_text("window_start_s = = 1\n")
    with pytest.raises(tomllib.TOMLDecodeError):
        oms.run_file(record, out=tmp_path / "not-toml")
    with pytest.raises(OSError):
        oms.run_file(tmp_path / "absent.toml", out=tmp_path / "absent")
    record.write_text(MINIMAL.replace("inputs/trips.parquet", "inputs/gone.parquet"))
    with pytest.raises(OSError, match="gone.parquet"):
        oms.run_file(record, out=tmp_path / "gone")


AGAIN = "write a run.toml (the record's lines before [result] are one), or copy the record to run.toml"


def test_the_two_files_are_found_by_their_names_and_another_name_is_refused(tmp_path, monkeypatch):
    record = minimal_case(tmp_path / "case")
    folder = record.parent
    by_folder = oms.run_file(folder, out=tmp_path / "by-folder")
    by_file = oms.run_file(record, out=tmp_path / "by-file")
    monkeypatch.chdir(folder)
    here = oms.run_file(out=tmp_path / "here")
    written = [Path(r.record).read_bytes() for r in (by_folder, by_file, here)]
    assert written[0] == written[1] == written[2]
    finished = tmp_path / "by-folder"
    empty = tmp_path / "empty"
    empty.mkdir()
    other = folder / "settings.toml"
    shutil.copyfile(record, other)
    refused = [  # (the call, the error, the message)
        (lambda: oms.run_file(finished, out=tmp_path / "refused"), ValueError, AGAIN),
        (lambda: oms.run_file(finished / "run_record.toml", out=tmp_path / "refused"), ValueError,
         "it is the record a run wrote; to run it again, " + AGAIN),
        (lambda: oms.run_file(other, out=tmp_path / "refused"), ValueError,
         "rename the file run.toml, or give its folder"),
        (lambda: oms.run_file(tmp_path / "nowhere", out=tmp_path / "refused"), FileNotFoundError,
         "run_file reads a settings file named run.toml"),
        (lambda: oms.Run.open(record), ValueError, "it is a settings file; Run.open opens a finished run"),
        (lambda: oms.Run.open(other), ValueError, "Run.open opens a finished run"),
        (lambda: oms.run_file(empty, out=tmp_path / "refused"), FileNotFoundError,
         f"{empty}: no run.toml there"),
        (lambda: oms.Run.open(empty), FileNotFoundError, f"{empty}: no run_record.toml there"),
    ]
    for call, error, message in refused:
        with pytest.raises(error, match=re.escape(message)):
            call()
    monkeypatch.chdir(finished)
    with pytest.raises(ValueError, match=re.escape(".: the folder holds a run's record")):
        oms.run_file("run.toml", out=tmp_path / "refused")
    assert not (tmp_path / "refused").exists()
    named = shutil.copytree(folder, tmp_path / "run.toml")
    from_named = oms.run_file(named, out=tmp_path / "from-named")
    assert Path(from_named.trips).read_bytes() == Path(by_folder.trips).read_bytes()
    assert oms.Run.open(finished).stopped_by_rule == by_folder.stopped_by_rule
    assert oms.Run.open(finished / "run_record.toml").stopped_by_rule == by_folder.stopped_by_rule
    # A run into a folder holding a finished run: one warning, its files replaced.
    with pytest.warns(UserWarning) as caught:
        again = oms.run_file(folder, out=finished)
    assert [str(w.message) for w in caught] == [
        f"{finished}: the folder holds a finished run (its run_record.toml); this run replaces its files"]
    assert Path(again.record).read_bytes() == written[0]
    # A folder holding the settings alone: no warning.
    with warnings.catch_warnings():
        warnings.simplefilter("error")
        oms.run_file(folder, out=folder)
    assert record.read_text() == MINIMAL


def test_one_window_end_the_end_given_kept_and_the_end_used_a_result(tmp_path):
    record = minimal_case(tmp_path / "case")
    record.write_text(MINIMAL.replace("window_end_s = 36000", "window_end_s = 35950"))
    with pytest.warns(UserWarning, match="window_end_moved; first: 36000"):
        moved = read(oms.run_file(record, out=tmp_path / "moved").record)
    assert (moved["window_end_s"], moved["result"]["window_end_used_s"]) == (35_950, 36_000)
    assert moved["result"]["repairs"]["window_end_moved"] == 1
    record.write_text(MINIMAL.replace("window_end_s = 36000", "window_end_s = 36300"))
    whole = read(oms.run_file(record, out=tmp_path / "whole").record)
    assert (whole["window_end_s"], whole["result"]["window_end_used_s"]) == (36_300, 36_300)
    assert "window_end_moved" not in whole["result"].get("repairs", {})


def test_a_stray_result_is_ignored_and_a_record_opens_without_its_end_given(tmp_path):
    record = minimal_case(tmp_path / "case")
    baseline = Path(oms.run_file(record, out=tmp_path / "baseline").record).read_bytes()
    record.write_text("result = 3\n" + MINIMAL)
    with pytest.warns(UserWarning) as caught:
        stray = oms.run_file(record, out=tmp_path / "stray")
    assert [str(w.message) for w in caught] == \
        ["run.toml: 1 keys a settings file never holds, ignored: result"]
    assert Path(stray.record).read_bytes() == baseline
    for name, part_two in (("empty", "\n[result]\n"), ("not-text", "\n[result.network]\nfingerprint = 5\n")):
        record.write_text(MINIMAL + part_two)
        with pytest.warns(UserWarning) as caught:
            again = oms.run_file(record, out=tmp_path / name)
        said = [str(w.message) for w in caught]
        assert len(said) == 1 and said[0].startswith(RENAMED), said
        assert Path(again.record).read_bytes() == baseline
    finished = tmp_path / "baseline" / "run_record.toml"
    text = finished.read_text(encoding="utf-8")
    assert text.count("window_end_s = 36000\n") == 1
    finished.write_text(text.replace("window_end_s = 36000\n", ""), encoding="utf-8")
    with warnings.catch_warnings():
        warnings.simplefilter("error")
        opened = oms.Run.open(tmp_path / "baseline")
    assert opened.stopped_by_rule == read(finished)["result"]["stopped_by_rule"]
