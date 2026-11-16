"""The hand run of crates/io-parquet, rebuilt from Python tables through the binding, and its three
files checked by that crate's own reader script, unchanged: read_back.py reads them with pyarrow
26.0.1 and compares every column against the hand numbers written in it.

The run (derived by hand in crates/io-parquet/tests/hand_run.rs and read_back.py): nodes a (0, 0),
b (1 500, 0), c (2 500, 0) metres; a path ab a->b of 1 500 m for bikes (15 km/h: 360 s) and
walkers (4.8 km/h: 1 125 s); a street st1 a->b of 2 000 m at 50 km/h, 1 lane, 1 800 veh/h; a
street st2 b->c of 1 000 m at 36 km/h, 1 lane, 900 veh/h. Seven trips: three cars a->c at 0, 1,
2 s after 07:00:00 (25 200 s of the clock), a bike and a walker a->b at 25 200 s, a walker at
28 200 s still walking at the window's end, a transit-only trip with no timetable. The window
25 200 s to 28 800 s in bins of 60 s, three iterations, the least cost on time alone (1 on each
of the nine time columns, in minutes, 0 on the other seven: a cost in minutes). For a
bike: the bike's row is its own trip at 360 s and its own count in the links table.
"""

import subprocess
import sys
from pathlib import Path

import openmobisim as oms

# Time alone: 1 on the nine time columns of the choice row (minutes), 0 on transfers, money,
# path size and the four mode constants.
TIME_ONLY = (1.0,) * 9 + (0.0,) * 7
READ_BACK = Path(__file__).resolve().parents[2] / "io-parquet" / "tests" / "read_back.py"
START = 25_200


def hand_tables():
    """The hand run's nodes, links and trips tables, dicts of lists."""
    nodes = {"node_id": ["a", "b", "c"], "x_m": [0.0, 1_500.0, 2_500.0], "y_m": [0.0, 0.0, 0.0]}
    links = {
        "link_id": ["ab", "st1", "st2"],
        "upstream": ["a", "a", "b"],
        "downstream": ["b", "b", "c"],
        "modes": ["bike;walk", "car", "car"],
        "length_m": [1_500.0, 2_000.0, 1_000.0],
        "lanes": [None, 1, 1],
        "car_speed_km_h": [None, 50.0, 36.0],
        "capacity_veh_h": [None, 1_800.0, 900.0],
    }
    rows = [  # id, to x (m), departure (s of the clock), class, modes
        ("car1", 2_500.0, START, "commute", "car"),
        ("car2", 2_500.0, START + 1, "commute", "car"),
        ("car3", 2_500.0, START + 2, "commute", "car"),
        ("bike", 1_500.0, START, "leisure", "bike"),
        ("walk", 1_500.0, START, "leisure", "walk"),
        ("walk-late", 1_500.0, START + 3_000, "leisure", "walk"),
        ("lost", 1_500.0, START, "leisure", "transit"),
    ]
    trips = {
        "trip_id": [r[0] for r in rows],
        "origin_x_m": [0.0] * len(rows),
        "origin_y_m": [0.0] * len(rows),
        "destination_x_m": [r[1] for r in rows],
        "destination_y_m": [0.0] * len(rows),
        "departure_s": [r[2] for r in rows],
        "class_id": [r[3] for r in rows],
        "modes": [r[4] for r in rows],
    }
    return nodes, links, trips


def hand_run(nodes, links, trips, out):
    """The hand run on the three tables given (in memory or files), written into `out`."""
    network = oms.Network.from_tables(
        nodes, links, capacity_per_lane=1_800.0, bike_speed=15.0, walk_speed=4.8
    )
    demand = oms.Demand.from_table(trips, ["commute", "leisure"], network)
    return oms.run(
        network, demand, window=(START, START + 3_600), out=out,
        link_bin=60, iteration_cap=3, choice="deterministic",
        coefficients={"commute": TIME_ONLY, "leisure": TIME_ONLY},
    )


def read_back(folder):
    """read_back.py on `folder`: its output, and whether it passed."""
    done = subprocess.run(
        [sys.executable, str(READ_BACK), str(folder)], capture_output=True, text=True
    )
    return done.stdout + done.stderr, done.returncode == 0


def test_the_hand_run_through_python_passes_read_back(tmp_path):
    result = hand_run(*hand_tables(), tmp_path)
    assert [Path(p).name for p in (result.trips, result.links, result.iterations)] == [
        "trips.parquet", "links.parquet", "iterations.parquet"
    ]
    assert all(Path(p).parent == tmp_path for p in (result.trips, result.links, result.iterations))
    # The rule reads the last five iterations' means, so three iterations end at the cap.
    assert result.stopped_by_rule is False
    assert result.osm_report is None and result.gtfs_report is None
    output, passed = read_back(tmp_path)
    print(output)
    assert passed, output
