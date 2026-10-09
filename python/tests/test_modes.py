"""Mode choice: a trip without a stated mode chooses one, with its route.

The toy network's hand values are the Rust suite's (``core-sim/tests/toy_modes.rs``): ``W → N1``
is fastest by bike (**144 s**), ``W → M`` by car (**80 s**, against the bike's 120 s).
"""

from __future__ import annotations

import openmobisim as ms
import pytest


def toy_trip(net: object, who: str, frm: str, to: str, depart: int, mode: str | None) -> tuple:
    o, d = net.node_lonlat(frm), net.node_lonlat(to)
    return (who, 0, o[0], o[1], d[0], d[1], depart, "everyone", None, mode)


def toy_run(rows: list[tuple], run_id: str, **kwargs: object) -> ms.Run:
    return ms.Scenario.from_parts(
        network=ms.examples.toy_network(),
        demand=rows,
        classes={"everyone": (True, True, False)},
        transit=kwargs.pop("transit", ms.examples.toy_network_transit()),
        parkings=kwargs.pop("parkings", ms.examples.toy_network_parkings()),
        choice_model=kwargs.pop("choice_model", "deterministic"),
        equilibration=kwargs.pop("equilibration", "free_flow"),
        flow_level=kwargs.pop("flow_level", 0),
        **kwargs,
    ).run(run_id=run_id)


def test_a_trip_without_a_mode_chooses_the_fastest_under_the_deterministic_model() -> None:
    net = ms.examples.toy_network()
    rows = [
        toy_trip(net, "a", "W", "N1", 0, None),
        toy_trip(net, "b", "W", "M", 0, None),
        toy_trip(net, "c", "W", "M", 0, "bike"),
    ]
    run = toy_run(rows, "modes-deterministic", modes=ms.MODES)
    by_mode = run.completion_by_mode
    assert by_mode["bike"]["total_trips"] == 2 and by_mode["car"]["total_trips"] == 1
    ch = run.itinerary_choices()
    assert list(ch["traveller_id"]) == ["a", "b"], "the stated bike trip does not choose"
    assert list(ch["mode"]) == ["bike", "car"] and list(ch["mode_choice"]) == [True, True]
    assert list(ch["expected_s"]) == pytest.approx([144.0, 80.554054], abs=1e-3)
    pq = pytest.importorskip("pyarrow.parquet")
    kpis = pq.read_table(run.kpis().path).to_pylist()
    shares = {k["mode"]: k["value"] for k in kpis if k["metric"] == "mode_share"}
    assert shares == pytest.approx({"bike": 2 / 3, "car": 1 / 3})


def test_without_modes_a_trip_without_one_is_a_car_trip_and_one_mode_is_no_choice() -> None:
    net = ms.examples.toy_network()
    rows = [toy_trip(net, "a", "W", "M", 0, None), toy_trip(net, "b", "M", "X0", 30, None)]
    plain = toy_run(rows, "modes-none", choice_model="logit", equilibration="msa")
    car = toy_run(rows, "modes-car", choice_model="logit", equilibration="msa", modes=("car",))
    assert plain.fingerprint == car.fingerprint
    assert plain.total_travel_time_s == car.total_travel_time_s
    assert car.itinerary_choices() is None
    bike = toy_run(rows, "modes-bike", modes=["bike"])
    assert bike.completion_by_mode["bike"]["total_trips"] == 2
    assert "car" not in bike.completion_by_mode


def test_the_nested_logit_and_msa_report_how_the_modes_settle() -> None:
    net = ms.examples.toy_network()
    rows = [toy_trip(net, f"a{i}", "W", "M", 0, None) for i in range(60)]
    run = toy_run(
        rows,
        "modes-msa",
        transit=None,
        parkings=None,
        modes=("car", "bike"),
        choice_model="nested_logit",
        equilibration="msa",
        equilibration_options={"iterations": 4},
        flow_level=4,
    )
    c = run.convergence()
    changed = list(c["mode_changed_share"])
    assert changed[0] != changed[0], "nan before anything chose"
    assert all(0.0 <= x <= 1.0 for x in changed[1:])
    ch = run.itinerary_choices()
    assert set(ch["mode"]) <= {"car", "bike"} and all(ch["mode_choice"])
    assert all(0.0 < p <= 1.0 for p in ch["probability"])


