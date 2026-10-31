"""The results of a run with a docked bike service: its events, its trips, its docks, its KPIs.

``bike_share_events.parquet`` (the logbook, one row per event in time order),
``bike_share_trips.parquet`` (one row per trip that chose a shared bike) and
``bike_share_docks.parquet`` (one row per dock and time bin) are written only by a run with a
service; the three reconcile with each other and with ``kpis.parquet``'s rows. A run without a
service writes none of them. The network: the 10 x 10 grid of ``test_services_in_runs``, two
bikes a dock, so that docks run empty, travellers go on to other docks and some find no bike
at all.
"""

from __future__ import annotations

from collections import Counter
from pathlib import Path

import numpy as np
import openmobisim as ms
import pytest
from test_services_in_runs import docks, grid, scenario, trips

pq = pytest.importorskip("pyarrow.parquet")

MSA = {"equilibration": "msa", "equilibration_options": {"iterations": 4, "gap_tolerance": 0}}


@pytest.fixture(scope="module")
def run(tmp_path_factory: pytest.TempPathFactory) -> ms.Run:
    net = grid()
    rows = trips(net, 12, (0, 0), (9, 9), "a") + trips(net, 6, (9, 9), (0, 1), "b")
    # The classes list ordinary modes only: a service's modes are offered all the same.
    classes = {"a": {"modes": ["walk"]}, "b": {"modes": ["walk"]}}
    s = scenario(
        net,
        rows,
        services=[ms.service_bike_docked(docks(net, bikes=2))],
        classes=classes,
        choice_model="logit",
        **MSA,
    )
    return s.run("share-results", output_dir=str(tmp_path_factory.mktemp("r")), quiet=True)


def _kpi(run: ms.Run, metric: str) -> dict[int, float]:
    k = pq.read_table(run._summary.kpis_path).to_pylist()
    return {r["iteration"]: r["value"] for r in k if r["metric"] == metric}


def test_the_files_hold_the_tables(run: ms.Run) -> None:
    for columns, table in (
        (run.bike_share_events(), run.bike_share_events_table()),
        (run.bike_share_trips(), run.bike_share_trips_table()),
        (run.bike_share_docks(), run.bike_share_docks_table()),
    ):
        t = pq.read_table(table.path)
        assert t.column_names[1:] == list(columns)
        assert t.num_rows == columns.row_count > 0
    # The docks found empty or full, by trip and by dock, in plain words (S305).
    trips = pq.read_table(run.bike_share_trips_table().path).column_names
    docks = pq.read_table(run.bike_share_docks_table().path).column_names
    assert {"docks_found_empty", "docks_found_full"} <= set(trips)
    went_on, gave_up, full = (
        "people_found_empty_went_on",
        "people_found_empty_gave_up",
        "people_found_full_went_on",
    )
    assert docks.index(went_on) + 1 == docks.index(gave_up) == docks.index(full) - 1
    assert not [c for c in trips + docks if "failed" in c or c.endswith("_no_bike")]


def test_the_trips_docks_and_events_reconcile(run: ms.Run) -> None:
    ev, tr, dk = run.bike_share_events(), run.bike_share_trips(), run.bike_share_docks()
    w = np.asarray(tr["travellers"], dtype=float)
    outcome: Counter[str] = Counter()
    for r, x in zip(tr["outcome"], w, strict=True):
        outcome[r] += x
    elsewhere = _elsewhere(tr)
    assert w[elsewhere].sum() > 0 and outcome["cancelled"] > 0
    people: Counter[str] = Counter()
    for e, x in zip(ev["event"], ev["travellers"], strict=True):
        people[e] += float(x)
    # Bikes taken and returned: the events, the docks and the trips agree.
    assert people["took_bike"] == pytest.approx(dk["bikes_taken"].sum())
    assert people["took_bike"] == pytest.approx(w[np.isfinite(tr["pickup_s"])].sum())
    returned = people["returned_bike"] + people["left_bike_over_capacity"]
    assert returned == pytest.approx(dk["bikes_returned"].sum())
    # Empty docks: went on to another, or found no bike at any dock tried and walked.
    went_on = people["walking_to_next_dock"]
    assert went_on == pytest.approx(dk["people_found_empty_went_on"].sum())
    gave_up = people["gave_up_walking_to_destination"]
    assert gave_up == pytest.approx(dk["people_found_empty_gave_up"].sum())
    assert gave_up == pytest.approx(outcome["cancelled"])
    assert people["found_no_bike"] == pytest.approx(np.sum(w * tr["docks_found_empty"]))
    assert people["arrived_for_bike"] == pytest.approx(np.sum(w * tr["docks_tried_for_bike"]))
    assert people["riding_to_next_dock"] == pytest.approx(dk["people_found_full_went_on"].sum())
    sent = people["walking_to_next_dock"] + people["riding_to_next_dock"]
    assert sent == pytest.approx(dk["people_sent_here"].sum())
    # Every trip that arrived reached its destination once.
    ended = people["reached_destination"] + people["reached_transit_stop"]
    assert ended == pytest.approx(w[np.isfinite(tr["leg_end_s"])].sum())
    # No operator in a run.
    assert not dk["bikes_brought_by_van"].any() and not dk["bikes_taken_by_van"].any()
    # A ride that took its bike at another dock found the planned one empty first.
    for i in np.flatnonzero(elsewhere):
        assert tr["docks_found_empty"][i] > 0
    # The classes list walk alone, and still took shared bikes.
    assert set(tr["mode"]) == {"bike_share"}


