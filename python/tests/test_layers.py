"""Bike and walk trips (S195): the layers of a network, trips with a mode, results by mode."""

from __future__ import annotations

import os

import numpy as np
import openmobisim as ms
import pytest


def grid() -> object:
    return ms.examples.manhattan_grid(5, 200.0, False)


def everyone_owns(car: bool = True, bike: bool = True) -> dict[str, tuple[bool, bool, bool]]:
    return {"everyone": (car, bike, False)}


def trip(net: object, who: str, frm: tuple[int, int], to: tuple[int, int], mode: str | None):
    """One trips row between two grid positions, departing at 8:00."""
    o = ms._core.grid_node_lonlat(net, *frm)
    d = ms._core.grid_node_lonlat(net, *to)
    row = (who, 0, o[0], o[1], d[0], d[1], 8 * 3600, "everyone", None)
    return row if mode is None else (*row, mode)


def run(net: object, rows: list[tuple], run_id: str, **kwargs: object) -> ms.Run:
    return ms.Scenario.from_parts(
        network=net,
        demand=rows,
        class_defaults=kwargs.pop("class_defaults", everyone_owns()),
        link_bin_s=300,
        **kwargs,
    ).run(run_id=run_id)


# --- the layers of a network ---------------------------------------------------------------------


def test_a_grid_has_bike_and_walk_layers_derived_from_its_roads() -> None:
    net = grid()
    assert net.layer_name == "road"
    bike, walk = net.layer("bike"), net.layer("walk")
    assert (bike.layer_name, walk.layer_name) == ("bike", "walk")
    # A two-way grid is the same graph for each.
    assert bike.link_count == walk.link_count == net.link_count
    assert np.allclose(bike.link_speed_km_h(), 15.0)
    assert np.allclose(walk.link_speed_km_h(), 4.8)
    assert (bike.link_infrastructure() == 0).all(), "no tags: mixed traffic"
    assert np.allclose(bike.link_free_flow_s(), bike.link_length_m() / (15.0 / 3.6))
    assert np.isnan(bike.link_capacity_pcu_h()).all()


def test_the_toy_network_has_its_own_hand_checkable_layers() -> None:
    net = ms.examples.toy_network()
    bike, walk = net.layer("bike"), net.layer("walk")
    # Every street, the contraflow on s1 and the cycle track's four links.
    assert bike.link_count == net.link_count + 1 + 4
    assert (bike.link_infrastructure() == 2).sum() == 4
    # Every street both ways, and two walk-only paths both ways.
    assert walk.link_count == 2 * net.link_count + 4


def test_layer_questions_asked_of_the_wrong_network_are_refused() -> None:
    net = grid()
    with pytest.raises(ValueError, match="bike layer"):
        net.link_infrastructure()
    with pytest.raises(ValueError, match="road"):
        net.layer("cycle")
    with pytest.raises(ValueError, match="bike layer"):
        net.layer("bike").layer("walk")


# --- trips with a mode ---------------------------------------------------------------------------


def test_bike_and_walk_trips_run_on_their_layers_and_are_counted_by_mode() -> None:
    net = grid()
    rows = [
        trip(net, "c", (0, 0), (4, 4), "car"),
        trip(net, "b", (0, 0), (4, 4), "bike"),
        trip(net, "w", (0, 0), (0, 2), "walk"),
        trip(net, "x", (0, 0), (4, 4), None),  # no mode: a car trip
    ]
    r = run(net, rows, "layers-modes")
    by_mode = r.completion_by_mode
    assert set(by_mode) == {"car", "bike", "walk"}
    assert by_mode["car"]["total_trips"] == 2
    assert by_mode["bike"]["completed"] == 1 and by_mode["walk"]["completed"] == 1
    # About 1600 m at 15 km/h and 400 m at 4.8 km/h.
    assert 384 <= by_mode["bike"]["total_travel_time_s"] <= 387
    assert 300 <= by_mode["walk"]["total_travel_time_s"] <= 302
    total = sum(m["total_travel_time_s"] for m in by_mode.values())
    assert total == pytest.approx(r.total_travel_time_s)
    assert r.completion["mode_not_available"] == 0


def test_the_layers_per_link_results_are_their_own() -> None:
    net = grid()
    rows = [trip(net, f"b{i}", (0, 0), (4, 4), "bike") for i in range(3)]
    r = run(net, rows, "layers-bins")
    bins = r.link_bins("bike")
    assert bins is not None and r.link_bins("walk") is None
    assert int(bins.crossings().sum()) == 3 * 8, "three riders, eight blocks each"
    assert bins.pcu().sum() == pytest.approx(3 * 8), "travellers, not PCU"
    assert r.link_bins().crossings().sum() == 0, "no car moved"
    table = r.link_bins_table("bike").to_polars()
    assert {"travellers", "traveller_seconds"} <= set(table.columns)
    assert r.link_bins_table("walk") is None
    with pytest.raises(ValueError, match="layer"):
        r.link_bins("cycle")


