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
        equilibration=kwargs.pop("equilibration", "none"),
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
