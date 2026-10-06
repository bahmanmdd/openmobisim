"""Traveller classes (S231): the modes a class may use, what it owns, its own coefficients.

What is defended: a class table is read as written and refused where it is wrong; classes are
drawn for travellers by their shares, exactly to within one traveller and the same for the same
seed; a class's modes bound what its trips choose among, and a class not listed may use them
all; a class's coefficients move its choices and only its; both are part of the run's
fingerprint; the old ``class_defaults`` is refused with a pointer.

The toy network's hand values are the mode tests' (``test_modes.py``): ``W → M`` is fastest by
car (80 s, against the bike's 120 s).
"""

from __future__ import annotations

import openmobisim as ms
import pytest

TABLE = """class,share,modes,owns_car,owns_bike,has_transit_pass,beta_mode_car,beta_mode_bike,beta_mode_walk,beta_mode_transit,walk_max_s,bike_max_s,access_walk_max_s
car_captive,0.05,car,1,0,0,0,,,,,,
bike_enthusiast,0.45,bike;walk;transit;bike_transit,0,1,1,,1.5,0,0,,5400,
transit_only,0.15,walk;transit,0,0,1,,,0,0.5,,,
open_to_all,0.25,car;bike;walk;transit;car_transit;bike_transit,1,1,1,0,0,0,0,,,
walker,0.05,walk;transit,0,0,1,,,1.0,0,2700,,
park_and_ride_commuter,0.05,car;transit;car_transit,1,0,1,0,,,0.3,,,
"""  # noqa: E501


def toy_trips(n: int, cls: str = "everyone", frm: str = "W", to: str = "M") -> list[tuple]:
    net = ms.examples.toy_network()
    o, d = net.node_lonlat(frm), net.node_lonlat(to)
    return [(f"{cls}{i}", 0, o[0], o[1], d[0], d[1], 0, cls, None, None) for i in range(n)]


def toy_run(rows: list[tuple], run_id: str, classes: dict, **kwargs: object) -> ms.Run:
    return ms.Scenario.from_parts(
        network=ms.examples.toy_network(),
        demand=rows,
        classes=classes,
        transit=None,
        equilibration="free_flow",
        flow_level=0,
        **kwargs,
    ).run(run_id=run_id, quiet=True)


def modes_of(run: ms.Run) -> dict[str, set[str]]:
    """The modes each class's travellers took."""
    ch = run.itinerary_choices()
    out: dict[str, set[str]] = {}
    for who, cls, mode in zip(ch["traveller_id"], ch["user_class"], ch["mode"], strict=True):
        assert who.rstrip("0123456789") == cls, "the class is the traveller id's prefix here"
        out.setdefault(cls, set()).add(mode)
    return out


# --- the class table ------------------------------------------------------------------------------


def test_the_example_table_is_read_as_written(tmp_path) -> None:
    path = tmp_path / "classes.csv"
    path.write_text(TABLE)
    classes = ms.demand_read_classes(path)
    assert list(classes) == [
        "car_captive", "bike_enthusiast", "transit_only", "open_to_all", "walker",
        "park_and_ride_commuter",
    ]  # fmt: skip
    assert sum(c["share"] for c in classes.values()) == pytest.approx(1.0)
    bike = classes["bike_enthusiast"]
    assert bike["modes"] == ["bike", "walk", "transit", "bike_transit"]
    assert (bike["owns_car"], bike["owns_bike"], bike["has_transit_pass"]) == (False, True, True)
    assert bike["betas"] == {"beta_mode_bike": 1.5, "beta_mode_walk": 0.0, "beta_mode_transit": 0.0}
    assert classes["car_captive"]["betas"] == {"beta_mode_car": 0.0}, "an empty cell is unsaid"
    assert bike["limits"] == {"bike_max_s": 5400.0}
    assert classes["walker"]["limits"] == {"walk_max_s": 2700.0}
    assert classes["car_captive"]["limits"] == {}, "the run's"


def test_unsaid_ownership_follows_the_modes_and_a_tuple_says_only_what_is_owned() -> None:
    from openmobisim.demand import _class_table

    t = _class_table({
        "pr": {"modes": "car;car_transit"},
        "cyclist": {"modes": ["bike"]},
        "old": (True, False, True),
        "nothing": {},
    })  # fmt: skip
    assert (t["pr"]["owns_car"], t["pr"]["owns_bike"], t["pr"]["has_transit_pass"]) == (
        True, False, True,
    )  # fmt: skip
    assert (t["cyclist"]["owns_car"], t["cyclist"]["owns_bike"]) == (False, True)
    assert t["old"]["modes"] is None and t["old"]["owns_car"] and t["old"]["has_transit_pass"]
    assert t["nothing"]["modes"] is None and not t["nothing"]["owns_car"]


