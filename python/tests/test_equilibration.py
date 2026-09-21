"""Equilibration across the boundary (S170).

What is defended: a run without it reports one iteration and is unchanged; with
``"msa"`` every iteration is reported, the run's result is the last iteration's,
the numbers agree between ``Run.convergence()``, the manifest and ``kpis.parquet``,
the settings are refused before any work if wrong, and a Python model can give the
probabilities the gap needs. The dynamics (that iterating moves travellers off a
jammed road, that the gap is measured against chance) are proved in Rust on a
hand-made bottleneck, where the answer can be worked out.
"""

import numpy as np
import openmobisim as ms
import pytest

CAR = {"commuter": (True, False, False)}
FIELDS = [
    "iteration", "reselected_share", "changed_share", "total_travel_time_s", "completed",
    "truncated", "time_change", "gap", "gap_network", "gap_flow", "gap_flow_floor",
    "gap_flow_excess", "routes_added", "route_searches", "gap_expected", "gap_excess",
    "gap_network_excess", "incomplete_share",
]  # fmt: skip


def scenario(trips=600, **kwargs):
    net = ms.examples.manhattan_grid(n=8, block_metres=200.0, signals=False)
    rows = ms.examples.trips_random(net, trips, seed=5, min_m=300.0, max_m=1400.0, spread_s=600)
    settings = {"class_defaults": CAR, "window_hours": 2, "flow_level": 4, "link_bin_s": 300}
    settings.update(kwargs)
    return ms.Scenario.from_parts(net, rows, **settings)


def go(name, **kwargs):
    return scenario(**kwargs).run(name)


# These tests defend equilibration on the method's own sets: no route update, whatever an iterating
# run defaults to (S179; `test_an_iterating_run_defaults_...` below defends that).
MSA = {
    "choice_model": "logit",
    "equilibration": "msa",
    "master_seed": 3,
    "route_method": "penalty",
    "route_update": "none",
}


def test_the_strategies_are_listed_with_the_default_first():
    assert ms.equilibration_strategies() == ["none", "msa"]


def test_without_equilibration_there_is_one_iteration_and_no_gap():
    run = go("eq-none")
    c = run.convergence()
    assert set(c) == set(FIELDS) and all(len(v) == 1 for v in c.values())
    assert c["iteration"].tolist() == [0] and c["reselected_share"][0] == 1.0
    assert np.isnan(c["gap_flow"][0]) and np.isnan(c["time_change"][0])
    assert c["total_travel_time_s"][0] == run.total_travel_time_s
    assert (run.equilibration, run.converged) == ("none", False)
    manifest = run.manifest()
    assert manifest["equilibration"] == "none" and manifest["iterations_run"] == 1
    assert manifest["converged"] is False and manifest["live_streams"] == []


def test_msa_reports_every_iteration_and_the_run_is_the_last_one():
    run = go("eq-msa", equilibration_options={"iterations": 5}, **MSA)
    c = run.convergence()
    assert c["iteration"].tolist() == [0, 1, 2, 3, 4]
    assert all(len(v) == 5 for v in c.values())
    assert run.equilibration == "msa"
    # Everyone chooses first; a share 1/(i + 1) chooses again at iteration i.
    assert c["reselected_share"][0] == 1.0
    assert np.allclose(c["reselected_share"][1:], [1 / 2, 1 / 3, 1 / 4, 1 / 5], atol=0.06)
    assert (c["changed_share"][1:] <= c["reselected_share"][1:] + 1e-12).all()
    # The result is what the last iteration loaded.
    assert run.total_travel_time_s == c["total_travel_time_s"][-1]
    assert run.completion["completed"] == c["completed"][-1]
    assert run.completion["truncated"] == c["truncated"][-1]
    # The gap is measured (the logit has probabilities), and so is its floor.
    assert np.isfinite(c["gap_flow"]).all() and np.isfinite(c["gap_flow_floor"]).all()
    assert np.allclose(c["gap_flow_excess"], c["gap_flow"] - c["gap_flow_floor"])
    assert np.isnan(c["time_change"][0]) and np.isfinite(c["time_change"][1:]).all()
    manifest = run.manifest()
    assert manifest["equilibration"] == "msa" and manifest["iterations_run"] == 5
    assert manifest["live_streams"] == ["choice", "msa_reselection"]
    assert "iterations=5" in manifest["equilibration_descriptor"]


