"""``demand_sample``: one traveller for every ``weight`` people, chains whole, shares kept."""

from __future__ import annotations

from collections import Counter

import openmobisim as ms
import pytest


def table(n: int) -> list[tuple]:
    """``n`` travellers with two trips each; a tenth of them cycle, the rest drive."""
    rows = []
    for i in range(n):
        mode = "bike" if i % 10 == 0 else "car"
        cls = "a" if i % 3 else "b"
        for seq in (0, 1):
            rows.append((f"t{i}", seq, 4.0, 52.0, 4.1, 52.1, 3600 * (7 + seq), cls, None, mode))
    return rows


def test_one_in_weight_travellers_is_kept_with_whole_chains_and_weights_multiplied() -> None:
    rows = table(1000)
    kept = ms.demand_sample(rows, 10)
    travellers = Counter(r[0] for r in kept)
    assert 98 <= len(travellers) <= 102, "a tenth of 1 000, per group to within one"
    assert set(travellers.values()) == {2}, "every kept traveller keeps both trips"
    assert {r[8] for r in kept} == {10}
    # The rows come back in their original order.
    order = {r[0]: i for i, r in enumerate(rows) if r[1] == 0}
    firsts = [order[r[0]] for r in kept if r[1] == 0]
    assert firsts == sorted(firsts)


def test_each_class_and_mode_keeps_its_share() -> None:
    rows = table(3000)
    kept = ms.demand_sample(rows, 10)

    def groups(rs: list[tuple]) -> Counter:
        return Counter((r[7], r[9]) for r in rs if r[1] == 0)

    full, sample = groups(rows), groups(kept)
    for key, n in full.items():
        assert abs(sample[key] * 10 - n) < 10, key


def test_the_sample_is_fixed_by_its_seed() -> None:
    rows = table(500)
    a, b = ms.demand_sample(rows, 5, seed=1), ms.demand_sample(rows, 5, seed=1)
    c = ms.demand_sample(rows, 5, seed=2)
    assert a == b
    assert {r[0] for r in a} != {r[0] for r in c}


def test_weight_one_keeps_everything_and_bad_weights_are_refused() -> None:
    rows = table(20)
    assert ms.demand_sample(rows, 1) == rows
    for bad in (0, 2.5, -3, True):
        with pytest.raises(ValueError, match="whole number"):
            ms.demand_sample(rows, bad)


def test_a_given_weight_is_multiplied() -> None:
    rows = [("x", 0, 4.0, 52.0, 4.1, 52.1, 0, "a", 3, "car")]
    assert ms.demand_sample(rows, 4)[0][8] in (12,)
