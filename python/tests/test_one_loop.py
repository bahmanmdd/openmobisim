"""The one-loop engine across the boundary: ``Scenario.run(engine="one-loop")``.

What is defended: at free flow both engines give every trip the same travel time (both are
exact there: a property of free flow, not a comparison of equilibria); a scenario asking for
what the engine does not have (a transit trip, a timetable, another flow level, the fixed times
of reaching and leaving the car) is refused by name; the convergence table has the engine's
measures and no verdict; the run folder is read back by ``Run.open``; the route sets and
choices are built from the chains.
"""

import json
import math

import numpy as np
import openmobisim as ms
import pytest

LAT = 45.70
KM = 1 / (111.32 * math.cos(math.radians(LAT)))
#: The car's drive alone: no time to reach or leave it.
DRIVES_ONLY = {"car_access_s": 0.0, "car_egress_s": 0.0}


def two_routes():
    """O → D by a short route (2 + 2 km, 1 200 PCU/h on its first link) or a long one."""
    nodes = [
        {"id": "O", "x": 4.80, "y": LAT},
        {"id": "P", "x": 4.80 + 2 * KM, "y": LAT + 0.004},
        {"id": "Q", "x": 4.80 + 2 * KM, "y": LAT - 0.004},
        {"id": "D", "x": 4.80 + 4 * KM, "y": LAT},
    ]

    def link(i, a, b, metres, capacity):
        return {
            "id": i,
            "from": a,
            "to": b,
            "class": "primary",
            "lanes": 3,
            "length": metres,
            "free_flow_speed": 50,
            "capacity": capacity,
        }

    links = [
        link("op", "O", "P", 2000, 1200.0),
        link("pd", "P", "D", 2000, 6000.0),
        link("oq", "O", "Q", 3200, 1800.0),
        link("qd", "Q", "D", 3200, 6000.0),
    ]
    return ms.network_read_table(links, nodes)


def cars(net, n, every_s, mode="car"):
    o, d = net.node_lonlat("O"), net.node_lonlat("D")
    return [
        (f"c{i}", 0, o[0], o[1], d[0], d[1], 60 + i * every_s, "car", None, mode) for i in range(n)
    ]


def scenario(net, trips, **kw):
    kw.setdefault("mode_options", DRIVES_ONLY)
    kw.setdefault("choice_model", "deterministic")
    return ms.Scenario.from_parts(net, trips, **kw)


@pytest.mark.parametrize("level", [0, 2])
def test_at_free_flow_both_engines_give_every_trip_the_same_time(tmp_path, level):
    # One car every 30 s (120 veh/h) on a road of 1 200 PCU/h: nobody waits at level 2.
    net = two_routes()
    trips = cars(net, 20, 30)
    s = scenario(net, trips, flow_level=level, equilibration="free_flow")
    old = s.run("old", output_dir=str(tmp_path / "old"), quiet=True)
    new = s.run("new", output_dir=str(tmp_path / "new"), quiet=True, engine="one-loop")
    t_old = np.asarray(old.trip_results()["travel_time_s"], dtype=float)
    t_new = np.asarray(new.trip_results()["travel_time_s"], dtype=float)
    ids = net.link_ids()
    ff = np.asarray(net.link_free_flow_s())
    short = ff[ids.index("op")] + ff[ids.index("pd")]
    # The current engine's clock is whole seconds; the one-loop engine's free flow is exact.
    assert np.all(np.abs(t_old - t_new) <= 1.0), (t_old, t_new)
    assert np.all(np.abs(t_new - short) <= 1.0)
    assert new.completion == old.completion
    assert new.engine == "one-loop" and old.engine == "current"


def test_a_transit_trip_is_refused_by_name(tmp_path):
    net = two_routes()
    trips = cars(net, 3, 60) + cars(net, 1, 60, mode="transit")[:1]
    trips[-1] = ("t0", *trips[-1][1:])
    s = scenario(net, trips)
    with pytest.raises(ms.EngineNotSupportedError, match="transit"):
        s.run("x", output_dir=str(tmp_path / "x"), quiet=True, engine="one-loop")


def test_what_the_engine_lacks_is_named_in_one_error(tmp_path):
    net = two_routes()
    s = ms.Scenario.from_parts(net, cars(net, 3, 60), flow_level=4, route_method="link_penalty")
    with pytest.raises(ms.EngineNotSupportedError) as caught:
        s.run("x", output_dir=str(tmp_path / "x"), quiet=True, engine="one-loop")
    text = str(caught.value)
    for part in ("flow_level 4", "route_method", "car_access_s", "car_egress_s"):
        assert part in text, text


def test_an_unknown_engine_is_refused(tmp_path):
    net = two_routes()
    with pytest.raises(ValueError, match="engine must be one of"):
        scenario(net, cars(net, 3, 60)).run("x", output_dir=str(tmp_path), engine="nope")


