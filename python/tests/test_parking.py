"""Park-and-ride and bike-and-ride.

Parkings from a table or OpenStreetMap, the station chosen by the choice model, soft
capacity, and what a run reports.

The toy network's hand values are the Rust suite's (``core-sim/tests/toy_parking.rs``):
PR1 (``W`` by car to ``H``, park, the tram to ``N1``: **1200 s**), BR1 (``S`` by bike,
the same tram: **1200 s**) and PR2 (seven cars at a six-space car park).
"""

from __future__ import annotations

import os

import openmobisim as ms
import pytest


def toy_trip(
    net: object, who: str, frm: str, to: str, depart: int, mode: str, seq: int = 0
) -> tuple:
    o, d = net.node_lonlat(frm), net.node_lonlat(to)
    return (who, seq, o[0], o[1], d[0], d[1], depart, "everyone", None, mode)


def toy_run(rows: list[tuple], run_id: str, parkings: object = None, **kwargs: object) -> ms.Run:
    return ms.Scenario.from_parts(
        network=ms.examples.toy_network(),
        demand=rows,
        classes={"everyone": (True, True, False)},
        transit=ms.examples.toy_network_transit(),
        parkings=ms.examples.toy_network_parkings() if parkings is None else parkings,
        choice_model=kwargs.pop("choice_model", "deterministic"),
        equilibration=kwargs.pop("equilibration", "free_flow"),
        flow_level=kwargs.pop("flow_level", 0),
        link_bin_s=300,
        **kwargs,
    ).run(run_id=run_id)


def only(*ids: str) -> ms.Parkings:
    rows = ms.examples.toy_network_parkings().rows()
    keep = [i for i, p in enumerate(rows["parking_id"]) if p in ids]
    return ms.parking_read_table(
        [
            {
                "parking_id": rows["parking_id"][i],
                "lon": rows["lon"][i],
                "lat": rows["lat"][i],
                "vehicle": rows["vehicle"][i],
                "capacity": int(rows["capacity"][i]),
                "hub_id": rows["hub_id"][i],
            }
            for i in keep
        ]
    )


# --- reading parkings -------------------------------------------------------------------------


def test_a_parking_table_is_read_from_rows_or_text() -> None:
    p = ms.parking_read_table(
        [
            {"parking_id": "a", "lon": 4.9, "lat": 52.37, "vehicle": "car", "capacity": 400},
            {
                "Parking_ID": "b",
                "LON": "4.91",
                "lat": "52.37",
                "vehicle": "Bike",
                "capacity": "80",
                "hub_id": "st",
                "initial_occupancy": 5,
            },
        ]
    )
    assert p.count == 2
    assert p.by_vehicle() == {"car": (1, 400), "bike": (1, 80)}
    rows = p.rows()
    assert rows["hub_id"] == [None, "st"] and list(rows["initial_occupancy"]) == [0, 5]
    assert p.read_report() is None
    text = "parking_id,lon,lat,vehicle,capacity\nx,4.9,52.37,car,10\n"
    assert ms.parking_read_table(text).by_vehicle()["car"] == (1, 10)
    # A fee per stay (S248), or none: the scenario's price for the kind.
    assert rows["fee_eur"] == [None, None]
    fees = (
        "parking_id,lon,lat,vehicle,capacity,fee_eur\nx,4.9,52.37,car,10,3.5\ny,4.9,52.37,bike,5,\n"
    )
    assert ms.parking_read_table(fees).rows()["fee_eur"] == [3.5, None]


def test_a_bad_parking_table_is_refused_with_what_is_wrong() -> None:
    row = {"parking_id": "a", "lon": 4.9, "lat": 52.37, "vehicle": "car", "capacity": 1}
    with pytest.raises(ValueError, match="vehicle"):
        ms.parking_read_table([{**row, "vehicle": "boat"}])
    with pytest.raises(ValueError, match="fee_eur must be 0 or more"):
        ms.parking_read_table([{**row, "fee_eur": -1}])
    with pytest.raises(ValueError, match="more than once"):
        ms.parking_read_table([row, row])
    with pytest.raises(ValueError, match="capacity"):
        ms.parking_read_table([{k: v for k, v in row.items() if k != "capacity"}])
    with pytest.raises(ValueError, match="whole number"):
        ms.parking_read_table([{**row, "capacity": 2.5}])
    with pytest.raises(ValueError, match="lon and lat"):
        ms.parking_read_table([{k: v for k, v in row.items() if k != "lat"}])


