"""The gap per origin, destination and departure bin (`Run.gap_pairs`, `Run.gap_pairs_relative`).

On a congested grid whose trips start and end at four zones' own points (`points_per_zone=1`), so
that the trips of a pair share their points: every trip measured is in one group, the groups'
people are the demand's, a group's best is what its best-off trip paid, and a bin as wide as the
window gives one group per pair. Where a group's trips are the same trip (one second of
departure), its best is no lower than the best route each was offered, so the gap read per group
is at most the run's `gap_route`; with departures spread over a bin it need not be, and is not
asserted. A run of one loading has no gap to read; a run read back from its folder neither.
"""

from __future__ import annotations

import numpy as np
import openmobisim as ms
import pytest

CAR = {"default": (True, False, False)}
CORNERS = {"a": "n_000_000", "b": "n_005_005", "c": "n_000_005", "d": "n_005_000"}
CELLS = [("a", "b", 1200), ("b", "a", 600), ("c", "d", 900), ("d", "c", 300), ("a", "c", 300)]
WINDOW = (0, 7200)


def demand(net, same_second: bool = False) -> list[tuple]:
    zones = {zone: net.node_lonlat(node) for zone, node in CORNERS.items()}
    od = [{"origin_zone": o, "destination_zone": d, "trips": n} for o, d, n in CELLS]
    rows = ms.demand_from_od(od, zones, start_s=0, end_s=1800, points_per_zone=1, seed=1)
    if same_second:  # the test's own demand: every trip of a pair and 5-minute bin leaves together
        rows = [(*r[:6], r[6] // 300 * 300, *r[7:]) for r in rows]
    return rows


def run_on(rows, net, output_dir, **kwargs) -> ms.Run:
    settings = {
        "classes": CAR,
        "window": WINDOW,
        "flow_level": 2,
        "choice_model": "logit",
        "route_method": "penalty",
        "master_seed": 3,
        "equilibration_options": {"iterations": 6, "gap_tolerance": 0},
    }
    settings.update(kwargs)
    return ms.Scenario.from_parts(net, rows, **settings).run(
        "gap-pairs", output_dir=str(output_dir), overwrite=True, quiet=True
    )


@pytest.fixture(scope="module")
def grid():
    return ms.examples.manhattan_grid(n=6, block_metres=200.0, signals=False)


@pytest.fixture(scope="module")
def spread(grid, tmp_path_factory) -> tuple[ms.Run, list[tuple], object]:
    folder = tmp_path_factory.mktemp("pairs") / "spread"
    rows = demand(grid)
    return run_on(rows, grid, folder), rows, folder


def test_every_trip_is_in_one_group_and_the_people_are_the_demands(spread) -> None:
    run, rows, _ = spread
    assert run.completion["truncated"] == 0, "every trip arrives: the inequality below needs it"
    pairs = run.gap_pairs()
    assert list(pairs) == [
        "origin_lon", "origin_lat", "destination_lon", "destination_lat", "bin_start_s",
        "trips", "people", "g_best_min", "g_mean_min", "excess_min", "gap", "unfinished",
    ]  # fmt: skip
    gap = run.gap_trips()
    assert pairs["trips"].sum() == len(gap["trip"]) == len(rows), "every car trip, in one group"
    assert pairs["people"].sum() == pytest.approx(len(rows)), "each trip one person"
    assert pairs["unfinished"].sum() == 0
    starts = pairs["bin_start_s"]
    assert set(starts.tolist()) <= {WINDOW[0] + 300 * k for k in range(6)}, "5-minute bins"
    ends = {(round(r[2], 6), round(r[3], 6), round(r[4], 6), round(r[5], 6)) for r in rows}
    found = zip(
        pairs["origin_lon"], pairs["origin_lat"], pairs["destination_lon"],
        pairs["destination_lat"], strict=True,
    )  # fmt: skip
    assert {tuple(round(float(x), 6) for x in p) for p in found} == ends


def test_a_groups_best_is_what_its_best_off_trip_paid(spread) -> None:
    run, rows, _ = spread
    pairs, gap, trips = run.gap_pairs(), run.gap_trips(), run.trip_results()
    t = gap["trip"]
    key = {
        (
            round(float(pairs["origin_lon"][i]), 6),
            round(float(pairs["origin_lat"][i]), 6),
            round(float(pairs["destination_lon"][i]), 6),
            round(float(pairs["destination_lat"][i]), 6),
            int(pairs["bin_start_s"][i]),
        ): i
        for i in range(pairs.row_count)
    }
    members: dict[int, list[float]] = {}
    for k, g in zip(t, gap["g_chosen_min"], strict=True):
        start = WINDOW[0] + (int(trips["departure_s"][k]) - WINDOW[0]) // 300 * 300
        i = key[
            (
                round(float(trips["origin_lon"][k]), 6),
                round(float(trips["origin_lat"][k]), 6),
                round(float(trips["destination_lon"][k]), 6),
                round(float(trips["destination_lat"][k]), 6),
                start,
            )
        ]
        assert pairs["g_best_min"][i] <= g, "no trip of a group paid less than its best"
        members.setdefault(i, []).append(float(g))
    for i, gs in members.items():
        assert pairs["g_best_min"][i] == min(gs)
        assert pairs["g_mean_min"][i] == pytest.approx(np.mean(gs))
        assert pairs["excess_min"][i] == pytest.approx(sum(gs) - len(gs) * min(gs))
    reference = (pairs["people"] * pairs["g_best_min"]).sum()
    assert run.gap_pairs_relative() == pytest.approx(pairs["excess_min"].sum() / reference)
    assert run.gap_pairs_relative() > 0, "a congested grid after six loadings is not at rest"


def test_trips_leaving_together_read_at_most_the_route_gap(grid, spread, tmp_path) -> None:
    # In principle: the trips of a group that leave at one second with one point each end are the
    # same trip, offered the same routes at the same costs, so each one's best offered route is
    # the same, and no trip of the group paid less than it. The group's best is then at least each
    # trip's best offered: a smaller excess over a larger reference. A bin of one second groups
    # only such trips, whatever the demand; 5-minute bins do too when the demand leaves in them.
    run, _, _ = spread
    route_gap = float(run.convergence()["gap_route"][-1])
    assert 0 < run.gap_pairs_relative(1) <= route_gap + 1e-12
    together = run_on(demand(grid, same_second=True), grid, tmp_path / "together")
    assert together.completion["truncated"] == 0
    route_gap = float(together.convergence()["gap_route"][-1])
    assert together.gap_pairs_relative(300) <= route_gap + 1e-12
    assert together.gap_pairs(1).row_count == together.gap_pairs(300).row_count
    # A bin as wide as the window: one group per pair, whatever the departures.
    assert run.gap_pairs(WINDOW[1] - WINDOW[0]).row_count == len(CELLS)
    assert together.gap_pairs(WINDOW[1] - WINDOW[0]).row_count == len(CELLS)


def test_trips_choosing_their_mode_are_grouped_by_the_alternative_they_took(grid, tmp_path):
    rows = demand(grid)
    run = run_on(
        rows, grid, tmp_path / "modes", classes={"default": (True, True, False)},
        modes=("car", "bike", "walk"),
    )  # fmt: skip
    pairs, gap = run.gap_pairs(), run.gap_trips()
    assert pairs["trips"].sum() == len(gap["trip"]) == len(rows)
    assert np.isfinite(pairs["g_best_min"]).all() and (pairs["excess_min"] >= 0).all()


def test_no_gap_to_read_without_iterations_or_from_the_files(grid, spread) -> None:
    run, rows, folder = spread
    single = run_on(
        rows, grid, folder.parent / "single", equilibration="free_flow", equilibration_options={}
    )
    assert single.gap_trips() is None and single.gap_pairs() is None
    assert np.isnan(single.gap_pairs_relative())
    with pytest.raises(RuntimeError, match="gap_trips"):
        ms.Run.open(folder).gap_pairs()
    for bad in (0, -300, 2.5):
        with pytest.raises(ValueError, match="departure_bin_s"):
            run.gap_pairs(bad)