def _elsewhere(tr: object) -> np.ndarray:
    """The rides that took their bike at a dock other than the planned one."""
    return np.array(
        [
            o == "completed" and p is not None and p != q
            for o, p, q in zip(
                tr["outcome"], tr["pickup_dock"], tr["planned_pickup_dock"], strict=True
            )
        ],
        dtype=bool,
    )


def test_a_travellers_events_follow_in_time(run: ms.Run) -> None:
    ev = run.bike_share_events()
    assert list(ev["time_s"]) == sorted(ev["time_s"])
    tr = run.bike_share_trips()
    i = int(np.flatnonzero(_elsewhere(tr))[0])
    who = tr["traveller_id"][i]
    mine = [e for e, t in zip(ev["event"], ev["traveller_id"], strict=True) if t == who]
    assert mine[:4] == [
        "arrived_for_bike",
        "found_no_bike",
        "walking_to_next_dock",
        "arrived_for_bike",
    ]
    assert "took_bike" in mine and mine[-1] == "reached_destination"
    s = int(ev["time_s"][0])
    assert ev["time"][0] == f"{s // 3600:02d}:{s % 3600 // 60:02d}:{s % 60:02d}"


def test_the_docks_keep_their_bikes(run: ms.Run) -> None:
    dk = run.bike_share_docks()
    assert (np.asarray(dk["bikes_min"]) >= 0).all()
    # Each bin starts where the one before ended.
    for d in set(dk["dock"]):
        rows = [i for i, x in enumerate(dk["dock"]) if x == d]
        for a, b in zip(rows, rows[1:], strict=False):
            assert dk["bikes_at_start"][b] == dk["bikes_at_end"][a]
            assert dk["bin_start_s"][b] == dk["bin_end_s"][a]
    # Bikes are conserved: docked at the end plus the trips still riding is the fleet.
    fleet = int(np.sum(run.bike_share_dock_places()["bikes_start"]))
    tr = run.bike_share_trips()
    riding = sum(
        x
        for x, r, p in zip(tr["travellers"], tr["outcome"], tr["pickup_s"], strict=True)
        if r == "unfinished" and p == p
    )
    last = np.asarray(dk["bin_end_s"]) == np.max(dk["bin_end_s"])
    assert np.sum(np.asarray(dk["bikes_at_end"])[last]) + riding == fleet == 10


DOCK_ROWS = [
    "requests",
    "rides",
    "no_bike",
    "unfinished",
    "extra_walk_at_pickup_mean_s",
    "extra_walk_at_return_mean_s",
    "assumed_extra_walk_at_pickup_s",
    "assumed_extra_walk_at_return_s",
    "redirected_share",
]


def test_the_kpi_rows_are_the_trips(run: ms.Run) -> None:
    tr = run.bike_share_trips()
    w = np.asarray(tr["travellers"], dtype=float)
    k = pq.read_table(run._summary.kpis_path).to_pylist()
    # The service's rows (under its name, here the mode's too): one row set, for the last
    # iteration, in this order.
    mine = [r for r in k if r["mode"] == "bike_share" and r["metric"] in DOCK_ROWS]
    assert [r["metric"] for r in mine] == DOCK_ROWS
    assert len({r["iteration"] for r in mine}) == 1, "one row set, for the last iteration (S303)"
    rows = {r["metric"]: r["value"] for r in mine}
    outcome = np.asarray(tr["outcome"])
    assert rows["requests"] == pytest.approx(w.sum())
    assert rows["rides"] == pytest.approx(w[outcome == "completed"].sum())
    assert rows["no_bike"] == pytest.approx(w[outcome == "cancelled"].sum())
    assert rows["requests"] == pytest.approx(rows["rides"] + rows["no_bike"] + rows["unfinished"])
    ride = outcome == "completed"
    for row, column in (
        ("extra_walk_at_pickup_mean_s", "extra_walk_at_pickup_s"),
        ("extra_walk_at_return_mean_s", "extra_walk_at_return_s"),
    ):
        x = np.asarray(tr[column], dtype=float)[ride]
        assert rows[row] == pytest.approx(np.sum(w[ride] * x) / w[ride].sum()), row
    assert np.isnan(rows["assumed_extra_walk_at_pickup_s"])
    assert np.isnan(rows["assumed_extra_walk_at_return_s"])
    tried = np.asarray(tr["docks_tried_for_bike"]) + np.asarray(tr["docks_tried_for_slot"])
    redirected = ride & (tried > 2)
    assert rows["redirected_share"] == pytest.approx(w[redirected].sum() / w[ride].sum())
    gone = {"service_trips", "served", "failed", "truncated", "mean_extra_s", "mismatch_mean_s"}
    assert not gone & {r["metric"] for r in k}
    # The trip rows: the planned trip, each shared-bike trip with the service day's outcome.
    t = run.trip_results()
    share = np.asarray(t["mode"], dtype=object) == "bike_share"
    service = np.asarray(t["service_outcome"])
    assert (service[share] != "").all() and (service[~share] == "").all()
    assert (service[share] == "cancelled").sum() == (outcome == "cancelled").sum()
    # Under the service's mode, the rows of its trips' choice only: how they went is the
    # service day's (the rows above), not the planned trips' (S305).
    mode = {r["metric"] for r in k if r["mode"] == "bike_share" and r["metric"] not in DOCK_ROWS}
    planned = {"completed_trips", "completed_people", "completion_rate", "truncated_trips"}
    assert not (planned | {"total_travel_time_s"}) & mode, mode
    assert {"trips", "people", "mode_share", "no_feasible_path_trips"} <= mode