def test_unknown_modes_are_refused() -> None:
    with pytest.raises(ValueError, match="modes must be among"):
        ms.Scenario.from_parts(
            network=ms.examples.toy_network(), demand=[], modes=("car", "scooter")
        )


def test_trip_modes_gives_every_trip_its_departure_and_the_mode_it_took() -> None:
    net = ms.examples.toy_network()
    rows = [
        toy_trip(net, "a", "W", "N1", 0, None),
        toy_trip(net, "b", "W", "M", 60, None),
        toy_trip(net, "c", "W", "M", 120, "bike"),
    ]
    run = toy_run(rows, "trip-modes", modes=ms.MODES)
    t = run.trip_modes()
    assert list(t["traveller_id"]) == ["a", "b", "c"] and list(t["trip_seq"]) == [0, 0, 0]
    assert list(t["departure_s"]) == [0, 60, 120]
    assert list(t["mode"]) == ["bike", "car", "bike"]
    assert list(t["mode_choice"]) == [True, True, False]
    assert list(t["weight"]) == [1, 1, 1]
    # Without modes a trip without one is a car trip; with one mode it takes that mode.
    assert list(toy_run(rows, "trip-modes-none").trip_modes()["mode"]) == ["car", "car", "bike"]
    one = toy_run(rows, "trip-modes-walk", modes=("walk",)).trip_modes()
    assert list(one["mode"]) == ["walk", "walk", "bike"] and not any(one["mode_choice"])


def test_chart_mode_share_draws_the_modes_taken(tmp_path) -> None:
    pytest.importorskip("matplotlib")
    from openmobisim import viz

    net = ms.examples.toy_network()
    rows = [toy_trip(net, f"a{i}", "W", "N1", 7 * 3600 + 60 * i, None) for i in range(20)]
    rows += [toy_trip(net, f"b{i}", "W", "M", 8 * 3600 + 60 * i, None) for i in range(20)]
    run = toy_run(rows, "mode-share-chart", modes=ms.MODES)
    for theme in ("paper", "night"):
        out = tmp_path / f"modes_{theme}.png"
        fig = viz.chart_mode_share(run, theme=theme, path=str(out))
        assert out.exists() and out.stat().st_size > 10_000
        texts = [t.get_text() for t in fig.texts]
        assert any(t.startswith("bike") and t.endswith(" 50.0%") for t in texts)
        assert any(t.startswith("car") and t.endswith(" 50.0%") for t in texts)
        assert any("40 trips, each choosing its mode" in t for t in texts)
    with pytest.raises(ValueError, match="bin_s"):
        viz.chart_mode_share(run, bin_s=0)


def test_mode_options_cut_off_a_long_walk_or_ride_and_refuse_unknown_names() -> None:
    net = ms.examples.toy_network()
    rows = [toy_trip(net, "a", "W", "N1", 0, None)]
    any_length = {"pr_min_km": 0}  # as the Rust case (MC7): P+R and B+R offered too
    full = toy_run(rows, "modes-cut-default", modes=ms.MODES, parking_options=any_length)
    short = toy_run(
        rows,
        "modes-cut-walk",
        modes=ms.MODES,
        parking_options=any_length,
        mode_options={"walk_max_s": 300},
    )
    both = toy_run(
        rows,
        "modes-cut-both",
        modes=ms.MODES,
        parking_options=any_length,
        mode_options={"walk_max_s": 300, "bike_max_s": 100},
    )
    # W → N1 walks in 318.198 s and rides in 144 s: offered under the 1800-s defaults; the walk
    # not under 300 s; the ride not under 100 s either. Nothing else changes.
    n = int(full.itinerary_choices()["alternatives"][0])
    assert int(short.itinerary_choices()["alternatives"][0]) == n - 1
    assert int(both.itinerary_choices()["alternatives"][0]) == n - 2
    assert full.fingerprint != short.fingerprint
    with pytest.raises(ValueError, match="walk_max_s, bike_max_s"):
        toy_run(rows, "modes-cut-bad", modes=ms.MODES, mode_options={"walk_max": 300})


