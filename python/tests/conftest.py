"""Shared test setup: every test runs in a folder of its own.

A run's default output folder is ``./openmobisim-runs/<run_id>`` under the current directory, and a
folder that already holds a run is refused (P-20). Starting each test in a fresh temporary folder
keeps the repository clean and one test's runs from meeting another's.
"""

import pytest


@pytest.fixture(autouse=True)
def _own_folder(tmp_path, monkeypatch):
    """Start each test in its own empty folder."""
    monkeypatch.chdir(tmp_path)
