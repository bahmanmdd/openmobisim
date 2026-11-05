"""The one-loop engine's reading of the link travel times.

The option is ``equilibration_options`` ``link_times``. What is defended: ``"bins"`` is the
default, named or not one run, and ``"fifo"`` another, recorded in the manifest and refused when
misspelt; the current engine has no such option; under ``"fifo"`` the same seed gives the same
run, and a relabelled network gives the same chains by id. The reading itself (its values
derived by hand, first-in-first-out on a loading whose queue clears) is tested where it is
built, in the Rust crate of the loading.
"""

import json
import math

import numpy as np
import openmobisim as ms
import pytest

LAT = 45.70
KM = 1 / (111.32 * math.cos(math.radians(LAT)))


def two_routes(names=None):
    """O → D by a short route (2 + 2 km, 1 200 PCU/h on its first link) or a long one.

    ``names`` renames every link and node and lists the links in another order.
    """
    rename = names or {}
    node = {n: ("n" + n if names else n) for n in "OPQD"}
    nodes = [
        {"id": node["O"], "x": 4.80, "y": LAT},
        {"id": node["P"], "x": 4.80 + 2 * KM, "y": LAT + 0.004},
        {"id": node["Q"], "x": 4.80 + 2 * KM, "y": LAT - 0.004},
        {"id": node["D"], "x": 4.80 + 4 * KM, "y": LAT},
    ]
    spec = [
        ("op", "O", "P", 2000, 1200.0),
        ("pd", "P", "D", 2000, 6000.0),
        ("oq", "O", "Q", 3200, 1800.0),
        ("qd", "Q", "D", 3200, 6000.0),
    ]
    if names:
        spec = spec[3:] + spec[:3]
    links = [
        {
            "id": rename.get(i, i),
            "from": node[a],
            "to": node[b],
            "class": "primary",
            "lanes": 3,
            "length": m,
            "free_flow_speed": 50,
            "capacity": c,
        }
        for i, a, b, m, c in spec
    ]
    return ms.network_read_table(links, nodes)


def cars(net, n, every_s):
    """``n`` cars from O to D, ``every_s`` seconds apart from 60 s.

    900 cars 2 s apart arrive faster than the short route's first link lets them out, one every
    3 s: a queue.
    """
    o, d = net.node_lonlat("O"), net.node_lonlat("D")
    return [
        (f"c{i}", 0, o[0], o[1], d[0], d[1], 60 + i * every_s, "car", None, "car") for i in range(n)
    ]


def run(tmp_path, name, net=None, trips=None, **options):
    net = net or two_routes()
    s = ms.Scenario.from_parts(
        net,
        trips or cars(net, 900, 2),
        mode_options={"car_access_s": 0.0, "car_egress_s": 0.0},
        choice_model="logit",
        master_seed=5,
        equilibration="msa",
        equilibration_options={"iterations": 6, **options},
    )
    return s.run(name, output_dir=str(tmp_path / name), quiet=True, engine="one-loop")


def test_the_reading_is_passed_through_recorded_and_refused_by_name(tmp_path):
    default = run(tmp_path, "default")
    bins = run(tmp_path, "bins", link_times="bins")
    fifo = run(tmp_path, "fifo", link_times="fifo")
    names = [
        json.loads((tmp_path / n / "manifest.json").read_text())["parameters"]["link_times"]
        for n in ("default", "bins", "fifo")
    ]
    assert names == ["bins", "bins", "fifo"]
    # The default named or not is the same run, to the bit; the FIFO reading is another run.
    assert default.fingerprint == bins.fingerprint != fifo.fingerprint
    assert list(default.convergence()["gap"]) == list(bins.convergence()["gap"])
    with pytest.raises(ValueError, match="link_times must be one of"):
        run(tmp_path, "misspelt", link_times="FIFO")


def test_the_current_engine_has_no_reading_option(tmp_path):
    net = two_routes()
    s = ms.Scenario.from_parts(
        net,
        cars(net, 5, 60),
        equilibration="msa",
        equilibration_options={"iterations": 2, "link_times": "fifo"},
    )
    # Refused by name, as the one-loop engine's other text option (``allocation``) is.
    with pytest.raises(ValueError, match="equilibration_options link_times 'fifo' is an option"):
        s.run("current", output_dir=str(tmp_path / "current"), quiet=True)


def test_the_same_seed_gives_the_same_run_under_the_fifo_reading(tmp_path):
    a = run(tmp_path, "a", link_times="fifo")
    b = run(tmp_path, "b", link_times="fifo")
    assert a.fingerprint == b.fingerprint
    assert np.array_equal(np.asarray(a.route_choices().route), np.asarray(b.route_choices().route))
    for k in ("gap", "switch_share", "mean_trip_s"):
        assert list(a.convergence()[k]) == list(b.convergence()[k]), k


def test_a_relabelled_network_gives_the_same_chains_by_id_under_the_fifo_reading(tmp_path):
    pytest.importorskip("pyarrow")
    # Every link and node renamed and the links listed in another order: the network's indices
    # change, its ids map one to one, so the run is the same by id (invariance to labels).
    rename = {"op": "z9", "pd": "a1", "oq": "m5", "qd": "b7"}
    net, relabelled = two_routes(), two_routes(rename)
    assert relabelled.link_ids() != net.link_ids()
    trips = cars(net, 900, 2)
    a = run(tmp_path, "a", net=net, trips=trips, link_times="fifo")
    b = run(tmp_path, "b", net=relabelled, trips=trips, link_times="fifo")
    back = {v: k for k, v in rename.items()}
    ca, cb = a.chains(), b.chains()
    assert ca["links"] == [[back[i] for i in x] for x in cb["links"]]
    for k in ("trip", "chain", "taken", "probability", "cost", "travel_time_s"):
        assert np.array_equal(np.asarray(ca[k]), np.asarray(cb[k])), k
