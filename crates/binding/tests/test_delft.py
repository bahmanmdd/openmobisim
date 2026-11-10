"""Delft through Python: the OpenStreetMap extract of crates/io-osm and the GTFS feed of crates/io-gtfs
read by Network.from_osm and Transit.from_gtfs at the default parameters, then one run, and the
GTFS reader's counts against crates/io-gtfs/tests/fixtures/delft/expected.csv, written by
gtfs-kit 14.1.1 (crates/io-gtfs/tests/delft_gtfs.py): the busiest weekday, its trips, runs,
calls and night runs, and the stops. The OSM reader's counts have no answer from another tool
(crates/io-osm checks 20 ways against libosmium and pyproj through the network's accessors,
which the binding does not expose); here they are only returned. The read times are printed.

A run is the one way the binding returns the readers' counts: one trip of two minutes past
08:00:00 from Delft's station to the Markt, open to walking and transit, one iteration.
For bikes: the network read here is the one whose 80 504 bike links crates/io-osm describes.
"""

import csv
import time
from pathlib import Path

import openmobisim as oms

CRATES = Path(__file__).resolve().parents[2]
OSM = CRATES / "io-osm" / "tests" / "fixtures" / "delft" / "network.osm.pbf"
GTFS = CRATES / "io-gtfs" / "tests" / "fixtures" / "delft" / "gtfs.zip"
EXPECTED = CRATES / "io-gtfs" / "tests" / "fixtures" / "delft" / "expected.csv"


def test_delft_reads_and_its_feed_counts_match_gtfs_kit(tmp_path):
    started = time.perf_counter()
    network = oms.Network.from_osm(OSM)
    osm_s = time.perf_counter() - started
    started = time.perf_counter()
    transit = oms.Transit.from_gtfs(GTFS, network)
    gtfs_s = time.perf_counter() - started
    print(f"Delft through Python: network read in {osm_s:.2f} s, feed and footpaths in "
          f"{gtfs_s:.2f} s")

    trips = {
        "trip_id": ["station-markt"],
        "origin_lon_deg": [4.3566], "origin_lat_deg": [52.0067],
        "destination_lon_deg": [4.3584], "destination_lat_deg": [52.0116],
        "departure_s": [28_920], "class_id": ["all"], "modes": ["walk;transit"],
    }
    demand = oms.Demand.from_table(trips, ["all"], network, transit=transit)
    result = oms.run(network, demand, transit, window=(28_800, 32_400), out=tmp_path,
                     iteration_cap=1)
    print(f"OSM reader: {result.osm_report}")
    print(f"GTFS reader: {result.gtfs_report}")

    with EXPECTED.open() as f:
        want = {row["key"]: int(row["value"]) for row in csv.DictReader(f)}
    assert set(want) == {"date", "trips", "runs", "calls", "night_runs", "stops"}
    for key, value in want.items():
        assert result.gtfs_report[key] == value, (key, result.gtfs_report[key], value)
    assert result.osm_report["nodes"] > 0 and result.osm_report["links"] > 0
