"""Shared bikes in a run: a docked bike service offers the mode ``bike_share`` to trips choosing.

The network: a 10 x 10 two-way grid of 200 m blocks. Docks of eight slots on its diagonal corners'
rows; travellers own no bike and no car and choose between walking and a shared bike.
"""

from __future__ import annotations

import time

import openmobisim as ms
import pytest

NO_ACCESS = {"car_access_s": 0, "car_egress_s": 0, "bike_access_s": 0, "bike_egress_s": 0}


def grid() -> object:
    return ms.examples.manhattan_grid(10, 200.0, False)


def at(net: object, row: int, col: int) -> tuple[float, float]:
    return ms._core.grid_node_lonlat(net, row, col)


def docks(net: object, bikes: int = 4) -> object:
    places = {"a": (0, 0), "b": (0, 1), "y": (9, 8), "z": (9, 9), "m": (5, 5)}
    return ms.dock_read_table(
        [
            {"dock_id": k, "lon": at(net, *p)[0], "lat": at(net, *p)[1], "capacity": 8}
            | {"bikes": bikes}
            for k, p in places.items()
        ]
    )


def trips(net: object, n: int, frm: tuple[int, int], to: tuple[int, int], cls: str = "c") -> list:
    o, d = at(net, *frm), at(net, *to)
    return [
        (f"{cls}{k}", 0, o[0], o[1], d[0], d[1], 28_800 + 20 * k, cls, None, None) for k in range(n)
    ]


def scenario(net: object, rows: list, **kwargs: object) -> ms.Scenario:
    classes = kwargs.pop("classes", {"c": (False, False, False)})
    return ms.Scenario.from_parts(
        network=net,
        demand=rows,
        classes=classes,
        modes=kwargs.pop("modes", ["walk", "bike_share"]),
        flow_level=0,
        mode_options=NO_ACCESS,
        **kwargs,
    )


def shares(run: ms.Run) -> dict[str, float]:
    by = run.completion_by_mode
    total = sum(m["total_trips"] for m in by.values())
    return {mode: m["total_trips"] / total for mode, m in by.items()}


def test_a_long_trip_takes_a_shared_bike_and_a_short_one_or_one_far_from_docks_walks() -> None:
    net = grid()
    bikes = ms.service_bike_docked(docks(net))
    rows = (
        trips(net, 3, (0, 0), (9, 9), "long")  # 3.6 km: docks a and z at its ends
        + trips(net, 2, (0, 0), (1, 0), "short")  # 200 m: walking is faster
        + trips(net, 2, (3, 9), (0, 9), "far")  # no dock within 600 s on foot of its origin
    )
    classes = {k: (False, False, False) for k in ("long", "short", "far")}
    run = scenario(net, rows, services=[bikes], classes=classes, choice_model="deterministic").run(
        "share-grid", quiet=True
    )
    modes = dict(zip(run.trip_modes()["traveller_id"], run.trip_modes()["mode"], strict=True))
    assert [modes[f"long{k}"] for k in range(3)] == ["bike_share"] * 3
    assert [modes[f"short{k}"] for k in range(2)] == ["walk"] * 2
    assert [modes[f"far{k}"] for k in range(2)] == ["walk"] * 2
    assert run.completion_by_mode["bike_share"]["completed"] == 3
    # A service's mode is listed by a run with the service.
    assert "gap_bike_share" in run.convergence()


def test_without_a_service_nothing_lists_bike_sharing_and_the_mode_is_refused() -> None:
    net = grid()
    rows = trips(net, 3, (0, 0), (9, 9))
    run = scenario(net, rows, modes=["walk", "bike"]).run("share-none", quiet=True)
    assert "gap_bike_share" not in run.convergence()
    assert "reason_bike_share" not in run.report_no_alternative()
    with pytest.raises(ValueError, match="bike_share.*services"):
        scenario(net, rows)


def test_a_class_without_shared_bikes_never_takes_one() -> None:
    net = grid()
    bikes = ms.service_bike_docked(docks(net))
    rows = trips(net, 3, (0, 0), (5, 5), "yes") + trips(net, 3, (0, 0), (5, 5), "no")
    classes = {
        "yes": {"owns_car": 0, "owns_bike": 0},
        "no": {"owns_car": 0, "owns_bike": 0, "has_bike_share": 0},
    }
    run = scenario(net, rows, services=[bikes], classes=classes, choice_model="deterministic").run(
        "share-class", quiet=True
    )
    t = run.trip_modes()
    modes = dict(zip(t["traveller_id"], t["mode"], strict=True))
    assert {modes[f"yes{k}"] for k in range(3)} == {"bike_share"}
    assert {modes[f"no{k}"] for k in range(3)} == {"walk"}


