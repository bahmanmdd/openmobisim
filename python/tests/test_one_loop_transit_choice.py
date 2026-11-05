"""Transit in mode choice and fares by kind of service, on the one-loop engine.

What is defended, each from a value derived by hand or a property that must hold.

**Fares.** Stops A and B, 12 km apart on one parallel; leaving A at 08:00 with a boarding slack of
60 s: a train (``route_type`` 2, rail) A 08:05 → B 08:15, a bus (3) A 08:03 → B 08:25, one
vehicle each. A journey's fare is ``fare_base_eur`` plus, per ride, its kind's price per km times
the km from stop to stop as the crow flies, ``d = N(φ) cos φ Δλ`` along the parallel (``N`` the
WGS84 prime-vertical radius). RAPTOR's Pareto set (arrival against vehicles) holds the train only:
the bus takes as many vehicles and arrives later. With every kind at one price per km the set is
the train alone; with the train dearer per km, the query on the cheaper services alone adds the
bus, which is later but cheaper, so the set holds both, each at its fare; a traveller holding a
transit pass pays 0 and has the train alone; with the bus dearer, the cheaper query finds the
train again, which is kept once. By the default logit coefficients (−0.2 a minute in a vehicle,
−0.29 a minute waiting) and ``beta_cost_eur`` −1.2, the train costs ``0.2·10 + 0.29·5 + 1.2·fare``
and the bus ``0.2·22 + 0.29·3 + 1.2·fare``, in utility units.

**Mode choice.** From O to D: a car on a 3.6 km road at 50 km/h (259.2 s), a bike on a 3.6 km
cycleway at the bike layer's 18 km/h (720 s), a walk of 3.2 km at 4.8 km/h (2 400 s), and one
transit journey (rail, O 08:05 → D 08:15: 5 min waiting, 10 min riding, a fare of 1 €). With the
modes' constants given, V = −0.2·4.32 − 1.5 for the car, −0.2·12 for the bike, −0.33·40 + 10 for
the walk and −(0.2·10 + 0.29·5 + 1.2·1) + 1 for transit; each nest holds one chain, so the nested
logit's shares are ``exp(V_m) / Σ exp(V_n)``, and 4 000 travellers take each mode in its share
within three standard deviations of sampling. Stated transit trips priced by kind change no car
result; the same seed gives the same run on one, two and every thread; relabelling the nodes,
links, stops and lines changes nothing; an hour later, everything arrives an hour later.
"""

import math
import tempfile
from pathlib import Path

import numpy as np
import openmobisim as ms
import pytest

pytest.importorskip("pyarrow")

LAT = 45.70
KM = 1 / (111.32 * math.cos(math.radians(LAT)))
H8 = 8 * 3600
PAYS = {"c": (True, True, False)}
PASS = {"c": (True, True, True)}


def along_parallel_m(lon_a, lon_b, lat):
    """The ground distance between two points of one parallel: ``N(φ) cos φ Δλ`` (WGS84)."""
    a, e2 = 6378137.0, 6.69437999014e-3
    phi = math.radians(lat)
    n = a / math.sqrt(1 - e2 * math.sin(phi) ** 2)
    return n * math.cos(phi) * abs(math.radians(lon_b - lon_a))


def hhmmss(s):
    s = int(s)
    return f"{s // 3600:02d}:{s % 3600 // 60:02d}:{s % 60:02d}"


