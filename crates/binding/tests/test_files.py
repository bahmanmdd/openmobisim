"""Tables from Parquet files: Network.from_tables and Demand.from_table given the path of a file
written by pyarrow 26.0.1 (another tool) from hand values, in place of a table in memory.

1. The hand run of crates/io-parquet (test_hand_run.py's tables, its derivation there) from
   three files passes read_back.py unchanged, and the three hand cases of test_defaults.py from
   files give their hand numbers: a bike rides 1 500 m at 15 km/h in 1 500 / (15 000 / 3 600) =
   360 s; a walker walks 1 000 m at 4.8 km/h in 750 s; three cars through a link of two lanes
   at 1 800 veh/h per lane (one car a second), 1 000 m at 36 km/h (100 s), take 100, 101, 102 s.
2. A file and the same table in memory give the same network and demand: equal fingerprints in
   run.toml (a fingerprint hashes what the network or demand was built from).
3. The column types read (strings Utf8 or LargeUtf8; integers Int32, Int64, UInt32, UInt64;
   floats Float64 or those integers), each in a file, give the fingerprints of the plain table;
   so do a text column of numbers for a float, a Float32 and a float column of whole numbers
   for an integer (the binding's cell rules).
4. Compression: pyarrow's default (snappy, checked in the file's metadata) and none both read,
   with equal fingerprints; zstd refused by name.
5. Refusals by name, one case each: a type that is not text or a number (a boolean), a missing
   column, a column name held twice (refused in memory too, by pyarrow); a file that is not
   Parquet (ValueError); a file that does not exist, a directory, an empty path and an
   unreadable file (OSError). Repaired, counted and warned once, one case each, by hand: an
   integer too large for its field (lanes 300, a UInt64 above 2^63 - 1) and a null where none is
   allowed (a column of nulls alone included) leave their row out (`links_unread`,
   `trips_unread`, naming the row and the column: the network then has the hand run's 3 links
   less those, the demand its 7 trips less those); an extra column is ignored
   (`columns_ignored`). A node row whose x_m is null leaves node b out (`nodes_unread`) and every
   link naming it: ab and st1 run a->b, st2 b->c, so all three (`links_to_no_node` 3).
6. A column of nulls alone (pyarrow's `null` type) where nulls are allowed reads as the same
   table in memory: the same fingerprint, and the bike's hand 360 s.
7. Files of more than 1 024 rows in several row groups (a 33 x 33 grid: 1 089 nodes, 4 224
   links, by counting; 1 100 trips) give those counts and the fingerprints of the same tables
   in memory.

For bikes: the hand run's bike (360 s) is read back from files, case 1 is a bike's ride from
files, and a link's bike permission is a string cell read like any other.
"""

import os
import re
import time
import tomllib
from pathlib import Path

import pyarrow as pa
import pyarrow.parquet as pq
import pytest

import openmobisim as oms
from grid import grid
from test_defaults import LINK_COLUMNS, TRIP_COLUMNS, table
from test_hand_run import START as HAND_START, hand_run, hand_tables, read_back

START, END = 28_800, 36_000


def write(folder, name, columns, **options):
    """`columns` (a dict of lists or a pyarrow table) written by pyarrow as `folder/name`."""
    folder.mkdir(parents=True, exist_ok=True)
    path = folder / name
    pq.write_table(columns if isinstance(columns, pa.Table) else pa.table(columns), path,
                   **options)
    return path


def fingerprints(record):
    with open(record, "rb") as f:
        r = tomllib.load(f)
    return r["network"]["fingerprint"], r["demand"]["fingerprint"]


def test_the_hand_run_from_files_passes_read_back(tmp_path):
    nodes, links, trips = hand_tables()
    files = [write(tmp_path / "in", f"{n}.parquet", t)
             for n, t in (("nodes", nodes), ("links", links), ("trips", trips))]
    started = time.perf_counter()
    from_files = hand_run(*files, tmp_path / "files")
    seconds = time.perf_counter() - started
    output, passed = read_back(tmp_path / "files")
    print(output)
    assert passed, output
    in_memory = hand_run(nodes, links, trips, tmp_path / "memory")
    assert fingerprints(from_files.record) == fingerprints(in_memory.record)
    print(f"the hand run from files: {seconds:.3f} s")