# --- park-and-ride and bike-and-ride on the toy network ---------------------------------------


def test_pr1_park_and_ride_across_the_python_boundary() -> None:
    net = ms.examples.toy_network()
    run = toy_run([toy_trip(net, "a", "W", "N1", 0, "car_transit")], "pr1")
    assert run.completion["completed"] == 1
    assert run.completion_by_mode["car_transit"]["total_travel_time_s"] == 1200.0
    choices = run.itinerary_choices()
    assert choices is not None
    assert choices["parking_id"] == ["H-car"] and choices["direction"] == ["out"]
    assert int(choices["alternatives"][0]) == 2 and int(choices["rides"][0]) == 1
    places = run.parking_places()
    assert places is not None and set(places["parking_id"]) == {"H-bike", "H-car", "P2"}
    bins = run.parking_bins()
    assert bins is not None
    h = [i for i, p in enumerate(bins["parking_id"]) if p == "H-car"]
    assert sum(bins["arrivals"][i] for i in h) == 1.0
    assert run.parking_summary["car_arrivals"] == 1.0
    assert run.parking_summary["car_left_at_end"] == 1.0, "nobody fetched it"


def test_the_parking_bins_file_and_the_kpis_rows() -> None:
    pa = pytest.importorskip("pyarrow.parquet")
    net = ms.examples.toy_network()
    run = toy_run([toy_trip(net, "a", "W", "N1", 0, "car_transit")], "pr1-files")
    table = run.parking_bins_table()
    assert table is not None
    t = pa.read_table(table.path)
    assert t.column_names == [
        "run_id",
        "bin",
        "start_s",
        "parking",
        "parking_id",
        "hub_id",
        "vehicle",
        "capacity",
        "arrivals",
        "departures",
        "occupancy_mean",
        "occupancy_max",
        "full_s",
        "overflow_max",
    ]
    assert t.schema.metadata[b"openmobisim.run_fingerprint"].decode() == run.fingerprint
    kpis = pa.read_table(run.kpis().path).to_pylist()
    got = {(r["mode"], r["metric"]): r["value"] for r in kpis}
    assert got[("car_transit", "parking_arrivals")] == 1.0
    assert got[("car_transit", "parking_overflow_arrivals")] == 0.0
    assert got[("car_transit", "trips")] == 1.0


def test_pr2_the_seventh_car_parks_anyway_and_is_counted() -> None:
    net = ms.examples.toy_network()
    rows = [toy_trip(net, f"c{i}", "W", "N1", 0, "car_transit") for i in range(7)]
    run = toy_run(rows, "pr2", parkings=only("H-car"))
    assert run.completion["completed"] == 7
    s = run.parking_summary
    assert (s["car_arrivals"], s["car_overflow_arrivals"]) == (7.0, 1.0)
    assert s["car_mismatch_s"] == pytest.approx(480.0)
    bins = run.parking_bins()
    assert max(bins["overflow_max"]) == 1.0


def test_br1_bike_and_ride() -> None:
    net = ms.examples.toy_network()
    run = toy_run([toy_trip(net, "a", "S", "N1", 0, "bike_transit")], "br1")
    assert run.completion_by_mode["bike_transit"]["total_travel_time_s"] == 1200.0
    assert run.itinerary_choices()["parking_id"] == ["H-bike"]
    assert run.link_bins(layer="bike") is not None


def test_options_replace_the_defaults_and_bad_ones_are_refused() -> None:
    net = ms.examples.toy_network()
    rows = [toy_trip(net, "a", "W", "N1", 0, "car_transit")]
    # Parking in no time: parked at 180, the 300 tram, N1 at 600.
    quick = toy_run(rows, "pr1-quick", parking_options={"floor_car_s": 0})
    assert quick.completion_by_mode["car_transit"]["total_travel_time_s"] == 600.0
    # A boarding slack of 0 makes no difference here (the car parks at 300.39: the 300 tram
    # has gone either way); a missing name and a bad value are refused.
    with pytest.raises(ValueError, match="floor_car"):
        toy_run(rows, "bad", parking_options={"floor_car": 0})
    with pytest.raises(ValueError, match="max_rides"):
        toy_run(rows, "bad", transit_options={"max_rides": 0})
    with pytest.raises(ValueError, match="board_slack"):
        toy_run(rows, "bad", transit_options={"board_slack": 0})