def test_one_iteration_of_msa_is_the_run_without_it():
    base = go("eq-one-none", choice_model="logit", master_seed=3)
    one = go("eq-one-msa", equilibration_options={"iterations": 1}, **MSA)
    assert one.total_travel_time_s == base.total_travel_time_s
    assert (one.route_choices().route == base.route_choices().route).all()
    assert one.fingerprint != base.fingerprint, "a strategy is an input, even one that does nothing"


def test_iterating_is_repeatable_and_follows_the_seed():
    def run(seed):
        return go(
            "eq-seed", equilibration_options={"iterations": 4}, **{**MSA, "master_seed": seed}
        )

    a, b, c = run(1), run(1), run(2)
    for key in FIELDS:
        assert np.array_equal(a.convergence()[key], b.convergence()[key], equal_nan=True), key
    assert (a.route_choices().route == b.route_choices().route).all()
    assert (a.route_choices().route != c.route_choices().route).any()


def test_the_kpis_file_has_a_row_set_per_iteration():
    pytest.importorskip("pandas")
    run = go("eq-kpis", equilibration_options={"iterations": 4}, **MSA)
    df = run.kpis().to_pandas()
    assert sorted(df["iteration"].unique()) == [0, 1, 2, 3]
    total = df[df["metric"] == "total_travel_time_s"].sort_values("iteration")["value"].to_numpy()
    assert np.allclose(total, run.convergence()["total_travel_time_s"])
    # The disequilibrium and what goes with it are in the file (S178), from the iteration measured.
    assert {"gap_expected", "gap_excess", "incomplete_share"} <= set(df["metric"])
    diseq = df[df["metric"] == "gap_excess"].sort_values("iteration")["value"].to_numpy()
    assert np.allclose(
        diseq, run.convergence()["gap_excess"][np.isfinite(run.convergence()["gap_excess"])]
    )
    excess = df[df["metric"] == "gap_flow_excess"].sort_values("iteration")["value"].to_numpy()
    assert np.allclose(excess, run.convergence()["gap_flow_excess"])
    # A number not measured has no row; what cannot change between loadings is written once.
    assert 0 not in df[df["metric"] == "time_change"]["iteration"].tolist()
    assert (df["metric"] == "completion_rate").sum() == 1
    # Without iterating, the file is what it was.
    plain = go("eq-kpis-plain").kpis().to_pandas()
    assert sorted(plain["metric"]) == sorted(
        [
            "total_travel_time_s", "completed_trips", "truncated_trips",
            "no_vehicle_available_trips", "no_feasible_path_trips", "completion_rate",
        ]
    )  # fmt: skip


def test_bad_settings_are_refused_before_any_work():
    net = ms.examples.toy_network()
    rows = [("t0", 0, *net.node_lonlat("W"), *net.node_lonlat("D1"), 0, "commuter", None)]

    def build(**kwargs):
        return ms.Scenario.from_parts(net, rows, class_defaults=CAR, **kwargs).run("eq-bad")

    with pytest.raises(
        ValueError, match='no equilibration strategy called "replanning".*none, msa'
    ):
        build(equilibration="replanning")
    with pytest.raises(ValueError, match="cost_bin_s, gap_sample, gap_tolerance, iterations"):
        build(equilibration="msa", equilibration_options={"steps": 3})
    for bad in (0, 1001, 2.5):
        with pytest.raises(ValueError, match="whole number from 1 to 1000"):
            build(equilibration="msa", equilibration_options={"iterations": bad})
    with pytest.raises(ValueError, match="from 0 to 1"):
        build(equilibration="msa", equilibration_options={"gap_tolerance": 1.5})
    with pytest.raises(ValueError, match="no options"):
        build(equilibration="none", equilibration_options={"iterations": 3})


def test_a_tolerance_stops_a_run_that_has_nothing_left_to_equilibrate():
    # Free flow: costs never change, so the gap is what the logit's dispersion gives.
    run = go(
        "eq-stop",
        flow_level=0,
        choice_model="logit",
        equilibration="msa",
        equilibration_options={"iterations": 12, "gap_tolerance": 0.5},
        master_seed=3,
    )
    assert run.converged and len(run.convergence()["iteration"]) == 4
    assert run.manifest()["converged"] is True and run.manifest()["iterations_run"] == 4


