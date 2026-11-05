"""Bike and walking trips whose mode is stated, on the one-loop engine (``engine="one-loop"``).

What is defended, each from a value derived by hand or a property that must hold: a bike trip on
a cycleway takes 60 s to reach and unlock the bike, its ride (the length at 18 km/h) and 60 s to
park and lock it, door to door; a walk takes its length at 4.8 km/h; the bike cost
(``bike_cost``) chooses between a cycleway and a mixed street as worked out by hand; bike trips
on a cycleway change no car result (trip table, chains, road link bins), and every bike crosses
each link of its route once in the bike layer's link bins; the run is the same on 1, 2 and every
thread; the folder is read back with the trips by mode; a run continued from one with bike trips
rebuilds their chains. Sampled bike route sets: ``test_one_loop_bike_route_sets.py``.
"""

import json
import math

import numpy as np
import openmobisim as ms
import pytest

LAT = 45.70
KM = 1 / (111.32 * math.cos(math.radians(LAT)))
#: The bike layer's speeds (the shipped defaults) and the walking speed, km/h.
BIKE_DEDICATED_KM_H, BIKE_MIXED_KM_H, WALK_KM_H = 18.0, 15.0, 4.8


def network(street_m=1800.0):
    """O → D by a cycleway, a residential street and a primary road.

    The cycleway runs through A (2 × 1 000 m), the street through B (2 × ``street_m / 2``, both
    ways), the primary road, for the cars, through C (2 × 1 500 m).
    """
    nodes = [
        {"id": "O", "x": 4.80, "y": LAT},
        {"id": "A", "x": 4.80 + 0.9 * KM, "y": LAT + 0.005},
        {"id": "B", "x": 4.80 + 0.9 * KM, "y": LAT - 0.003},
        {"id": "C", "x": 4.80 + 0.9 * KM, "y": LAT - 0.010},
        {"id": "D", "x": 4.80 + 1.8 * KM, "y": LAT},
    ]

    def link(i, a, b, metres, cls, **kw):
        return {"id": i, "from": a, "to": b, "class": cls, "lanes": 1, "length": metres, **kw}

    half = street_m / 2
    links = [
        link("oa", "O", "A", 1000, "cycleway"),
        link("ad", "A", "D", 1000, "cycleway"),
        link("ob", "O", "B", half, "residential"),
        link("bd", "B", "D", half, "residential"),
        link("bo", "B", "O", half, "residential"),
        link("db", "D", "B", half, "residential"),
        link("oc", "O", "C", 1500, "primary", free_flow_speed=50, capacity=900.0),
        link("cd", "C", "D", 1500, "primary", free_flow_speed=50, capacity=1800.0),
    ]
    return ms.network_read_table(links, nodes)


def trips(net, n, every_s, mode, prefix, start_s=7 * 3600):
    o, d = net.node_lonlat("O"), net.node_lonlat("D")
    return [
        (f"{prefix}{i:04d}", 0, o[0], o[1], d[0], d[1], start_s + i * every_s, "c", None, mode)
        for i in range(n)
    ]


def scenario(net, rows, **kw):
    kw.setdefault("classes", {"c": (True, True, False)})
    kw.setdefault("choice_model", "deterministic")
    kw.setdefault("window", (7 * 3600, 10 * 3600))
    return ms.Scenario.from_parts(net, rows, **kw)


def run(s, path, **kw):
    return s.run(path.name, output_dir=str(path), quiet=True, engine="one-loop", **kw)


# --- one bike, one walk ------------------------------------------------------------------------


@pytest.mark.parametrize("level", [0, 2])
def test_a_bike_trip_takes_its_ride_and_its_fixed_times(tmp_path, level):
    pytest.importorskip("pyarrow")
    # Derived: the ride is 2 × 1 000 m at 18 km/h = 400 s on the cycleway; with the shipped
    # 60 s to reach and unlock the bike and 60 s to park and lock it, 520 s door to door.
    net = network()
    ride = 2000 / (BIKE_DEDICATED_KM_H / 3.6)
    r = run(scenario(net, trips(net, 1, 0, "bike", "b"), flow_level=level), tmp_path / "x")
    t = r.trip_results()
    assert list(t["mode"]) == ["bike"] and list(t["outcome"]) == ["completed"]
    assert t["vehicle_s"][0] == pytest.approx(ride, abs=1e-9)
    assert t["access_egress_s"][0] == 120.0
    assert t["travel_time_s"][0] == pytest.approx(60 + ride + 60, abs=1e-9)
    c = r.chains()
    assert list(c["mode"]) == ["bike"] and c["links"] == [["oa", "ad"]]
    # Its cost at the deterministic choice's coefficients (a minute costs 1 whatever it is spent
    # on): the door-to-door minutes.
    assert c["cost"][0] == pytest.approx((60 + ride + 60) / 60, rel=1e-12)
    m = json.loads((tmp_path / "x" / "manifest.json").read_text())
    p = m["parameters"]
    assert (p["bike_access_s"], p["bike_egress_s"], p["bike_cost"]) == (60.0, 60.0, "utility")
    assert r.completion_by_mode["bike"]["completed"] == 1
    assert r.total_vehicle_time_s == pytest.approx(ride, abs=1e-9)


