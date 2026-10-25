"""``demand_sample``: one traveller for every ``weight`` people, chains whole, shares kept.

``demand_aggregate_space`` (S298): with ``grid_m`` every trip end of the listed modes goes to the
centre of its square cell, by hand on a 1 km square; with ``zones`` it does what it did before;
the original ends come back in ``stats`` for the moved trips only, and a run takes the result.
"""

from __future__ import annotations

import math
from collections import Counter

import openmobisim as ms
import pytest

CAR = {"commuter": (True, False, False)}


def table(n: int) -> list[tuple]:
    """``n`` travellers with two trips each; a tenth of them cycle, the rest drive."""
    rows = []
    for i in range(n):
        mode = "bike" if i % 10 == 0 else "car"
        cls = "a" if i % 3 else "b"
        for seq in (0, 1):
            rows.append((f"t{i}", seq, 4.0, 52.0, 4.1, 52.1, 3600 * (7 + seq), cls, None, mode))
    return rows


def test_one_in_weight_travellers_is_kept_with_whole_chains_and_weights_multiplied() -> None:
    rows = table(1000)
    kept = ms.demand_sample(rows, 10)
    travellers = Counter(r[0] for r in kept)
    assert 98 <= len(travellers) <= 102, "a tenth of 1 000, per group to within one"
    assert set(travellers.values()) == {2}, "every kept traveller keeps both trips"
    assert {r[8] for r in kept} == {10}
    # The rows come back in their original order.
    order = {r[0]: i for i, r in enumerate(rows) if r[1] == 0}
    firsts = [order[r[0]] for r in kept if r[1] == 0]
    assert firsts == sorted(firsts)


def test_each_class_and_mode_keeps_its_share() -> None:
    rows = table(3000)
    kept = ms.demand_sample(rows, 10)

    def groups(rs: list[tuple]) -> Counter:
        return Counter((r[7], r[9]) for r in rs if r[1] == 0)

    full, sample = groups(rows), groups(kept)
    for key, n in full.items():
        assert abs(sample[key] * 10 - n) < 10, key


def test_the_sample_is_fixed_by_its_seed() -> None:
    rows = table(500)
    a, b = ms.demand_sample(rows, 5, seed=1), ms.demand_sample(rows, 5, seed=1)
    c = ms.demand_sample(rows, 5, seed=2)
    assert a == b
    assert {r[0] for r in a} != {r[0] for r in c}


def test_weight_one_keeps_everything_and_bad_weights_are_refused() -> None:
    rows = table(20)
    assert ms.demand_sample(rows, 1) == rows
    for bad in (0, 2.5, -3, True):
        with pytest.raises(ValueError, match="whole number"):
            ms.demand_sample(rows, bad)


def test_a_given_weight_is_multiplied() -> None:
    rows = [("x", 0, 4.0, 52.0, 4.1, 52.1, 0, "a", 3, "car")]
    assert ms.demand_sample(rows, 4)[0][8] in (12,)


# The grid's corner: the whole degrees at or below the westernmost and southernmost trip end.
LON0, LAT0 = 4.0, 52.0
# Metres per degree, as the package converts them (110 574 m per degree of latitude, 111 320 m ×
# cos(latitude) per degree of longitude), at the 1 km square's middle latitude: every trip below
# has its two ends mirrored about it, so it is the mean latitude of the trip ends.
LAT_M = 110_574.0
LAT_MID = LAT0 + 500.0 / LAT_M
LON_M = 111_320.0 * math.cos(math.radians(LAT_MID))


def at(x_m: float, y_m: float) -> tuple[float, float]:
    """The point ``x_m`` metres east and ``y_m`` metres north of the grid's corner."""
    return LON0 + x_m / LON_M, LAT0 + y_m / LAT_M


