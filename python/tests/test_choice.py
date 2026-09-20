"""Route choice across the boundary (S169).

What is defended: the built-in models do what they say (all-or-nothing takes
the best route; the logit spreads travellers, follows its seed and its
coefficients, and refuses what it cannot do), and **a model written in Python
plugs in with nothing but a ``choose`` method** and, given the same inputs,
chooses exactly what the built-in logit does.
"""

import numpy as np
import openmobisim as ms
import pytest
from openmobisim import choice

CAR = {"commuter": (True, False, False)}


def grid_run(trips=400, seed=5, **kwargs):
    net = ms.examples.manhattan_grid(n=8, block_metres=200.0, signals=False)
    rows = ms.examples.trips_random(net, trips, seed=seed, min_m=300.0, max_m=1200.0)
    settings = {"class_defaults": CAR, "window_hours": 1, "flow_level": 4, "link_bin_s": 300}
    settings.update(kwargs)
    scenario = ms.Scenario.from_parts(net, rows, **settings)
    return net, rows, scenario


def go(name, **kwargs):
    return grid_run(**kwargs)[2].run(name)


# --- the models by name -------------------------------------------------------------


def test_the_models_are_listed_with_the_default_first():
    assert ms.choice_models() == ["deterministic", "logit"]


def test_by_default_everyone_takes_the_best_route():
    run = go("choice-default")
    rc = run.route_choices()
    routed = rc.rank >= 0
    assert routed.sum() > 300 and (rc.rank[routed] == 0).all()
    assert (rc.probability[routed] == 1.0).all()
    assert (rc.alternatives[routed] >= 1).all() and (rc.weight == 1).all()
    # The route is the first of its pair's set.
    sets = run.route_sets()
    assert (rc.route[routed] == sets.set_offsets()[rc.pair[routed]]).all()
    # Asking for it by name changes nothing.
    named = go("choice-default-named", choice_model="deterministic")
    assert (named.route_choices().route == rc.route).all()
    assert named.total_travel_time_s == run.total_travel_time_s
    assert named.manifest()["live_streams"] == []


def test_a_logit_spreads_travellers_over_the_alternatives():
    run = go("choice-logit", choice_model="logit", master_seed=3)
    rc = run.route_choices()
    routed = rc.rank >= 0
    shares = np.bincount(rc.rank[routed]) / routed.sum()
    assert len(shares) >= 3 and shares[0] < 0.7 and shares[1:].sum() > 0.3
    p = rc.probability[routed]
    assert ((p > 0) & (p <= 1)).all() and p.mean() < 0.6
    sets = run.route_sets()
    assert (rc.route[routed] == sets.set_offsets()[rc.pair[routed]] + rc.rank[routed]).all()
    manifest = run.manifest()
    assert manifest["choice_model"] == "logit" and manifest["live_streams"] == ["choice"]
    assert "beta_time_min=-0.2" in manifest["choice_descriptor"]
    # The loading follows the choices: the same demand puts more traffic on more links.
    best = go("choice-best")
    assert len(np.unique(run.link_bins().links())) > len(np.unique(best.link_bins().links()))
    assert run.link_bins().pcu().sum() > best.link_bins().pcu().sum()


def test_the_seed_decides_a_sampled_choice_and_only_that():
    a = go("choice-seed-a", choice_model="logit", master_seed=1)
    again = go("choice-seed-a2", choice_model="logit", master_seed=1)
    other = go("choice-seed-b", choice_model="logit", master_seed=2)
    assert (a.route_choices().route == again.route_choices().route).all()
    assert a.fingerprint == again.fingerprint
    assert (a.route_choices().route != other.route_choices().route).any()
    assert a.fingerprint != other.fingerprint
    # The all-or-nothing default draws nothing, so the seed cannot move anyone.
    d1 = go("choice-det-1", master_seed=1).route_choices().route
    d2 = go("choice-det-2", master_seed=2).route_choices().route
    assert (d1 == d2).all()