def test_a_walk_takes_its_length_at_the_walking_speed(tmp_path):
    # Derived: the cycleway is not walked; the street, 1 800 m at 4.8 km/h, is 1 350 s.
    net = network()
    r = run(scenario(net, trips(net, 1, 0, "walk", "w")), tmp_path / "x")
    t = r.trip_results()
    assert list(t["mode"]) == ["walk"]
    assert t["travel_time_s"][0] == pytest.approx(1800 / (WALK_KM_H / 3.6), abs=1e-9)
    assert (t["vehicle_s"][0], t["access_egress_s"][0]) == (0.0, 0.0)


@pytest.mark.parametrize(
    ("street_m", "bike_cost", "model", "expected"),
    [
        # Street 1 800 m: 432 s against the cycleway's 400 s, by every cost the cycleway.
        (1800, "time", "deterministic", ["oa", "ad"]),
        (1800, "dedicated", "deterministic", ["oa", "ad"]),
        (1800, "utility", "logit", ["oa", "ad"]),
        # Street 1 600 m: 384 s, faster than the cycleway; × 1.2 in mixed traffic 460.8 > 400;
        # the logit's utility 0.2 × 6.4 + 0.16 × 1.6 = 1.536 > 0.2 × 6.667 = 1.333; the
        # deterministic choice weighs time alone (no weight on mixed traffic): the street.
        (1600, "time", "deterministic", ["ob", "bd"]),
        (1600, "dedicated", "deterministic", ["oa", "ad"]),
        (1600, "utility", "logit", ["oa", "ad"]),
        (1600, "utility", "deterministic", ["ob", "bd"]),
    ],
)
def test_the_bike_cost_chooses_the_route(tmp_path, street_m, bike_cost, model, expected):
    pytest.importorskip("pyarrow")
    net = network(street_m)
    s = scenario(net, trips(net, 1, 0, "bike", "b"), bike_cost=bike_cost, choice_model=model)
    assert run(s, tmp_path / "x").chains()["links"] == [expected]


# --- bikes beside cars -------------------------------------------------------------------------


def test_bikes_on_a_cycleway_change_no_car_result(tmp_path):
    pytest.importorskip("pyarrow")
    # 1 200 veh/h for 30 min onto the primary road (900 PCU/h on its first link) or the street,
    # with and without 200 bikes on the cycleway; the bikes' travellers sort after the cars'.
    # Property: every car trip's row, every car chain and the road link bins are equal to the
    # bit. And every bike that finished crosses each of its two links once: the bike layer's
    # link bins count 2 crossings per bike, each bike's ride at 200 s a link.
    net = network()
    car_rows = trips(net, 600, 3, "car", "car")
    bike_rows = trips(net, 200, 9, "bike", "zbike")
    kw = {"flow_level": 2, "link_bin_s": 900, "choice_model": "logit", "master_seed": 5}
    kw["equilibration_options"] = {"iterations": 6, "min_iterations": 6}
    alone = run(scenario(net, car_rows, **kw), tmp_path / "alone")
    both = run(scenario(net, car_rows + bike_rows, **kw), tmp_path / "both")
    ta, tb = alone.trip_results(), both.trip_results()
    n = len(car_rows)
    for k in ("traveller_id", "outcome"):
        assert list(ta[k]) == list(tb[k])[:n], k
    for k in ("travel_time_s", "vehicle_s", "arrival_s"):
        a, b = np.asarray(ta[k], dtype=float), np.asarray(tb[k], dtype=float)[:n]
        assert np.array_equal(a, b, equal_nan=True), k
    ca, cb = alone.chains(), both.chains()
    car = np.asarray(cb["mode"]) == "car"
    assert ca["links"] == [x for x, m in zip(cb["links"], car, strict=True) if m]
    for k in ("trip", "chain", "taken", "probability", "cost", "travel_time_s"):
        assert np.array_equal(np.asarray(ca[k]), np.asarray(cb[k])[car]), k
    la, lb = (
        alone.link_bins_table().to_arrow().to_pydict(),
        both.link_bins_table().to_arrow().to_pydict(),
    )
    for k in ("bin", "link", "crossings", "pcu", "pcu_seconds"):
        assert la[k] == lb[k], k
    bikes = both.link_bins_table("bike").to_arrow().to_pydict()
    finished = both.completion_by_mode["bike"]["completed"]
    assert finished == 200
    assert sum(bikes["crossings"]) == 2 * finished
    assert sorted(set(bikes["link"])) == ["ad", "oa"]
    assert sum(bikes["traveller_seconds"]) == pytest.approx(finished * 400.0, abs=1e-6)
    assert alone.link_bins_table("bike") is None