def test_the_kpis_file_has_rows_per_mode() -> None:
    net = grid()
    rows = [trip(net, "c", (0, 0), (4, 4), "car"), trip(net, "b", (0, 0), (4, 4), "bike")]
    k = run(net, rows, "layers-kpis").kpis().to_polars()
    assert set(k["mode"].unique()) == {"all", "car", "bike"}
    trips = {m: v for m, v in k.filter(k["metric"] == "trips").select(["mode", "value"]).rows()}
    assert trips == {"all": 2.0, "car": 1.0, "bike": 1.0}


def test_a_bike_trip_needs_the_traveller_s_bike() -> None:
    net = grid()
    r = run(
        net,
        [trip(net, "b", (0, 0), (4, 4), "bike")],
        "layers-no-bike",
        class_defaults=everyone_owns(car=True, bike=False),
    )
    assert r.completion_by_mode["bike"]["no_vehicle_available"] == 1


def test_transit_is_reported_as_not_available_yet() -> None:
    net = grid()
    r = run(net, [trip(net, "t", (0, 0), (4, 4), "transit")], "layers-transit")
    assert r.completion["mode_not_available"] == 1


def test_a_bad_row_mode_or_bike_cost_is_refused() -> None:
    net = grid()
    with pytest.raises(ValueError, match="unknown mode"):
        run(net, [trip(net, "x", (0, 0), (4, 4), "cycle")], "layers-bad-mode")
    with pytest.raises(ValueError, match="9 fields"):
        ms.Scenario.from_parts(network=net, demand=[("x", 0, 4.8)])
    with pytest.raises(ValueError, match="bike_cost"):
        ms.Scenario.from_parts(network=net, demand=[], bike_cost="fastest")


def test_the_bike_cost_is_part_of_the_run_s_identity_only_when_bikes_ride() -> None:
    net = grid()
    bikes = [trip(net, "b", (0, 0), (4, 4), "bike")]
    cars = [trip(net, "c", (0, 0), (4, 4), "car")]
    fp = lambda rows, cost, name: run(net, rows, name, bike_cost=cost).fingerprint  # noqa: E731
    assert fp(bikes, "dedicated", "fp-bd") != fp(bikes, "time", "fp-bt")
    assert fp(cars, "dedicated", "fp-cd") == fp(cars, "time", "fp-ct")


def test_random_trips_can_carry_a_mode() -> None:
    net = grid()
    rows = ms.examples.trips_random(net, 5, seed=1, min_m=100, max_m=2000, mode="bike")
    assert all(len(r) == 10 and r[-1] == "bike" for r in rows)
    assert all(len(r) == 9 for r in ms.examples.trips_random(net, 5, seed=1, min_m=100))


def test_a_bike_map_draws_the_bike_layer() -> None:
    pytest.importorskip("matplotlib")
    from openmobisim import viz

    net = grid()
    rows = [trip(net, f"b{i}", (0, i % 5), (4, 4), "bike") for i in range(5)]
    r = run(net, rows, "layers-map")
    fig = viz.map_link(r, layer="bike")
    assert fig is not None
    with pytest.raises(ValueError, match="capacity"):
        viz.map_link(r, layer="bike", colour="volume_capacity")
    with pytest.raises(ValueError, match="walk layer"):
        viz.map_link(r, layer="walk")


# --- a real extract ------------------------------------------------------------------------------

PBF = os.environ.get("OPENMOBISIM_TEST_PBF")


@pytest.mark.skipif(PBF is None, reason="set OPENMOBISIM_TEST_PBF to an .osm.pbf extract")
def test_a_real_extract_reads_its_bike_and_walk_layers() -> None:
    net = ms.network_read_osm(PBF)
    report = net.report_import()
    assert set(report["layers"]) == {"bike", "walk"}
    bike = net.layer("bike")
    infra = bike.link_infrastructure()
    assert bike.link_count > 0 and set(np.unique(infra)) <= {0, 1, 2}
    assert report["layers"]["bike"]["dedicated_length_m"] <= report["layers"]["bike"]["length_m"]
    rows = ms.examples.trips_random(net, 20, seed=3, min_m=500, max_m=3000, mode="bike")
    r = ms.Scenario.from_parts(
        network=net, demand=rows, class_defaults={"commuter": (False, True, False)}
    ).run(run_id="layers-osm")
    assert r.completion_by_mode["bike"]["total_trips"] == 20
    assert r.completion_by_mode["bike"]["completed"] >= 18
    # Without layers read from the tags, the fallback derives them from the roads.
    plain = ms.network_read_osm(PBF, layers=False)
    assert plain.report_import()["layers"] == {}
    assert plain.layer("bike").link_count > 0
