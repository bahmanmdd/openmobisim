"""Run.map on io-parquet's hand run (test_hand_run.py), checked as a file: it is written and
returned, names no address, is the same bytes when written twice and in another folder, and is
titled by the file's stem; a folder as the path is an OSError and leaves no partial file; a path
that is a file of the run (one of its outputs, or an input given by another spelling of its path)
is refused, the file unchanged. The page's behaviour in a browser is not tested automatically.

For bikes: the hand run holds a bike trip and the path it rides, which the map is written from
like every other trip; what the map's data hold for cyclists is checked in crates/map's tests.
"""

import re
import time
from pathlib import Path

import pyarrow as pa
import pyarrow.parquet as pq
import pytest

from test_hand_run import hand_run, hand_tables

# An address: a scheme's or a protocol-relative "//host.domain". Base64 has no ".", and a comment's
# "//" is followed by a space, so neither is one.
ADDRESS = re.compile(r"//[A-Za-z0-9-]+(\.[A-Za-z0-9-]+)+")


def test_map_writes_one_file_naming_no_address_the_same_bytes_twice(tmp_path):
    result = hand_run(*hand_tables(), tmp_path / "run")
    path = tmp_path / "hand run.html"
    start = time.perf_counter()
    returned = result.map(path)
    print(f"map: {time.perf_counter() - start:.3f} s, {path.stat().st_size} bytes")
    assert Path(returned) == path and path.is_file()
    first = path.read_bytes()
    result.map(str(path))
    assert path.read_bytes() == first, "a second call wrote other bytes"
    (tmp_path / "other").mkdir()
    assert Path(result.map(tmp_path / "other" / "hand run.html")).read_bytes() == first
    text = first.decode("utf-8")
    assert "http://" not in text and "https://" not in text
    found = ADDRESS.search(text)
    assert found is None, f"an address: {found.group(0)}"
    # The title is the file's stem.
    assert "<title>hand run</title>" in text
    assert sorted(p.name for p in tmp_path.iterdir()) == ["hand run.html", "other", "run"]


def test_a_folder_as_the_path_is_an_os_error_and_leaves_no_partial_file(tmp_path):
    result = hand_run(*hand_tables(), tmp_path / "run")
    (tmp_path / "folder").mkdir()
    with pytest.raises(OSError):
        result.map(tmp_path / "folder")
    assert sorted(p.name for p in tmp_path.iterdir()) == ["folder", "run"]
    assert list((tmp_path / "folder").iterdir()) == []


def test_a_file_of_the_run_is_refused_and_left_unchanged(tmp_path):
    result = hand_run(*hand_tables(), tmp_path / "run")
    for output in (result.trips, result.links, result.iterations, result.routes, result.record):
        before = Path(output).read_bytes()
        with pytest.raises(ValueError, match=re.escape(Path(output).name)):
            result.map(output)
        assert Path(output).read_bytes() == before
    # The run's input files, one given through another spelling of its path.
    folder = tmp_path / "in"
    folder.mkdir()
    files = []
    for name, columns in zip(("nodes", "links", "trips"), hand_tables()):
        pq.write_table(pa.table(columns), folder / f"{name}.parquet")
        files.append(folder / f"{name}.parquet")
    from_files = hand_run(*files, tmp_path / "files")
    for path in (files[0], folder / ".." / "in" / "trips.parquet"):
        before = Path(path).read_bytes()
        with pytest.raises(ValueError, match=re.escape(Path(path).name)):
            from_files.map(path)
        assert Path(path).read_bytes() == before
    assert not any(p.name.endswith(".partial") for p in folder.iterdir())
