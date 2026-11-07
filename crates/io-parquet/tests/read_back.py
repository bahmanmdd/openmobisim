"""The hand run's three Parquet files, read by another tool (pyarrow 26.0.1), checked against the
hand numbers written below: the column names in order, their Arrow types, their nullability,
every value, and no NaN anywhere. Exits 1 on any difference, naming it.

    cargo test -p openmobisim-io-parquet --test hand_run
    python3 crates/io-parquet/tests/read_back.py target/tmp/io-parquet-hand-run

CI runs it with pyarrow 26.0.1 in a virtual environment. The derivations are in the header of
`hand_run.rs`: a path a->b of 1 500 m (a bike at 15 km/h in 360 s, a walker at 4.8 km/h in
1 125 s); streets a->b (144 s, a car every 2 s) and b->c (100 s, a car every 4 s); the point
queue's exit max(t + free flow, previous exit + headway); bins of 60 s; three iterations.
"""

import math
import sys
from pathlib import Path

import pyarrow as pa
import pyarrow.compute as pc
import pyarrow.parquet as pq

N = None  # null
STR, U32, U64, F64, BOOL = pa.string(), pa.uint32(), pa.uint64(), pa.float64(), pa.bool_()

# (name, Arrow type, nullable), in the file's order.
TRIPS_SCHEMA = [
    ("trip_id", STR, False), ("class_id", STR, False), ("departure_s", U32, False),
    ("mode", STR, True), ("arrival_s", F64, True), ("travel_time_s", F64, True),
    ("finished", BOOL, False), ("cost", F64, True), ("alternatives", U32, False),
]
LINKS_SCHEMA = [
    ("link_id", STR, False), ("bin_start_s", U32, False), ("cars", U32, False),
    ("car_time_s", F64, True), ("first_exit_s", F64, True), ("last_exit_s", F64, True),
    ("bikes", U32, False), ("walkers", U32, False),
]
STAGES = ["search_s", "choose_s", "load_s", "measure_s", "assess_s"]
ITERATIONS_SCHEMA = [
    ("iteration", U32, False), ("mean_trip_time_s", F64, True), ("finished", U64, False),
    ("unfinished", U64, False), ("without_alternative", U64, False),
    ("switch_share", F64, True), ("found_share", F64, True), ("gap", F64, True),
] + [(s, F64, False) for s in STAGES]

# Trips in id order (bytewise): id, class, departure, mode, arrival, travel time, finished,
# cost, alternatives. Cars: st1 exits 144, 146, 148; st2 exits 244, 248, 252.
TRIPS = [
    ("bike", "leisure", 0, "bike", 360.0, 360.0, True, 360.0, 1),
    ("car1", "commute", 0, "car", 244.0, 244.0, True, 244.0, 1),
    ("car2", "commute", 1, "car", 248.0, 247.0, True, 247.0, 1),
    ("car3", "commute", 2, "car", 252.0, 250.0, True, 250.0, 1),
    ("lost", "leisure", 0, N, N, N, False, N, 0),
    ("walk", "leisure", 0, "walk", 1125.0, 1125.0, True, 1125.0, 1),
    ("walk-late", "leisure", 3000, "walk", N, N, False, 1125.0, 1),
]
# Links in id order then bin: id, bin start, cars, their summed crossing time, exit less entry
# (st1: 144 + 145 + 146 = 435; st2: 100 + 102 + 104 = 306), first exit, last exit, bikes,
# walkers. st2's cars enter at 144 to 148 s, the bin from 120 s.
LINKS = [
    ("ab", 0, 0, N, N, N, 1, 1),
    ("ab", 3000, 0, N, N, N, 0, 1),
    ("st1", 0, 3, 435.0, 144.0, 148.0, 0, 0),
    ("st2", 120, 3, 306.0, 244.0, 252.0, 0, 0),
]
# Iterations: mean over the five finished (360 + 244 + 247 + 250 + 1 125) / 5 = 445.2 s;
# 5 finished, 1 unfinished, 1 without an alternative; nobody switches; at iteration 0 all six
# with an alternative found their first, after it nothing new; each takes its set's least: gap 0.
ITERATIONS = [
    (k, 445.2, 5, 1, 1, 0.0, found, 0.0) for k, found in [(0, 1.0), (1, 0.0), (2, 0.0)]
]


def same(got, want):
    """Equal, or both null, or floats within 1e-9 s."""
    if want is None or got is None:
        return got is None and want is None
    if isinstance(want, float):
        return isinstance(got, float) and math.isclose(got, want, rel_tol=0.0, abs_tol=1e-9)
    return type(got) is type(want) and got == want


def check(folder, name, schema, rows, errors):
    table = pq.read_table(folder / name)
    got = [(f.name, f.type, f.nullable) for f in table.schema]
    if got != schema:
        errors.append(f"{name}: schema {got} is not {schema}")
        return
    for column in table.column_names:
        if pa.types.is_floating(table.schema.field(column).type):
            if pc.any(pc.is_nan(table[column])).as_py():
                errors.append(f"{name}: {column} holds a NaN")
    if table.num_rows != len(rows):
        errors.append(f"{name}: {table.num_rows} rows, not {len(rows)}")
        return
    names = [s[0] for s in schema]
    for i, (have, want) in enumerate(zip(table.to_pylist(), rows)):
        for column, value in zip(names, want):
            if not same(have[column], value):
                errors.append(f"{name}: row {i}, {column} is {have[column]!r}, not {value!r}")
        for column in names[len(want):]:  # the stage seconds: timed, so only >= 0
            if not (have[column] is not None and have[column] >= 0.0):
                errors.append(f"{name}: row {i}, {column} is {have[column]!r}, not >= 0")


def main():
    folder = Path(sys.argv[1])
    errors = []
    check(folder, "trips.parquet", TRIPS_SCHEMA, TRIPS, errors)
    check(folder, "links.parquet", LINKS_SCHEMA, LINKS, errors)
    check(folder, "iterations.parquet", ITERATIONS_SCHEMA, ITERATIONS, errors)
    for e in errors:
        print(e)
    print(f"pyarrow {pa.__version__}: {'FAILED' if errors else 'the three files match the hand run'}")
    sys.exit(1 if errors else 0)


if __name__ == "__main__":
    main()