def test_one_thread_and_several_give_the_same_run(tmp_path):
    net = network()
    rows = trips(net, 300, 4, "car", "car") + trips(net, 300, 4, "bike", "zb")
    rows += trips(net, 300, 4, "walk", "zw")
    s = scenario(
        net, rows, choice_model="logit", master_seed=3, equilibration_options={"iterations": 4}
    )
    runs = [run(s, tmp_path / f"t{n}", threads=n) for n in (1, 2, None)]
    first = runs[0]
    for r in runs[1:]:
        assert r.fingerprint == first.fingerprint
        for k in ("gap", "switch_share", "mean_trip_s"):
            assert list(r.convergence()[k]) == list(first.convergence()[k]), k
        assert np.array_equal(
            np.asarray(r.trip_results()["travel_time_s"]),
            np.asarray(first.trip_results()["travel_time_s"]),
        )


# --- the folder, a run continued, the fingerprint, what is refused ------------------------------


def test_the_folder_is_read_back_with_the_trips_by_mode(tmp_path):
    pytest.importorskip("pyarrow")
    net = network()
    rows = trips(net, 3, 60, "car", "c") + trips(net, 2, 60, "bike", "zb")
    rows += trips(net, 1, 60, "walk", "zw")
    r = run(scenario(net, rows, link_bin_s=900), tmp_path / "x")
    back = ms.Run.open(tmp_path / "x")
    assert back.completion_by_mode == r.completion_by_mode
    assert set(r.completion_by_mode) == {"car", "bike", "walk"}
    assert back.total_vehicle_time_s == pytest.approx(r.total_vehicle_time_s)
    assert (
        back.link_bins_table("walk").to_arrow().to_pydict()["link"]
        == r.link_bins_table("walk").to_arrow().to_pydict()["link"]
    )
    assert r.bike_route_sets() is None


def test_a_run_continued_from_one_with_bike_trips_rebuilds_them(tmp_path):
    pytest.importorskip("pyarrow")
    net = network()
    rows = trips(net, 50, 20, "car", "c") + trips(net, 20, 30, "bike", "zb")
    rows += trips(net, 10, 30, "walk", "zw")
    s = scenario(net, rows, flow_level=2, equilibration_options={"iterations": 3})
    first = run(s, tmp_path / "a")
    more = run(s, tmp_path / "b", start_from=first)
    ca, cb = first.chains(), more.chains()
    bike = np.asarray(ca["mode"]) != "car"
    assert [x for x, m in zip(ca["links"], bike, strict=True) if m] == [
        x for x, m in zip(cb["links"], np.asarray(cb["mode"]) != "car", strict=True) if m
    ]
    assert list(more.convergence()["iteration"]) == [3, 4, 5]


def test_the_layers_enter_the_fingerprint_only_with_bike_or_walking_trips(tmp_path):
    net = network()
    cars_only = trips(net, 5, 60, "car", "c")
    a = run(scenario(net, cars_only), tmp_path / "a")
    b = run(scenario(net, cars_only, bike_cost="time"), tmp_path / "b")
    assert a.fingerprint == b.fingerprint, "no bike trip: the bike cost changes nothing"
    with_bike = cars_only + trips(net, 1, 60, "bike", "zb")
    c = run(scenario(net, with_bike), tmp_path / "c")
    d = run(scenario(net, with_bike, bike_cost="time"), tmp_path / "d")
    assert c.fingerprint != d.fingerprint


def test_a_traveller_without_a_bike_has_no_bike_trip(tmp_path):
    net = network()
    s = scenario(net, trips(net, 1, 0, "bike", "b"), classes={"c": (True, False, False)})
    r = run(s, tmp_path / "x")
    assert list(r.trip_results()["outcome"]) == ["no_vehicle_available"]


def test_the_select_link_analysis_counts_every_bike_on_the_cycleway(tmp_path):
    # Every bike rides the cycleway (oa, ad): the select-link analysis on oa counts each of them
    # once, as bikes, and no walker (the cycleway is not walked).
    net = network()
    rows = trips(net, 4, 60, "car", "c") + trips(net, 5, 60, "bike", "zb")
    rows += trips(net, 3, 60, "walk", "zw")
    r = run(scenario(net, rows, link_bin_s=900), tmp_path / "x")
    oa = net.layer("bike").link_ids().index("oa")
    picked = r.link_select(oa, "bike")
    assert list(picked["mode"]) == ["bike"] and list(picked["trips"]) == [5]
