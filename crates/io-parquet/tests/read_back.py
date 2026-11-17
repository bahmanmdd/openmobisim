"""The hand run's four Parquet files, read by another tool (pyarrow 26.0.1), checked against the
hand numbers written below: the column names in order, their Arrow types, their nullability,
every value, no NaN anywhere, and every finished trip's arrival_s equal to its departure_s plus
its travel_time_s. Every time of day is in seconds of the clock; the window starts at 07:00:00
(25 200 s), so a time after the window's start would differ by 25 200. Exits 1 on any
difference, naming it.

    cargo test -p openmobisim-io-parquet --test hand_run
    python3 crates/io-parquet/tests/read_back.py target/tmp/io-parquet-hand-run

CI runs it with pyarrow 26.0.1 in a virtual environment. The derivations are in the header of
`hand_run.rs`: a path a->b of 1 500 m (a bike at 15 km/h in 360 s, a walker at 4.8 km/h in
1 125 s); streets a->b (144 s, a car every 2 s) and b->c (100 s, a car every 4 s); the point
queue's exit max(t + free flow, previous exit + headway); a window of 3 600 s from 25 200 s in
bins of 60 s; three iterations.
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
    ("without_transit", U64, False), ("without_stop", U64, False),
    ("offered_share", F64, True), ("switch_share", F64, True), ("found_share", F64, True),
    ("gap", F64, True),
] + [(s, F64, False) for s in STAGES]
ROUTES_SCHEMA = [
    ("trip_id", STR, False), ("leg", U32, False), ("kind", STR, False), ("seconds", F64, True),
    ("links", pa.list_(STR), True), ("run_id", STR, True), ("board_stop", STR, True),
    ("alight_stop", STR, True), ("board_s", U32, True), ("alight_s", U32, True),
    ("from_stop", STR, True), ("to_stop", STR, True),
]

# Every time of day is in seconds of the clock: the window starts at 07:00:00, S = 25 200 s, and
# each instant is S plus the seconds after the start derived in `hand_run.rs`.
# Trips in id order (bytewise): id, class, departure, mode, arrival, travel time, finished,
# cost, alternatives. The cost is in minutes: the time-only run weighs each time column, in
# minutes, at 1, so a cost is the door-to-door seconds over 60. Departures S + 0, 1, 2 (cars),
# S + 3 000 = 28 200 (walk-late), S (the rest). Arrivals: bike S + 360 = 25 560; cars, their
# st2 exits S + 244, 248, 252 = 25 444, 25 448, 25 452; walk S + 1 125 = 26 325.
TRIPS = [
    ("bike", "leisure", 25200, "bike", 25560.0, 360.0, True, 360.0 / 60, 1),
    ("car1", "commute", 25200, "car", 25444.0, 244.0, True, 244.0 / 60, 1),
    ("car2", "commute", 25201, "car", 25448.0, 247.0, True, 247.0 / 60, 1),
    ("car3", "commute", 25202, "car", 25452.0, 250.0, True, 250.0 / 60, 1),
    ("lost", "leisure", 25200, N, N, N, False, N, 0),
    ("walk", "leisure", 25200, "walk", 26325.0, 1125.0, True, 1125.0 / 60, 1),
    ("walk-late", "leisure", 28200, "walk", N, N, False, 1125.0 / 60, 1),
]
# Links in id order then bin: id, bin start, cars, their summed crossing time, exit less entry
# (st1: 144 + 145 + 146 = 435; st2: 100 + 102 + 104 = 306), first exit, last exit, bikes,
# walkers. Bins from S + 0 = 25 200 and S + 3 000 = 28 200 (ab), S + 0 = 25 200 (st1), and
# S + 120 = 25 320 (st2, its cars entering at S + 144 to S + 148). Exits: st1 S + 144 = 25 344
# and S + 148 = 25 348; st2 S + 244 = 25 444 and S + 252 = 25 452.
LINKS = [
    ("ab", 25200, 0, N, N, N, 1, 1),
    ("ab", 28200, 0, N, N, N, 0, 1),
    ("st1", 25200, 3, 435.0, 25344.0, 25348.0, 0, 0),
    ("st2", 25320, 3, 306.0, 25444.0, 25452.0, 0, 0),
]
# The finished trips, for each of which the file's arrival must be its departure plus its travel
# time exactly, on one clock (by hand: 25 200 + 360 = 25 560, 25 200 + 244 = 25 444,
# 25 201 + 247 = 25 448, 25 202 + 250 = 25 452, 25 200 + 1 125 = 26 325).
FINISHED = ["bike", "car1", "car2", "car3", "walk"]
# Iterations: mean over the five finished (360 + 244 + 247 + 250 + 1 125) / 5 = 445.2 s;
# 5 finished, 1 unfinished, 1 without an alternative; at iteration 0 all six with an
# alternative choose (offered 1), after it each is offered with chance 1 / (k + 1) by its own
# draw, so the share offered is one of 0, 1/6, ..., 1 (SIXTHS: any of them); nobody switches;
# at iteration 0 all six found their first, after it nothing new; each takes its set's least:
# gap 0. `lost` is the one trip with transit open to it: no journey offered (1), no stop in
# reach (1).
SIXTHS = [i / 6 for i in range(7)]
ITERATIONS = [
    (k, 445.2, 5, 1, 1, 1, 1, offered, 0.0, found, 0.0)
    for k, offered, found in [(0, 1.0, 1.0), (1, SIXTHS, 0.0), (2, SIXTHS, 0.0)]
]


# Routes: per trip with an alternative, in id order, what it took: a walk gap of 0 s (every door
# is at a node), its route's links in order, a walk gap of 0 s. The cars a->c by st1 then st2; the
# bike, the walker and the late walker a->b by the path ab; `lost` (no alternative) no row. A
# route's seconds are null (the loop keeps the trip's time, not the route's); no ride, no stops.
# A route's links are a tuple below: the list read must be exactly those, in order.
def legs(trip, kind, links):
    gap = lambda k: (trip, k, "walk_gap", 0.0, N, N, N, N, N, N, N, N)
    return [gap(0), (trip, 1, kind, N, links, N, N, N, N, N, N, N), gap(2)]


ROUTES = (legs("bike", "bike", ("ab",)) + legs("car1", "car", ("st1", "st2"))
          + legs("car2", "car", ("st1", "st2")) + legs("car3", "car", ("st1", "st2"))
          + legs("walk", "walk", ("ab",)) + legs("walk-late", "walk", ("ab",)))


def same(got, want):
    """Equal, or both null, or floats within 1e-9 s; a list of wants: equal to any of them; a
    tuple: a list of exactly those (the links of a route)."""
    if isinstance(want, tuple):
        return isinstance(got, list) and tuple(got) == want
    if isinstance(want, list):
        return any(same(got, w) for w in want)
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


def check_arrivals(folder, errors):
    """The finished trips are the hand run's, and each one's arrival_s is its departure_s plus
    its travel_time_s, exactly: departure and arrival on the same clock."""
    rows = [r for r in pq.read_table(folder / "trips.parquet").to_pylist() if r["finished"]]
    ids = [r["trip_id"] for r in rows]
    if ids != FINISHED:
        errors.append(f"trips.parquet: finished trips {ids}, not {FINISHED}")
    for r in rows:
        a, d, t = r["arrival_s"], r["departure_s"], r["travel_time_s"]
        if a is None or t is None or a != d + t:
            errors.append(
                f"trips.parquet: {r['trip_id']} arrives at {a!r}, not departure {d} + travel "
                f"time {t!r}"
            )


def main():
    folder = Path(sys.argv[1])
    errors = []
    check(folder, "trips.parquet", TRIPS_SCHEMA, TRIPS, errors)
    check_arrivals(folder, errors)
    check(folder, "links.parquet", LINKS_SCHEMA, LINKS, errors)
    check(folder, "iterations.parquet", ITERATIONS_SCHEMA, ITERATIONS, errors)
    check(folder, "routes.parquet", ROUTES_SCHEMA, ROUTES, errors)
    for e in errors:
        print(e)
    print(f"pyarrow {pa.__version__}: {'FAILED' if errors else 'the four files match the hand run'}")
    sys.exit(1 if errors else 0)


if __name__ == "__main__":
    main()
