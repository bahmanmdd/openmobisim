"""The Rust/Python boundary, asserted from the Python side.

These tests are about the primitives (reductions, the step grid, seeded
draws), not the Phase 1 pipeline — see `test_pipeline.py` for that. They
exist so that the wheel-building machinery is proven end to end on every
platform: the extension loads, the abi3 tag is honoured, errors cross the
boundary as exceptions, and the core's determinism guarantee is still true
once a user is standing in Python.
"""

import openmobisim
import pytest


def test_extension_module_loads_and_reports_itself() -> None:
    info = openmobisim.build_info()
    assert info["version"] == openmobisim.__version__
    assert isinstance(info["code_version"], int)
    assert isinstance(info["rng_scheme_version"], int)
    assert isinstance(info["parallel"], bool)
    assert info["profile"] in {"debug", "release"}
    assert info["target"]


def test_version_is_a_semver_string() -> None:
    parts = openmobisim.__version__.split(".")
    assert len(parts) >= 3
    assert all(p[0].isdigit() for p in parts[:3])


def test_the_sum_is_deterministic_across_the_boundary() -> None:
    # Magnitudes several orders apart, so association order would show.
    values = [1e12, 1e-12, -1e12, 0.1] * 25_000
    first = openmobisim.fixed_order_sum_f64(values)
    for _ in range(10):
        assert openmobisim.fixed_order_sum_f64(values) == first


def test_the_sum_handles_the_empty_case() -> None:
    assert openmobisim.fixed_order_sum_f64([]) == 0.0


def test_step_lookup_matches_the_grid() -> None:
    # A four-hour window from 07:00 at the default 300-second step.
    origin = 7 * 3600
    assert openmobisim.step_of(origin, 300, 4 * 3600, origin) == 0
    assert openmobisim.step_of(origin, 300, 4 * 3600, origin + 300) == 1
    assert openmobisim.step_of(origin, 300, 4 * 3600, origin + 3600) == 12


def test_step_lookup_clamps_rather_than_failing() -> None:
    # The simulation never stops: an instant outside the window is clamped,
    # not an error.
    origin = 7 * 3600
    assert openmobisim.step_of(origin, 300, 3600, 0) == 0
    assert openmobisim.step_of(origin, 300, 3600, 10**9) == 11


@pytest.mark.parametrize(
    ("step_seconds", "window_seconds"),
    [(0, 3600), (300, 3500), (7, 100)],
)
def test_an_invalid_grid_raises_value_error(step_seconds: int, window_seconds: int) -> None:
    with pytest.raises(ValueError):
        openmobisim.step_of(0, step_seconds, window_seconds, 0)


def test_a_draw_is_a_pure_function_of_its_key() -> None:
    first = openmobisim.choice_draw(42, 412_002, 3, 7, 19)
    assert all(openmobisim.choice_draw(42, 412_002, 3, 7, 19) == first for _ in range(20))
    assert 0.0 <= first < 1.0


def test_every_key_component_changes_the_draw() -> None:
    base = openmobisim.choice_draw(42, 10, 20, 30, 40)
    assert openmobisim.choice_draw(43, 10, 20, 30, 40) != base
    assert openmobisim.choice_draw(42, 11, 20, 30, 40) != base
    assert openmobisim.choice_draw(42, 10, 21, 30, 40) != base
    assert openmobisim.choice_draw(42, 10, 20, 31, 40) != base
    assert openmobisim.choice_draw(42, 10, 20, 30, 41) != base
