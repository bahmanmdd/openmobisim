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
        class_defaults={"everyone": (True, True, False)},
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