#: Eight trips on a 1 km square, in metres east and north of the grid's corner: each from
#: ``(x, y)`` to a point ``1000 - y`` metres north, so the mean latitude is the square's middle.
SQUARE = [((100, 100), (900, 900)), ((200, 300), (800, 700)), ((400, 200), (600, 800)),
          ((700, 100), (300, 900)), ((900, 400), (100, 600)), ((100, 600), (900, 400)),
          ((300, 800), (200, 200)), ((760, 250), (760, 750))]  # fmt: skip


def square_trips(mode: str | None = "car") -> list[tuple]:
    """The trips of ``SQUARE`` as rows, all of ``mode``."""
    return [
        (f"s{k}", 0, *at(*o), *at(*d), 25_200, "a", None, mode) for k, (o, d) in enumerate(SQUARE)
    ]


def test_a_grid_moves_every_trip_end_to_its_cell_s_most_used_point() -> None:
    rows = square_trips()
    moved, stats = ms.demand_aggregate_space(rows, grid_m=500, stats=True, quiet=True)
    # Cells of 500 m: (i, j) the i-th eastward and the j-th northward; the cell's width in degrees
    # is fixed at the grid's corner latitude (so a second pass is the identity). A cell's point
    # is the trip-end point its trips use most; here every end is its own point, so it is the
    # first end listed that falls in the cell (origins before destinations, row by row).
    lon_m_corner = 111_320.0 * math.cos(math.radians(LAT0))
    dlon, dlat = 500 / lon_m_corner, 500 / LAT_M
    cell = lambda lon, lat: (math.floor((lon - LON0) / dlon), math.floor((lat - LAT0) / dlat))  # noqa: E731
    # A cell's point is the end its trips use most, the first listed among equals: (900, 400) in
    # cell (1, 0) and (100, 600) in cell (0, 1) are each an origin and a destination.
    uses: dict[tuple[int, int], Counter] = {}
    for r in rows:
        for end in ((r[2], r[3]), (r[4], r[5])):
            uses.setdefault(cell(*end), Counter())[end] += 1
    point = {
        c: max(cnt, key=lambda e, cnt=cnt: (cnt[e], -list(cnt).index(e))) for c, cnt in uses.items()
    }
    assert point[cell(*at(900, 400))] == at(900, 400) and point[cell(*at(100, 600))] == at(100, 600)
    cells = [((0, 0), (1, 1)), ((0, 0), (1, 1)), ((0, 0), (1, 1)), ((1, 0), (0, 1)),
             ((1, 0), (0, 1)), ((0, 1), (1, 0)), ((0, 1), (0, 0)), ((1, 0), (1, 1))]  # fmt: skip
    assert len(point) == 4 and all(cell(*p) == c for c, p in point.items())
    distances = []
    for t, r, (o, d) in zip(moved, rows, cells, strict=True):
        assert (t.origin_lon, t.origin_lat) == point[o], t
        assert (t.destination_lon, t.destination_lat) == point[d], t
        for before, after in (((r[2], r[3]), point[o]), ((r[4], r[5]), point[d])):
            dx = (
                (after[0] - before[0])
                * 111_320.0
                * math.cos(math.radians((before[1] + after[1]) / 2))
            )
            dy = (after[1] - before[1]) * LAT_M
            if before != after:
                distances.append(math.hypot(dx, dy))
    assert [t[6:] for t in moved] == [r[6:] for r in rows], "nothing else about a trip changes"
    # The ends at their cell's point stay (two cells' points are used twice); the others move.
    kept = sum(1 for r in rows for end in ((r[2], r[3]), (r[4], r[5])) if point[cell(*end)] == end)
    assert kept == 6 and stats["moved"] == 16 - kept and stats["kept"] == kept
    assert stats["collapsed"] == 0
    # Eight distinct pairs before; after, (0,0)→(1,1) three times and (1,0)→(0,1) twice.
    assert stats["pairs_before"] == 8 and stats["pairs_after"] == 5
    assert stats["moved_m_mean"] == pytest.approx(sum(distances) / len(distances), abs=1.0)
    assert stats["moved_m_p95"] <= max(distances) + 1.0