def test_a_model_without_probabilities_has_no_gap_and_one_with_them_has():
    class Plain:
        name = "plain"
        sampled = False
        attributes = ["time_min"]

        def choose(self, batch):
            return np.zeros(len(batch), dtype=np.int64)

    class Knowing(Plain):
        name = "knowing"

        def probabilities(self, batch):
            one_hot = np.zeros(len(batch.identity))
            one_hot[batch.offsets[:-1]] = 1.0
            return one_hot

    without = go("eq-plain", choice_model=Plain(), equilibration="msa",
                 equilibration_options={"iterations": 3})  # fmt: skip
    c = without.convergence()
    assert np.isnan(c["gap_flow"]).all(), "no probabilities, no consistency check"
    assert np.isfinite(c["gap"]).all(), "but the gap does not need them"
    assert np.isfinite(c["time_change"][1:]).all(), "the stability is measured whatever the model"
    knowing = go("eq-knowing", choice_model=Knowing(), equilibration="msa",
                 equilibration_options={"iterations": 3})  # fmt: skip
    k = knowing.convergence()
    assert np.isfinite(k["gap_flow"]).all() and np.isfinite(k["gap"]).all()
    # It always takes the first route and says so with certainty; the flow gap is then the share
    # who should be elsewhere at the current times, and the fresh sample is the same route again.
    assert np.allclose(k["gap_flow_floor"], 0.0)


def test_the_gap_has_a_verdict_and_the_network_is_tested_at_the_last_iteration():
    run = go(
        "eq-gap",
        flow_level=0,
        choice_model="logit",
        choice_options={"beta_time_min": -2.0},
        equilibration="msa",
        equilibration_options={"iterations": 4, "gap_sample": 100},
        master_seed=3,
    )
    c = run.convergence()
    assert np.isfinite(c["gap"]).all() and (c["gap"] >= 0).all()
    assert np.isnan(c["gap_network"][:-1]).all() and np.isfinite(c["gap_network"][-1])
    # The verdict is on the disequilibrium: the gap less what the logit itself expects (S178).
    assert np.allclose(c["gap_excess"], c["gap"] - c["gap_expected"])
    assert (c["gap_expected"] >= 0).all() and (c["incomplete_share"] == 0).all()
    assert np.isnan(c["gap_network_excess"][:-1]).all() and np.isfinite(c["gap_network_excess"][-1])
    worst = max(c["gap_excess"][-1], c["gap_network_excess"][-1])
    assert run.convergence_gap == worst
    assert run.convergence_verdict == (
        "good" if worst < 0.05 else "acceptable" if worst < 0.15 else "poor"
    )
    # Free flow, a strong preference for the faster route: a small gap. Without equilibration, none.
    assert run.convergence_verdict in {"good", "acceptable"}
    none = go("eq-gap-none")
    assert np.isnan(none.convergence_gap) and none.convergence_verdict is None
    off = go(
        "eq-gap-off", flow_level=0, choice_model="logit", equilibration="msa",
        equilibration_options={"iterations": 3, "gap_sample": 0},
    )  # fmt: skip
    assert np.isnan(off.convergence()["gap_network"]).all()


def test_the_footer_says_the_run_iterated():
    pytest.importorskip("matplotlib")
    from openmobisim import viz

    run = go("eq-footer", equilibration_options={"iterations": 3}, **MSA)
    fig = viz.map_link(run, size=(12, 6.75), dpi=50)
    text = " ".join(t.get_text() for t in fig.texts)
    assert "choice logit · msa 3 it, gap " in text and "% · seed 3" in text


# --- the disequilibrium, the choice-set limit and the warm-up (S178) ----------------------------


def test_the_disequilibrium_is_what_the_choice_model_does_not_explain():
    logit = go("eq-diseq-logit", equilibration_options={"iterations": 4}, **MSA)
    c = logit.convergence()
    # A logit's gap is never 0 by design; the model expects it, and what is left is small.
    assert (c["gap_expected"][:-1] > 0).all() and np.allclose(
        c["gap_excess"], c["gap"] - c["gap_expected"]
    )
    assert abs(c["gap_excess"][-1]) < 0.05 and logit.convergence_verdict == "good"
    # An all-or-nothing model expects no gap: its disequilibrium is its gap.
    aon = go(
        "eq-diseq-aon", equilibration="msa", equilibration_options={"iterations": 3}, master_seed=3
    )
    a = aon.convergence()
    assert np.allclose(a["gap_expected"], 0) and np.allclose(a["gap_excess"], a["gap"])
    assert (a["incomplete_share"] == 0).all()


