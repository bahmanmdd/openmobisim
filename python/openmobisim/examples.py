"""Built-in fixtures for getting started without any input data.

``manhattan_grid`` is the N3 fixture (``06_INTERFACE_V0.md`` §3, S105).
``fixed_car_trips`` is deliberately thin: "car, fixed departure time, from
here to there" is not a different demand format from the general one — it
is what the same trips schema already expresses when a class's
``class_defaults`` give it a car and nothing else (S127). This helper only
saves writing out the eight-column table by hand; it does not introduce a
second demand shape.
"""

from __future__ import annotations

from openmobisim import _core

__all__ = ["fixed_car_trips", "manhattan_grid"]

# A direct re-export, not a wrapper: PyO3 already carries the Rust doc
# comment as this function's `__doc__` (and `_core.pyi` is the hand-kept
# stub `mypy`/editors actually read), and a builtin function's `__doc__`
# cannot be reassigned from Python even if a different one were wanted.
manhattan_grid = _core.manhattan_grid


def fixed_car_trips(
    network: _core.Network,
    trips: list[tuple[str, tuple[int, int], tuple[int, int], float, int | None]],
    user_class: str = "commuter",
) -> list[tuple]:
    """Build a trips table for car-only demand at a fixed departure time.

    A convenience over the general schema, for the common case in a demo or
    a test: give the mode a fixed availability (a car, via
    ``class_defaults``) and a fixed departure time, rather than letting a
    choice model decide. Every row gets ``trip_seq = 0`` — this helper does
    not build multi-trip chains.

    Args:
        network: A grid built by ``manhattan_grid``.
        trips: ``(traveller_id, (origin_row, origin_col),
            (destination_row, destination_col), departure_hour, weight)``
            tuples. ``weight=None`` takes the scenario's default weight.
        user_class: The class every row is tagged with. Give this class a
            car in ``class_defaults`` — passed to ``Scenario.from_parts``,
            not here — or every trip in this table reports
            ``no_vehicle_available``.

    Returns:
        Rows in the schema ``Scenario.from_parts``' ``demand`` argument (or
        ``run_pipeline``) expects — the same table a ``trips.parquet`` file
        would produce.
    """
    rows = []
    for traveller_id, origin, destination, hour, weight in trips:
        origin_lon, origin_lat = _core.grid_node_lonlat(network, *origin)
        destination_lon, destination_lat = _core.grid_node_lonlat(network, *destination)
        departure_s = round(hour * 3600)
        rows.append(
            (
                traveller_id,
                0,
                origin_lon,
                origin_lat,
                destination_lon,
                destination_lat,
                departure_s,
                user_class,
                weight,
            )
        )
    return rows