def test_a_price_per_ride_lowers_the_shared_bikes_share() -> None:
    net = grid()
    bikes = ms.service_bike_docked(docks(net, bikes=8))
    rows = trips(net, 60, (0, 0), (5, 6))

    def share(prices: dict[str, float]) -> float:
        s = scenario(net, rows, services=[bikes], choice_model="logit", price_options=prices)
        return shares(s.run("share-price", quiet=True, overwrite=True)).get("bike_share", 0.0)

    free = share({})
    priced = share({"bike_share_eur_per_ride": 0.5})
    assert 0 < priced < free, (free, priced)


def test_the_docks_expectations_are_seconds_and_service_met_gives_what_the_day_met(
    tmp_path: object,
) -> None:
    pytest.importorskip("pyarrow")  # the trip tables are read from parquet
    import math

    import numpy as np

    net = grid()
    # Two bikes a dock for thirty trips of 2.2 km (walking is an alternative): most find a and b
    # empty and walk on or give up.
    bikes = ms.service_bike_docked(docks(net, bikes=2))
    rows = trips(net, 30, (0, 0), (5, 6))
    prices = {"bike_share_eur_per_ride": 0.5}
    s = scenario(net, rows, services=[bikes], choice_model="logit", price_options=prices)
    share = {}
    for preset in ("optimistic", "default", "pessimistic"):
        r = s.run(
            preset, output_dir=str(tmp_path / preset), quiet=True, service_expectations=preset
        )
        share[preset] = shares(r).get("bike_share", 0.0)
    print(f"\nshared-bike share by preset: {share}")
    assert share["optimistic"] >= share["default"] >= share["pessimistic"]
    assert share["optimistic"] > share["pessimistic"], "the presets move the share"
    run = s.run("d", output_dir=str(tmp_path / "d"), quiet=True)
    e = run.service_expectations()
    assert e["dock_pickup_extra_s"] is None and e["dock_return_extra_s"] is None
    # What the day met at the docks: the people-weighted means of the trips whose leg ended.
    met = run.service_met()
    t = run.bike_share_trips()
    w = np.asarray(t["travellers"], dtype=float)
    for name, column in (
        ("dock_pickup_extra_s", "pickup_extra_s"),
        ("dock_return_extra_s", "return_extra_s"),
    ):
        v = np.asarray(t[column], dtype=float)
        ended = np.isfinite(v)
        assert ended.any()
        assert met[name] == pytest.approx(float(np.sum(w[ended] * v[ended]) / np.sum(w[ended])))
    extra = np.asarray(t["extra_s"], dtype=float)
    split = np.asarray(t["pickup_extra_s"], dtype=float) + np.asarray(
        t["return_extra_s"], dtype=float
    )
    assert np.allclose(split[np.isfinite(extra)], extra[np.isfinite(extra)])
    assert met["dock_pickup_extra_s"] > 0, "empty pickup docks cost the travellers"
    # Read back from its folder, the same; and a next run plans on it.
    opened = ms.Run.open(tmp_path / "d")
    assert opened.service_met() == pytest.approx(met)
    assert opened.service_expectations() == e
    again = s.run("met", output_dir=str(tmp_path / "met"), quiet=True, service_expectations=met)
    assert again.service_expectations()["dock_pickup_extra_s"] == met["dock_pickup_extra_s"]
    assert again.fingerprint != run.fingerprint
    # A dock's extra walk is NaN or seconds, 0 or more.
    nan = s.run(
        "nan",
        output_dir=str(tmp_path / "nan"),
        quiet=True,
        service_expectations={"dock_pickup_extra_s": math.nan},
    )
    assert nan.fingerprint == run.fingerprint, "NaN is the default"
    with pytest.raises(ValueError, match="dock_pickup_extra_s"):
        s.run(
            "bad",
            output_dir=str(tmp_path / "bad"),
            service_expectations={"dock_pickup_extra_s": -5},
        )
    with pytest.raises(ValueError, match="dock_return_extra_s"):
        s.run(
            "bad", output_dir=str(tmp_path / "bad"), service_expectations={"dock_return_factor": 1}
        )


