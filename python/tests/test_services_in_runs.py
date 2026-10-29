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
