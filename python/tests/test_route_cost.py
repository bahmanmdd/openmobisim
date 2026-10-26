"""Every route search minimises the choice model's link-additive utility (S299).

What is defended: a run whose utility weighs time alone (the defaults: every price 0) searches by
time, and ``max_cost_vectors`` then changes nothing, not even the fingerprint; a run with a price
per kilometre searches by the generalised cost, and its fingerprint names the cost and the most
vectors kept; a bad ``max_cost_vectors`` is refused.
"""

from __future__ import annotations

import openmobisim as ms
import pytest


def _scenario(**options: object) -> ms.Scenario:
    net = ms.examples.toy_network()
    rows = ms.examples.trips_random(net, 80, seed=4, min_m=300.0, max_m=3000.0, spread_s=1800)
    return ms.Scenario.from_parts(
        net, rows, classes={"driver": (True, False, False)}, choice_model="logit", **options
    )


def test_a_time_only_run_hashes_as_before_whatever_the_most_vectors() -> None:
    assert _scenario()._fingerprint() == _scenario(max_cost_vectors=5)._fingerprint()


def test_a_priced_run_names_its_route_cost_in_the_fingerprint() -> None:
    priced = {"price_options": {"car_eur_km": 0.2}}
    one, two = _scenario(max_cost_vectors=1, **priced), _scenario(**priced)
    assert one._fingerprint() != two._fingerprint()
    assert _scenario(**priced)._fingerprint() != _scenario()._fingerprint()


def test_a_bad_most_vectors_is_refused() -> None:
    with pytest.raises(ValueError, match="max_cost_vectors"):
        _scenario(max_cost_vectors=0)
