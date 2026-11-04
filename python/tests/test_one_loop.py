"""The one-loop engine across the boundary: ``Scenario.run(engine="one-loop")``.

What is defended: at free flow both engines give every trip the same travel time (both are
exact there: a property of free flow, not a comparison of equilibria); a car trip pays its
fixed times (reaching the car, parking and leaving it) door to door, as derived by hand, and the
choice weighs them as walking and parking time; a scenario asking for what the engine does not
have (a transit trip, a timetable, another flow level) is refused by name; the convergence table
has the engine's measures and no verdict; the run folder is read back by ``Run.open``; the route
sets and choices are built from the chains, whose links are the network's ids; a run continued
from a run that stopped by its own rule moves nobody beyond chance (idempotence), and one
continued from a run of another engine or scenario is refused; a relabelled network gives the
same chains by id.
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
    for part in ("flow_level 4", "route_method"):
        assert part in text, text
    # The car's fixed times are the engine's own now (120 s and 300 s by default).
    assert "car_access_s" not in text and "car_egress_s" not in text, text


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
    ids = net.link_ids()
    for row in np.flatnonzero(taken)[:50]:
        t = int(chains["trip"][row])
        r = route[t]
        assert list(links[offsets[r] : offsets[r + 1]]) == list(chains["link_index"][row])
    # The links by the network's ids, the same links as the indices.
    for named, index in zip(chains["links"], chains["link_index"], strict=True):
        assert named == [ids[i] for i in index]
    assert {tuple(x) for x in chains["links"]} == {("op", "pd"), ("oq", "qd")}
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


def test_the_allocation_is_passed_through_recorded_and_refused_by_name(tmp_path):
    net = two_routes()
    trips = cars(net, 900, 2)

    def run(name, **options):
        s = scenario(
            net,
            trips,
            equilibration="msa",
            equilibration_options={"iterations": 3, "min_iterations": 3, **options},
        )
        return s, s.run(name, output_dir=str(tmp_path / name), quiet=True, engine="one-loop")

    _, default = run("default")
    _, drawn = run("draws", allocation="draws")
    _, quota = run("quota", allocation="quota")
    names = [
        json.loads((tmp_path / n / "manifest.json").read_text())["parameters"]["allocation"]
        for n in ("default", "draws", "quota")
    ]
    assert names == ["draws", "draws", "quota"]
    # The default named or not is the same run; the quota is another.
    assert default.fingerprint == drawn.fingerprint != quota.fingerprint
    with pytest.raises(ValueError, match="allocation must be one of"):
        run("misspelt", allocation="quotas")


# --- the car's fixed times ---------------------------------------------------------------


@pytest.mark.parametrize("level", [0, 2])
def test_a_car_trip_pays_its_fixed_times_door_to_door(tmp_path, level):
    # One car on the empty two-route network with the shipped fixed times, 120 s to reach the
    # car and 300 s to park and leave it. Derived: the car sets off 120 s after the departure
    # and drives the short route (op, pd) at free flow; the trip ends 300 s after it arrives:
    # door to door 120 + the drive + 300.
    net = two_routes()
    trips = cars(net, 1, 60)
    run = scenario(net, trips, mode_options=None, flow_level=level, equilibration="free_flow").run(
        "x", output_dir=str(tmp_path / "x"), quiet=True, engine="one-loop"
    )
    ids = net.link_ids()
    ff = np.asarray(net.link_free_flow_s())
    drive = ff[ids.index("op")] + ff[ids.index("pd")]
    t = run.trip_results()
    departure = float(t["departure_s"][0])
    # Level 2 records arrivals at the whole second, floored: within 1 s below.
    assert drive - 1.0 < t["vehicle_s"][0] <= drive
    assert t["access_egress_s"][0] == 420.0
    assert t["travel_time_s"][0] == t["vehicle_s"][0] + 420.0
    assert t["arrival_s"][0] == departure + t["travel_time_s"][0]
    m = json.loads((tmp_path / "x" / "manifest.json").read_text())
    assert (m["parameters"]["car_access_s"], m["parameters"]["car_egress_s"]) == (120.0, 300.0)
    assert run.convergence()["mean_trip_s"][-1] == t["travel_time_s"][0]


def test_the_choice_weighs_the_fixed_times_as_walking_and_parking(tmp_path):
    pytest.importorskip("pyarrow")
    # The logit's defaults: a minute in the car costs 0.2, walking 0.2 + 0.13 = 0.33, parking
    # 0.2 + beta_parking_min (0 by default; here -0.1, so 0.3). A chain's cost is then
    # 0.2 × drive/60 + 0.33 × 120/60 + 0.3 × 300/60, its time drive + 420 s.
    net = two_routes()
    run = scenario(
        net,
        cars(net, 1, 60),
        mode_options=None,
        choice_model="logit",
        choice_options={"beta_parking_min": -0.1},
        flow_level=0,
        equilibration="free_flow",
    ).run("x", output_dir=str(tmp_path / "x"), quiet=True, engine="one-loop")
    ids = net.link_ids()
    ff = np.asarray(net.link_free_flow_s())
    drive = ff[ids.index("op")] + ff[ids.index("pd")]
    c = run.chains()
    assert c["travel_time_s"][0] == pytest.approx(drive + 420.0, abs=1e-9)
    expected = 0.2 * drive / 60 + 0.33 * 120 / 60 + 0.3 * 300 / 60
    assert c["cost"][0] == pytest.approx(expected, rel=1e-12)
    k = json.loads((tmp_path / "x" / "manifest.json").read_text())["coefficients"]["car"]
    assert k["walk_per_minute"] == pytest.approx(0.33)
    assert k["parking_per_minute"] == pytest.approx(0.3)


def test_the_fixed_times_are_refused_when_not_a_number_of_seconds(tmp_path):
    net = two_routes()
    s = scenario(net, cars(net, 2, 60), mode_options={"car_access_s": -5.0})
    with pytest.raises(ValueError, match="car_access_s must be a finite number"):
        s.run("x", output_dir=str(tmp_path / "x"), quiet=True, engine="one-loop")


def test_zero_fixed_times_and_none_are_one_run(tmp_path):
    # Times of 0 are the drive alone: the fingerprint of a run without them, and other than
    # a run with them.
    net = two_routes()
    trips = cars(net, 50, 10)
    zero = scenario(net, trips).run(
        "a", output_dir=str(tmp_path / "a"), quiet=True, engine="one-loop"
    )
    paid = scenario(net, trips, mode_options=None).run(
        "b", output_dir=str(tmp_path / "b"), quiet=True, engine="one-loop"
    )
    assert zero.fingerprint != paid.fingerprint
    t0, t1 = zero.trip_results(), paid.trip_results()
    assert np.all(np.asarray(t0["access_egress_s"]) == 0.0)
    assert np.all(np.asarray(t1["access_egress_s"]) == 420.0)


# --- a run continued from another --------------------------------------------------------


def stopped_run(tmp_path, name, **kw):
    # 1 800 veh/h for half an hour on the two routes, the logit, given 200 iterations to stop
    # by its own rule.
    net = kw.pop("net", None) or two_routes()
    s = scenario(
        net,
        cars(net, 900, 2),
        mode_options=None,
        choice_model="logit",
        equilibration="msa",
        equilibration_options={"iterations": 200},
        **kw,
    )
    return s, s.run(name, output_dir=str(tmp_path / name), quiet=True, engine="one-loop")


def test_a_run_continued_from_its_stop_moves_nobody_beyond_chance(tmp_path):
    pytest.importorskip("pyarrow")
    # The property (idempotence): the first run stopped by its own rule, so at its final state
    # the search finds a chain beyond the band for at most found_tolerance of the trips and the
    # expected switch excess is within the switch tolerance. A run continued from it starts from
    # that state (its sets, chains, probabilities and link times), so its first iteration finds
    # no more and keeps the expected excess within the tolerance; its iterations are numbered on.
    s, first = stopped_run(tmp_path, "first")
    assert first.converged, "the first run stops by its own rule"
    n = len(first.convergence()["iteration"])
    s2 = s.replace(equilibration_options={"iterations": 3, "min_iterations": 3})
    again = s2.run(
        "again", output_dir=str(tmp_path / "again"), quiet=True, engine="one-loop", start_from=first
    )
    c = again.convergence()
    assert list(c["iteration"]) == [n, n + 1, n + 2]
    assert c["found_share"][0] <= 0.005
    assert abs(c["switch_excess_expected"][0]) <= 0.05
    m = json.loads((tmp_path / "again" / "manifest.json").read_text())
    assert m["start_from"]["run_fingerprint"] == first.fingerprint
    assert m["start_from"]["iterations"] == n
    # Read back from its folder, the run is a start just as well.
    opened = ms.Run.open(tmp_path / "first")
    third = s2.run(
        "third",
        output_dir=str(tmp_path / "third"),
        quiet=True,
        engine="one-loop",
        start_from=opened,
    )
    assert third.fingerprint == again.fingerprint
    assert list(third.convergence()["gap"]) == list(c["gap"])


def test_a_run_to_continue_from_that_does_not_fit_is_refused(tmp_path):
    s, first = stopped_run(tmp_path, "first")
    other = scenario(two_routes(), cars(two_routes(), 800, 2), mode_options=None)
    with pytest.raises(ValueError, match="not of this scenario"):
        other.run(
            "o", output_dir=str(tmp_path / "o"), quiet=True, engine="one-loop", start_from=first
        )
    current = s.run("cur", output_dir=str(tmp_path / "cur"), quiet=True)
    with pytest.raises(ValueError, match="ran on the current engine"):
        s.run(
            "c", output_dir=str(tmp_path / "c"), quiet=True, engine="one-loop", start_from=current
        )
    with pytest.raises(ValueError, match='engine="one-loop"'):
        s.run("d", output_dir=str(tmp_path / "d"), quiet=True, start_from=first)
    (tmp_path / "first" / "chains.parquet").unlink()
    with pytest.raises(ValueError, match="no chains table"):
        s.run("e", output_dir=str(tmp_path / "e"), quiet=True, engine="one-loop", start_from=first)


# --- the chains by the network's ids -----------------------------------------------------


def test_a_relabelled_network_gives_the_same_chains_by_id(tmp_path):
    pytest.importorskip("pyarrow")
    # The two routes with every link and node renamed and the links listed in another order:
    # the network's indices change, its ids map one to one. Mapped back, every trip's chains
    # are the same, in the same order, with the same choice (a property: invariance to labels).
    net = two_routes()
    rename = {"op": "z9", "pd": "a1", "oq": "m5", "qd": "b7"}
    nodes = [
        {"id": "nO", "x": 4.80, "y": LAT},
        {"id": "nP", "x": 4.80 + 2 * KM, "y": LAT + 0.004},
        {"id": "nQ", "x": 4.80 + 2 * KM, "y": LAT - 0.004},
        {"id": "nD", "x": 4.80 + 4 * KM, "y": LAT},
    ]
    spec = [
        ("qd", "Q", "D", 3200, 6000.0),
        ("op", "O", "P", 2000, 1200.0),
        ("pd", "P", "D", 2000, 6000.0),
        ("oq", "O", "Q", 3200, 1800.0),
    ]
    links = [
        {
            "id": rename[i],
            "from": "n" + a,
            "to": "n" + b,
            "class": "primary",
            "lanes": 3,
            "length": m,
            "free_flow_speed": 50,
            "capacity": c,
        }
        for i, a, b, m, c in spec
    ]
    relabelled = ms.network_read_table(links, nodes)
    assert relabelled.link_ids() != net.link_ids()
    trips = cars(net, 900, 2)
    kw = {
        "choice_model": "logit",
        "equilibration": "msa",
        "equilibration_options": {"iterations": 8},
        "mode_options": None,
    }
    a = scenario(net, trips, **kw).run(
        "a", output_dir=str(tmp_path / "a"), quiet=True, engine="one-loop"
    )
    b = scenario(relabelled, trips, **kw).run(
        "b", output_dir=str(tmp_path / "b"), quiet=True, engine="one-loop"
    )
    back = {v: k for k, v in rename.items()}
    ca, cb = a.chains(), b.chains()
    assert ca["links"] == [[back[i] for i in x] for x in cb["links"]]
    for k in ("trip", "chain", "taken", "probability", "cost", "travel_time_s"):
        assert np.array_equal(np.asarray(ca[k]), np.asarray(cb[k])), k


def test_one_thread_and_several_give_the_same_run(tmp_path):
    net = two_routes()
    s = scenario(
        net,
        cars(net, 900, 2),
        mode_options=None,
        choice_model="logit",
        equilibration="msa",
        equilibration_options={"iterations": 5},
        master_seed=3,
    )
    runs = [
        s.run(f"t{n}", output_dir=str(tmp_path / f"t{n}"), quiet=True, engine="one-loop", threads=n)
        for n in (1, 2, None)
    ]
    first = runs[0]
    for r in runs[1:]:
        assert r.fingerprint == first.fingerprint
        for k in ("gap", "switch_share", "mean_trip_s"):
            assert list(r.convergence()[k]) == list(first.convergence()[k]), k
        assert np.array_equal(
            np.asarray(r.trip_results()["travel_time_s"]),
            np.asarray(first.trip_results()["travel_time_s"]),
        )