def test_same_seed_same_results_and_the_service_is_in_the_fingerprint() -> None:
    net = grid()
    bikes = ms.service_bike_docked(docks(net, bikes=2))
    rows = trips(net, 30, (0, 0), (9, 9))
    s = scenario(net, rows, services=[bikes], choice_model="logit", equilibration="msa")
    a, b = s.run("share-a", quiet=True), s.run("share-b", quiet=True)
    assert a.fingerprint == b.fingerprint
    assert list(a.trip_modes()["mode"]) == list(b.trip_modes()["mode"])
    assert a.total_travel_time_s == b.total_travel_time_s
    other = ms.service_bike_docked(docks(net, bikes=3))
    c = scenario(net, rows, services=[other], choice_model="logit", equilibration="msa")
    assert c._fingerprint() != a.fingerprint, "the docks' bikes are in the fingerprint"


def test_the_grid_run_time_with_and_without_the_service() -> None:
    """The same 600 trips with the service and without it: seconds each (``pytest -s``)."""
    net = grid()
    bikes = ms.service_bike_docked(docks(net, bikes=8))
    rows = [
        r
        for k, (frm, to) in enumerate(
            [((0, 0), (9, 9)), ((0, 1), (5, 5)), ((9, 8), (0, 0)), ((5, 5), (9, 9))] * 1
        )
        for r in trips(net, 150, frm, to, f"g{k}")
    ]
    classes = {f"g{k}": (False, True, False) for k in range(4)}
    kw = {"classes": classes, "choice_model": "logit", "equilibration": "msa"}
    started = time.perf_counter()
    plain = scenario(net, rows, modes=["walk", "bike"], **kw).run("time-plain", quiet=True)
    t_plain = time.perf_counter() - started
    started = time.perf_counter()
    shared = scenario(net, rows, modes=["walk", "bike", "bike_share"], services=[bikes], **kw).run(
        "time-share", quiet=True
    )
    t_share = time.perf_counter() - started
    print(f"\ngrid 10 x 10, 600 trips: without the service {t_plain:.2f} s, with {t_share:.2f} s")
    print(f"shares with the service: {shares(shared)}")
    assert plain.completion["total_trips"] == shared.completion["total_trips"] == 600


# --- a shared bike to a stop's hub, then transit (bike_share_transit) ---------------------------
#
# A line on the grid's last column, from stop A (row 0) to stop B (row 9), five minutes, every
# ten from 08:00. Dock "o" at the origin's corner (row 0, column 0) and dock "a" at stop A (it
# joins A's hub); travellers own nothing. From row 0, column 0 to row 9, column 9 (3.6 km) the
# walk is beyond the shipped 1 800 s; walking to A (1.8 km, within the 1 800 s to a stop) and
# riding is transit; a shared bike from "o" to "a" and the line is the chain.

LINE = {
    "routes.txt": "route_id,route_short_name,route_long_name,route_type\nL,L,Line,3\n",
    "calendar.txt": "service_id,monday,tuesday,wednesday,thursday,friday,saturday,sunday,"
    "start_date,end_date\nWK,1,1,1,1,1,1,1,20261005,20261011\n",
}


def line_feed(net: object, folder: object) -> object:
    a, b = at(net, 0, 9), at(net, 9, 9)
    files = dict(LINE)
    files["stops.txt"] = (
        "stop_id,stop_name,stop_lat,stop_lon,location_type,parent_station\n"
        f"A,Alpha,{a[1]},{a[0]},0,\nB,Bravo,{b[1]},{b[0]},0,\n"
    )
    runs = [f"T{k}" for k in range(10)]
    files["trips.txt"] = "route_id,service_id,trip_id\n" + "".join(f"L,WK,{r}\n" for r in runs)
    times = "trip_id,arrival_time,departure_time,stop_id,stop_sequence\n"
    for k, r in enumerate(runs):
        h, m = divmod(8 * 60 + 10 * k, 60)
        times += f"{r},{h:02}:{m:02}:00,{h:02}:{m:02}:00,A,1\n"
        h, m = divmod(8 * 60 + 10 * k + 5, 60)
        times += f"{r},{h:02}:{m:02}:00,{h:02}:{m:02}:00,B,2\n"
    files["stop_times.txt"] = times
    folder.mkdir(parents=True, exist_ok=True)
    for name, text in files.items():
        (folder / name).write_text(text, encoding="utf-8")
    return ms.transit_read_gtfs(str(folder), network=net)


