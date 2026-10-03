"""The loading's rules (S213): the gridlock report, priority at merges, en-route rerouting.

On a small grid loaded far beyond its capacity: the report finds what stands and checks the
loading's guarantee (no front waits for room that is there); the rules are off unless asked for
and then enter the run's fingerprint; vehicles that re-route are recorded, and their realised
routes are whole, continuous, and turn where the record says.
"""

from __future__ import annotations

import numpy as np
import openmobisim as ms
import pytest

CAR = {"commuter": (True, False, False)}


def jammed(**kwargs: object) -> ms.Run:
    net = ms.examples.manhattan_grid(n=6, block_metres=150.0, signals=False)
    rows = ms.examples.trips_random(net, 4000, seed=3, min_m=300.0, max_m=900.0, spread_s=600)
    return ms.Scenario.from_parts(
        net, rows, class_defaults=CAR, window_hours=3, equilibration="none", **kwargs
    ).run("rules")


def test_the_gridlock_report_finds_what_stands_and_checks_the_guarantee() -> None:
    run = jammed()
    g = run.report_gridlock()
    assert g is not None
    assert set(g) >= {"vehicles_on_network", "vehicles_outside", "links_waiting", "loops",
                      "loop_count", "loop_links", "room_waits_with_room"}  # fmt: skip
    assert g["room_waits_with_room"] == 0, "traffic stops only at jam density"
    assert g["loop_count"] == len(g["loops"]) and g["loop_links"] == sum(map(len, g["loops"]))
    truncated = run.completion["truncated"]
    assert g["vehicles_on_network"] + g["vehicles_outside"] == truncated
    if g["loops"]:
        net = run.network
        frm, to = np.asarray(net.link_from()), np.asarray(net.link_to())
        for loop in g["loops"]:
            # Each link waits on the next: they join end to end into a closed walk.
            for a, b in zip(loop, loop[1:] + loop[:1], strict=True):
                assert to[a] == frm[b]
    level0 = ms.Scenario.from_parts(
        run.network, ms.examples.trips_random(run.network, 50, seed=1), class_defaults=CAR,
        flow_level=0, equilibration="none",
    ).run("level0")  # fmt: skip
    assert level0.report_gridlock() is None


def test_rules_are_off_unless_asked_and_then_in_the_fingerprint() -> None:
    plain, empty = jammed(), jammed(loading_options={})
    assert plain.fingerprint == empty.fingerprint
    assert plain.route_changes()["trip"].size == 0
    with_priority = jammed(loading_options={"priority": 1})
    assert with_priority.fingerprint != plain.fingerprint
    rerouting = jammed(loading_options={"reroute": 1, "reroute_after_s": 60})
    assert rerouting.fingerprint not in (plain.fingerprint, with_priority.fingerprint)
    for bad, match in (
        ({"prority": 1}, "the options are: priority, reroute"),
        ({"priority": 2}, "must be 0 or 1"),
        ({"reroute_min_gain": 1.0}, "below 1"),
        ({"reroute_max": 2.5}, "whole number"),
        ({"reroute_after_s": -1}, "at least 0"),
    ):
        with pytest.raises(ValueError, match=match):
            jammed(loading_options=bad)


def test_reroutes_are_recorded_and_realised_routes_turn_where_recorded() -> None:
    run = jammed(loading_options={"reroute": 1, "reroute_after_s": 60})
    ch, real = run.route_changes(), run.route_realised()
    n = ch["trip"].size
    assert n > 0, "vehicles re-route on a jammed grid"
    assert (np.asarray(ch["next_taken"]) != np.asarray(ch["next_planned"])).all()
    assert (np.diff(np.asarray(ch["second"])) >= 0).all(), "in the order they happened"
    assert len(ch["traveller_id"]) == n and ch["trip_seq"].size == n
    net = run.network
    frm, to = np.asarray(net.link_from()), np.asarray(net.link_to())
    assert len(real["links"]) == real["trip"].size > 0
    assert set(real["trip"].tolist()) <= set(ch["trip"].tolist())
    for trip, links in zip(real["trip"].tolist(), real["links"], strict=True):
        links = links.tolist()
        assert all(to[a] == frm[b] for a, b in zip(links, links[1:], strict=False)), "continuous"
        # Its last reroute: the link it re-routed at is followed by the link it took.
        rows = [i for i, t in enumerate(ch["trip"].tolist()) if t == trip]
        last = rows[-1]
        at, taken = int(ch["link"][last]), int(ch["next_taken"][last])
        assert taken in links and links[links.index(taken) - 1] == at
    again = jammed(loading_options={"reroute": 1, "reroute_after_s": 60})
    assert np.array_equal(again.route_changes()["trip"], ch["trip"]), "deterministic"
    assert run.convergence()["reroutes"].tolist() == [n]


def test_priority_and_rerouting_keep_the_guarantee() -> None:
    for opts in (
        {"priority": 1},
        {"reroute": 1, "reroute_after_s": 60},
        {"priority": 1, "reroute": 1},
    ):
        g = jammed(loading_options=opts).report_gridlock()
        assert g["room_waits_with_room"] == 0, opts
