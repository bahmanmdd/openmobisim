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


def test_a_toll_changes_the_routes_generated_and_names_the_cost() -> None:
    net = ms.examples.manhattan_grid(n=6, block_metres=200.0, signals=False)
    rows = ms.examples.trips_random(net, 60, seed=5, min_m=600.0, max_m=2000.0, spread_s=900)

    def sets(**options: object) -> ms._core.RouteSets:
        return (
            ms.Scenario.from_parts(
                net,
                rows,
                classes={"driver": (True, False, False)},
                choice_model="logit",
                route_method="link_penalty",
                equilibration="free_flow",
                **options,
            )
            .run("rc-toll", quiet=True, overwrite=True)
            .route_sets()
        )

    toll = {"road": {"toll_eur": [1.0 if i % 3 == 0 else 0.0 for i in range(net.link_count)]}}
    a, b = sets(), sets(link_values=toll)
    assert a.method == b.method == "penalty", "the method keeps its name"
    assert ";cost=given:" in b.descriptor and ";cost=" not in a.descriptor
    assert a.key_count == b.key_count
    changed = sum(
        [list(r) for r in a.routes(k)] != [list(r) for r in b.routes(k)] for k in range(a.key_count)
    )
    assert changed > 0, "a euro on every third link (six minutes at the defaults) moves some sets"


@pytest.mark.parametrize(
    ("old", "new", "argument"),
    [
        ("penalty", "link_penalty", "route_method"),
        ("montecarlo", "simulation", "route_method"),
        ("sampled", "doubly_stochastic", "route_method"),
        ("best_response", "least_cost", "route_update"),
    ],
)
def test_an_old_name_warns_and_selects_the_same_method(old: str, new: str, argument: str) -> None:
    with pytest.warns(DeprecationWarning, match=f'"{new}"'):
        before = _scenario(**{argument: old})
    assert before._fingerprint() == _scenario(**{argument: new})._fingerprint()


@pytest.mark.parametrize(
    ("old", "new"),
    [
        ("PenaltyOptions", "LinkPenaltyOptions"),
        ("MonteCarloOptions", "SimulationOptions"),
        ("BestResponseOptions", "LeastCostOptions"),
    ],
)
def test_an_old_options_class_warns_and_is_the_new_one(old: str, new: str) -> None:
    with pytest.warns(DeprecationWarning, match=new):
        given = getattr(ms, old)(max_routes=3) if "Response" in old else getattr(ms, old)()
    assert isinstance(given, getattr(ms, new))
