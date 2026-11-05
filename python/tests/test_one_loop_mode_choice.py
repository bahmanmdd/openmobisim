"""Trips choosing their mode among the car, the bike and walking on the one-loop engine.

What is defended, each from a value derived by hand or a property that must hold: on a car road of
10 minutes, a cycleway of 12 and a footway of 40 (no fixed time, 0.1 utility units a minute of
any kind), every trip's choice set holds one chain of each mode and the logit gives them the
probabilities ``e^V / Σ e^V`` of ``V = −1.0, −1.2, −4.0``; the nested logit is the default with
several modes, with its nest scale ``mu`` 0.5 in the manifest; a mode's constant moves the
probabilities by its formula; a walk or a ride longer than the limit (the run's or the class's) is
left out and counted, a class without a car has no car chain, a trip with no alternative does not
travel; the convergence table gives each mode's shares beside the totals, ``nan`` where a mode's
group is empty; one mode offered is the run without mode choice, to the fingerprint; the folder is
read back and a run continued from one with mode choice rebuilds every chain; a class limit the
engine does not have is refused by name.
"""

import math

import numpy as np
import openmobisim as ms
import pytest

LAT = 45.70
KM = 1 / (111.32 * math.cos(math.radians(LAT)))
MODES = ["car", "bike", "walk"]
#: 0.1 utility units a minute of any kind, no fixed time around the car or the bike, and every
#: walk offered (the shipped longest walk, 30 minutes, would leave out the 40-minute walk).
FLAT = {"beta_time_min": -0.1, "beta_walk_min": 0.0, "beta_parking_min": 0.0}
NO_FIXED = {
    "car_access_s": 0,
    "car_egress_s": 0,
    "bike_access_s": 0,
    "bike_egress_s": 0,
    "walk_max_s": 7200,
}


def network():
    """O → D by a trunk road, a cycleway and a footway.

    The trunk road runs through C (2 × 7 500 m at 90 km/h: 600 s), the cycleway through A
    (2 × 1 800 m at 18 km/h: 720 s), the footway through W (2 × 1 600 m at 4.8 km/h: 2 400 s).
    """
    nodes = [
        {"id": "O", "x": 4.80, "y": LAT},
        {"id": "C", "x": 4.80 + 1.5 * KM, "y": LAT - 0.014},
        {"id": "A", "x": 4.80 + 1.5 * KM, "y": LAT + 0.007},
        {"id": "W", "x": 4.80 + 1.5 * KM, "y": LAT + 0.003},
        {"id": "D", "x": 4.80 + 3.0 * KM, "y": LAT},
    ]

    def link(i, a, b, metres, cls, **kw):
        return {"id": i, "from": a, "to": b, "class": cls, "lanes": 1, "length": metres} | kw

    links = [
        link("oc", "O", "C", 7500, "trunk", free_flow_speed=90, capacity=7200.0, lanes=4),
        link("cd", "C", "D", 7500, "trunk", free_flow_speed=90, capacity=7200.0, lanes=4),
        link("oa", "O", "A", 1800, "cycleway"),
        link("ad", "A", "D", 1800, "cycleway"),
        link("ow", "O", "W", 1600, "footway"),
        link("wd", "W", "D", 1600, "footway"),
    ]
    return ms.network_read_table(links, nodes)


def trips(net, n, every_s, cls="c", prefix="t", mode=None):
    o, d = net.node_lonlat("O"), net.node_lonlat("D")
    return [
        (f"{prefix}{i:04d}", 0, o[0], o[1], d[0], d[1], 7 * 3600 + i * every_s, cls, None, mode)
        for i in range(n)
    ]


def scenario(net, rows, **kw):
    kw.setdefault("classes", {"c": {"owns_car": 1, "owns_bike": 1}})
    kw.setdefault("modes", MODES)
    kw.setdefault("choice_options", dict(FLAT))
    kw.setdefault("mode_options", dict(NO_FIXED))
    kw.setdefault("window", (7 * 3600, 10 * 3600))
    kw.setdefault("equilibration", "free_flow")
    return ms.Scenario.from_parts(net, rows, **kw)


def run(s, path, **kw):
    return s.run(path.name, output_dir=str(path), quiet=True, engine="one-loop", **kw)


def logit(values):
    e = np.exp(np.asarray(values))
    return e / e.sum()