def chain_docks(net: object, a_row: int = 0) -> object:
    places = {"o": (0, 0), "a": (a_row, 9)}
    return ms.dock_read_table(
        [
            {"dock_id": k, "lon": at(net, *p)[0], "lat": at(net, *p)[1], "capacity": 8, "bikes": 4}
            for k, p in places.items()
        ]
    )


CHAIN_MODES = ["walk", "transit", "bike_share_transit"]


def test_a_shared_bike_to_the_stop_is_taken_over_walking_to_it(tmp_path: object) -> None:
    net = grid()
    transit = line_feed(net, tmp_path / "line")
    rows = trips(net, 3, (0, 0), (9, 9), "yes") + trips(net, 2, (0, 0), (9, 9), "no")
    classes = {
        "yes": {"owns_car": 0, "owns_bike": 0},
        "no": {"owns_car": 0, "owns_bike": 0, "has_bike_share": 0},
    }
    kw = {"transit": transit, "classes": classes, "modes": CHAIN_MODES}
    kw |= {"choice_model": "deterministic"}
    run = scenario(net, rows, services=[ms.service_bike_docked(chain_docks(net))], **kw).run(
        "chain", quiet=True
    )
    t = run.trip_modes()
    modes = dict(zip(t["traveller_id"], t["mode"], strict=True))
    assert {modes[f"yes{k}"] for k in range(3)} == {"bike_share_transit"}
    # A class without shared bikes walks to the stop.
    assert {modes[f"no{k}"] for k in range(2)} == {"transit"}
    assert run.completion_by_mode["bike_share_transit"]["completed"] == 3
    assert "gap_bike_share_transit" in run.convergence()
    assert "reason_bike_share_transit" in run.report_no_alternative()
    # Dock "a" 400 m from the stop joins no stop's hub: no chain.
    far = scenario(net, rows, services=[ms.service_bike_docked(chain_docks(net, 2))], **kw)
    t = far.run("chain-far", quiet=True).trip_modes()
    assert set(t["mode"]) == {"transit"}


def test_the_chain_needs_a_service_and_a_timetable(tmp_path: object) -> None:
    net = grid()
    transit = line_feed(net, tmp_path / "line")
    rows = trips(net, 2, (0, 0), (9, 9))
    with pytest.raises(ValueError, match="bike_share_transit.*services"):
        scenario(net, rows, transit=transit, modes=CHAIN_MODES)
    with pytest.raises(ValueError, match="bike_share_transit.*transit"):
        scenario(
            net,
            rows,
            services=[ms.service_bike_docked(chain_docks(net))],
            modes=["walk", "bike_share_transit"],
        )
    with pytest.raises(ValueError, match="among other modes"):
        scenario(
            net,
            rows,
            transit=transit,
            services=[ms.service_bike_docked(chain_docks(net))],
            modes=["bike_share_transit"],
        )
    # Without a service nothing lists the chain.
    run = scenario(net, rows, transit=transit, modes=["walk", "transit"]).run(
        "no-chain", quiet=True
    )
    assert "gap_bike_share_transit" not in run.convergence()


def test_same_seed_same_chain(tmp_path: object) -> None:
    net = grid()
    transit = line_feed(net, tmp_path / "line")
    rows = trips(net, 30, (0, 0), (9, 9))
    bikes = ms.service_bike_docked(chain_docks(net))
    kw = {"transit": transit, "modes": CHAIN_MODES, "services": [bikes]}
    s = scenario(net, rows, choice_model="logit", equilibration="msa", **kw)
    a, b = s.run("chain-a", quiet=True), s.run("chain-b", quiet=True)
    assert a.fingerprint == b.fingerprint
    assert list(a.trip_modes()["mode"]) == list(b.trip_modes()["mode"])
    assert a.total_travel_time_s == b.total_travel_time_s
    assert "bike_share_transit" in set(a.trip_modes()["mode"])
