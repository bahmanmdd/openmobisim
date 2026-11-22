"""The hand run's record, run_record.toml, read by another tool (Python's standard tomllib, TOML 1.0),
checked against the numbers written below: every key in its order, every value and its type
(a float read as a float, an integer as an integer). The numbers are the keywords the hand run
was given (hand_run.rs: a deterministic run of three iterations in bins of 60 s, the cost on
time alone for both classes, the build values 1 800 veh/h per lane, 15 km/h and 4.8 km/h), the
defaults of PARAMETERS.md for every other row, and the hand counts
of hand_run.rs (3 nodes, 3 links, 7 trips in the classes commute and leisure). The version is
the workspace's, read from Cargo.toml. The run has no transit, so stop_walk_max and
transfer_walk_max, which only transit reads, are absent. Its tables were given in memory, so
[network] holds no file and [demand] only the classes; part two, [result], holds the version,
the end used (the window is a whole number of bins: the end given), how the run ended and the
inputs' counts and fingerprints. Exits 1 on any difference, naming it.

    cargo test -p openmobisim-io-parquet --test hand_run
    python3 crates/io-parquet/tests/read_record.py target/tmp/io-parquet-hand-run

For bikes: bike_speed is the bike's one speed on every link, 15 km/h, a row of [parameters].
"""

import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
HEX16 = re.compile(r"[0-9a-f]{16}")
# The columns of the choice row, in order (crates/loop/README.md).
COLUMNS = [
    "car_drive_min", "bike_ride_min", "walk_min", "transit_in_vehicle_min", "transit_access_min",
    "transit_egress_min", "transit_first_wait_min", "transit_transfer_wait_min",
    "transit_transfer_walk_min", "transit_transfers", "money_eur", "car_ln_path_size", "is_car",
    "is_bike", "is_walk", "is_transit",
]
# Time alone, as the hand run gives both classes: 1 per minute on the nine times, 0 on the rest.
TIME_ONLY = [1.0] * 9 + [0.0] * 7
PARAMETERS = {
    # Given by the hand run.
    "capacity_per_lane": 1_800.0, "bike_speed": 15.0, "walk_speed": 4.8,
    # PARAMETERS.md's defaults (stop_walk_max and transfer_walk_max are absent: no transit).
    "boarding_slack": 60, "max_rides": 4, "nearby_stops": 3,
    # Given.
    "choice": "deterministic",
    # Default.
    "nest_scale": 0.5,
    "step_exponent": 0.75,
    # Given.
    "iteration_cap": 3,
    # Default.
    "iteration_floor": 0,
    # Given.
    "link_bin": 60,
    # Default.
    "seed": 0,
    "coefficients": {"commute": dict(zip(COLUMNS, TIME_ONLY)),
                     "leisure": dict(zip(COLUMNS, TIME_ONLY))},
}


def version():
    with open(ROOT / "Cargo.toml", "rb") as f:
        return tomllib.load(f)["workspace"]["package"]["version"]


def same(path, got, want, errors):
    """`got` equals `want` with the same types, recursively; keys in the same order."""
    if isinstance(want, dict):
        if not isinstance(got, dict) or list(got) != list(want):
            keys = list(got) if isinstance(got, dict) else got
            errors.append(f"{path}: keys {keys}, not {list(want)}")
            return
        for k in want:
            same(f"{path}.{k}", got[k], want[k], errors)
    elif type(got) is not type(want) or got != want:
        errors.append(f"{path}: {got!r} ({type(got).__name__}), not {want!r}")


def main():
    path = Path(sys.argv[1]) / "run_record.toml"
    text = path.read_text(encoding="utf-8")
    errors = []
    if not text.startswith("# "):
        errors.append("run_record.toml: the first line is not a comment saying what the file is")
    record = tomllib.loads(text)
    found = record.get("result", {})
    prints = {t: found.get(t, {}).get("fingerprint") for t in ("network", "demand")}
    for table, value in prints.items():
        if not (isinstance(value, str) and HEX16.fullmatch(value)):
            errors.append(f"result.{table}.fingerprint: {value!r} is not 16 lower-case hexadecimal digits")
    want = {
        "format": 1,
        "window_start_s": 25_200,
        "window_end_s": 28_800,
        "parameters": PARAMETERS,
        "network": {},
        "demand": {"classes": ["commute", "leisure"]},
        "result": {
            "openmobisim_version": version(),
            "window_end_used_s": 28_800,
            "iterations": 3,
            "stopped_by_rule": False,
            "network": {"source": "tables", "nodes": 3, "links": 3, "fingerprint": prints["network"]},
            "demand": {"trips": 7, "fingerprint": prints["demand"]},
        },
    }
    same("run_record.toml", record, want, errors)
    for e in errors:
        print(e)
    print(f"tomllib: {'FAILED' if errors else 'the record matches the hand run'}")
    sys.exit(1 if errors else 0)


if __name__ == "__main__":
    main()