@pytest.mark.parametrize(
    ("text", "message"),
    [
        ("class,modes\na,car\na,bike\n", "listed twice"),
        ("class,modes\n,car\n", "has no class"),
        ("class,modes\na,car;plane\n", "modes must be among"),
        ("class,owns_car\na,maybe\n", "true or false"),
        ("class,colour\na,red\n", "no such column"),
        ("class,share\na,-1\n", "share must be"),
        ("class,beta_mode_bike\na,nan\n", "finite"),
        ("class,bike_max_s\na,0\n", "bike_max_s must be a number of seconds above 0"),
        ("class,access_walk_max_s\na,inf\n", "access_walk_max_s must be a finite number"),
        ("class,bike_max\na,60\n", "no such column"),
    ],
)
def test_a_wrong_table_is_refused_with_its_row(tmp_path, text: str, message: str) -> None:
    path = tmp_path / "classes.csv"
    path.write_text(text)
    with pytest.raises(ValueError, match=message):
        ms.demand_read_classes(path)


# --- drawing classes ------------------------------------------------------------------------------


def test_classes_are_drawn_by_share_to_within_one_traveller_and_by_seed(tmp_path) -> None:
    path = tmp_path / "classes.csv"
    path.write_text(TABLE)
    classes = ms.demand_read_classes(path)
    # Two trips per traveller: a chain keeps one class.
    rows = [r for t in toy_trips(1000) for r in (t, (*t[:1], 1, *t[2:]))]
    drawn = ms.demand_assign_classes(rows, classes, seed=0)
    assert [r[:7] for r in drawn] == [r[:7] for r in rows], "only the class changes"
    of = {}
    for r in drawn:
        assert of.setdefault(r[0], r[7]) == r[7], "a traveller's trips share a class"
    for name, c in classes.items():
        assert abs(sum(1 for v in of.values() if v == name) - 1000 * c["share"]) <= 1, name
    assert ms.demand_assign_classes(rows, classes, seed=0) == drawn
    assert ms.demand_assign_classes(rows, classes, seed=1) != drawn


def test_the_shares_are_of_people_by_weight() -> None:
    rows = [(*t[:8], 3 if i < 50 else 1, None) for i, t in enumerate(toy_trips(200))]
    drawn = ms.demand_assign_classes(rows, {"a": {"share": 1}, "b": {"share": 1}}, seed=4)
    people = {"a": 0, "b": 0}
    for r in drawn:
        people[r[7]] += r[8]
    assert abs(people["a"] - people["b"]) <= 3, people


def test_a_class_without_a_share_cannot_be_drawn() -> None:
    with pytest.raises(ValueError, match="needs a share"):
        ms.demand_assign_classes(toy_trips(3), {"a": {"share": 1}, "b": {"modes": "car"}})


# --- running classes ------------------------------------------------------------------------------


def test_a_class_chooses_among_its_modes_only_and_an_unlisted_class_among_all() -> None:
    rows = toy_trips(20, "drivers") + toy_trips(20, "cyclists") + toy_trips(20, "others")
    run = toy_run(
        rows,
        "classes-modes",
        {"drivers": {"modes": ["car"]}, "cyclists": {"modes": ["bike", "walk"]},
         "others": (True, True, False)},
        choice_model="logit",
    )  # fmt: skip
    took = modes_of(run)
    assert took["drivers"] == {"car"}
    assert took["cyclists"] <= {"bike", "walk"} and "bike" in took["cyclists"]
    assert {"car", "bike"} <= took["others"], "a class that names no modes may use the run's"


def test_without_modes_the_run_offers_every_mode_a_class_names() -> None:
    rows = toy_trips(10, "drivers") + toy_trips(10, "cyclists")
    classes = {"drivers": {"modes": ["car"]}, "cyclists": {"modes": ["bike"]}}
    run = toy_run(rows, "classes-union", classes, choice_model="deterministic")
    assert modes_of(run) == {"drivers": {"car"}, "cyclists": {"bike"}}
    narrowed = toy_run(rows, "classes-narrowed", classes, modes=["car"])
    assert set(narrowed.completion_by_mode) == {"car"}, "modes= still bounds the run"


def test_a_class_coefficient_moves_that_class_only_and_is_in_the_fingerprint() -> None:
    rows = toy_trips(200, "keen") + toy_trips(200, "plain")
    both = {"modes": ["car", "bike"], "owns_car": True, "owns_bike": True}
    plain = toy_run(rows, "classes-plain", {"keen": both, "plain": both}, choice_model="logit")
    keen = toy_run(
        rows,
        "classes-keen",
        {"keen": {**both, "beta_mode_bike": 3.0}, "plain": both},
        choice_model="logit",
    )

    def bike_share(run: ms.Run, cls: str) -> float:
        ch = run.itinerary_choices()
        pairs = zip(ch["traveller_id"], ch["mode"], strict=True)
        picks = [m for w, m in pairs if w.startswith(cls)]
        return picks.count("bike") / len(picks)

    assert bike_share(keen, "keen") > bike_share(plain, "keen") + 0.3
    assert bike_share(keen, "plain") == bike_share(plain, "plain"), "the other class is untouched"
    assert keen.fingerprint != plain.fingerprint
    narrower = {"keen": {**both, "modes": ["car"]}, "plain": both}
    narrowed = toy_run(rows, "classes-narrower", narrower, choice_model="logit")
    assert narrowed.fingerprint != plain.fingerprint, "a class's modes are in it too"


