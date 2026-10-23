"""Every parameter a run uses is in its fingerprint.

Two runs given different inputs must not share a fingerprint, or the fingerprint's promise (the
same fingerprint, the same results) breaks. On a multimodal scenario of the toy network (mode
choice, classes, transit, parkings, money, iterations), each parameter of the groups that scenario
uses is changed by name, alone, and the fingerprint must change with it. The network's own
parameters are not changed here: they act through the network, whose every link value is hashed.
"""

from __future__ import annotations

import openmobisim as ms

#: The groups this scenario uses, by the argument that overrides them (the route method of an
#: iterating run is ``shortest``, which has no options; ``nested_logit`` and ``free_flow`` are
#: other choices, not this run's).
GROUPS = {
    "loading": "loading_options",
    "modes": "mode_options",
    "transit": "transit_options",
    "parking": "parking_options",
    "prices": "price_options",
    "equilibration.msa": "equilibration_options",
    "route_update.best_response": "route_update_options",
    "choice_model.logit": "choice_options",
    "equilibration.frank_wolfe": "equilibration_options",
}

#: What only the static assignment reads (S294): changed on a car-only run at ``flow_level=1``.
STATIC = {"bpr_alpha", "bpr_beta", "period_s"}


def _scenario(**options: object) -> ms.Scenario:
    net = ms.examples.toy_network()
    rows = ms.examples.trips_random(net, 60, seed=3, min_m=100.0, max_m=2000.0, spread_s=1800)
    classes = ms.examples.traveller_classes()
    rows = ms.demand_assign_classes(rows, classes, seed=1)
    return ms.Scenario.from_parts(
        net,
        rows,
        classes=classes,
        transit=ms.examples.toy_network_transit(),
        parkings=ms.examples.toy_network_parkings(),
        flow_level=4,
        **options,
    )


def _static(**options: object) -> ms.Scenario:
    """A car-only run of the toy network at flow level 1, solved by Frank-Wolfe."""
    net = ms.examples.toy_network()
    rows = ms.examples.trips_random(net, 60, seed=3, min_m=100.0, max_m=2000.0, spread_s=1800)
    return ms.Scenario.from_parts(net, rows, flow_level=1, equilibration="frank_wolfe", **options)


def _changed(value: float) -> float:
    """Another valid value: a whole number raised by one (counts, seconds), anything else scaled."""
    if value == int(value):
        return value + 1
    return value * 1.5 + 0.1 if value >= 0 else value * 1.5 - 0.1


def test_every_parameter_the_run_uses_changes_its_fingerprint() -> None:
    base = _scenario()._fingerprint()
    static_base = _static()._fingerprint()
    table = ms.parameters()
    missed = []
    for group, name, value in zip(table["group"], table["name"], table["value"], strict=True):
        argument = GROUPS.get(group)
        if argument is None:
            continue
        # A flag's listed value may be another level's default (``reroute``: 0 at level 2, 1
        # here), so both of its values are tried: one of them must change the fingerprint.
        flag = name in (
            "priority",
            "reroute",
            "node_inflow",
            "origin_inflow",
            "fallback_car",
            "biased",
        )
        values = (0.0, 1.0) if flag else (_changed(float(value)),)
        static = group == "equilibration.frank_wolfe" or name in STATIC
        make, against = (_static, static_base) if static else (_scenario, base)
        got = set()
        for new in values:
            try:
                got.add(make(**{argument: {name: new}})._fingerprint())
            except ValueError:
                continue  # a value the option refuses: not a run this test can make
        if got == {against}:
            missed.append(f"{group} {name}")
    assert not missed, f"parameters not in the fingerprint: {missed}"