def test_the_choice_detour_limit_is_offered_routes_within_it_of_the_best():
    net = ms.examples.toy_network()
    rows = [
        (f"t{i}", 0, *net.node_lonlat("W"), *net.node_lonlat("D1"), 10 * i, "commuter", None)
        for i in range(400)
    ]

    def routes(limit):
        run = ms.Scenario.from_parts(
            net,
            rows,
            class_defaults=CAR,
            choice_model="logit",
            choice_options={"beta_time_min": -0.05},
            choice_detour_limit=limit,
            master_seed=3,
        ).run("eq-limit")
        return run.route_choices()

    everyone = routes(0.0)
    assert routes(None).route.tolist() == everyone.route.tolist(), "0 is the library's default"
    # A tiny limit leaves only the best route on offer: everyone takes it, certainly.
    only = routes(1e-9)
    assert (only.rank == 0).all() and (only.probability == 1.0).all()
    assert (everyone.alternatives == only.alternatives).all(), "the set is the same"
    with pytest.raises(ValueError, match="choice_detour_limit"):
        routes(-0.1)
    with pytest.raises(ValueError, match="choice_detour_limit"):
        routes(float("nan"))


def test_a_warmup_needs_a_whole_number_and_leaves_the_last_loading_at_full_fidelity():
    # A jammed grid, where the point-queue model and the full one differ.
    heavy = {"trips": 3_500, **MSA}
    warm = go(
        "eq-warm",
        equilibration_options={"iterations": 3, "warmup": 1, "gap_sample": 50},
        **heavy,
    )
    plain = go(
        "eq-warm-plain",
        equilibration_options={"iterations": 3, "warmup": 0, "gap_sample": 50},
        **heavy,
    )
    # The first loading is the point-queue model's: not the same; the run says what it was told.
    first = "total_travel_time_s"
    assert warm.convergence()[first][0] != plain.convergence()[first][0]
    assert "warmup=1" in warm.manifest()["equilibration_descriptor"]
    assert warm.fingerprint != plain.fingerprint
    # Cut to leave the last loading: 9 warm-up loadings of 3 are 2.
    cut = go(
        "eq-warm-cut", equilibration_options={"iterations": 3, "warmup": 9, "gap_sample": 50}, **MSA
    )
    two = go(
        "eq-warm-two", equilibration_options={"iterations": 3, "warmup": 2, "gap_sample": 50}, **MSA
    )
    assert cut.total_travel_time_s == two.total_travel_time_s
    for bad in (-1, 1.5, 1001):
        with pytest.raises(ValueError, match="warmup"):
            go("eq-warm-bad", equilibration_options={"iterations": 3, "warmup": bad}, **MSA)


def test_an_iterating_run_defaults_to_one_route_per_pair_an_update_and_a_warmup():
    net = ms.examples.manhattan_grid(n=6, block_metres=200.0, signals=False)
    rows = ms.examples.trips_random(net, 300, seed=5, min_m=300.0, max_m=1000.0, spread_s=300)

    def run(name, **kwargs):
        settings = {"class_defaults": CAR, "flow_level": 4, "choice_model": "logit"}
        settings.update(kwargs)
        return ms.Scenario.from_parts(net, rows, **settings).run(name)

    # One loading: the penalty method's alternatives, no update (nothing to iterate over).
    once = run("eq-default-once")
    assert once.route_sets().method == "penalty" and once.route_update == "none"
    # One iteration of msa is one loading too.
    one = run("eq-default-one", equilibration="msa", equilibration_options={"iterations": 1})
    assert one.route_sets().method == "penalty" and one.route_update == "none"
    # Iterating: one route per pair to start, the update to find the rest, a warm-up of one loading.
    many = run("eq-default-many", equilibration="msa", equilibration_options={"iterations": 3})
    assert many.route_sets().method == "shortest" and many.route_update == "best_response"
    assert "warmup=1" in many.manifest()["equilibration_descriptor"]
    assert many.manifest()["route_method"] == "shortest"
    # Every default can be overridden, and options alone mean the method they belong to.
    named = run(
        "eq-default-named",
        equilibration="msa",
        equilibration_options={"iterations": 3, "warmup": 0},
        route_method="penalty",
        route_update="none",
    )
    assert named.route_sets().method == "penalty" and named.route_update == "none"
    assert "warmup=0" in named.manifest()["equilibration_descriptor"]
    opts = run(
        "eq-default-options",
        equilibration="msa",
        equilibration_options={"iterations": 3},
        route_options={"max_paths": 2},
    )
    assert opts.route_sets().method == "penalty" and opts.route_update == "best_response"
    assert "max_paths=2" in opts.route_sets().descriptor, "the options reached the method"