def test_class_coefficients_need_a_random_utility_model() -> None:
    rows = toy_trips(2, "keen")
    keen = {"keen": {"modes": ["car", "bike"], "beta_mode_bike": 1.0}}
    with pytest.raises(ValueError, match="logit"):
        toy_run(rows, "classes-deterministic", keen, choice_model="deterministic")

    class Mine:
        def choose(self, batch):  # pragma: no cover - refused before any choice
            raise AssertionError

    with pytest.raises(ValueError, match="constructor"):
        toy_run(rows, "classes-mine", keen, choice_model=Mine())
    with pytest.raises(ValueError, match="tme_min"):
        toy_run(rows, "classes-typo", {"keen": {"beta_tme_min": -1.0}}, choice_model="logit")


def test_the_old_class_defaults_is_refused_with_a_pointer() -> None:
    with pytest.raises(ValueError, match="class_defaults is now classes"):
        ms.Scenario.from_parts(
            ms.examples.toy_network(), toy_trips(1), class_defaults={"everyone": (1, 1, 0)}
        )


def test_the_example_classes_are_the_table_s(tmp_path) -> None:
    path = tmp_path / "classes.csv"
    path.write_text(TABLE)
    assert ms.examples.traveller_classes() == ms.demand_read_classes(path)


def test_a_class_allowed_every_mode_leaves_the_fingerprint_as_it_was() -> None:
    rows = toy_trips(5, "everyone")
    old = toy_run(rows, "classes-tuple", {"everyone": (True, True, False)}, modes=ms.MODES)
    every = {"modes": list(ms.MODES), "owns_car": 1, "owns_bike": 1, "has_transit_pass": 0}
    new = toy_run(rows, "classes-every", {"everyone": every})
    assert new.fingerprint == old.fingerprint


# --- a class's own choice-set limits (S235) -------------------------------------------------------


def test_a_class_s_own_ride_limit_replaces_the_run_s_for_its_trips_only() -> None:
    # W → M: the bike takes 120 s, the car 80 s (the module's docstring). A run offering rides of
    # up to 100 s offers none; a class riding up to 200 s is offered one, the others still not.
    rows = toy_trips(10, "keen") + toy_trips(10, "plain")
    both = {"modes": ["car", "bike"], "beta_mode_bike": 3.0}
    short = {"bike_max_s": 100}
    plain = toy_run(
        rows, "limits-plain", {"keen": both, "plain": both},
        choice_model="logit", mode_options=short,
    )  # fmt: skip
    keen = toy_run(
        rows, "limits-keen", {"keen": {**both, "bike_max_s": 200}, "plain": both},
        choice_model="logit", mode_options=short,
    )  # fmt: skip
    assert modes_of(plain) == {"keen": {"car"}, "plain": {"car"}}
    took = modes_of(keen)
    assert "bike" in took["keen"] and took["plain"] == {"car"}
    assert keen.fingerprint != plain.fingerprint
    assert keen.manifest()["class_limits"] == {"keen": {"bike_max_s": 200}}
    assert plain.manifest()["class_limits"] is None, "no class gives one"
    assert plain.manifest()["bike_max_s"] == 100


def test_the_shipped_limits_and_an_infinite_one_are_recorded() -> None:
    rows = toy_trips(2, "everyone")
    run = toy_run(
        rows, "limits-inf", {"everyone": {"modes": ["car", "bike", "walk"], "walk_max_s": "inf"}}
    )
    m = run.manifest()
    assert (m["walk_max_s"], m["bike_max_s"]) == (1800, 3600), "S235: a ride up to an hour"
    assert m["class_limits"] == {"everyone": {"walk_max_s": "inf"}}, "JSON has no infinity"


# --- a model of one's own sees the classes (S236, roadmap I-bb U1) -------------------------------


def test_a_model_of_one_s_own_sees_each_situation_s_class_by_name() -> None:
    import numpy as np
    from openmobisim import choice

    class BikesForKeenOnes:
        # The keen class always rides when it can; everyone else takes the quickest.
        name = "bikes_for_keen_ones"
        seen: set[str] = set()

        def choose(self, batch):
            keen = np.array([batch.class_names[c] == "keen" for c in batch.user_class])
            BikesForKeenOnes.seen.update(batch.class_names[c] for c in batch.user_class)
            bike = batch.attributes["mode_bike"] > 0
            utility = -batch.attributes["time_min"] + 100.0 * (bike & keen[batch.situation_of])
            return choice.segment_argmax(utility, batch.offsets)

    rows = toy_trips(5, "keen") + toy_trips(5, "plain")
    both = {"modes": ["car", "bike"], "owns_car": True, "owns_bike": True}
    run = toy_run(rows, "classes-own-model", {"keen": both, "plain": both},
                  choice_model=BikesForKeenOnes())  # fmt: skip
    assert modes_of(run) == {"keen": {"bike"}, "plain": {"car"}}, "car 80 s, bike 120 s"
    assert BikesForKeenOnes.seen == {"keen", "plain"}