def test_a_ferry_crossing_counts_in_no_bike_facility(tmp_path) -> None:
    # S241: a ride across a ferry and then along a cycle track is on a track for the track's
    # length only; the crossing is ridden on no facility (it read as mixed traffic before), and
    # ``link_class`` names it (``LINK_CLASSES``).
    import numpy as np
    from openmobisim import choice

    nodes = "node_id,x_coord,y_coord\na,4.9000,52.3700\nb,4.9100,52.3700\nc,4.9200,52.3700\n"
    links = {
        "road": "1,a,b,false,680,30,residential,auto\n2,b,c,false,680,30,residential,auto\n",
        "bike": "1,a,b,false,680,6,ferry,bike,\n2,b,c,false,680,18,cycleway,bike,separated\n",
        "walk": "1,a,b,false,680,6,ferry,walk\n2,b,c,false,680,5,footway,walk\n",
    }
    for layer, rows in links.items():
        (tmp_path / layer).mkdir()
        (tmp_path / layer / "node.csv").write_text(nodes)
        header = "link_id,from_node_id,to_node_id,directed,length,free_speed,facility_type,"
        header += "allowed_uses,bike_facility" if layer == "bike" else "allowed_uses"
        (tmp_path / layer / "link.csv").write_text(f"{header}\n{rows}")
    net = ms.network_read_gmns(str(tmp_path))
    bike = net.layer("bike")
    ferry = ms.LINK_CLASSES.index("ferry")
    assert ms.LINK_CLASSES[0] == "motorway" and list(bike.link_class()).count(ferry) == 2
    seen = []

    class Recorder:
        name = "recorder"

        def choose(self, batch):
            seen.append({k: np.array(v) for k, v in batch.attributes.items()})
            return choice.segment_argmax(-batch.attributes["time_min"], batch.offsets)

    a, c = net.node_lonlat("a"), net.node_lonlat("c")
    rows = [("r", 0, a[0], a[1], c[0], c[1], 0, "rider", None, None)]
    ms.Scenario.from_parts(
        net, rows, classes={"rider": {"modes": ["bike", "walk"]}}, choice_model=Recorder(),
        equilibration="free_flow", flow_level=0,
    ).run("ferry-bike", output_dir=str(tmp_path / "run"), quiet=True)  # fmt: skip
    ride = int(np.flatnonzero(seen[0]["mode_bike"] == 1)[0])  # a choice: bike or walk
    names = ("bike_separated_km", "bike_mixed_km", "length_km")
    got = {k: float(seen[0][k][ride]) for k in names}
    assert got["length_km"] == pytest.approx(1.36, abs=0.01), got
    assert got["bike_separated_km"] == pytest.approx(0.68, abs=0.01), got
    assert got["bike_mixed_km"] == 0.0, got


def test_a_model_sees_the_bike_leg_by_facility_and_transit_s_parts(tmp_path) -> None:
    # S236 (roadmap I-bb U2, U3): the parts add up to their totals, by kind of service.
    import numpy as np
    from openmobisim import choice

    seen: list[dict[str, np.ndarray]] = []

    class Recorder:
        name = "recorder"

        def choose(self, batch):
            seen.append({k: np.array(v) for k, v in batch.attributes.items()})
            return choice.segment_argmax(-batch.attributes["time_min"], batch.offsets)

    net = ms.examples.toy_network()
    rows = [toy_trip(net, "a", "W", "N1", 0, None), toy_trip(net, "b", "W", "M", 0, None)]
    toy_run(rows, "modes-parts", modes=ms.MODES, choice_model=Recorder(),
            parking_options={"pr_min_km": 0})  # fmt: skip
    a = {k: np.concatenate([s[k] for s in seen]) for k in seen[0]}
    by_facility = a["bike_separated_km"] + a["bike_lane_km"] + a["bike_mixed_km"]
    bike = (a["mode_bike"] + a["mode_bike_transit"]) > 0
    assert bike.any() and np.allclose(by_facility[bike], a["length_km"][bike])
    assert np.all(by_facility[~bike] == 0), "only bike legs have facilities"
    transit = (a["mode_transit"] + a["mode_car_transit"] + a["mode_bike_transit"]) > 0
    assert transit.any()
    walks = a["walk_access_min"] + a["walk_egress_min"] + a["walk_transfer_min"]
    assert np.allclose(walks[transit], a["walk_min"][transit])
    waits = a["wait_first_min"] + a["wait_transfer_min"]
    assert np.allclose(waits, a["wait_min"])
    kinds = ["rail", "metro", "tram", "bus", "ferry", "other"]
    rides = sum(a[f"ride_{k}_min"] for k in kinds)
    assert np.allclose(rides, a["ride_min"])
    assert a["ride_tram_min"].max() > 0 and a["ride_bus_min"].max() > 0, "the toy's tram and bus"
    assert set(choice.ROUTE_ATTRIBUTES) == set(a), "Python's list is the core's"


