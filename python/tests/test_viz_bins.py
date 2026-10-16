"""The flow map's time bins as clock times beside bin numbers (C2, D-5 a)."""

from __future__ import annotations

import openmobisim as ms
import pytest
from openmobisim.viz._link import _bin_range


def test_clock_times_name_the_bins_that_cover_them() -> None:
    assert _bin_range("07:45", 900) == (31, 32)
    assert _bin_range(("07:30", "08:30"), 900) == (30, 34)
    assert _bin_range(("07:30", "08:20"), 900) == (30, 34), "a part of a bin takes the bin"
    assert _bin_range((30, 34), 900) == (30, 34) and _bin_range(4, 300) == (4, 5)
    with pytest.raises(ValueError, match="later"):
        _bin_range(("08:00", "07:30"), 900)
    with pytest.raises(ValueError, match="07:30"):
        _bin_range("half past seven", 900)


def test_the_map_draws_the_same_bins_by_clock_and_by_number(tmp_path) -> None:
    pytest.importorskip("matplotlib")
    from openmobisim import viz

    net = ms.examples.toy_network()
    rows = ms.examples.trips_random(net, 200, seed=3, min_m=100.0, max_m=2000.0, spread_s=1800)
    rows = [(*r[:6], 7 * 3600 + r[6], *r[7:]) for r in rows]
    run = ms.Scenario.from_parts(net, rows, link_bin_s=300).run(
        "bins", output_dir=str(tmp_path / "bins"), quiet=True
    )
    by_clock = viz.map_link(run, bins=("07:05", "07:15"), path=str(tmp_path / "clock.svg"))
    by_number = viz.map_link(run, bins=(85, 87), path=str(tmp_path / "number.svg"))
    assert by_clock is not None and by_number is not None
    clock = (tmp_path / "clock.svg").read_text()
    number = (tmp_path / "number.svg").read_text()
    assert clock.count("<path") == number.count("<path")