def test_the_log_can_be_turned_off(tmp_path: Path) -> None:
    net = grid()
    rows = trips(net, 12, (0, 0), (9, 9), "a")

    def go(options: dict[str, float], name: str) -> ms.Run:
        bikes = ms.service_bike_docked(docks(net, bikes=2), options=options)
        s = scenario(net, rows, services=[bikes], choice_model="deterministic")
        return s.run(name, output_dir=str(tmp_path / name), quiet=True)

    on, off = go({}, "log"), go({"log_events": 0}, "nolog")
    assert on.bike_share_events().row_count > 0 and off.bike_share_events().row_count == 0
    # The same run otherwise: its identity, its trips.
    assert on.fingerprint == off.fingerprint
    assert list(on.bike_share_trips()["outcome"]) == list(off.bike_share_trips()["outcome"])
    with pytest.raises(ValueError, match="log_events"):
        go({"log_events": 2}, "bad")


def test_a_run_without_a_service_writes_none_of_it(tmp_path: Path) -> None:
    net = grid()
    rows = trips(net, 3, (0, 0), (1, 0))
    out = tmp_path / "none"
    r = scenario(net, rows, modes=["walk"]).run("none", output_dir=str(out), quiet=True)
    assert r.bike_share_events() is None and r.bike_share_trips() is None
    assert r.bike_share_docks() is None and r.bike_share_dock_places() is None
    assert not list(out.glob("bike_share_*.parquet"))
    k = pq.read_table(out / "kpis.parquet").to_pylist()
    assert not [r for r in k if r["mode"] == "bike_share"]
    # Without a service every mode keeps the rows of how its trips went.
    walk = {r["metric"] for r in k if r["mode"] == "walk"}
    planned = {"completed_trips", "completed_people", "completion_rate", "truncated_trips"}
    assert planned | {"trips", "people", "total_travel_time_s"} <= walk
    # A run at flow level 0 reads back from its folder (its manifest has no flow step).
    opened = ms.Run.open(out)
    assert opened.bike_share_trips_table() is None


def test_a_class_with_has_bike_share_0_never_takes_one(tmp_path: Path) -> None:
    net = grid()
    rows = trips(net, 3, (0, 0), (9, 9), "no")
    s = scenario(
        net,
        rows,
        services=[ms.service_bike_docked(docks(net))],
        classes={"no": {"modes": ["walk"], "has_bike_share": 0}},
        choice_model="deterministic",
    )
    r = s.run("no-share", output_dir=str(tmp_path / "no"), quiet=True)
    assert r.bike_share_trips().row_count == 0


def test_the_figures_draw(run: ms.Run, tmp_path: Path) -> None:
    pytest.importorskip("matplotlib")
    from openmobisim import viz

    for theme in ("paper", "night"):
        viz.map_hubs(run, "08:05", theme=theme, path=str(tmp_path / f"hubs_{theme}.png"))
        viz.chart_service(run, "bike_share", theme=theme, path=str(tmp_path / f"day_{theme}.png"))
        viz.chart_mode_share(run, theme=theme, path=str(tmp_path / f"modes_{theme}.png"))
        for name in ("hubs", "day", "modes"):
            assert (tmp_path / f"{name}_{theme}.png").stat().st_size > 0
    with pytest.raises(ValueError, match="outside"):
        viz.map_hubs(run, "23:00")
    with pytest.raises(ValueError, match="no service"):
        viz.chart_service(run, "taxi")