def test_the_convergence_table_has_the_measures_and_no_verdict(tmp_path):
    pq = pytest.importorskip("pyarrow.parquet")
    net = two_routes()
    # 1 800 veh/h for half an hour: the short route's queue sends some to the long one.
    run = scenario(
        net, cars(net, 900, 2), equilibration="msa", equilibration_options={"iterations": 6}
    ).run("q", output_dir=str(tmp_path / "q"), quiet=True, engine="one-loop")
    c = run.convergence()
    for name in (
        "iteration",
        "found_share",
        "added",
        "added_share",
        "switch_share",
        "switch_floor",
        "switch_excess",
        "switch_above_floor",
        "switch_excess_expected",
        "unfinished_share",
        "gap",
        "gap_deterministic",
        "spread",
        "mean_trip_s",
        "search_s",
        "choose_s",
        "load_s",
        "measure_s",
        "assess_s",
    ):
        assert name in c, name
    assert not any("verdict" in k for k in c)
    on_disk = pq.read_table(tmp_path / "q" / "convergence.parquet").column_names
    assert not any("verdict" in k for k in on_disk)
    assert run.convergence_verdict is None
    assert run.converged == (len(c["iteration"]) < 6)
    # Iteration 0 is everyone's first choice: all switch, and the floor says so.
    assert c["switch_share"][0] == 1.0 and c["switch_floor"][0] == 1.0


def test_the_folder_is_read_back_and_the_route_sets_come_from_the_chains(tmp_path):
    pytest.importorskip("pyarrow")
    net = two_routes()
    run = scenario(
        net,
        cars(net, 900, 2),
        equilibration="msa",
        equilibration_options={"iterations": 6},
        link_bin_s=300,
    ).run("q", output_dir=str(tmp_path / "q"), quiet=True, engine="one-loop")
    again = ms.Run.open(tmp_path / "q")
    assert again.engine == "one-loop"
    assert again.fingerprint == run.fingerprint
    assert again.total_travel_time_s == pytest.approx(run.total_travel_time_s)
    assert again.completion == run.completion
    assert list(again.convergence()["gap"]) == pytest.approx(list(run.convergence()["gap"]))
    manifest = json.loads((tmp_path / "q" / "manifest.json").read_text())
    assert manifest["engine"] == "one-loop" and manifest["fingerprint_covers"]
    # Each trip's route is the chain it took; the sets hold every chain of the pair.
    sets, choices = run.route_sets(), run.route_choices()
    chains = run.chains()
    taken = np.asarray(chains["taken"], dtype=bool)
    offsets, links = np.asarray(sets.route_offsets()), np.asarray(sets.links())
    route = np.asarray(choices.route)
    for row in np.flatnonzero(taken)[:50]:
        t = int(chains["trip"][row])
        r = route[t]
        assert list(links[offsets[r] : offsets[r + 1]]) == list(chains["links"][row])
    assert sets.route_count == 2
    bins = run.link_bins("road")
    assert bins is not None and len(bins) > 0
    # Every car crossed one link of each route it took: the crossings of the first links add
    # up to the trips that arrived.
    ids = net.link_ids()
    first = np.isin(np.asarray(bins.links()), [ids.index("op"), ids.index("oq")])
    assert int(np.asarray(bins.crossings())[first].sum()) == run.completion["completed"]


def test_the_same_seed_gives_the_same_run(tmp_path):
    net = two_routes()
    s = scenario(
        net,
        cars(net, 900, 2),
        choice_model="logit",
        equilibration="msa",
        equilibration_options={"iterations": 4},
        master_seed=7,
    )
    a = s.run("a", output_dir=str(tmp_path / "a"), quiet=True, engine="one-loop")
    b = s.run("b", output_dir=str(tmp_path / "b"), quiet=True, engine="one-loop")
    assert a.fingerprint == b.fingerprint
    assert np.array_equal(np.asarray(a.route_choices().route), np.asarray(b.route_choices().route))
    assert list(a.convergence()["gap"]) == list(b.convergence()["gap"])


def test_the_run_configuration_file_carries_the_engine(tmp_path):
    net = two_routes()
    s = scenario(net, cars(net, 50, 10))
    first = s.run("a", output_dir=str(tmp_path / "a"), quiet=True, engine="one-loop")
    assert 'engine = "one-loop"' in (tmp_path / "a" / "run.toml").read_text()
    again = s.run(
        output_dir=str(tmp_path / "b"), quiet=True, config=str(tmp_path / "a" / "run.toml")
    )
    assert again.engine == "one-loop" and again.fingerprint == first.fingerprint


def test_the_indifference_band_is_passed_through_and_in_the_fingerprint(tmp_path):
    net = two_routes()
    trips = cars(net, 900, 2)

    def run(name, **options):
        s = scenario(
            net,
            trips,
            equilibration="msa",
            equilibration_options={"iterations": 3, "min_iterations": 3, **options},
        )
        return s.run(name, output_dir=str(tmp_path / name), quiet=True, engine="one-loop")

    default, wide = run("default"), run("wide", indifference_band=0.05)
    bands = []
    for r, name in ((default, "default"), (wide, "wide")):
        manifest = json.loads((tmp_path / name / "manifest.json").read_text())
        bands.append(manifest["parameters"]["indifference_band"])
        c = r.convergence()
        # A chain counts as found only if it beats the set's best by more than the band: never
        # more trips found than had a chain added.
        assert np.all(np.asarray(c["found"]) <= np.asarray(c["added"]))
        assert c["found"][0] == c["added"][0] == len(trips), "every set is empty at first"
    assert bands == [0.01, 0.05]
    assert default.fingerprint != wide.fingerprint
    with pytest.raises(ValueError, match="indifference_band"):
        run("negative", indifference_band=-0.1)
