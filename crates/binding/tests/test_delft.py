"""Delft through Python: the OpenStreetMap extract of crates/io-osm and the GTFS feed of crates/io-gtfs
read by Network.from_osm and Transit.from_gtfs at the default parameters, then one run, and the
GTFS reader's counts against crates/io-gtfs/tests/fixtures/delft/expected.csv, written by
gtfs-kit 14.1.1 (crates/io-gtfs/tests/delft_gtfs.py): the busiest weekday, its trips, runs,
calls and night runs, and the stops. The OSM reader's counts have no answer from another tool
(crates/io-osm checks 20 ways against libosmium and pyproj, and the network of the region below
against version 1's tables, through the network's accessors, which the binding does not
expose); here they are only returned. The read times are printed.

A run is the one way the binding returns the readers' counts: one trip of two minutes past
08:00:00 from Delft's station to the Markt, open to walking and transit, one iteration.

The region: the extract read with region=(west, south, east, north), version 1's Delft box
(4.315, 51.965, 4.410, 52.035), and with the same box as a list of its four corners, a polygon:
the same network (one fingerprint, as no node lies on its edge), and a smaller one than the whole
extract. The same polygon given as lists ([[lon, lat], ...], as a record's region reads back from
TOML) is accepted; the record of a run cut to it holds those four corners, and both run_file on
that record and Run.open of it read the network again: the same link count as the run's (a
property: one extract, one region, one network). A region that is no area, or not numbers, is
refused by name (ValueError): by hand, three points of which two are the same, three on the
parallel 52.0°N, and three on one diagonal (4.30, 52.00), (4.35, 52.05), (4.40, 52.10) enclose no
area; a figure eight of four points (two triangles meeting at the box's centre, its signed area
0) does, and is read.
For bikes: the bike links are a layer of their own, read with the network (crates/io-osm).
"""

import csv
import time
import tomllib
from pathlib import Path

import pyarrow as pa
import pyarrow.parquet as pq
import pytest

import openmobisim as oms

CRATES = Path(__file__).resolve().parents[2]
OSM = CRATES / "io-osm" / "tests" / "fixtures" / "delft" / "network.osm.pbf"
GTFS = CRATES / "io-gtfs" / "tests" / "fixtures" / "delft" / "gtfs.zip"
EXPECTED = CRATES / "io-gtfs" / "tests" / "fixtures" / "delft" / "expected.csv"
REGION = (4.315, 51.965, 4.410, 52.035)


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


def test_delft_cut_to_a_box_or_the_same_polygon_is_one_network(tmp_path):
    started = time.perf_counter()
    box = oms.Network.from_osm(OSM, region=REGION)
    box_s = time.perf_counter() - started
    w, s, e, n = REGION
    polygon = oms.Network.from_osm(OSM, region=[(w, s), (e, s), (e, n), (w, n)])
    whole = oms.Network.from_osm(OSM)
    print(f"Delft cut to its box: {box_s:.2f} s")
    trips = {"trip_id": ["station-markt"], "origin_lon_deg": [4.3566], "origin_lat_deg": [52.0067],
             "destination_lon_deg": [4.3584], "destination_lat_deg": [52.0116],
             "departure_s": [28_920], "class_id": ["all"], "modes": ["walk"]}
    read = []
    for name, network in (("box", box), ("polygon", polygon), ("whole", whole)):
        demand = oms.Demand.from_table(trips, ["all"], network)
        result = oms.run(network, demand, window=(28_800, 32_400), out=tmp_path / name,
                         iteration_cap=1)
        with open(result.record, "rb") as f:
            read.append((result.osm_report, tomllib.load(f)["network"]["fingerprint"]))
    assert read[0] == read[1], "the box and the same polygon: one network"
    assert read[0][0]["links"] < read[2][0]["links"]


def test_a_polygon_region_given_as_lists_runs_again_from_its_record(tmp_path):
    w, s, e, n = REGION
    corners = [[w, s], [e, s], [e, n], [w, n]]
    trips = {"trip_id": ["station-markt"], "origin_lon_deg": [4.3566], "origin_lat_deg": [52.0067],
             "destination_lon_deg": [4.3584], "destination_lat_deg": [52.0116],
             "departure_s": [28_920], "class_id": ["all"], "modes": ["bike"]}
    trips_file = tmp_path / "trips.parquet"
    pq.write_table(pa.table(trips), trips_file)
    started = time.perf_counter()
    network = oms.Network.from_osm(OSM, region=corners)
    demand = oms.Demand.from_table(trips_file, ["all"], network)
    first = oms.run(network, demand, window=(28_800, 32_400), out=tmp_path / "first",
                    iteration_cap=1)
    with open(first.record, "rb") as f:
        assert tomllib.load(f)["network"]["region"] == corners
    again = oms.run_file(first.record, out=tmp_path / "again")
    opened = oms.Run.open(first.record)
    seconds = time.perf_counter() - started
    links = [r.osm_report["links"] for r in (first, again, opened)]
    print(f"Delft cut to a polygon of lists, run, run again and opened: {seconds:.2f} s; "
          f"links {links}")
    assert links[0] > 0 and links == [links[0]] * 3


def test_a_figure_eight_is_an_area_and_is_read():
    w, s, e, n = REGION
    oms.Network.from_osm(OSM, region=[(w, s), (e, n), (e, s), (w, n)])


@pytest.mark.parametrize("region, says", [
    ((4.41, 51.965, 4.315, 52.035), "west < east"),
    ((4.315, 51.965, 4.410), "neither a"),
    ([(4.3, 52.0), (4.4, 52.0)], "three vertices"),
    ((4.315, 51.965, 4.410, True), "boolean"),
    ("Delft", "neither a"),
    ([(4.3, 52.0), (4.3, 52.0), (4.4, 52.01)], "not on one line"),
    ([(4.3, 52.0), (4.35, 52.0), (4.4, 52.0)], "not on one line"),
    ([[4.30, 52.00], [4.35, 52.05], [4.40, 52.10]], "not on one line"),
])
def test_a_region_that_is_no_area_is_refused_by_name(region, says):
    with pytest.raises(ValueError, match=f"region(\\[3\\])?: .*{says}"):
        oms.Network.from_osm(OSM, region=region)