def test_the_case_is_as_stated():
    net = network()
    ff = dict(zip(net.link_ids(), np.asarray(net.link_free_flow_s()), strict=True))
    assert ff["oc"] + ff["cd"] == pytest.approx(600.0)
    bike = net.layer("bike")
    secs = dict(zip(bike.link_ids(), np.asarray(bike.link_length_m()) / (18 / 3.6), strict=True))
    assert secs["oa"] + secs["ad"] == pytest.approx(720.0)


@pytest.mark.parametrize("model", ["logit", "nested_logit"])
def test_three_modes_are_chosen_by_the_formula(tmp_path, model):
    pytest.importorskip("pyarrow")  # chains() reads its table
    net = network()
    r = run(scenario(net, trips(net, 30, 10), choice_model=model), tmp_path / "x")
    chains = r.chains()
    formula = logit([-1.0, -1.2, -4.0])
    mode = np.asarray(chains["mode"])
    trip = np.asarray(chains["trip"])
    p = np.asarray(chains["probability"])
    for t in range(30):
        assert list(mode[trip == t]) == MODES
        np.testing.assert_allclose(p[trip == t], formula, atol=1e-12)
    np.testing.assert_allclose(np.asarray(chains["cost"])[trip == 0], [1.0, 1.2, 4.0], atol=1e-12)
    tr = r.trip_results()
    assert all(tr["mode_choice"])
    taken = set(tr["mode"])
    assert taken <= set(MODES)
    assert r.manifest()["choice_model"] == model


def test_the_nested_logit_is_the_default_with_several_modes(tmp_path):
    net = network()
    r = run(scenario(net, trips(net, 5, 10), choice_model=None), tmp_path / "x")
    m = r.manifest()
    assert m["choice_model"] == "nested_logit"
    assert m["parameters"]["nest_scale"] == 0.5
    assert m["parameters"]["mode_choice"]["c"]["modes"] == MODES


def test_a_modes_constant_moves_the_probabilities_by_its_formula(tmp_path):
    pytest.importorskip("pyarrow")  # chains() reads its table
    net = network()
    options = {**FLAT, "beta_mode_bike": 0.2, "beta_mode_walk": -1.0}
    r = run(
        scenario(net, trips(net, 3, 10), choice_model="logit", choice_options=options),
        tmp_path / "x",
    )
    c = r.chains()
    p = np.asarray(c["probability"])[np.asarray(c["trip"]) == 0]
    np.testing.assert_allclose(p, logit([-1.0, -1.0, -5.0]), atol=1e-12)


def test_modes_that_cannot_serve_a_trip_are_left_out_and_counted(tmp_path):
    pytest.importorskip("pyarrow")  # chains() reads its table
    net = network()
    rows = trips(net, 4, 10, "c", "a") + trips(net, 3, 10, "nocar", "b")
    rows += trips(net, 2, 10, "stuck", "s")
    classes = {
        "c": {"owns_car": 1, "owns_bike": 1},
        "nocar": {"owns_car": 0, "owns_bike": 1, "bike_max_s": 600},
        "stuck": {"owns_car": 0, "owns_bike": 0},
    }
    # The shipped longest walk, 30 minutes, under the walk's 40: no walk is offered.
    s = scenario(
        net,
        rows,
        classes=classes,
        choice_model="logit",
        mode_options={k: v for k, v in NO_FIXED.items() if k != "walk_max_s"},
    )
    r = run(s, tmp_path / "x")
    left = r.manifest()["modes_left_out"]
    assert left == {
        "choosing": 9,
        "no_chain": 5,
        "car_no_vehicle": 5,
        "car_no_route": 0,
        "bike_no_vehicle": 2,
        "bike_no_route": 0,
        "bike_too_long": 3,
        "walk_no_route": 0,
        "walk_too_long": 9,
    }
    tr = r.trip_results()
    outcome = dict(zip(tr["traveller_id"], tr["outcome"], strict=True))
    mode = dict(zip(tr["traveller_id"], tr["mode"], strict=True))
    assert all(outcome[f"b{i:04d}"] == "no_alternative" for i in range(3))
    assert all(mode[f"b{i:04d}"] is None for i in range(3))
    c = r.chains()
    first = np.asarray(c["trip"]) == 0  # the first trip is a0000's (traveller ids sort)
    assert list(np.asarray(c["mode"])[first]) == ["car", "bike"], "the walk is left out"
    assert all(mode[f"a{i:04d}"] in ("car", "bike") for i in range(4))
    assert r.completion["no_feasible_path"] == 5