def line_files(folder, length_m, modes, rows, lanes=None, speed=None):
    """A link a->b of `length_m` in metres and the trips `rows` (id, destination x, modes) from
    (0, 0) leaving at START, as three files; run with no keyword, the trips by id and the links
    table's rows."""
    nodes = {"node_id": ["a", "b"], "x_m": [0.0, length_m], "y_m": [0.0, 0.0]}
    links = pa.table(table(LINK_COLUMNS, [("ab", "a", "b", modes, length_m, lanes, speed, None)]),
                     schema=pa.schema([(c, pa.string()) for c in LINK_COLUMNS[:4]]
                                      + [("length_m", pa.float64()), ("lanes", pa.int64()),
                                         ("car_speed_km_h", pa.float64()),
                                         ("capacity_veh_h", pa.float64())]))
    trips = table(TRIP_COLUMNS, [(i, 0.0, 0.0, x, 0.0, START, "all", m) for i, x, m in rows])
    network = oms.Network.from_tables(write(folder, "nodes.parquet", nodes),
                                      write(folder, "links.parquet", links))
    demand = oms.Demand.from_table(write(folder, "trips.parquet", trips), ["all"], network)
    result = oms.run(network, demand, window=(START, END), out=folder / "run")
    return ({r["trip_id"]: r for r in pq.read_table(result.trips).to_pylist()},
            pq.read_table(result.links).to_pylist())


def test_the_hand_cases_of_the_defaults_from_files(tmp_path):
    trips, _ = line_files(tmp_path / "bike", 1_500.0, "bike;walk", [("ride", 1_500.0, "bike")])
    assert trips["ride"]["mode"] == "bike" and trips["ride"]["travel_time_s"] == 360.0
    trips, _ = line_files(tmp_path / "walk", 1_000.0, "bike;walk", [("walk", 1_000.0, "walk")])
    assert trips["walk"]["mode"] == "walk" and trips["walk"]["travel_time_s"] == 750.0
    rows = [(f"car{i}", 1_000.0, "car") for i in range(3)]
    trips, links = line_files(tmp_path / "car", 1_000.0, "car", rows, lanes=2, speed=36.0)
    assert sorted(t["travel_time_s"] for t in trips.values()) == [100.0, 101.0, 102.0]
    assert [(r["link_id"], r["cars"], r["first_exit_s"], r["last_exit_s"]) for r in links] == \
        [("ab", 3, 28_900.0, 28_902.0)]


def built(tmp_path, name, nodes, links, trips):
    """Network and demand built from the hand run's three tables or files given and run for
    one iteration in its window; the two fingerprints of its record."""
    network = oms.Network.from_tables(nodes, links)
    demand = oms.Demand.from_table(trips, ["commute", "leisure"], network)
    result = oms.run(network, demand, window=(HAND_START, HAND_START + 3_600), out=tmp_path / name,
                     iteration_cap=1)
    return fingerprints(result.record)