def test_coefficients_change_who_takes_which_route_and_the_fingerprint():
    def excess_s(**options):
        """Mean seconds the chosen routes cost over their pair's best route."""
        run = go("choice-beta", choice_model="logit", choice_options=options, master_seed=4)
        rc, sets = run.route_choices(), run.route_sets()
        routed = rc.route >= 0
        costs = sets.costs()
        best = costs[sets.set_offsets()[rc.pair[routed]]]
        return float((costs[rc.route[routed]] - best).mean()), run.fingerprint

    keen, fp_keen = excess_s(beta_time_min=-2.0)
    lax, fp_lax = excess_s(beta_time_min=0.0, beta_ln_path_size=0.0)
    assert keen < 0.5 * lax, (
        f"time-averse travellers lose less time: {keen:.1f} s against {lax:.1f} s"
    )
    assert fp_keen != fp_lax
    # Any attribute can carry the choice: disliking length picks shorter routes than ignoring it.

    def length_km(**options):
        run = go("choice-len", choice_model="logit", choice_options=options, master_seed=4)
        rc = run.route_choices()
        lengths = run.link_bins().pcu().sum() / max(int((rc.route >= 0).sum()), 1)
        return lengths

    short = length_km(beta_time_min=0.0, beta_ln_path_size=0.0, beta_length_km=-8.0)
    plain = length_km(beta_time_min=0.0, beta_ln_path_size=0.0)
    assert short < plain


def test_bad_choices_are_refused_before_any_work_and_say_what_is_wrong():
    net, rows, _ = grid_run(trips=5)

    def build(**kwargs):
        return ms.Scenario.from_parts(net, rows, class_defaults=CAR, **kwargs).run("choice-bad")

    with pytest.raises(ValueError, match='no choice model called "mnl".*deterministic, logit'):
        build(choice_model="mnl")
    with pytest.raises(ValueError, match='no option "scale"'):
        build(choice_model="logit", choice_options={"scale": 1.0})
    with pytest.raises(ValueError, match="finite"):
        build(choice_model="logit", choice_options={"beta_time_min": float("nan")})
    with pytest.raises(ValueError, match="comfort.*time_min"):
        build(choice_model="logit", choice_options={"beta_comfort": 1.0})
    with pytest.raises(ValueError, match="no options"):
        build(choice_model="deterministic", choice_options={"beta_time_min": 1.0})
    with pytest.raises(ValueError, match="choose"):
        build(choice_model=object())


# --- a model written in Python ---------------------------------------------------------


class MyLogit:
    """The built-in logit's defaults, written the way a researcher would write them."""

    name = "my_logit"
    descriptor = "my_logit;v1"
    attributes = ["ln_path_size", "time_min"]

    def choose(self, batch):
        """Sample the logit of the standard utility."""
        a = batch.attributes
        return choice.sample_random_utility(batch, 1.0 * a["ln_path_size"] - 0.2 * a["time_min"])


def test_a_python_model_chooses_exactly_what_the_builtin_logit_does():
    builtin = go("choice-builtin", choice_model="logit", master_seed=9)
    mine = go("choice-python", choice_model=MyLogit(), master_seed=9)
    assert (mine.route_choices().route == builtin.route_choices().route).all()
    assert np.allclose(mine.route_choices().probability, builtin.route_choices().probability)
    assert mine.total_travel_time_s == builtin.total_travel_time_s
    manifest = mine.manifest()
    assert (manifest["choice_model"], manifest["choice_descriptor"]) == ("my_logit", "my_logit;v1")
    assert manifest["live_streams"] == ["choice"] and mine.fingerprint != builtin.fingerprint


def test_a_model_can_be_written_with_arrays_alone():
    class Plain:
        """No helpers: the batch's arrays and numpy."""

        sampled = True

        def choose(self, batch):
            u = -0.2 * batch.attributes["time_min"] + batch.attributes["ln_path_size"]
            noisy = u + batch.gumbel
            best = np.array(
                [
                    np.argmax(noisy[lo:hi])
                    for lo, hi in zip(batch.offsets[:-1], batch.offsets[1:], strict=True)
                ]
            )
            return best

    builtin = go("choice-plain-b", choice_model="logit", master_seed=2, trips=150)
    plain = go("choice-plain-p", choice_model=Plain(), master_seed=2, trips=150)
    assert (plain.route_choices().route == builtin.route_choices().route).all()
    assert np.isnan(plain.route_choices().probability[plain.route_choices().rank >= 0]).all()
    assert plain.manifest()["choice_model"] == "Plain"


def test_a_python_model_can_ignore_the_random_stream():
    class LeastLength:
        sampled = False
        attributes = ["length_km"]

        def choose(self, batch):
            return choice.segment_argmax(-batch.attributes["length_km"], batch.offsets).tolist()

    run = go("choice-length", choice_model=LeastLength())
    assert run.manifest()["live_streams"] == []
    rc = run.route_choices()
    assert (rc.rank[rc.rank >= 0] >= 0).all()


