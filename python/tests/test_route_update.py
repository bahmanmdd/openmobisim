"""Route updates across the boundary (S176).

What is defended: a run without one is what it was (no routes added, sets as the method made them,
the same fingerprint as a run that names ``"none"``); with ``"best_response"`` on a jammed grid the
sets grow between iterations, every number about it agrees between ``Run.convergence()``, the
sets' own stamps and the manifest, the settings are refused before any work if wrong, and the
grown sets are still sets: consistent with the choices made from them. Why it works (a road that
only pays under congestion enters the set and the gap to the whole network falls) is proved in
Rust on a hand-made bottleneck, where the answer can be worked out.
"""

import numpy as np
import openmobisim as ms
import pytest

CAR = {"commuter": (True, False, False)}
UPDATE = "best_response;max_routes=10;searches=1;slack=0.02"


def scenario(trips=1_800, **kwargs):
    # A grid whose middle jams: many trips leaving within five minutes.
    net = ms.examples.manhattan_grid(n=8, block_metres=200.0, signals=False)
    rows = ms.examples.trips_random(net, trips, seed=5, min_m=300.0, max_m=1400.0, spread_s=300)
    settings = {
        "class_defaults": CAR,
        "window_hours": 2,
        "flow_level": 4,
        "choice_model": "logit",
        "equilibration": "msa",
        "equilibration_options": {"iterations": 6, "gap_sample": 300, "warmup": 0},
        "master_seed": 3,
        # The method's own sets unless a test asks for the update (S179 made an update and one
        # route per pair the default of an iterating run).
        "route_method": "penalty",
        "route_update": "none",
    }
    settings.update(kwargs)
    return ms.Scenario.from_parts(net, rows, **settings)


def go(name, **kwargs):
    return scenario(**kwargs).run(name)


def test_the_updates_are_listed_with_the_default_first():
    assert ms.route_update_methods() == ["none", "best_response"]


def test_without_an_update_nothing_is_added_and_the_sets_are_the_methods():
    run = go("ru-none")
    c = run.convergence()
    assert run.route_update == "none"
    assert (c["routes_added"] == 0).all() and (c["route_searches"] == 0).all()
    sets = run.route_sets()
    assert sets.update == "" and (sets.stamps() == 0).all()
    assert run.manifest()["route_update"] == "none"
    # Naming the default is the run without naming it.
    named = go("ru-named", route_update="none", route_update_options=None)
    assert named.fingerprint == run.fingerprint
    assert named.total_travel_time_s == run.total_travel_time_s
    assert (named.route_choices().route == run.route_choices().route).all()


def test_an_update_grows_the_sets_between_iterations_and_says_so_everywhere():
    plain = go("ru-plain")
    run = go("ru-best", route_update="best_response")
    c, sets = run.convergence(), run.route_sets()
    assert run.route_update == "best_response"
    assert run.fingerprint != plain.fingerprint, "an update is an input"
    # It found routes on the jam, looked after every loading but the last, and only then.
    added = int(c["routes_added"].sum())
    assert added > 0, "the grid jams: some pair has a faster route than its set holds"
    assert (c["route_searches"][:-1] > 0).all()
    assert c["route_searches"][-1] == 0 and c["routes_added"][-1] == 0
    # The sets agree with the count: each added route carries the iteration it was added for.
    stamps = sets.stamps()
    assert (stamps > 0).sum() == added
    for i in range(1, 6):
        assert (stamps == i).sum() == c["routes_added"][i - 1], i
    assert sets.route_count == plain.route_sets().route_count + added
    assert sets.update == UPDATE and sets.method == plain.route_sets().method
    assert sets.identity != plain.route_sets().identity
    # The method's routes come first, best first; added routes follow in the order they came.
    offsets, costs = sets.set_offsets(), sets.costs()
    for k in range(sets.key_count):
        s, e = offsets[k], offsets[k + 1]
        assert (np.diff(stamps[s:e].astype(np.int64)) >= 0).all(), "stamps grow along a set"
        made = costs[s:e][stamps[s:e] == 0]
        assert (np.diff(made) >= -1e-6).all(), "the method's routes are best first"
    # The choices are consistent with the grown sets.
    rc = run.route_choices()
    routed = rc.rank >= 0
    assert (rc.route[routed] == offsets[rc.pair[routed]] + rc.rank[routed]).all()
    sizes = np.diff(offsets)
    assert (rc.alternatives[routed] == sizes[rc.pair[routed]]).all()
    assert (rc.route[routed] < offsets[rc.pair[routed] + 1]).all()
    # Someone took a route that was added: the sets grew to be used.
    assert (stamps[rc.route[routed]] > 0).any()
    # The manifest names the update and its options.
    manifest = run.manifest()
    assert manifest["route_update"] == "best_response"
    assert manifest["route_update_descriptor"] == UPDATE