def test_parkings_need_a_timetable() -> None:
    net = ms.examples.toy_network()
    with pytest.raises(ValueError, match="transit"):
        ms.Scenario.from_parts(
            network=net,
            demand=[toy_trip(net, "a", "W", "N1", 0, "car_transit")],
            classes={"everyone": (True, True, False)},
            parkings=ms.examples.toy_network_parkings(),
        ).run(run_id="no-timetable")


def test_the_trip_back_and_an_iterating_run_under_the_loading_model() -> None:
    net = ms.examples.toy_network()
    rows = [
        toy_trip(net, "a", "W", "N1", 0, "car_transit"),
        toy_trip(net, "a", "N1", "D2", 1800, "car_transit", seq=1),
        toy_trip(net, "b", "S", "N1", 0, "bike_transit"),
        toy_trip(net, "c", "N1", "D2", 0, "transit"),
    ]
    run = toy_run(
        rows,
        "pr-msa",
        parkings=only("P2", "H-bike"),
        choice_model="logit",
        equilibration="msa",
        equilibration_options={"iterations": 3},
        flow_level=4,
    )
    assert run.completion["completed"] == 4
    choices = run.itinerary_choices()
    assert choices["direction"] == ["out", "back", "out", ""]
    assert choices["traveller_id"] == ["a", "a", "b", "c"]
    assert list(choices["trip_seq"]) == [0, 1, 0, 0]
    assert choices["mode"] == ["car_transit", "car_transit", "bike_transit", "transit"]
    c = run.convergence()
    assert {"gap_transit", "gap_car_transit", "gap_bike_transit", "hub_mismatch_s"} <= set(c)
    assert len(c["gap_car_transit"]) == 3
    # Out 1200, as the Rust PR4. Back: planned on the schedule at first (the tram to H, 225 s
    # on foot to P2: 2925), then on the times the run made, where the bus B1 reaches D1 35 s
    # early (the Rust B1 case): walk N1 to W (318 s), the 2400 bus, D1 at 2605, 292 s on foot to
    # P2: 2897, 28 s sooner. Fetch 120 s, drive R2 to D2 36 s: 3053, 1253 s.
    assert run.completion_by_mode["car_transit"]["total_travel_time_s"] == pytest.approx(
        1200.0 + 1253.0
    )


@pytest.mark.skipif(
    not os.environ.get("OPENMOBISIM_TEST_PBF"), reason="set OPENMOBISIM_TEST_PBF to a real extract"
)
def test_parkings_are_read_from_a_real_extract() -> None:
    parkings = ms.parking_read_osm(os.environ["OPENMOBISIM_TEST_PBF"])
    report = parkings.read_report()
    assert report is not None
    assert parkings.count == report["sites_car"] + report["sites_bike"]
    assert report["capacity_tagged"] + report["capacity_from_area"] + report[
        "capacity_default"
    ] == (report["car_found"] + report["bike_found"])


def test_map_parking_draws_a_run_with_parkings(tmp_path) -> None:
    pytest.importorskip("matplotlib")
    from openmobisim import viz

    net = ms.examples.toy_network()
    rows = [
        toy_trip(net, "a", "W", "N1", 0, "car_transit"),
        toy_trip(net, "b", "S", "N1", 0, "bike_transit"),
    ]
    run = toy_run(rows, "pr-map")
    for theme in ("paper", "night"):
        out = tmp_path / f"parking_{theme}.png"
        fig = viz.map_parking(run, theme=theme, path=str(out))
        assert out.exists() and out.stat().st_size > 10_000
        texts = [t.get_text() for t in fig.texts]
        assert any("park-and-ride car park" in t for t in texts)
        assert any("bike parking" in t for t in texts)
        assert any("1 cars and 1 bikes parked" in t for t in texts)
    plain = ms.Scenario.from_parts(
        network=net,
        demand=[toy_trip(net, "c", "N1", "D2", 0, "transit")],
        classes={"everyone": (True, True, False)},
        transit=ms.examples.toy_network_transit(),
        equilibration="free_flow",
    ).run(run_id="no-parkings")
    with pytest.raises(ValueError, match="parkings"):
        viz.map_parking(plain)