def test_a_painted_lane_s_factor_is_a_network_option() -> None:
    assert ms.network_options()["bike_lane_cost_factor"] == 1.0, "S236: a lane counted as a track"


# --- the user's link values (S236, roadmap I-bb U4) ----------------------------------------------


def test_link_values_are_offered_along_each_leg_and_weighed_by_the_built_in_models() -> None:
    import numpy as np
    from openmobisim import choice

    net = ms.examples.toy_network()
    bike = net.layer("bike")
    km = np.asarray(bike.link_length_m()) / 1000.0
    green = {link: 1.0 for link in bike.link_ids()}  # every bike link: greenery 1
    seen: list[dict[str, np.ndarray]] = []

    class Recorder:
        name = "recorder"
        attributes = ["length_km", "mode_bike", "mode_car", "bike_green_km", "bike_green_sum"]

        def choose(self, batch):
            seen.append({k: np.array(v) for k, v in batch.attributes.items()})
            return choice.segment_argmax(-batch.attributes["length_km"], batch.offsets)

    rows = [toy_trip(net, "b", "W", "M", 0, None)]
    plain = toy_run(rows, "values-none", modes=["car", "bike"], transit=None, parkings=None)
    toy_run(rows, "values-seen", modes=["car", "bike"], transit=None, parkings=None,
            choice_model=Recorder(), link_values={"bike": {"green": green}})  # fmt: skip
    a = seen[0]
    on_bike = a["mode_bike"] > 0
    assert np.allclose(a["bike_green_km"][on_bike], a["length_km"][on_bike]), "value 1 × km"
    assert np.all(a["bike_green_km"][~on_bike] == 0), "a car route runs on no bike link"
    assert np.all(a["bike_green_sum"][on_bike] >= 1), "a count of links"
    assert km.sum() > 0
    # W → M is quicker by car (80 s against 120 s); a strong liking for green kilometres turns it.
    greenery = {"bike": {"green": [1.0] * bike.link_count}}
    keen = toy_run(rows, "values-keen", modes=["car", "bike"], transit=None, parkings=None,
                   choice_model="logit", link_values=greenery,
                   choice_options={"beta_bike_green_km": 50.0})  # fmt: skip
    assert list(plain.itinerary_choices()["mode"]) == ["car"]
    assert list(keen.itinerary_choices()["mode"]) == ["bike"]
    assert keen.manifest()["link_values"] == {"bike": ["green"]}
    assert plain.manifest()["link_values"] is None
    assert keen.fingerprint != plain.fingerprint


def test_road_link_values_reach_route_choice_too() -> None:
    import numpy as np
    from openmobisim import choice

    net = ms.examples.toy_network()
    seen: list[np.ndarray] = []

    class Tolls:
        name = "tolls"
        attributes = ["time_min", "road_toll_eur_sum"]

        def choose(self, batch):
            seen.append(np.array(batch.attributes["road_toll_eur_sum"]))
            return choice.segment_argmax(-batch.attributes["time_min"], batch.offsets)

    rows = [toy_trip(net, "a", "W", "M", 0, None)]
    toll = {"road": {"toll_eur": [2.0] * net.link_count}}
    toy_run(rows, "values-routes", transit=None, parkings=None, choice_model=Tolls(),
            link_values=toll)  # fmt: skip
    assert seen and np.all(seen[0] > 0) and np.all(seen[0] % 2.0 == 0), "2 per link on the route"


@pytest.mark.parametrize(
    ("values", "message"),
    [
        ({"rail": {"x": [1.0]}}, "no layer 'rail'"),
        ({"bike": {"green": [1.0]}}, "the bike layer has"),
        ({"bike": {"green": {"no-such-link": 1.0}}}, "has no link 'no-such-link'"),
        ({"bike": {"Green": {}}}, "lower-case"),
        ({"bike": {"mixed": {}}}, "built-in attribute"),
    ],
)
def test_wrong_link_values_are_refused(values: dict, message: str) -> None:
    net = ms.examples.toy_network()
    with pytest.raises(ValueError, match=message):
        toy_run([toy_trip(net, "b", "W", "M", 0, None)], "values-bad", modes=["car", "bike"],
                transit=None, parkings=None, link_values=values)  # fmt: skip


