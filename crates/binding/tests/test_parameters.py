"""The keywords and the tables: every row of PARAMETERS.md (parsed here) is accepted at exactly one
entry point and refused at the others; an unknown keyword, a missing or unknown column and a
missing window are refused by name; a crate's error comes through unchanged; a non-default value
takes effect.

The entry points by the crate that reads each row (crates/binding/README.md): the graph's three
build values at Network.from_osm and Network.from_tables (one entry point, the network's two
constructors), osm_class_defaults at Network.from_osm alone (a network from tables has no OSM
classes), transfer_walk_max at Transit.from_gtfs (the GTFS reader), stop_walk_max at
Demand.from_table (the demand crate), every other row at run (the loop).

The hand case for bikes: a path of 1 500 m ridden at 15 km/h takes 1 500 / (15 000 / 3 600) =
360 s; at bike_speed 30 km/h, 1 500 / (30 000 / 3 600) = 180 s, half.
"""

import re
from pathlib import Path

import pyarrow.parquet as pq
import pytest

import openmobisim as oms

PARAMETERS = Path(__file__).resolve().parents[3] / "PARAMETERS.md"
NETWORK = {"capacity_per_lane", "bike_speed", "walk_speed"}
ELSEWHERE = {
    "osm_class_defaults": {"Network.from_osm"},
    "transfer_walk_max": {"Transit.from_gtfs"},
    "stop_walk_max": {"Demand.from_table"},
}
# Time alone: 1 on the nine time columns of the choice row (minutes), 0 on the other seven.
TIME_ONLY = (1.0,) * 9 + (0.0,) * 7
# A value each row can take (test values, not the defaults).
VALUES = {
    "capacity_per_lane": 2_000.0, "bike_speed": 18.0, "walk_speed": 5.0,
    "osm_class_defaults": None, "transfer_walk_max": 300.0, "stop_walk_max": 1_000.0,
    "boarding_slack": 30, "max_rides": 3, "choice": "deterministic",
    "coefficients": {"all": TIME_ONLY}, "nest_scale": 0.7, "iteration_cap": 2,
    "iteration_floor": 1, "link_bin": 600, "seed": 7,
}
NODES = {"node_id": ["a", "b"], "x_m": [0.0, 1_500.0], "y_m": [0.0, 0.0]}
LINKS = {
    "link_id": ["ab"], "upstream": ["a"], "downstream": ["b"], "modes": ["bike;walk"],
    "length_m": [1_500.0], "lanes": [None], "car_speed_km_h": [None], "capacity_veh_h": [None],
}
TRIPS = {
    "trip_id": ["ride"], "origin_x_m": [0.0], "origin_y_m": [0.0], "destination_x_m": [1_500.0],
    "destination_y_m": [0.0], "departure_s": [0], "class_id": ["all"], "modes": ["bike"],
}


def rows():
    """The parameter names of PARAMETERS.md's table, after its header and rule lines."""
    names = re.findall(r"^\| *([A-Za-z_][A-Za-z0-9_]*) *\|", PARAMETERS.read_text(), re.M)
    return [n for n in names if n != "Parameter"]


def entry_points(tmp_path):
    """Each entry point as a call taking keywords. Reading an absent file is an OSError (OSM) or,
    on a network in metres, a ValueError (GTFS): both come after the keywords are read."""
    network = oms.Network.from_tables(NODES, LINKS)
    demand = oms.Demand.from_table(TRIPS, ["all"], network)
    return {
        "Network.from_osm": lambda **k: oms.Network.from_osm(tmp_path / "absent.osm.pbf", **k),
        "Network.from_tables": lambda **k: oms.Network.from_tables(NODES, LINKS, **k),
        "Transit.from_gtfs": lambda **k: oms.Transit.from_gtfs(tmp_path / "absent.zip", network,
                                                               **k),
        "Demand.from_table": lambda **k: oms.Demand.from_table(TRIPS, ["all"], network, **k),
        "run": lambda **k: oms.run(network, demand, window=(0, 3_600), out=tmp_path / "run", **k),
    }


def accepted(call, name):
    """Whether `call` reads the keyword `name`: a refusal is a TypeError naming it."""
    try:
        call(**{name: VALUES[name]})
    except TypeError as e:
        assert name in str(e), str(e)
        return False
    except (OSError, ValueError):
        pass
    return True


