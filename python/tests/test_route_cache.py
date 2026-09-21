"""The route-set cache across the boundary (S178).

What is defended: a run that takes its route sets from the cache is the run that made them, the
counters say what happened, other inputs are other entries, and the cache can be cleared.
"""

import numpy as np
import openmobisim as ms

CAR = {"commuter": (True, False, False)}


def scenario(trips=200, **kwargs):
    net = ms.examples.manhattan_grid(n=6, block_metres=200.0, signals=False)
    rows = ms.examples.trips_random(net, trips, seed=5, min_m=300.0, max_m=1000.0, spread_s=300)
    settings = {"class_defaults": CAR, "window_hours": 1, "master_seed": 3}
    settings.update(kwargs)
    return ms.Scenario.from_parts(net, rows, **settings)


def test_a_cached_run_is_the_run_that_made_the_sets_and_the_counters_say_so():
    ms.route_cache_clear()
    base = ms.route_cache_info()
    plain = scenario().run("rc-plain")
    assert ms.route_cache_info() == base, "a run that does not ask does not touch the cache"
    first = scenario(route_cache=True).run("rc-first")
    second = scenario(route_cache=True).run("rc-second")
    hits, misses, held = ms.route_cache_info()
    assert (hits - base[0], misses - base[1], held) == (1, 1, 1)
    for run in (first, second):
        assert run.total_travel_time_s == plain.total_travel_time_s
        assert (run.route_choices().route == plain.route_choices().route).all()
        assert np.array_equal(run.route_sets().links(), plain.route_sets().links())
        assert run.route_sets().identity == plain.route_sets().identity
        assert run.fingerprint == plain.fingerprint, "the cache is not an input"


def test_other_inputs_are_other_entries_and_the_cache_can_be_cleared():
    ms.route_cache_clear()
    scenario(route_cache=True).run("rc-a")
    scenario(route_cache=True, route_method="shortest").run("rc-b")
    scenario(route_cache=True, route_options={"max_paths": 2}).run("rc-c")
    hits, misses, held = ms.route_cache_info()
    assert held == 3
    scenario(route_cache=True).run("rc-a-again")
    assert ms.route_cache_info()[0] == hits + 1
    ms.route_cache_clear()
    assert ms.route_cache_info()[2] == 0
    scenario(route_cache=True).run("rc-after-clear")
    assert ms.route_cache_info()[1] == misses + 1


def test_a_run_that_grows_its_sets_does_not_change_the_cached_ones():
    ms.route_cache_clear()
    settings = {
        "flow_level": 4,
        "choice_model": "logit",
        "equilibration": "msa",
        "equilibration_options": {"iterations": 3},
        "route_method": "shortest",
        "route_cache": True,
    }
    grown = scenario(trips=1500, route_update="best_response", **settings).run("rc-grown")
    plain = scenario(trips=1500, **settings).run("rc-plain-again")
    assert ms.route_cache_info()[0] >= 1
    assert plain.route_sets().update == "" and (plain.route_sets().stamps() == 0).all()
    assert grown.route_sets().route_count >= plain.route_sets().route_count