def test_a_python_model_is_told_when_it_fails_or_answers_wrongly():
    class Raises:
        def choose(self, batch):
            raise RuntimeError("no gradient today")

    class TooShort:
        def choose(self, batch):
            return np.zeros(len(batch) - 1, dtype=np.int64)

    class OutOfRange:
        def choose(self, batch):
            return np.full(len(batch), 99)

    class Negative:
        def choose(self, batch):
            return np.full(len(batch), -1)

    class NotArrays:
        def choose(self, batch):
            return "left"

    net, rows, _ = grid_run(trips=20)

    def build(model):
        scenario = ms.Scenario.from_parts(net, rows, class_defaults=CAR, choice_model=model)
        return scenario.run("choice-fail")

    with pytest.raises(ValueError, match="RuntimeError: no gradient today"):
        build(Raises())
    with pytest.raises(ValueError, match="expected .* choices, got"):
        build(TooShort())
    with pytest.raises(ValueError, match="only .* were offered"):
        build(OutOfRange())
    with pytest.raises(ValueError, match="chose alternative -1"):
        build(Negative())
    with pytest.raises(ValueError, match="could not read choose"):
        build(NotArrays())
    with pytest.raises(ValueError, match="own its settings|choice_options"):
        ms.Scenario.from_parts(
            net, rows, class_defaults=CAR, choice_model=MyLogit(), choice_options={"beta": 1.0}
        ).run("choice-opts")


def test_a_model_that_wants_an_attribute_that_does_not_exist_is_told_what_does():
    class Wants:
        attributes = ["comfort"]

        def choose(self, batch):
            return np.zeros(len(batch), dtype=np.int64)

    net, rows, _ = grid_run(trips=5)
    with pytest.raises(ValueError, match="comfort.*time_min.*ln_path_size"):
        ms.Scenario.from_parts(net, rows, class_defaults=CAR, choice_model=Wants()).run("c-w")


def test_the_batch_a_model_sees_is_what_the_docs_say():
    seen = {}

    class Look:
        def choose(self, batch):
            seen["batch"] = batch
            seen["arrays"] = {
                "offsets": batch.offsets.copy(),
                "identity": batch.identity.copy(),
                "attributes": {k: v.copy() for k, v in batch.attributes.items()},
                "situation_of": batch.situation_of.copy(),
                "gumbel": batch.gumbel.copy(),
            }
            return np.zeros(len(batch), dtype=np.int64)

    go("choice-look", choice_model=Look(), trips=60)
    a = seen["arrays"]
    n, m = len(seen["batch"]), len(a["identity"])
    assert n > 0 and a["offsets"][0] == 0 and a["offsets"][-1] == m and len(a["offsets"]) == n + 1
    assert set(a["attributes"]) == set(choice.ROUTE_ATTRIBUTES)
    assert all(v.shape == (m,) and v.dtype == np.float64 for v in a["attributes"].values())
    assert (np.diff(a["offsets"]) >= 1).all(), "every situation has an alternative"
    assert (a["situation_of"] == np.repeat(np.arange(n), np.diff(a["offsets"]))).all()
    # Within a situation, identities differ; times are best first; the best has no detour.
    first = a["offsets"][:-1]
    assert (a["attributes"]["detour"][first] == 0).all()
    for lo, hi in zip(a["offsets"][:-1], a["offsets"][1:], strict=True):
        assert len(set(a["identity"][lo:hi])) == hi - lo
        assert (np.diff(a["attributes"]["time_min"][lo:hi]) >= -1e-9).all()
    assert (a["attributes"]["ln_path_size"] <= 1e-12).all() and (a["gumbel"] != 0).all()
    assert seen["batch"].iteration == 0 and seen["batch"].attribute_names


# --- the helpers ------------------------------------------------------------------------


def test_segment_helpers_do_the_per_situation_arithmetic():
    offsets = np.array([0, 3, 4, 6])
    values = np.array([1.0, 5.0, 5.0, -2.0, 0.0, 3.0])
    assert choice.segment_argmax(values, offsets).tolist() == [1, 0, 1], "the first of a tie"
    p = choice.segment_softmax(values, offsets)
    assert np.allclose([p[:3].sum(), p[3:4].sum(), p[4:].sum()], 1.0)
    assert p[3] == pytest.approx(1.0)
    assert p[4] == pytest.approx(1 / (1 + np.exp(3.0)))
    assert choice.segment_argmax([], [0]).size == 0 and choice.segment_softmax([], [0]).size == 0
    assert choice.segment_softmax([1000.0, 1001.0], [0, 2]) == pytest.approx(
        [1 / (1 + np.e), np.e / (1 + np.e)]
    )
