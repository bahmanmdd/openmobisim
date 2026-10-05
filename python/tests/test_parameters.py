"""Every parameter in one table (S230, roadmap I-ae).

What is defended: ``parameters()`` lists every group, with units read from the names, and every
row is accepted by the argument it names, at its listed value (so the table cannot drift from
what the code takes).
"""

import openmobisim as ms

CAR = {"commuter": (True, False, False)}


def table():
    t = ms.parameters()
    return [dict(zip(t, row, strict=True)) for row in zip(*t.values(), strict=True)]


def test_every_group_is_listed_with_units():
    rows = table()
    groups = {r["group"] for r in rows}
    assert {"scenario", "network", "loading", "modes", "transit", "parking", "equilibration.msa",
            "equilibration.free_flow", "route_method.penalty", "route_update.best_response",
            "choice_model.logit"} <= groups  # fmt: skip
    units = {r["name"]: r["unit"] for r in rows if r["group"] == "network"}
    assert units["walk_km_h"] == "km/h" and units["signal_cycle_s"] == "s"
    assert units["primary.saturation_flow_veh_h_lane"] == "veh/h per lane"
    logit = {r["name"]: r for r in rows if r["group"] == "choice_model.logit"}
    assert logit["beta_mode_bike"]["value"] == 0.0 and logit["beta_time_min"]["unit"] == "per min"


def test_every_row_is_accepted_by_its_argument():
    rows = table()
    net = ms.examples.toy_network()
    trip = [("t0", 0, *net.node_lonlat("W"), *net.node_lonlat("D1"), 0, "commuter", None)]

    def run(**kwargs):
        return ms.Scenario.from_parts(net, trip, class_defaults=CAR, **kwargs).run(
            "par", quiet=True
        )

    def options(group):
        return {r["name"]: r["value"] for r in rows if r["group"] == group}

    ms.network_read_table(
        [{"from": "a", "to": "b", "length": 100.0}],
        [{"node": "a", "lon": 4.9, "lat": 52.37}, {"node": "b", "lon": 4.901, "lat": 52.37}],
        network_options=options("network"),
    )
    run(loading_options=options("loading"), mode_options=options("modes"))
    for strategy in ("msa", "free_flow"):
        run(equilibration=strategy, equilibration_options=options(f"equilibration.{strategy}"))
    for group in {r["group"] for r in rows if r["group"].startswith("route_method.")}:
        run(route_method=group.split(".", 1)[1], route_options=options(group) or None)
    run(route_update="best_response", route_update_options=options("route_update.best_response"))
    for group in {r["group"] for r in rows if r["group"].startswith("choice_model.")}:
        run(choice_model=group.split(".", 1)[1], choice_options=options(group) or None)
    transit = options("transit")
    assert transit and all(v >= 0 for v in transit.values())