def feed(stops, lines, shift_s=0):
    """A one-day GTFS feed of one run per line, every time ``shift_s`` later.

    ``stops`` is {id: (lon, lat)}, ``lines`` {id: (route_type, [(stop, second of the day)])}.
    """
    folder = Path(tempfile.mkdtemp()) / "feed"
    folder.mkdir(parents=True)
    (folder / "agency.txt").write_text(
        "agency_id,agency_name,agency_url,agency_timezone\nA,V,http://x,Europe/Paris\n"
    )
    (folder / "stops.txt").write_text(
        "stop_id,stop_name,stop_lat,stop_lon\n"
        + "".join(f"{s},{s},{lat},{lon}\n" for s, (lon, lat) in stops.items())
    )
    (folder / "routes.txt").write_text(
        "route_id,route_short_name,route_long_name,route_type\n"
        + "".join(f"{r},{r},{r},{kind}\n" for r, (kind, _) in lines.items())
    )
    (folder / "calendar.txt").write_text(
        "service_id,monday,tuesday,wednesday,thursday,friday,saturday,sunday,start_date,end_date\n"
        "S,1,1,1,1,1,1,1,20261007,20261007\n"
    )
    (folder / "trips.txt").write_text(
        "route_id,service_id,trip_id\n" + "".join(f"{r},S,{r}_0\n" for r in lines)
    )
    times = ["trip_id,arrival_time,departure_time,stop_id,stop_sequence\n"]
    for r, (_, calls) in lines.items():
        for seq, (stop, t) in enumerate(calls):
            t = hhmmss(t + shift_s)
            times.append(f"{r}_0,{t},{t},{stop},{seq + 1}\n")
    (folder / "stop_times.txt").write_text("".join(times))
    return folder


# --- fares --------------------------------------------------------------------------------------


def street():
    """A street A – B, 12 km, both ways, and its train and bus."""
    nodes = [{"id": n, "x": 4.80 + 12 * i * KM, "y": LAT} for i, n in enumerate("AB")]
    links = [
        {"id": f"{a}{b}", "from": a, "to": b, "class": "residential", "length": 12000}
        for a, b in (("A", "B"), ("B", "A"))
    ]
    net = ms.network_read_table(links, nodes)
    stops = {n["id"]: (n["x"], LAT) for n in nodes}
    lines = {
        "T": (2, [("A", H8 + 300), ("B", H8 + 900)]),
        "U": (3, [("A", H8 + 180), ("B", H8 + 1500)]),
    }
    timetable = ms.transit_read_gtfs(str(feed(stops, lines)), network=net, date="2026-10-07")
    return net, nodes, timetable


def journeys(tmp_path, prices, classes=PAYS, n=10, name="x"):
    """The first trip's chains of ``n`` riders from A to B at 08:00 under ``prices``.

    As {line: (fare, cost)}, with the set of every trip's set size.
    """
    net, nodes, timetable = street()
    a, b = nodes
    rows = [(f"r{i}", 0, a["x"], LAT, b["x"], LAT, H8, "c", None, "transit") for i in range(n)]
    s = ms.Scenario.from_parts(
        net,
        rows,
        transit=timetable,
        classes=classes,
        choice_model="logit",
        choice_options={"beta_cost_eur": -1.2},
        window=(7 * 3600, 10 * 3600),
        equilibration="free_flow",
        price_options=prices,
    )
    r = s.run(name, output_dir=str(tmp_path / name), quiet=True, engine="one-loop")
    c = r.chains()
    trip = np.asarray(c["trip"])
    out = {
        c["ride_line"][i][0]: (float(c["fare_eur"][i]), float(c["cost"][i]))
        for i in np.flatnonzero(trip == 0)
    }
    return out, set(np.bincount(trip))


def test_with_one_price_per_km_the_set_is_the_train_alone_at_its_fare(tmp_path):
    d_km = along_parallel_m(4.80, 4.80 + 12 * KM, LAT) / 1000
    sets, sizes = journeys(tmp_path, {"fare_base_eur": 1.0, "fare_km_eur": 0.2})
    assert sizes == {1}
    assert set(sets) == {"T"}
    fare = 1.0 + 0.2 * d_km
    assert sets["T"][0] == pytest.approx(fare, abs=1e-9)
    assert sets["T"][1] == pytest.approx(0.2 * 10 + 0.29 * 5 + 1.2 * fare, abs=1e-9)


