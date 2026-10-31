"""Every parameter in one table.

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
    assert {"scenario", "network", "loading", "modes", "transit", "parking", "prices",
            "equilibration.msa", "equilibration.free_flow", "route_method.link_penalty",
            "route_update.least_cost", "choice_model.logit"} <= groups  # fmt: skip
    units = {r["name"]: r["unit"] for r in rows if r["group"] == "network"}
    assert units["walk_km_h"] == "km/h" and units["signal_cycle_s"] == "s"
    assert units["primary.saturation_flow_veh_h_lane"] == "veh/h per lane"
    logit = {r["name"]: r for r in rows if r["group"] == "choice_model.logit"}
    assert logit["beta_mode_bike"]["value"] == 0.0 and logit["beta_time_min"]["unit"] == "per min"
    # Money (S248): prices in euros, weighed by beta_cost_eur, -1.2 by default (D-1).
    assert logit["beta_cost_eur"]["value"] == -1.2 and logit["beta_cost_eur"]["unit"] == "per EUR"
    assert logit["beta_bike_mixed_km"]["value"] == -0.16  # D-4
    prices = {r["name"]: r for r in rows if r["group"] == "prices"}
    assert prices["car_eur_km"]["unit"] == "EUR/km" and prices["fare_base_eur"]["unit"] == "EUR"
    assert all(r["argument"] == "price_options" for r in prices.values())


def test_every_row_is_accepted_by_its_argument():
    rows = table()
    net = ms.examples.toy_network()
    trip = [("t0", 0, *net.node_lonlat("W"), *net.node_lonlat("D1"), 0, "commuter", None)]

    def run(**kwargs):
        return ms.Scenario.from_parts(net, trip, classes=CAR, **kwargs).run(
            "par", quiet=True, overwrite=True
        )

    def options(group):
        return {r["name"]: r["value"] for r in rows if r["group"] == group}

    ms.network_read_table(
        [{"from": "a", "to": "b", "length": 100.0}],
        [{"node": "a", "lon": 4.9, "lat": 52.37}, {"node": "b", "lon": 4.901, "lat": 52.37}],
        network_options=options("network"),
    )
    run(loading_options=options("loading"), mode_options=options("modes"),
        price_options=options("prices"))  # fmt: skip
    for strategy in ("msa", "free_flow"):
        run(equilibration=strategy, equilibration_options=options(f"equilibration.{strategy}"))
    for group in {r["group"] for r in rows if r["group"].startswith("route_method.")}:
        run(route_method=group.split(".", 1)[1], route_options=options(group) or None)
    run(route_update="least_cost", route_update_options=options("route_update.least_cost"))
    for group in {r["group"] for r in rows if r["group"].startswith("choice_model.")}:
        run(choice_model=group.split(".", 1)[1], choice_options=options(group) or None)
    transit = options("transit")
    assert transit and all(v >= 0 for v in transit.values())


def test_the_services_assumptions_policies_and_fleet_rows_are_accepted_as_listed():
    rows = table()

    def options(group):
        return {r["name"]: r["value"] for r in rows if r["group"] == group}

    net = ms.examples.manhattan_grid(3, 200.0, False)
    at = ms._core.grid_node_lonlat(net, 0, 0)
    docks = ms.dock_read_table([{"dock_id": "a", "lon": at[0], "lat": at[1], "capacity": 4}])
    taxis = ms.vehicle_read_table([{"vehicle_id": "v", "lon": at[0], "lat": at[1]}])
    bikes = ms.service_bike_docked(docks, options=options("service.bike_docked"))
    fleet = ms.service_taxi(taxis, options=options("service.taxi"))
    assert options("service.taxi")["request_cancel_s"] == 600.0
    assumed = options("service_assumptions")
    assert set(assumed) == {
        "assumed_wait_s",
        "assumed_no_taxi_share",
        "assumed_extra_walk_at_pickup_s",
        "assumed_extra_walk_at_return_s",
    }
    for name in ("noop", "reset", "threshold", "charge_rule"):
        ms.fleet_day(net, [bikes, fleet], [], policy=name, policy_options=options(f"policy.{name}"))
    # The three listed from S306: a service's log switch, noop's step, each class's BPR pair.
    assert (
        options("service.bike_docked")["log_events"] == options("service.taxi")["log_events"] == 1
    )
    assert options("policy.noop") == {"step_s": 900.0}
    loading = options("loading")
    assert loading["motorway.bpr_alpha"] == loading["bpr_alpha"]
    assert loading["service.bpr_beta"] == loading["bpr_beta"]
    ms.fleet_environment(
        net, [bikes], [], reward_weights=options("fleet_environment"),
        zone_grid_m=options("fleet")["zone_grid_m"],
    )  # fmt: skip
    assert options("bike_route.doubly_stochastic"), "the bike route sets' options"
    units = {r["name"]: r["unit"] for r in rows if r["group"] == "service_prices"}
    assert units == {
        "bike_share_eur_per_ride": "EUR per ride",
        "bike_share_eur_per_min": "EUR/min",
        "taxi_eur_per_ride": "EUR per ride",
        "taxi_eur_per_km": "EUR/km",
    }


def test_every_row_has_a_meaning_and_a_source_written_once():
    # The meanings and sources are one copy, the package's data/parameters.csv: every row of
    # the registry has both, the file names no row the registry lacks, and the run configuration
    # file's comments are the registry's meanings, not a second copy (S306).
    import csv
    from pathlib import Path

    from openmobisim import config as cfg

    rows = table()
    empty = [(r["group"], r["name"]) for r in rows if not r["meaning"].strip()]
    assert not empty, f"parameters without a meaning: {empty}"
    unsourced = [(r["group"], r["name"]) for r in rows if not r["source"].strip()]
    assert not unsourced, f"parameters without a source: {unsourced}"
    path = Path(ms.__file__).parent / "data" / "parameters.csv"
    with path.open(newline="", encoding="utf-8") as f:
        written = [(r["group"], r["name"]) for r in csv.DictReader(f)]
    assert len(written) == len(set(written)), "each parameter once"
    assert set(written) == {(r["group"], r["name"]) for r in rows}
    assert not hasattr(cfg, "_MEANING"), "no second copy of the meanings"
    meaning = {(r["group"], r["name"]): r["meaning"] for r in rows}
    line = next(x for x in cfg._lines()["choice"] if x.key == "beta_time_min")
    assert meaning[("choice_model.logit", "beta_time_min")].lower().startswith(line.meaning)