def test_every_column_type_read_gives_the_plain_tables_fingerprints(tmp_path):
    nodes, links, trips = hand_tables()
    plain = built(tmp_path, "plain", nodes, links, trips)
    folder = tmp_path / "typed"
    # Strings as LargeUtf8; integers as Int32, UInt32, UInt64; floats as integers.
    n = pa.table({"node_id": pa.array(nodes["node_id"], pa.large_string()),
                  "x_m": pa.array([0, 1_500, 2_500], pa.int32()),
                  "y_m": pa.array([0, 0, 0], pa.uint64())})
    lk = pa.table({**{c: pa.array(links[c], pa.large_string())
                      for c in ("link_id", "upstream", "downstream", "modes")},
                   "length_m": pa.array([1_500, 2_000, 1_000], pa.int64()),
                   "lanes": pa.array(links["lanes"], pa.uint32()),
                   "car_speed_km_h": pa.array([None, 50, 36], pa.uint32()),
                   "capacity_veh_h": pa.array(links["capacity_veh_h"], pa.float64())})
    tr = pa.table({**{c: trips[c] for c in trips if c != "departure_s"},
                   "departure_s": pa.array(trips["departure_s"], pa.int32())})
    files = [write(folder, f"{k}.parquet", t) for k, t in (("n", n), ("l", lk), ("t", tr))]
    assert built(tmp_path, "typed", *files) == plain
    tr = pa.table({**{c: trips[c] for c in trips if c != "departure_s"},
                   "departure_s": pa.array(trips["departure_s"], pa.uint64())})
    files[2] = write(folder, "t64.parquet", tr)
    assert built(tmp_path, "typed64", *files) == plain


def test_snappy_and_no_compression_read_and_zstd_is_refused(tmp_path):
    nodes, links, trips = hand_tables()
    plain = built(tmp_path, "plain", nodes, links, trips)
    for codec, options in (("SNAPPY", {}), ("UNCOMPRESSED", {"compression": "none"})):
        folder = tmp_path / codec
        files = [write(folder, f"{k}.parquet", t, **options)
                 for k, t in (("nodes", nodes), ("links", links), ("trips", trips))]
        assert pq.ParquetFile(files[0]).metadata.row_group(0).column(0).compression == codec
        assert built(tmp_path, codec + "-run", *files) == plain, codec
    zstd = write(tmp_path / "zstd", "trips.parquet", trips, compression="zstd")
    network = oms.Network.from_tables(nodes, links)
    with pytest.raises(ValueError, match="compressed by zstd"):
        oms.Demand.from_table(zstd, ["commute", "leisure"], network)


NODES, LINKS, TRIPS = hand_tables()
REFUSED = {  # case: (the table, its columns, the message)
    "a boolean for a float": (
        "links", lambda: pa.table({**LINKS, "length_m": [True, False, True]}),
        "links: column length_m has type Boolean"),
    "a missing column": (
        "trips", lambda: {k: v for k, v in TRIPS.items() if k != "class_id"},
        "trips: missing column class_id"),
    "a column name twice": (
        "links", lambda: pa.table({k: v for k, v in LINKS.items() if k != "length_m"})
        .append_column("length_m", pa.array(LINKS["length_m"]))
        .append_column("length_m", pa.array(LINKS["length_m"])),
        "the file has two columns length_m"),
}


def from_file(path, what):
    """The network (links) or the demand (trips) built from the file `path`."""
    network = oms.Network.from_tables(NODES, LINKS)
    if what == "trips":
        return oms.Demand.from_table(path, ["commute", "leisure"], network)
    return oms.Network.from_tables(NODES, path)


@pytest.mark.parametrize("case", list(REFUSED))
def test_a_table_file_is_refused_by_name(tmp_path, case):
    what, columns, match = REFUSED[case]
    path = write(tmp_path, f"{what}.parquet", columns())
    with pytest.raises(ValueError, match=match):
        from_file(path, what)


READ_AS_PLAIN = {  # case: (the table, its columns)
    "a string for a float": ("links", lambda: {**LINKS, "length_m": ["1500", "2000", "1000"]}),
    "a Float32": (
        "links", lambda: pa.table({**LINKS, "length_m": pa.array(LINKS["length_m"], pa.float32())})),
    "a float for an integer": (
        "trips", lambda: {**TRIPS, "departure_s": [float(d) for d in TRIPS["departure_s"]]}),
}


@pytest.mark.parametrize("case", list(READ_AS_PLAIN))
def test_a_number_of_another_type_reads_as_the_plain_table(tmp_path, case):
    what, columns = READ_AS_PLAIN[case]
    tables = dict(zip(("nodes", "links", "trips"), hand_tables()))
    plain = built(tmp_path, "plain", *tables.values())
    tables[what] = write(tmp_path, f"{what}.parquet", columns())
    assert built(tmp_path, "typed", *tables.values()) == plain


