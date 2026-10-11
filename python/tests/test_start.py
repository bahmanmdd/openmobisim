"""The first steps (S264): the welcome, the tutorials copied, the pointer on import, the links.

Every way in leads to ``00_getting_started.ipynb``: ``openmobisim`` with no command prints what to
run first, ``openmobisim notebooks`` / ``openmobisim.notebooks()`` copy the tutorials and link the
first, and ``import openmobisim`` in a console says how, once, until they have been fetched.
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

import openmobisim as ms
import pytest
from openmobisim import _start
from openmobisim.__main__ import main


@pytest.fixture
def tutorials(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    """Three notebooks in a folder named by ``OPENMOBISIM_NOTEBOOKS``; a cache of our own."""
    source = tmp_path / "source"
    source.mkdir()
    for name in ("00_getting_started", "01_networks", "02_demand"):
        (source / f"{name}.ipynb").write_text(json.dumps({"cells": [], "name": name}))
    (source / "notes.txt").write_text("not a notebook")
    monkeypatch.setenv("OPENMOBISIM_NOTEBOOKS", str(source))
    monkeypatch.setenv("XDG_CACHE_HOME", str(tmp_path / "cache"))
    monkeypatch.chdir(tmp_path)
    return source


def test_the_welcome_says_what_to_run_first(capsys: pytest.CaptureFixture) -> None:
    assert main([]) == 0
    out = capsys.readouterr().out
    assert out.startswith(f"openmobisim {ms.__version__}")
    assert "Start here" in out and "00_getting_started" in out and _start.ISSUES_URL in out
    for command in ("notebooks", "cases", "run amsterdam", "check"):
        assert f"openmobisim {command}" in out


def test_the_tutorials_are_copied_kept_and_linked(
    tutorials: Path, tmp_path: Path, capsys: pytest.CaptureFixture
) -> None:
    folder = ms.notebooks()
    assert folder == tmp_path / "openmobisim-notebooks"
    assert sorted(p.name for p in folder.iterdir()) == [
        "00_getting_started.ipynb", "01_networks.ipynb", "02_demand.ipynb",
    ]  # fmt: skip
    out = capsys.readouterr().out
    assert "3 tutorial notebooks copied" in out
    assert (folder / "00_getting_started.ipynb").as_uri() in out
    # Your changes are kept, unless you ask.
    (folder / "01_networks.ipynb").write_text("mine")
    ms.notebooks()
    assert (folder / "01_networks.ipynb").read_text() == "mine"
    assert "(3 already there, kept)" in capsys.readouterr().out
    ms.notebooks(folder, overwrite=True)
    assert (folder / "01_networks.ipynb").read_text() != "mine"
    # The command line copies to the folder it is given, without JupyterLab when asked.
    assert main(["notebooks", "elsewhere", "--no-open"]) == 0
    assert (tmp_path / "elsewhere" / "00_getting_started.ipynb").exists()


def test_a_build_without_the_tutorials_points_to_the_repository(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture
) -> None:
    monkeypatch.setenv("OPENMOBISIM_NOTEBOOKS", str(tmp_path / "nothing"))
    monkeypatch.setattr(_start, "_TUTORIALS", tmp_path / "nothing")
    with pytest.raises(FileNotFoundError, match="tree/main/notebooks"):
        ms.notebooks(tmp_path / "x")
    assert main(["notebooks", str(tmp_path / "x"), "--no-open"]) == 1
    assert main([]) == 0
    assert _start.NOTEBOOKS_URL in capsys.readouterr().out


def _import_interactively(env: dict[str, str]) -> str:
    """``python -i -c 'import openmobisim'``: a console, as far as the pointer can tell."""
    done = subprocess.run(
        [sys.executable, "-i", "-c", "import openmobisim"],
        input="", capture_output=True, text=True, env=env, timeout=120,
    )  # fmt: skip
    return done.stdout + done.stderr


def test_the_pointer_shows_in_a_console_until_the_tutorials_are_fetched(
    tutorials: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    import os

    env = {**os.environ}
    assert "openmobisim.notebooks()" in _import_interactively(env)
    quiet = subprocess.run(
        [sys.executable, "-c", "import openmobisim"], capture_output=True, text=True, env=env
    )
    assert quiet.stdout == ""  # never in a script
    assert "notebooks()" not in _import_interactively({**env, "OPENMOBISIM_QUIET": "1"})
    ms.notebooks()
    assert "notebooks()" not in _import_interactively(env)  # fetched: silent from now on


def test_a_written_page_is_linked(tmp_path: Path, capsys: pytest.CaptureFixture) -> None:
    _start.show_path("Interactive map", tmp_path / "map.html")
    assert capsys.readouterr().out == f"Interactive map: {(tmp_path / 'map.html').as_uri()}\n"
