"""The run's record, run_record.toml, read with Python's standard tomllib (TOML 1.0), against answers from
outside the simulator: the rows of PARAMETERS.md (their names, units and defaults, written here
from that table), the inputs of the run itself (rebuilt from the record alone), and TOML's
string rules.

1. The names under [parameters] are the rows of PARAMETERS.md, less osm_class_defaults (never
   written: its one value is the OSM reader's one table) and, in a run without transit, less
   transfer_walk_max and stop_walk_max (no transit: nothing reads them); each key's unit
   comment is that row's Unit cell; with no keyword anywhere every value is the default of
   PARAMETERS.md written below.
2. Reconstruction: a run with transit and non-default keywords at every entry point, then the
   same input tables and feed rebuilt and rerun with ONLY values read from its record:
   trips.parquet, links.parquet and run_record.toml byte-identical, iterations.parquet equal but for
   its five wall-clock columns, the fingerprints equal; [transit.gtfs_report] is the reader's
   counts as Run.gtfs_report returns them, key by key (both in part two, [result]).
3. Each fingerprint answers to its own input: one link's car speed changed changes the
   network's alone, one trip's departure the demand's alone, one call's time the transit's.
4. Determinism, the one self-comparison: grid.py's logit run and the case of 2, each twice in
   this process and at RAYON_NUM_THREADS=1 and 8 in a fresh interpreter (as
   test_determinism.py runs it): run_record.toml byte-identical across the four.
5. Class ids with a quote, a backslash, control characters and a letter outside ASCII read back
   unchanged as TOML strings and quoted keys.
6. Delft's OSM extract and GTFS feed (the fixtures test_delft.py reads): source "osm", and
   [result.network.osm_report] and [result.transit.gtfs_report] equal Run.osm_report and
   Run.gtfs_report.

The case of 1 and 2: a 6 x 6 grid of grid.py (streets for car, bike and walk, one way for bikes
on a drawn share) placed in longitude/latitude, two bus lines on it, 120 trips open to every mode
in two classes, 08:00:00 to 10:00:00. For bikes: bike_speed is a row of [parameters] like any
other, and in the reconstruction it is 17 km/h, read back from the record and given again.
"""

import os
import random
import subprocess
import sys
import time
import tomllib
from pathlib import Path

import pyarrow.parquet as pq

import openmobisim as oms
from grid import grid, logit_run
from test_defaults import feed, lon_lat
from test_parameters import PARAMETERS, rows

TESTS = Path(__file__).resolve().parent
START, END = 28_800, 36_000
# The columns of the choice row, in order (crates/loop/README.md).
COLUMNS = [
    "car_drive_min", "bike_ride_min", "walk_min", "transit_in_vehicle_min", "transit_access_min",
    "transit_egress_min", "transit_first_wait_min", "transit_transfer_wait_min",
    "transit_transfer_walk_min", "transit_transfers", "money_eur", "car_ln_path_size", "is_car",
    "is_bike", "is_walk", "is_transit",
]
# The defaults of PARAMETERS.md, its Default column.
DEFAULTS = {
    "capacity_per_lane": 1_800.0, "bike_speed": 15.0, "walk_speed": 4.8, "stop_walk_max": 1_500.0,
    "transfer_walk_max": 400.0, "boarding_slack": 60, "max_rides": 4, "nearby_stops": 3,
    "max_car_routes": 6, "max_transit_journeys": 10,
    "choice": "logit", "nest_scale": 0.5, "step_exponent": 0.75, "iteration_cap": 50,
    "iteration_floor": 0, "link_bin": 300, "seed": 0,
}
DEFAULT_COEFFICIENTS = [0.1, 0.1, 0.165, 0.1, 0.165, 0.165, 0.145, 0.145, 0.165, 0.5, 0.6, -1.0,
                        0.0, 0.0, 0.5, 0.25]
CLASSES = ["peak", "off"]
# Non-default keywords at every entry point; 0.1 + 0.2 and 1e-7 test the floats' round trip.
NETWORK_KW = {"capacity_per_lane": 1_500.0, "bike_speed": 17.0, "walk_speed": 4.7}
TRANSIT_KW = {"transfer_walk_max": 350.0}
DEMAND_KW = {"stop_walk_max": 1_200.0}
RUN_KW = {
    "boarding_slack": 45, "max_rides": 3, "nearby_stops": 2, "max_car_routes": 5,
    "max_transit_journeys": 8, "choice": "logit",
    "nest_scale": 0.6, "step_exponent": 0.9, "iteration_cap": 4, "iteration_floor": 2,
    "link_bin": 600, "seed": 11,
    "coefficients": {
        "peak": [0.12, 0.1, 0.2, 0.1, 0.2, 0.2, 0.15, 0.15, 0.2, 0.4, 0.5, -1.0, 0.0, 0.1 + 0.2,
                 0.5, 0.25],
        "off": [0.08, 0.09, 0.15, 0.1, 0.15, 0.15, 0.13, 0.13, 0.15, 0.6, 0.7, -0.9, 1e-7, 0.0,
                0.4, 0.3],
    },
}