def test_each_modes_shares_stand_beside_the_totals(tmp_path):
    pq = pytest.importorskip("pyarrow.parquet")
    net = network()
    s = scenario(
        net,
        trips(net, 200, 9),
        choice_model="logit",
        flow_level=2,
        equilibration="msa",
        equilibration_options={"iterations": 4, "min_iterations": 4},
    )
    r = run(s, tmp_path / "x")
    c = r.convergence()
    names = (
        [
            f"{k}_{m}"
            for k in ("found_share", "switch_share", "mode_share_change_expected")
            for m in MODES
        ]
        + ["mode_change_share", "choosing"]
        + [f"mode_share_{m}" for m in MODES]
    )
    on_disk = pq.read_table(tmp_path / "x" / "convergence.parquet").column_names
    for name in names:
        assert name in c and name in on_disk, name
    # Iteration 0: no trip held a chain, so no mode's group has a share.
    assert all(math.isnan(c[f"switch_share_{m}"][0]) for m in MODES)
    assert math.isnan(c["mode_change_share"][0])
    assert c["choosing"][0] == 200
    # The expected changes of the three modes' shares sum to 0: each trip holds one chain.
    change = sum(np.asarray(c[f"mode_share_change_expected_{m}"], float) for m in MODES)
    np.testing.assert_allclose(change, 0.0, atol=1e-12)
    split = [c[f"mode_share_{m}"][-1] for m in MODES]
    assert sum(split) == pytest.approx(1.0)
    tr = r.trip_results()
    assert split[1] == pytest.approx(np.mean(np.asarray(tr["mode"]) == "bike"))
    # A trip that changed mode changed chain: the mode changes are at most the switches.
    later = slice(1, None)
    assert np.all(
        np.asarray(c["mode_change_share"][later]) <= np.asarray(c["switch_share"][later]) + 1e-12
    )


def test_one_mode_offered_is_the_run_without_mode_choice(tmp_path):
    net = network()
    rows = trips(net, 6, 10)
    a = run(scenario(net, rows, modes=None, choice_model="logit"), tmp_path / "a")
    b = run(scenario(net, rows, modes=["car"], choice_model="logit"), tmp_path / "b")
    assert a.fingerprint == b.fingerprint
    assert list(b.trip_results()["mode_choice"]) == [False] * 6
    walkers = scenario(
        net,
        rows,
        classes={"c": {"owns_car": 1, "owns_bike": 1, "modes": ["walk"]}},
        choice_model="logit",
        mode_options={**NO_FIXED, "walk_max_s": 60},
    )
    w = run(walkers, tmp_path / "w")
    tr = w.trip_results()
    assert set(tr["mode"]) == {"walk"}, "a class offered one mode walks at any length"
    assert w.convergence()["choosing"][0] == 0


def test_the_folder_and_a_continued_run_keep_every_chain(tmp_path):
    pytest.importorskip("pyarrow")
    net = network()
    s = scenario(
        net,
        trips(net, 60, 30),
        choice_model="nested_logit",
        flow_level=2,
        equilibration="msa",
        equilibration_options={"iterations": 3, "min_iterations": 3},
    )
    first = run(s, tmp_path / "a")
    back = ms.Run.open(tmp_path / "a")
    assert back.completion_by_mode == first.completion_by_mode
    more = run(s, tmp_path / "b", start_from=first)
    ca, cb = first.chains(), more.chains()
    assert list(ca["mode"]) == list(cb["mode"])
    assert ca["links"] == cb["links"]
    assert list(more.convergence()["iteration"]) == [3, 4, 5]


def test_a_limit_it_does_not_have_is_refused_by_name(tmp_path):
    # (A mode other than the three needs its own inputs, a timetable or parkings, which the
    # engine refuses by name before the mode: see test_one_loop.py.)
    net = network()
    s = scenario(
        net,
        trips(net, 2, 10),
        choice_model="logit",
        classes={"c": {"owns_car": 1, "access_walk_max_s": 300}},
    )
    with pytest.raises(ms.EngineNotSupportedError, match="access_walk_max_s"):
        run(s, tmp_path / "y")