def test_a_layer_s_link_ids_tie_its_links_to_the_road_s() -> None:
    net = ms.examples.toy_network()
    road, bike = net.link_ids(), net.layer("bike").link_ids()
    assert len(road) == net.link_count and len(bike) == net.layer("bike").link_count
    # The toy's bike layer: its streets' links keep the road's ids (a contraflow one adds
    # ":c", a reverse one ":r"); its cycle track (t1, t2) is on no road.
    along = {b.split(":")[0] for b in bike if not b.startswith("t")}
    assert along and along <= set(road)
    grid = ms.examples.manhattan_grid(n=3, block_metres=100.0, signals=False)
    assert set(grid.layer("bike").link_ids()) <= set(grid.link_ids()), "derived: the road's"


# --- money (S248, roadmap I-bb U5) ---------------------------------------------------------------


def test_a_model_sees_what_each_alternative_costs_by_part() -> None:
    import numpy as np
    from openmobisim import choice

    seen: list[dict[str, np.ndarray]] = []

    class Recorder:
        name = "recorder"

        def choose(self, batch):
            seen.append({k: np.array(v) for k, v in batch.attributes.items()})
            return choice.segment_argmax(-batch.attributes["time_min"], batch.offsets)

    net = ms.examples.toy_network()
    rows = [toy_trip(net, "a", "W", "N1", 0, None), toy_trip(net, "b", "W", "M", 0, None)]
    prices = {
        "car_eur_km": 0.2,
        "bike_eur_km": 0.05,
        "fare_base_eur": 1.16,
        "fare_km_eur": 0.217,
        "fare_transfer_eur": 0.5,
        "parking_car_eur": 3.0,
        "parking_bike_eur": 0.5,
    }
    toll = {"road": {"toll_eur": [0.1] * net.link_count}}
    toy_run(rows, "money-parts", modes=ms.MODES, choice_model=Recorder(), price_options=prices,
            link_values=toll, parking_options={"pr_min_km": 0})  # fmt: skip
    a = {k: np.concatenate([s[k] for s in seen]) for k in seen[0]}
    parts = ("cost_running_eur", "cost_toll_eur", "cost_parking_eur", "cost_fare_eur")
    assert np.allclose(a["cost_eur"], sum(a[k] for k in parts)), "the total is its parts"
    car = (a["mode_car"] + a["mode_car_transit"]) > 0
    bike = (a["mode_bike"] + a["mode_bike_transit"]) > 0
    assert car.any() and bike.any()
    assert np.allclose(a["cost_running_eur"][car], 0.2 * a["length_km"][car])
    assert np.allclose(a["cost_running_eur"][bike], 0.05 * a["length_km"][bike])
    assert np.allclose(a["cost_toll_eur"][car], 0.1 * a["n_links"][car]), "0.1 per car link"
    assert np.all(a["cost_toll_eur"][~car] == 0) and np.all(
        a["cost_running_eur"][~(car | bike)] == 0
    )
    # A parking's fee on the trip that parks: the run's price for its kind (the toy's have none).
    assert np.all(a["cost_parking_eur"][a["mode_car_transit"] > 0] == 3.0)
    assert np.all(a["cost_parking_eur"][a["mode_bike_transit"] > 0] == 0.5)
    assert np.all(a["cost_parking_eur"][(a["mode_car_transit"] + a["mode_bike_transit"]) == 0] == 0)
    # A fare: the base once, 0.5 per transfer and something per km; nothing without a ride.
    rides = a["ride_min"] > 0
    assert rides.any() and np.all(a["cost_fare_eur"][~rides] == 0)
    by_km = a["cost_fare_eur"][rides] - 1.16 - 0.5 * a["transfers"][rides]
    assert np.all(by_km > 0), "every ride covers some distance"
    assert np.all(a["cost_eur"][a["mode_walk"] > 0] == 0), "a walk costs nothing"
    tolls = {"road_toll_eur_km", "road_toll_eur_sum"}  # the link value's own attributes
    assert set(choice.ROUTE_ATTRIBUTES) == set(a) - tolls, "Python's list is the core's"


