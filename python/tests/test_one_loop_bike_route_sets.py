"""Sampled bike route sets on the one-loop engine (``bike_route_choice="doubly_stochastic"``).

What is defended, each from a value derived by hand or a property that must hold. Three routes
from O to D share no link: A, a cycleway of 2 × 1 500 m (18 km/h: 10.0 min); B, a residential
street of 2 × 1 300 m (15 km/h: 10.4 min); C, one of 2 × 1 400 m (11.2 min). The logit weighs a
minute riding −0.2 and a kilometre in mixed traffic −0.16, so A, B and C have the utilities
−2.000, −2.496 and −2.688 and, sharing no link (path size 1), the probabilities 0.47359,
0.28840 and 0.23802. Every stated bike trip's set holds A, B and C, best first, with those
probabilities; ``bike_route_sets()`` and ``bike_route_choices()`` give the sets and the routes
taken; a trip choosing its mode has every bike route as a chain; the fingerprint and the manifest
name the option; ``"shortest"`` stays the default.
"""

import math

import numpy as np
import openmobisim as ms
import pytest

LAT = 45.70
KM = 1 / (111.32 * math.cos(math.radians(LAT)))
OPTIONS = {"draws": 32, "sigma": 3.0}


def network():
    nodes = [
        {"id": "O", "x": 4.80, "y": LAT},
        {"id": "D", "x": 4.80 + 2.5 * KM, "y": LAT},
        {"id": "A", "x": 4.80 + 1.25 * KM, "y": LAT + 0.008},
        {"id": "B", "x": 4.80 + 1.25 * KM, "y": LAT - 0.004},
        {"id": "C", "x": 4.80 + 1.25 * KM, "y": LAT - 0.009},
    ]

    def link(i, a, b, metres, cls):
        return {"id": i, "from": a, "to": b, "class": cls, "lanes": 1, "length": metres}

    links = [
        link("oa", "O", "A", 1500, "cycleway"),
        link("ad", "A", "D", 1500, "cycleway"),
        link("ob", "O", "B", 1300, "residential"),
        link("bd", "B", "D", 1300, "residential"),
        link("oc", "O", "C", 1400, "residential"),
        link("cd", "C", "D", 1400, "residential"),
    ]
    return ms.network_read_table(links, nodes)


def derived():
    """The logit's probabilities of A, B and C (see the module docstring)."""
    minutes = np.array([3.0 / 18.0, 2.6 / 15.0, 2.8 / 15.0]) * 60
    mixed_km = np.array([0.0, 2.6, 2.8])
    u = -0.2 * minutes - 0.16 * mixed_km
    return np.exp(u) / np.exp(u).sum()


def rows(net, n, mode, prefix):
    o, d = net.node_lonlat("O"), net.node_lonlat("D")
    return [
        (f"{prefix}{i:03d}", 0, o[0], o[1], d[0], d[1], 7 * 3600 + i, "c", None, mode)
        for i in range(n)
    ]


def run(net, trips, path, **kw):
    kw.setdefault("bike_route_choice", "doubly_stochastic")
    if kw["bike_route_choice"] != "shortest":
        kw.setdefault("bike_route_options", OPTIONS)
    s = ms.Scenario.from_parts(
        net,
        trips,
        classes={"c": (True, True, False)},
        choice_model="logit",
        equilibration="free_flow",
        window=(7 * 3600, 10 * 3600),
        mode_options={"bike_access_s": 0, "bike_egress_s": 0},
        master_seed=11,
        **kw,
    )
    return s.run(path.name, output_dir=str(path), quiet=True, engine="one-loop")


def test_every_bike_trip_chooses_among_the_three_routes_by_the_logit(tmp_path):
    pytest.importorskip("pyarrow")
    net = network()
    r = run(net, rows(net, 50, "bike", "b"), tmp_path / "x")
    sets, choices = r.bike_route_sets(), r.bike_route_choices()
    assert sets is not None and sets.method == "sampled"
    assert (sets.key_count, sets.route_count) == (1, 3)
    assert np.all(choices.alternatives == 3)
    assert np.all((choices.route >= 0) & (choices.rank <= 2))
    c = r.chains()
    links = [tuple(x) for x in c["links"]]
    trip0 = np.asarray(c["trip"]) == 0
    assert [x for x, m in zip(links, trip0, strict=True) if m] == [
        ("oa", "ad"),
        ("ob", "bd"),
        ("oc", "cd"),
    ], "A, B and C, best first"
    p = np.asarray(c["probability"]).reshape(50, 3)
    assert np.allclose(p, derived(), rtol=0, atol=1e-12)
    # The route each trip took is its chain taken, and its probability the model's.
    taken = np.asarray(c["taken"]).reshape(50, 3)
    assert np.array_equal(np.argmax(taken, axis=1), choices.rank)
    assert np.allclose(choices.probability, derived()[choices.rank], rtol=0, atol=1e-12)
    params = r.manifest()["parameters"]
    assert params["bike_route_choice"] == "doubly_stochastic"
    options = params["bike_route_options"]
    assert (options["draws"], options["sigma"], options["tau"]) == (32, 3.0, 0.3)


def test_a_trip_choosing_its_mode_has_every_bike_route(tmp_path):
    pytest.importorskip("pyarrow")
    net = network()
    r = run(net, rows(net, 5, None, "m"), tmp_path / "x", modes=["car", "bike"])
    c = r.chains()
    modes = np.asarray(c["mode"], dtype=object)[np.asarray(c["trip"]) == 0]
    assert list(modes).count("bike") == 3 and modes[0] == "car"
    choices = r.bike_route_choices()
    assert np.all(choices.route == -1), "a trip choosing its mode is in chains(), not here"


def test_the_option_is_named_by_the_fingerprint_and_shortest_is_the_default(tmp_path):
    net = network()
    trips = rows(net, 5, "bike", "b")
    plain = run(net, trips, tmp_path / "a", bike_route_choice="shortest")
    assert plain.bike_route_sets() is None and plain.bike_route_choices() is None
    default = ms.Scenario.from_parts(
        net,
        trips,
        classes={"c": (True, True, False)},
        choice_model="logit",
        equilibration="free_flow",
        window=(7 * 3600, 10 * 3600),
        mode_options={"bike_access_s": 0, "bike_egress_s": 0},
        master_seed=11,
    ).run("d", output_dir=str(tmp_path / "d"), quiet=True, engine="one-loop")
    assert default.fingerprint == plain.fingerprint
    sampled = run(net, trips, tmp_path / "b")
    assert sampled.fingerprint != plain.fingerprint
    with pytest.warns(DeprecationWarning, match='"doubly_stochastic"'):
        old = run(net, trips, tmp_path / "c", bike_route_choice="sampled")
    assert old.fingerprint == sampled.fingerprint
    other = run(net, trips, tmp_path / "e", bike_route_options={"draws": 32, "sigma": 2.0})
    assert other.fingerprint != sampled.fingerprint
    with pytest.raises(ValueError, match="draws"):
        run(net, trips, tmp_path / "f", bike_route_options={"draws": 99})