def tables():
    """The nodes, links and trips tables of the case, and the stops and runs of its feed."""
    nodes, links, pos, _ = grid(6, seed=5, cars=True)
    lonlat = {n: lon_lat(*xy) for n, xy in pos.items()}
    nodes = {"node_id": list(lonlat), "lon_deg": [p[0] for p in lonlat.values()],
             "lat_deg": [p[1] for p in lonlat.values()]}
    stops = {f"S{n}": lonlat[f"n{n}"] for n in (0, 2, 5, 11, 23, 35)}
    runs = {}
    for k in range(6):
        h, m = 8 + k // 6, (k * 10) % 60
        runs[f"A{k}"] = [("S0", f"{h:02d}:{m:02d}:00"), ("S2", f"{h:02d}:{m + 2:02d}:00"),
                         ("S5", f"{h:02d}:{m + 5:02d}:00")]
        runs[f"B{k}"] = [("S11", f"{h:02d}:{m + 3:02d}:00"), ("S23", f"{h:02d}:{m + 6:02d}:00"),
                         ("S35", f"{h:02d}:{m + 9:02d}:00")]
    rng = random.Random(6)
    ids = list(pos)
    trips = {c: [] for c in ["trip_id", "origin_lon_deg", "origin_lat_deg",
                             "destination_lon_deg", "destination_lat_deg", "departure_s",
                             "class_id", "modes"]}
    for i in range(120):
        o, d = rng.sample(ids, 2)
        row = [f"t{i:03d}", *lonlat[o], *lonlat[d], START + rng.randrange(900), CLASSES[i % 2],
               "car;bike;walk;transit"]
        for column, value in zip(trips, row):
            trips[column].append(value)
    return nodes, links, trips, stops, runs


def run_case(out, gtfs, network_kw, transit_kw, demand_kw, run_kw, classes=CLASSES,
             window=(START, END), inputs=None):
    """The case run with the keywords given, written into `out`; its Run. `inputs`, if given,
    replaces the nodes, links and trips tables."""
    nodes, links, trips = inputs or tables()[:3]
    network = oms.Network.from_tables(nodes, links, **network_kw)
    transit = oms.Transit.from_gtfs(gtfs, network, **transit_kw)
    demand = oms.Demand.from_table(trips, classes, network, transit, **demand_kw)
    return oms.run(network, demand, transit, window=window, out=out, **run_kw)


def gtfs_of(tmp_path):
    _, _, _, stops, runs = tables()
    return feed(tmp_path / "gtfs.zip", stops, runs)


def read(path):
    with open(path, "rb") as f:
        return tomllib.load(f)


def units():
    """Each row of PARAMETERS.md and its Unit cell."""
    cells = {}
    for line in PARAMETERS.read_text(encoding="utf-8").splitlines():
        parts = [p.strip() for p in line.split("|")]
        if len(parts) > 3 and parts[1] in rows():
            cells[parts[1]] = parts[2]
    return cells