def test_a_dearer_train_adds_the_cheaper_bus_each_at_its_fare(tmp_path):
    d_km = along_parallel_m(4.80, 4.80 + 12 * KM, LAT) / 1000
    prices = {"fare_base_eur": 1.0, "fare_km_eur_rail": 0.3, "fare_km_eur_bus": 0.1}
    sets, sizes = journeys(tmp_path, prices)
    assert sizes == {2}
    train, bus = 1.0 + 0.3 * d_km, 1.0 + 0.1 * d_km
    assert sets["T"][0] == pytest.approx(train, abs=1e-9)
    assert sets["U"][0] == pytest.approx(bus, abs=1e-9)
    assert sets["T"][1] == pytest.approx(0.2 * 10 + 0.29 * 5 + 1.2 * train, abs=1e-9)
    assert sets["U"][1] == pytest.approx(0.2 * 22 + 0.29 * 3 + 1.2 * bus, abs=1e-9)
    # A pass holder pays nothing and has the train alone, as with one price.
    held, sizes = journeys(tmp_path, prices, classes=PASS, name="pass")
    assert sizes == {1}
    assert held == {"T": (0.0, pytest.approx(0.2 * 10 + 0.29 * 5, abs=1e-12))}
    # A dearer bus: the cheaper query (the train alone) finds the train again, kept once.
    sets, sizes = journeys(
        tmp_path, {"fare_km_eur_rail": 0.1, "fare_km_eur_bus": 0.3}, name="bus_dearer"
    )
    assert sizes == {1}
    assert set(sets) == {"T"}
    assert sets["T"][0] == pytest.approx(0.1 * d_km, abs=1e-9)


def test_the_engine_refuses_prices_it_does_not_have(tmp_path):
    with pytest.raises(ms.EngineNotSupportedError, match="car_eur_km"):
        journeys(tmp_path, {"car_eur_km": 0.12})
    with pytest.raises(ms.EngineNotSupportedError, match="fare_km_eur_rail"):
        journeys(tmp_path, {"fare_km_eur_train": 0.1})


# --- mode choice --------------------------------------------------------------------------------

#: Node and link names, and their renaming for the relabelling test (sorted orders reversed).
NODES = {"O": "z5", "C": "z4", "A": "z3", "W": "z2", "D": "z1"}
LINKS = {"oc": "y7", "cd": "y6", "od": "y5", "oa": "y4", "ad": "y3", "ow": "y2", "wd": "y1"}


def town(names=None, link_names=None, shift_s=0, stop_names=None, line_names=None):
    """From O to D: a road, a cycleway, a footway, a detour and one rail line.

    The road 3.6 km at 50 km/h, the cycleway 3.6 km, the footway 3.2 km, the detour by trunk
    road, the line O 08:05 → D 08:15; ``names`` and the others rename.
    """
    names = names or {k: k for k in NODES}
    link_names = link_names or {k: k for k in LINKS}
    stop_names = stop_names or {"O": "SO", "D": "SD"}
    line_names = line_names or {"L": "L"}
    at = {
        "O": (4.80, LAT),
        "C": (4.80 + 1.5 * KM, LAT - 0.014),
        "A": (4.80 + 1.5 * KM, LAT + 0.007),
        "W": (4.80 + 1.5 * KM, LAT + 0.003),
        "D": (4.80 + 3.0 * KM, LAT),
    }
    nodes = [{"id": names[k], "x": x, "y": y} for k, (x, y) in at.items()]

    def link(i, a, b, metres, cls, **kw):
        return {
            "id": link_names[i],
            "from": names[a],
            "to": names[b],
            "class": cls,
            "lanes": 1,
            "length": metres,
        } | kw

    links = [
        link("oc", "O", "C", 7500, "trunk", free_flow_speed=90, capacity=900.0),
        link("cd", "C", "D", 7500, "trunk", free_flow_speed=90, capacity=900.0),
        link("od", "O", "D", 3600, "primary", free_flow_speed=50, capacity=600.0),
        link("oa", "O", "A", 1800, "cycleway"),
        link("ad", "A", "D", 1800, "cycleway"),
        link("ow", "O", "W", 1600, "footway"),
        link("wd", "W", "D", 1600, "footway"),
    ]
    net = ms.network_read_table(links, nodes)
    so, sd = stop_names["O"], stop_names["D"]
    stops = {so: at["O"], sd: at["D"]}
    lines = {line_names["L"]: (2, [(so, H8 + 300), (sd, H8 + 900)])}
    timetable = ms.transit_read_gtfs(
        str(feed(stops, lines, shift_s)), network=net, date="2026-10-07"
    )
    return net, timetable, at


#: The modes' constants of the mode-choice toy (utility units).
CONSTANTS = {"beta_mode_car": -1.5, "beta_mode_walk": 10.0, "beta_mode_transit": 1.0}


