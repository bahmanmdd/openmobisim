"""Working with a trips table before a run.

``demand_sample`` is the traveller-weight dial (S209): simulate one traveller for every
``weight`` people, the rest of the table unchanged. Fewer travellers cost proportionally
less to choose for, route and load, at the price of sampling noise, which replications
(different ``seed`` values) measure.
"""

from __future__ import annotations

import hashlib
from collections import defaultdict

__all__ = ["demand_sample"]

# Columns of a trips row (``Scenario.from_parts``' demand schema).
_TRAVELLER, _CLASS, _WEIGHT, _MODE = 0, 7, 8, 9


def _draw(seed: int, traveller: object) -> bytes:
    """A traveller's place in the sample's random order: a hash of the seed and its id."""
    return hashlib.blake2b(f"{seed}\x1f{traveller}".encode(), digest_size=8).digest()


def demand_sample(trips: list[tuple], weight: int, seed: int = 0) -> list[tuple]:
    """Keep one traveller in every ``weight``, each standing for ``weight`` times as many people.

    A traveller is kept or left out with all of their trips, so a day's chain of trips stays
    whole. The travellers are put in a random order fixed by ``seed`` and their id, and every
    ``weight``-th is kept, **within each group of the same user class and the same stated mode
    of their first trip**: each group keeps its share of the demand to within one traveller,
    and a mode or class that is rare is not lost or doubled by chance. Every kept trip's
    weight (column 9, ``None`` meaning the scenario's ``default_weight``, taken as 1 here) is
    multiplied by ``weight``, so totals in the results (travel time, volumes, mode shares by
    weight) estimate those of the whole table.

    The same table, ``weight`` and ``seed`` always keep the same travellers, on any machine;
    another ``seed`` is another sample, for replications.

    Args:
        trips: Rows in the schema of ``Scenario.from_parts``' ``demand``.
        weight: People per kept traveller, a whole number of at least 1 (1 keeps everyone).
        seed: Which sample.

    Returns:
        The kept rows, in their original order, with their weights multiplied.

    Raises:
        ValueError: If ``weight`` is not a whole number of at least 1.
    """
    if isinstance(weight, bool) or int(weight) != weight or weight < 1:
        raise ValueError(f"weight must be a whole number of at least 1, got {weight!r}")
    weight = int(weight)
    if weight == 1:
        return list(trips)
    first: dict[object, tuple] = {}
    for row in trips:
        first.setdefault(row[_TRAVELLER], row)
    groups: dict[tuple, list[object]] = defaultdict(list)
    for traveller, row in first.items():
        mode = row[_MODE] if len(row) > _MODE else None
        groups[(row[_CLASS], mode)].append(traveller)
    kept: set[object] = set()
    for members in groups.values():
        members.sort(key=lambda t: (_draw(seed, t), str(t)))
        kept.update(members[::weight])
    out = []
    for row in trips:
        if row[_TRAVELLER] in kept:
            row = list(row)
            row[_WEIGHT] = (row[_WEIGHT] or 1) * weight
            out.append(tuple(row))
    return out