def test_the_parameter_names_units_and_defaults_are_those_of_parameters_md(tmp_path):
    t0 = time.perf_counter()
    result = run_case(tmp_path / "run", gtfs_of(tmp_path), {}, {}, {}, {})
    seconds = time.perf_counter() - t0
    record = read(result.record)
    p = record["parameters"]
    names = set(rows()) - {"osm_class_defaults"}
    assert set(p) == names, sorted(set(p) ^ names)
    for name, default in DEFAULTS.items():
        assert type(p[name]) is type(default) and p[name] == default, (name, p[name], default)
    assert list(p["coefficients"]) == sorted(CLASSES)
    for class_id, row in p["coefficients"].items():
        assert list(row) == COLUMNS, class_id
        assert list(row.values()) == DEFAULT_COEFFICIENTS, class_id
    # The unit after each key of [parameters], against the Unit column (coefficients, whose cell
    # is a sentence, is a table and has no unit comment).
    want, section, got = units(), None, {}
    for line in Path(result.record).read_text().splitlines():
        if line.startswith("["):
            section = line
        elif section == "[parameters]" and " = " in line:
            key, _, rest = line.partition(" = ")
            got[key] = rest.partition("  # ")[2]
    assert got == {k: want[k] for k in got} and set(got) == names - {"coefficients"}, got
    # Without transit, transfer_walk_max has no value and is absent.
    nodes, links = {"node_id": ["a", "b"], "x_m": [0.0, 1_500.0], "y_m": [0.0, 0.0]}, {
        "link_id": ["ab"], "upstream": ["a"], "downstream": ["b"], "modes": ["bike;walk"],
        "length_m": [1_500.0], "lanes": [None], "car_speed_km_h": [None],
        "capacity_veh_h": [None]}
    trips = {"trip_id": ["ride"], "origin_x_m": [0.0], "origin_y_m": [0.0],
             "destination_x_m": [1_500.0], "destination_y_m": [0.0], "departure_s": [0],
             "class_id": ["all"], "modes": ["bike"]}
    network = oms.Network.from_tables(nodes, links)
    demand = oms.Demand.from_table(trips, ["all"], network)
    alone = read(oms.run(network, demand, window=(0, 3_600), out=tmp_path / "alone").record)
    assert set(alone["parameters"]) == names - {"transfer_walk_max", "stop_walk_max"}
    assert "transit" not in alone and "transit" not in alone["result"]
    assert "osm_report" not in alone["result"]["network"]
    print(f"the default run with transit: {seconds:.2f} s, "
          f"{record['result']['iterations']} iterations")


def test_a_run_rebuilt_from_its_record_alone_gives_the_same_files(tmp_path):
    gtfs = gtfs_of(tmp_path)
    t0 = time.perf_counter()
    first = run_case(tmp_path / "first", gtfs, NETWORK_KW, TRANSIT_KW, DEMAND_KW, RUN_KW)
    seconds = time.perf_counter() - t0
    record = read(first.record)
    p = record["parameters"]
    network_kw = {k: p[k] for k in ("capacity_per_lane", "bike_speed", "walk_speed")}
    run_kw = {k: p[k] for k in ("boarding_slack", "max_rides", "nearby_stops", "max_car_routes",
                                "max_transit_journeys", "choice",
                                "nest_scale", "step_exponent", "iteration_cap", "iteration_floor",
                                "link_bin", "seed")}
    run_kw["coefficients"] = {c: [row[k] for k in COLUMNS] for c, row in p["coefficients"].items()}
    again = run_case(tmp_path / "again", gtfs, network_kw,
                     {"transfer_walk_max": p["transfer_walk_max"]},
                     {"stop_walk_max": p["stop_walk_max"]}, run_kw,
                     classes=record["demand"]["classes"],
                     window=(record["window_start_s"], record["window_end_s"]))
    for name in ("trips.parquet", "links.parquet", "run_record.toml"):
        a, b = (tmp_path / "first" / name).read_bytes(), (tmp_path / "again" / name).read_bytes()
        assert a == b, name
    clocked = {"search_s", "choose_s", "load_s", "measure_s", "assess_s"}
    a, b = (pq.read_table(r.iterations).to_pylist() for r in (first, again))
    assert [{k: v for k, v in row.items() if k not in clocked} for row in a] == \
        [{k: v for k, v in row.items() if k not in clocked} for row in b]
    assert len(a) == record["result"]["iterations"]
    rebuilt = read(again.record)
    for table in ("network", "demand", "transit"):
        assert rebuilt["result"][table]["fingerprint"] == record["result"][table]["fingerprint"], table
    assert record["result"]["transit"]["gtfs_report"] == first.gtfs_report
    # The keywords given are the ones recorded, so nothing above was read from a default.
    given = {**NETWORK_KW, **TRANSIT_KW, **DEMAND_KW, **RUN_KW}
    del given["coefficients"]
    assert {k: p[k] for k in given} == given
    assert run_kw["coefficients"] == RUN_KW["coefficients"]
    modes = sorted({m for m in pq.read_table(first.trips).column("mode").to_pylist() if m})
    print(f"the run: {seconds:.2f} s; modes taken: {modes}")


def test_one_input_changed_changes_its_own_fingerprint_alone(tmp_path):
    nodes, links, trips, stops, runs = tables()
    base = {"network": (nodes, links, trips), "feed": runs}
    faster = {**links, "car_speed_km_h": [31.0] + links["car_speed_km_h"][1:]}
    later = {**trips, "departure_s": [trips["departure_s"][0] + 1] + trips["departure_s"][1:]}
    moved = {**runs, "A0": [runs["A0"][0], ("S2", "08:02:30"), runs["A0"][2]]}
    cases = {"base": base, "network": {**base, "network": (nodes, faster, trips)},
             "demand": {**base, "network": (nodes, links, later)}, "transit": {**base, "feed": moved}}
    prints = {}
    for name, case in cases.items():
        folder = tmp_path / name
        folder.mkdir()
        gtfs = feed(folder / "gtfs.zip", stops, case["feed"])
        result = run_case(folder / "run", gtfs, {}, {}, {}, {"iteration_cap": 1},
                          inputs=case["network"])
        record = read(result.record)
        prints[name] = {t: record["result"][t]["fingerprint"] for t in ("network", "demand", "transit")}
    for changed in ("network", "demand", "transit"):
        for table in ("network", "demand", "transit"):
            same = prints[changed][table] == prints["base"][table]
            assert same == (table != changed), (changed, table)