def test_a_car_route_of_a_trip_given_the_car_costs_its_km_and_tolls_too() -> None:
    import numpy as np
    from openmobisim import choice

    seen: list[dict[str, np.ndarray]] = []

    class Recorder:
        name = "recorder"
        attributes = ["time_min", "length_km", "n_links", "cost_eur", "cost_running_eur",
                      "cost_toll_eur", "cost_fare_eur"]  # fmt: skip

        def choose(self, batch):
            seen.append({k: np.array(v) for k, v in batch.attributes.items()})
            return choice.segment_argmax(-batch.attributes["time_min"], batch.offsets)

    net = ms.examples.toy_network()
    toy_run([toy_trip(net, "a", "W", "M", 0, "car")], "money-routes", transit=None, parkings=None,
            choice_model=Recorder(), price_options={"car_eur_km": 0.3},
            link_values={"road": {"toll_eur": [0.25] * net.link_count}})  # fmt: skip
    a = seen[0]
    assert np.allclose(a["cost_running_eur"], 0.3 * a["length_km"])
    assert np.allclose(a["cost_toll_eur"], 0.25 * a["n_links"])
    assert np.allclose(a["cost_eur"], a["cost_running_eur"] + a["cost_toll_eur"])
    assert np.all(a["cost_fare_eur"] == 0)


def test_money_weighs_by_default_and_not_when_a_model_is_told_not_to() -> None:
    net = ms.examples.toy_network()
    rows = [toy_trip(net, "b", "W", "M", 0, None)]
    common = {"modes": ["car", "bike"], "transit": None, "parkings": None, "choice_model": "logit"}
    time = {"beta_time_min": -50.0}  # time all but decides: the car, 80 s against the bike's 120 s
    off = {**time, "beta_cost_eur": 0.0}
    dear = {"car_eur_km": 1000.0}
    plain = toy_run(rows, "money-plain", choice_options=off, **common)
    priced = toy_run(rows, "money-unweighed", choice_options=off, price_options=dear, **common)
    # Told not to weigh money: the prices change nothing, not even the fingerprint.
    assert list(plain.itinerary_choices()["mode"]) == ["car"]
    assert list(priced.itinerary_choices()["mode"]) == ["car"]
    assert priced.fingerprint == plain.fingerprint and plain.manifest()["prices"] is None
    # By default money weighs -1.2 per euro (D-1): at 1000 € a km the car is too dear.
    weighed = toy_run(rows, "money-weighed", price_options=dear, choice_options=time, **common)
    assert list(weighed.itinerary_choices()["mode"]) == ["bike"], "1000 € a km is too dear"
    assert weighed.manifest()["prices"]["car_eur_km"] == 1000.0
    assert weighed.fingerprint != plain.fingerprint


def test_unknown_or_negative_prices_are_refused() -> None:
    net = ms.examples.toy_network()
    rows = [toy_trip(net, "b", "W", "M", 0, None)]
    with pytest.raises(ValueError, match="fare_base_eur"):
        toy_run(rows, "money-bad", transit=None, parkings=None, price_options={"fare": 2.0})
    with pytest.raises(ValueError, match="0 or more"):
        toy_run(rows, "money-neg", transit=None, parkings=None, price_options={"car_eur_km": -1})


# --- traveller and trip values (S249, roadmap I-bb U6) -------------------------------------------


def test_a_model_sees_each_traveller_s_and_trip_s_values() -> None:
    import numpy as np
    from openmobisim import choice

    seen: list[dict[str, np.ndarray]] = []

    class Recorder:
        name = "recorder"
        attributes = ["time_min", "person_income_keur", "trip_work", "trip_departure_h"]

        def choose(self, batch):
            seen.append({k: np.array(v) for k, v in batch.attributes.items()})
            return choice.segment_argmax(-batch.attributes["time_min"], batch.offsets)

    net = ms.examples.toy_network()
    rows = [
        toy_trip(net, "a", "W", "M", 7 * 3600, None),
        toy_trip(net, "b", "W", "M", 9 * 3600, None),
    ]
    toy_run(rows, "values-demand", modes=["car", "bike"], transit=None, parkings=None,
            choice_model=Recorder(),
            person_values={"income_keur": {"a": 30.0, "b": 55.0, "not-here": 1.0}},
            trip_values={"work": [1.0, 0.0]})  # fmt: skip
    a = {k: np.concatenate([s[k] for s in seen]) for k in seen[0]}
    got = set(zip(a["trip_departure_h"], a["person_income_keur"], a["trip_work"], strict=True))
    assert got == {(7.0, 30.0, 1.0), (9.0, 55.0, 0.0)}, got