def test_trips_in_one_cell_collapse_and_only_the_listed_modes_move() -> None:
    rows = [
        ("c1", 0, *at(100, 100), *at(200, 300), 25_200, "a", None, "car"),
        ("c2", 0, *at(600, 600), *at(900, 900), 25_200, "a", None, "car"),
        ("b", 0, *at(100, 100), *at(900, 900), 25_200, "a", None, "bike"),
        ("n", 0, *at(300, 300), *at(700, 700), 25_200, "a", None, None),
    ]
    moved, stats = ms.demand_aggregate_space(rows, grid_m=500, stats=True, quiet=True)
    # Each car trip's two ends lie in one cell: both land on the cell's point, which is one of
    # them (the first listed), so one end of each moves and the trip collapses.
    assert stats["collapsed"] == 2 and stats["moved"] == 2 and stats["kept"] == 2
    assert moved[0][2:4] == moved[0][4:6] and moved[1][2:4] == moved[1][4:6]
    assert tuple(moved[2]) == rows[2], "a bike trip keeps its ends by default"
    assert tuple(moved[3]) == rows[3], "a trip with no stated mode keeps its ends"
    every, stats = ms.demand_aggregate_space(rows, grid_m=500, modes=None, stats=True, quiet=True)
    # With every trip moving, the cells' points are the ends used most over all four trips:
    # (100, 100) and (900, 900), each an origin and a destination; the bike's ends land there.
    assert every[2][2:4] == every[0][2:4] and every[2][4:6] == every[1][4:6], "the bike moves"
    assert every[3][2:6] != rows[3][2:6], "and so does the trip with no stated mode"
    assert stats["moved"] == 4 and stats["kept"] == 4 and stats["collapsed"] == 2
    bikes = ms.demand_aggregate_space(rows, grid_m=500, modes="bike", quiet=True)
    assert bikes[2] == every[2] and tuple(bikes[0]) == rows[0]


def test_exactly_one_of_zones_and_grid_m_and_a_cell_above_zero() -> None:
    rows = square_trips()
    for kwargs in ({"zones": {"z": (4.0, 52.0)}, "grid_m": 500}, {}):
        with pytest.raises(ValueError, match="exactly one of zones"):
            ms.demand_aggregate_space(rows, **kwargs)
    for bad in (0, -50, math.nan, True):
        with pytest.raises(ValueError, match="grid_m"):
            ms.demand_aggregate_space(rows, grid_m=bad)
    with pytest.raises(ValueError, match="modes"):
        ms.demand_aggregate_space(rows, grid_m=500, modes=("lorry",))


def test_the_zone_form_moves_every_trip_to_the_nearest_zone_point_as_before() -> None:
    zones = {"w": at(250, 250), "e": at(750, 250), "n": at(500, 900)}
    rows = [
        ("p", 0, *at(100, 100), *at(800, 300), 25_200, "a", None),  # 9 fields: no mode
        ("q", 0, *at(600, 200), *at(450, 800), 25_300, "a", 2, "bike"),
        ("r", 0, *zones["w"], *zones["e"], 25_400, "a", None, "car"),
        ("s", 0, *at(200, 300), *at(300, 200), 25_500, "a", None, "car"),
    ]
    out = ms.demand_aggregate_space(rows, zones, quiet=True)  # the positional call of before
    assert isinstance(out, list)
    nearest = [("w", "e"), ("e", "n"), ("w", "e"), ("w", "w")]
    for t, r, (o, d) in zip(out, rows, nearest, strict=True):
        assert t == ms.Trip(*r[:2], *zones[o], *zones[d], *r[6:], *([None] * (10 - len(r))))
    out_cars, stats = ms.demand_aggregate_space(rows, zones, modes="car", stats=True, quiet=True)
    assert out_cars[2:] == out[2:] and tuple(out_cars[1]) == rows[1]
    assert out_cars[0] == ms.Trip(*rows[0], None), "no stated mode: kept with modes='car'"
    assert stats["moved"] == 2 and stats["kept"] == 2 and stats["collapsed"] == 1
    assert stats["pairs_before"] == 2 and stats["pairs_after"] == 2


