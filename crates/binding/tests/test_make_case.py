"""The two experimental helpers on Delft (the extract of crates/io-osm, version 1's box):
`Network.from_osm(..., car_roads="main")` and `make_case`.

car_roads, by property: with "main" the bike and walk links are the very rows of the reader as
it is ("all"), cars drive no residential street, living street or service road, and fewer links;
a value that is neither is refused by name; a run on it records `car_roads = "main"` and runs
again from its record to the same trips.

make_case, by hand and property: population x trips_per_person trips (2 000 = 20 000 x 0.1), ids
counting up, departures in the hour from 07:30, classes from the reader's five; a through trip is
a car's alone and ends at least 1 km (GATES_APART_M) from where it starts; the share of through
trips is the reader's 5 % within four standard deviations of a binomial draw; the same arguments
give the same file, byte for byte; the folder runs with `run_file` and every trip is in its
table. For bikes: the classes that own a bike list `bike` among their modes, the others do not.
"""

import math
import tomllib
import warnings
from pathlib import Path

import pyarrow.parquet as pq
import pytest

import openmobisim as oms

CRATES = Path(__file__).resolve().parents[2]
OSM = CRATES / "io-osm" / "tests" / "fixtures" / "delft" / "network.osm.pbf"
GTFS = CRATES / "io-gtfs" / "tests" / "fixtures" / "delft" / "gtfs.zip"
REGION = (4.315, 51.965, 4.410, 52.035)
MINOR = {"residential", "living_street", "service"}
BIKE = {"car_and_bike": True, "bike_only": True, "car_only": False, "no_vehicle": False, "through": False}


def read(**keywords):
    with warnings.catch_warnings():
        warnings.simplefilter("ignore", UserWarning)  # the reader's repairs, not this test's subject
        return oms.Network.from_osm(OSM, region=REGION, **keywords)


def rows(links, mode):
    names = list(links)
    return [r for r in zip(*(links[n] for n in names)) if r[names.index("modes")] == mode]


def test_main_car_roads_leave_bikes_and_walkers_as_they_are_and_cars_off_minor_streets():
    _, every = read().tables()
    _, main = read(car_roads="main").tables()
    for mode in ("bike", "walk"):
        assert rows(main, mode) == rows(every, mode)
    classes = lambda t: {c for m, c in zip(t["modes"], t["road_class"]) if m == "car"}
    assert classes(every) & MINOR and not classes(main) & MINOR
    assert 0 < len(rows(main, "car")) < len(rows(every, "car"))
    with pytest.raises(ValueError, match="car_roads"):
        oms.Network.from_osm(OSM, region=REGION, car_roads="some")


def test_make_case_draws_the_trips_and_the_folder_runs(tmp_path):
    with warnings.catch_warnings():
        warnings.simplefilter("ignore", UserWarning)
        made = [oms.make_case(tmp_path / name, OSM, 20_000, region=REGION, gtfs=GTFS, car_roads="main")
                for name in ("a", "b")]
        assert made[0] == tmp_path / "a"
        assert (tmp_path / "a/trips.parquet").read_bytes() == (tmp_path / "b/trips.parquet").read_bytes()
        trips = pq.read_table(tmp_path / "a/trips.parquet").to_pylist()
        assert [t["trip_id"] for t in trips] == [f"t{k:07d}" for k in range(2_000)]
        through = [t for t in trips if t["class_id"] == "through"]
        deviation = math.sqrt(0.05 * 0.95 / 2_000)
        assert abs(len(through) / 2_000 - 0.05) < 4 * deviation
        for t in trips:
            assert 27_000 <= t["departure_s"] < 30_600
            assert ("bike" in t["modes"].split(";")) == BIKE[t["class_id"]]
        for t in through:
            assert t["modes"] == "car"
            east = (t["destination_lon_deg"] - t["origin_lon_deg"]) * 111_320 * math.cos(math.radians(52.0))
            north = (t["destination_lat_deg"] - t["origin_lat_deg"]) * 110_540
            assert math.hypot(east, north) > 990  # 1 km, less the flat-earth formula's error
        settings = tomllib.loads((tmp_path / "a/run.toml").read_text())
        assert settings["network"]["car_roads"] == "main" and settings["network"]["region"] == list(REGION)
        assert (settings["window_start_s"], settings["window_end_s"]) == (27_000, 34_200)
        first = oms.run_file(tmp_path / "a", out=tmp_path / "out")
        assert pq.read_table(first.trips).num_rows == 2_000
        record = tomllib.loads(Path(first.record).read_text())
        assert record["network"]["car_roads"] == "main"
        (tmp_path / "out/run.toml").write_text(Path(first.record).read_text())
        again = oms.run_file(tmp_path / "out", out=tmp_path / "again")
        assert pq.read_table(again.trips).equals(pq.read_table(first.trips))


def test_make_case_refuses_no_trips(tmp_path):
    with pytest.raises(ValueError, match="from 1 to"):
        oms.make_case(tmp_path / "none", OSM, 3, region=REGION)
