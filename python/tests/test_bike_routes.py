"""Bike route sets and the bike route choice (S298): ``bike_route_choice="doubly_stochastic"``.

A bike trip has one route by default; with the option its pair has a sampled set of routes on
the bike layer, the trip chooses one through the choice model, and a trip choosing its mode is
offered every bike route. The car's results do not change.
"""

from __future__ import annotations

import numpy as np
import openmobisim as ms
import pytest

BIKES = {"cyclist": (False, True, False)}
BOTH = {"everyone": (True, True, False)}


def grid(n=6):
    return ms.examples.manhattan_grid(n=n, block_metres=200.0, signals=False)


def rows(net, n, mode, seed=5):
    """``n`` random trips of ``mode``, travellers named after it so two calls can be joined.

    ``None`` is a trip choosing its mode; the trips are 600 to 1 200 m apart.
    """
    cls = "cyclist" if mode == "bike" else "everyone"
    made = ms.examples.trips_random(
        net, n, seed=seed, min_m=600.0, max_m=1200.0, user_class=cls, mode=mode
    )
    return [(f"{mode}-{r[0]}", *r[1:]) for r in made]


def run(net, trips, name, classes, **kwargs):
    settings = {
        "classes": classes,
        "window": (0, 3_600),
        "flow_level": 2,
        "choice_model": "logit",
        "equilibration": "free_flow",
        "equilibration_options": {"increments": 1, "warmup": 0},
        "master_seed": 3,
    }
    settings.update(kwargs)
    return ms.Scenario.from_parts(net, trips, **settings).run(name, overwrite=True)


def test_without_the_option_bikes_have_no_route_sets():
    net = grid()
    r = run(net, rows(net, 20, "bike"), "bike-plain", BIKES)
    assert r.bike_route_sets() is None and r.bike_route_choices() is None


def test_sampled_bike_route_sets_give_bikes_a_choice_and_leave_the_car_alone():
    net = grid()
    trips = rows(net, 40, "bike") + rows(net, 20, "car", seed=9)
    plain = run(net, trips, "bike-sets-plain", BOTH)
    sampled = run(net, trips, "bike-sets", BOTH, bike_route_choice="doubly_stochastic")
    sets = sampled.bike_route_sets()
    assert sets is not None and sets.method == "sampled"
    assert sets.route_count > sets.key_count, "several routes for some pair"
    choices = sampled.bike_route_choices()
    assert choices is not None
    routed = choices.route >= 0
    assert routed.sum() == 40, "every stated bike trip chose"
    assert np.all(choices.route[routed] < sets.route_count)
    assert choices.alternatives[routed].max() >= 2, "some trip had a choice"
    # The car's sets and choices are the same; the fingerprint names the option.
    assert plain.route_sets().route_count == sampled.route_sets().route_count
    assert np.array_equal(plain.route_choices().route, sampled.route_choices().route)
    assert plain.fingerprint != sampled.fingerprint
    again = run(net, trips, "bike-sets-again", BOTH, bike_route_choice="doubly_stochastic")
    assert sampled.fingerprint == again.fingerprint
    # The old name selects the same, with a warning and the same fingerprint.
    with pytest.warns(DeprecationWarning, match='"doubly_stochastic"'):
        old = run(net, trips, "bike-sets-old", BOTH, bike_route_choice="sampled")
    assert old.fingerprint == sampled.fingerprint
    # The summary of a stated-bike run holds the car's numbers only.
    assert sampled.route_set_summary()["pairs"] == plain.route_set_summary()["pairs"]


def test_bike_route_options_are_the_sampled_method_s_and_are_checked():
    net = grid()
    trips = rows(net, 10, "bike")
    with pytest.raises(ValueError, match="bike_route_choice"):
        run(net, trips, "bike-bad", BIKES, bike_route_choice="penalty")
    with pytest.raises(ValueError, match="doubly_stochastic"):
        run(net, trips, "bike-opts-plain", BIKES, bike_route_options={"draws": 2})
    with pytest.raises(ValueError, match="draws"):
        run(
            net,
            trips,
            "bike-opts-bad",
            BIKES,
            bike_route_choice="doubly_stochastic",
            bike_route_options={"draws": 99},
        )
    with pytest.raises(ValueError, match="nope"):
        run(
            net,
            trips,
            "bike-opts-unknown",
            BIKES,
            bike_route_choice="doubly_stochastic",
            bike_route_options={"nope": 1},
        )
    one = run(
        net,
        trips,
        "bike-one",
        BIKES,
        bike_route_choice="doubly_stochastic",
        bike_route_options={"draws": 0},
    )
    sets = one.bike_route_sets()
    assert sets.route_count == sets.key_count, (
        "no draws and the default coefficients: the anchor alone"
    )
    assert "draws=0;" in sets.descriptor


def test_mode_choice_offers_every_bike_route():
    net = grid()
    trips = rows(net, 40, None)
    settings = {"modes": ["bike", "walk"], "mode_options": {"walk_max_s": 7200, "bike_max_s": 7200}}
    plain = run(net, trips, "bike-modes-plain", BIKES, **settings)
    sampled = run(
        net, trips, "bike-modes", BIKES, bike_route_choice="doubly_stochastic", **settings
    )
    alternatives = lambda r: int(r.itinerary_choices()["alternatives"].max())  # noqa: E731
    assert alternatives(plain) == 2, "one bike route and the walk"
    assert alternatives(sampled) > 2, "several bike routes and the walk"