def test_trip_values_by_key_and_a_traveller_not_given_take_zero() -> None:
    import numpy as np
    from openmobisim import choice

    seen: list[dict[str, np.ndarray]] = []

    class Recorder:
        name = "recorder"
        attributes = ["trip_departure_h", "person_age", "trip_work"]

        def choose(self, batch):
            seen.append({k: np.array(v) for k, v in batch.attributes.items()})
            return choice.segment_argmax(batch.attributes["trip_departure_h"], batch.offsets)

    net = ms.examples.toy_network()
    rows = [
        toy_trip(net, "a", "W", "M", 7 * 3600, None),
        toy_trip(net, "b", "W", "M", 9 * 3600, None),
    ]
    toy_run(rows, "values-keyed", modes=["car", "bike"], transit=None, parkings=None,
            choice_model=Recorder(), person_values={"age": {"b": 40.0}},
            trip_values={"work": {("a", 0): 1.0, ("zz", 3): 5.0}})  # fmt: skip
    a = {k: np.concatenate([s[k] for s in seen]) for k in seen[0]}
    got = set(zip(a["trip_departure_h"], a["person_age"], a["trip_work"], strict=True))
    assert got == {(7.0, 0.0, 1.0), (9.0, 40.0, 0.0)}, got


def test_a_built_in_model_weighs_a_traveller_s_value_through_a_product() -> None:
    net = ms.examples.toy_network()
    rows = [toy_trip(net, "a", "W", "M", 0, None), toy_trip(net, "b", "W", "M", 0, None)]
    common = {"modes": ["car", "bike"], "transit": None, "parkings": None, "choice_model": "logit"}
    # Time all but decides (the car, 80 s against 120 s); a bike constant for the young turns it.
    options = {"beta_time_min": -50.0, "beta_person_young*mode_bike": 100.0}
    run = toy_run(rows, "values-product", choice_options=options,
                  person_values={"young": {"a": 1.0, "b": 0.0}}, **common)  # fmt: skip
    chosen = run.itinerary_choices()
    by_traveller = dict(zip(chosen["traveller_id"], chosen["mode"], strict=True))
    assert by_traveller == {"a": "bike", "b": "car"}, by_traveller
    assert run.manifest()["person_values"] == ["young"] and run.manifest()["trip_values"] is None
    plain = toy_run(rows, "values-product-none", choice_options=options,
                    person_values={"young": {"a": 0.0}}, **common)  # fmt: skip
    assert list(plain.itinerary_choices()["mode"]) == ["car", "car"]
    assert plain.fingerprint != run.fingerprint


def test_a_parking_fee_at_the_destination_is_paid_by_who_arrives_by_car() -> None:
    import numpy as np
    from openmobisim import choice

    seen: list[dict[str, np.ndarray]] = []

    class Recorder:
        name = "recorder"
        attributes = ["time_min", "mode_car", "cost_parking_eur", "trip_parking_eur"]

        def choose(self, batch):
            seen.append({k: np.array(v) for k, v in batch.attributes.items()})
            return choice.segment_argmax(-batch.attributes["time_min"], batch.offsets)

    net = ms.examples.toy_network()
    rows = [toy_trip(net, "a", "W", "M", 0, None), toy_trip(net, "b", "W", "M", 0, "car")]
    toy_run(rows, "values-parking", modes=["car", "bike"], transit=None, parkings=None,
            choice_model=Recorder(), trip_values={"parking_eur": [6.0, 4.0]})  # fmt: skip
    a = {k: np.concatenate([s[k] for s in seen]) for k in seen[0]}
    car = a["mode_car"] > 0
    assert set(a["cost_parking_eur"][car]) == {6.0, 4.0}, "the car routes, chosen or given"
    assert np.all(a["cost_parking_eur"][~car] == 0), "a bike pays no car park"
    assert set(a["trip_parking_eur"]) == {6.0, 4.0}
    # Weighed, a dear one turns the car away.
    common = {"modes": ["car", "bike"], "transit": None, "parkings": None, "choice_model": "logit"}
    options = {"beta_time_min": -50.0, "beta_cost_eur": -1.0}
    dear = toy_run(rows[:1], "values-parking-dear", choice_options=options,
                   trip_values={"parking_eur": [500.0]}, **common)  # fmt: skip
    assert list(dear.itinerary_choices()["mode"]) == ["bike"]