REPAIRED = {  # case: (the table, its columns, the repair's key, its first ids, the counts left)
    "300 lanes": ("links", lambda: {**LINKS, "lanes": [None, 300, 1]},
                  "links_unread", ["st1 (lanes)"], {"links": 2, "trips": 7}),
    "a UInt64 above 2^63 - 1": (
        "trips", lambda: pa.table({**TRIPS, "departure_s": pa.array(
            [2**63] + TRIPS["departure_s"][1:], pa.uint64())}),
        "trips_unread", ["car1 (departure_s)"], {"links": 3, "trips": 6}),
    "an extra column": ("links", lambda: {**LINKS, "speed": [30.0] * 3},
                        "columns_ignored", ["links: speed"], {"links": 3, "trips": 7}),
    "a null departure": (
        "trips", lambda: {**TRIPS, "departure_s": [None] + TRIPS["departure_s"][1:]},
        "trips_unread", ["car1 (departure_s)"], {"links": 3, "trips": 6}),
    "a null mode set": ("links", lambda: {**LINKS, "modes": ["bike;walk", None, "car"]},
                        "links_unread", ["st1 (modes)"], {"links": 2, "trips": 7}),
}


@pytest.mark.parametrize("case", list(REPAIRED))
def test_a_cell_that_cannot_be_read_leaves_its_row_out_counted_and_warned(tmp_path, case):
    what, columns, key, first, left = REPAIRED[case]
    tables = dict(zip(("nodes", "links", "trips"), hand_tables()))
    tables[what] = write(tmp_path, f"{what}.parquet", columns())
    entry = "Demand.from_table" if what == "trips" else "Network.from_tables"
    with pytest.warns(UserWarning) as caught:
        network = oms.Network.from_tables(tables["nodes"], tables["links"])
        demand = oms.Demand.from_table(tables["trips"], ["commute", "leisure"], network)
    shown = [str(w.message) for w in caught if str(w.message).startswith(entry)]
    assert len(shown) == 1 and shown[0].startswith(f"{entry}: 1 ")
    assert shown[0].endswith(f"({key}; first: {', '.join(first)})")
    assert (network if what == "links" else demand).repairs == {key: 1}
    assert demand.trips_given == 7
    result = oms.run(network, demand, window=(HAND_START, HAND_START + 3_600),
                     out=tmp_path / "run", iteration_cap=1)
    with open(result.record, "rb") as f:
        record = tomllib.load(f)
    assert {"links": record["network"]["links"], "trips": record["demand"]["trips"]} == left
    # trips_given is written only when the demand dropped some: else it is the trips.
    assert record["demand"].get("trips_given", record["demand"]["trips"]) == 7


def test_a_node_row_that_cannot_be_read_leaves_the_node_and_its_links_out():
    # Node b's x_m null: b left out; ab and st1 run a->b, st2 b->c: all three name b.
    nodes = {**NODES, "x_m": [0.0, None, 2_500.0]}
    with pytest.warns(UserWarning, match=re.escape("(nodes_unread; first: b (x_m))")):
        network = oms.Network.from_tables(nodes, LINKS)
    assert network.repairs == {"nodes_unread": 1, "links_to_no_node": 3}


def test_a_column_of_nulls_alone_where_none_is_allowed_leaves_every_row_out(tmp_path):
    path = write(tmp_path, "links.parquet", pa.table({**LINKS, "modes": pa.nulls(3)}))
    with pytest.warns(UserWarning, match=r"Network.from_tables: 3 .*\(links_unread; first: ab \(modes\), "
                                         r"st1 \(modes\), st2 \(modes\)\)"):
        network = oms.Network.from_tables(NODES, path)
    assert network.repairs == {"links_unread": 3}


def test_the_same_column_name_twice_is_refused_in_memory_too():
    twice = REFUSED["a column name twice"][1]()
    with pytest.raises(KeyError, match="length_m"):  # pyarrow's refusal of the name
        oms.Network.from_tables(NODES, twice)


