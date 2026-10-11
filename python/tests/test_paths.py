"""Where a figure or page was written, said as a link (S264)."""

from __future__ import annotations

from pathlib import Path

import pytest
from openmobisim._paths import show_path


def test_a_written_page_is_linked(tmp_path: Path, capsys: pytest.CaptureFixture) -> None:
    show_path("Interactive map", tmp_path / "map.html")
    assert capsys.readouterr().out == f"Interactive map: {(tmp_path / 'map.html').as_uri()}\n"