def test_the_original_ends_come_back_for_the_moved_trips_only() -> None:
    zones = {"w": at(250, 250), "e": at(750, 250)}
    rows = [
        ("moved", 1, *at(100, 100), *at(800, 300), 25_200, "a", None, "car"),
        ("there", 0, *zones["w"], *zones["e"], 25_200, "a", None, "car"),
        ("bike", 0, *at(100, 100), *at(800, 300), 25_200, "a", None, "bike"),
    ]
    _, stats = ms.demand_aggregate_space(rows, zones, modes="car", stats=True, quiet=True)
    assert stats["original"] == [("moved", 1, *rows[0][2:6])]
    rows = square_trips() + [("bike", 0, *at(100, 100), *at(900, 900), 25_200, "a", None, "bike")]
    moved, stats = ms.demand_aggregate_space(rows, grid_m=500, stats=True, quiet=True)
    # The car trips with at least one end moved, in order (s0, s4 and s5 have both ends at their
    # cells' points, so they are not listed); never the bike.
    expected = [
        (r[0], r[1], *r[2:6])
        for r, m in zip(rows[:8], moved[:8], strict=True)
        if tuple(m)[2:6] != r[2:6]
    ]
    assert stats["original"] == expected and len(expected) == 5


def test_a_run_takes_the_demand_on_a_grid_and_the_original_ends_join_its_trips() -> None:
    net = ms.examples.manhattan_grid(n=6, block_metres=150.0, signals=False)
    drawn = ms.examples.trips_random(net, 300, seed=3, min_m=300.0, max_m=900.0, spread_s=600)
    rows = [(*r[:8], None, "car") for r in drawn]
    moved, stats = ms.demand_aggregate_space(rows, grid_m=250, stats=True, quiet=True)
    assert stats["moved"] > 0 and stats["pairs_after"] < stats["pairs_before"]
    run = ms.Scenario.from_parts(net, moved, classes=CAR).run("grid-demand", quiet=True)
    t = run.trip_results()
    assert run.completion["total_trips"] == len(rows) == len(t["trip"])
    assert run.completion["completed"] == len(rows), "a trip of two ends on one point too"
    # The run's trips start and end where the helper put them; ``original`` says where they began.
    given = {(r.traveller_id, r.trip_seq): r for r in moved}
    before = {(r[0], r[1]): r[2:6] for r in rows}
    original = {(k, s): tuple(ends) for k, s, *ends in stats["original"]}
    assert len(original) == len(rows) - sum(
        r[2:6] == m[2:6] for r, m in zip(rows, moved, strict=True)
    )
    for k, s, olon, olat in zip(
        t["traveller_id"], t["trip_seq"], t["origin_lon"], t["origin_lat"], strict=True
    ):
        trip = given[(k, int(s))]
        assert (olon, olat) == pytest.approx((trip.origin_lon, trip.origin_lat), abs=1e-9)
        assert original.get((k, int(s)), before[(k, int(s))]) == before[(k, int(s))]


def test_the_grid_applied_twice_moves_nothing_more():
    """The cell's degrees are fixed at the grid's corner, so a second pass is the identity."""
    rows = []
    for i in range(40):
        lon, lat = 4.80 + (i % 8) * 0.0007, 52.00 + (i // 8) * 0.0005
        rows.append((f"p{i}", 0, lon, lat, lon + 0.003, lat + 0.002, 7 * 3600 + i, "c", 1, "car"))
    once, stats1 = ms.demand_aggregate_space(rows, grid_m=250, stats=True)
    twice, stats2 = ms.demand_aggregate_space(once, grid_m=250, stats=True)
    assert stats1["moved"] > 0
    assert stats2["moved"] == 0 and stats2["kept"] == 80
    assert [tuple(r) for r in twice] == [tuple(r) for r in once]