def test_a_file_not_parquet_is_a_value_error_and_one_not_read_an_os_error(tmp_path):
    network = oms.Network.from_tables(NODES, LINKS)
    (tmp_path / "not.parquet").write_text("trip_id\nride\n")
    with pytest.raises(ValueError, match="not a readable Parquet file"):
        oms.Demand.from_table(tmp_path / "not.parquet", ["commute", "leisure"], network)
    locked = write(tmp_path, "locked.parquet", TRIPS)
    locked.chmod(0)
    for path in (tmp_path / "absent.parquet", tmp_path, "", locked):
        if path == locked and os.access(locked, os.R_OK):
            continue  # a user who reads every file (root) has no unreadable file
        with pytest.raises(OSError):
            oms.Demand.from_table(path, ["commute", "leisure"], network)


def test_a_column_of_nulls_alone_reads_as_the_same_table_in_memory(tmp_path):
    # pyarrow types a column holding only nulls as `null`; the optional car fields of a links
    # table without cars are such columns. In memory and from a file: the same fingerprint.
    links = pa.table({"link_id": ["ab", "ba"], "upstream": ["a", "b"], "downstream": ["b", "a"],
                      "modes": ["bike;walk"] * 2, "length_m": [1_500.0] * 2,
                      "lanes": [None, None], "car_speed_km_h": [None, None],
                      "capacity_veh_h": [None, None]})
    assert links.schema.field("capacity_veh_h").type == pa.null()
    nodes = {"node_id": ["a", "b"], "x_m": [0.0, 1_500.0], "y_m": [0.0, 0.0]}
    trips = table(TRIP_COLUMNS, [("ride", 0.0, 0.0, 1_500.0, 0.0, START, "all", "bike")])
    prints = []
    for name, link_table in (("memory", links), ("file", write(tmp_path, "links.parquet", links))):
        network = oms.Network.from_tables(nodes, link_table)
        demand = oms.Demand.from_table(trips, ["all"], network)
        result = oms.run(network, demand, window=(START, END), out=tmp_path / name)
        assert pq.read_table(result.trips).column("travel_time_s").to_pylist() == [360.0]
        prints.append(fingerprints(result.record))
    assert prints[0] == prints[1]


def test_files_of_several_row_groups_and_batches_read_every_row(tmp_path):
    # A 33 x 33 grid: 1 089 nodes and 2 x 2 x 33 x 32 = 4 224 links (a street each way between
    # neighbours); 1 100 trips; pyarrow writes row groups of 300 rows, and the reader's batches
    # hold 1 024 rows, so each table spans several of both.
    nodes, links, pos, _ = grid(33, seed=8)
    ids = list(pos)
    rows = [(f"t{i:04d}", *pos[ids[i % 1_089]], *pos[ids[(i * 7 + 1) % 1_089]], START, "all",
             "walk") for i in range(1_100)]
    trips = table(TRIP_COLUMNS, rows)
    files = [write(tmp_path / "in", f"{k}.parquet", t, row_group_size=300)
             for k, t in (("nodes", nodes), ("links", links), ("trips", trips))]
    assert all(pq.ParquetFile(f).metadata.num_row_groups > 3 for f in files)
    records = []
    for name, (n, lk, tr) in (("memory", (nodes, links, trips)), ("files", files)):
        network = oms.Network.from_tables(n, lk)
        demand = oms.Demand.from_table(tr, ["all"], network)
        result = oms.run(network, demand, window=(START, END), out=tmp_path / name,
                         iteration_cap=1)
        with open(result.record, "rb") as f:
            records.append(tomllib.load(f))
    memory, files = records
    assert (files["network"]["nodes"], files["network"]["links"], files["demand"]["trips"]) == \
        (1_089, 4_224, 1_100)
    assert fingerprints_of(files) == fingerprints_of(memory)


def fingerprints_of(record):
    return record["network"]["fingerprint"], record["demand"]["fingerprint"]