def choosing(tmp_path, n=4000, name="x", depart_s=H8, town_kw=None, threads=None, **kw):
    """``n`` trips from O to D choosing among the four modes, run on the one-loop engine."""
    net, timetable, at = town(**(town_kw or {}))
    (ox, oy), (dx, dy) = at["O"], at["D"]
    rows = [(f"t{i:05d}", 0, ox, oy, dx, dy, depart_s, "c", None, None) for i in range(n)]
    options = {
        "transit": timetable,
        "classes": PAYS,
        "choice_model": "nested_logit",
        "choice_options": {"beta_cost_eur": -1.2, **CONSTANTS},
        "window": (7 * 3600, 10 * 3600),
        "equilibration": "free_flow",
        "flow_level": 0,
        "modes": ["car", "bike", "walk", "transit"],
        "mode_options": {
            "walk_max_s": 7200,
            "car_access_s": 0,
            "car_egress_s": 0,
            "bike_access_s": 0,
            "bike_egress_s": 0,
        },
        "price_options": {"fare_base_eur": 1.0},
    } | kw
    s = ms.Scenario.from_parts(net, rows, **options)
    return s.run(
        name, output_dir=str(tmp_path / name), quiet=True, engine="one-loop", threads=threads
    )


def test_transit_is_a_mode_of_the_choice_with_the_nested_logit_shares(tmp_path):
    r = choosing(tmp_path)
    v = {
        "car": -0.2 * (3600 / (50 / 3.6)) / 60 - 1.5,
        "bike": -0.2 * (3600 / (18 / 3.6)) / 60,
        "walk": -(0.2 + 0.13) * (3200 / (4.8 / 3.6)) / 60 + 10.0,
        "transit": -(0.2 * 10 + 0.29 * 5 + 1.2 * 1.0) + 1.0,
    }
    total = sum(math.exp(x) for x in v.values())
    p = {m: math.exp(x) / total for m, x in v.items()}
    c = r.chains()
    first = np.asarray(c["trip"]) == 0
    modes = [c["mode"][i] for i in np.flatnonzero(first)]
    assert sorted(modes) == ["bike", "car", "transit", "walk"]
    for i in np.flatnonzero(first):
        m = c["mode"][i]
        assert -float(c["cost"][i]) == pytest.approx(v[m], abs=1e-9), m
        assert float(c["probability"][i]) == pytest.approx(p[m], abs=1e-9), m
    mode = np.asarray(r.trip_results()["mode"])
    for m, share in p.items():
        seen = float((mode == m).mean())
        assert abs(seen - share) < 3 * math.sqrt(share * (1 - share) / 4000), (m, seen, share)
    conv = r.convergence()
    assert float(conv["mode_share_transit"][-1]) == pytest.approx(float((mode == "transit").mean()))
    left = r.manifest()["modes_left_out"]
    assert (left["transit_no_stop"], left["transit_no_journey"]) == (0, 0)


def test_a_trip_with_no_journey_has_no_transit_chain_and_is_counted(tmp_path):
    # Leaving at 08:30, after the line's one run: no journey.
    r = choosing(tmp_path, n=20, depart_s=H8 + 1800)
    assert "transit" not in set(r.chains()["mode"])
    left = r.manifest()["modes_left_out"]
    assert (left["choosing"], left["transit_no_journey"], left["transit_no_stop"]) == (20, 20, 0)
    # From W, 1.2 km from the stop at O, with walks to a stop of at most 0 s: no stop in reach.
    net, timetable, at = town()
    rows = [(f"t{i}", 0, *at["W"], *at["D"], H8, "c", None, None) for i in range(5)]
    s = ms.Scenario.from_parts(
        net,
        rows,
        transit=timetable,
        classes=PAYS,
        modes=["car", "transit"],
        transit_options={"access_walk_max_s": 0},
        equilibration="free_flow",
    )
    r = s.run("v", output_dir=str(tmp_path / "v"), quiet=True, engine="one-loop")
    assert r.manifest()["modes_left_out"]["transit_no_stop"] == 5