def test_when_nothing_beats_the_set_nothing_is_added_and_nothing_changes():
    # One traveller on an empty network never queues: no route is faster than the set's best, so
    # every search comes back empty-handed. It looked (a search per loading but the last), and
    # the run is the run without the update.
    net = ms.examples.toy_network()
    rows = [("t0", 0, *net.node_lonlat("W"), *net.node_lonlat("D1"), 0, "commuter", None)]
    settings = {
        "class_defaults": CAR,
        "flow_level": 4,
        "choice_model": "logit",
        "equilibration": "msa",
        "equilibration_options": {"iterations": 3, "gap_sample": 10},
        "master_seed": 3,
        "route_update": "none",
    }
    plain = ms.Scenario.from_parts(net, rows, **settings).run("ru-empty-plain")
    run = ms.Scenario.from_parts(net, rows, **{**settings, "route_update": "best_response"}).run(
        "ru-empty"
    )
    c = run.convergence()
    assert run.route_update == "best_response" and run.fingerprint != plain.fingerprint
    assert (c["routes_added"] == 0).all()
    # With the default slack of 2% nobody looks: the best route is at free flow, nothing to gain.
    assert c["route_searches"].tolist() == [0, 0, 0]
    looked = ms.Scenario.from_parts(
        net,
        rows,
        **{**settings, "route_update": "best_response"},
        route_update_options={"slack": 0},
    ).run("ru-empty-looked")
    assert looked.convergence()["route_searches"].tolist() == [1, 1, 0]
    assert (looked.convergence()["routes_added"] == 0).all()
    assert (
        run.route_sets().update == "" and run.route_sets().identity == plain.route_sets().identity
    )
    assert (run.route_choices().route == plain.route_choices().route).all()
    assert run.total_travel_time_s == plain.total_travel_time_s


def test_a_run_is_repeatable_with_an_update():
    a = go("ru-rep", route_update="best_response")
    b = go("ru-rep", route_update="best_response")
    assert (a.route_choices().route == b.route_choices().route).all()
    assert (a.route_sets().links() == b.route_sets().links()).all()
    assert (a.route_sets().stamps() == b.route_sets().stamps()).all()
    assert a.total_travel_time_s == b.total_travel_time_s


def test_the_options_change_what_it_does():
    one = go("ru-one", route_update="best_response")
    capped = go("ru-cap", route_update="best_response", route_update_options={"max_routes": 1})
    # A limit of one route per pair leaves no room: nothing is searched or added.
    assert (capped.convergence()["routes_added"] == 0).all()
    assert (capped.convergence()["route_searches"] == 0).all()
    three = go("ru-3", route_update="best_response", route_update_options={"searches": 3})
    assert three.route_sets().update == "best_response;max_routes=10;searches=3;slack=0.02"
    assert three.convergence()["route_searches"][0] > one.convergence()["route_searches"][0]


def test_an_update_needs_iterations_to_do_anything():
    # One loading, no next iteration to choose among new routes: the sets stay as made.
    run = go(
        "ru-once", route_update="best_response", equilibration="none", equilibration_options=None
    )
    assert (run.convergence()["routes_added"] == 0).all()
    assert run.route_sets().update == ""


def test_bad_settings_are_refused_before_any_work():
    net = ms.examples.toy_network()
    rows = [("t0", 0, *net.node_lonlat("W"), *net.node_lonlat("D1"), 0, "commuter", None)]

    def build(**kwargs):
        return ms.Scenario.from_parts(net, rows, class_defaults=CAR, **kwargs).run("ru-bad")

    with pytest.raises(ValueError, match='no route update called "nope".*none, best_response'):
        build(route_update="nope")
    with pytest.raises(ValueError, match="max_routes, searches"):
        build(route_update="best_response", route_update_options={"nope": 1})
    for name, value in [
        ("searches", 0),
        ("searches", 17),
        ("searches", 1.5),
        ("max_routes", 0),
        ("max_routes", 33),
    ]:
        with pytest.raises(ValueError, match=f"{name}.*whole number"):
            build(route_update="best_response", route_update_options={name: value})
    for value in (-0.1, 1.5):
        with pytest.raises(ValueError, match="slack.*from 0 to 1"):
            build(route_update="best_response", route_update_options={"slack": value})
    with pytest.raises(ValueError, match="no options"):
        build(route_update="none", route_update_options={"searches": 1})