def transit_run(folder):
    """The case of the reconstruction, its feed and its run in `folder`."""
    folder.mkdir(parents=True, exist_ok=True)
    return run_case(folder / "run", gtfs_of(folder), NETWORK_KW, TRANSIT_KW, DEMAND_KW, RUN_KW)


def test_the_record_is_the_same_bytes_twice_and_at_1_and_8_threads(tmp_path):
    for name, script, in_process in (("grid", TESTS / "grid.py", logit_run),
                                     ("transit", Path(__file__), transit_run)):
        folders = [tmp_path / name / "first", tmp_path / name / "second"]
        for folder in folders:
            in_process(folder)
        for threads in ("1", "8"):
            folder = tmp_path / name / f"threads-{threads}"
            env = {**os.environ, "RAYON_NUM_THREADS": threads}
            subprocess.run([sys.executable, str(script), str(folder)], env=env, check=True)
            folders.append(folder)
        records = [next(folder.rglob("run_record.toml")).read_bytes() for folder in folders]
        assert all(r == records[0] for r in records), name


def test_delft_from_osm_records_the_readers_counts_as_the_run_returns_them(tmp_path):
    crates = Path(__file__).resolve().parents[2]
    started = time.perf_counter()
    network = oms.Network.from_osm(crates / "io-osm/tests/fixtures/delft/network.osm.pbf")
    transit = oms.Transit.from_gtfs(crates / "io-gtfs/tests/fixtures/delft/gtfs.zip", network)
    trips = {"trip_id": ["station-markt"], "origin_lon_deg": [4.3566],
             "origin_lat_deg": [52.0067], "destination_lon_deg": [4.3584],
             "destination_lat_deg": [52.0116], "departure_s": [28_920], "class_id": ["all"],
             "modes": ["walk;transit"]}
    demand = oms.Demand.from_table(trips, ["all"], network, transit=transit)
    result = oms.run(network, demand, transit, window=(28_800, 32_400), out=tmp_path,
                     iteration_cap=1)
    seconds = time.perf_counter() - started
    record = read(result.record)
    found = record["result"]
    assert found["network"]["source"] == "osm"
    assert "region" not in record["network"], "the whole extract: no region"
    assert found["network"]["osm_report"] == result.osm_report
    assert found["transit"]["gtfs_report"] == result.gtfs_report
    assert (found["network"]["nodes"], found["network"]["links"]) == \
        (result.osm_report["nodes"], result.osm_report["links"])
    print(f"Delft read, run and written: {seconds:.2f} s")


def test_class_ids_with_quotes_backslashes_and_control_characters_read_back_unchanged(tmp_path):
    classes = ['say "hi"', "back\\slash", "line\nbreak\ttab\x7f\x01", "fahrräder"]
    nodes = {"node_id": ["a", "b"], "x_m": [0.0, 1_500.0], "y_m": [0.0, 0.0]}
    links = {"link_id": ["ab"], "upstream": ["a"], "downstream": ["b"], "modes": ["bike;walk"],
             "length_m": [1_500.0], "lanes": [None], "car_speed_km_h": [None],
             "capacity_veh_h": [None]}
    n = len(classes)
    trips = {"trip_id": [f"t{i}" for i in range(n)], "origin_x_m": [0.0] * n,
             "origin_y_m": [0.0] * n, "destination_x_m": [1_500.0] * n,
             "destination_y_m": [0.0] * n, "departure_s": [0] * n, "class_id": classes,
             "modes": ["bike"] * n}
    network = oms.Network.from_tables(nodes, links)
    demand = oms.Demand.from_table(trips, classes, network)
    record = read(oms.run(network, demand, window=(0, 3_600), out=tmp_path).record)
    # The ids in id order: sorted by their UTF-8 bytes, which is Python's order of code points.
    assert record["demand"]["classes"] == sorted(classes)
    assert list(record["parameters"]["coefficients"]) == sorted(classes)


if __name__ == "__main__":
    transit_run(Path(sys.argv[1]))
