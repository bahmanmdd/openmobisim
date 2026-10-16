"""Where a figure or page was written, said as a link."""

from __future__ import annotations

import logging
from pathlib import Path

import pytest
from openmobisim._paths import show_path


def test_a_written_page_is_linked(tmp_path: Path, caplog: pytest.LogCaptureFixture) -> None:
    with caplog.at_level(logging.INFO, logger="openmobisim"):
        show_path("Interactive map", tmp_path / "map.html")
    assert caplog.messages == [f"Interactive map: {(tmp_path / 'map.html').as_uri()}"]