def test_stated_transit_trips_priced_by_kind_change_no_car_result(tmp_path):
    net, timetable, at = town()
    (ox, oy), (dx, dy) = at["O"], at["D"]
    cars = [(f"c{i:04d}", 0, ox, oy, dx, dy, H8 + 4 * i, "c", None, "car") for i in range(300)]
    riders = [(f"r{i:04d}", 0, ox, oy, dx, dy, H8, "c", None, "transit") for i in range(200)]
    options = {
        "transit": timetable,
        "classes": PAYS,
        "flow_level": 2,
        "equilibration": "msa",
        "equilibration_options": {"iterations": 5},
        "choice_model": "logit",
        "choice_options": {"beta_cost_eur": -1.2},
        "window": (7 * 3600, 10 * 3600),
        "price_options": {"fare_base_eur": 1.0, "fare_km_eur_rail": 0.3, "fare_km_eur_bus": 0.1},
    }
    alone = ms.Scenario.from_parts(net, cars, **options).run(
        "a", output_dir=str(tmp_path / "a"), quiet=True, engine="one-loop"
    )
    both = ms.Scenario.from_parts(net, cars + riders, **options).run(
        "b", output_dir=str(tmp_path / "b"), quiet=True, engine="one-loop"
    )
    ta, tb = alone.trip_results(), both.trip_results()
    car = np.asarray(tb["mode"]) == "car"
    assert car.sum() == 300
    for col in ("arrival_s", "travel_time_s", "vehicle_s"):
        assert np.array_equal(np.asarray(ta[col]), np.asarray(tb[col])[car]), col
    ca, cb = alone.convergence(), both.convergence()
    for col in ("switch_share", "gap", "found_share", "switch_excess_expected"):
        assert np.array_equal(np.asarray(ca[col]), np.asarray(cb[col])), col


LOOP = {
    "flow_level": 2,
    "equilibration": "msa",
    "equilibration_options": {"iterations": 4},
    "price_options": {"fare_base_eur": 1.0, "fare_km_eur_rail": 0.3, "fare_km_eur_bus": 0.1},
}


@pytest.mark.parametrize("threads", [2, None])
def test_the_same_seed_gives_the_same_run_on_any_thread_count(tmp_path, threads):
    a = choosing(tmp_path, n=600, name="a", threads=threads, **LOOP)
    b = choosing(tmp_path, n=600, name="b", threads=1, **LOOP)
    assert a.fingerprint == b.fingerprint
    for col in ("arrival_s", "travel_time_s", "mode"):
        assert np.array_equal(np.asarray(a.trip_results()[col]), np.asarray(b.trip_results()[col]))
    ca, cb = a.chains(), b.chains()
    for col in ("taken", "probability", "cost"):
        assert np.array_equal(np.asarray(ca[col]), np.asarray(cb[col])), col


def test_relabelled_nodes_links_stops_and_lines_change_nothing(tmp_path):
    a = choosing(tmp_path, n=600, name="a", **LOOP)
    renamed = {
        "names": NODES,
        "link_names": LINKS,
        "stop_names": {"O": "s9", "D": "s1"},
        "line_names": {"L": "R9"},
    }
    b = choosing(tmp_path, n=600, name="b", town_kw=renamed, **LOOP)
    for col in ("arrival_s", "travel_time_s", "mode"):
        assert np.array_equal(np.asarray(a.trip_results()[col]), np.asarray(b.trip_results()[col]))
    ca, cb = a.chains(), b.chains()
    for col in ("taken", "probability", "cost", "mode"):
        assert np.array_equal(np.asarray(ca[col]), np.asarray(cb[col])), col


def test_an_hour_later_everything_arrives_an_hour_later(tmp_path):
    a = choosing(tmp_path, n=600, name="a", **LOOP)
    b = choosing(
        tmp_path,
        n=600,
        name="b",
        depart_s=H8 + 3600,
        town_kw={"shift_s": 3600},
        window=(8 * 3600, 11 * 3600),
        **LOOP,
    )
    ta, tb = a.trip_results(), b.trip_results()
    assert np.array_equal(np.asarray(ta["arrival_s"]) + 3600, np.asarray(tb["arrival_s"]))
    assert np.array_equal(np.asarray(ta["travel_time_s"]), np.asarray(tb["travel_time_s"]))
    assert np.array_equal(np.asarray(ta["mode"]), np.asarray(tb["mode"]))
