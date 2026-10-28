"""Shared-mobility services' tables: docks from a table and from GBFS, vehicles and chargers."""

from __future__ import annotations

import json

import openmobisim as ms
import pytest


def test_a_dock_table_is_read_with_its_optional_columns():
    text = (
        "dock_id,lon,lat,capacity,name,hub_id,bikes\n"
        "d1,4.90,52.37,20,Dam,H,7\n"
        "d2,4.91,52.36,10,,,\n"
    )
    d = ms.dock_read_table(text)
    assert d.count == 2
    rows = d.rows()
    assert rows["dock_id"] == ["d1", "d2"]
    assert rows["name"] == ["Dam", None]
    assert rows["hub_id"] == ["H", None]
    assert list(rows["capacity"]) == [20, 10]
    assert rows["bikes"] == [7, None], "no count: the service's share of the slots"


@pytest.mark.parametrize(
    ("text", "message"),
    [
        ("dock_id,lon,lat,capacity\nd,4.9,52.3,\n", "capacity"),
        ("dock_id,lon,lat,capacity,bikes\nd,4.9,52.3,5,6\n", "6 bikes in 5 slots"),
        ("dock_id,lon,lat,capacity\nd,4.9,52.3,5\nd,4.9,52.3,5\n", "more than once"),
        ("dock_id,lon,lat,capacity\nd,400,52.3,5\n", "not a place"),
        ("lon,lat,capacity\n4.9,52.3,5\n", "no dock_id"),
    ],
)
def test_a_dock_table_that_is_not_valid_is_refused_with_why(text, message):
    with pytest.raises(ValueError, match=message):
        ms.dock_read_table(text)


def test_gbfs_versions_2_and_3_are_read_with_the_bikes_docked_now():
    info = {
        "data": {
            "stations": [
                {"station_id": "1", "name": "Bastille", "lat": 48.85, "lon": 2.37, "capacity": 30},
                {
                    "station_id": "2",
                    "name": [{"text": "Nation", "language": "fr"}],
                    "lat": 48.85,
                    "lon": 2.39,
                    "capacity": 20,
                },
                {"station_id": "3", "lat": 48.86, "lon": 2.38},  # no capacity: left out
            ]
        }
    }
    status = {
        "data": {
            "stations": [
                {"station_id": "1", "num_bikes_available": 12},
                {"station_id": "2", "num_vehicles_available": 25},  # above capacity: capped
            ]
        }
    }
    d = ms.dock_read_gbfs(info, json.dumps(status))
    rows = d.rows()
    assert rows["dock_id"] == ["1", "2"]
    assert rows["name"] == ["Bastille", "Nation"]
    assert rows["bikes"] == [12, 20]
    assert ms.dock_read_gbfs(info).rows()["bikes"] == [None, None]
    with pytest.raises(ValueError, match="not a GBFS file"):
        ms.dock_read_gbfs({"stations": []})


def test_vehicles_and_chargers_are_read_and_checked():
    v = ms.vehicle_read_table("vehicle_id,lon,lat,soc\nv1,4.9,52.3,0.5\nv2,4.91,52.31,\n")
    assert (v.count, v.rows()["soc"]) == (2, [0.5, None])
    c = ms.charger_read_table(
        [{"charger_id": "c1", "lon": 4.9, "lat": 52.3, "plugs": 2, "kw": 22, "hub_id": "H"}]
    )
    rows = c.rows()
    assert (c.count, list(rows["plugs"]), rows["kw"], rows["hub_id"]) == (1, [2], [22.0], ["H"])
    with pytest.raises(ValueError, match="soc must be a share"):
        ms.vehicle_read_table("vehicle_id,lon,lat,soc\nv,4.9,52.3,1.5\n")
    with pytest.raises(ValueError, match="kw must be above 0"):
        ms.charger_read_table("charger_id,lon,lat,plugs,kw\nc,4.9,52.3,1,0\n")
    with pytest.raises(ValueError, match="more than once"):
        ms.vehicle_read_table("vehicle_id,lon,lat\nv,4.9,52.3\nv,4.9,52.3\n")
