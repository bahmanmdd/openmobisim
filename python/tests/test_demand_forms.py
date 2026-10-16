"""The demand in any form, checked row by row (C2, D-5 a): rows, ``Trip``, columns, data frames."""

from __future__ import annotations

import openmobisim as ms
import pytest


def _rows() -> tuple[ms.Network, list[tuple]]:
    net = ms.examples.toy_network()
    return net, ms.examples.trips_random(net, 30, seed=3, min_m=100.0, max_m=2000.0, spread_s=1800)


def test_every_form_of_one_demand_makes_one_scenario() -> None:
    net, rows = _rows()
    base = ms.Scenario.from_parts(net, rows)._fingerprint()
    named = [ms.Trip(*row) for row in rows]
    columns = {
        name: list(column)
        for name, column in zip(ms.Trip._fields, zip(*rows, strict=True), strict=False)
    }
    assert ms.Scenario.from_parts(net, named)._fingerprint() == base
    assert ms.Scenario.from_parts(net, columns)._fingerprint() == base
    assert ms.Scenario.from_parts(net, ms.Columns(columns))._fingerprint() == base
    pd = pytest.importorskip("pandas")
    assert ms.Scenario.from_parts(net, pd.DataFrame(columns))._fingerprint() == base


def test_a_trip_names_the_fields_of_a_row() -> None:
    trip = ms.Trip("a", 0, 4.8, 45.7, 4.81, 45.71, 25200, "commuter")
    assert trip.weight is None and trip.mode is None and len(trip) == 10
    assert trip.departure_time_s == 25200


@pytest.mark.parametrize(
    ("field", "value", "message"),
    [
        (6, "07:30", "departure_time_s must be whole seconds"),
        (3, 999.0, "origin_lat must be degrees from -90 to 90"),
        (2, float("nan"), "origin_lon must be degrees"),
        (1, -1, "trip_seq must be a whole number from 0"),
        (0, 7, "traveller_id must be a non-empty string"),
        (7, None, "user_class must be a class name"),
        (8, 0, "weight must be a whole number from 1"),
    ],
)
def test_a_bad_row_is_refused_with_its_number_and_field(field, value, message) -> None:
    net, rows = _rows()
    bad = list(rows[4])
    bad[field] = value
    rows[4] = tuple(bad)
    with pytest.raises(ValueError, match=rf"demand row 4 \(from 0\): {message}"):
        ms.Scenario.from_parts(net, rows)


def test_columns_must_be_the_trips_columns() -> None:
    net, _ = _rows()
    with pytest.raises(ValueError, match=r"unknown \['depart'\].*missing \['departure_time_s'\]"):
        ms.Scenario.from_parts(
            net,
            {
                "traveller_id": ["a"],
                "trip_seq": [0],
                "origin_lon": [4.8],
                "origin_lat": [45.7],
                "destination_lon": [4.81],
                "destination_lat": [45.71],
                "depart": [25200],
                "user_class": ["c"],
            },  # fmt: skip
        )
