"""A scenario's arguments, read back and changed (D-5 a): ``Scenario.parts`` and ``replace``."""

from __future__ import annotations

import openmobisim as ms
import pytest


def _base() -> ms.Scenario:
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
    )


def test_the_parts_rebuild_the_same_scenario() -> None:
    base = _base()
    assert ms.Scenario.from_parts(**base.parts)._fingerprint() == base._fingerprint()
    assert base.replace()._fingerprint() == base._fingerprint()


def test_replace_changes_what_it_names_and_nothing_else() -> None:
    base = _base()
    changed = base.replace(flow_level=4, master_seed=7)
    assert changed.parts["flow_level"] == 4 and changed.parts["master_seed"] == 7
    assert base.parts["flow_level"] == 2 and base.parts["master_seed"] == 0, "the original kept"
    assert {k for k in base.parts if base.parts[k] is not changed.parts[k]} == {
        "flow_level",
        "master_seed",
    }
    assert changed._fingerprint() != base._fingerprint()


def test_parts_is_a_copy() -> None:
    base = _base()
    parts = base.parts
    parts["flow_level"] = 4
    assert base.parts["flow_level"] == 2


def test_replace_refuses_an_unknown_name_with_the_list() -> None:
    with pytest.raises(TypeError, match="flow_levl.*flow_level"):
        _base().replace(flow_levl=4)


def test_design_id_and_replication_label_the_kpis_not_the_fingerprint(tmp_path) -> None:
    pytest.importorskip("pandas")
    net = ms.examples.toy_network()
    rows = ms.examples.trips_random(net, 20, seed=3, min_m=100.0, max_m=2000.0, spread_s=1800)
    scenario = ms.Scenario.from_parts(net, rows)
    plain = scenario.run("plain", output_dir=str(tmp_path / "plain"), quiet=True)
    named = scenario.run(
        "named", output_dir=str(tmp_path / "named"), quiet=True, design_id="tracks", replication=2
    )
    assert set(plain.kpis().to_pandas()["design_id"]) == {"default"}
    kpis = named.kpis().to_pandas()
    assert set(kpis["design_id"]) == {"tracks"} and set(kpis["replication"]) == {2}
    assert named.fingerprint == plain.fingerprint
    with pytest.raises(ValueError, match="replication"):
        scenario.run("bad", output_dir=str(tmp_path / "bad"), quiet=True, replication=-1)


def test_the_fast_preset_is_the_cases_speed_settings_under_options_given() -> None:
    base = _base()
    parts = base.parts
    fast = base.replace(preset="fast")
    explicit = base.replace(parking_options={"reach_car_step_s": 1800})
    assert fast._fingerprint() == explicit._fingerprint() != base._fingerprint()
    assert base.replace(preset="complete")._fingerprint() == base._fingerprint()
    mine = base.replace(preset="fast", route_update_options={"tree_step_s": 600})
    both = base.replace(
        route_update_options={"tree_step_s": 600}, parking_options={"reach_car_step_s": 1800}
    )
    assert mine._fingerprint() == both._fingerprint(), "an option given by name wins"
    assert fast.parts["route_update_options"] is parts["route_update_options"], "kept as given"
    assert ms.PRESETS["fast"] == {"parking_options": {"reach_car_step_s": 1800}}
    with pytest.raises(ValueError, match="preset"):
        base.replace(preset="quick")