def test_every_row_is_accepted_at_exactly_one_entry_point(tmp_path):
    names = rows()
    assert len(names) == len(set(names)) and "bike_speed" in names and "seed" in names
    calls = entry_points(tmp_path)
    for name in names:
        want = ({"Network.from_osm", "Network.from_tables"} if name in NETWORK
                else ELSEWHERE.get(name, {"run"}))
        got = {entry for entry, call in calls.items() if accepted(call, name)}
        assert got == want, f"{name}: accepted at {sorted(got)}, expected {sorted(want)}"
        assert len({e.split(".")[0] for e in got}) == 1, name


def test_an_unknown_keyword_and_a_missing_window_are_refused_by_name(tmp_path):
    network = oms.Network.from_tables(NODES, LINKS)
    demand = oms.Demand.from_table(TRIPS, ["all"], network)
    with pytest.raises(TypeError, match="bike_sped"):
        oms.Network.from_tables(NODES, LINKS, bike_sped=15.0)
    with pytest.raises(TypeError, match="iterations"):
        oms.run(network, demand, window=(0, 3_600), out=tmp_path, iterations=3)
    with pytest.raises(TypeError, match="window"):
        oms.run(network, demand, out=tmp_path)
    with pytest.raises(TypeError, match="bike_speed is read by Network.from_osm"):
        oms.run(network, demand, window=(0, 3_600), out=tmp_path, bike_speed=30.0)
    with pytest.raises(ValueError, match="osm_class_defaults"):
        oms.Network.from_osm(tmp_path / "absent.osm.pbf", osm_class_defaults={"residential": 30})
    with pytest.raises(ValueError, match="bike_speed"):
        oms.Network.from_tables(NODES, LINKS, bike_speed="fast")
    with pytest.raises(ValueError, match="choice"):
        oms.run(network, demand, window=(0, 3_600), out=tmp_path, choice="probit")
    with pytest.raises(ValueError, match="other is not a class of the demand"):
        oms.run(network, demand, window=(0, 3_600), out=tmp_path,
                coefficients={"all": TIME_ONLY, "other": TIME_ONLY})


def test_a_missing_or_unknown_column_is_refused_by_name():
    with pytest.raises(ValueError, match="nodes: missing column y_m"):
        oms.Network.from_tables({k: v for k, v in NODES.items() if k != "y_m"}, LINKS)
    with pytest.raises(ValueError, match="links: unknown column speed"):
        oms.Network.from_tables(NODES, {**LINKS, "speed": [30.0]})
    # Both kinds of position: longitude/latitude is told by its names, so x_m is the other one.
    with pytest.raises(ValueError, match="nodes: unknown column x_m"):
        oms.Network.from_tables({**NODES, "lon_deg": [4.0, 4.1], "lat_deg": [52.0, 52.0]}, LINKS)
    network = oms.Network.from_tables(NODES, LINKS)
    with pytest.raises(ValueError, match="trips: missing column class_id"):
        oms.Demand.from_table({k: v for k, v in TRIPS.items() if k != "class_id"}, ["all"],
                              network)
    with pytest.raises(ValueError, match="trips: trip ride: \"bicycle\" is not a mode"):
        oms.Demand.from_table({**TRIPS, "modes": ["bicycle"]}, ["all"], network)


def test_a_crates_error_comes_through_unchanged(tmp_path):
    # The graph crate's message for a link to an undeclared node (its BuildError::UnknownNode).
    with pytest.raises(ValueError) as e:
        oms.Network.from_tables(NODES, {**LINKS, "downstream": ["z"]})
    assert str(e.value) == "link ab names node z, which is never declared"
    # The demand crate's message for a trip of a class not given (DemandError).
    network = oms.Network.from_tables(NODES, LINKS)
    with pytest.raises(ValueError, match="ride"):
        oms.Demand.from_table(TRIPS, ["other"], network)
    with pytest.raises(OSError):
        oms.Network.from_osm(tmp_path / "absent.osm.pbf")


def ride_seconds(tmp_path, **network_parameters):
    network = oms.Network.from_tables(NODES, LINKS, **network_parameters)
    demand = oms.Demand.from_table(TRIPS, ["all"], network)
    result = oms.run(network, demand, window=(0, 3_600), out=tmp_path)
    return pq.read_table(result.trips).column("travel_time_s").to_pylist()


def test_bike_speed_30_halves_a_ride(tmp_path):
    assert ride_seconds(tmp_path / "15", bike_speed=15.0) == [360.0]
    assert ride_seconds(tmp_path / "30", bike_speed=30.0) == [180.0]