@pytest.mark.parametrize(
    ("kwargs", "message"),
    [
        ({"trip_values": {"work": [1.0]}}, "has 1 values; the demand has 2 rows"),
        ({"trip_values": {"Work": [1.0, 0.0]}}, "lower-case"),
        ({"trip_values": {"departure_h": [1.0, 0.0]}}, "built-in attribute"),
        ({"trip_values": {"parking_eur": [-1.0, 0.0]}}, "below 0"),
        ({"trip_values": {"work": {"a": 1.0}}}, "traveller_id, trip_seq"),
        ({"person_values": {"age": [1.0, 2.0]}}, "mapping from traveller id"),
        ({"person_values": {"age": {"a": float("nan")}}}, "finite"),
    ],
)
def test_wrong_traveller_or_trip_values_are_refused(kwargs: dict, message: str) -> None:
    net = ms.examples.toy_network()
    rows = [toy_trip(net, "a", "W", "M", 0, None), toy_trip(net, "b", "W", "M", 0, None)]
    with pytest.raises(ValueError, match=message):
        toy_run(rows, "values-wrong", modes=["car", "bike"], transit=None, parkings=None,
                **kwargs)  # fmt: skip


def test_each_trip_s_logsum_rises_with_a_better_alternative_and_a_model_may_give_its_own() -> None:
    # S238: utility-based accessibility. W → N1 chooses among bike, walk and the parkings.
    import numpy as np

    net = ms.examples.toy_network()
    rows = [toy_trip(net, "a", "W", "N1", 0, None)]
    plain = toy_run(rows, "logsum-plain", modes=ms.MODES, choice_model="logit")
    keen = toy_run(rows, "logsum-keen", modes=ms.MODES, choice_model="logit",
                   choice_options={"beta_mode_bike": 1.0})  # fmt: skip
    ls_plain, ls_keen = plain.itinerary_choices()["logsum"], keen.itinerary_choices()["logsum"]
    assert np.isfinite(ls_plain).all() and ls_keen[0] > ls_plain[0], "a better bike, a higher sum"
    assert np.isnan(toy_run(rows, "logsum-none", modes=ms.MODES).itinerary_choices()["logsum"][0])

    class Own:
        name = "own"

        def choose(self, batch):
            from openmobisim import choice

            return choice.segment_argmax(-batch.attributes["time_min"], batch.offsets)

        def logsum(self, batch):
            return np.full(len(batch), 7.0)

    own = toy_run(rows, "logsum-own", modes=ms.MODES, choice_model=Own())
    assert list(own.itinerary_choices()["logsum"]) == [7.0]


def test_the_logsums_by_pair_are_the_people_weighted_means_of_its_trips() -> None:
    # S245: a pair's logsum is its trips' mean, by weight; a trip given its mode has none.
    net = ms.examples.toy_network()
    rows = [toy_trip(net, f"a{i}", "W", "N1", 0, None) for i in range(3)]
    rows += [toy_trip(net, f"b{i}", "W", "M", 10 * i, None) for i in range(2)]
    rows += [toy_trip(net, "c0", "W", "M", 0, "car")]
    rows[0] = rows[0][:8] + (3,) + rows[0][9:]  # a0 stands for three people
    rows = [r[:7] + ("keen" if r[0] in ("a1", "b1") else "plain",) + r[8:] for r in rows]
    run = toy_run(rows, "logsum-od", modes=ms.MODES, choice_model="logit")
    ch = run.itinerary_choices()
    trip = {t: v for t, v in zip(ch["traveller_id"], ch["logsum"], strict=True)}
    zones = {
        name: net.node_lonlat(n) for name, n in (("west", "W"), ("north", "N1"), ("merge", "M"))
    }
    od = run.logsum_od(rows, zones)
    assert (od["origin"], od["destination"]) == (["west", "west"], ["merge", "north"])
    assert od["trips"] == [2, 3] and od["people"] == [2.0, 5.0], "c0's stated car has no logsum"
    assert od["logsum"][1] == pytest.approx((3 * trip["a0"] + trip["a1"] + trip["a2"]) / 5)
    nodes = run.logsum_od(rows)  # by the nodes the trips start and end at
    assert (
        sorted(nodes["trips"]) == [2, 3]
        and nodes["origin"] == [net.node_nearest(*zones["west"])] * 2
    )
    by_class = run.logsum_od(rows, zones, by_class=True)
    assert set(zip(by_class["destination"], by_class["user_class"], strict=True)) == {
        ("merge", "keen"), ("merge", "plain"), ("north", "keen"), ("north", "plain"),
    }  # fmt: skip
    with pytest.raises(ValueError, match="not in trips"):
        run.logsum_od(rows[1:], zones)
